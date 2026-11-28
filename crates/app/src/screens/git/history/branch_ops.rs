use super::commit_ops::overwritten_untracked;
use super::repo::{self, commit_of, count_both, count_strict, is_ancestor, tracked_rows};
use super::run::{self, Cancel, JOB_LIMIT, Out, lines, read, read_ok, split_z};
use super::stash_ops;
use super::words::{Outcome, branch_name_problem, describe, failure, few_paths, plural, refusal, success};
use std::collections::HashSet;
use std::path::Path;

pub const SWITCH_TAG: &str = "nixa-switch:";
pub const KEEP_IGNORED: &str = "--no-overwrite-ignore";
const CONFLICT_HINT: &str = "Fix the files in the Changes panel, then press Continue.";
const CLASH_HINT: &str = "Your own changes did not fit with the new commits. They are kept in the stash, and the clashing files are marked. Fix those files, or open the stash window.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub gone: bool,
    pub remote: bool,
    pub remote_name: Option<String>,
    pub short_name: String,
    pub worktree: Option<String>,
    pub subject: String,
    pub at: i64,
    pub sha: Option<String>,
}

fn track_num(track: &str, word: &str) -> usize {
    track
        .split(',')
        .map(str::trim)
        .find_map(|p| p.strip_prefix(word))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
}

pub fn parse_branches(text: &str) -> Vec<Branch> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\0').collect();
        let get = |i: usize| f.get(i).copied().unwrap_or_default();
        let r = get(1);
        if r.is_empty() {
            continue;
        }
        let remote = r.starts_with("refs/remotes/");
        if remote && r.ends_with("/HEAD") {
            continue;
        }
        let current = get(0) == "*";
        let name = if remote {
            r["refs/remotes/".len()..].to_string()
        } else if let Some(n) = r.strip_prefix("refs/heads/") {
            n.to_string()
        } else {
            get(2).to_string()
        };
        let track = get(4);
        let (remote_name, short_name) = if remote {
            let (a, b) = name.split_once('/').unwrap_or((&name, ""));
            (Some(a.to_string()), b.to_string())
        } else {
            (None, name.clone())
        };
        out.push(Branch {
            current,
            upstream: Some(get(3)).filter(|u| !u.is_empty()).map(str::to_string),
            ahead: track_num(track, "ahead "),
            behind: track_num(track, "behind "),
            gone: track.trim() == "gone",
            remote,
            remote_name,
            short_name,
            worktree: Some(get(5)).filter(|w| !remote && !current && !w.is_empty()).map(str::to_string),
            subject: f.get(8..).map(|s| s.join("\0")).unwrap_or_default(),
            at: get(6).parse().unwrap_or(0),
            sha: Some(get(7)).filter(|s| !s.is_empty()).map(str::to_string),
            name,
        });
    }
    out
}

pub fn list_branches(root: &Path) -> Result<Vec<Branch>, String> {
    let fmt = "--format=%(HEAD)%00%(refname)%00%(refname:short)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(worktreepath)%00%(committerdate:unix)%00%(objectname:short)%00%(subject)";
    let text = read_ok(root, &["for-each-ref", "--sort=-committerdate", fmt, "refs/heads", "refs/remotes"])?;
    let all = parse_branches(&text);
    let (local, remote): (Vec<Branch>, Vec<Branch>) = all.into_iter().partition(|b| !b.remote);
    Ok(local.into_iter().chain(remote).collect())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub local: Vec<Branch>,
    pub remote_only: Vec<Branch>,
}

pub fn visible(all: &[Branch], filter: &str) -> Shown {
    let q = filter.trim().to_lowercase();
    let names: HashSet<&str> = all.iter().filter(|b| !b.remote).map(|b| b.name.as_str()).collect();
    let tracked: HashSet<&str> = all.iter().filter(|b| !b.remote).filter_map(|b| b.upstream.as_deref()).collect();
    let hit = |b: &Branch| q.is_empty() || b.name.to_lowercase().contains(&q);
    let mut local: Vec<Branch> = all.iter().filter(|b| !b.remote && hit(b)).cloned().collect();
    local.sort_by_key(|b| !b.current);
    let remote_only = all
        .iter()
        .filter(|b| b.remote && !names.contains(b.short_name.as_str()) && !tracked.contains(b.name.as_str()) && hit(b))
        .cloned()
        .collect();
    Shown { local, remote_only }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Found {
    reference: String,
    sha: Option<String>,
    current: bool,
    worktree: Option<String>,
    upstream: Option<String>,
    upstream_short: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Lookup {
    local: Option<Found>,
    remote: Option<Found>,
    tag: Option<Found>,
}

fn lookup(root: &Path, name: &str) -> Result<Lookup, String> {
    let fmt = "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(worktreepath)%00%(upstream)%00%(upstream:short)";
    let heads = format!("refs/heads/{name}");
    let remotes = format!("refs/remotes/{name}");
    let tags = format!("refs/tags/{name}");
    let text = read_ok(root, &["for-each-ref", fmt, &heads, &remotes, &tags])?;
    let rows: Vec<Found> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\0').collect();
            let get = |i: usize| Some(f.get(i).copied().unwrap_or_default()).filter(|s| !s.is_empty()).map(str::to_string);
            let current = f.get(2) == Some(&"*");
            Found {
                reference: f[0].to_string(),
                sha: get(1),
                current,
                worktree: get(3).filter(|_| !current),
                upstream: get(4),
                upstream_short: get(5),
            }
        })
        .collect();
    let pick = |r: &str| rows.iter().find(|x| x.reference == r).cloned();
    Ok(Lookup {
        local: pick(&heads),
        remote: pick(&remotes),
        tag: pick(&tags),
    })
}

