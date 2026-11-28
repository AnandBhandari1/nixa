use crate::patch::{RawHunk, apply_patch, change_indexes, hunk_complete, split_hunks};
use crate::outcome::{Outcome, failure, success};
use crate::run::git;
use crate::status::{FileChange, READ};
use crate::worddiff::{Span, WordRow, word_spans};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MAX_ROWS: usize = 2000;

pub const MAX_ROW_TEXT: usize = 4000;

pub const MAX_FRESH_DIFF: usize = 1_500_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub kind: String,
    pub number: Option<u64>,
    pub text: String,
    pub hunk: Option<usize>,
    pub at: Option<usize>,
    pub side: String,
    pub path: String,
    pub patch: String,
    pub label: String,
    pub spans: Option<Vec<Span>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub label: String,
    pub side: String,
    pub header: Vec<String>,
    pub hunks: Vec<RawHunk>,
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub id: usize,
    pub side: String,
    pub path: String,
    pub header: Vec<String>,
    pub lines: Vec<String>,
    pub patch: String,
    pub whole: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub rows: Vec<Row>,
    pub hunks: Vec<Hunk>,
    pub sections: Vec<Section>,
    pub add: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinePick {
    pub hunk: usize,
    pub patch: String,
    pub side: String,
    pub lines: Vec<usize>,
    pub anchor: Option<usize>,
}

fn text_or_space(text: &str) -> String {
    if text.is_empty() { " ".into() } else { text.into() }
}

pub fn diff_rows(lines: &[String]) -> Vec<Row> {
    let (mut old, mut next) = (0u64, 0u64);
    lines
        .iter()
        .map(|line| {
            if let Some(h) = loose_header(line) {
                old = h.0;
                next = h.1;
                return Row { kind: "@".into(), text: line.clone(), ..Row::default() };
            }
            let rest = line.get(1..).unwrap_or("");
            match line.chars().next() {
                Some('-') => {
                    old += 1;
                    Row { kind: "-".into(), number: Some(old - 1), text: text_or_space(rest), ..Row::default() }
                }
                Some('+') => {
                    next += 1;
                    Row { kind: "+".into(), number: Some(next - 1), text: text_or_space(rest), ..Row::default() }
                }
                Some(' ') => {
                    old += 1;
                    next += 1;
                    Row { kind: " ".into(), number: Some(next - 1), text: text_or_space(rest), ..Row::default() }
                }
                _ => Row { kind: String::new(), text: line.clone(), ..Row::default() },
            }
        })
        .collect()
}

fn loose_header(line: &str) -> Option<(u64, u64)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(' ')?;
    let new = rest.strip_prefix('+')?.split([' ', ',']).next()?;
    Some((old.split(',').next()?.parse().ok()?, new.parse().ok()?))
}

pub fn section_rows(sections: &[Section]) -> (Vec<Row>, Vec<Hunk>) {
    let mut rows = Vec::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    for section in sections {
        if section.hunks.is_empty() {
            continue;
        }
        rows.push(Row { kind: "section".into(), text: section.label.clone(), ..Row::default() });
        for hunk in &section.hunks {
            let id = hunks.len();
            let patch = format!("{}\n{}\n", section.header.join("\n"), hunk.lines.join("\n"));
            hunks.push(Hunk {
                id,
                side: section.side.clone(),
                path: section.path.clone(),
                header: section.header.clone(),
                lines: hunk.lines.clone(),
                patch: patch.clone(),
                whole: hunk_complete(&hunk.lines),
            });
            rows.push(Row {
                kind: "hunkbar".into(),
                hunk: Some(id),
                side: section.side.clone(),
                path: section.path.clone(),
                label: format!("{} · {}", section.path, hunk.header),
                patch,
                ..Row::default()
            });
            for (at, row) in diff_rows(&hunk.lines).into_iter().enumerate() {
                if row.kind == "+" || row.kind == "-" {
                    rows.push(Row { hunk: Some(id), at: Some(at), ..row });
                } else {
                    rows.push(row);
                }
            }
        }
    }
    (rows, hunks)
}

