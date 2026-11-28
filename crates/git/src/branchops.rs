use crate::branches::{self, CreateBranch};
use crate::errors::describe;
use crate::history;
use crate::ops::{self, Identity, State, branch_name_problem, op_word, read_state, remote_name_problem};
use crate::outcome::{Outcome, failure, plural, refusal, stopped, success};
use crate::remote;
use crate::run::{self, Job, Out};
use crate::stash::{self, StashPush, StashUse};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SWITCH_TAG: &str = "nixa-switch:";

const CLASH_HINT: &str = "Your own changes did not fit with the new commits. They are kept in the stash, and the clashing files are marked. Fix those files, or open the stash window.";

const CONFLICT_HINT: &str = "Fix the files in the Changes panel, then press Continue.";

const MIXED_TITLE: &str = "Some files have staged and unstaged parts";

const MIXED_HINT: &str = "Commit or stash them first.";

const REF_FORMAT: &str = "%(refname)%00%(objectname)%00%(HEAD)%00%(worktreepath)%00%(upstream)%00%(upstream:short)";

const PATH_CHUNK: usize = 200;

const PART_WAY: &str = "Git was stopped part way. Check the Changes panel.";

static LEFT_STASH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^On .*?: {} left (\S+) for (\S+)$", regex::escape(SWITCH_TAG))).expect("left stash")
});

type Stop = Box<Outcome>;