fn used_by(text: &str) -> Option<String> {
    for key in ["used by worktree at '", "checked out at '"] {
        if let Some(i) = text.find(key) {
            let rest = &text[i + key.len()..];
            return rest.find('\'').map(|j| rest[..j].to_string());
        }
    }
    None
}

fn worktree_refusal(name: &str, folder: &str) -> Outcome {
    let mut o = refusal(
        "worktree",
        format!("{name} is open in another folder"),
        "Open that worktree to work on it, or pick another branch here.",
    );
    o.detail = folder.to_string();
    o.folder = Some(folder.to_string());
    o.branch = Some(name.to_string());
    o
}

fn in_the_way(text: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let low = line.to_lowercase();
        if low.contains("untracked working tree files would be") {
            inside = true;
            continue;
        }
        if inside && line.starts_with('\t') {
            files.push(line.trim().to_string());
        } else {
            inside = false;
        }
    }
    files
}

fn switch_failure(r: &Out, name: &str) -> Outcome {
    let text = r.said();
    if let Some(folder) = used_by(&text) {
        return worktree_refusal(name, &folder);
    }
    let files = in_the_way(&text);
    if !files.is_empty() {
        let mut o = refusal(
            "dirty",
            "Files that git does not track are in the way",
            format!("Nothing was changed. Move or rename these first: {}", few_paths(&files, 5)),
        );
        o.branch = Some(name.to_string());
        o.detail = describe(&text).3;
        o.files = files;
        return o;
    }
    let mut o = failure(&text);
    o.branch = Some(name.to_string());
    if o.kind == "dirty" {
        o.files = super::commit_ops::blocked_files(&text);
    }
    o
}

fn switch_to(root: &Path, name: &str) -> Option<Outcome> {
    let r = run::git(root, &["switch", "--no-guess", KEEP_IGNORED, "--end-of-options", name]);
    (!r.ok()).then(|| switch_failure(&r, name))
}

fn on_branch(mut o: Outcome, name: &str) -> Outcome {
    o.branch = Some(name.to_string());
    o
}

pub fn checkout_branch(root: &Path, name: &str) -> Outcome {
    if let Some(p) = branch_name_problem(name) {
        return refusal("invalid", "That branch name is not valid", p);
    }
    let found = match lookup(root, name) {
        Ok(f) => f,
        Err(e) => return failure(&e),
    };
    if let Some(local) = found.local {
        if local.current {
            return on_branch(success(format!("Already on {name}")), name);
        }
        if let Some(w) = local.worktree {
            return worktree_refusal(name, &w);
        }
        return switch_to(root, name).unwrap_or_else(|| on_branch(success(format!("Now on {name}")), name));
    }
    if found.remote.is_none() {
        return refusal("notFound", format!("There is no branch named {name}"), "Fetch, then try again.");
    }
    let local = name.split_once('/').map(|(_, b)| b.to_string()).unwrap_or_default();
    if local.is_empty() || branch_name_problem(&local).is_some() {
        return refusal("invalid", "That branch name is not valid", branch_name_problem(&local).unwrap_or_default());
    }
    let mine = match lookup(root, &local) {
        Ok(f) => f.local,
        Err(e) => return failure(&e),
    };
    let Some(mine) = mine else {
        let r = run::git(root, &["switch", KEEP_IGNORED, "--create", &local, "--track", "--end-of-options", name]);
        if !r.ok() {
            return switch_failure(&r, &local);
        }
        return on_branch(success(format!("Now on {local}, tracking {name}")), &local);
    };
    if let Some(w) = mine.worktree.as_ref().filter(|_| !mine.current) {
        return worktree_refusal(&local, w);
    }
    if mine.upstream.is_none() {
        let up = format!("--set-upstream-to={name}");
        run::git(root, &["branch", &up, "--end-of-options", &local]);
    }
    if !mine.current
        && let Some(failed) = switch_to(root, &local)
    {
        return failed;
    }
    if run::git(root, &["merge-base", "--is-ancestor", &local, name]).ok() {
        let both = lines(&run::git(root, &["rev-parse", &local, name]).stdout);
        if both.len() == 2 && both[0] != both[1] {
            run::git(root, &["merge", "--ff-only", KEEP_IGNORED, "--end-of-options", name]);
        }
    }
    on_branch(success(format!("Now on {local}")), &local)
}