pub fn with_spans(rows: Vec<Row>) -> Vec<Row> {
    let words: Vec<WordRow> = rows.iter().map(|r| WordRow { kind: r.kind.clone(), text: r.text.clone() }).collect();
    let spans = word_spans(&words);
    rows.into_iter().zip(spans).map(|(row, s)| Row { spans: s, ..row }).collect()
}

pub fn text_rows(lines: &[&str]) -> Vec<Row> {
    diff_rows(&lines.iter().map(|l| l.to_string()).collect::<Vec<_>>())
}

fn count_lines(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    (text.split('\n').count() - usize::from(text.ends_with('\n'))) as u64
}

fn read_diff(root: &Path, args: &[&str]) -> crate::run::Out {
    let mut all = READ.to_vec();
    all.extend_from_slice(args);
    git(root, &all)
}

fn trim_rows(rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter()
        .take(MAX_ROWS)
        .map(|row| {
            if row.text.chars().count() > MAX_ROW_TEXT {
                let short: String = row.text.chars().take(MAX_ROW_TEXT).collect();
                Row { text: format!("{short}… (line truncated)"), ..row }
            } else {
                row
            }
        })
        .collect()
}

fn fresh_diff(root: &Path, file: &FileChange, common: &[&str]) -> FileDiff {
    let path = file.path.as_str();
    let mut args = vec!["diff", "--no-index"];
    args.extend_from_slice(common);
    args.extend(["--", "/dev/null", path]);
    let made = read_diff(root, &args);
    let parsed = if made.code == Some(0) || made.code == Some(1) { Some(split_hunks(&made.stdout)) } else { None };
    let mut out = FileDiff::default();
    if let Some(p) = &parsed
        && !p.hunks.is_empty()
    {
        out.add = Some(p.hunks.iter().map(|h| h.lines.iter().filter(|l| l.starts_with('+')).count() as u64).sum());
        if made.stdout.len() <= MAX_FRESH_DIFF {
            let sections = vec![Section {
                label: "New file, not tracked yet".into(),
                side: "untracked".into(),
                header: p.header.clone(),
                hunks: p.hunks.clone(),
                path: path.into(),
            }];
            let (rows, hunks) = section_rows(&sections);
            return FileDiff { rows, hunks, sections, add: out.add };
        }
        out.rows = diff_rows(&p.hunks[0].lines.iter().take(1501).cloned().collect::<Vec<_>>());
        return out;
    }
    if parsed.is_some() && made.stdout.lines().any(|l| l.starts_with("Binary files ")) {
        out.rows = text_rows(&["Binary file · no text diff"]);
        return out;
    }
    if parsed.is_some() && made.stdout.lines().any(|l| l.starts_with("new file mode ")) {
        out.rows = text_rows(&["Empty new file"]);
        return out;
    }
    match std::fs::read(root.join(path)) {
        Err(_) => out.rows = text_rows(&["Cannot preview this file (binary, too large, or unreadable)"]),
        Ok(bytes) if bytes.contains(&0) => out.rows = text_rows(&["Binary file · no text diff"]),
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let add = count_lines(&text);
            out.add = Some(add);
            let cut: String = text.chars().take(400000).collect();
            let cut = cut.strip_suffix('\n').unwrap_or(&cut);
            let mut lines = vec![format!("@@ -0,0 +1,{add} @@")];
            lines.extend(cut.split('\n').take(1500).map(|l| format!("+{l}")));
            out.rows = diff_rows(&lines);
        }
    }
    out
}

