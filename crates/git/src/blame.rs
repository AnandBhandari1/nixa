use crate::outcome::{Outcome, failure, refusal};
use crate::run::{self, Job, Out};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

pub const MAX_LINES: usize = 20000;

pub const BAND_COLORS: [&str; 5] = ["info", "purple", "cyan", "ok", "warn"];

pub const BACKGROUND_SECS: u64 = 900;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

static HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9a-f]{40}|[0-9a-f]{64}) (\d+) (\d+)(?: (\d+))?$").expect("header"));

static SHA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$").expect("sha"));

static ZERO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^0+$").expect("zero"));

static ZERO_LINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)has only 0 lines").expect("zero lines"));

static IGNORE_REVS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)could not open object name list").expect("ignore revs"));

static NO_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)no such ref: HEAD|bad default revision 'HEAD'").expect("no head"));

static NOT_IN_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)no such path .* in HEAD").expect("not in head"));

static NO_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)no such path").expect("no path"));

static BAD_REV: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)bad revision|unknown revision|no such ref|Needed a single revision|ambiguous argument|bad object").expect("bad rev")
});

static GONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)cannot stat path|No such file or directory").expect("gone"));

static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^:?(\d+)$").expect("number"));

static TZ: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([+-])(\d{2})(\d{2})$").expect("tz"));

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Previous {
    pub sha: String,
    pub file: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameCommit {
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlameLine {
    pub n: u64,
    pub orig: u64,
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Blame {
    pub lines: Vec<BlameLine>,
    pub commits: BTreeMap<String, BlameCommit>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Band {
    pub first: bool,
    pub band: usize,
}

#[derive(Debug, Clone, Default)]
pub struct BlameOpts {
    pub rev: String,
    pub max: usize,
    pub background: bool,
    pub job: Job,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFacts {
    pub tracked: bool,
    pub in_head: bool,
}

pub fn clean_path(path: &str) -> Result<String, String> {
    crate::ops::clean_path(path)
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

pub fn is_uncommitted(sha: &str) -> bool {
    ZERO.is_match(sha)
}

pub fn short_sha(sha: &str) -> String {
    if is_uncommitted(sha) { String::new() } else { sha.chars().take(7).collect() }
}

fn escape_char(c: char) -> char {
    match c {
        'a' => '\x07',
        'b' => '\x08',
        't' => '\t',
        'n' => '\n',
        'v' => '\x0b',
        'f' => '\x0c',
        'r' => '\r',
        other => other,
    }
}

fn flush_bytes(out: &mut String, bytes: &mut Vec<u8>) {
    if bytes.is_empty() {
        return;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => out.push_str(text),
        Err(_) => out.push('\u{FFFD}'),
    }
    bytes.clear();
}

fn octal_at(body: &[char], i: usize) -> Option<u8> {
    let digits = body.get(i + 1..i + 4)?;
    if !digits.iter().all(|c| ('0'..='7').contains(c)) {
        return None;
    }
    let text: String = digits.iter().collect();
    u32::from_str_radix(&text, 8).ok().map(|n| (n & 0xff) as u8)
}

pub fn unquote_path(text: &str) -> String {
    if text.chars().count() < 2 || !text.starts_with('"') || !text.ends_with('"') {
        return text.to_string();
    }
    let body: Vec<char> = text[1..text.len() - 1].chars().collect();
    let mut out = String::new();
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let ch = body[i];
        if ch != '\\' || i + 1 >= body.len() {
            flush_bytes(&mut out, &mut bytes);
            out.push(ch);
            i += 1;
            continue;
        }
        if let Some(byte) = octal_at(&body, i) {
            bytes.push(byte);
            i += 4;
            continue;
        }
        flush_bytes(&mut out, &mut bytes);
        out.push(escape_char(body[i + 1]));
        i += 2;
    }
    flush_bytes(&mut out, &mut bytes);
    out
}

fn new_commit(sha: &str) -> BlameCommit {
    BlameCommit { sha: sha.to_string(), short: short_sha(sha), uncommitted: is_uncommitted(sha), ..BlameCommit::default() }
}

fn mail(value: &str) -> String {
    let v = value.strip_prefix('<').unwrap_or(value);
    v.strip_suffix('>').unwrap_or(v).to_string()
}

struct Group {
    sha: String,
    orig: u64,
    fin: u64,
    filename: String,
}

fn apply_key(commit: &mut BlameCommit, group: &mut Group, key: &str, value: &str) {
    match key {
        "author" => commit.author = value.to_string(),
        "author-mail" => commit.email = mail(value),
        "author-time" => commit.at = value.parse().unwrap_or(0),
        "author-tz" => commit.tz = value.to_string(),
        "committer" => commit.committer = value.to_string(),
        "committer-mail" => commit.committer_email = mail(value),
        "committer-time" => commit.committed_at = value.parse().unwrap_or(0),
        "summary" => commit.summary = value.to_string(),
        "boundary" => commit.boundary = true,
        "previous" => {
            let (sha, file) = match value.find(' ') {
                Some(space) => (&value[..space], unquote_path(&value[space + 1..])),
                None => (value, String::new()),
            };
            if commit.previous.is_none() && SHA.is_match(sha) {
                commit.previous = Some(Previous { sha: sha.to_string(), file });
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

fn line_of(group: &Group, commit: &BlameCommit, raw: &str) -> BlameLine {
    let text = raw[1..].strip_suffix('\r').unwrap_or(&raw[1..]).to_string();
    BlameLine {
        n: group.fin,
        orig: group.orig,
        text,
        sha: commit.sha.clone(),
        short: commit.short.clone(),
        author: commit.author.clone(),
        at: commit.at,
        summary: commit.summary.clone(),
        boundary: commit.boundary,
        uncommitted: commit.uncommitted,
        filename: if group.filename.is_empty() { commit.filename.clone() } else { group.filename.clone() },
    }
}

pub fn parse_blame(text: &str, max: usize) -> Blame {
    let max = if max == 0 { MAX_LINES } else { max };
    let mut commits: BTreeMap<String, BlameCommit> = BTreeMap::new();
    let mut lines = Vec::new();
    let mut group: Option<Group> = None;
    let mut truncated = false;
    for raw in text.split('\n') {
        if raw.starts_with('\t') {
            let Some(g) = group.take() else { continue };
            if lines.len() >= max {
                truncated = true;
                continue;
            }
            if let Some(commit) = commits.get(&g.sha) {
                lines.push(line_of(&g, commit, raw));
            }
            continue;
        }
        if let Some(head) = HEADER.captures(raw) {
            let sha = head[1].to_string();
            let commit = commits.entry(sha.clone()).or_insert_with(|| new_commit(&sha));
            group = Some(Group {
                sha,
                orig: head[2].parse().unwrap_or(0),
                fin: head[3].parse().unwrap_or(0),
                filename: commit.filename.clone(),
            });
            continue;
        }
        let Some(g) = group.as_mut() else { continue };
        if raw.is_empty() {
            continue;
        }
        let (key, value) = match raw.find(' ') {
            Some(space) => (&raw[..space], &raw[space + 1..]),
            None => (raw, ""),
        };
        if let Some(commit) = commits.get_mut(&g.sha) {
            apply_key(commit, g, key, value);
        }
    }
    Blame { lines, commits, truncated }
}

pub fn blame_args(file: &str, rev: &str, max: usize) -> Vec<String> {
    let mut args: Vec<String> = ["-c", "core.quotepath=off", "--no-optional-locks", "blame", "--porcelain", "-w", "-L"]
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

fn merge_extra(mut out: Outcome, extra: &Map<String, Value>) -> Outcome {
    for (key, value) in extra {
        out.extra.insert(key.clone(), value.clone());
    }
    out
}

fn base_extra(file: &str, rev: &str, blame: &Blame) -> Map<String, Value> {
    let mut extra = Map::new();
    extra.insert("lines".into(), serde_json::to_value(&blame.lines).unwrap_or(Value::Null));
    extra.insert("commits".into(), serde_json::to_value(&blame.commits).unwrap_or(Value::Null));
    extra.insert("truncated".into(), blame.truncated.into());
    extra.insert("file".into(), file.into());
    extra.insert("rev".into(), if rev.is_empty() { Value::Null } else { rev.into() });
    extra
}

fn blame_failure(text: &str, rev: &str, extra: &Map<String, Value>) -> Option<Outcome> {
    if ZERO_LINES.is_match(text) {
        return None;
    }
    let problem = |kind: &str, title: &str, hint: &str| Some(merge_extra(refusal(kind, title, hint).stderr(text), extra));
    if IGNORE_REVS.is_match(text) {
        return problem(
            "ignoreRevs",
            "Git cannot read the blame.ignoreRevsFile file",
            "Your git settings name a file of commits to skip, and that file is missing. Make the file, or remove the blame.ignoreRevsFile setting.",
        );
    }
    if NO_HEAD.is_match(text) && rev.is_empty() {
        return problem("new", "This repo has no commits yet", "Make the first commit. Then you can see who changed each line.");
    }
    if NOT_IN_HEAD.is_match(text) && rev.is_empty() {
        return problem("new", "This file has no commits yet", "Commit it first. Then you can see who changed each line.");
    }
    if NO_PATH.is_match(text) {
        return problem("missing", "This file is not in that commit", "It was added later, or it had another name then.");
    }
    if BAD_REV.is_match(text) {
        return problem("badRev", "Git cannot find that commit", "It may be gone after a rebase. Reload to see the file as it is now.");
    }
    if GONE.is_match(text) {
        return problem("gone", "The file is not in the folder", "It may be deleted. Try History of this file.");
    }
    Some(merge_extra(failure(text), extra))
}

fn finish(stdout: &str, file: &str, rev: &str, max: usize) -> Outcome {
    let parsed = parse_blame(stdout, max);
    if parsed.lines.iter().any(|line| line.text.contains('\0')) {
        let none = Blame { truncated: parsed.truncated, ..Blame::default() };
        return merge_extra(
            refusal("binary", "This is a binary file", "Only text files show who changed each line."),
            &base_extra(file, rev, &none),
        );
    }
    merge_extra(Outcome { ok: true, ..Outcome::default() }, &base_extra(file, rev, &parsed))
}

fn run_blame(root: &Path, args: &[&str], opts: &BlameOpts) -> Out {
    if !opts.background {
        return run::git(root, args);
    }
    let job = Job { timeout: opts.job.timeout.or(Some(Duration::from_secs(BACKGROUND_SECS))), ..opts.job.clone() };
    run::job(root, args, &job)
}

pub fn blame_file(root: &Path, file: &str, opts: &BlameOpts) -> Outcome {
    let checked = clean_path(file).and_then(|p| clean_rev(&opts.rev).map(|r| (p, r)));
    let (p, rev) = match checked {
        Ok(pair) => pair,
        Err(error) => {
            return merge_extra(refusal("bad", "Cannot show this file", &error), &base_extra(file, &opts.rev, &Blame::default()));
        }
    };
    let max = if opts.max == 0 { MAX_LINES } else { opts.max.min(MAX_LINES) };
    let extra = base_extra(&p, &rev, &Blame::default());
    let args = blame_args(&p, &rev, max);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let r = run_blame(root, &refs, opts);
    if r.cancelled {
        return merge_extra(refusal("cancelled", "Stopped", ""), &extra).with("cancelled", true.into());
    }
    if r.timed_out {
        return merge_extra(failure("timed out waiting"), &extra).with("timedOut", true.into());
    }
    if !r.ok() {
        let text = r.said();
        let text = if text.is_empty() { format!("git blame failed ({})", r.code.map(|c| c.to_string()).unwrap_or_default()) } else { text };
        return blame_failure(&text, &rev, &extra).unwrap_or_else(|| merge_extra(Outcome { ok: true, ..Outcome::default() }, &extra));
    }
    finish(&r.stdout, &p, &rev, max)
}

pub fn blame_of(out: &Outcome) -> Blame {
    let field = |key: &str| out.extra.get(key).cloned().unwrap_or(Value::Null);
    Blame {
        lines: serde_json::from_value(field("lines")).unwrap_or_default(),
        commits: serde_json::from_value(field("commits")).unwrap_or_default(),
        truncated: out.flag("truncated"),
    }
}

pub fn blame_bands(lines: &[BlameLine], colors: usize) -> Vec<Band> {
    let colors = if colors == 0 { BAND_COLORS.len() } else { colors };
    let mut out = Vec::with_capacity(lines.len());
    let mut run = 0usize;
    let mut last: Option<&str> = None;
    for (i, line) in lines.iter().enumerate() {
        let first = i == 0 || last != Some(line.sha.as_str());
        if first && i > 0 {
            run += 1;
        }
        last = Some(line.sha.as_str());
        out.push(Band { first, band: run % colors });
    }
    out
}

pub fn expand_tabs(text: &str, size: usize) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let size = size.max(1);
    let mut out = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        if ch == '\t' {
            let n = size - (col % size);
            out.push_str(&" ".repeat(n));
            col += n;
        } else {
            out.push(ch);
            col += ch.len_utf16();
        }
    }
    out
}

pub fn age_text(at: i64, now: i64) -> String {
    if at == 0 {
        return String::new();
    }
    let diff = (now - at).max(0);
    match diff {
        d if d < 60 => "now".into(),
        d if d < 3600 => format!("{}m ago", d / 60),
        d if d < 86400 => format!("{}h ago", d / 3600),
        d if d < 604800 => format!("{}d ago", d / 86400),
        d if d < 2592000 => format!("{}w ago", d / 604800),
        d if d < 31536000 => format!("{}mo ago", d / 2592000),
        d => format!("{}y ago", d / 31536000),
    }
}

fn tz_minutes(tz: &str) -> i64 {
    let Some(m) = TZ.captures(tz) else { return 0 };
    let sign = if &m[1] == "-" { -1 } else { 1 };
    sign * (m[2].parse::<i64>().unwrap_or(0) * 60 + m[3].parse::<i64>().unwrap_or(0))
}

fn civil_from_days(days: i64) -> (i64, usize, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, (month - 1) as usize, day)
}

pub fn format_date(at: i64, tz: &str) -> String {
    if at == 0 {
        return String::new();
    }
    let local = at + tz_minutes(tz) * 60;
    let days = local.div_euclid(86400);
    let secs = local.rem_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    format!("{day} {} {year}, {:02}:{:02}", MONTHS[month], secs / 3600, (secs % 3600) / 60)
}

pub fn find_line(lines: &[BlameLine], query: &str, from: Option<usize>) -> Option<usize> {
    let q = query.trim();
    if q.is_empty() || lines.is_empty() {
        return None;
    }
    if let Some(m) = NUMBER.captures(q) {
        let n: u64 = m[1].parse().unwrap_or(u64::MAX);
        if let Some(at) = lines.iter().position(|line| line.n == n) {
            return Some(at);
        }
        return Some(if n < 1 { 0 } else { lines.len() - 1 });
    }
    let needle = q.to_lowercase();
    let len = lines.len();
    let start = from.map(|f| f % len + 1).unwrap_or(0);
    (0..len).map(|step| (start + step) % len).find(|&i| lines[i].text.to_lowercase().contains(&needle))
}

pub fn exact_pattern(path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains(['\r', '\n', '\0']) {
        return Err("this name cannot go in an ignore file".into());
    }
    let mut body = String::new();
    for ch in path.trim_start_matches('/').chars() {
        if matches!(ch, '\\' | '*' | '?' | '[') {
            body.push('\\');
        }
        body.push(ch);
    }
    let kept = body.trim_end_matches(' ');
    let spaces = body.len() - kept.len();
    Ok(format!("/{kept}{}", "\\ ".repeat(spaces)))
}

pub fn exclude_path(root: &Path) -> Result<PathBuf, String> {
    let first = run::git(root, &["rev-parse", "--path-format=absolute", "--git-path", "info/exclude"]);
    let text = if first.ok() { first.stdout.trim().to_string() } else { String::new() };
    if text.starts_with('/') {
        return Ok(PathBuf::from(text));
    }
    let second = if first.ok() { first } else { run::git(root, &["rev-parse", "--git-path", "info/exclude"]) };
    if !second.ok() {
        let said = second.said();
        return Err(if said.is_empty() { "not a git repository".into() } else { said });
    }
    let rel = second.stdout.trim();
    if rel.is_empty() {
        return Err("git did not say where the exclude file is".into());
    }
    if rel.starts_with('/') {
        return Ok(PathBuf::from(rel));
    }
    Ok(root.join(rel.strip_prefix("./").unwrap_or(rel)))
}

fn append_line(path: &Path, line: &str) -> Result<bool, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    }
    if path.is_file() {
        let old = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let with_cr = format!("{line}\r");
        if old.split(|b| *b == b'\n').any(|l| l == line.as_bytes() || l == with_cr.as_bytes()) {
            return Ok(false);
        }
        if !old.is_empty() && old.last() != Some(&b'\n') {
            write_more(path, "\n")?;
        }
    }
    write_more(path, &format!("{line}\n"))?;
    Ok(true)
}

fn write_more(path: &Path, text: &str) -> Result<(), String> {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    f.write_all(text.as_bytes()).map_err(|e| format!("could not write {}: {e}", path.display()))
}

pub fn exclude_locally(root: &Path, pattern: &str) -> Outcome {
    if pattern.trim().is_empty() || pattern.contains(['\r', '\n', '\0']) {
        return refusal("bad", "This cannot be ignored", "The name is empty or has a line break in it.");
    }
    let path = match exclude_path(root) {
        Ok(path) => path,
        Err(error) => return failure(&error),
    };
    let shown = path.to_string_lossy().into_owned();
    let added = match append_line(&path, pattern) {
        Ok(added) => added,
        Err(error) => return failure(&error).with("path", shown.into()),
    };
    Outcome {
        ok: true,
        title: if added { "Ignored on this computer" } else { "Already ignored on this computer" }.into(),
        hint: "Only this copy of the repo changed. No shared file changed.".into(),
        ..Outcome::default()
    }
    .with("added", added.into())
    .with("path", shown.into())
    .with("pattern", pattern.into())
}

pub fn file_facts(root: &Path, file: &str) -> Result<FileFacts, String> {
    let p = clean_path(file)?;
    let spec = format!("HEAD:{p}");
    let index = run::read(root, &["--literal-pathspecs", "ls-files", "-z", "--", &p]);
    let head = run::read(root, &["cat-file", "-e", &spec]);
    Ok(FileFacts { tracked: index.ok() && !index.stdout.is_empty(), in_head: head.ok() })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrepo;

    fn line(sha: &str, n: u64, text: &str) -> BlameLine {
        BlameLine { sha: sha.into(), n, text: text.into(), ..BlameLine::default() }
    }

    #[test]
    fn blames_a_real_file_with_uncommitted_lines() {
        let repo = testrepo::new();
        let second = repo.commit_file("a.txt", "one\nTWO\nthree\n", "change two");
        repo.write("a.txt", "one\nTWO\nthree\nfour\n");
        let out = blame_file(repo.root(), "a.txt", &BlameOpts::default());
        assert!(out.ok, "{out:?}");
        assert_eq!(out.text("file"), Some("a.txt"));
        assert_eq!(out.get("rev"), Some(&Value::Null));
        let blame = blame_of(&out);
        assert_eq!(blame.lines.len(), 4);
        assert!(!blame.truncated);
        assert_eq!(blame.lines[1].sha, second);
        assert_eq!(blame.lines[1].short, &second[..7]);
        assert_eq!(blame.lines[1].summary, "change two");
        assert_eq!(blame.lines[1].author, "Test User");
        assert_eq!(blame.lines[0].summary, "first");
        assert!(blame.lines[0].boundary || !blame.lines[0].uncommitted);
        assert_eq!(blame.lines[0].filename, "a.txt");
        let last = &blame.lines[3];
        assert!(last.uncommitted);
        assert_eq!(last.short, "");
        assert_eq!(last.text, "four");
        assert_eq!(last.n, 4);
        let commit = &blame.commits[&second];
        assert_eq!(commit.email, "test@example.com");
        assert!(commit.at > 0);
        assert!(commit.previous.as_ref().is_some_and(|p| p.file == "a.txt"));
        let old = blame_file(repo.root(), "a.txt", &BlameOpts { rev: "HEAD~1".into(), ..BlameOpts::default() });
        assert!(old.ok);
        assert_eq!(old.text("rev"), Some("HEAD~1"));
        assert_eq!(blame_of(&old).lines[1].text, "two");
        let bg = blame_file(repo.root(), "a.txt", &BlameOpts { background: true, max: 2, ..BlameOpts::default() });
        assert!(bg.ok);
        let short = blame_of(&bg);
        assert_eq!(short.lines.len(), 2);
        assert!(short.truncated);
    }

    #[test]
    fn blame_refusals() {
        let repo = testrepo::new();
        assert_eq!(blame_file(repo.root(), "../x", &BlameOpts::default()).kind, "bad");
        assert_eq!(blame_file(repo.root(), "a.txt", &BlameOpts { rev: "-x".into(), ..BlameOpts::default() }).kind, "bad");
        repo.write("new.txt", "hi\n");
        let fresh = blame_file(repo.root(), "new.txt", &BlameOpts::default());
        assert_eq!(fresh.kind, "new", "{fresh:?}");
        assert_eq!(fresh.title, "This file has no commits yet");
        let bad = blame_file(repo.root(), "a.txt", &BlameOpts { rev: "nope".into(), ..BlameOpts::default() });
        assert_eq!(bad.kind, "badRev", "{bad:?}");
        let missing = blame_file(repo.root(), "new.txt", &BlameOpts { rev: "HEAD".into(), ..BlameOpts::default() });
        assert_eq!(missing.kind, "missing", "{missing:?}");
        repo.commit_file("bin.dat", "a\0b\n", "binary");
        let bin = blame_file(repo.root(), "bin.dat", &BlameOpts::default());
        assert_eq!(bin.kind, "binary");
        assert!(blame_of(&bin).lines.is_empty());
        repo.commit_file("empty.txt", "", "empty");
        let none = blame_file(repo.root(), "empty.txt", &BlameOpts::default());
        assert!(none.ok, "{none:?}");
        assert!(blame_of(&none).lines.is_empty());
        let empty = testrepo::empty();
        empty.write("x.txt", "x\n");
        let first = blame_file(empty.root(), "x.txt", &BlameOpts::default());
        assert!(!first.ok);
        assert_eq!(first.kind, "new", "{first:?}");
    }

    #[test]
    fn parses_porcelain_text() {
        let sha = "a".repeat(40);
        let zero = "0".repeat(40);
        let text = format!(
            "{sha} 1 1 2\nauthor Ann\nauthor-mail <ann@x.org>\nauthor-time 100\nauthor-tz +0100\nsummary hello\nprevious {} \"caf\\303\\251.txt\"\nfilename \"caf\\303\\251.txt\"\n\tline one\r\n{sha} 2 2\n\tline two\n{zero} 3 3 1\nauthor Not Committed Yet\nfilename x\n\tthree\n",
            "b".repeat(40)
        );
        let all = parse_blame(&text, 0);
        assert_eq!(all.lines.len(), 3);
        assert_eq!(all.lines[0].text, "line one");
        assert_eq!(all.lines[0].filename, "café.txt");
        assert_eq!(all.lines[1].author, "Ann");
        assert_eq!(all.lines[1].filename, "café.txt");
        assert!(all.lines[2].uncommitted);
        let c = &all.commits[&sha];
        assert_eq!(c.email, "ann@x.org");
        assert_eq!(c.at, 100);
        assert_eq!(c.tz, "+0100");
        assert_eq!(c.previous.as_ref().unwrap().file, "café.txt");
        let cut = parse_blame(&text, 2);
        assert_eq!(cut.lines.len(), 2);
        assert!(cut.truncated);
        assert_eq!(blame_args("a.txt", "", 10)[7], "1,11");
        assert_eq!(blame_args("a.txt", "HEAD", 10)[8], "HEAD");
    }

    #[test]
    fn small_helpers() {
        assert_eq!(unquote_path("plain"), "plain");
        assert_eq!(unquote_path("\"a\\tb\\\\c\\\"d\""), "a\tb\\c\"d");
        assert_eq!(unquote_path("\"\\303\""), "\u{FFFD}");
        assert!(is_uncommitted("0000000"));
        assert!(!is_uncommitted(""));
        assert_eq!(short_sha("0000000000"), "");
        assert_eq!(short_sha("abcdef123456"), "abcdef1");
        assert_eq!(clean_rev(" HEAD~2 "), Ok("HEAD~2".into()));
        assert_eq!(clean_rev(""), Ok(String::new()));
        assert!(clean_rev("a..b").is_err());
        assert!(clean_rev("a b").is_err());
        assert_eq!(expand_tabs("a\tb", 4), "a   b");
        assert_eq!(expand_tabs("\t\tx", 2), "    x");
        assert_eq!(expand_tabs("none", 4), "none");
    }

    #[test]
    fn bands_follow_runs() {
        let lines = vec![line("a", 1, ""), line("a", 2, ""), line("b", 3, ""), line("a", 4, "")];
        let bands = blame_bands(&lines, 0);
        assert_eq!(bands.iter().map(|b| b.first).collect::<Vec<_>>(), vec![true, false, true, true]);
        assert_eq!(bands.iter().map(|b| b.band).collect::<Vec<_>>(), vec![0, 0, 1, 2]);
        assert_eq!(blame_bands(&lines, 2)[3].band, 0);
    }

    #[test]
    fn dates_and_ages() {
        assert_eq!(age_text(0, 100), "");
        assert_eq!(age_text(100, 130), "now");
        assert_eq!(age_text(100, 100 + 125), "2m ago");
        assert_eq!(age_text(100, 100 + 7200), "2h ago");
        assert_eq!(age_text(100, 100 + 3 * 86400), "3d ago");
        assert_eq!(age_text(100, 100 + 15 * 86400), "2w ago");
        assert_eq!(age_text(100, 100 + 70 * 86400), "2mo ago");
        assert_eq!(age_text(100, 100 + 800 * 86400), "2y ago");
        assert_eq!(age_text(500, 100), "now");
        assert_eq!(format_date(1700000000, ""), "14 Nov 2023, 22:13");
        assert_eq!(format_date(1700000000, "+0530"), "15 Nov 2023, 03:43");
        assert_eq!(format_date(1700000000, "-0800"), "14 Nov 2023, 14:13");
        assert_eq!(format_date(951782400, "+0000"), "29 Feb 2000, 00:00");
        assert_eq!(format_date(0, "+0100"), "");
    }

    #[test]
    fn finds_lines() {
        let lines = vec![line("a", 1, "Alpha"), line("a", 2, "beta"), line("b", 3, "alphabet")];
        assert_eq!(find_line(&lines, "2", None), Some(1));
        assert_eq!(find_line(&lines, ":3", None), Some(2));
        assert_eq!(find_line(&lines, "0", None), Some(0));
        assert_eq!(find_line(&lines, "99", None), Some(2));
        assert_eq!(find_line(&lines, "ALPHA", None), Some(0));
        assert_eq!(find_line(&lines, "alpha", Some(0)), Some(2));
        assert_eq!(find_line(&lines, "alpha", Some(2)), Some(0));
        assert_eq!(find_line(&lines, "zeta", None), None);
        assert_eq!(find_line(&lines, " ", None), None);
        assert_eq!(find_line(&[], "1", None), None);
    }

    #[test]
    fn exact_patterns() {
        assert_eq!(exact_pattern("dir/a.txt"), Ok("/dir/a.txt".into()));
        assert_eq!(exact_pattern("//a*b?[c]\\d  "), Ok("/a\\*b\\?\\[c]\\\\d\\ \\ ".into()));
        assert!(exact_pattern("a\nb").is_err());
        assert!(exact_pattern("").is_err());
    }

    #[test]
    fn excludes_locally_in_the_temp_repo() {
        let repo = testrepo::new();
        let path = exclude_path(repo.root()).unwrap();
        assert!(path.starts_with(repo.root().canonicalize().unwrap()) || path.starts_with(repo.root()));
        assert!(path.ends_with(".git/info/exclude"));
        std::fs::write(&path, "# local\nold").unwrap();
        repo.write("secret.txt", "x\n");
        let pattern = exact_pattern("secret.txt").unwrap();
        let first = exclude_locally(repo.root(), &pattern);
        assert!(first.ok, "{first:?}");
        assert!(first.flag("added"));
        assert_eq!(first.title, "Ignored on this computer");
        assert_eq!(first.text("pattern"), Some("/secret.txt"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# local\nold\n/secret.txt\n");
        let again = exclude_locally(repo.root(), &pattern);
        assert!(again.ok);
        assert!(!again.flag("added"));
        assert_eq!(again.title, "Already ignored on this computer");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# local\nold\n/secret.txt\n");
        assert_eq!(repo.git(&["status", "--porcelain"]), "");
        assert_eq!(exclude_locally(repo.root(), "a\nb").kind, "bad");
        assert_eq!(exclude_locally(repo.root(), "  ").kind, "bad");
        std::fs::remove_file(&path).unwrap();
        assert!(exclude_locally(repo.root(), "/x").flag("added"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "/x\n");
    }

    #[test]
    fn file_facts_for_tracked_staged_and_new_files() {
        let repo = testrepo::new();
        assert_eq!(file_facts(repo.root(), "a.txt"), Ok(FileFacts { tracked: true, in_head: true }));
        repo.write("b.txt", "b\n");
        assert_eq!(file_facts(repo.root(), "b.txt"), Ok(FileFacts { tracked: false, in_head: false }));
        repo.git(&["add", "b.txt"]);
        assert_eq!(file_facts(repo.root(), "b.txt"), Ok(FileFacts { tracked: true, in_head: false }));
        assert!(file_facts(repo.root(), "/etc/passwd").is_err());
    }
}