pub fn create_branch(root: &Path, name: &str, from: Option<&str>, checkout: bool) -> Outcome {
    if let Some(p) = branch_name_problem(name) {
        return refusal("invalid", "That branch name is not valid", p);
    }
    if let Some(f) = from
        && (f.starts_with('-') || f.chars().any(|c| c.is_whitespace() || c == '\0'))
    {
        return refusal("invalid", "That start point is not valid", "Use a branch name or a commit id.");
    }
    match lookup(root, name) {
        Ok(f) if f.local.is_some() => {
            return refusal("exists", format!("A branch named {name} already exists"), "Pick another name.");
        }
        Err(e) => return failure(&e),
        _ => {}
    }
    let mut args: Vec<&str> = if checkout {
        vec!["switch", KEEP_IGNORED, "--create", name, "--end-of-options"]
    } else {
        vec!["branch", "--end-of-options", name]
    };
    if let Some(f) = from {
        args.push(f);
    }
    let r = run::git(root, &args);
    if !r.ok() {
        return if checkout { switch_failure(&r, name) } else { on_branch(failure(&r.said()), name) };
    }
    let title = if checkout { format!("Now on new branch {name}") } else { format!("Made branch {name}") };
    on_branch(success(title), name)
}

pub fn delete_branch(root: &Path, name: &str, force: bool) -> Outcome {
    if let Some(p) = branch_name_problem(name) {
        return refusal("invalid", "That branch name is not valid", p);
    }
    let r = run::git(root, &["branch", if force { "-D" } else { "-d" }, "--end-of-options", name]);
    if r.ok() {
        return on_branch(success(format!("Deleted {name}")), name);
    }
    let text = r.said();
    if text.contains("not fully merged") {
        return on_branch(
            refusal("unmerged", format!("{name} has commits that are not merged"), "Delete anyway only if you do not need them."),
            name,
        );
    }
    let folder = used_by(&text);
    if folder.is_some() || text.contains("Cannot delete branch") {
        let mut o = refusal("inUse", format!("{name} is checked out"), "Switch to another branch first.");
        o.detail = folder.clone().unwrap_or_default();
        o.folder = folder;
        return on_branch(o, name);
    }
    on_branch(failure(&text), name)
}

pub fn set_upstream(root: &Path, branch: &str, upstream: Option<&str>) -> Outcome {
    if let Some(p) = branch_name_problem(branch) {
        return refusal("invalid", "That branch name is not valid", p);
    }
    let Some(up) = upstream else {
        let r = run::git(root, &["branch", "--unset-upstream", "--end-of-options", branch]);
        if !r.ok() {
            return failure(&r.said());
        }
        return success(format!("{branch} has no upstream now"));
    };
    if let Some(p) = branch_name_problem(up) {
        return refusal("invalid", "That upstream name is not valid", p);
    }
    let flag = format!("--set-upstream-to={up}");
    let r = run::git(root, &["branch", &flag, "--end-of-options", branch]);
    if !r.ok() {
        return failure(&r.said());
    }
    success(format!("{branch} now tracks {up}"))
}

pub fn rename_branch(root: &Path, from: &str, to: &str) -> Outcome {
    let to = to.trim();
    if let Some(p) = branch_name_problem(from) {
        return refusal("invalid", "That branch name is not valid", p);
    }
    if let Some(p) = branch_name_problem(to) {
        return refusal("invalid", "That new name is not valid", p);
    }
    if from == to {
        return refusal("same", "That is the same name", "Type a different name.");
    }
    let (old, taken) = match (lookup(root, from), lookup(root, to)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return failure(&e),
    };
    if old.local.is_none() {
        return refusal("notFound", format!("There is no branch named {from}"), "Reload the list and try again.");
    }
    let case_only = from.to_lowercase() == to.to_lowercase();
    if taken.local.is_some() {
        return refusal("exists", format!("A branch named {to} already exists"), "Pick another name.");
    }
    let mv = |a: &str, b: &str| run::git(root, &["branch", "-m", "--end-of-options", a, b]);
    let step = case_only.then(|| format!("{from}-renaming-{}", std::process::id()));
    let mut r = mv(from, step.as_deref().unwrap_or(to));
    if r.ok()
        && let Some(s) = &step
    {
        r = mv(s, to);
        if !r.ok() {
            mv(s, from);
        }
    }
    if !r.ok() {
        let text = r.said();
        if text.contains("is being rebased") || text.contains("is being bisected") {
            return refusal("busy", format!("{from} is in the middle of a rebase"), "Finish or cancel it first.");
        }
        return failure(&text);
    }
    let up = lookup(root, to).ok().and_then(|l| l.local).and_then(|l| l.upstream_short);
    let mut o = on_branch(success(format!("Renamed {from} to {to}")), to);
    if let Some(u) = up {
        o.hint = format!("It still follows {u}.");
    }
    o
}

pub fn untracked_in_the_way(root: &Path, tree: &str) -> Result<Vec<String>, String> {
    let wanted = split_z(&read_ok(root, &["ls-tree", "-r", "-z", "--name-only", "--full-tree", "--end-of-options", tree])?);
    let known: HashSet<String> = split_z(&read_ok(root, &["ls-files", "-z", "--cached"])?).into_iter().collect();
    let fresh: Vec<String> = wanted.into_iter().filter(|p| !known.contains(p)).collect();
    let mut found = Vec::new();
    for part in fresh.chunks(200) {
        let specs: Vec<String> = part.iter().map(|p| format!(":(top,literal){p}")).collect();
        let mut args = vec!["ls-files", "-z", "--others", "--full-name", "--"];
        args.extend(specs.iter().map(String::as_str));
        found.extend(split_z(&read_ok(root, &args)?));
    }
    Ok(found)
}

