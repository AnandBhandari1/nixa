use super::git::{self, FileEntry};
use super::patch;
use std::path::Path;

pub const MAX_ROWS: usize = 2000;
pub const MAX_TEXT: usize = 4000;
pub const BIG_DIFF: usize = 1_500_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Staged,
    Unstaged,
    Untracked,
    Conflict,
}

impl Side {
    pub fn lines_ok(self) -> bool {
        self != Side::Conflict
    }

    pub fn working(self) -> bool {
        matches!(self, Side::Unstaged | Side::Untracked)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Section,
    Bar,
    At,
    Add,
    Del,
    Same,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: RowKind,
    pub number: Option<u32>,
    pub text: String,
    pub hunk: Option<usize>,
    pub at: usize,
    pub spans: Vec<(usize, usize)>,
}

impl Row {
    fn plain(kind: RowKind, text: impl Into<String>) -> Row {
        Row {
            kind,
            number: None,
            text: text.into(),
            hunk: None,
            at: 0,
            spans: Vec::new(),
        }
    }

    pub fn change(&self) -> bool {
        matches!(self.kind, RowKind::Add | RowKind::Del)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub id: usize,
    pub side: Side,
    pub path: String,
    pub header: Vec<String>,
    pub lines: Vec<String>,
    pub patch: String,
    pub whole: bool,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub label: String,
    pub side: Side,
    pub header: Vec<String>,
    pub hunks: Vec<Vec<String>>,
    pub path: String,
}

fn non_empty(text: &str) -> String {
    if text.is_empty() { " ".into() } else { text.to_string() }
}

fn header_numbers(line: &str) -> Option<(u32, u32)> {
    let (head, _) = patch::parse_head(line)?;
    Some((head.old_start, head.new_start))
}

pub fn diff_rows(lines: &[String]) -> Vec<Row> {
    let mut old = 0u32;
    let mut new = 0u32;
    lines
        .iter()
        .enumerate()
        .map(|(at, line)| {
            if let Some((o, n)) = header_numbers(line) {
                old = o;
                new = n;
                let mut r = Row::plain(RowKind::At, line.clone());
                r.at = at;
                return r;
            }
            let rest = line.get(1..).unwrap_or("");
            let mut row = match line.chars().next() {
                Some('-') => {
                    let r = Row {
                        number: Some(old),
                        ..Row::plain(RowKind::Del, non_empty(rest))
                    };
                    old += 1;
                    r
                }
                Some('+') => {
                    let r = Row {
                        number: Some(new),
                        ..Row::plain(RowKind::Add, non_empty(rest))
                    };
                    new += 1;
                    r
                }
                Some(' ') => {
                    old += 1;
                    let r = Row {
                        number: Some(new),
                        ..Row::plain(RowKind::Same, non_empty(rest))
                    };
                    new += 1;
                    r
                }
                _ => Row::plain(RowKind::Plain, line.clone()),
            };
            row.at = at;
            row
        })
        .collect()
}

pub fn note_rows(lines: &[&str]) -> Vec<Row> {
    lines.iter().map(|l| Row::plain(RowKind::Plain, *l)).collect()
}

pub fn section_rows(sections: &[Section]) -> (Vec<Row>, Vec<Hunk>) {
    let mut rows = Vec::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    for s in sections {
        if s.hunks.is_empty() {
            continue;
        }
        rows.push(Row::plain(RowKind::Section, s.label.clone()));
        for lines in &s.hunks {
            let id = hunks.len();
            let patch = format!("{}\n{}\n", s.header.join("\n"), lines.join("\n"));
            let label = format!("{} · {}", s.path, lines.first().cloned().unwrap_or_default());
            hunks.push(Hunk {
                id,
                side: s.side,
                path: s.path.clone(),
                header: s.header.clone(),
                lines: lines.clone(),
                patch,
                whole: patch::complete(lines),
                label: label.clone(),
            });
            rows.push(Row {
                hunk: Some(id),
                ..Row::plain(RowKind::Bar, label)
            });
            for mut row in diff_rows(lines) {
                if row.change() {
                    row.hunk = Some(id);
                }
                rows.push(row);
            }
        }
    }
    (rows, hunks)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Mark,
}

fn class(c: char) -> Class {
    if c.is_ascii_alphanumeric() || c == '_' || (c as u32) >= 0x80 && !c.is_whitespace() {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Mark
    }
}

pub fn tokenize(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut last: Option<Class> = None;
    for (i, c) in text.char_indices() {
        let k = class(c);
        match last {
            Some(prev) if prev == k && k != Class::Mark => {}
            Some(_) => {
                out.push(&text[start..i]);
                start = i;
            }
            None => {}
        }
        last = Some(k);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

const MAX_LINE: usize = 500;
const MAX_HUNK: usize = 400;
const MAX_CELLS: usize = 10_000;
const MIN_SHARED: f32 = 0.3;
const MAX_WORK: usize = 400_000;

fn blank(t: &str) -> bool {
    t.chars().all(char::is_whitespace)
}

fn spans_of(tokens: &[&str], same: &[bool]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut pos = 0;
    for (i, t) in tokens.iter().enumerate() {
        let end = pos + t.len();
        if !same[i] {
            match out.last_mut() {
                Some(last) if last.1 == pos => last.1 = end,
                _ => out.push((pos, end)),
            }
        }
        pos = end;
    }
    out
}

fn merge_spans(text: &str, spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for s in spans {
        match out.last_mut() {
            Some(last) if blank(&text[last.1..s.0]) => last.1 = s.1,
            _ => out.push(s),
        }
    }
    out
}

fn solid(tokens: &[&str], same: &[bool], want: bool) -> usize {
    tokens
        .iter()
        .enumerate()
        .filter(|(i, t)| same[*i] == want && !blank(t))
        .map(|(_, t)| t.len())
        .sum()
}

pub type LineSpans = (Vec<(usize, usize)>, Vec<(usize, usize)>);

pub fn line_spans(a: &str, b: &str, budget: &mut usize) -> Option<LineSpans> {
    if a == b || a.len() > MAX_LINE || b.len() > MAX_LINE {
        return None;
    }
    let ta = tokenize(a);
    let tb = tokenize(b);
    let mut pre = 0;
    while pre < ta.len() && pre < tb.len() && ta[pre] == tb[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < ta.len() - pre && suf < tb.len() - pre && ta[ta.len() - 1 - suf] == tb[tb.len() - 1 - suf] {
        suf += 1;
    }
    let mut same_a: Vec<bool> = (0..ta.len()).map(|i| i < pre || i >= ta.len() - suf).collect();
    let mut same_b: Vec<bool> = (0..tb.len()).map(|i| i < pre || i >= tb.len() - suf).collect();
    let mid_a = &ta[pre..ta.len() - suf];
    let mid_b = &tb[pre..tb.len() - suf];
    let cells = mid_a.len() * mid_b.len();
    if cells > 0 && cells <= MAX_CELLS && *budget >= cells {
        *budget -= cells;
        let n = mid_a.len();
        let m = mid_b.len();
        let mut dp = vec![vec![0u16; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                dp[i][j] = if mid_a[i] == mid_b[j] {
                    dp[i + 1][j + 1] + 1
                } else {
                    dp[i + 1][j].max(dp[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if mid_a[i] == mid_b[j] {
                same_a[pre + i] = true;
                same_b[pre + j] = true;
                i += 1;
                j += 1;
            } else if dp[i + 1][j] >= dp[i][j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    let shared = solid(&ta, &same_a, true);
    let longest = (solid(&ta, &same_a, true) + solid(&ta, &same_a, false))
        .max(solid(&tb, &same_b, true) + solid(&tb, &same_b, false));
    if longest > 0 && (shared as f32) / (longest as f32) < MIN_SHARED {
        return None;
    }
    let sa = merge_spans(a, spans_of(&ta, &same_a));
    let sb = merge_spans(b, spans_of(&tb, &same_b));
    if sa.is_empty() && sb.is_empty() {
        return None;
    }
    Some((sa, sb))
}

fn pair_blocks(rows: &mut [Row], from: usize, to: usize, budget: &mut usize) {
    let mut i = from;
    while i < to {
        if rows[i].kind != RowKind::Del {
            i += 1;
            continue;
        }
        let mut minus = Vec::new();
        let mut plus = Vec::new();
        while i < to && matches!(rows[i].kind, RowKind::Del | RowKind::Plain) {
            if rows[i].kind == RowKind::Del {
                minus.push(i);
            }
            i += 1;
        }
        while i < to && matches!(rows[i].kind, RowKind::Add | RowKind::Plain) {
            if rows[i].kind == RowKind::Add {
                plus.push(i);
            }
            i += 1;
        }
        for k in 0..minus.len().min(plus.len()) {
            let (a, b) = (rows[minus[k]].text.clone(), rows[plus[k]].text.clone());
            if let Some((sa, sb)) = line_spans(&a, &b, budget) {
                rows[minus[k]].spans = sa;
                rows[plus[k]].spans = sb;
            }
        }
    }
}

fn close(rows: &mut [Row], start: &mut Option<usize>, end: usize, budget: &mut usize) {
    if let Some(s) = start.take()
        && end - s <= MAX_HUNK {
            pair_blocks(rows, s, end, budget);
        }
}

pub fn word_spans(rows: &mut [Row]) {
    let mut budget = MAX_WORK;
    let mut start: Option<usize> = None;
    let len = rows.len();
    for i in 0..len {
        match rows[i].kind {
            RowKind::At => {
                close(rows, &mut start, i, &mut budget);
                start = Some(i + 1);
            }
            RowKind::Section | RowKind::Bar => close(rows, &mut start, i, &mut budget),
            _ => {}
        }
    }
    close(rows, &mut start, len, &mut budget);
}

pub fn change_starts(rows: &[Row]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut before = false;
    for (i, row) in rows.iter().enumerate() {
        if row.kind == RowKind::Plain {
            continue;
        }
        let now = row.change();
        if now && !before {
            out.push(i);
        }
        before = now;
    }
    out
}

pub fn next_change(starts: &[usize], from: Option<usize>, down: bool) -> Option<usize> {
    if down {
        starts.iter().copied().find(|i| from.is_none_or(|f| *i > f))
    } else {
        let f = from?;
        starts.iter().rev().copied().find(|i| *i < f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub hunk: usize,
    pub patch: String,
    pub side: Side,
    pub lines: Vec<usize>,
    pub anchor: Option<usize>,
}

pub fn keep_pick(pick: Option<Pick>, hunks: &[Hunk]) -> Option<Pick> {
    let p = pick?;
    let h = hunks.iter().find(|h| h.patch == p.patch && h.side == p.side)?;
    Some(Pick { hunk: h.id, ..p })
}

pub fn toggle_pick(pick: Option<Pick>, hunk: &Hunk, at: usize, shift: bool) -> Option<Pick> {
    let lines = patch::change_indexes(&hunk.lines);
    if !lines.contains(&at) {
        return pick;
    }
    let same = pick
        .as_ref()
        .is_some_and(|p| p.patch == hunk.patch && p.side == hunk.side);
    let base = match pick {
        Some(p) if same => p,
        _ => empty_pick(hunk),
    };
    let mut next = Pick {
        hunk: hunk.id,
        ..base.clone()
    };
    match (shift && same, base.anchor) {
        (true, Some(anchor)) => {
            let (lo, hi) = (anchor.min(at), anchor.max(at));
            next.lines.extend(lines.iter().copied().filter(|i| *i >= lo && *i <= hi));
        }
        _ => {
            if next.lines.contains(&at) {
                next.lines.retain(|i| *i != at);
            } else {
                next.lines.push(at);
            }
            next.anchor = Some(at);
        }
    }
    next.lines.sort_unstable();
    next.lines.dedup();
    (!next.lines.is_empty()).then_some(next)
}

fn empty_pick(hunk: &Hunk) -> Pick {
    Pick {
        hunk: hunk.id,
        patch: hunk.patch.clone(),
        side: hunk.side,
        lines: Vec::new(),
        anchor: None,
    }
}

pub struct Loaded {
    pub rows: Vec<Row>,
    pub hunks: Vec<Hunk>,
    pub add: Option<u32>,
}

fn common(ignore_space: bool) -> Vec<&'static str> {
    let mut c = vec!["--no-ext-diff", "--no-textconv", "--no-color", "--no-renames"];
    if ignore_space {
        c.push("-w");
    }
    c
}

pub fn count_lines(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let n = text.split('\n').count();
    (n - usize::from(text.ends_with('\n'))) as u32
}

pub fn finish(mut rows: Vec<Row>, hunks: Vec<Hunk>, add: Option<u32>) -> Loaded {
    rows.truncate(MAX_ROWS);
    for r in rows.iter_mut() {
        if r.text.len() > MAX_TEXT {
            let mut cut = MAX_TEXT;
            while !r.text.is_char_boundary(cut) {
                cut -= 1;
            }
            r.text = format!("{}… (line truncated)", &r.text[..cut]);
        }
    }
    word_spans(&mut rows);
    Loaded { rows, hunks, add }
}

fn load_fresh(root: &str, file: &FileEntry, ignore_space: bool) -> Loaded {
    let mut args = vec!["diff", "--no-index"];
    args.extend(common(ignore_space));
    args.extend(["--", "/dev/null", file.path.as_str()]);
    let made = git::read(root, &args);
    let parsed = (made.code == Some(0) || made.code == Some(1)).then(|| patch::split_hunks(&made.stdout));
    if let Some(p) = parsed.as_ref().filter(|p| !p.hunks.is_empty()) {
        let add = p
            .hunks
            .iter()
            .map(|h| h.iter().filter(|l| l.starts_with('+')).count() as u32)
            .sum();
        if made.stdout.len() <= BIG_DIFF {
            let sections = vec![Section {
                label: "New file, not tracked yet".into(),
                side: Side::Untracked,
                header: p.header.clone(),
                hunks: p.hunks.clone(),
                path: file.path.clone(),
            }];
            let (rows, hunks) = section_rows(&sections);
            return finish(rows, hunks, Some(add));
        }
        let lines: Vec<String> = p.hunks[0].iter().take(1501).cloned().collect();
        return finish(diff_rows(&lines), Vec::new(), Some(add));
    }
    if parsed.is_some() && made.stdout.lines().any(|l| l.starts_with("Binary files ")) {
        return finish(note_rows(&["Binary file · no text diff"]), Vec::new(), None);
    }
    if parsed.is_some() && made.stdout.lines().any(|l| l.starts_with("new file mode ")) {
        return finish(note_rows(&["Empty new file"]), Vec::new(), None);
    }
    match std::fs::read(Path::new(root).join(&file.path)) {
        Err(_) => finish(
            note_rows(&["Cannot preview this file (binary, too large, or unreadable)"]),
            Vec::new(),
            None,
        ),
        Ok(bytes) if bytes.contains(&0) => finish(note_rows(&["Binary file · no text diff"]), Vec::new(), None),
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes).to_string();
            let add = count_lines(&text);
            let mut lines = vec![format!("@@ -0,0 +1,{add} @@")];
            let mut cut = text.len().min(400_000);
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            let body = text[..cut].strip_suffix('\n').unwrap_or(&text[..cut]);
            lines.extend(body.split('\n').take(1500).map(|l| format!("+{l}")));
            finish(diff_rows(&lines), Vec::new(), Some(add))
        }
    }
}

pub fn load(root: &str, file: &FileEntry, ignore_space: bool) -> Result<Loaded, String> {
    if file.fresh {
        return Ok(load_fresh(root, file, ignore_space));
    }
    let mut sections = Vec::new();
    if file.staged {
        let mut args = vec!["diff", "--cached"];
        args.extend(common(ignore_space));
        args.extend(["--", file.path.as_str()]);
        let out = git::read(root, &args);
        if !out.ok() {
            return Err(git::detail_of(&out.said()));
        }
        let p = patch::split_hunks(&out.stdout);
        if !p.hunks.is_empty() {
            sections.push(Section {
                label: "Staged changes".into(),
                side: Side::Staged,
                header: p.header,
                hunks: p.hunks,
                path: file.path.clone(),
            });
        }
    }
    if file.y != ' ' || file.conflict {
        let mut args = vec!["diff"];
        args.extend(common(ignore_space));
        args.extend(["--", file.path.as_str()]);
        let out = git::read(root, &args);
        if !out.ok() {
            return Err(git::detail_of(&out.said()));
        }
        let p = patch::split_hunks(&out.stdout);
        if !p.hunks.is_empty() {
            let (label, side) = if file.conflict {
                ("Conflicted file", Side::Conflict)
            } else {
                ("Unstaged changes", Side::Unstaged)
            };
            sections.push(Section {
                label: label.into(),
                side,
                header: p.header,
                hunks: p.hunks,
                path: file.path.clone(),
            });
        }
    }
    let (rows, hunks) = section_rows(&sections);
    if rows.is_empty() {
        let text = if file.binary {
            "Binary file changed"
        } else if ignore_space {
            "Only spaces changed. Turn off ignore spaces to see them."
        } else {
            "No text diff available"
        };
        return Ok(finish(note_rows(&[text]), Vec::new(), None));
    }
    Ok(finish(rows, hunks, None))
}

pub fn file_entry(root: &str, path: &str) -> Result<Option<FileEntry>, String> {
    let status = git::read(
        root,
        &["status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all", "--", path],
    );
    if !status.ok() {
        return Err(git::detail_of(&status.said()));
    }
    let Some(entry) = status.stdout.split('\0').find(|e| e.get(3..) == Some(path)) else {
        return Ok(None);
    };
    let stat = git::read(root, &["diff", "HEAD", "--numstat", "-z", "--no-renames", "--", path]);
    let mut stats = std::collections::HashMap::new();
    git::add_numstat(&stat.stdout, &mut stats);
    Ok(git::status_file(entry, &stats))
}
