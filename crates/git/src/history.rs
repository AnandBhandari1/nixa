use crate::branches::{self, CreateBranch, Remote};
use crate::errors::describe;
use crate::ops::{self, Identity, Message, State, compose_message, current_branch, identity_gate, lines, op_word, read_state, split_message_text, split_z, temp_text};
use crate::outcome::{Outcome, described, failure, plural, refusal, success};
use crate::run::{self, Job, Out, git, git_env, read, read_ok};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

pub const RESET_MODES: [&str; 3] = ["soft", "mixed", "hard"];

const INFO_FORMAT: &str = "%H%x00%h%x00%P%x00%an%x00%ae%x00%at%x00%B";

const CHAIN_FORMAT: &str = "%H%x00%T%x00%P%x00%an%x00%ae%x00%ad%x00%e";

static HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^https?://([^/]+)").unwrap());

static BLOCK_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)would be (overwritten|removed) by|would lose untracked files").unwrap());

static UNTRACKED_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)untracked working tree files would be|would lose untracked files").unwrap());

static TRACKED_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)Your local changes to the following files|Please commit your changes or stash them").unwrap());

static DIRTY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)would be (overwritten|removed) by|would lose untracked files|Please commit your changes or stash them|Your local changes to the following files").unwrap()
});

static NOTHING_TO_COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)nothing to commit|nothing added to commit").unwrap());

static UNKNOWN_EMPTY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)unknown option.*empty|usage: git cherry-pick").unwrap());

static SAFE_ENCODING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._:-]*$").unwrap());

static RAW_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-?\d+ [+-]\d{4}$").unwrap());

