use crate::errors::describe;
use crate::ops::{branch_name_problem, lines, remote_name_problem, upstream_of, web_url};
use crate::outcome::{Outcome, described, failure, refusal, success};
use crate::run::{Out, git, read_ok};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::sync::LazyLock;

const KEEP_IGNORED: &str = "--no-overwrite-ignore";

const BRANCH_FORMAT: &str = "%(HEAD)%00%(refname)%00%(refname:short)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(worktreepath)%00%(committerdate:unix)%00%(objectname:short)%00%(subject)";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub name: String,
    pub fetch_url: Option<String>,
    pub push_url: Option<String>,
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub gone: bool,
    pub remote: bool,
    pub remote_name: Option<String>,
    pub short_name: String,
    pub worktree: Option<String>,
    pub subject: String,
    pub at: i64,
    pub sha: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateBranch {
    pub name: String,
    pub from: Option<String>,
    pub checkout: bool,
}

fn track_count(track: &str, word: &str) -> u32 {
    let re = Regex::new(&format!(r"{word} (\d+)")).expect("track");
    re.captures(track).and_then(|m| m[1].parse().ok()).unwrap_or(0)
}

fn some(text: &str) -> Option<String> {
    if text.is_empty() { None } else { Some(text.to_string()) }
}

pub fn parse_branches(text: &str) -> Vec<Branch> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\0').collect();
        let get = |i: usize| f.get(i).copied().unwrap_or("");
        let reference = get(1);
        if reference.is_empty() {
            continue;
        }
        let remote = reference.starts_with("refs/remotes/");
        if remote && reference.ends_with("/HEAD") {
            continue;
        }
        let current = get(0) == "*";
        let name = if remote {
            reference["refs/remotes/".len()..].to_string()
        } else if let Some(n) = reference.strip_prefix("refs/heads/") {
            n.to_string()
        } else {
            get(2).to_string()
        };
        let track = get(4);
        let (remote_name, short_name) = if remote {
            let (r, rest) = name.split_once('/').unwrap_or((name.as_str(), ""));
            (Some(r.to_string()), rest.to_string())
        } else {
            (None, name.clone())
        };
        out.push(Branch {
            current,
            upstream: some(get(3)),
            ahead: track_count(track, "ahead"),
            behind: track_count(track, "behind"),
            gone: track.trim() == "gone",
            remote,
            remote_name,
            short_name,
            worktree: if !remote && !current { some(get(5)) } else { None },
            subject: f.get(8..).map(|s| s.join("\0")).unwrap_or_default(),
            at: get(6).parse().unwrap_or(0),
            sha: some(get(7)),
            name,
        });
    }
    out
}

pub fn list_branches(root: &Path) -> Result<Vec<Branch>, String> {
    let format = format!("--format={BRANCH_FORMAT}");
    let text = read_ok(root, &["for-each-ref", "--sort=-committerdate", &format, "refs/heads", "refs/remotes"])?;
    let all = parse_branches(&text);
    let (remote, local): (Vec<Branch>, Vec<Branch>) = all.into_iter().partition(|b| b.remote);
    Ok(local.into_iter().chain(remote).collect())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefRow {
    pub reference: String,
    pub worktree: Option<String>,
    pub current: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoundRefs {
    pub local: Option<RefRow>,
    pub remote: Option<RefRow>,
}

pub fn find_refs(root: &Path, name: &str) -> Result<FoundRefs, String> {
    let heads = format!("refs/heads/{name}");
    let remotes = format!("refs/remotes/{name}");
    let text = read_ok(root, &["for-each-ref", "--format=%(refname)%00%(worktreepath)%00%(HEAD)%00%(upstream)", &heads, &remotes])?;
    let rows: Vec<RefRow> = text
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\0').collect();
            let get = |i: usize| f.get(i).copied().unwrap_or("");
            RefRow { reference: get(0).into(), worktree: some(get(1)), current: get(2) == "*", upstream: some(get(3)) }
        })
        .collect();
    Ok(FoundRefs {
        local: rows.iter().find(|r| r.reference == heads).cloned(),
        remote: rows.iter().find(|r| r.reference == remotes).cloned(),
    })
}