pub fn file_diff(root: &Path, file: &FileChange, ignore_space: bool) -> Result<FileDiff, String> {
    let mut common = vec!["--no-ext-diff", "--no-textconv", "--no-color", "--no-renames"];
    if ignore_space {
        common.push("-w");
    }
    let mut out = if file.fresh {
        fresh_diff(root, file, &common)
    } else {
        let mut sections = Vec::new();
        let path = file.path.as_str();
        if file.staged {
            let mut args = vec!["diff", "--cached"];
            args.extend_from_slice(&common);
            args.extend(["--", path]);
            let staged = read_diff(root, &args);
            if !staged.ok() {
                return Err(staged.stderr);
            }
            let parsed = split_hunks(&staged.stdout);
            if !parsed.hunks.is_empty() {
                sections.push(Section { label: "Staged changes".into(), side: "staged".into(), header: parsed.header, hunks: parsed.hunks, path: path.into() });
            }
        }
        if file.y != ' ' || file.conflict {
            let mut args = vec!["diff"];
            args.extend_from_slice(&common);
            args.extend(["--", path]);
            let unstaged = read_diff(root, &args);
            if !unstaged.ok() {
                return Err(unstaged.stderr);
            }
            let parsed = split_hunks(&unstaged.stdout);
            if !parsed.hunks.is_empty() {
                let (label, side) = if file.conflict { ("Conflicted file", "conflict") } else { ("Unstaged changes", "unstaged") };
                sections.push(Section { label: label.into(), side: side.into(), header: parsed.header, hunks: parsed.hunks, path: path.into() });
            }
        }
        let (mut rows, hunks) = section_rows(&sections);
        if rows.is_empty() {
            let note = if file.binary {
                "Binary file changed"
            } else if ignore_space {
                "Only spaces changed. Turn off ignore spaces to see them."
            } else {
                "No text diff available"
            };
            rows = text_rows(&[note]);
        }
        FileDiff { rows, hunks, sections, add: None }
    };
    out.rows = with_spans(trim_rows(out.rows));
    Ok(out)
}

pub fn apply_hunk(root: &Path, hunk: &Hunk, mode: &str) -> Outcome {
    if hunk.side == "untracked" && mode == "discard" {
        let out = git(root, &["--literal-pathspecs", "clean", "-fd", "--", &hunk.path]);
        if !out.ok() {
            return failure(&out.said());
        }
        return success("Done");
    }
    apply_patch(root, &hunk.patch, mode)
}

pub fn change_starts(rows: &[Row]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut before = false;
    for (i, row) in rows.iter().enumerate() {
        if row.kind.is_empty() {
            continue;
        }
        let now = row.kind == "+" || row.kind == "-";
        if now && !before {
            out.push(i);
        }
        before = now;
    }
    out
}

pub fn next_change(starts: &[usize], from: usize, forward: bool) -> Option<usize> {
    if forward { starts.iter().copied().find(|&i| i > from) } else { starts.iter().rev().copied().find(|&i| i < from) }
}

pub fn keep_pick(pick: Option<&LinePick>, hunks: &[Hunk]) -> Option<LinePick> {
    let pick = pick?;
    let hunk = hunks.iter().find(|h| h.patch == pick.patch && h.side == pick.side)?;
    Some(LinePick { hunk: hunk.id, ..pick.clone() })
}

