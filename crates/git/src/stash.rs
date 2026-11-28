use crate::ops::{EMPTY_TREE, clean_path, split_z, stash_ref};
use crate::outcome::{Outcome, failure, refusal, success};
use crate::run::{git, read_ok};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stash {
    pub reference: String,
    pub index: usize,
    pub sha: String,
    pub message: String,
    pub branch: Option<String>,
    pub wip: bool,
    pub untracked: bool,
    pub at: i64,
    pub subject: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashPush {
    pub message: String,
    pub include_untracked: bool,
    pub keep_index: bool,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashUse {
    pub index: bool,
    pub sha: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashFile {
    pub path: String,
    pub status: String,
    pub untracked: bool,
}

pub fn parse_stashes(text: &str) -> Vec<Stash> {
    static SUBJECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^(WIP on|On) (.*?): (.*)$").expect("stash subject"));
    static INDEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(\d+)\}").expect("stash index"));
    text.split('\n')
        .filter(|l| !l.is_empty())
        .enumerate()
        .map(|(i, line)| {
            let f: Vec<&str> = line.split('\0').collect();
            let get = |k: usize| f.get(k).copied().unwrap_or("");
            let reference = get(0).to_string();
            let subject = f.get(4..).map(|s| s.join("\0")).unwrap_or_default();
            let m = SUBJECT.captures(&subject);
            Stash {
                index: INDEX.captures(&reference).and_then(|c| c[1].parse().ok()).unwrap_or(i),
                sha: get(1).to_string(),
                message: m.as_ref().map(|c| c[3].to_string()).unwrap_or_else(|| subject.clone()),
                branch: m.as_ref().map(|c| c[2].to_string()),
                wip: m.as_ref().is_some_and(|c| &c[1] == "WIP on"),
                untracked: get(2).split(' ').filter(|p| !p.is_empty()).count() > 2,
                at: get(3).parse().unwrap_or(0),
                reference,
                subject,
            }
        })
        .collect()
}

pub fn list_stashes(root: &Path) -> Result<Vec<Stash>, String> {
    let text = read_ok(root, &["stash", "list", "--format=%gd%x00%H%x00%P%x00%ct%x00%gs"])?;
    Ok(parse_stashes(&text))
}

pub fn stash_push(root: &Path, opts: &StashPush) -> Outcome {
    static GAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\n\s*").expect("gap"));
    let mut paths = Vec::new();
    for p in &opts.paths {
        match clean_path(p) {
            Ok(p) => paths.push(format!(":(literal){p}")),
            Err(e) => return failure(&e),
        }
    }
    let message = GAP.replace_all(&opts.message, " ").trim().to_string();
    let flag = format!("--message={message}");
    let mut args = vec!["stash", "push"];
    if opts.include_untracked {
        args.push("--include-untracked");
    }
    if opts.keep_index {
        args.push("--keep-index");
    }
    if !message.is_empty() {
        args.push(&flag);
    }
    if !paths.is_empty() {
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
    }
    let out = git(root, &args);
    if !out.ok() {
        return failure(&out.said());
    }
    if out.said().to_lowercase().contains("no local changes to save") {
        return refusal("nothing", "Nothing to stash", "There are no changes to put away.");
    }
    success("Changes stashed").with("ref", "stash@{0}".into())
}

fn stash_moved(root: &Path, target: &str, sha: &Option<String>) -> Option<Outcome> {
    let sha = sha.as_ref()?;
    let out = git(root, &["rev-parse", "-q", "--verify", "--end-of-options", target]);
    if out.ok() && out.stdout.trim() == sha {
        return None;
    }
    Some(
        refusal("stashMoved", "The stash list changed", "Nothing was changed. The list was read again. Pick the stash again.")
            .with("ref", target.into()),
    )
}

fn stash_use(root: &Path, verb: &str, reference: &str, opts: &StashUse) -> Outcome {
    let target = match stash_ref(reference) {
        Ok(t) => t,
        Err(e) => return failure(&e),
    };
    if let Some(moved) = stash_moved(root, &target, &opts.sha) {
        return moved;
    }
    let mut args = vec!["stash", verb];
    if opts.index {
        args.push("--index");
    }
    args.push("--end-of-options");
    args.push(&target);
    let out = git(root, &args);
    if out.ok() {
        let title = if verb == "pop" { "Stash applied and removed" } else { "Stash applied" };
        return success(title).with("ref", target.into());
    }
    let result = failure(&out.said()).with("ref", target.into());
    if result.kind == "conflict" {
        return Outcome { title: "The stash was applied with conflicts".into(), hint: "Resolve the files. The stash was kept.".into(), ..result };
    }
    result
}

pub fn stash_apply(root: &Path, reference: &str, opts: &StashUse) -> Outcome {
    stash_use(root, "apply", reference, opts)
}

pub fn stash_pop(root: &Path, reference: &str, opts: &StashUse) -> Outcome {
    stash_use(root, "pop", reference, opts)
}

pub fn stash_drop(root: &Path, reference: &str, sha: Option<String>) -> Outcome {
    let target = match stash_ref(reference) {
        Ok(t) => t,
        Err(e) => return failure(&e),
    };
    if let Some(moved) = stash_moved(root, &target, &sha) {
        return moved;
    }
    let out = git(root, &["stash", "drop", "--end-of-options", &target]);
    if !out.ok() {
        return failure(&out.said()).with("ref", target.into());
    }
    success("Stash dropped").with("ref", target.into())
}

fn has_untracked_part(root: &Path, target: &str) -> bool {
    let spec = format!("{target}^3");
    git(root, &["rev-parse", "-q", "--verify", &spec]).ok()
}

pub fn stash_files(root: &Path, reference: &str) -> Result<Vec<StashFile>, String> {
    let target = stash_ref(reference)?;
    let parent = format!("{target}^1");
    let tracked = read_ok(root, &["-c", "core.quotepath=off", "diff-tree", "-r", "--no-commit-id", "--name-status", "-z", "--no-renames", &parent, &target])?;
    let parts = split_z(&tracked);
    let mut files: Vec<StashFile> = parts
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| StashFile { path: pair[1].clone(), status: pair[0].chars().next().map(String::from).unwrap_or_default(), untracked: false })
        .collect();
    if has_untracked_part(root, &target) {
        let third = format!("{target}^3");
        let extra = read_ok(root, &["-c", "core.quotepath=off", "ls-tree", "-r", "--name-only", "-z", &third])?;
        files.extend(split_z(&extra).into_iter().map(|path| StashFile { path, status: "?".into(), untracked: true }));
    }
    Ok(files)
}

pub fn stash_diff(root: &Path, reference: &str, path: &str, untracked: Option<bool>) -> Result<String, String> {
    let target = stash_ref(reference)?;
    let p = clean_path(path)?;
    let flags = ["--literal-pathspecs", "diff", "--no-color", "--no-ext-diff", "--no-textconv"];
    if untracked != Some(true) {
        let parent = format!("{target}^1");
        let mut args = flags.to_vec();
        args.extend([parent.as_str(), target.as_str(), "--", p.as_str()]);
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
    args.extend([EMPTY_TREE, third.as_str(), "--", p.as_str()]);
    read_ok(root, &args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrepo;

    #[test]
    fn stash_push_list_apply_drop() {
        let repo = testrepo::new();
        let root = repo.root();
        assert_eq!(stash_push(root, &StashPush::default()).kind, "nothing");
        repo.write("a.txt", "changed\n");
        repo.write("new.txt", "fresh\n");
        let out = stash_push(root, &StashPush { message: "my\nwork".into(), include_untracked: true, ..StashPush::default() });
        assert!(out.ok, "{out:?}");
        let list = list_stashes(root).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].message, "my work");
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert!(list[0].untracked);
        assert!(!list[0].wip);
        let files = stash_files(root, "0").unwrap();
        assert!(files.iter().any(|f| f.path == "a.txt" && f.status == "M"));
        assert!(files.iter().any(|f| f.path == "new.txt" && f.untracked));
        assert!(stash_diff(root, "0", "a.txt", None).unwrap().contains("+changed"));
        assert!(stash_diff(root, "0", "new.txt", None).unwrap().contains("+fresh"));
        let moved = stash_apply(root, "0", &StashUse { sha: Some("deadbeef".into()), ..StashUse::default() });
        assert_eq!(moved.kind, "stashMoved");
        let applied = stash_apply(root, "0", &StashUse { sha: Some(list[0].sha.clone()), ..StashUse::default() });
        assert!(applied.ok, "{applied:?}");
        assert_eq!(repo.read("a.txt"), "changed\n");
        assert!(stash_drop(root, "stash@{0}", None).ok);
        assert!(list_stashes(root).unwrap().is_empty());
    }

    #[test]
    fn stash_pop_with_conflict_keeps_stash() {
        let repo = testrepo::new();
        let root = repo.root();
        repo.write("a.txt", "mine\n");
        assert!(stash_push(root, &StashPush::default()).ok);
        repo.commit_file("a.txt", "theirs\n", "other");
        let out = stash_pop(root, "0", &StashUse::default());
        assert_eq!(out.kind, "conflict", "{out:?}");
        assert_eq!(out.title, "The stash was applied with conflicts");
        assert_eq!(list_stashes(root).unwrap().len(), 1);
    }
}