fn way_refusal(name: &str, files: Vec<String>) -> Outcome {
    let them = if files.len() == 1 { "it" } else { "them" };
    let title = if files.len() == 1 { "A file here would be written over" } else { "Files here would be written over" };
    let mut o = refusal(
        "untracked",
        title,
        format!(
            "{name} has {} that git does not track here: {}. Move {them} out of the folder first.",
            if files.len() == 1 { "a file" } else { "files" },
            few_paths(&files, 4)
        ),
    );
    o.files = files;
    o
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Prepared {
    current: String,
    target_ref: String,
    head: String,
    theirs: String,
    ambiguous: bool,
}

fn prepare(root: &Path, name: &str, verb: &str) -> Result<Prepared, Box<Outcome>> {
    if let Some(p) = branch_name_problem(name) {
        return Err(Box::new(refusal("invalid", "That branch name is not valid", p)));
    }
    let current = repo::current_branch(root).ok_or_else(|| refusal("detached", "Not on a branch", format!("Check out a branch, then {verb}.")))?;
    let state = repo::read_state(root).map_err(|e| failure(&e))?;
    if let Some(op) = state.op {
        return Err(Box::new(refusal("busy", format!("A {} is in progress", op.lower()), "Finish or cancel it in the Changes panel first.")));
    }
    let found = lookup(root, name).map_err(|e| failure(&e))?;
    if found.local.as_ref().is_some_and(|l| l.current) {
        return Err(Box::new(refusal("same", format!("You are on {name} now"), format!("Pick another branch to {verb}."))));
    }
    let target = found
        .local
        .clone()
        .or(found.remote.clone())
        .ok_or_else(|| refusal("notFound", format!("There is no branch named {name}"), "Fetch, then try again."))?;
    let head = commit_of(root, "HEAD").ok_or_else(|| refusal("empty", format!("{current} has no commits yet"), "Make the first commit, then try again."))?;
    let theirs = commit_of(root, &target.reference).ok_or_else(|| refusal("notFound", format!("{name} has no commit"), "Fetch, then try again."))?;
    Ok(Prepared {
        current,
        target_ref: target.reference,
        head,
        theirs,
        ambiguous: found.tag.is_some(),
    })
}

fn merged_tree(root: &Path, head: &str, theirs: &str) -> Option<String> {
    let r = read(root, &["merge-tree", "--write-tree", "--no-messages", "--end-of-options", head, theirs]);
    if !matches!(r.code, Some(0) | Some(1)) {
        return None;
    }
    r.stdout.lines().next().map(|l| l.trim().to_string()).filter(|s| !s.is_empty())
}

fn tree_after(root: &Path, rebase: bool, p: &Prepared) -> Option<String> {
    if rebase || is_ancestor(root, &p.head, &p.theirs).unwrap_or(false) {
        return Some(p.theirs.clone());
    }
    merged_tree(root, &p.head, &p.theirs)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Staged {
    pub dirty: usize,
    pub staged: usize,
    pub mixed: usize,
}

pub fn staged_state(root: &Path) -> Staged {
    let rows = tracked_rows(root);
    let unmerged = |x: char, y: char| x == 'U' || y == 'U' || (x == y && (x == 'A' || x == 'D'));
    Staged {
        dirty: rows.len(),
        staged: rows.iter().filter(|r| r.x != ' ' && r.x != 'A').count(),
        mixed: rows.iter().filter(|r| r.x != ' ' && r.y != ' ' && !unmerged(r.x, r.y)).count(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergePlan {
    pub head: String,
    pub theirs: String,
    pub incoming: usize,
    pub own: usize,
    pub pushed: usize,
    pub staged: Staged,
    pub blocked: Vec<String>,
    pub upstream: Option<String>,
    pub up_to_date: bool,
    pub fast_forward: bool,
}

pub fn merge_plan(root: &Path, name: &str, rebase: bool) -> Result<MergePlan, Box<Outcome>> {
    let p = prepare(root, name, if rebase { "rebase" } else { "merge" })?;
    let (own, incoming) = count_both(root, &p.head, &p.theirs).map_err(|e| failure(&e))?;
    let mine = lookup(root, &p.current).ok().and_then(|l| l.local);
    let up_sha = mine.as_ref().and_then(|m| m.upstream.as_deref()).and_then(|u| commit_of(root, u));
    let pushed = match &up_sha {
        Some(u) if own > 0 => {
            let not_theirs = format!("^{}", p.theirs);
            let not_up = format!("^{u}");
            own.saturating_sub(count_strict(root, &[&p.head, &not_theirs, &not_up]).map_err(|e| failure(&e))?)
        }
        _ => 0,
    };
    let tree = if incoming > 0 { tree_after(root, rebase, &p) } else { None };
    let blocked = match tree {
        Some(t) => untracked_in_the_way(root, &t).map_err(|e| failure(&e))?,
        None => Vec::new(),
    };
    Ok(MergePlan {
        incoming,
        own,
        pushed,
        staged: staged_state(root),
        blocked,
        upstream: up_sha.and(mine.and_then(|m| m.upstream_short)),
        up_to_date: incoming == 0,
        fast_forward: own == 0 && incoming > 0,
        head: p.head,
        theirs: p.theirs,
    })
}

fn moved_stop(name: &str, p: &Prepared, head: Option<&str>, target: Option<&str>) -> Option<Outcome> {
    if head.is_some_and(|h| h != p.head) {
        return Some(refusal(
            "headChanged",
            "The branch moved",
            format!("{} changed since you looked. Nothing was changed. Look again, then try once more.", p.current),
        ));
    }
    if target.is_some_and(|t| t != p.theirs) {
        return Some(refusal(
            "headChanged",
            format!("{name} moved"),
            format!("{name} changed since you looked. Nothing was changed. Look again, then try once more."),
        ));
    }
    None
}

fn job_stop(r: &Out, hint: &str) -> Option<Outcome> {
    if r.cancelled {
        return Some(refusal("cancelled", "Stopped", hint));
    }
    if r.timed_out {
        let mut o = failure("timed out waiting");
        o.detail.clear();
        return Some(o);
    }
    None
}

fn checks_before(root: &Path, name: &str, p: &Prepared, rebase: bool) -> Option<Outcome> {
    if staged_state(root).mixed > 0 {
        return Some(on_branch(
            refusal("mixed", "Some files have staged and unstaged parts", "Commit or stash them first."),
            &p.current,
        ));
    }
    let tree = tree_after(root, rebase, p)?;
    match untracked_in_the_way(root, &tree) {
        Ok(files) if !files.is_empty() => Some(way_refusal(name, files)),
        Ok(_) => None,
        Err(e) => Some(failure(&e)),
    }
}

fn clash(text: &str) -> bool {
    text.contains("Applying autostash resulted in conflicts")
}

fn joined(title: String, fast: bool, current: &str, text: &str) -> Outcome {
    let mut o = on_branch(success(title.clone()), current);
    if clash(text) {
        o.title = format!("{title}, but your changes clashed");
        o.warning = true;
        o.hint = CLASH_HINT.into();
    }
    o.up_to_date = false;
    o.count = usize::from(fast);
    o
}

pub fn merge_into(root: &Path, name: &str, no_ff: bool, expect_head: Option<&str>, expect_target: Option<&str>) -> Outcome {
    let p = match prepare(root, name, "merge") {
        Ok(p) => p,
        Err(o) => return *o,
    };
    if let Some(o) = moved_stop(name, &p, expect_head, expect_target) {
        return o;
    }
    if is_ancestor(root, &p.theirs, &p.head).unwrap_or(false) {
        let mut o = on_branch(success(format!("{} already has {name}", p.current)), &p.current);
        o.up_to_date = true;
        o.hint = "There was nothing to merge.".into();
        return o;
    }
    if let Some(o) = checks_before(root, name, &p, false) {
        return o;
    }
    let arg = if p.ambiguous { p.target_ref.clone() } else { name.to_string() };
    let mut args = vec!["merge", "--no-edit", "--autostash", KEEP_IGNORED];
    if no_ff {
        args.push("--no-ff");
    }
    args.extend(["--end-of-options", &arg]);
    let r = run::job(root, &args, None, JOB_LIMIT);
    if let Some(o) = job_stop(&r, "Git was stopped part way. Check the Changes panel.") {
        return o;
    }
    let text = r.said();
    if !r.ok() {
        let state = repo::read_state(root).ok();
        if let Some(s) = state.filter(|s| s.op == Some(repo::Op::Merge) && !s.conflicts.is_empty()) {
            let mut o = refusal("conflict", format!("Merging {name} has conflicts"), CONFLICT_HINT);
            o.files = s.conflicts;
            o.detail = describe(&text).3;
            return on_branch(o, &p.current);
        }
        if text.contains("refusing to merge unrelated histories") {
            return refusal("unrelated", format!("{name} and {} share no history", p.current), "Git will not merge them.");
        }
        return on_branch(failure(&text), &p.current);
    }
    let fast = text.contains("Fast-forward");
    let title = if fast { format!("Moved {} forward to {name}", p.current) } else { format!("Merged {name} into {}", p.current) };
    joined(title, fast, &p.current, &text)
}

pub fn rebase_onto(root: &Path, name: &str, expect_head: Option<&str>, expect_target: Option<&str>) -> Outcome {
    let p = match prepare(root, name, "rebase") {
        Ok(p) => p,
        Err(o) => return *o,
    };
    if let Some(o) = moved_stop(name, &p, expect_head, expect_target) {
        return o;
    }
    if is_ancestor(root, &p.theirs, &p.head).unwrap_or(false) {
        let mut o = on_branch(success(format!("{} already has every commit of {name}", p.current)), &p.current);
        o.up_to_date = true;
        o.hint = "There was nothing to rebase.".into();
        return o;
    }
    if let Some(o) = checks_before(root, name, &p, true) {
        return o;
    }
    let fast = is_ancestor(root, &p.head, &p.theirs).unwrap_or(false);
    let r = run::job(root, &["rebase", "--autostash", "--end-of-options", &p.target_ref], None, JOB_LIMIT);
    if let Some(o) = job_stop(&r, "Git was stopped part way. Check the Changes panel.") {
        return o;
    }
    let text = r.said();
    if !r.ok() {
        let state = repo::read_state(root).ok().filter(|s| s.op == Some(repo::Op::Rebase));
        if let Some(s) = state {
            let detail = describe(&text).3;
            if !s.conflicts.is_empty() {
                let title = match s.step {
                    Some((n, t)) => format!("Step {n} of {t} has conflicts"),
                    None => format!("Rebasing onto {name} has conflicts"),
                };
                let mut o = refusal("conflict", title, CONFLICT_HINT);
                o.files = s.conflicts;
                o.detail = detail;
                return on_branch(o, &p.current);
            }
            let mut o = refusal("paused", "The rebase stopped part way", "Check the Changes panel, then press Continue or Cancel.");
            o.detail = detail;
            return on_branch(o, &p.current);
        }
        return on_branch(failure(&text), &p.current);
    }
    let title = if fast { format!("Moved {} forward to {name}", p.current) } else { format!("Rebased {} onto {name}", p.current) };
    joined(title, fast, &p.current, &text)
}

pub fn fast_forward(root: &Path, branch: &str) -> Outcome {
    if let Some(p) = branch_name_problem(branch) {
        return refusal("invalid", "That branch name is not valid", p);
    }
    let found = match lookup(root, branch) {
        Ok(f) => f.local,
        Err(e) => return failure(&e),
    };
    let Some(found) = found else {
        return refusal("notFound", format!("There is no branch named {branch}"), "Reload the list and try again.");
    };
    let Some(upstream) = found.upstream.clone() else {
        return refusal("noUpstream", format!("{branch} does not follow a remote branch"), "Set its upstream first.");
    };
    let up = found.upstream_short.clone().unwrap_or(upstream.clone());
    let Some(theirs) = commit_of(root, &upstream) else {
        return refusal("gone", format!("{up} is gone"), "It was deleted on the remote. Set another upstream, or delete this branch.");
    };
    let mine = found.sha.clone().unwrap_or_default();
    if mine == theirs {
        let mut o = success(format!("{branch} is up to date"));
        o.up_to_date = true;
        return o;
    }
    if !is_ancestor(root, &mine, &theirs).unwrap_or(false) {
        if is_ancestor(root, &theirs, &mine).unwrap_or(false) {
            let mut o = success(format!("{branch} is up to date"));
            o.up_to_date = true;
            o.hint = format!("It has commits that are not on {up} yet. Push them.");
            return o;
        }
        return refusal("diverged", format!("Cannot fast-forward {branch}"), format!("{branch} and {up} both have new commits. Merge or rebase instead."));
    }
    let count = count_both(root, &mine, &theirs).map(|c| c.1).unwrap_or(0);
    let mut done = on_branch(success(format!("Moved {branch} forward by {}", plural(count, "commit"))), branch);
    done.hint = format!("It now matches {up}.");
    done.count = count;
    if found.current {
        if let Ok(state) = repo::read_state(root)
            && let Some(op) = state.op
        {
            return refusal("busy", format!("A {} is in progress", op.lower()), "Finish or cancel it in the Changes panel first.");
        }
        match untracked_in_the_way(root, &theirs) {
            Ok(files) if !files.is_empty() => return way_refusal(&up, files),
            Err(e) => return failure(&e),
            _ => {}
        }
        let r = run::job(root, &["merge", "--ff-only", KEEP_IGNORED, "--end-of-options", "@{u}"], None, JOB_LIMIT);
        if let Some(o) = job_stop(&r, "Git was stopped. Check the Changes panel.") {
            return o;
        }
        if !r.ok() {
            return failure(&r.said());
        }
        return done;
    }
    if let Some(w) = found.worktree {
        let mut o = refusal("worktree", format!("{branch} is open in another folder"), "Fast-forward it in that folder.");
        o.detail = w.clone();
        o.folder = Some(w);
        return o;
    }
    if let Ok(s) = repo::read_state(root)
        && s.op == Some(repo::Op::Rebase)
        && s.branch.as_deref() == Some(branch)
    {
        return refusal("busy", format!("{branch} is in the middle of a rebase"), "Finish or cancel the rebase first.");
    }
    let why = format!("fast-forward {branch} to {up}");
    let full = format!("refs/heads/{branch}");
    let r = run::git(root, &["update-ref", "-m", &why, &full, &theirs, &mine]);
    if !r.ok() {
        return failure(&r.said());
    }
    done
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compare {
    pub ahead: usize,
    pub behind: usize,
}

pub fn compare_counts(root: &Path, a: &str, b: &str) -> Result<Compare, Box<Outcome>> {
    let resolve = |name: &str| -> Option<String> {
        if name == "HEAD" {
            return Some("HEAD".into());
        }
        if branch_name_problem(name).is_some() {
            return None;
        }
        let f = lookup(root, name).ok()?;
        f.local.or(f.remote).map(|x| x.reference)
    };
    let ra = resolve(a).ok_or_else(|| refusal("notFound", format!("There is no branch named {a}"), "Fetch, then try again."))?;
    let rb = resolve(b).ok_or_else(|| refusal("notFound", format!("There is no branch named {b}"), "Fetch, then try again."))?;
    let (Some(x), Some(y)) = (commit_of(root, &ra), commit_of(root, &rb)) else {
        return Err(Box::new(refusal("empty", "There are no commits to compare yet", "")));
    };
    let (ahead, behind) = count_both(root, &x, &y).map_err(|e| failure(&e))?;
    Ok(Compare { ahead, behind })
}

fn remote_names_problem(remote: &str, branch: &str) -> Option<Outcome> {
    if let Some(p) = branch_name_problem(remote) {
        return Some(refusal("invalid", "That remote name is not valid", p));
    }
    branch_name_problem(branch).map(|p| refusal("invalid", "That branch name is not valid", p))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteDeletePlan {
    pub sha: Option<String>,
    pub only: usize,
}

pub fn remote_delete_plan(root: &Path, remote: &str, branch: &str) -> Result<RemoteDeletePlan, Box<Outcome>> {
    if let Some(o) = remote_names_problem(remote, branch) {
        return Err(Box::new(o));
    }
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let Some(sha) = commit_of(root, &tracking) else {
        return Ok(RemoteDeletePlan::default());
    };
    let exclude = format!("--exclude={tracking}");
    let only = count_strict(root, &[&tracking, "--not", &exclude, "--all"]).map_err(|e| failure(&e))?;
    Ok(RemoteDeletePlan { sha: Some(sha), only })
}

fn short_of(root: &Path, sha: &str) -> String {
    let r = read(root, &["rev-parse", "--short", "--end-of-options", sha]);
    if r.ok() && !r.line().is_empty() { r.line() } else { super::words::short_sha(sha) }
}

pub fn delete_remote_branch(root: &Path, remote: &str, branch: &str, expect_sha: Option<&str>, cancel: Option<&Cancel>) -> Outcome {
    if let Some(o) = remote_names_problem(remote, branch) {
        return o;
    }
    if !lines(&run::git(root, &["remote"]).stdout).iter().any(|r| r == remote) {
        return refusal("noRemote", format!("There is no remote named {remote}"), "Check the remotes in the repo settings.");
    }
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let head_ref = format!("refs/remotes/{remote}/HEAD");
    let head = run::git(root, &["symbolic-ref", "-q", &head_ref]);
    if head.ok() && head.line() == tracking {
        return refusal("default", format!("{branch} is the main branch of {remote}"), "It cannot be deleted from here.");
    }
    if let Some(e) = expect_sha
        && !super::words::is_full_sha(&e.to_lowercase())
    {
        return refusal("invalid", "That commit id is not valid", "Look again, then try once more.");
    }
    let tip = commit_of(root, &tracking);
    if let (Some(e), Some(t)) = (expect_sha, &tip)
        && t != e
    {
        return refusal(
            "headChanged",
            "The branch moved",
            format!("{remote}/{branch} changed since you looked. Nothing was deleted. Look again, then try once more."),
        );
    }
    let sha = tip.or(expect_sha.map(str::to_string));
    let was = sha.as_ref().map(|s| format!("It was at {}.", short_of(root, s))).unwrap_or_default();
    let drop_tracking = || {
        if commit_of(root, &tracking).is_some() {
            run::git(root, &["update-ref", "-d", &tracking]);
        }
    };
    let gone = || {
        drop_tracking();
        let mut o = success(format!("{branch} was already gone from {remote}"));
        o.hint = [was.as_str(), "Removed the old copy here too."].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
        o
    };
    let lease = expect_sha.map(|e| format!("--force-with-lease=refs/heads/{branch}:{e}"));
    let target = format!("refs/heads/{branch}");
    let mut args = vec!["push", "--porcelain"];
    if let Some(l) = &lease {
        args.push(l);
    }
    args.extend(["--delete", "--end-of-options", remote, &target]);
    let r = run::job(root, &args, cancel, JOB_LIMIT);
    if let Some(o) = job_stop(&r, "Fetch to see if it was deleted.") {
        return o;
    }
    let text = r.said();
    if text.contains("remote ref does not exist") {
        return gone();
    }
    let bad = r.stdout.lines().find(|l| l.starts_with("!\t")).map(str::to_string);
    if let Some(b) = &bad {
        if lease.is_some() && b.contains("stale info") {
            let check = run::job(root, &["ls-remote", "--heads", "--end-of-options", remote, &target], cancel, JOB_LIMIT);
            if let Some(o) = job_stop(&check, "Fetch to see if it is still there.") {
                return o;
            }
            if check.ok() && check.stdout.trim().is_empty() {
                return gone();
            }
            return refusal("headChanged", format!("{branch} on {remote} has new commits"), "Nothing was deleted. Fetch to see them, then try again.");
        }
        let mut o = refusal(
            "remoteRejected",
            format!("{remote} did not delete {branch}"),
            "The branch may be protected, or this account may not be allowed to delete it.",
        );
        o.detail = b.rsplit('\t').next().unwrap_or_default().to_string();
        return o;
    }
    if !r.ok() {
        return failure(&text);
    }
    drop_tracking();
    let mut o = success(format!("Deleted {branch} on {remote}"));
    o.hint = [was.as_str(), "Your local branches were not changed."].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    o
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResetPlan {
    pub branch: String,
    pub upstream: String,
    pub head: Option<String>,
    pub target: String,
    pub ahead: usize,
    pub dirty: usize,
    pub untracked: Vec<String>,
}

pub fn reset_plan(root: &Path) -> Result<ResetPlan, Box<Outcome>> {
    let branch = repo::current_branch(root).ok_or_else(|| refusal("detached", "Not on a branch", "Check out a branch first."))?;
    let mine = lookup(root, &branch).map_err(|e| failure(&e))?.local;
    let Some(full_up) = mine.as_ref().and_then(|m| m.upstream.clone()) else {
        return Err(Box::new(refusal("noUpstream", format!("{branch} does not follow a remote branch"), "Set its upstream first.")));
    };
    let upstream = mine.and_then(|m| m.upstream_short).unwrap_or(full_up.clone());
    let target = commit_of(root, &full_up).ok_or_else(|| refusal("gone", format!("{upstream} is gone"), "It was deleted on the remote."))?;
    let head = commit_of(root, "HEAD");
    let ahead = match &head {
        Some(h) => count_strict(root, &[h, &format!("^{target}")]).map_err(|e| failure(&e))?,
        None => 0,
    };
    let untracked = overwritten_untracked(root, &target).map_err(|e| failure(&e))?;
    Ok(ResetPlan {
        dirty: tracked_rows(root).len(),
        branch,
        upstream,
        head,
        target,
        ahead,
        untracked,
    })
}

fn same_sha(full: Option<&str>, expect: &str) -> bool {
    full.is_some_and(|f| f.to_lowercase().starts_with(&expect.to_lowercase()))
}

pub fn reset_to_remote(root: &Path, expect_head: Option<&str>, expect_target: Option<&str>) -> Outcome {
    for e in [expect_head, expect_target].into_iter().flatten() {
        if !super::words::is_sha_like(e) {
            return refusal("invalid", "That commit id is not valid", "Nothing was reset.");
        }
    }
    let Some(upstream) = repo::upstream_of(root) else {
        let mut o = failure("has no upstream branch");
        o.detail.clear();
        return o;
    };
    let head = commit_of(root, "HEAD");
    let Some(target) = commit_of(root, "@{u}") else {
        return refusal("notFound", format!("Cannot find {upstream}"), "Fetch, then try again. Nothing was reset.");
    };
    if let Some(e) = expect_head
        && !same_sha(head.as_deref(), e)
    {
        return refusal("headChanged", "The branch moved since you looked", "Nothing was reset. Look at it again, then try again.");
    }
    if let Some(e) = expect_target
        && !same_sha(Some(&target), e)
    {
        return refusal(
            "headChanged",
            format!("{upstream} changed since you looked"),
            "Nothing was reset. A fetch brought new commits. Look at it again, then try again.",
        );
    }
    let r = run::git(root, &["reset", "--hard", &target]);
    if !r.ok() {
        return failure(&r.said());
    }
    success(format!("Branch reset to {upstream}"))
}

pub fn switch_changes(root: &Path) -> Result<usize, String> {
    let out = read(root, &["status", "--porcelain=v1", "-z", "--untracked-files=normal"]);
    if !out.ok() {
        return Err(run::non_empty(out.said(), "cannot read the changes"));
    }
    Ok(repo::status_rows(&out.stdout).len())
}

pub fn switch_stash_message(from: &str, to: &str) -> String {
    format!("{SWITCH_TAG} left {from} for {to}")
}

pub fn stash_then_switch(root: &Path, from: &str, to: &str, create: bool) -> Outcome {
    if let Some(p) = branch_name_problem(to) {
        return refusal("invalid", "That branch name is not valid", p);
    }
    let from = if from.trim().is_empty() { "HEAD" } else { from.trim() };
    if !create && to == from {
        return success(format!("Already on {to}"));
    }
    let saved = stash_ops::push(root, &switch_stash_message(from, to), true, false);
    if !saved.ok {
        return saved;
    }
    let mut moved = if create { create_branch(root, to, None, true) } else { checkout_branch(root, to) };
    if moved.ok {
        moved.stashed = true;
        moved.hint = format!("Your changes are stashed on {from}. Go back to {from} to bring them back.");
        return moved;
    }
    let mut back = stash_ops::apply(root, "stash@{0}", true, true, None);
    if !back.ok && back.kind != "conflict" {
        back = stash_ops::apply(root, "stash@{0}", true, false, None);
    }
    let after = if back.ok {
        "Your changes are back in the files.".to_string()
    } else {
        format!("Your changes are still in the stash list. {}", back.title).trim().to_string()
    };
    moved.hint = [moved.hint.as_str(), after.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    moved
}