pub fn toggle_pick(pick: Option<&LinePick>, hunk: &Hunk, at: usize, shift: bool) -> Option<LinePick> {
    let lines = change_indexes(&hunk.lines);
    if !lines.contains(&at) {
        return pick.cloned();
    }
    let same = pick.is_some_and(|p| p.patch == hunk.patch && p.side == hunk.side);
    let base = match pick {
        Some(p) if same => p.clone(),
        _ => LinePick { hunk: hunk.id, patch: hunk.patch.clone(), side: hunk.side.clone(), lines: Vec::new(), anchor: None },
    };
    let sorted = |mut list: Vec<usize>| {
        list.sort();
        list.dedup();
        list
    };
    let next = match base.anchor {
        Some(anchor) if shift && same => {
            let (lo, hi) = (anchor.min(at), anchor.max(at));
            let mut all = base.lines.clone();
            all.extend(lines.iter().copied().filter(|&i| i >= lo && i <= hi));
            LinePick { hunk: hunk.id, lines: sorted(all), ..base }
        }
        _ => {
            let list = if base.lines.contains(&at) {
                base.lines.iter().copied().filter(|&i| i != at).collect()
            } else {
                let mut l = base.lines.clone();
                l.push(at);
                sorted(l)
            };
            LinePick { hunk: hunk.id, lines: list, anchor: Some(at), ..base }
        }
    };
    if next.lines.is_empty() { None } else { Some(next) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::status;
    use crate::testrepo;

    #[test]
    fn rows_number_lines() {
        let rows = text_rows(&["@@ -3,2 +3,2 @@", " keep", "-old", "+new", "note"]);
        assert_eq!(rows[0].kind, "@");
        assert_eq!(rows[1].number, Some(3));
        assert_eq!(rows[2].number, Some(4));
        assert_eq!(rows[3].number, Some(4));
        assert_eq!(rows[4].kind, "");
        assert_eq!(change_starts(&rows), vec![2]);
        let spans = with_spans(text_rows(&["@@ -1 +1 @@", "-value = 10", "+value = 20"]));
        assert_eq!(spans[1].spans, Some(vec![(8, 10)]));
        assert_eq!(next_change(&[2, 9], 2, true), Some(9));
        assert_eq!(next_change(&[2, 9], 9, false), Some(2));
    }

    #[test]
    fn file_diff_sections_and_hunk_apply() {
        let repo = testrepo::new();
        let root = repo.root();
        repo.write("a.txt", "ONE\ntwo\nthree\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "ONE\ntwo\nTHREE\n");
        repo.write("n.txt", "x\ny\n");
        let s = status(root).unwrap();
        let a = s.files.iter().find(|f| f.path == "a.txt").unwrap();
        let d = file_diff(root, a, false).unwrap();
        assert_eq!(d.sections.len(), 2);
        assert_eq!(d.sections[0].side, "staged");
        assert_eq!(d.hunks.len(), 2);
        assert!(d.rows.iter().any(|r| r.kind == "hunkbar"));
        let staged = apply_hunk(root, &d.hunks[0], "unstage");
        assert!(staged.ok, "{staged:?}");
        let n = s.files.iter().find(|f| f.path == "n.txt").unwrap();
        let fresh = file_diff(root, n, false).unwrap();
        assert_eq!(fresh.add, Some(2));
        assert_eq!(fresh.hunks[0].side, "untracked");
        let added = apply_hunk(root, &fresh.hunks[0], "stage");
        assert!(added.ok, "{added:?}");
        assert!(repo.git(&["diff", "--cached", "--name-only"]).contains("n.txt"));
        repo.write("m.txt", "z\n");
        let m = status(root).unwrap().files.into_iter().find(|f| f.path == "m.txt").unwrap();
        let md = file_diff(root, &m, false).unwrap();
        assert!(apply_hunk(root, &md.hunks[0], "discard").ok);
        assert!(!repo.path("m.txt").exists());
    }

    #[test]
    fn spaces_only_and_picks() {
        let repo = testrepo::new();
        let root = repo.root();
        repo.write("a.txt", "one  \ntwo\nthree\n");
        let a = status(root).unwrap().files.remove(0);
        let d = file_diff(root, &a, true).unwrap();
        assert_eq!(d.rows[0].text, "Only spaces changed. Turn off ignore spaces to see them.");
        let d = file_diff(root, &a, false).unwrap();
        let hunk = &d.hunks[0];
        let pick = toggle_pick(None, hunk, 1, false).unwrap();
        assert_eq!(pick.lines, vec![1]);
        let pick = toggle_pick(Some(&pick), hunk, 2, true).unwrap();
        assert_eq!(pick.lines, vec![1, 2]);
        assert!(keep_pick(Some(&pick), &d.hunks).is_some());
        let off = toggle_pick(Some(&LinePick { lines: vec![1], anchor: Some(1), ..pick.clone() }), hunk, 1, false);
        assert!(off.is_none());
    }
}
