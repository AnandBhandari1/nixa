use super::run::{self, read_ok, split_z};
use super::words::{Outcome, failure, refusal, success};
use std::path::Path;

pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stash {
    pub reference: String,
    pub index: usize,
    pub sha: String,
    pub message: String,
    pub branch: Option<String>,
    pub wip: bool,
    pub untracked: bool,
    pub at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StashFile {
    pub path: String,
    pub status: char,
    pub untracked: bool,
}

pub fn parse_subject(subject: &str) -> (Option<String>, bool, String) {
    for (lead, wip) in [("WIP on ", true), ("On ", false)] {
        if let Some(rest) = subject.strip_prefix(lead)
            && let Some((branch, message)) = rest.split_once(": ")
        {
            return (Some(branch.to_string()), wip, message.to_string());
        }
    }
    (None, false, subject.to_string())
}

pub fn parse_list(text: &str) -> Vec<Stash> {
    text.lines()
        .filter(|l| !l.is_empty())
        .enumerate()
        .map(|(i, line)| {
            let f: Vec<&str> = line.splitn(5, '\0').collect();
            let get = |k: usize| f.get(k).copied().unwrap_or_default();
            let reference = get(0).to_string();
            let index = reference
                .split_once('{')
                .and_then(|(_, r)| r.trim_end_matches('}').parse().ok())
                .unwrap_or(i);
            let (branch, wip, message) = parse_subject(get(4));
            Stash {
                index,
                sha: get(1).to_string(),
                untracked: get(2).split_whitespace().count() > 2,
                at: get(3).parse().unwrap_or(0),
                branch,
                wip,
                message,
                reference,
            }
        })
        .collect()
}

pub fn list(root: &Path) -> Result<Vec<Stash>, String> {
    Ok(parse_list(&read_ok(root, &["stash", "list", "--format=%gd%x00%H%x00%P%x00%ct%x00%gs"])?))
}

pub fn stash_ref(r: &str) -> Result<String, String> {
    let is_digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if let Some(inner) = r.strip_prefix("stash@{").and_then(|x| x.strip_suffix('}'))
        && is_digits(inner)
    {
        return Ok(r.to_string());
    }
    if (7..=64).contains(&r.len()) && r.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
        return Ok(r.to_string());
    }
    if is_digits(r) {
        return Ok(format!("stash@{{{r}}}"));
    }
    Err(format!("not a stash: {r}"))
}

pub fn push(root: &Path, message: &str, include_untracked: bool, keep_index: bool) -> Outcome {
    let message = message.split('\n').map(str::trim).collect::<Vec<_>>().join(" ").trim().to_string();
    let flag = format!("--message={message}");
    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("--include-untracked");
    }
    if keep_index {
        args.push("--keep-index");
    }
    if !message.is_empty() {
        args.push(&flag);
    }
    let r = run::git(root, &args);
    if !r.ok() {
        return failure(&r.said());
    }
    if r.said().contains("No local changes to save") {
        return refusal("nothing", "Nothing to stash", "There are no changes to put away.");
    }
    success("Changes stashed")
}

fn moved(root: &Path, target: &str, sha: Option<&str>) -> Option<Outcome> {
    let want = sha?;
    let r = run::git(root, &["rev-parse", "-q", "--verify", "--end-of-options", target]);
    if r.ok() && r.line() == want {
        return None;
    }
    Some(refusal(
        "stashMoved",
        "The stash list changed",
        "Nothing was changed. The list was read again. Pick the stash again.",
    ))
}

pub fn apply(root: &Path, r: &str, pop: bool, index: bool, sha: Option<&str>) -> Outcome {
    let target = match stash_ref(r) {
        Ok(t) => t,
        Err(e) => return failure(&e),
    };
    if let Some(o) = moved(root, &target, sha) {
        return o;
    }
    let mut args = vec!["stash", if pop { "pop" } else { "apply" }];
    if index {
        args.push("--index");
    }
    args.extend(["--end-of-options", &target]);
    let out = run::git(root, &args);
    if out.ok() {
        return success(if pop { "Stash applied and removed" } else { "Stash applied" });
    }
    let mut o = failure(&out.said());
    if o.kind == "conflict" {
        o.title = "The stash was applied with conflicts".into();
        o.hint = "Resolve the files. The stash was kept.".into();
    }
    o
}

pub fn drop(root: &Path, r: &str, sha: Option<&str>) -> Outcome {
    let target = match stash_ref(r) {
        Ok(t) => t,
        Err(e) => return failure(&e),
    };
    if let Some(o) = moved(root, &target, sha) {
        return o;
    }
    let out = run::git(root, &["stash", "drop", "--end-of-options", &target]);
    if !out.ok() {
        return failure(&out.said());
    }
    success("Stash dropped")
}

