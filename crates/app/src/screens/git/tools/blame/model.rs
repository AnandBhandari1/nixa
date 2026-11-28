use std::collections::HashMap;

pub const MAX_LINES: usize = 20_000;
pub const BANDS: usize = 5;
pub const CHAR: f32 = 7.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Previous {
    pub sha: String,
    pub file: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub short: String,
    pub author: String,
    pub email: String,
    pub at: i64,
    pub tz: String,
    pub committer: String,
    pub committer_email: String,
    pub committed_at: i64,
    pub summary: String,
    pub boundary: bool,
    pub uncommitted: bool,
    pub previous: Option<Previous>,
    pub filename: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub n: u32,
    pub orig: u32,
    pub text: String,
    pub sha: String,
    pub short: String,
    pub author: String,
    pub at: i64,
    pub summary: String,
    pub boundary: bool,
    pub uncommitted: bool,
    pub filename: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub lines: Vec<Line>,
    pub commits: HashMap<String, Commit>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bad,
    IgnoreRevs,
    New,
    Missing,
    BadRev,
    Gone,
    Binary,
    Cancelled,
    Timeout,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub kind: Kind,
    pub title: String,
    pub hint: String,
    pub detail: String,
}

pub fn problem(kind: Kind, title: &str, hint: &str) -> Problem {
    Problem {
        kind,
        title: title.into(),
        hint: hint.into(),
        detail: String::new(),
    }
}

pub fn is_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64)
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn is_uncommitted(sha: &str) -> bool {
    !sha.is_empty() && sha.bytes().all(|b| b == b'0')
}

pub fn short_sha(sha: &str) -> String {
    if is_uncommitted(sha) {
        String::new()
    } else {
        sha.chars().take(7).collect()
    }
}

pub fn short_rev(rev: &str) -> String {
    if is_sha(rev) {
        rev[..7].to_string()
    } else {
        rev.to_string()
    }
}

pub fn clean_path(path: &str) -> Result<String, String> {
    if path.is_empty()
        || path.contains('\0')
        || path.starts_with('/')
        || path.split('/').any(|p| p == "..")
    {
        return Err(format!("not a repo file path: {path}"));
    }
    Ok(path.to_string())
}

pub fn clean_rev(rev: &str) -> Result<String, String> {
    let r = rev.trim();
    if r.is_empty() {
        return Ok(String::new());
    }
    if r.starts_with('-') || r.chars().any(|c| c.is_whitespace() || c == '\0') || r.contains("..") {
        return Err(format!("not a commit: {r}"));
    }
    Ok(r.to_string())
}

fn escape(c: char) -> Option<u8> {
    Some(match c {
        'a' => 7,
        'b' => 8,
        't' => b'\t',
        'n' => b'\n',
        'v' => 11,
        'f' => 12,
        'r' => b'\r',
        '"' => b'"',
        '\\' => b'\\',
        _ => return None,
    })
}