static REF_MOVED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)but expected|is at [0-9a-f]+").unwrap());

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub sha: String,
    pub short: String,
    pub subject: String,
    pub body: String,
    pub author: String,
    pub email: String,
    pub at: i64,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Refs {
    pub branches: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Web {
    pub remote: String,
    pub host: String,
    pub name: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    #[serde(flatten)]
    pub info: Info,
    pub is_merge: bool,
    pub is_root: bool,
    pub on_head: bool,
    pub is_head: bool,
    pub pushed: bool,
    pub remote_branches: Vec<String>,
    pub refs: Refs,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub op: Option<String>,
    pub after: usize,
    pub merges_after: usize,
    pub web: Option<Web>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChainCommit {
    pub sha: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub name: String,
    pub email: String,
    pub date: String,
    pub encoding: String,
}

#[derive(Debug, Clone, Default)]
pub struct CheckoutCommit {
    pub sha: String,
    pub force: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CreateBranchAt {
    pub name: String,
    pub sha: String,
    pub checkout: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CreateTag {
    pub name: String,
    pub sha: String,
    pub message: String,
    pub identity: Identity,
}

#[derive(Debug, Clone, Default)]
pub struct CherryPick {
    pub sha: String,
    pub mainline: Option<u32>,
    pub record_origin: bool,
    pub expect_head: Option<String>,
    pub expect_branch: Option<Option<String>>,
    pub identity: Identity,
    pub job: Job,
}

#[derive(Debug, Clone, Default)]
pub struct RevertCommit {
    pub sha: String,
    pub mainline: Option<u32>,
    pub expect_head: Option<String>,
    pub expect_branch: Option<Option<String>>,
    pub identity: Identity,
    pub job: Job,
}

#[derive(Debug, Clone, Default)]
pub struct ResetTo {
    pub sha: String,
    pub mode: String,
    pub expect_head: Option<String>,
    pub expect_branch: Option<Option<String>>,
    pub expect_untracked: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UndoLastCommit {
    pub expect_head: Option<String>,
    pub expect_branch: Option<Option<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct RewordCommit {
    pub sha: String,
    pub summary: String,
    pub body: String,
    pub expect_head: Option<String>,
    pub expect_branch: Option<Option<String>>,
    pub identity: Identity,
    pub job: Job,
}

fn is_full_sha(text: &str) -> bool {
    (40..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn short_of(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn missing() -> Outcome {
    refusal("notFound", "This commit is not in the repo", "Reload the history and try again.")
}

fn head_changed() -> Outcome {
    refusal("headChanged", "The branch moved", "Something changed since you opened this. Open it again. Nothing was changed.")
}

fn branch_changed() -> Outcome {
    refusal("headChanged", "You are on another branch now", "Something changed since you opened this. Open it again. Nothing was changed.")
}

fn stale_refusal(expect_head: &Option<String>, expect_branch: &Option<Option<String>>, head: &str, branch: &Option<String>) -> Option<Outcome> {
    if let Some(expect) = expect_head
        && !expect.is_empty()
        && expect != head
    {
        return Some(head_changed());
    }
    if let Some(expect) = expect_branch
        && expect != branch
    {
        return Some(branch_changed());
    }
    None
}

fn unchanged(result: Outcome) -> Outcome {
    let hint = format!("{} Nothing was changed.", result.hint).trim().to_string();
    Outcome { hint, ..result }
}

fn unborn() -> Outcome {
    refusal("unborn", "This branch has no commits yet", "Make a first commit, then try again.")
}

pub fn rev_problem(rev: &str) -> Option<&'static str> {
    if rev.trim().is_empty() {
        return Some("No commit was given.");
    }
    if rev.starts_with('-') || rev.chars().any(|c| c.is_whitespace() || c == '\0') || rev.contains("..") {
        return Some("That is not a commit id.");
    }
    None
}

fn resolve_commit(root: &Path, rev: &str) -> Option<String> {
    if rev_problem(rev).is_some() {
        return None;
    }
    let out = read(root, &["rev-parse", "-q", "--verify", &format!("{rev}^{{commit}}")]);
    let sha = if out.ok() { out.stdout.trim().to_string() } else { String::new() };
    is_full_sha(&sha).then_some(sha)
}

fn head_of(root: &Path) -> Option<String> {
    let out = read(root, &["rev-parse", "-q", "--verify", "HEAD"]);
    let sha = if out.ok() { out.stdout.trim().to_string() } else { String::new() };
    is_full_sha(&sha).then_some(sha)
}

fn is_ancestor(root: &Path, a: &str, b: &str) -> bool {
    read(root, &["merge-base", "--is-ancestor", a, b]).ok()
}

fn on_line(root: &Path, sha: &str, head: Option<&str>) -> bool {
    match head {
        Some(head) => sha == head || is_ancestor(root, sha, head),
        None => false,
    }
}

fn count_revs(root: &Path, args: &[&str]) -> Result<usize, String> {
    let mut all = vec!["rev-list", "--count"];
    all.extend_from_slice(args);
    Ok(read_ok(root, &all)?.trim().parse().unwrap_or(0))
}

fn parents_of(root: &Path, sha: &str) -> Result<Vec<String>, String> {
    let text = read_ok(root, &["rev-list", "--parents", "-n", "1", sha])?;
    Ok(lines(&text).first().map(|line| line.split(' ').skip(1).map(String::from).collect()).unwrap_or_default())
}

fn remote_branches_with(root: &Path, sha: &str) -> Result<Vec<String>, String> {
    let text = read_ok(root, &["for-each-ref", "--contains", sha, "--format=%(refname)", "refs/remotes"])?;
    Ok(lines(&text)
        .into_iter()
        .filter(|r| !r.ends_with("/HEAD"))
        .map(|r| r.strip_prefix("refs/remotes/").unwrap_or(&r).to_string())
        .collect())
}

fn refs_at(root: &Path, sha: &str) -> Result<Refs, String> {
    let all = lines(&read_ok(root, &["for-each-ref", "--points-at", sha, "--format=%(refname)", "refs/heads", "refs/tags"])?);
    Ok(Refs {
        branches: all.iter().filter_map(|r| r.strip_prefix("refs/heads/")).map(String::from).collect(),
        tags: all.iter().filter_map(|r| r.strip_prefix("refs/tags/")).map(String::from).collect(),
    })
}

fn place_name(branch: &Option<String>) -> String {
    branch.clone().unwrap_or_else(|| "this detached commit".into())
}

fn state_value(state: &State) -> Value {
    serde_json::to_value(state).unwrap_or(Value::Null)
}

fn op_gate(root: &Path) -> Result<Option<Outcome>, String> {
    let state = read_state(root)?;
    let Some(op) = state.op.clone() else { return Ok(None) };
    Ok(Some(
        refusal("busy", &format!("Finish the {} first", op_word(&op).to_lowercase()), "Continue or cancel it in the Changes panel. Nothing was changed.")
            .with("state", state_value(&state)),
    ))
}

fn run_making(root: &Path, args: &[&str], job: &Job, known: Option<Vec<String>>) -> (Out, Vec<String>) {
    let hooks = known.unwrap_or_else(|| ops::commit_hooks(root));
    let out = if hooks.is_empty() { git_env(root, args, &job.env) } else { run::job(root, args, job) };
    (out, hooks)
}

fn job_stopped(out: &Out) -> Option<Outcome> {
    if out.cancelled {
        return Some(refusal("cancelled", "Stopped", ""));
    }
    if out.timed_out {
        return Some(described("timed out waiting").with("timedOut", true.into()));
    }
    None
}

pub fn blocked_files(text: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut inside = false;
    for line in text.split('\n') {
        if BLOCK_START.is_match(line) {
            inside = true;
            continue;
        }
        if inside && line.starts_with('\t') {
            files.push(line.trim().to_string());
            continue;
        }
        inside = false;
    }
    files
}

fn is_dirty(text: &str) -> bool {
    DIRTY.is_match(text)
}

fn dirty_refusal(text: &str, title: &str) -> Outcome {
    let files = blocked_files(text);
    let untracked = UNTRACKED_BLOCK.is_match(text);
    let tracked = TRACKED_BLOCK.is_match(text);
    let (title, hint) = if untracked && !tracked {
        ("Files that git does not track are in the way", "Git would replace them. Move or rename them first. Nothing was changed.")
    } else if untracked {
        (title, "Commit or stash your changes, and move the files that git does not track. Nothing was changed.")
    } else {
        (title, "Commit or stash them first. Nothing was changed.")
    };
    let detail = files.iter().take(12).cloned().collect::<Vec<_>>().join("\n");
    refusal("dirty", title, hint).detail(&detail).stderr(text).with("files", json!(files)).with("untracked", untracked.into())
}

fn hook_kind(result: Outcome, hooks: &[String]) -> Outcome {
    if hooks.is_empty() || (result.kind != "other" && result.kind != "hook") {
        return result;
    }
    let words = describe("pre-commit hook");
    Outcome { kind: "hook".into(), title: words.title, hint: words.hint, ..result }
}

fn host_of(url: &str) -> String {
    HOST.captures(url).and_then(|c| c.get(1)).map(|m| m.as_str().to_lowercase()).unwrap_or_default()
}

pub fn commit_url(base: &str, sha: &str) -> Option<String> {
    if base.is_empty() || sha.is_empty() {
        return None;
    }
    let host = host_of(base);
    if host.contains("gitlab") {
        return Some(format!("{base}/-/commit/{sha}"));
    }
    if host == "bitbucket.org" {
        return Some(format!("{base}/commits/{sha}"));
    }
    Some(format!("{base}/commit/{sha}"))
}

pub fn forge_name(host: &str) -> String {
    let h = host.to_lowercase();
    if h == "github.com" || h.starts_with("github.") {
        return "GitHub".into();
    }
    if h.contains("gitlab") {
        return "GitLab".into();
    }
    if h == "bitbucket.org" {
        return "Bitbucket".into();
    }
    if h == "codeberg.org" {
        return "Codeberg".into();
    }
    if h.is_empty() { "the web".into() } else { h }
}

pub fn pick_web_remote(remotes: &[Remote], remote_branches: &[String], upstream: Option<&str>) -> Option<Remote> {
    let usable: Vec<&Remote> = remotes.iter().filter(|r| r.web_url.as_deref().is_some_and(|u| !u.is_empty())).collect();
    let holds = |r: &Remote| remote_branches.iter().any(|b| b.starts_with(&format!("{}/", r.name)));
    let up_name = usable
        .iter()
        .map(|r| r.name.as_str())
        .filter(|name| upstream.is_some_and(|u| !u.is_empty() && u.starts_with(&format!("{name}/"))))
        .max_by_key(|name| name.len());
    let is_up = |r: &Remote| up_name.is_some_and(|n| n == r.name);
    let pick = usable
        .iter()
        .find(|r| holds(r) && is_up(r))
        .or_else(|| usable.iter().find(|r| holds(r) && r.name == "origin"))
        .or_else(|| usable.iter().find(|r| holds(r)))
        .or_else(|| usable.iter().find(|r| is_up(r)))
        .or_else(|| usable.iter().find(|r| r.name == "origin"))
        .or_else(|| usable.first());
    pick.map(|r| (*r).clone())
}

fn web_for(remotes: &[Remote], remote_branches: &[String], upstream: Option<&str>, sha: &str) -> Option<Web> {
    let remote = pick_web_remote(remotes, remote_branches, upstream)?;
    let base = remote.web_url.clone().unwrap_or_default();
    let host = host_of(&base);
    Some(Web { remote: remote.name, name: forge_name(&host), host, url: commit_url(&base, sha) })
}

pub fn parse_info(text: &str) -> Info {
    let parts: Vec<&str> = text.split('\0').collect();
    let at = |k: usize| parts.get(k).copied().unwrap_or("");
    let message = split_message_text(&parts.get(6..).map(|rest| rest.join("\0")).unwrap_or_default());
    let parents = at(2).trim();
    Info {
        sha: at(0).trim().into(),
        short: at(1).trim().into(),
        subject: message.summary,
        body: message.body,
        author: at(3).into(),
        email: at(4).into(),
        at: at(5).trim().parse().unwrap_or(0),
        parents: if parents.is_empty() { Vec::new() } else { parents.split_whitespace().map(String::from).collect() },
    }
}

pub fn commit_info(root: &Path, sha: &str) -> Result<CommitInfo, String> {
    let sha = resolve_commit(root, sha).ok_or_else(|| "This commit is not in the repo.".to_string())?;
    let text = read_ok(root, &["log", "-1", "--no-show-signature", &format!("--format={INFO_FORMAT}"), &sha, "--"])?;
    let head = head_of(root);
    let branch = current_branch(root);
    let refs = refs_at(root, &sha)?;
    let remote_branches = remote_branches_with(root, &sha)?;
    let state = read_state(root).ok();
    let remotes = branches::list_remotes(root).unwrap_or_default();
    let upstream = if branch.is_some() { ops::upstream_of(root) } else { None };
    let mut info = parse_info(&text);
    let on_head = on_line(root, &sha, head.as_deref());
    let (after, merges_after) = match &head {
        Some(h) if on_head && *h != sha => {
            let range = format!("{sha}..{h}");
            (count_revs(root, &[&range])?, count_revs(root, &["--min-parents=2", &range])?)
        }
        _ => (0, 0),
    };
    if info.short.is_empty() {
        info.short = short_of(&sha);
    }
    info.sha = sha.clone();
    let web = web_for(&remotes, &remote_branches, upstream.as_deref(), &sha);
    Ok(CommitInfo {
        is_merge: info.parents.len() > 1,
        is_root: info.parents.is_empty(),
        on_head,
        is_head: head.as_deref() == Some(sha.as_str()),
        pushed: !remote_branches.is_empty(),
        remote_branches,
        refs,
        detached: branch.is_none() && head.is_some(),
        head,
        branch,
        upstream,
        op: state.and_then(|s| s.op),
        after,
        merges_after,
        web,
        info,
    })
}

pub fn commit_web_url(root: &Path, sha: &str) -> Result<Option<String>, String> {
    Ok(commit_info(root, sha)?.web.and_then(|w| w.url))
}

pub fn checkout_commit(root: &Path, opts: &CheckoutCommit) -> Outcome {
    try_checkout_commit(root, opts).unwrap_or_else(|e| failure(&e))
}

fn try_checkout_commit(root: &Path, opts: &CheckoutCommit) -> Result<Outcome, String> {
    if let Some(busy) = op_gate(root)? {
        return Ok(busy);
    }
    let Some(sha) = resolve_commit(root, &opts.sha) else { return Ok(missing()) };
    let head = head_of(root);
    let branch = current_branch(root);
    if branch.is_none() && head.as_deref() == Some(sha.as_str()) {
        return Ok(success("You are already on this commit")
            .with("sha", sha.into())
            .with("detached", true.into())
            .with("already", true.into()));
    }
    if branch.is_none()
        && let Some(head) = &head
        && !opts.force
    {
        let left = count_revs(root, &[head, "--not", "--branches", "--tags", "--remotes", &sha])?;
        if left > 0 {
            return Ok(refusal(
                "leaving",
                &format!("{} would be left behind", plural(left, "commit", "commits")),
                "They are on no branch, so they are hard to find later. Make a branch for them first, or check out anyway.",
            )
            .with("count", left.into()));
        }
    }
    let out = git(root, &["switch", "--no-overwrite-ignore", "--detach", &sha]);
    if !out.ok() {
        let text = out.said();
        return Ok(if is_dirty(&text) { dirty_refusal(&text, "Your local changes would be lost") } else { failure(&text) });
    }
    Ok(success(&format!("Checked out {}", short_of(&sha)))
        .hint("You are not on a branch now. Make a branch here if you want to keep new work.")
        .with("sha", sha.into())
        .with("detached", true.into())
        .with("from", json!(branch)))
}

pub fn create_branch_at(root: &Path, opts: &CreateBranchAt) -> Outcome {
    try_create_branch_at(root, opts).unwrap_or_else(|e| failure(&e))
}

fn try_create_branch_at(root: &Path, opts: &CreateBranchAt) -> Result<Outcome, String> {
    let name = opts.name.trim();
    if let Some(problem) = ops::branch_name_problem(name) {
        return Ok(refusal("invalid", "That branch name is not valid", problem));
    }
    let Some(sha) = resolve_commit(root, &opts.sha) else { return Ok(missing()) };
    if !opts.checkout {
        let made = branches::create_branch(root, &CreateBranch { name: name.into(), from: Some(sha.clone()), checkout: false });
        return Ok(if made.ok { made.with("sha", sha.into()) } else { made });
    }
    if let Some(busy) = op_gate(root)? {
        return Ok(busy);
    }
    if read(root, &["rev-parse", "-q", "--verify", &format!("refs/heads/{name}")]).ok() {
        return Ok(refusal("exists", &format!("A branch named {name} already exists"), "Pick another name."));
    }
    let out = git(root, &["switch", "--no-overwrite-ignore", "--create", name, "--end-of-options", &sha]);
    if !out.ok() {
        let text = out.said();
        return Ok(if is_dirty(&text) {
            dirty_refusal(&text, "Your local changes would be lost")
        } else {
            failure(&text).with("branch", name.into())
        });
    }
    Ok(success(&format!("Now on new branch {name}")).with("branch", name.into()).with("sha", sha.into()))
}

pub fn create_tag(root: &Path, opts: &CreateTag) -> Outcome {
    try_create_tag(root, opts).unwrap_or_else(|e| failure(&e))
}

fn try_create_tag(root: &Path, opts: &CreateTag) -> Result<Outcome, String> {
    let name = opts.name.trim();
    if let Some(problem) = ops::branch_name_problem(name) {
        return Ok(refusal("invalid", "That tag name is not valid", problem));
    }
    let Some(sha) = resolve_commit(root, &opts.sha) else { return Ok(missing()) };
    if git(root, &["rev-parse", "-q", "--verify", &format!("refs/tags/{name}")]).ok() {
        return Ok(refusal("exists", &format!("A tag named {name} already exists"), "Pick another name."));
    }
    let message = opts.message.replace("\r\n", "\n");
    let message = message.trim();
    let mut identity = Value::Null;
    let mut file = None;
    if !message.is_empty() {
        identity = match identity_gate(&opts.identity) {
            Ok(value) => value,
            Err(stop) => return Ok(*stop),
        };
        file = Some(temp_text("nixa-git-hist.", &format!("{message}\n"))?);
    }
    let path = file.as_ref().map(|f| f.path().to_string_lossy().into_owned());
    let args = match &path {
        Some(p) => vec!["tag", "-a", "--cleanup=whitespace", "-F", p, name, &sha],
        None => vec!["tag", name, &sha],
    };
    let out = git(root, &args);
    if !out.ok() {
        return Ok(failure(&out.said()));
    }
    Ok(success(&format!("Made tag {name}"))
        .with("tag", name.into())
        .with("sha", sha.clone().into())
        .with("annotated", path.is_some().into())
        .with("identity", identity))
}

fn empty_refusal(op: &str, branch: &Option<String>, text: &str) -> Outcome {
    let place = place_name(branch);
    if op == "revert" {
        refusal("empty", "Nothing to undo", &format!("The changes of this commit are already gone from {place}. Nothing was changed.")).stderr(text)
    } else {
        refusal("empty", "Nothing to pick", &format!("{place} already has these changes. Nothing was changed.")).stderr(text)
    }
}

fn pick_outcome(root: &Path, op: &str, out: &Out, hooks: &[String], before: &str, branch: &Option<String>, done: &str) -> Outcome {
    let text = out.said();
    let words = op_word(op).to_lowercase();
    if let Ok(state) = read_state(root)
        && state.op.as_deref() == Some(op)
    {
        if !state.conflicts.is_empty() {
            let detail = state.conflicts.iter().take(12).cloned().collect::<Vec<_>>().join("\n");
            return refusal("conflict", &format!("The {words} stopped on a conflict"), "Fix the files in the Changes panel, then press Continue. Or cancel it there.")
                .detail(&detail)
                .stderr(&text)
                .with("state", state_value(&state))
                .with("conflicts", json!(state.conflicts));
        }
        let result = hook_kind(failure(&text).with("state", state_value(&state)), hooks);
        let kind = if result.kind == "other" { "stopped".to_string() } else { result.kind.clone() };
        let hint = format!("{} The {words} is waiting. Continue or cancel it in the Changes panel.", result.hint).trim().to_string();
        return Outcome { kind, hint, ..result };
    }
    if let Some(stop) = job_stopped(out) {
        return stop;
    }
    if !out.ok() {
        if is_dirty(&text) {
            return dirty_refusal(&text, "Your local changes are in the way");
        }
        if NOTHING_TO_COMMIT.is_match(&text) {
            return empty_refusal(op, branch, &text);
        }
        return hook_kind(failure(&text), hooks);
    }
    let after = head_of(root);
    if after.as_deref() == Some(before) {
        return empty_refusal(op, branch, &text);
    }
    let short = after.as_deref().map(short_of).unwrap_or_default();
    success(done).stderr(&out.stderr).with("sha", json!(after)).with("short", short.into()).with("branch", json!(branch))
}

fn mainline_args(parents: &[String], mainline: Option<u32>) -> Result<Vec<String>, String> {
    if parents.len() < 2 {
        return Ok(Vec::new());
    }
    let n = match mainline {
        Some(0) | None => 1,
        Some(n) => n as usize,
    };
    if n > parents.len() {
        return Err(format!("Pick a parent from 1 to {}.", parents.len()));
    }
    Ok(vec!["-m".into(), n.to_string()])
}

pub fn cherry_pick(root: &Path, opts: &CherryPick) -> Outcome {
    try_cherry_pick(root, opts).unwrap_or_else(|e| failure(&e))
}

fn try_cherry_pick(root: &Path, opts: &CherryPick) -> Result<Outcome, String> {
    if let Some(busy) = op_gate(root)? {
        return Ok(busy);
    }
    let Some(sha) = resolve_commit(root, &opts.sha) else { return Ok(missing()) };
    let branch = current_branch(root);
    let Some(head) = head_of(root) else { return Ok(unborn()) };
    if let Some(stale) = stale_refusal(&opts.expect_head, &opts.expect_branch, &head, &branch) {
        return Ok(stale);
    }
    if on_line(root, &sha, Some(&head)) {
        return Ok(refusal("onHead", "This commit is already here", &format!("{} already has it. Nothing was changed.", place_name(&branch))));
    }
    let parents = parents_of(root, &sha)?;
    let line = match mainline_args(&parents, opts.mainline) {
        Ok(line) => line,
        Err(problem) => return Ok(refusal("invalid", "That parent does not exist", &problem)),
    };
    let identity = match identity_gate(&opts.identity) {
        Ok(value) => value,
        Err(stop) => return Ok(*stop),
    };
    let mut tail: Vec<&str> = Vec::new();
    if opts.record_origin {
        tail.push("-x");
    }
    tail.extend(line.iter().map(String::as_str));
    tail.push(&sha);
    let mut args = vec!["cherry-pick", "--empty=drop"];
    args.extend_from_slice(&tail);
    let (mut out, mut hooks) = run_making(root, &args, &opts.job, None);
    if !out.ok() && UNKNOWN_EMPTY.is_match(&out.said()) && read_state(root)?.op.is_none() {
        let mut plain = vec!["cherry-pick"];
        plain.extend_from_slice(&tail);
        (out, hooks) = run_making(root, &plain, &opts.job, None);
    }
    let done = format!("Cherry-picked {} onto {}", short_of(&sha), place_name(&branch));
    let result = pick_outcome(root, "cherry-pick", &out, &hooks, &head, &branch, &done);
    Ok(result.with("identity", identity).with("from", sha.into()))
}

pub fn revert_commit(root: &Path, opts: &RevertCommit) -> Outcome {
    try_revert_commit(root, opts).unwrap_or_else(|e| failure(&e))
}

fn try_revert_commit(root: &Path, opts: &RevertCommit) -> Result<Outcome, String> {
    if let Some(busy) = op_gate(root)? {
        return Ok(busy);
    }
    let Some(sha) = resolve_commit(root, &opts.sha) else { return Ok(missing()) };
    let branch = current_branch(root);
    let Some(head) = head_of(root) else { return Ok(unborn()) };
    if let Some(stale) = stale_refusal(&opts.expect_head, &opts.expect_branch, &head, &branch) {
        return Ok(stale);
    }
    if !on_line(root, &sha, Some(&head)) {
        return Ok(refusal(
            "notOnHead",
            &format!("This commit is not on {}", place_name(&branch)),
            "Revert undoes a commit of the branch you are on. Switch to its branch first.",
        ));
    }
    let parents = parents_of(root, &sha)?;
    let line = match mainline_args(&parents, opts.mainline) {
        Ok(line) => line,
        Err(problem) => return Ok(refusal("invalid", "That parent does not exist", &problem)),
    };
    let identity = match identity_gate(&opts.identity) {
        Ok(value) => value,
        Err(stop) => return Ok(*stop),
    };
    let mut args = vec!["revert", "--no-edit"];
    args.extend(line.iter().map(String::as_str));
    args.push(&sha);
    let (out, hooks) = run_making(root, &args, &opts.job, None);
    let done = format!("Reverted {}", short_of(&sha));
    let result = pick_outcome(root, "revert", &out, &hooks, &head, &branch, &done);
    let result = if result.ok { result.hint("A new commit undoes it. The old commit stays in the history.") } else { result };
    Ok(result.with("identity", identity).with("from", sha.into()))
}

fn parse_status_z(text: &str) -> Vec<String> {
    let parts: Vec<&str> = text.split('\0').collect();
    let mut files = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let entry = parts[i];
        if entry.len() >= 4 {
            files.push(entry.get(3..).unwrap_or("").to_string());
            if entry.starts_with('R') || entry.starts_with('C') {
                i += 1;
            }
        }
        i += 1;
    }
    files
}

fn tree_paths(text: &str) -> Vec<String> {
    split_z(text)
        .into_iter()
        .filter_map(|entry| {
            let (meta, path) = entry.split_once('\t')?;
            if meta.split(' ').nth(1) == Some("commit") { None } else { Some(path.to_string()) }
        })
        .collect()
}

fn file_hit(top: &Path, rel: &str) -> Option<String> {
    let meta = fs::symlink_metadata(top.join(rel)).ok()?;
    if meta.file_type().is_symlink() {
        return Some(rel.into());
    }
    if meta.is_dir() {
        let mut entries = fs::read_dir(top.join(rel)).ok()?;
        return entries.next().is_some().then(|| format!("{rel}/"));
    }
    Some(rel.into())
}

fn folder_hit(top: &Path, rel: &str) -> Option<String> {
    let meta = fs::symlink_metadata(top.join(rel)).ok()?;
    (meta.file_type().is_symlink() || !meta.is_dir()).then(|| rel.into())
}

pub fn overwritten_untracked(root: &Path, sha: &str) -> Result<Vec<String>, String> {
    let sha = resolve_commit(root, sha).ok_or_else(|| "This commit is not in the repo.".to_string())?;
    let top = read_ok(root, &["rev-parse", "--show-toplevel"])?.trim().to_string();
    if top.is_empty() {
        return Err("This folder has no work tree.".into());
    }
    let top = Path::new(&top);
    let tree_text = read_ok(top, &["ls-tree", "-r", "-z", "--full-tree", &sha])?;
    let index_text = read_ok(top, &["ls-files", "-z", "--full-name"])?;
    let tracked: HashSet<String> = split_z(&index_text).into_iter().collect();
    let leaves: Vec<String> = tree_paths(&tree_text).into_iter().filter(|p| !tracked.contains(p)).collect();
    let mut folders: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for path in &leaves {
        let mut cut = path.rfind('/');
        while let Some(at) = cut.filter(|at| *at > 0) {
            let folder = &path[..at];
            if !seen.insert(folder.to_string()) {
                break;
            }
            folders.push(folder.to_string());
            cut = folder.rfind('/');
        }
    }
    let mut found: BTreeSet<String> = BTreeSet::new();
    found.extend(leaves.iter().filter_map(|p| file_hit(top, p)));
    found.extend(folders.iter().filter(|f| !tracked.contains(*f)).filter_map(|f| folder_hit(top, f)));
    Ok(found.into_iter().collect())
}

pub fn reset_preview(root: &Path, sha: &str) -> Outcome {
    try_reset_preview(root, sha).unwrap_or_else(|e| failure(&e))
}

fn try_reset_preview(root: &Path, sha: &str) -> Result<Outcome, String> {
    let Some(sha) = resolve_commit(root, sha) else { return Ok(missing()) };
    let branch = current_branch(root);
    let Some(head) = head_of(root) else { return Ok(unborn()) };
    let exclude = branch.as_ref().map(|b| format!("--exclude={b}"));
    let mut others: Vec<&str> = exclude.iter().map(String::as_str).collect();
    others.extend(["--branches", "--tags", "--remotes"]);
    let leaving = count_revs(root, &[&format!("{sha}..{head}")])?;
    let joining = count_revs(root, &[&format!("{head}..{sha}")])?;
    let mut only_args = vec![head.as_str(), "--not", sha.as_str()];
    only_args.extend_from_slice(&others);
    let only_here = count_revs(root, &only_args)?;
    let unpushed = count_revs(root, &[&head, "--not", &sha, "--remotes"])?;
    let status = read_ok(root, &["-c", "core.quotepath=off", "status", "--porcelain=v1", "-z", "--untracked-files=no"])?;
    let untracked_lost = overwritten_untracked(root, &sha).ok();
    let files = parse_status_z(&status);
    let shown: Vec<String> = files.iter().take(50).cloned().collect();
    Ok(success("")
        .with("same", (sha == head).into())
        .with("sha", sha.into())
        .with("head", head.into())
        .with("branch", json!(branch))
        .with("leaving", leaving.into())
        .with("joining", joining.into())
        .with("onlyHere", only_here.into())
        .with("pushedLeaving", leaving.saturating_sub(unpushed).into())
        .with("files", json!(shown))
        .with("fileCount", files.len().into())
        .with("untrackedLost", json!(untracked_lost)))
}

fn untracked_gate(root: &Path, sha: &str, known: &[String]) -> Option<Outcome> {
    let lost = match overwritten_untracked(root, sha) {
        Ok(lost) => lost,
        Err(e) => return Some(refusal("untracked", "Cannot check the files that git does not track", "Nothing was changed.").detail(&e)),
    };
    let fresh: Vec<String> = lost.iter().filter(|p| !known.contains(p)).cloned().collect();
    if fresh.is_empty() {
        return None;
    }
    let title = if fresh.len() == 1 {
        "A file that git does not track would be replaced".to_string()
    } else {
        format!("{} files that git does not track would be replaced", fresh.len())
    };
    let detail = fresh.iter().take(12).cloned().collect::<Vec<_>>().join("\n");
    Some(
        refusal("untracked", &title, "Move them first, or open Reset again to see them all. Nothing was changed.")
            .detail(&detail)
            .with("files", json!(fresh))
            .with("untrackedLost", json!(lost)),
    )
}

fn reset_done(mode: &str) -> &'static str {
    match mode {
        "soft" => "The changes are staged.",
        "mixed" => "The changes are in your files, not staged.",
        _ => "Your files match this commit now.",
    }
}

pub fn reset_to(root: &Path, opts: &ResetTo) -> Outcome {
    try_reset_to(root, opts).unwrap_or_else(|e| failure(&e))
}

fn try_reset_to(root: &Path, opts: &ResetTo) -> Result<Outcome, String> {
    let mode = opts.mode.as_str();
    if !RESET_MODES.contains(&mode) {
        return Ok(refusal("invalid", "Pick how to reset", "Soft, mixed or hard."));
    }
    if let Some(busy) = op_gate(root)? {
        return Ok(busy);
    }
    let Some(sha) = resolve_commit(root, &opts.sha) else { return Ok(missing()) };
    let branch = current_branch(root);
    let Some(head) = head_of(root) else { return Ok(unborn()) };
    if let Some(stale) = stale_refusal(&opts.expect_head, &opts.expect_branch, &head, &branch) {
        return Ok(stale);
    }
    if mode == "hard"
        && let Some(lost) = untracked_gate(root, &sha, &opts.expect_untracked)
    {
        return Ok(lost);
    }
    let flag = format!("--{mode}");
    let out = git(root, &["reset", &flag, "-q", &sha, "--"]);
    if !out.ok() {
        return Ok(failure(&out.said()));
    }
    Ok(success(&format!("Moved {} to {}", branch.as_deref().unwrap_or("HEAD"), short_of(&sha)))
        .hint(&format!("{} It was at {} before.", reset_done(mode), short_of(&head)))
        .with("mode", mode.into())
        .with("sha", sha.into())
        .with("from", head.into())
        .with("branch", json!(branch)))
}

pub fn undo_last_commit(root: &Path, opts: &UndoLastCommit) -> Outcome {
    try_undo_last_commit(root, opts).unwrap_or_else(|e| failure(&e))
}

fn try_undo_last_commit(root: &Path, opts: &UndoLastCommit) -> Result<Outcome, String> {
    if let Some(busy) = op_gate(root)? {
        return Ok(busy);
    }
    let branch = current_branch(root);
    let Some(head) = head_of(root) else { return Ok(refusal("unborn", "There is no commit to undo", "")) };
    if let Some(stale) = stale_refusal(&opts.expect_head, &opts.expect_branch, &head, &branch) {
        return Ok(stale);
    }
    let parents = parents_of(root, &head)?;
    if parents.is_empty() {
        return Ok(refusal("root", "This is the first commit", "It has nothing before it, so it cannot be undone this way."));
    }
    if parents.len() > 1 {
        return Ok(refusal("merge", "The last commit is a merge", "Use Reset to here on the commit you want to go back to."));
    }
    let remote_branches = remote_branches_with(root, &head)?;
    if !remote_branches.is_empty() {
        return Ok(refusal("pushed", "This commit is already on the remote", "Undo would make your branch differ from the remote. Use Revert instead.")
            .with("remoteBranches", json!(remote_branches)));
    }
    let text = read_ok(root, &["log", "-1", "--no-show-signature", "--format=%B", &head, "--"])?;
    let out = git(root, &["reset", "--soft", "-q", &parents[0], "--"]);
    if !out.ok() {
        return Ok(failure(&out.said()));
    }
    let message: Message = split_message_text(&text);
    Ok(success("Last commit undone")
        .hint("Its changes are staged, so nothing was lost.")
        .with("sha", parents[0].clone().into())
        .with("from", head.into())
        .with("message", json!(message)))
}

pub fn parse_chain(text: &str) -> Vec<ChainCommit> {
    text.split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split('\0').collect();
            let at = |k: usize| parts.get(k).copied().unwrap_or("");
            let parents = at(2).trim();
            ChainCommit {
                sha: at(0).into(),
                tree: at(1).into(),
                parents: if parents.is_empty() { Vec::new() } else { parents.split_whitespace().map(String::from).collect() },
                name: at(3).into(),
                email: at(4).into(),
                date: at(5).trim().into(),
                encoding: at(6).trim().into(),
            }
        })
        .collect()
}

pub fn chain_problem(chain: &[ChainCommit], sha: &str, head: &str, parents: &[String]) -> bool {
    let (Some(first), Some(last)) = (chain.first(), chain.last()) else { return true };
    if first.sha != sha || last.sha != head {
        return true;
    }
    if first.parents.join(" ") != parents.join(" ") {
        return true;
    }
    chain.iter().enumerate().any(|(k, c)| {
        !is_full_sha(&c.sha)
            || !is_full_sha(&c.tree)
            || !RAW_DATE.is_match(&c.date)
            || (!c.encoding.is_empty() && !SAFE_ENCODING.is_match(&c.encoding))
            || (k > 0 && (c.parents.len() != 1 || c.parents[0] != chain[k - 1].sha))
    })
}

fn read_chain(root: &Path, head: &str, parents: &[String]) -> Result<Vec<ChainCommit>, String> {
    let format = format!("--format={CHAIN_FORMAT}");
    let mut args = vec!["log", "--no-show-signature", "--topo-order", "--reverse", "--date=raw", &format, head];
    if !parents.is_empty() {
        args.push("--not");
        args.extend(parents.iter().map(String::as_str));
    }
    args.push("--");
    Ok(parse_chain(&read_ok(root, &args)?))
}

fn rebuild_chain(root: &Path, chain: &[ChainCommit], file: &Path, dir: &Path, sign: bool, job: &Job) -> Result<String, Box<Outcome>> {
    let mut parent = chain.first().and_then(|c| c.parents.first().cloned());
    for (k, c) in chain.iter().enumerate() {
        if job.cancelled() {
            return Err(Box::new(refusal("cancelled", "Stopped", "Nothing was changed.")));
        }
        let encoding = if c.encoding.is_empty() { "UTF-8" } else { c.encoding.as_str() };
        let text = if k == 0 { file.to_path_buf() } else { dir.join(format!("m{k}")) };
        let text = text.to_string_lossy().into_owned();
        if k > 0 {
            let enc = format!("--encoding={encoding}");
            let output = format!("--output={text}");
            let out = git(root, &["log", "-1", "--no-show-signature", "--no-notes", "--no-color", &enc, "--pretty=format:%B", &output, &c.sha, "--"]);
            if !out.ok() {
                return Err(Box::new(unchanged(failure(&out.said()))));
            }
        }
        let env = vec![
            ("GIT_AUTHOR_NAME".to_string(), c.name.clone()),
            ("GIT_AUTHOR_EMAIL".to_string(), c.email.clone()),
            ("GIT_AUTHOR_DATE".to_string(), format!("@{}", c.date)),
        ];
        let config = format!("i18n.commitEncoding={encoding}");
        let mut args: Vec<&str> = Vec::new();
        if k > 0 {
            args.extend(["-c", &config]);
        }
        args.push("commit-tree");
        if sign {
            args.push("-S");
        }
        args.push(&c.tree);
        if let Some(p) = &parent {
            args.extend(["-p", p]);
        }
        args.extend(["-F", &text]);
        let made = git_env(root, &args, &env);
        let next = made.stdout.trim().to_string();
        if !made.ok() || !is_full_sha(&next) {
            return Err(Box::new(unchanged(failure(&made.said()))));
        }
        parent = Some(next);
    }
    parent.ok_or_else(|| Box::new(unchanged(failure("no commits to rebuild"))))
}

fn run_message_hook(root: &Path, file: &Path, job: &Job) -> Option<Outcome> {
    let hooks = ops::commit_hooks(root);
    if !hooks.iter().any(|h| h == "commit-msg") {
        return None;
    }
    let path = file.to_string_lossy().into_owned();
    let (out, hooks) = run_making(root, &["hook", "run", "--ignore-missing", "commit-msg", "--", &path], job, Some(hooks));
    if let Some(stop) = job_stopped(&out) {
        return Some(stop.hint("Nothing was changed."));
    }
    if !out.ok() {
        return Some(unchanged(hook_kind(failure(&out.said()), &hooks)));
    }
    if let Ok(after) = fs::read_to_string(file)
        && after.trim().is_empty()
    {
        return Some(refusal("empty", "The commit-msg hook left the message empty", "Nothing was changed."));
    }
    None
}

struct Older<'a> {
    sha: &'a str,
    head: &'a str,
    branch: &'a Option<String>,
    parents: &'a [String],
    file: &'a Path,
}

fn reword_older(root: &Path, older: &Older, job: &Job) -> Result<Result<String, Outcome>, String> {
    let chain = read_chain(root, older.head, older.parents)?;
    if chain_problem(&chain, older.sha, older.head, older.parents) {
        return Ok(Err(refusal("chain", "Cannot change this message here", "The commits after it could not be read in order. Nothing was changed.")));
    }
    if let Some(hook) = run_message_hook(root, older.file, job) {
        return Ok(Err(hook));
    }
    let sign = read(root, &["config", "--bool", "commit.gpgSign"]).stdout.trim() == "true";
    let dir = tempfile::Builder::new().prefix("nixa-git-hist.").tempdir().map_err(|e| format!("cannot make a folder for the message: {e}"))?;
    let built = match rebuild_chain(root, &chain, older.file, dir.path(), sign, job) {
        Ok(head) => head,
        Err(stop) => return Ok(Err(*stop)),
    };
    if current_branch(root) != *older.branch {
        return Ok(Err(branch_changed()));
    }
    let reference = older.branch.as_ref().map(|b| format!("refs/heads/{b}")).unwrap_or_else(|| "HEAD".into());
    let note = format!("edit message of {}", short_of(older.sha));
    let moved = git(root, &["update-ref", "-m", &note, &reference, &built, older.head]);
    if !moved.ok() {
        let text = moved.said();
        return Ok(Err(if REF_MOVED.is_match(&text) { head_changed() } else { unchanged(failure(&text)) }));
    }
    Ok(Ok(built))
}

pub fn reword_commit(root: &Path, opts: &RewordCommit) -> Outcome {
    try_reword_commit(root, opts).unwrap_or_else(|e| failure(&e))
}

fn try_reword_commit(root: &Path, opts: &RewordCommit) -> Result<Outcome, String> {
    if opts.summary.trim().is_empty() {
        return Ok(refusal("empty", "Write a summary first", "The first line says what changed."));
    }
    let message = compose_message(&opts.summary, &opts.body);
    if let Some(busy) = op_gate(root)? {
        return Ok(busy);
    }
    let Some(sha) = resolve_commit(root, &opts.sha) else { return Ok(missing()) };
    let branch = current_branch(root);
    let Some(head) = head_of(root) else { return Ok(unborn()) };
    if let Some(stale) = stale_refusal(&opts.expect_head, &opts.expect_branch, &head, &branch) {
        return Ok(stale);
    }
    if !on_line(root, &sha, Some(&head)) {
        return Ok(refusal(
            "notOnHead",
            &format!("This commit is not on {}", place_name(&branch)),
            "You can only change the message of a commit on the branch you are on.",
        ));
    }
    let old = read_ok(root, &["log", "-1", "--no-show-signature", "--format=%B", &sha, "--"])?;
    let was = split_message_text(&old);
    if compose_message(&was.summary, &was.body) == message {
        return Ok(refusal("same", "The message is the same", "Nothing was changed."));
    }
    let parents = parents_of(root, &sha)?;
    if sha != head {
        if parents.len() > 1 {
            return Ok(refusal("merge", "This commit is a merge", "Only the newest commit's message can be changed when it is a merge."));
        }
        let merges = count_revs(root, &["--min-parents=2", &format!("{sha}..{head}")])?;
        if merges > 0 {
            return Ok(refusal(
                "mergeBetween",
                "There is a merge after this commit",
                "Older commits cannot be changed when a merge comes after them. Nothing was changed.",
            ));
        }
    }
    let pushed = !remote_branches_with(root, &sha)?.is_empty();
    let identity = match identity_gate(&opts.identity) {
        Ok(value) => value,
        Err(stop) => return Ok(*stop),
    };
    let written = temp_text("nixa-git-hist.", &message)?;
    let after = if sha == head {
        let path = written.path().to_string_lossy().into_owned();
        let (out, hooks) = run_making(root, &["commit", "--amend", "--only", "-F", &path], &opts.job, None);
        if let Some(stop) = job_stopped(&out) {
            return Ok(stop);
        }
        if !out.ok() {
            return Ok(hook_kind(failure(&out.said()), &hooks));
        }
        head_of(root)
    } else {
        let older = Older { sha: &sha, head: &head, branch: &branch, parents: &parents, file: written.path() };
        match reword_older(root, &older, &opts.job)? {
            Ok(built) => Some(built),
            Err(stop) => return Ok(stop),
        }
    };
    let hint = if pushed { "This commit was already on the remote. You need a force push to update it there." } else { "" };
    let short = after.as_deref().map(short_of).unwrap_or_default();
    Ok(success("Message changed")
        .hint(hint)
        .with("sha", json!(after))
        .with("short", short.into())
        .with("pushed", pushed.into())
        .with("autostashClash", false.into())
        .with("identity", identity)
        .with("from", sha.into())
        .with("branch", json!(branch)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrepo;

    fn head(repo: &testrepo::Repo) -> String {
        repo.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn remote(name: &str, web: Option<&str>) -> Remote {
        Remote { name: name.into(), web_url: web.map(String::from), ..Remote::default() }
    }

    #[test]
    fn small_helpers_match_the_rules() {
        assert_eq!(rev_problem(""), Some("No commit was given."));
        assert_eq!(rev_problem("-x"), Some("That is not a commit id."));
        assert_eq!(rev_problem("a..b"), Some("That is not a commit id."));
        assert_eq!(rev_problem("HEAD~1"), None);
        let text = "error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt\n\tb.txt\nPlease commit your changes or stash them before you switch branches.";
        assert_eq!(blocked_files(text), vec!["a.txt", "b.txt"]);
        assert_eq!(commit_url("https://gitlab.com/a/b", "abc").unwrap(), "https://gitlab.com/a/b/-/commit/abc");
        assert_eq!(commit_url("https://bitbucket.org/a/b", "abc").unwrap(), "https://bitbucket.org/a/b/commits/abc");
        assert_eq!(commit_url("https://github.com/a/b", "abc").unwrap(), "https://github.com/a/b/commit/abc");
        assert_eq!(commit_url("", "abc"), None);
        assert_eq!(forge_name("GitHub.com"), "GitHub");
        assert_eq!(forge_name("codeberg.org"), "Codeberg");
        assert_eq!(forge_name(""), "the web");
        assert_eq!(forge_name("git.example.com"), "git.example.com");
    }

    #[test]
    fn web_remote_prefers_the_one_that_holds_the_commit() {
        let remotes = vec![remote("origin", Some("https://github.com/a/b")), remote("fork", Some("https://gitlab.com/c/d")), remote("plain", None)];
        assert_eq!(pick_web_remote(&remotes, &["fork/main".into()], Some("origin/main")).unwrap().name, "fork");
        assert_eq!(pick_web_remote(&remotes, &[], Some("fork/main")).unwrap().name, "fork");
        assert_eq!(pick_web_remote(&remotes, &[], None).unwrap().name, "origin");
        assert_eq!(pick_web_remote(&[remote("plain", None)], &[], None), None);
        let web = web_for(&remotes, &["fork/main".into()], None, "abc").unwrap();
        assert_eq!(web.name, "GitLab");
        assert_eq!(web.url.as_deref(), Some("https://gitlab.com/c/d/-/commit/abc"));
    }

    #[test]
    fn parses_info_and_chain_lines() {
        let info = parse_info("abc\x00ab\x00p1 p2\x00Ann\x00ann@x\x00123\x00Subject\n\nBody text\n");
        assert_eq!(info.sha, "abc");
        assert_eq!(info.parents, vec!["p1", "p2"]);
        assert_eq!(info.at, 123);
        assert_eq!(info.subject, "Subject");
        assert!(info.body.contains("Body text"));
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let t = "c".repeat(40);
        let text = format!("{a}\x00{t}\x00\x00Ann\x00ann@x\x001 +0000\x00\n{b}\x00{t}\x00{a}\x00Ann\x00ann@x\x002 +0000\x00\n");
        let chain = parse_chain(&text);
        assert_eq!(chain.len(), 2);
        assert!(!chain_problem(&chain, &a, &b, &[]));
        assert!(chain_problem(&chain, &b, &b, &[]));
        assert!(chain_problem(&chain, &a, &b, std::slice::from_ref(&t)));
    }

    #[test]
    fn commit_info_reads_a_commit() {
        let repo = testrepo::new();
        let first = head(&repo);
        let second = repo.commit_file("b.txt", "b\n", "second\n\nmore words");
        let info = commit_info(repo.root(), &first).unwrap();
        assert_eq!(info.info.subject, "first");
        assert!(info.is_root && info.on_head && !info.is_head && !info.pushed && !info.detached);
        assert_eq!(info.after, 1);
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert!(info.refs.branches.is_empty());
        let info = commit_info(repo.root(), &second).unwrap();
        assert_eq!(info.info.subject, "second");
        assert!(info.info.body.contains("more words"));
        assert_eq!(info.info.parents, vec![first]);
        assert!(info.is_head);
        assert_eq!(info.refs.branches, vec!["main"]);
        assert_eq!(info.info.short, short_of(&second));
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["isHead"], true);
        assert_eq!(json["subject"], "second");
        assert!(commit_info(repo.root(), "nope").is_err());
        let (pushed, _remote) = testrepo::with_remote();
        let info = commit_info(pushed.root(), "HEAD").unwrap();
        assert!(info.pushed);
        assert_eq!(info.remote_branches, vec!["origin/main"]);
        assert_eq!(info.upstream.as_deref(), Some("origin/main"));
    }

    #[test]
    fn checkout_commit_warns_before_leaving_commits() {
        let repo = testrepo::new();
        let first = head(&repo);
        repo.commit_file("b.txt", "b\n", "second");
        let out = checkout_commit(repo.root(), &CheckoutCommit { sha: first.clone(), force: false });
        assert!(out.ok, "{out:?}");
        assert!(out.flag("detached"));
        assert_eq!(out.text("from"), Some("main"));
        assert_eq!(head(&repo), first);
        let again = checkout_commit(repo.root(), &CheckoutCommit { sha: first.clone(), force: false });
        assert!(again.ok && again.flag("already"));
        repo.commit_file("c.txt", "c\n", "loose");
        let out = checkout_commit(repo.root(), &CheckoutCommit { sha: first.clone(), force: false });
        assert_eq!(out.kind, "leaving");
        assert_eq!(out.title, "1 commit would be left behind");
        let out = checkout_commit(repo.root(), &CheckoutCommit { sha: first.clone(), force: true });
        assert!(out.ok, "{out:?}");
        assert_eq!(checkout_commit(repo.root(), &CheckoutCommit { sha: "nope".into(), force: false }).kind, "notFound");
    }

    #[test]
    fn branch_and_tag_at_a_commit() {
        let repo = testrepo::new();
        let first = head(&repo);
        repo.commit_file("b.txt", "b\n", "second");
        let out = create_branch_at(repo.root(), &CreateBranchAt { name: "old".into(), sha: first.clone(), checkout: false });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.text("sha"), Some(first.as_str()));
        assert_eq!(repo.git(&["rev-parse", "old"]).trim(), first);
        assert_eq!(current_branch(repo.root()).as_deref(), Some("main"));
        let out = create_branch_at(repo.root(), &CreateBranchAt { name: "work".into(), sha: first.clone(), checkout: true });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Now on new branch work");
        assert_eq!(current_branch(repo.root()).as_deref(), Some("work"));
        let out = create_branch_at(repo.root(), &CreateBranchAt { name: "work".into(), sha: first.clone(), checkout: true });
        assert_eq!(out.kind, "exists");
        let out = create_branch_at(repo.root(), &CreateBranchAt { name: "bad name".into(), sha: first.clone(), checkout: true });
        assert_eq!(out.kind, "invalid");
        let out = create_tag(repo.root(), &CreateTag { name: "v1".into(), sha: first.clone(), ..CreateTag::default() });
        assert!(out.ok && !out.flag("annotated"), "{out:?}");
        assert_eq!(repo.git(&["cat-file", "-t", "v1"]).trim(), "commit");
        let out = create_tag(repo.root(), &CreateTag { name: "v2".into(), sha: "main".into(), message: "Release\r\n".into(), ..CreateTag::default() });
        assert!(out.ok && out.flag("annotated"), "{out:?}");
        assert_eq!(repo.git(&["cat-file", "-t", "v2"]).trim(), "tag");
        assert!(repo.git(&["tag", "-l", "--format=%(contents)", "v2"]).contains("Release"));
        let out = create_tag(repo.root(), &CreateTag { name: "v1".into(), sha: first, ..CreateTag::default() });
        assert_eq!(out.kind, "exists");
    }

    #[test]
    fn cherry_pick_clean_and_conflict() {
        let repo = testrepo::new();
        repo.git(&["switch", "-q", "-c", "side"]);
        let pick = repo.commit_file("b.txt", "b\n", "add b");
        let clash = repo.commit_file("a.txt", "one\nSIDE\nthree\n", "side change");
        repo.git(&["switch", "-q", "main"]);
        let out = cherry_pick(repo.root(), &CherryPick { sha: pick.clone(), record_origin: true, ..CherryPick::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, format!("Cherry-picked {} onto main", short_of(&pick)));
        assert_eq!(out.text("from"), Some(pick.as_str()));
        assert_eq!(repo.read("b.txt"), "b\n");
        assert!(repo.git(&["log", "-1", "--format=%B"]).contains("cherry picked from commit"));
        let here = head(&repo);
        assert_eq!(cherry_pick(repo.root(), &CherryPick { sha: here.clone(), ..CherryPick::default() }).kind, "onHead");
        let stale = cherry_pick(repo.root(), &CherryPick { sha: clash.clone(), expect_head: Some("0".repeat(40)), ..CherryPick::default() });
        assert_eq!(stale.kind, "headChanged");
        let stale = cherry_pick(repo.root(), &CherryPick { sha: clash.clone(), expect_branch: Some(None), ..CherryPick::default() });
        assert_eq!(stale.title, "You are on another branch now");
        repo.commit_file("a.txt", "one\nMAIN\nthree\n", "main change");
        let out = cherry_pick(repo.root(), &CherryPick { sha: clash.clone(), ..CherryPick::default() });
        assert_eq!(out.kind, "conflict", "{out:?}");
        assert_eq!(out.title, "The cherry-pick stopped on a conflict");
        assert_eq!(out.get("conflicts"), Some(&json!(["a.txt"])));
        let busy = cherry_pick(repo.root(), &CherryPick { sha: clash, ..CherryPick::default() });
        assert_eq!(busy.kind, "busy");
        assert_eq!(busy.title, "Finish the cherry-pick first");
    }

    #[test]
    fn revert_undoes_a_commit() {
        let repo = testrepo::new();
        let change = repo.commit_file("a.txt", "one\nTWO\nthree\n", "change");
        let out = revert_commit(repo.root(), &RevertCommit { sha: change.clone(), ..RevertCommit::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, format!("Reverted {}", short_of(&change)));
        assert_eq!(out.hint, "A new commit undoes it. The old commit stays in the history.");
        assert_eq!(repo.read("a.txt"), "one\ntwo\nthree\n");
        repo.git(&["switch", "-q", "-c", "side"]);
        let other = repo.commit_file("b.txt", "b\n", "side only");
        repo.git(&["switch", "-q", "main"]);
        let out = revert_commit(repo.root(), &RevertCommit { sha: other, ..RevertCommit::default() });
        assert_eq!(out.kind, "notOnHead");
        assert_eq!(out.title, "This commit is not on main");
    }

    #[test]
    fn reset_preview_and_reset_modes() {
        let repo = testrepo::new();
        let first = head(&repo);
        let second = repo.commit_file("b.txt", "b\n", "second");
        let third = repo.commit_file("c.txt", "c\n", "third");
        let preview = reset_preview(repo.root(), &first);
        assert!(preview.ok, "{preview:?}");
        assert_eq!(preview.get("leaving"), Some(&json!(2)));
        assert_eq!(preview.get("joining"), Some(&json!(0)));
        assert_eq!(preview.get("onlyHere"), Some(&json!(2)));
        assert_eq!(preview.get("pushedLeaving"), Some(&json!(0)));
        assert_eq!(preview.get("untrackedLost"), Some(&json!([])));
        assert!(!preview.flag("same"));
        let bad = reset_to(repo.root(), &ResetTo { sha: first.clone(), mode: "wild".into(), ..ResetTo::default() });
        assert_eq!(bad.kind, "invalid");
        let stale = reset_to(repo.root(), &ResetTo { sha: first.clone(), mode: "soft".into(), expect_head: Some(first.clone()), ..ResetTo::default() });
        assert_eq!(stale.kind, "headChanged");
        let out = reset_to(repo.root(), &ResetTo { sha: second.clone(), mode: "soft".into(), expect_head: Some(third.clone()), ..ResetTo::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, format!("Moved main to {}", short_of(&second)));
        assert_eq!(out.hint, format!("The changes are staged. It was at {} before.", short_of(&third)));
        assert_eq!(repo.git(&["diff", "--cached", "--name-only"]).trim(), "c.txt");
        let out = reset_to(repo.root(), &ResetTo { sha: first.clone(), mode: "mixed".into(), ..ResetTo::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(repo.git(&["diff", "--cached", "--name-only"]).trim(), "");
        assert_eq!(repo.git(&["status", "--porcelain"]), "?? b.txt\n?? c.txt\n");
        let preview = reset_preview(repo.root(), &third);
        assert_eq!(preview.get("untrackedLost"), Some(&json!(["b.txt", "c.txt"])));
        assert_eq!(preview.get("joining"), Some(&json!(2)));
        let out = reset_to(repo.root(), &ResetTo { sha: third.clone(), mode: "hard".into(), ..ResetTo::default() });
        assert_eq!(out.kind, "untracked");
        assert_eq!(out.title, "2 files that git does not track would be replaced");
        assert_eq!(head(&repo), first);
        let out = reset_to(repo.root(), &ResetTo { sha: third.clone(), mode: "hard".into(), expect_untracked: vec!["b.txt".into()], ..ResetTo::default() });
        assert_eq!(out.title, "A file that git does not track would be replaced");
        let out = reset_to(repo.root(), &ResetTo { sha: third.clone(), mode: "hard".into(), expect_untracked: vec!["b.txt".into(), "c.txt".into()], ..ResetTo::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(head(&repo), third);
        assert_eq!(repo.read("c.txt"), "c\n");
        assert_eq!(repo.git(&["status", "--porcelain"]), "");
    }

    #[test]
    fn overwritten_untracked_sees_folders_and_files() {
        let repo = testrepo::new();
        let first = head(&repo);
        repo.commit_file("d/x.txt", "x\n", "folder");
        repo.commit_file("e/y.txt", "y\n", "other folder");
        let last = head(&repo);
        repo.git(&["reset", "-q", "--hard", &first]);
        repo.write("d/x.txt", "mine\n");
        repo.write("e", "a file where a folder goes\n");
        assert_eq!(overwritten_untracked(repo.root(), &last).unwrap(), vec!["d/x.txt", "e"]);
        fs::remove_file(repo.path("d/x.txt")).unwrap();
        repo.write("d/x.txt/inner", "deep\n");
        assert_eq!(overwritten_untracked(repo.root(), &last).unwrap(), vec!["d/x.txt/", "e"]);
        assert!(overwritten_untracked(repo.root(), "nope").is_err());
    }

    #[test]
    fn undo_last_commit_keeps_changes_staged() {
        let repo = testrepo::new();
        let first = head(&repo);
        assert_eq!(undo_last_commit(repo.root(), &UndoLastCommit::default()).kind, "root");
        let second = repo.commit_file("b.txt", "b\n", "second\n\nwhy it changed");
        let out = undo_last_commit(repo.root(), &UndoLastCommit { expect_head: Some(second.clone()), expect_branch: Some(Some("main".into())) });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Last commit undone");
        assert_eq!(head(&repo), first);
        assert_eq!(out.text("from"), Some(second.as_str()));
        assert_eq!(out.get("message").and_then(|m| m.get("summary")), Some(&json!("second")));
        assert_eq!(repo.git(&["diff", "--cached", "--name-only"]).trim(), "b.txt");
        let (pushed, _remote) = testrepo::with_remote();
        pushed.commit_file("b.txt", "b\n", "second");
        pushed.git(&["push", "-q"]);
        let out = undo_last_commit(pushed.root(), &UndoLastCommit::default());
        assert_eq!(out.kind, "pushed");
        assert_eq!(out.get("remoteBranches"), Some(&json!(["origin/main"])));
    }

    #[test]
    fn reword_head_commit() {
        let repo = testrepo::new();
        let second = repo.commit_file("b.txt", "b\n", "second");
        let out = reword_commit(repo.root(), &RewordCommit { sha: "HEAD".into(), summary: "better words".into(), body: "more".into(), ..RewordCommit::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Message changed");
        assert_eq!(out.text("from"), Some(second.as_str()));
        assert_eq!(out.text("sha"), Some(head(&repo).as_str()));
        assert_eq!(repo.git(&["log", "-1", "--format=%s"]).trim(), "better words");
        assert_eq!(repo.git(&["log", "-1", "--format=%b"]).trim(), "more");
        let same = reword_commit(repo.root(), &RewordCommit { sha: "HEAD".into(), summary: "better words".into(), body: "more".into(), ..RewordCommit::default() });
        assert_eq!(same.kind, "same");
        let empty = reword_commit(repo.root(), &RewordCommit { sha: "HEAD".into(), summary: "  ".into(), ..RewordCommit::default() });
        assert_eq!(empty.kind, "empty");
    }

    #[test]
    fn reword_older_commit_rebuilds_the_chain() {
        let repo = testrepo::new();
        let first = head(&repo);
        repo.commit_file("b.txt", "b\n", "second");
        let third = repo.commit_file("c.txt", "c\n", "third");
        let tree = repo.git(&["rev-parse", "HEAD^{tree}"]);
        let dates = repo.git(&["log", "--format=%an %ae %ad", "--date=raw"]);
        let out = reword_commit(repo.root(), &RewordCommit { sha: first.clone(), summary: "new first".into(), body: "why".into(), ..RewordCommit::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.text("branch"), Some("main"));
        assert!(!out.flag("pushed"));
        assert_ne!(head(&repo), third);
        assert_eq!(out.text("sha"), Some(head(&repo).as_str()));
        assert_eq!(repo.git(&["log", "--format=%s", "main"]), "third\nsecond\nnew first\n");
        assert_eq!(repo.git(&["log", "--format=%B", "-1", "main~2"]).trim(), "new first\n\nwhy");
        assert_eq!(repo.git(&["rev-parse", "HEAD^{tree}"]), tree);
        assert_eq!(repo.git(&["log", "--format=%an %ae %ad", "--date=raw"]), dates);
        assert_eq!(current_branch(repo.root()).as_deref(), Some("main"));
        assert!(repo.git(&["reflog", "-1", "main"]).contains("edit message of"));
    }
}