fn worktree_refusal(name: &str, folder: &str) -> Outcome {
    refusal("worktree", &format!("{name} is open in another folder"), "Open that worktree to work on it, or pick another branch here.")
        .detail(folder)
        .with("folder", folder.into())
        .with("branch", name.into())
}

pub fn used_by(text: &str) -> Option<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:used by worktree at|checked out at) '([^']+)'").expect("used by"));
    RE.captures(text).map(|m| m[1].to_string())
}

pub fn in_the_way(text: &str) -> Vec<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)untracked working tree files would be (?:overwritten|removed) by [a-z]+:\n((?:\t.*\n?)+)").expect("in the way")
    });
    RE.captures(text)
        .map(|m| m[1].split('\n').map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

fn switch_failure(out: &Out, name: &str) -> Outcome {
    let text = out.said();
    if let Some(folder) = used_by(&text) {
        return worktree_refusal(name, &folder);
    }
    let files = in_the_way(&text);
    if !files.is_empty() {
        let mut shown = files.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
        if files.len() > 5 {
            shown.push_str(&format!(" and {} more", files.len() - 5));
        }
        return refusal(
            "dirty",
            "Files that git does not track are in the way",
            &format!("Nothing was changed. Move or rename these first: {shown}"),
        )
        .with("branch", name.into())
        .with("files", json!(files))
        .stderr(&text)
        .detail(&describe(&text).detail);
    }
    failure(&text).with("branch", name.into())
}

fn switch_to(root: &Path, name: &str) -> Option<Outcome> {
    let out = git(root, &["switch", "--no-guess", KEEP_IGNORED, "--end-of-options", name]);
    if out.ok() { None } else { Some(switch_failure(&out, name)) }
}

fn invalid(problem: &str) -> Outcome {
    refusal("invalid", "That branch name is not valid", problem)
}

pub fn checkout_branch(root: &Path, name: &str) -> Outcome {
    if let Some(problem) = branch_name_problem(name) {
        return invalid(problem);
    }
    let found = match find_refs(root, name) {
        Ok(f) => f,
        Err(e) => return failure(&e),
    };
    if let Some(local) = &found.local {
        if local.current {
            return success(&format!("Already on {name}")).with("branch", name.into());
        }
        if let Some(folder) = &local.worktree {
            return worktree_refusal(name, folder);
        }
        return switch_to(root, name).unwrap_or_else(|| success(&format!("Now on {name}")).with("branch", name.into()));
    }
    if found.remote.is_none() {
        return refusal("notFound", &format!("There is no branch named {name}"), "Fetch, then try again.");
    }
    let local = name.split_once('/').map(|(_, rest)| rest.to_string()).unwrap_or_default();
    if local.is_empty() {
        return invalid("");
    }
    if let Some(problem) = branch_name_problem(&local) {
        return invalid(problem);
    }
    let mine = match find_refs(root, &local) {
        Ok(f) => f.local,
        Err(e) => return failure(&e),
    };
    let Some(mine) = mine else {
        let out = git(root, &["switch", KEEP_IGNORED, "--create", &local, "--track", "--end-of-options", name]);
        if !out.ok() {
            return switch_failure(&out, &local);
        }
        return success(&format!("Now on {local}, tracking {name}")).with("branch", local.into()).with("created", true.into());
    };
    if let Some(folder) = &mine.worktree
        && !mine.current
    {
        return worktree_refusal(&local, folder);
    }
    if mine.upstream.is_none() {
        let flag = format!("--set-upstream-to={name}");
        let _ = git(root, &["branch", &flag, "--end-of-options", &local]);
    }
    if !mine.current
        && let Some(failed) = switch_to(root, &local)
    {
        return failed;
    }
    if git(root, &["merge-base", "--is-ancestor", &local, name]).ok() {
        let same = lines(&git(root, &["rev-parse", &local, name]).stdout);
        if same.len() == 2 && same[0] != same[1] {
            let _ = git(root, &["merge", "--ff-only", KEEP_IGNORED, "--end-of-options", name]);
        }
    }
    success(&format!("Now on {local}")).with("branch", local.into())
}

pub fn create_branch(root: &Path, opts: &CreateBranch) -> Outcome {
    let name = opts.name.as_str();
    if let Some(problem) = branch_name_problem(name) {
        return invalid(problem);
    }
    if let Some(from) = &opts.from
        && (from.starts_with('-') || from.chars().any(|c| c.is_whitespace() || c == '\0'))
    {
        return refusal("invalid", "That start point is not valid", "Use a branch name or a commit id.");
    }
    match find_refs(root, name) {
        Ok(f) if f.local.is_some() => return refusal("exists", &format!("A branch named {name} already exists"), "Pick another name."),
        Ok(_) => {}
        Err(e) => return failure(&e),
    }
    let mut args = if opts.checkout { vec!["switch", KEEP_IGNORED, "--create", name, "--end-of-options"] } else { vec!["branch", "--end-of-options", name] };
    if let Some(from) = &opts.from {
        args.push(from);
    }
    let out = git(root, &args);
    if !out.ok() {
        return if opts.checkout { switch_failure(&out, name) } else { failure(&out.said()).with("branch", name.into()) };
    }
    let title = if opts.checkout { format!("Now on new branch {name}") } else { format!("Made branch {name}") };
    success(&title).with("branch", name.into())
}

pub fn delete_branch(root: &Path, name: &str, force: bool) -> Outcome {
    if let Some(problem) = branch_name_problem(name) {
        return invalid(problem);
    }
    let out = git(root, &["branch", if force { "-D" } else { "-d" }, "--end-of-options", name]);
    if out.ok() {
        return success(&format!("Deleted {name}")).with("branch", name.into());
    }
    let text = out.said();
    if text.to_lowercase().contains("not fully merged") {
        return refusal("unmerged", &format!("{name} has commits that are not merged"), "Delete anyway only if you do not need them.")
            .with("branch", name.into())
            .stderr(&text);
    }
    let folder = used_by(&text);
    if folder.is_some() || text.to_lowercase().contains("cannot delete branch") {
        return refusal("inUse", &format!("{name} is checked out"), "Switch to another branch first.")
            .with("branch", name.into())
            .with("folder", json!(folder))
            .detail(folder.as_deref().unwrap_or(""))
            .stderr(&text);
    }
    failure(&text).with("branch", name.into())
}

pub fn set_upstream(root: &Path, branch: &str, upstream: Option<&str>) -> Outcome {
    if let Some(problem) = branch_name_problem(branch) {
        return invalid(problem);
    }
    let Some(upstream) = upstream else {
        let out = git(root, &["branch", "--unset-upstream", "--end-of-options", branch]);
        if !out.ok() {
            return failure(&out.said()).with("branch", branch.into());
        }
        return success(&format!("{branch} has no upstream now")).with("branch", branch.into()).with("upstream", json!(null));
    };
    if let Some(bad) = branch_name_problem(upstream) {
        return refusal("invalid", "That upstream name is not valid", bad);
    }
    let flag = format!("--set-upstream-to={upstream}");
    let out = git(root, &["branch", &flag, "--end-of-options", branch]);
    if !out.ok() {
        return failure(&out.said()).with("branch", branch.into());
    }
    success(&format!("{branch} now tracks {upstream}")).with("branch", branch.into()).with("upstream", upstream.into())
}

fn is_sha(text: &str) -> bool {
    (4..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn sha_of(root: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    let out = git(root, &["rev-parse", "-q", "--verify", "--end-of-options", &spec]);
    let text = out.stdout.trim();
    if out.ok() && !text.is_empty() { Some(text.to_string()) } else { None }
}

fn same_sha(full: &Option<String>, expect: &str) -> bool {
    full.as_deref().is_some_and(|f| f.to_lowercase().starts_with(&expect.to_lowercase()))
}

pub fn reset_to_remote(root: &Path, expect_head: Option<&str>, expect_target: Option<&str>) -> Outcome {
    for value in [expect_head, expect_target].into_iter().flatten() {
        if !is_sha(value) {
            return refusal("invalid", "That commit id is not valid", "Nothing was reset.");
        }
    }
    let Some(upstream) = upstream_of(root) else { return described("has no upstream branch") };
    let head = sha_of(root, "HEAD");
    let Some(target) = sha_of(root, "@{u}") else {
        return refusal("notFound", &format!("Cannot find {upstream}"), "Fetch, then try again. Nothing was reset.").with("upstream", upstream.into());
    };
    let target_opt = Some(target.clone());
    if let Some(expect) = expect_head
        && !same_sha(&head, expect)
    {
        return refusal("headChanged", "The branch moved since you looked", "Nothing was reset. Look at it again, then try again.")
            .with("upstream", upstream.into())
            .with("head", json!(head))
            .with("target", target.into());
    }
    if let Some(expect) = expect_target
        && !same_sha(&target_opt, expect)
    {
        return refusal(
            "headChanged",
            &format!("{upstream} changed since you looked"),
            "Nothing was reset. A fetch brought new commits. Look at it again, then try again.",
        )
        .with("upstream", upstream.into())
        .with("head", json!(head))
        .with("target", target.into());
    }
    let out = git(root, &["reset", "--hard", &target]);
    if !out.ok() {
        return failure(&out.said()).with("upstream", upstream.into());
    }
    success(&format!("Branch reset to {upstream}")).with("upstream", upstream.into()).with("head", target.into()).with("before", json!(head))
}

pub fn list_remotes(root: &Path) -> Result<Vec<Remote>, String> {
    static LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\S+)\t(.*) \((fetch|push)\)$").expect("remote line"));
    let text = read_ok(root, &["remote", "-v"])?;
    let mut list: Vec<Remote> = Vec::new();
    for line in text.split('\n') {
        let Some(m) = LINE.captures(line) else { continue };
        let at = match list.iter().position(|r| r.name == m[1]) {
            Some(i) => i,
            None => {
                list.push(Remote { name: m[1].to_string(), ..Remote::default() });
                list.len() - 1
            }
        };
        if &m[3] == "fetch" {
            list[at].fetch_url = Some(m[2].to_string());
        } else {
            list[at].push_url = Some(m[2].to_string());
        }
    }
    for remote in &mut list {
        remote.web_url = remote.fetch_url.as_deref().or(remote.push_url.as_deref()).and_then(web_url);
    }
    Ok(list)
}

fn url_problem(url: &str) -> Option<&'static str> {
    let u = url.trim();
    if u.is_empty() {
        return Some("Type the remote URL.");
    }
    if u.starts_with('-') || u.chars().any(|c| c.is_whitespace() || c == '\0') {
        return Some("That URL is not valid.");
    }
    None
}

pub fn add_remote(root: &Path, name: &str, url: &str) -> Outcome {
    if let Some(problem) = remote_name_problem(name).or_else(|| url_problem(url)) {
        return refusal("invalid", "Check the name and URL", problem);
    }
    let out = git(root, &["remote", "add", "--end-of-options", name, url.trim()]);
    if !out.ok() {
        return failure(&out.said());
    }
    success(&format!("Added remote {name}")).with("name", name.into())
}

pub fn set_remote_url(root: &Path, name: &str, url: &str) -> Outcome {
    if let Some(problem) = remote_name_problem(name).or_else(|| url_problem(url)) {
        return refusal("invalid", "Check the name and URL", problem);
    }
    let out = git(root, &["remote", "set-url", "--end-of-options", name, url.trim()]);
    if !out.ok() {
        return failure(&out.said());
    }
    success(&format!("Changed the URL of {name}")).with("name", name.into())
}

pub fn rename_remote(root: &Path, name: &str, new_name: &str) -> Outcome {
    if let Some(problem) = remote_name_problem(name).or_else(|| remote_name_problem(new_name)) {
        return refusal("invalid", "That remote name is not valid", problem);
    }
    let out = git(root, &["remote", "rename", "--end-of-options", name, new_name]);
    if !out.ok() {
        return failure(&out.said());
    }
    success(&format!("Renamed {name} to {new_name}")).with("name", new_name.into())
}

pub fn remove_remote(root: &Path, name: &str) -> Outcome {
    if let Some(problem) = remote_name_problem(name) {
        return refusal("invalid", "That remote name is not valid", problem);
    }
    let out = git(root, &["remote", "remove", "--end-of-options", name]);
    if !out.ok() {
        return failure(&out.said());
    }
    success(&format!("Removed remote {name}")).with("name", name.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::current_branch;
    use crate::testrepo;

    #[test]
    fn branches_create_switch_delete() {
        let repo = testrepo::new();
        let root = repo.root();
        let made = create_branch(root, &CreateBranch { name: "feat".into(), from: None, checkout: true });
        assert!(made.ok, "{made:?}");
        assert_eq!(current_branch(root).as_deref(), Some("feat"));
        assert_eq!(create_branch(root, &CreateBranch { name: "feat".into(), ..CreateBranch::default() }).kind, "exists");
        assert_eq!(create_branch(root, &CreateBranch { name: "bad name".into(), ..CreateBranch::default() }).kind, "invalid");
        repo.commit_file("b.txt", "b\n", "b");
        assert_eq!(checkout_branch(root, "feat").title, "Already on feat");
        assert!(checkout_branch(root, "main").ok);
        assert_eq!(delete_branch(root, "feat", false).kind, "unmerged");
        assert!(delete_branch(root, "feat", true).ok);
        assert_eq!(delete_branch(root, "main", true).kind, "inUse");
        assert_eq!(checkout_branch(root, "nope").kind, "notFound");
        let list = list_branches(root).unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].current);
        assert_eq!(list[0].subject, "first");
    }

    #[test]
    fn untracked_files_in_the_way() {
        let repo = testrepo::new();
        let root = repo.root();
        repo.git(&["switch", "-q", "-c", "other"]);
        repo.commit_file("new.txt", "tracked\n", "add new");
        repo.git(&["switch", "-q", "main"]);
        repo.write("new.txt", "mine\n");
        let out = checkout_branch(root, "other");
        assert_eq!(out.kind, "dirty", "{out:?}");
        assert!(out.hint.contains("new.txt"));
    }

    #[test]
    fn remote_branches_and_remotes() {
        let (repo, remote) = testrepo::with_remote();
        let root = repo.root();
        let other = testrepo::clone_of(&remote);
        other.git(&["switch", "-q", "-c", "topic"]);
        other.commit_file("t.txt", "t\n", "topic");
        other.git(&["push", "-q", "origin", "topic"]);
        repo.git(&["fetch", "-q", "origin"]);
        let out = checkout_branch(root, "origin/topic");
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Now on topic, tracking origin/topic");
        let list = list_branches(root).unwrap();
        let remote_row = list.iter().find(|b| b.name == "origin/topic").unwrap();
        assert_eq!(remote_row.remote_name.as_deref(), Some("origin"));
        assert_eq!(remote_row.short_name, "topic");
        let remotes = list_remotes(root).unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].name, "origin");
        assert!(add_remote(root, "up", "git@github.com:me/x.git").ok);
        let up = list_remotes(root).unwrap().into_iter().find(|r| r.name == "up").unwrap();
        assert_eq!(up.web_url.as_deref(), Some("https://github.com/me/x"));
        assert!(set_remote_url(root, "up", "https://github.com/me/y").ok);
        assert!(rename_remote(root, "up", "up2").ok);
        assert!(remove_remote(root, "up2").ok);
        assert_eq!(add_remote(root, "bad", "-x").kind, "invalid");
        assert!(set_upstream(root, "topic", None).ok);
        assert!(set_upstream(root, "topic", Some("origin/topic")).ok);
    }

    #[test]
    fn reset_to_remote_checks_heads() {
        let (repo, _remote) = testrepo::with_remote();
        let root = repo.root();
        let before = repo.commit_file("a.txt", "local\n", "local");
        assert_eq!(reset_to_remote(root, Some("zz"), None).kind, "invalid");
        assert_eq!(reset_to_remote(root, Some("0000000"), None).kind, "headChanged");
        let out = reset_to_remote(root, Some(&before[..7]), None);
        assert!(out.ok, "{out:?}");
        assert_eq!(repo.read("a.txt"), "one\ntwo\nthree\n");
    }
}