pub fn unquote_path(text: &str) -> String {
    if text.len() < 2 || !text.starts_with('"') || !text.ends_with('"') {
        return text.to_string();
    }
    let body: Vec<char> = text[1..text.len() - 1].chars().collect();
    let mut bytes: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let c = body[i];
        if c != '\\' || i + 1 >= body.len() {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            i += 1;
            continue;
        }
        let octal: String = body[i + 1..(i + 4).min(body.len())].iter().collect();
        if octal.len() == 3 && octal.chars().all(|d| ('0'..='7').contains(&d)) {
            if let Ok(b) = u8::from_str_radix(&octal, 8) {
                bytes.push(b);
            }
            i += 4;
            continue;
        }
        let next = body[i + 1];
        match escape(next) {
            Some(b) => bytes.push(b),
            None => {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(next.encode_utf8(&mut buf).as_bytes());
            }
        }
        i += 2;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn new_commit(sha: &str) -> Commit {
    Commit {
        sha: sha.to_string(),
        short: short_sha(sha),
        uncommitted: is_uncommitted(sha),
        ..Commit::default()
    }
}

fn mail(value: &str) -> String {
    let v = value.strip_prefix('<').unwrap_or(value);
    v.strip_suffix('>').unwrap_or(v).to_string()
}

fn header(raw: &str) -> Option<(String, u32, u32)> {
    let mut parts = raw.split(' ');
    let sha = parts.next()?;
    if !is_sha(sha) {
        return None;
    }
    let orig = parts.next()?.parse().ok()?;
    let fin = parts.next()?.parse().ok()?;
    match parts.next() {
        None => {}
        Some(count) => {
            count.parse::<u32>().ok()?;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some((sha.to_string(), orig, fin))
}

struct Group {
    sha: String,
    orig: u32,
    fin: u32,
    filename: String,
}

fn apply_key(commit: &mut Commit, group: &mut Group, key: &str, value: &str) {
    match key {
        "author" => commit.author = value.into(),
        "author-mail" => commit.email = mail(value),
        "author-time" => commit.at = value.parse().unwrap_or(0),
        "author-tz" => commit.tz = value.into(),
        "committer" => commit.committer = value.into(),
        "committer-mail" => commit.committer_email = mail(value),
        "committer-time" => commit.committed_at = value.parse().unwrap_or(0),
        "summary" => commit.summary = value.into(),
        "boundary" => commit.boundary = true,
        "previous" => {
            let (sha, file) = value.split_once(' ').unwrap_or((value, ""));
            if commit.previous.is_none() && is_sha(sha) {
                commit.previous = Some(Previous {
                    sha: sha.into(),
                    file: unquote_path(file),
                });
            }
        }
        "filename" => {
            let name = unquote_path(value);
            commit.filename = name.clone();
            group.filename = name;
        }
        _ => {}
    }
}

pub fn parse_blame(text: &str, max: usize) -> Parsed {
    let max = max.max(1);
    let mut out = Parsed::default();
    let mut group: Option<Group> = None;
    for raw in text.split('\n') {
        if let Some(body) = raw.strip_prefix('\t') {
            let Some(g) = group.take() else { continue };
            if out.lines.len() >= max {
                out.truncated = true;
                continue;
            }
            let Some(c) = out.commits.get(&g.sha) else {
                continue;
            };
            out.lines.push(Line {
                n: g.fin,
                orig: g.orig,
                text: body.strip_suffix('\r').unwrap_or(body).to_string(),
                sha: c.sha.clone(),
                short: c.short.clone(),
                author: c.author.clone(),
                at: c.at,
                summary: c.summary.clone(),
                boundary: c.boundary,
                uncommitted: c.uncommitted,
                filename: if g.filename.is_empty() {
                    c.filename.clone()
                } else {
                    g.filename
                },
            });
            continue;
        }
        if let Some((sha, orig, fin)) = header(raw) {
            let commit = out
                .commits
                .entry(sha.clone())
                .or_insert_with(|| new_commit(&sha));
            group = Some(Group {
                sha,
                orig,
                fin,
                filename: commit.filename.clone(),
            });
            continue;
        }
        let Some(g) = group.as_mut() else { continue };
        if raw.is_empty() {
            continue;
        }
        let (key, value) = raw.split_once(' ').unwrap_or((raw, ""));
        if let Some(commit) = out.commits.get_mut(&g.sha) {
            apply_key(commit, g, key, value);
        }
    }
    out
}

pub fn blame_args(file: &str, rev: &str, max: usize) -> Vec<String> {
    let mut args: Vec<String> = [
        "-c",
        "core.quotepath=off",
        "--no-optional-locks",
        "blame",
        "--porcelain",
        "-w",
        "-L",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(format!("1,{}", max + 1));
    if !rev.is_empty() {
        args.push(rev.to_string());
    }
    args.push("--".into());
    args.push(file.to_string());
    args
}

fn has_any(low: &str, keys: &[&str]) -> bool {
    keys.iter().any(|k| low.contains(k))
}

fn no_such_path_in_head(low: &str) -> bool {
    low.find("no such path")
        .is_some_and(|at| low[at..].contains(" in head"))
}

pub fn blame_failure(text: &str, rev: &str) -> Option<Problem> {
    let low = text.to_lowercase();
    let with = |kind, title: &str, hint: &str| {
        Some(Problem {
            detail: text.trim().to_string(),
            ..problem(kind, title, hint)
        })
    };
    if low.contains("has only 0 lines") {
        return None;
    }
    if low.contains("could not open object name list") {
        return with(
            Kind::IgnoreRevs,
            "Git cannot read the blame.ignoreRevsFile file",
            "Your git settings name a file of commits to skip, and that file is missing. Make the file, or remove the blame.ignoreRevsFile setting.",
        );
    }
    if rev.is_empty() && has_any(&low, &["no such ref: head", "bad default revision 'head'"]) {
        return with(
            Kind::New,
            "This repo has no commits yet",
            "Make the first commit. Then you can see who changed each line.",
        );
    }
    if rev.is_empty() && no_such_path_in_head(&low) {
        return with(
            Kind::New,
            "This file has no commits yet",
            "Commit it first. Then you can see who changed each line.",
        );
    }
    if low.contains("no such path") {
        return with(
            Kind::Missing,
            "This file is not in that commit",
            "It was added later, or it had another name then.",
        );
    }
    if has_any(
        &low,
        &[
            "bad revision",
            "unknown revision",
            "no such ref",
            "needed a single revision",
            "ambiguous argument",
            "bad object",
        ],
    ) {
        return with(
            Kind::BadRev,
            "Git cannot find that commit",
            "It may be gone after a rebase. Reload to see the file as it is now.",
        );
    }
    if has_any(&low, &["cannot stat path", "no such file or directory"]) {
        return with(
            Kind::Gone,
            "The file is not in the folder",
            "It may be deleted. Try History of this file.",
        );
    }
    let said = super::super::errors::describe(text);
    let kind = if said.kind == super::super::errors::Kind::Timeout {
        Kind::Timeout
    } else {
        Kind::Other
    };
    Some(Problem {
        kind,
        title: said.title,
        hint: said.hint,
        detail: said.detail,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub first: bool,
    pub band: usize,
}

pub fn bands(lines: &[Line]) -> Vec<Band> {
    let mut run: usize = 0;
    let mut out = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let first = i == 0 || lines[i - 1].sha != line.sha;
        if first && i > 0 {
            run += 1;
        }
        out.push(Band {
            first,
            band: run % BANDS,
        });
    }
    out
}

pub fn expand_tabs(text: &str, size: usize) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut width = 0;
    for c in text.chars() {
        if c == '\t' {
            let pad = size - (width % size);
            out.extend(std::iter::repeat_n(' ', pad));
            width += pad;
        } else {
            out.push(c);
            width += 1;
        }
    }
    out
}

pub fn age_text(at: i64, now: i64) -> String {
    if at == 0 {
        return String::new();
    }
    let diff = (now - at).max(0);
    if diff < 60 {
        "now".into()
    } else if diff < 3_600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86_400 {
        format!("{}h ago", diff / 3_600)
    } else if diff < 604_800 {
        format!("{}d ago", diff / 86_400)
    } else if diff < 2_592_000 {
        format!("{}w ago", diff / 604_800)
    } else if diff < 31_536_000 {
        format!("{}mo ago", diff / 2_592_000)
    } else {
        format!("{}y ago", diff / 31_536_000)
    }
}

pub fn tz_minutes(tz: &str) -> i64 {
    let b = tz.as_bytes();
    if b.len() != 5 || !(b[0] == b'+' || b[0] == b'-') || !b[1..].iter().all(u8::is_ascii_digit) {
        return 0;
    }
    let hours: i64 = tz[1..3].parse().unwrap_or(0);
    let mins: i64 = tz[3..5].parse().unwrap_or(0);
    let sign = if b[0] == b'-' { -1 } else { 1 };
    sign * (hours * 60 + mins)
}

pub fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn format_date(at: i64, tz: &str) -> String {
    if at == 0 {
        return String::new();
    }
    let local = at + tz_minutes(tz) * 60;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    format!(
        "{d} {} {y}, {:02}:{:02}",
        MONTHS[(m - 1) as usize],
        secs / 3_600,
        (secs % 3_600) / 60
    )
}

pub fn line_number(query: &str) -> Option<u32> {
    let q = query.strip_prefix(':').unwrap_or(query);
    (!q.is_empty() && q.bytes().all(|b| b.is_ascii_digit())).then(|| q.parse().unwrap_or(u32::MAX))
}

pub fn find_line(lines: &[Line], query: &str, from: Option<usize>) -> Option<usize> {
    let q = query.trim();
    if q.is_empty() || lines.is_empty() {
        return None;
    }
    if let Some(n) = line_number(q) {
        if let Some(at) = lines.iter().position(|l| l.n == n) {
            return Some(at);
        }
        return Some(if n < 1 { 0 } else { lines.len() - 1 });
    }
    let needle = q.to_lowercase();
    let count = lines.len();
    let start = from.map_or(0, |f| f + 1);
    (0..count)
        .map(|step| (start + step) % count)
        .find(|&i| lines[i].text.to_lowercase().contains(&needle))
}

pub fn gutter_width(count: usize) -> f32 {
    let digits = count.max(1).to_string().len() as f32;
    (digits * CHAR + 10.0).max(28.0)
}

pub fn commas(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", commas(n), if n == 1 { one } else { many })
}

pub fn base_name(path: &str) -> String {
    path.split('/')
        .rfind(|p| !p.is_empty())
        .unwrap_or(path)
        .to_string()
}

pub fn commit_count(parsed: &Parsed) -> usize {
    parsed.commits.values().filter(|c| !c.uncommitted).count()
}

pub fn is_binary(parsed: &Parsed) -> bool {
    parsed.lines.iter().any(|l| l.text.contains('\0'))
}