fn has_untracked_part(root: &Path, target: &str) -> bool {
    run::git(root, &["rev-parse", "-q", "--verify", &format!("{target}^3")]).ok()
}

pub fn files(root: &Path, r: &str) -> Result<Vec<StashFile>, String> {
    let target = stash_ref(r)?;
    let parent = format!("{target}^1");
    let text = read_ok(root, &["diff-tree", "-r", "--no-commit-id", "--name-status", "-z", "--no-renames", &parent, &target])?;
    let parts = split_z(&text);
    let mut out: Vec<StashFile> = parts
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| StashFile {
            path: c[1].clone(),
            status: c[0].chars().next().unwrap_or('M'),
            untracked: false,
        })
        .collect();
    if has_untracked_part(root, &target) {
        let third = format!("{target}^3");
        let extra = read_ok(root, &["ls-tree", "-r", "--name-only", "-z", &third])?;
        out.extend(split_z(&extra).into_iter().map(|path| StashFile {
            path,
            status: '?',
            untracked: true,
        }));
    }
    Ok(out)
}

pub fn clean_path(p: &str) -> Result<&str, String> {
    if p.is_empty() || p.contains('\0') || p.starts_with('/') || p.split('/').any(|x| x == "..") {
        return Err(format!("not a repo file path: {p}"));
    }
    Ok(p)
}

pub fn diff(root: &Path, r: &str, path: &str, untracked: Option<bool>) -> Result<String, String> {
    let target = stash_ref(r)?;
    let p = clean_path(path)?;
    let flags = ["--literal-pathspecs", "diff", "--no-color", "--no-ext-diff", "--no-textconv"];
    if untracked != Some(true) {
        let parent = format!("{target}^1");
        let mut args = flags.to_vec();
        args.extend([parent.as_str(), target.as_str(), "--", p]);
        let text = read_ok(root, &args)?;
        if !text.trim().is_empty() || untracked == Some(false) {
            return Ok(text);
        }
    }
    if !has_untracked_part(root, &target) {
        return Ok(String::new());
    }
    let third = format!("{target}^3");
    let mut args = flags.to_vec();
    args.extend([EMPTY_TREE, third.as_str(), "--", p]);
    read_ok(root, &args)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Hunk,
    Add,
    Del,
    Same,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    pub kind: DiffKind,
    pub number: Option<usize>,
    pub text: String,
}

pub const MAX_LINES: usize = 2000;

fn hunk_starts(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(' ')?;
    let new = rest.strip_prefix('+')?;
    let num = |s: &str| s.split([',', ' ']).next().and_then(|n| n.parse::<usize>().ok());
    Some((num(old)?, num(new)?))
}

pub fn diff_rows(text: &str) -> Vec<DiffRow> {
    let trimmed = text.strip_suffix('\n').unwrap_or(text);
    let all: Vec<&str> = trimmed.split('\n').collect();
    let Some(first) = all.iter().position(|l| l.starts_with("@@")) else {
        return Vec::new();
    };
    let (mut old, mut new) = (0usize, 0usize);
    all[first..]
        .iter()
        .take(MAX_LINES)
        .map(|raw| {
            let line = if raw.chars().count() > 2000 {
                format!("{}…", raw.chars().take(2000).collect::<String>())
            } else {
                raw.to_string()
            };
            if let Some((o, n)) = hunk_starts(&line) {
                old = o;
                new = n;
                return DiffRow { kind: DiffKind::Hunk, number: None, text: line };
            }
            let body = |l: &str| {
                let b: String = l.chars().skip(1).collect();
                if b.is_empty() { " ".to_string() } else { b }
            };
            match line.chars().next() {
                Some('-') => {
                    old += 1;
                    DiffRow { kind: DiffKind::Del, number: Some(old - 1), text: body(&line) }
                }
                Some('+') => {
                    new += 1;
                    DiffRow { kind: DiffKind::Add, number: Some(new - 1), text: body(&line) }
                }
                Some(' ') => {
                    old += 1;
                    new += 1;
                    DiffRow { kind: DiffKind::Same, number: Some(new - 1), text: body(&line) }
                }
                _ => DiffRow { kind: DiffKind::Other, number: None, text: line },
            }
        })
        .collect()
}

pub fn status_word(status: char, untracked: bool) -> &'static str {
    if untracked {
        return "new file";
    }
    match status {
        'M' => "changed",
        'A' => "added",
        'D' => "deleted",
        'T' => "type changed",
        '?' => "new file",
        _ => "changed",
    }
}