type Flow = Result<Outcome, Stop>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCounts {
    pub dirty: usize,
    pub staged: usize,
    pub mixed: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePlan {
    pub name: String,
    pub mode: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compare {
    pub a: String,
    pub b: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameBranch {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default)]
pub struct FastForward {
    pub branch: String,
    pub job: Job,
}

#[derive(Debug, Clone, Default)]
pub struct MergeInto {
    pub name: String,
    pub no_ff: bool,
    pub expect_head: Option<String>,
    pub expect_target: Option<String>,
    pub identity: Identity,
    pub job: Job,
}

#[derive(Debug, Clone, Default)]
pub struct RebaseOnto {
    pub name: String,
    pub expect_head: Option<String>,
    pub expect_target: Option<String>,
    pub identity: Identity,
    pub job: Job,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteBranch {
    pub remote: String,
    pub branch: String,
}

#[derive(Debug, Clone, Default)]
pub struct DeleteRemoteBranch {
    pub remote: String,
    pub branch: String,
    pub expect_sha: Option<String>,
    pub job: Job,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashThenSwitch {
    pub to: String,
    pub from: String,
    pub create: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeftStash {
    pub reference: String,
    pub sha: String,
    pub to: String,
}

#[derive(Debug, Clone, Default)]
struct RefRow {
    name: String,
    sha: Option<String>,
    current: bool,
    worktree: Option<String>,
    upstream: Option<String>,
    upstream_short: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Found {
    local: Option<RefRow>,
    remote: Option<RefRow>,
    tag: Option<RefRow>,
}

#[derive(Debug, Clone, Default)]
struct Prepared {
    current: String,
    target: RefRow,
    head: String,
    theirs: String,
    ambiguous: bool,
}

#[derive(Debug, Clone, Default)]
struct StatusRow {
    x: char,
    y: char,
}

fn settle(flow: Flow) -> Outcome {
    flow.unwrap_or_else(|stop| *stop)
}

fn look(root: &Path, args: &[&str]) -> Out {
    run::read(root, args)
}

fn broke(out: &Out, what: &str) -> Outcome {
    let text = out.said();
    failure(if text.is_empty() { what } else { &text })
}

fn non_empty(text: &str) -> Option<String> {
    if text.is_empty() { None } else { Some(text.to_string()) }
}

fn job_stop(out: &Out, hint: &str) -> Option<Outcome> {
    if out.cancelled {
        return Some(refusal("cancelled", "Stopped", hint).with("cancelled", true.into()));
    }
    stopped(out)
}

fn state_value(state: &Option<State>) -> Value {
    serde_json::to_value(state).unwrap_or(Value::Null)
}

fn lookup(root: &Path, name: &str) -> Result<Found, Stop> {
    let format = format!("--format={REF_FORMAT}");
    let heads = format!("refs/heads/{name}");
    let remotes = format!("refs/remotes/{name}");
    let tags = format!("refs/tags/{name}");
    let out = look(root, &["for-each-ref", &format, &heads, &remotes, &tags]);
    if !out.ok() {
        return Err(broke(&out, "cannot read the branches").into());
    }
    let rows: Vec<RefRow> = out
        .stdout
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split('\0').collect();
            let part = |i: usize| parts.get(i).copied().unwrap_or("");
            let current = part(2) == "*";
            RefRow {
                name: part(0).to_string(),
                sha: non_empty(part(1)),
                current,
                worktree: if current { None } else { non_empty(part(3)) },
                upstream: non_empty(part(4)),
                upstream_short: non_empty(part(5)),
            }
        })
        .collect();
    let pick = |full: &str| rows.iter().find(|row| row.name == full).cloned();
    Ok(Found { local: pick(&heads), remote: pick(&remotes), tag: pick(&tags) })
}

fn commit_of(root: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    let out = look(root, &["rev-parse", "--verify", "-q", "--end-of-options", &spec]);
    if out.ok() { non_empty(out.stdout.trim()) } else { None }
}

fn is_ancestor(root: &Path, older: &str, newer: &str) -> Result<bool, Stop> {
    let out = look(root, &["merge-base", "--is-ancestor", older, newer]);
    match out.code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(broke(&out, "cannot compare the commits").into()),
    }
}

fn count_both(root: &Path, a: &str, b: &str) -> Result<(usize, usize), Stop> {
    let range = format!("{a}...{b}");
    let out = look(root, &["rev-list", "--left-right", "--count", &range]);
    if !out.ok() {
        return Err(broke(&out, "cannot count the commits").into());
    }
    let nums: Vec<usize> = out.stdout.split_whitespace().map(|v| v.parse().unwrap_or(0)).collect();
    Ok((nums.first().copied().unwrap_or(0), nums.get(1).copied().unwrap_or(0)))
}

fn count_only(root: &Path, args: &[&str]) -> Result<usize, Stop> {
    let mut all = vec!["rev-list", "--count"];
    all.extend_from_slice(args);
    let out = look(root, &all);
    if !out.ok() {
        return Err(broke(&out, "cannot count the commits").into());
    }
    Ok(out.stdout.trim().parse().unwrap_or(0))
}

fn branch_here(root: &Path) -> Option<String> {
    let out = run::git(root, &["symbolic-ref", "-q", "--short", "HEAD"]);
    if out.ok() { non_empty(out.stdout.trim()) } else { None }
}

fn after_state(root: &Path) -> Option<State> {
    read_state(root).ok()
}

fn step_title(state: &Option<State>, fallback: &str) -> String {
    match state.as_ref().and_then(|s| s.step) {
        Some(step) => format!("Step {} of {} has conflicts", step.n, step.total),
        None => fallback.to_string(),
    }
}

fn renamed(part: &str) -> bool {
    let b = part.as_bytes();
    b.len() > 2 && b[2] == b' ' && (matches!(b[0], b'R' | b'C') || matches!(b[1], b'R' | b'C'))
}

fn status_rows(stdout: &str) -> Vec<StatusRow> {
    let parts: Vec<&str> = stdout.split('\0').collect();
    let mut rows = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let part = parts[i];
        if part.len() >= 4 {
            let mut chars = part.chars();
            let x = chars.next().unwrap_or(' ');
            let y = chars.next().unwrap_or(' ');
            rows.push(StatusRow { x, y });
            if renamed(part) {
                i += 1;
            }
        }
        i += 1;
    }
    rows
}

fn tracked_rows(root: &Path) -> Vec<StatusRow> {
    let out = look(root, &["-c", "core.quotepath=off", "status", "--porcelain=v1", "-z", "--untracked-files=no"]);
    if out.ok() { status_rows(&out.stdout) } else { Vec::new() }
}

fn unmerged(row: &StatusRow) -> bool {
    row.x == 'U' || row.y == 'U' || (row.x == row.y && (row.x == 'A' || row.x == 'D'))
}

fn stage_counts(rows: &[StatusRow]) -> StageCounts {
    StageCounts {
        dirty: rows.len(),
        staged: rows.iter().filter(|row| row.x != ' ' && row.x != 'A').count(),
        mixed: rows.iter().filter(|row| row.x != ' ' && row.y != ' ' && !unmerged(row)).count(),
    }
}

pub fn local_changes(root: &Path) -> usize {
    tracked_rows(root).len()
}

pub fn staged_state(root: &Path) -> StageCounts {
    stage_counts(&tracked_rows(root))
}

fn mixed_stop(root: &Path, branch: &str) -> Result<(), Stop> {
    let counts = staged_state(root);
    if counts.mixed > 0 {
        return Err(refusal("mixed", MIXED_TITLE, MIXED_HINT).with("branch", json!(branch)).with("mixed", json!(counts.mixed)).into());
    }
    Ok(())
}

fn list_or_throw(root: &Path, args: &[&str], what: &str) -> Result<Vec<String>, Stop> {
    let mut all = vec!["-c", "core.quotepath=off"];
    all.extend_from_slice(args);
    let out = look(root, &all);
    if !out.ok() {
        return Err(broke(&out, what).into());
    }
    Ok(ops::split_z(&out.stdout))
}

fn way_files(root: &Path, tree: &str) -> Result<Vec<String>, Stop> {
    let wanted = list_or_throw(
        root,
        &["ls-tree", "-r", "-z", "--name-only", "--full-tree", "--end-of-options", tree],
        "cannot read the files of that commit",
    )?;
    let known: HashSet<String> = list_or_throw(root, &["ls-files", "-z", "--cached"], "cannot read the tracked files")?.into_iter().collect();
    let fresh: Vec<&String> = wanted.iter().filter(|path| !known.contains(*path)).collect();
    let mut found = Vec::new();
    for chunk in fresh.chunks(PATH_CHUNK) {
        let specs: Vec<String> = chunk.iter().map(|path| format!(":(top,literal){path}")).collect();
        let mut args = vec!["ls-files", "-z", "--others", "--full-name", "--"];
        args.extend(specs.iter().map(String::as_str));
        found.extend(list_or_throw(root, &args, "cannot read the untracked files")?);
    }
    Ok(found)
}

pub fn untracked_in_the_way(root: &Path, tree: &str) -> Result<Vec<String>, String> {
    way_files(root, tree).map_err(|stop| stop.stderr)
}

fn merged_tree(root: &Path, head: &str, theirs: &str) -> Option<String> {
    let out = look(root, &["merge-tree", "--write-tree", "--no-messages", "--end-of-options", head, theirs]);
    if !matches!(out.code, Some(0) | Some(1)) {
        return None;
    }
    non_empty(out.stdout.split('\n').next().unwrap_or("").trim())
}

fn tree_after(root: &Path, mode: &str, p: &Prepared) -> Result<Option<String>, Stop> {
    if mode == "rebase" || is_ancestor(root, &p.head, &p.theirs)? {
        return Ok(Some(p.theirs.clone()));
    }
    Ok(merged_tree(root, &p.head, &p.theirs))
}

fn few_paths(paths: &[String], max: usize) -> String {
    if paths.len() <= max {
        return paths.join(", ");
    }
    format!("{} and {} more", paths[..max].join(", "), paths.len() - max)
}

fn way_refusal(name: &str, files: &[String], branch: &str) -> Outcome {
    let one = files.len() == 1;
    let title = if one { "A file here would be written over" } else { "Files here would be written over" };
    let hint = format!(
        "{name} has {} that git does not track here: {}. Move {} out of the folder first.",
        if one { "a file" } else { "files" },
        few_paths(files, 4),
        if one { "it" } else { "them" }
    );
    refusal("untracked", title, &hint).with("branch", json!(branch)).with("files", json!(files))
}

fn way_stop(root: &Path, mode: &str, name: &str, p: &Prepared) -> Result<(), Stop> {
    let Some(tree) = tree_after(root, mode, p)? else { return Ok(()) };
    let files = way_files(root, &tree)?;
    if files.is_empty() { Ok(()) } else { Err(way_refusal(name, &files, &p.current).into()) }
}

fn moved_stop(name: &str, p: &Prepared, expect_head: &Option<String>, expect_target: &Option<String>) -> Result<(), Stop> {
    if expect_head.as_deref().is_some_and(|e| !e.is_empty() && e != p.head) {
        let hint = format!("{} changed since you looked. Nothing was changed. Look again, then try once more.", p.current);
        return Err(refusal("headChanged", "The branch moved", &hint).with("branch", json!(p.current)).into());
    }
    if expect_target.as_deref().is_some_and(|e| !e.is_empty() && e != p.theirs) {
        let hint = format!("{name} changed since you looked. Nothing was changed. Look again, then try once more.");
        return Err(refusal("headChanged", &format!("{name} moved"), &hint).with("branch", json!(p.current)).into());
    }
    Ok(())
}

fn busy(op: &str) -> String {
    format!("A {} is in progress", op_word(op).to_lowercase())
}

fn prepare(root: &Path, name: &str, verb: &str) -> Result<Prepared, Stop> {
    if let Some(problem) = branch_name_problem(name) {
        return Err(refusal("invalid", "That branch name is not valid", problem).into());
    }
    let Some(current) = branch_here(root) else {
        return Err(refusal("detached", "Not on a branch", &format!("Check out a branch, then {verb}.")).into());
    };
    let state = read_state(root).map_err(|e| failure(&e))?;
    if let Some(op) = state.op.as_deref() {
        let title = busy(op);
        return Err(refusal("busy", &title, "Finish or cancel it in the Changes panel first.").with("state", state_value(&Some(state.clone()))).into());
    }
    let found = lookup(root, name)?;
    let Some(target) = found.local.clone().or(found.remote.clone()) else {
        return Err(refusal("notFound", &format!("There is no branch named {name}"), "Fetch, then try again.").into());
    };
    if found.local.as_ref().is_some_and(|row| row.current) {
        return Err(refusal("same", &format!("You are on {name} now"), &format!("Pick another branch to {verb}.")).into());
    }
    let Some(head) = commit_of(root, "HEAD") else {
        return Err(refusal("empty", &format!("{current} has no commits yet"), "Make the first commit, then try again.").into());
    };
    let Some(theirs) = commit_of(root, &target.name) else {
        return Err(refusal("notFound", &format!("{name} has no commit"), "Fetch, then try again.").into());
    };
    Ok(Prepared { current, target, head, theirs, ambiguous: found.tag.is_some() })
}

pub fn merge_plan(root: &Path, opts: &MergePlan) -> Outcome {
    settle(merge_plan_flow(root, opts))
}

fn merge_plan_flow(root: &Path, opts: &MergePlan) -> Flow {
    let name = opts.name.as_str();
    let mode = if opts.mode == "rebase" { "rebase" } else { "merge" };
    let p = prepare(root, name, mode)?;
    let (own, incoming) = count_both(root, &p.head, &p.theirs)?;
    let mine = lookup(root, &p.current)?.local;
    let upstream = mine.as_ref().and_then(|m| m.upstream.as_deref()).and_then(|u| commit_of(root, u));
    let pushed = match &upstream {
        Some(up) if own > 0 => {
            let not_theirs = format!("^{}", p.theirs);
            let not_up = format!("^{up}");
            own.saturating_sub(count_only(root, &[&p.head, &not_theirs, &not_up])?)
        }
        _ => 0,
    };
    let counts = staged_state(root);
    let tree = if incoming > 0 { tree_after(root, mode, &p)? } else { None };
    let blocked = match &tree {
        Some(t) => way_files(root, t)?,
        None => Vec::new(),
    };
    let upstream_short = if upstream.is_some() { mine.as_ref().and_then(|m| m.upstream_short.clone()) } else { None };
    Ok(success(&format!("{name} and {}", p.current))
        .with("name", json!(name))
        .with("mode", json!(mode))
        .with("current", json!(p.current))
        .with("head", json!(p.head))
        .with("theirs", json!(p.theirs))
        .with("incoming", json!(incoming))
        .with("own", json!(own))
        .with("pushed", json!(pushed))
        .with("dirty", json!(counts.dirty))
        .with("staged", json!(counts.staged))
        .with("mixed", json!(counts.mixed))
        .with("blocked", json!(blocked))
        .with("upstream", json!(upstream_short))
        .with("upToDate", json!(incoming == 0))
        .with("fastForward", json!(own == 0 && incoming > 0)))
}

pub fn compare_counts(root: &Path, opts: &Compare) -> Outcome {
    settle(compare_flow(root, opts))
}

fn ref_of(root: &Path, name: &str) -> Result<Option<String>, Stop> {
    if name == "HEAD" {
        return Ok(Some("HEAD".into()));
    }
    if branch_name_problem(name).is_some() {
        return Ok(None);
    }
    let found = lookup(root, name)?;
    Ok(found.local.or(found.remote).map(|row| row.name))
}

fn compare_flow(root: &Path, opts: &Compare) -> Flow {
    let a = if opts.a.is_empty() { "HEAD" } else { opts.a.as_str() };
    let b = if opts.b.is_empty() { "HEAD" } else { opts.b.as_str() };
    let Some(ra) = ref_of(root, a)? else {
        return Err(refusal("notFound", &format!("There is no branch named {a}"), "Fetch, then try again.").into());
    };
    let Some(rb) = ref_of(root, b)? else {
        return Err(refusal("notFound", &format!("There is no branch named {b}"), "Fetch, then try again.").into());
    };
    let (Some(x), Some(y)) = (commit_of(root, &ra), commit_of(root, &rb)) else {
        return Err(refusal("empty", "There are no commits to compare yet", "").into());
    };
    let (ahead, behind) = count_both(root, &x, &y)?;
    Ok(success(&format!("{a} compared with {b}"))
        .with("a", json!(a))
        .with("b", json!(b))
        .with("ahead", json!(ahead))
        .with("behind", json!(behind))
        .with("same", json!(x == y)))
}

fn base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn now_millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

pub fn rename_branch(root: &Path, opts: &RenameBranch) -> Outcome {
    settle(rename_flow(root, opts))
}

fn move_branch(root: &Path, a: &str, b: &str) -> Out {
    run::git(root, &["branch", "-m", "--end-of-options", a, b])
}

fn rename_flow(root: &Path, opts: &RenameBranch) -> Flow {
    let from = opts.from.as_str();
    let to = opts.to.trim();
    if let Some(bad) = branch_name_problem(from) {
        return Err(refusal("invalid", "That branch name is not valid", bad).into());
    }
    if let Some(problem) = branch_name_problem(to) {
        return Err(refusal("invalid", "That new name is not valid", problem).into());
    }
    if from == to {
        return Err(refusal("same", "That is the same name", "Type a different name.").into());
    }
    let old = lookup(root, from)?;
    let taken = lookup(root, to)?;
    if old.local.is_none() {
        return Err(refusal("notFound", &format!("There is no branch named {from}"), "Reload the list and try again.").into());
    }
    if taken.local.is_some() {
        return Err(refusal("exists", &format!("A branch named {to} already exists"), "Pick another name.").into());
    }
    let step_one = (from.to_lowercase() == to.to_lowercase()).then(|| format!("{from}-renaming-{}", base36(now_millis())));
    let mut out = move_branch(root, from, step_one.as_deref().unwrap_or(to));
    if let Some(step) = step_one.as_deref().filter(|_| out.ok()) {
        out = move_branch(root, step, to);
        if !out.ok() {
            move_branch(root, step, from);
        }
    }
    if !out.ok() {
        let text = out.said();
        let low = text.to_lowercase();
        if low.contains("is being rebased") || low.contains("is being bisected") {
            return Err(refusal("busy", &format!("{from} is in the middle of a rebase"), "Finish or cancel it first.")
                .with("branch", json!(from))
                .stderr(&text).into());
        }
        return Err(failure(&text).with("branch", json!(from)).into());
    }
    let upstream = lookup(root, to)?.local.and_then(|row| row.upstream_short);
    let hint = upstream.as_ref().map(|u| format!("It still follows {u}.")).unwrap_or_default();
    Ok(success(&format!("Renamed {from} to {to}"))
        .with("branch", json!(to))
        .with("from", json!(from))
        .with("upstream", json!(upstream))
        .hint(&hint))
}

pub fn fast_forward(root: &Path, opts: &FastForward) -> Outcome {
    settle(fast_forward_flow(root, opts))
}

fn fast_forward_flow(root: &Path, opts: &FastForward) -> Flow {
    let branch = opts.branch.as_str();
    if let Some(problem) = branch_name_problem(branch) {
        return Err(refusal("invalid", "That branch name is not valid", problem).into());
    }
    let Some(found) = lookup(root, branch)?.local else {
        return Err(refusal("notFound", &format!("There is no branch named {branch}"), "Reload the list and try again.").into());
    };
    let Some(upstream) = found.upstream.clone() else {
        return Err(refusal("noUpstream", &format!("{branch} does not follow a remote branch"), "Set its upstream first.")
            .with("branch", json!(branch)).into());
    };
    let up = found.upstream_short.clone().unwrap_or_else(|| upstream.clone());
    let Some(theirs) = commit_of(root, &upstream) else {
        return Err(refusal(
            "gone",
            &format!("{up} is gone"),
            "It was deleted on the remote. Set another upstream, or delete this branch.",
        )
        .with("branch", json!(branch)).into());
    };
    let mine = found.sha.clone().unwrap_or_default();
    let fresh = || success(&format!("{branch} is up to date")).with("branch", json!(branch)).with("upToDate", true.into()).with("count", json!(0));
    if mine == theirs {
        return Ok(fresh());
    }
    if !is_ancestor(root, &mine, &theirs)? {
        if is_ancestor(root, &theirs, &mine)? {
            return Ok(fresh().hint(&format!("It has commits that are not on {up} yet. Push them.")));
        }
        return Err(refusal(
            "diverged",
            &format!("Cannot fast-forward {branch}"),
            &format!("{branch} and {up} both have new commits. Merge or rebase instead."),
        )
        .with("branch", json!(branch)).into());
    }
    let count = count_both(root, &mine, &theirs)?.1;
    let done = success(&format!("Moved {branch} forward by {}", plural(count, "commit", "commits")))
        .with("branch", json!(branch))
        .with("count", json!(count))
        .with("from", json!(mine))
        .with("to", json!(theirs))
        .hint(&format!("It now matches {up}."));
    if found.current {
        let state = read_state(root).map_err(|e| failure(&e))?;
        if let Some(op) = state.op.as_deref() {
            let title = busy(op);
            return Err(refusal("busy", &title, "Finish or cancel it in the Changes panel first.")
                .with("branch", json!(branch))
                .with("state", state_value(&Some(state.clone()))).into());
        }
        let files = way_files(root, &theirs)?;
        if !files.is_empty() {
            return Err(way_refusal(&up, &files, branch).into());
        }
        let out = run::job(root, &["merge", "--ff-only", "--no-overwrite-ignore", "--end-of-options", "@{u}"], &opts.job);
        if let Some(stop) = job_stop(&out, "Git was stopped. Check the Changes panel.") {
            return Err(stop.with("branch", json!(branch)).into());
        }
        if !out.ok() {
            return Err(failure(&out.said()).with("branch", json!(branch)).into());
        }
        return Ok(done.stderr(&out.stderr));
    }
    if let Some(folder) = found.worktree.as_deref() {
        return Err(refusal("worktree", &format!("{branch} is open in another folder"), "Fast-forward it in that folder.")
            .with("branch", json!(branch))
            .with("folder", json!(folder))
            .detail(folder).into());
    }
    let state = after_state(root);
    if state.as_ref().is_some_and(|s| s.op.as_deref() == Some("rebase") && s.branch.as_deref() == Some(branch)) {
        return Err(refusal("busy", &format!("{branch} is in the middle of a rebase"), "Finish or cancel the rebase first.")
            .with("branch", json!(branch))
            .with("state", state_value(&state)).into());
    }
    let message = format!("fast-forward {branch} to {up}");
    let full = format!("refs/heads/{branch}");
    let out = run::git(root, &["update-ref", "-m", &message, &full, &theirs, &mine]);
    if !out.ok() {
        return Err(failure(&out.said()).with("branch", json!(branch)).into());
    }
    Ok(done)
}

fn finish(title: &str, branch: &str, fast: bool, out: &Out, text: &str, identity: Value) -> Outcome {
    let done = if remote::autostash_clash(text) {
        success(&format!("{title}, but your changes clashed"))
            .with("warning", true.into())
            .with("stashConflict", true.into())
            .hint(CLASH_HINT)
    } else {
        success(title)
    };
    let done = done.with("branch", json!(branch)).with("fastForward", json!(fast)).stderr(&out.stderr);
    if identity.is_null() { done } else { done.with("identity", identity) }
}

fn gate(fast: bool, identity: &Identity) -> Result<Value, Stop> {
    if fast { Ok(Value::Null) } else { ops::identity_gate(identity) }
}

fn conflict_stop(title: &str, branch: &str, state: &Option<State>, out: &Out, text: &str) -> Outcome {
    let conflicts = state.as_ref().map(|s| s.conflicts.clone()).unwrap_or_default();
    refusal("conflict", title, CONFLICT_HINT)
        .with("branch", json!(branch))
        .with("state", state_value(state))
        .with("conflicts", json!(conflicts))
        .stderr(&out.stderr)
        .detail(&describe(text).detail)
}

fn has_conflicts(state: &Option<State>, op: &str) -> bool {
    state.as_ref().is_some_and(|s| s.op.as_deref() == Some(op) && !s.conflicts.is_empty())
}

pub fn merge_into(root: &Path, opts: &MergeInto) -> Outcome {
    settle(merge_into_flow(root, opts))
}

fn merge_into_flow(root: &Path, opts: &MergeInto) -> Flow {
    let name = opts.name.as_str();
    let p = prepare(root, name, "merge")?;
    let current = p.current.clone();
    moved_stop(name, &p, &opts.expect_head, &opts.expect_target)?;
    if is_ancestor(root, &p.theirs, &p.head)? {
        return Ok(success(&format!("{current} already has {name}"))
            .with("branch", json!(current))
            .with("upToDate", true.into())
            .hint("There was nothing to merge."));
    }
    mixed_stop(root, &current)?;
    way_stop(root, "merge", name, &p)?;
    let fast = !opts.no_ff && is_ancestor(root, &p.head, &p.theirs)?;
    let identity = gate(fast, &opts.identity)?;
    let arg = if p.ambiguous { p.target.name.as_str() } else { name };
    let mut args = vec!["merge", "--no-edit", "--autostash", "--no-overwrite-ignore"];
    if opts.no_ff {
        args.push("--no-ff");
    }
    args.extend(["--end-of-options", arg]);
    let out = run::job(root, &args, &opts.job);
    if let Some(stop) = job_stop(&out, PART_WAY) {
        return Err(stop.with("branch", json!(current)).with("state", state_value(&after_state(root))).into());
    }
    let text = out.said();
    if !out.ok() {
        let state = after_state(root);
        if has_conflicts(&state, "merge") {
            return Err(conflict_stop(&format!("Merging {name} has conflicts"), &current, &state, &out, &text).into());
        }
        if text.to_lowercase().contains("refusing to merge unrelated histories") {
            return Err(refusal("unrelated", &format!("{name} and {current} share no history"), "Git will not merge them.")
                .with("branch", json!(current))
                .stderr(&text).into());
        }
        return Err(failure(&text).with("branch", json!(current)).with("state", state_value(&state)).into());
    }
    let forward = text.to_lowercase().contains("fast-forward");
    let title = if forward { format!("Moved {current} forward to {name}") } else { format!("Merged {name} into {current}") };
    Ok(finish(&title, &current, forward, &out, &text, identity))
}

pub fn rebase_onto(root: &Path, opts: &RebaseOnto) -> Outcome {
    settle(rebase_onto_flow(root, opts))
}

fn rebase_onto_flow(root: &Path, opts: &RebaseOnto) -> Flow {
    let name = opts.name.as_str();
    let p = prepare(root, name, "rebase")?;
    let current = p.current.clone();
    moved_stop(name, &p, &opts.expect_head, &opts.expect_target)?;
    if is_ancestor(root, &p.theirs, &p.head)? {
        return Ok(success(&format!("{current} already has every commit of {name}"))
            .with("branch", json!(current))
            .with("upToDate", true.into())
            .hint("There was nothing to rebase."));
    }
    mixed_stop(root, &current)?;
    way_stop(root, "rebase", name, &p)?;
    let fast = is_ancestor(root, &p.head, &p.theirs)?;
    let identity = gate(fast, &opts.identity)?;
    let out = run::job(root, &["rebase", "--autostash", "--end-of-options", &p.target.name], &opts.job);
    if let Some(stop) = job_stop(&out, PART_WAY) {
        return Err(stop.with("branch", json!(current)).with("state", state_value(&after_state(root))).into());
    }
    let text = out.said();
    if !out.ok() {
        let state = after_state(root);
        if has_conflicts(&state, "rebase") {
            let title = step_title(&state, &format!("Rebasing onto {name} has conflicts"));
            return Err(conflict_stop(&title, &current, &state, &out, &text).into());
        }
        if state.as_ref().is_some_and(|s| s.op.as_deref() == Some("rebase")) {
            return Err(refusal("paused", "The rebase stopped part way", "Check the Changes panel, then press Continue or Cancel.")
                .with("branch", json!(current))
                .with("state", state_value(&state))
                .stderr(&out.stderr)
                .detail(&describe(&text).detail).into());
        }
        return Err(failure(&text).with("branch", json!(current)).with("state", state_value(&state)).into());
    }
    let title = if fast { format!("Moved {current} forward to {name}") } else { format!("Rebased {current} onto {name}") };
    Ok(finish(&title, &current, fast, &out, &text, identity))
}

fn short_of(root: &Path, sha: Option<&str>) -> Option<String> {
    let sha = sha?;
    let cut: String = sha.chars().take(7).collect();
    let out = look(root, &["rev-parse", "--short", "--end-of-options", sha]);
    Some(if out.ok() { non_empty(out.stdout.trim()).unwrap_or(cut) } else { cut })
}

fn remote_names_stop(remote: &str, branch: &str) -> Result<(), Stop> {
    if let Some(bad) = remote_name_problem(remote) {
        return Err(refusal("invalid", "That remote name is not valid", bad).into());
    }
    if let Some(problem) = branch_name_problem(branch) {
        return Err(refusal("invalid", "That branch name is not valid", problem).into());
    }
    Ok(())
}

fn with_base(outcome: Outcome, remote: &str, branch: &str) -> Outcome {
    outcome.with("remote", json!(remote)).with("branch", json!(branch))
}

pub fn remote_delete_plan(root: &Path, opts: &RemoteBranch) -> Outcome {
    with_base(settle(remote_delete_plan_flow(root, opts)), &opts.remote, &opts.branch)
}

fn remote_delete_plan_flow(root: &Path, opts: &RemoteBranch) -> Flow {
    let (remote, branch) = (opts.remote.as_str(), opts.branch.as_str());
    remote_names_stop(remote, branch)?;
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let title = format!("{branch} on {remote}");
    let Some(sha) = commit_of(root, &tracking) else {
        return Ok(success(&title).with("sha", Value::Null).with("short", Value::Null).with("only", json!(0)));
    };
    let exclude = format!("--exclude={tracking}");
    let only = count_only(root, &[&tracking, "--not", &exclude, "--all"])?;
    let short = short_of(root, Some(&sha));
    Ok(success(&title).with("sha", json!(sha)).with("short", json!(short)).with("only", json!(only)))
}

pub fn delete_remote_branch(root: &Path, opts: &DeleteRemoteBranch) -> Outcome {
    with_base(settle(delete_remote_flow(root, opts)), &opts.remote, &opts.branch)
}

fn full_sha(text: &str) -> bool {
    (40..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn drop_tracking(root: &Path, tracking: &str) {
    if commit_of(root, tracking).is_some() {
        run::git(root, &["update-ref", "-d", tracking]);
    }
}

fn join_hint(parts: &[&str]) -> String {
    parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

fn delete_remote_flow(root: &Path, opts: &DeleteRemoteBranch) -> Flow {
    let (remote, branch) = (opts.remote.as_str(), opts.branch.as_str());
    remote_names_stop(remote, branch)?;
    let names = ops::lines(&run::git(root, &["remote"]).stdout);
    if !names.iter().any(|n| n == remote) {
        return Err(refusal("noRemote", &format!("There is no remote named {remote}"), "Check the remotes in the repo settings.").into());
    }
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let head_ref = format!("refs/remotes/{remote}/HEAD");
    let head = run::git(root, &["symbolic-ref", "-q", &head_ref]);
    if head.ok() && head.stdout.trim() == tracking {
        return Err(refusal("default", &format!("{branch} is the main branch of {remote}"), "It cannot be deleted from here.").into());
    }
    let expect = opts.expect_sha.clone().filter(|e| !e.is_empty());
    if expect.as_deref().is_some_and(|e| !full_sha(e)) {
        return Err(refusal("invalid", "That commit id is not valid", "Look again, then try once more.").into());
    }
    let tip = commit_of(root, &tracking);
    if let (Some(e), Some(t)) = (&expect, &tip)
        && e != t
    {
        let hint = format!("{remote}/{branch} changed since you looked. Nothing was deleted. Look again, then try once more.");
        return Err(refusal("headChanged", "The branch moved", &hint).into());
    }
    let sha = tip.or(expect.clone());
    let short = short_of(root, sha.as_deref());
    let was = short.as_ref().map(|s| format!("It was at {s}.")).unwrap_or_default();
    let gone = || {
        drop_tracking(root, &tracking);
        success(&format!("{branch} was already gone from {remote}"))
            .with("sha", json!(sha))
            .with("short", json!(short))
            .with("alreadyGone", true.into())
            .hint(&join_hint(&[&was, "Removed the old copy here too."]))
    };
    let lease = expect.as_ref().map(|e| format!("--force-with-lease=refs/heads/{branch}:{e}"));
    let target = format!("refs/heads/{branch}");
    let mut args = vec!["push", "--porcelain"];
    if let Some(l) = lease.as_deref() {
        args.push(l);
    }
    args.extend(["--delete", "--end-of-options", remote, &target]);
    let out = run::job(root, &args, &opts.job);
    if let Some(stop) = job_stop(&out, "Fetch to see if it was deleted.") {
        return Err(stop.into());
    }
    let text = out.said();
    if text.to_lowercase().contains("remote ref does not exist") {
        return Ok(gone());
    }
    let bad = remote::parse_push_porcelain(&out.stdout).into_iter().find(|r| r.flag == '!');
    if let Some(rejected) = &bad {
        let why = if rejected.reason.is_empty() { rejected.summary.clone() } else { rejected.reason.clone() };
        if lease.is_some() && why.to_lowercase().contains("stale info") {
            let check = run::job(root, &["ls-remote", "--heads", "--end-of-options", remote, &target], &opts.job);
            if let Some(stop) = job_stop(&check, "Fetch to see if it is still there.") {
                return Err(stop.into());
            }
            if check.ok() && check.stdout.trim().is_empty() {
                return Ok(gone());
            }
            return Err(refusal(
                "headChanged",
                &format!("{branch} on {remote} has new commits"),
                "Nothing was deleted. Fetch to see them, then try again.",
            )
            .stderr(&out.stderr).into());
        }
        return Err(refusal(
            "remoteRejected",
            &format!("{remote} did not delete {branch}"),
            "The branch may be protected, or this account may not be allowed to delete it.",
        )
        .detail(&why)
        .stderr(&out.stderr).into());
    }
    if !out.ok() {
        return Err(failure(&text).into());
    }
    drop_tracking(root, &tracking);
    Ok(success(&format!("Deleted {branch} on {remote}"))
        .with("sha", json!(sha))
        .with("short", json!(short))
        .hint(&join_hint(&[&was, "Your local branches were not changed."]))
        .stderr(&out.stderr))
}

pub fn reset_plan(root: &Path) -> Outcome {
    settle(reset_plan_flow(root))
}

fn reset_plan_flow(root: &Path) -> Flow {
    let Some(branch) = branch_here(root) else {
        return Err(refusal("detached", "Not on a branch", "Check out a branch first.").into());
    };
    let mine = lookup(root, &branch)?.local;
    let Some(full) = mine.as_ref().and_then(|m| m.upstream.clone()) else {
        return Err(refusal("noUpstream", &format!("{branch} does not follow a remote branch"), "Set its upstream first.")
            .with("branch", json!(branch)).into());
    };
    let upstream = mine.as_ref().and_then(|m| m.upstream_short.clone()).unwrap_or_else(|| full.clone());
    let Some(target) = commit_of(root, &full) else {
        return Err(refusal("gone", &format!("{upstream} is gone"), "It was deleted on the remote.")
            .with("branch", json!(branch))
            .with("upstream", json!(upstream)).into());
    };
    let head = commit_of(root, "HEAD");
    let ahead = match &head {
        Some(h) => count_only(root, &[h, &format!("^{target}")])?,
        None => 0,
    };
    let dirty = local_changes(root);
    let untracked = history::overwritten_untracked(root, &target).map_err(|e| failure(&e))?;
    Ok(success(&format!("Reset {branch} to {upstream}"))
        .with("branch", json!(branch))
        .with("upstream", json!(upstream))
        .with("head", json!(head))
        .with("target", json!(target))
        .with("ahead", json!(ahead))
        .with("dirty", json!(dirty))
        .with("untracked", json!(untracked)))
}

pub fn switch_stash_message(from: &str, to: &str) -> String {
    format!("{SWITCH_TAG} left {from} for {to}")
}

pub fn switch_changes(root: &Path) -> Result<usize, String> {
    let out = look(root, &["-c", "core.quotepath=off", "status", "--porcelain=v1", "-z", "--untracked-files=normal"]);
    if !out.ok() {
        let text = out.said();
        return Err(if text.is_empty() { "cannot read the changes".into() } else { text });
    }
    Ok(status_rows(&out.stdout).len())
}

pub fn left_stash_for(text: &str, branch: &str) -> Option<LeftStash> {
    if branch.is_empty() {
        return None;
    }
    text.split('\n').find_map(|line| {
        let parts: Vec<&str> = line.split('\0').collect();
        let reference = parts.first().copied().unwrap_or("");
        let sha = parts.get(1).copied().unwrap_or("");
        let subject = parts.get(2).copied().unwrap_or("");
        let caps = LEFT_STASH.captures(subject.trim())?;
        (&caps[1] == branch && !reference.is_empty()).then(|| LeftStash { reference: reference.into(), sha: sha.into(), to: caps[2].into() })
    })
}

pub fn read_left_stash(root: &Path, branch: &str) -> Option<LeftStash> {
    if branch.is_empty() {
        return None;
    }
    let out = look(root, &["stash", "list", "--format=%gd%x00%H%x00%gs"]);
    if out.ok() { left_stash_for(&out.stdout, branch) } else { None }
}

fn pop_back(root: &Path) -> Outcome {
    let with_index = stash::stash_pop(root, "stash@{0}", &StashUse { index: true, sha: None });
    if with_index.ok || with_index.kind == "conflict" {
        return with_index;
    }
    stash::stash_pop(root, "stash@{0}", &StashUse { index: false, sha: None })
}

pub fn stash_then_switch(root: &Path, opts: &StashThenSwitch) -> Outcome {
    let to = opts.to.as_str();
    let from = if opts.from.trim().is_empty() { "HEAD" } else { opts.from.trim() };
    if let Some(problem) = branch_name_problem(to) {
        return refusal("invalid", "That branch name is not valid", problem);
    }
    if !opts.create && to == from {
        return success(&format!("Already on {to}")).with("branch", json!(to));
    }
    let push = StashPush { message: switch_stash_message(from, to), include_untracked: true, keep_index: false, paths: Vec::new() };
    let saved = stash::stash_push(root, &push);
    if !saved.ok {
        return saved;
    }
    let moved = if opts.create {
        branches::create_branch(root, &CreateBranch { name: to.into(), from: None, checkout: true })
    } else {
        branches::checkout_branch(root, to)
    };
    if moved.ok {
        let hint = format!("Your changes are stashed on {from}. Go back to {from} to bring them back.");
        return moved.with("stashed", true.into()).hint(&hint);
    }
    let back = pop_back(root);
    let after = if back.ok {
        "Your changes are back in the files.".to_string()
    } else {
        format!("Your changes are still in the stash list. {}", back.title).trim().to_string()
    };
    let hint = join_hint(&[&moved.hint, &after]);
    moved.hint(&hint)
}

pub fn bring_back_stash(root: &Path, stash: Option<&LeftStash>) -> Outcome {
    let Some(left) = stash.filter(|s| !s.reference.is_empty()) else {
        return refusal("nothing", "Nothing to bring back", "");
    };
    let sha = non_empty(&left.sha);
    let with_index = stash::stash_pop(root, &left.reference, &StashUse { index: true, sha: sha.clone() });
    if with_index.ok || with_index.kind == "conflict" || with_index.kind == "stashMoved" {
        return with_index;
    }
    stash::stash_pop(root, &left.reference, &StashUse { index: false, sha })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrepo::{self, Repo};

    fn branch_with(repo: &Repo, name: &str, rel: &str, text: &str) -> String {
        repo.git(&["checkout", "-q", "-b", name]);
        let sha = repo.commit_file(rel, text, &format!("{name} work"));
        repo.git(&["checkout", "-q", "main"]);
        sha
    }

    fn merge(repo: &Repo, name: &str) -> Outcome {
        merge_into(repo.root(), &MergeInto { name: name.into(), ..MergeInto::default() })
    }

    #[test]
    fn merge_plan_and_merge_clean() {
        let repo = testrepo::new();
        branch_with(&repo, "feat", "b.txt", "bee\n");
        let plan = merge_plan(repo.root(), &MergePlan { name: "feat".into(), mode: "merge".into() });
        assert!(plan.ok, "{plan:?}");
        assert_eq!(plan.get("incoming"), Some(&json!(1)));
        assert_eq!(plan.get("own"), Some(&json!(0)));
        assert!(plan.flag("fastForward"));
        assert_eq!(plan.get("blocked"), Some(&json!([])));
        repo.commit_file("c.txt", "see\n", "main work");
        let plan = merge_plan(repo.root(), &MergePlan { name: "feat".into(), mode: "merge".into() });
        assert!(!plan.flag("fastForward"));
        assert_eq!(plan.get("own"), Some(&json!(1)));
        let done = merge(&repo, "feat");
        assert!(done.ok, "{done:?}");
        assert_eq!(done.title, "Merged feat into main");
        assert_eq!(repo.read("b.txt"), "bee\n");
        let again = merge(&repo, "feat");
        assert!(again.ok && again.flag("upToDate"));
        assert_eq!(again.title, "main already has feat");
    }

    #[test]
    fn merge_fast_forward_and_refusals() {
        let repo = testrepo::new();
        branch_with(&repo, "feat", "b.txt", "bee\n");
        let same = merge(&repo, "main");
        assert_eq!(same.kind, "same");
        let missing = merge(&repo, "nope");
        assert_eq!(missing.kind, "notFound");
        let moved = merge_into(repo.root(), &MergeInto { name: "feat".into(), expect_target: Some("0".repeat(40)), ..MergeInto::default() });
        assert_eq!(moved.kind, "headChanged");
        let done = merge(&repo, "feat");
        assert!(done.ok, "{done:?}");
        assert_eq!(done.title, "Moved main forward to feat");
        assert!(done.flag("fastForward"));
    }

    #[test]
    fn merge_stops_for_untracked_and_identity() {
        let repo = testrepo::new();
        branch_with(&repo, "feat", "b.txt", "bee\n");
        repo.write("b.txt", "mine\n");
        let plan = merge_plan(repo.root(), &MergePlan { name: "feat".into(), mode: String::new() });
        assert_eq!(plan.get("blocked"), Some(&json!(["b.txt"])));
        let stop = merge(&repo, "feat");
        assert_eq!(stop.kind, "untracked");
        assert_eq!(stop.title, "A file here would be written over");
        std::fs::remove_file(repo.path("b.txt")).unwrap();
        repo.commit_file("c.txt", "see\n", "main work");
        let ask = merge_into(repo.root(), &MergeInto { name: "feat".into(), identity: Identity::Ask { warnings: vec![] }, ..MergeInto::default() });
        assert_eq!(ask.kind, "identity");
    }

    #[test]
    fn merge_into_conflict() {
        let repo = testrepo::new();
        branch_with(&repo, "feat", "a.txt", "one\nTWO\nthree\n");
        repo.commit_file("a.txt", "one\n2\nthree\n", "main edit");
        let out = merge(&repo, "feat");
        assert!(!out.ok);
        assert_eq!(out.kind, "conflict", "{out:?}");
        assert_eq!(out.title, "Merging feat has conflicts");
        assert_eq!(out.get("conflicts"), Some(&json!(["a.txt"])));
        let busy = merge(&repo, "feat");
        assert_eq!(busy.kind, "busy");
        assert_eq!(busy.title, "A merge is in progress");
    }

    #[test]
    fn mixed_files_stop_a_merge() {
        let repo = testrepo::new();
        branch_with(&repo, "feat", "b.txt", "bee\n");
        repo.commit_file("c.txt", "see\n", "main work");
        repo.write("a.txt", "staged\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "staged\nand not\n");
        assert_eq!(staged_state(repo.root()), StageCounts { dirty: 1, staged: 1, mixed: 1 });
        assert_eq!(local_changes(repo.root()), 1);
        let out = merge(&repo, "feat");
        assert_eq!(out.kind, "mixed");
        assert_eq!(out.title, MIXED_TITLE);
    }

    #[test]
    fn rebase_onto_moves_and_conflicts() {
        let repo = testrepo::new();
        branch_with(&repo, "feat", "b.txt", "bee\n");
        repo.commit_file("c.txt", "see\n", "main work");
        repo.git(&["checkout", "-q", "feat"]);
        let out = rebase_onto(repo.root(), &RebaseOnto { name: "main".into(), ..RebaseOnto::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Rebased feat onto main");
        assert_eq!(repo.read("c.txt"), "see\n");
        let again = rebase_onto(repo.root(), &RebaseOnto { name: "main".into(), ..RebaseOnto::default() });
        assert!(again.flag("upToDate"));
        repo.git(&["checkout", "-q", "main"]);
        repo.commit_file("a.txt", "one\n2\nthree\n", "main edit");
        repo.git(&["checkout", "-q", "feat"]);
        repo.commit_file("a.txt", "one\nTWO\nthree\n", "feat edit");
        let clash = rebase_onto(repo.root(), &RebaseOnto { name: "main".into(), ..RebaseOnto::default() });
        assert_eq!(clash.kind, "conflict", "{clash:?}");
        assert_eq!(clash.title, "Step 2 of 2 has conflicts");
        repo.git(&["rebase", "--abort"]);
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["checkout", "-q", "-b", "behind", "HEAD~1"]);
        let fast = rebase_onto(repo.root(), &RebaseOnto { name: "main".into(), ..RebaseOnto::default() });
        assert!(fast.ok, "{fast:?}");
        assert_eq!(fast.title, "Moved behind forward to main");
    }

    #[test]
    fn fast_forward_current_and_other_branch() {
        let (repo, remote) = testrepo::with_remote();
        repo.git(&["branch", "side", "origin/main"]);
        let fresh = fast_forward(repo.root(), &FastForward { branch: "main".into(), ..FastForward::default() });
        assert!(fresh.ok && fresh.flag("upToDate"));
        let other = testrepo::clone_of(&remote);
        other.commit_file("n.txt", "new\n", "from other");
        other.git(&["push", "-q", "origin", "main"]);
        repo.git(&["fetch", "-q", "origin"]);
        let out = fast_forward(repo.root(), &FastForward { branch: "main".into(), ..FastForward::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Moved main forward by 1 commit");
        assert_eq!(repo.read("n.txt"), "new\n");
        let side = fast_forward(repo.root(), &FastForward { branch: "side".into(), ..FastForward::default() });
        assert!(side.ok, "{side:?}");
        assert_eq!(side.get("count"), Some(&json!(1)));
        assert_eq!(repo.git(&["rev-parse", "side"]), repo.git(&["rev-parse", "origin/main"]));
        repo.git(&["branch", "lonely"]);
        let none = fast_forward(repo.root(), &FastForward { branch: "lonely".into(), ..FastForward::default() });
        assert_eq!(none.kind, "noUpstream");
    }

    #[test]
    fn rename_branch_plain_and_case_only() {
        let repo = testrepo::new();
        repo.git(&["branch", "feat"]);
        repo.git(&["branch", "taken"]);
        let out = rename_branch(repo.root(), &RenameBranch { from: "feat".into(), to: " feat2 ".into() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Renamed feat to feat2");
        let case = rename_branch(repo.root(), &RenameBranch { from: "feat2".into(), to: "Feat2".into() });
        assert!(case.ok, "{case:?}");
        assert!(repo.git(&["branch", "--list", "Feat2"]).contains("Feat2"));
        let same = rename_branch(repo.root(), &RenameBranch { from: "Feat2".into(), to: "Feat2".into() });
        assert_eq!(same.kind, "same");
        let exists = rename_branch(repo.root(), &RenameBranch { from: "Feat2".into(), to: "taken".into() });
        assert_eq!(exists.kind, "exists");
        let missing = rename_branch(repo.root(), &RenameBranch { from: "gone".into(), to: "other".into() });
        assert_eq!(missing.kind, "notFound");
    }

    #[test]
    fn compare_counts_both_ways() {
        let repo = testrepo::new();
        branch_with(&repo, "feat", "b.txt", "bee\n");
        repo.commit_file("c.txt", "see\n", "one");
        repo.commit_file("d.txt", "dee\n", "two");
        let out = compare_counts(repo.root(), &Compare { a: "main".into(), b: "feat".into() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.get("ahead"), Some(&json!(2)));
        assert_eq!(out.get("behind"), Some(&json!(1)));
        assert!(!out.flag("same"));
        let same = compare_counts(repo.root(), &Compare { a: String::new(), b: "main".into() });
        assert!(same.flag("same"));
        let missing = compare_counts(repo.root(), &Compare { a: "nope".into(), b: "main".into() });
        assert_eq!(missing.kind, "notFound");
    }

    #[test]
    fn delete_remote_branch_flow() {
        let (repo, _remote) = testrepo::with_remote();
        repo.git(&["checkout", "-q", "-b", "feature"]);
        repo.commit_file("f.txt", "eff\n", "feature work");
        repo.git(&["push", "-q", "origin", "feature"]);
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["branch", "-D", "feature"]);
        let ask = RemoteBranch { remote: "origin".into(), branch: "feature".into() };
        let plan = remote_delete_plan(repo.root(), &ask);
        assert!(plan.ok, "{plan:?}");
        assert_eq!(plan.get("only"), Some(&json!(1)));
        let sha = plan.text("sha").unwrap().to_string();
        let wrong = delete_remote_branch(repo.root(), &DeleteRemoteBranch { remote: "origin".into(), branch: "feature".into(), expect_sha: Some("a".repeat(40)), ..DeleteRemoteBranch::default() });
        assert_eq!(wrong.kind, "headChanged");
        let bad = delete_remote_branch(repo.root(), &DeleteRemoteBranch { remote: "origin".into(), branch: "feature".into(), expect_sha: Some("xyz".into()), ..DeleteRemoteBranch::default() });
        assert_eq!(bad.kind, "invalid");
        let out = delete_remote_branch(repo.root(), &DeleteRemoteBranch { remote: "origin".into(), branch: "feature".into(), expect_sha: Some(sha), ..DeleteRemoteBranch::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Deleted feature on origin");
        assert_eq!(out.text("remote"), Some("origin"));
        assert!(repo.git(&["ls-remote", "--heads", "origin"]).lines().all(|l| !l.ends_with("refs/heads/feature")));
        assert!(commit_of(repo.root(), "refs/remotes/origin/feature").is_none());
        let again = delete_remote_branch(repo.root(), &DeleteRemoteBranch { remote: "origin".into(), branch: "feature".into(), ..DeleteRemoteBranch::default() });
        assert!(again.ok, "{again:?}");
        let none = delete_remote_branch(repo.root(), &DeleteRemoteBranch { remote: "upstream".into(), branch: "feature".into(), ..DeleteRemoteBranch::default() });
        assert_eq!(none.kind, "noRemote");
        repo.git(&["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
        let main = delete_remote_branch(repo.root(), &DeleteRemoteBranch { remote: "origin".into(), branch: "main".into(), ..DeleteRemoteBranch::default() });
        assert_eq!(main.kind, "default");
    }

    #[test]
    fn reset_plan_counts() {
        let (repo, remote) = testrepo::with_remote();
        let other = testrepo::clone_of(&remote);
        other.commit_file("n.txt", "new\n", "from other");
        other.git(&["push", "-q", "origin", "main"]);
        repo.git(&["fetch", "-q", "origin"]);
        repo.commit_file("mine.txt", "mine\n", "local only");
        repo.write("a.txt", "changed\n");
        repo.write("n.txt", "untracked here\n");
        let out = reset_plan(repo.root());
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Reset main to origin/main");
        assert_eq!(out.get("ahead"), Some(&json!(1)));
        assert_eq!(out.get("dirty"), Some(&json!(1)));
        assert_eq!(out.get("untracked"), Some(&json!(["n.txt"])));
        repo.git(&["branch", "--unset-upstream"]);
        assert_eq!(reset_plan(repo.root()).kind, "noUpstream");
    }

    #[test]
    fn untracked_in_the_way_lists_files() {
        let repo = testrepo::new();
        let sha = branch_with(&repo, "feat", "dir/b.txt", "bee\n");
        repo.write("dir/b.txt", "mine\n");
        repo.write("other.txt", "x\n");
        assert_eq!(untracked_in_the_way(repo.root(), &sha).unwrap(), vec!["dir/b.txt".to_string()]);
        assert_eq!(switch_changes(repo.root()).unwrap(), 2);
    }

    #[test]
    fn left_stash_parses_switch_stashes() {
        let msg = switch_stash_message("main", "feat");
        assert_eq!(msg, "nixa-switch: left main for feat");
        let text = format!("stash@{{0}}\0abc\0On feat: other\nstash@{{1}}\0def\0On main: {msg}\n");
        let found = left_stash_for(&text, "main").unwrap();
        assert_eq!(found, LeftStash { reference: "stash@{1}".into(), sha: "def".into(), to: "feat".into() });
        assert!(left_stash_for(&text, "feat").is_none());
        assert!(left_stash_for(&text, "").is_none());
    }

    #[test]
    fn stash_then_switch_and_bring_back() {
        let repo = testrepo::new();
        repo.git(&["branch", "other"]);
        repo.write("a.txt", "changed\n");
        repo.write("u.txt", "new\n");
        let out = stash_then_switch(repo.root(), &StashThenSwitch { to: "other".into(), from: "main".into(), create: false });
        assert!(out.ok, "{out:?}");
        assert!(out.flag("stashed"));
        assert_eq!(repo.read("a.txt"), "one\ntwo\nthree\n");
        assert!(!repo.path("u.txt").exists());
        assert_eq!(ops::current_branch(repo.root()).as_deref(), Some("other"));
        let left = read_left_stash(repo.root(), "main").unwrap();
        assert_eq!(left.to, "other");
        repo.git(&["checkout", "-q", "main"]);
        let back = bring_back_stash(repo.root(), Some(&left));
        assert!(back.ok, "{back:?}");
        assert_eq!(repo.read("a.txt"), "changed\n");
        assert_eq!(repo.read("u.txt"), "new\n");
        assert!(read_left_stash(repo.root(), "main").is_none());
        assert_eq!(bring_back_stash(repo.root(), None).kind, "nothing");
        let made = stash_then_switch(repo.root(), &StashThenSwitch { to: "fresh".into(), from: "main".into(), create: true });
        assert!(made.ok, "{made:?}");
        assert_eq!(ops::current_branch(repo.root()).as_deref(), Some("fresh"));
        let already = stash_then_switch(repo.root(), &StashThenSwitch { to: "fresh".into(), from: "fresh".into(), create: false });
        assert_eq!(already.title, "Already on fresh");
    }
}
