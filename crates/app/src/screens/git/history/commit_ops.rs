use super::branch_ops;
use super::repo::{self, Op, RefsAt, Web};
use super::run::{self, JOB_LIMIT, Out, read, read_ok, split_z};
use super::words::{
    Outcome, branch_name_problem, failure, is_full_sha, plural, refusal, short_sha, success, unchanged,
};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

impl ResetMode {
    pub fn flag(self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        }
    }

    pub fn done(self) -> &'static str {
        match self {
            ResetMode::Soft => "The changes are staged.",
            ResetMode::Mixed => "The changes are in your files, not staged.",
            ResetMode::Hard => "Your files match this commit now.",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expect {
    pub head: Option<String>,
    pub branch: Option<Option<String>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommitInfo {
    pub sha: String,
    pub short: String,
    pub subject: String,
    pub body: String,
    pub author: String,
    pub email: String,
    pub at: i64,
    pub parents: Vec<String>,
    pub is_merge: bool,
    pub is_root: bool,
    pub on_head: bool,
    pub is_head: bool,
    pub pushed: bool,
    pub refs: RefsAt,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub op: Option<Op>,
    pub after: usize,
    pub merges_after: usize,
    pub web: Option<Web>,
}

impl CommitInfo {
    pub fn expect(&self) -> Expect {
        Expect {
            head: self.head.clone(),
            branch: Some(self.branch.clone()),
        }
    }
}

pub fn compose_message(summary: &str, body: &str) -> String {
    let head = summary.split('\n').map(str::trim).collect::<Vec<_>>().join(" ").trim().to_string();
    let body = body.replace("\r\n", "\n");
    let lead = body.len() - body.trim_start().len();
    let cut = body[..lead].rfind('\n').map_or(0, |i| i + 1);
    let rest = body[cut..].trim_end().to_string();
    if head.is_empty() && rest.is_empty() {
        return String::new();
    }
    if rest.is_empty() { format!("{head}\n") } else { format!("{head}\n\n{rest}\n") }
}

pub fn split_message(text: &str) -> (String, String) {
    let clean = text.replace("\r\n", "\n");
    let mut lines: Vec<&str> = clean.trim_end().split('\n').collect();
    let summary = if lines.is_empty() { String::new() } else { lines.remove(0).trim().to_string() };
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    (summary, lines.join("\n"))
}

fn missing() -> Outcome {
    refusal("notFound", "This commit is not in the repo", "Reload the history and try again.")
}

fn head_changed() -> Outcome {
    refusal(
        "headChanged",
        "The branch moved",
        "Something changed since you opened this. Open it again. Nothing was changed.",
    )
}

fn branch_changed() -> Outcome {
    refusal(
        "headChanged",
        "You are on another branch now",
        "Something changed since you opened this. Open it again. Nothing was changed.",
    )
}

fn unborn() -> Outcome {
    refusal("unborn", "This branch has no commits yet", "Make a first commit, then try again.")
}

fn stale(expect: &Expect, head: &str, branch: &Option<String>) -> Option<Outcome> {
    if expect.head.as_deref().is_some_and(|h| h != head) {
        return Some(head_changed());
    }
    if let Some(want) = &expect.branch
        && want != branch
    {
        return Some(branch_changed());
    }
    None
}

fn place_name(branch: &Option<String>) -> String {
    branch.clone().unwrap_or_else(|| "this detached commit".into())
}

pub fn blocked_files(text: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let low = line.to_lowercase();
        if low.contains("would be overwritten by") || low.contains("would be removed by") || low.contains("would lose untracked files") {
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

pub fn is_dirty(text: &str) -> bool {
    let low = text.to_lowercase();
    ["would be overwritten by", "would be removed by", "would lose untracked files", "please commit your changes or stash them", "your local changes to the following files"]
        .iter()
        .any(|w| low.contains(w))
}

pub fn dirty_refusal(text: &str, title: &str) -> Outcome {
    let files = blocked_files(text);
    let low = text.to_lowercase();
    let untracked = low.contains("untracked working tree files would be") || low.contains("would lose untracked files");
    let tracked = low.contains("your local changes to the following files") || low.contains("please commit your changes or stash them");
    let (title, hint) = if untracked && !tracked {
        (
            "Files that git does not track are in the way".to_string(),
            "Git would replace them. Move or rename them first. Nothing was changed.",
        )
    } else if untracked {
        (
            title.to_string(),
            "Commit or stash your changes, and move the files that git does not track. Nothing was changed.",
        )
    } else {
        (title.to_string(), "Commit or stash them first. Nothing was changed.")
    };
    let mut o = refusal("dirty", title, hint);
    o.detail = files.iter().take(12).cloned().collect::<Vec<_>>().join("\n");
    o.files = files;
    o
}

fn hook_kind(mut o: Outcome, hooks: &[String]) -> Outcome {
    if hooks.is_empty() || (o.kind != "other" && o.kind != "hook") {
        return o;
    }
    let (_, title, hint, _) = super::words::describe("pre-commit hook");
    o.kind = "hook";
    o.title = title.into();
    o.hint = hint.into();
    o
}

fn job_stopped(r: &Out) -> Option<Outcome> {
    if r.cancelled {
        return Some(refusal("cancelled", "Stopped", ""));
    }
    if r.timed_out {
        let mut o = failure("timed out waiting");
        o.detail.clear();
        return Some(o);
    }
    None
}

fn making(root: &Path, args: &[&str]) -> (Out, Vec<String>) {
    let hooks = repo::commit_hooks(root);
    (run::job(root, args, None, JOB_LIMIT), hooks)
}

pub fn commit_info(root: &Path, rev: &str) -> Result<CommitInfo, String> {
    let sha = repo::resolve_commit(root, rev).ok_or("This commit is not in the repo.")?;
    let fmt = "--format=%H%x00%h%x00%P%x00%an%x00%ae%x00%at%x00%B";
    let text = read_ok(root, &["log", "-1", "--no-show-signature", fmt, &sha, "--"])?;
    let head = repo::head_sha(root);
    let branch = repo::current_branch(root);
    let refs = repo::refs_at(root, &sha);
    let held = repo::remote_branches_with(root, &sha);
    let op = repo::read_state(root).ok().and_then(|s| s.op);
    let remotes = repo::list_remotes(root);
    let upstream = branch.as_ref().and(repo::upstream_of(root));
    let mut f = text.splitn(7, '\0');
    let mut next = || f.next().unwrap_or_default().to_string();
    let (_, short, parents, author, email, at) = (next(), next(), next(), next(), next(), next());
    let (subject, body) = split_message(&next());
    let parents: Vec<String> = parents.split_whitespace().map(str::to_string).collect();
    let on_head = repo::on_line(root, &sha, head.as_deref());
    let (after, merges_after) = match &head {
        Some(h) if on_head && *h != sha => {
            let range = format!("{sha}..{h}");
            (repo::count_revs(root, &[&range]), repo::count_revs(root, &["--min-parents=2", &range]))
        }
        _ => (0, 0),
    };
    let web = repo::web_for(&remotes, &held, upstream.as_deref(), &sha);
    Ok(CommitInfo {
        short: if short.trim().is_empty() { short_sha(&sha) } else { short.trim().to_string() },
        subject,
        body,
        author,
        email,
        at: at.trim().parse().unwrap_or(0),
        is_merge: parents.len() > 1,
        is_root: parents.is_empty(),
        parents,
        on_head,
        is_head: head.as_deref() == Some(sha.as_str()),
        pushed: !held.is_empty(),
        refs,
        detached: branch.is_none() && head.is_some(),
        head,
        branch,
        upstream,
        op,
        after,
        merges_after,
        web,
        sha,
    })
}

pub fn checkout_commit(root: &Path, rev: &str, force: bool) -> Outcome {
    match repo::op_gate(root) {
        Ok(Some(busy)) => return busy,
        Err(e) => return failure(&e),
        Ok(None) => {}
    }
    let Some(sha) = repo::resolve_commit(root, rev) else {
        return missing();
    };
    let head = repo::head_sha(root);
    let branch = repo::current_branch(root);
    if branch.is_none() && head.as_deref() == Some(sha.as_str()) {
        let mut o = success("You are already on this commit");
        o.up_to_date = true;
        return o;
    }
    if let (None, Some(h)) = (&branch, &head)
        && !force
    {
        let left = repo::count_revs(root, &[h, "--not", "--branches", "--tags", "--remotes", &sha]);
        if left > 0 {
            let mut o = refusal(
                "leaving",
                format!("{} would be left behind", plural(left, "commit")),
                "They are on no branch, so they are hard to find later. Make a branch for them first, or check out anyway.",
            );
            o.count = left;
            return o;
        }
    }
    let r = run::git(root, &["switch", "--no-overwrite-ignore", "--detach", &sha]);
    if !r.ok() {
        let text = r.said();
        return if is_dirty(&text) { dirty_refusal(&text, "Your local changes would be lost") } else { failure(&text) };
    }
    let mut o = success(format!("Checked out {}", short_sha(&sha)));
    o.hint = "You are not on a branch now. Make a branch here if you want to keep new work.".into();
    o
}

pub fn create_branch_at(root: &Path, name: &str, rev: &str, checkout: bool) -> Outcome {
    let name = name.trim();
    if let Some(p) = branch_name_problem(name) {
        return refusal("invalid", "That branch name is not valid", p);
    }
    let Some(sha) = repo::resolve_commit(root, rev) else {
        return missing();
    };
    if !checkout {
        return branch_ops::create_branch(root, name, Some(&sha), false);
    }
    match repo::op_gate(root) {
        Ok(Some(busy)) => return busy,
        Err(e) => return failure(&e),
        Ok(None) => {}
    }
    let local = format!("refs/heads/{name}");
    if read(root, &["rev-parse", "-q", "--verify", &local]).ok() {
        return refusal("exists", format!("A branch named {name} already exists"), "Pick another name.");
    }
    let r = run::git(root, &["switch", "--no-overwrite-ignore", "--create", name, "--end-of-options", &sha]);
    if !r.ok() {
        let text = r.said();
        return if is_dirty(&text) { dirty_refusal(&text, "Your local changes would be lost") } else { failure(&text) };
    }
    let mut o = success(format!("Now on new branch {name}"));
    o.branch = Some(name.to_string());
    o
}

pub fn create_tag(root: &Path, name: &str, rev: &str, message: &str) -> Outcome {
    let name = name.trim();
    if let Some(p) = branch_name_problem(name) {
        return refusal("invalid", "That tag name is not valid", p);
    }
    let Some(sha) = repo::resolve_commit(root, rev) else {
        return missing();
    };
    let tag = format!("refs/tags/{name}");
    if run::git(root, &["rev-parse", "-q", "--verify", &tag]).ok() {
        return refusal("exists", format!("A tag named {name} already exists"), "Pick another name.");
    }
    let message = message.replace("\r\n", "\n").trim().to_string();
    let r = if message.is_empty() {
        run::git(root, &["tag", name, &sha])
    } else {
        let dir = match repo::temp_dir() {
            Ok(d) => d,
            Err(e) => return failure(&e),
        };
        let file = dir.0.join("msg");
        if let Err(e) = fs::write(&file, format!("{message}\n")) {
            return failure(&e.to_string());
        }
        let path = file.to_string_lossy().to_string();
        run::git(root, &["tag", "-a", "--cleanup=whitespace", "-F", &path, name, &sha])
    };
    if !r.ok() {
        return failure(&r.said());
    }
    success(format!("Made tag {name}"))
}

fn empty_refusal(op: Op, branch: &Option<String>) -> Outcome {
    if op == Op::Revert {
        refusal(
            "empty",
            "Nothing to undo",
            format!("The changes of this commit are already gone from {}. Nothing was changed.", place_name(branch)),
        )
    } else {
        refusal(
            "empty",
            "Nothing to pick",
            format!("{} already has these changes. Nothing was changed.", place_name(branch)),
        )
    }
}

fn pick_outcome(root: &Path, op: Op, r: &Out, hooks: &[String], before: &str, branch: &Option<String>, done: String) -> Outcome {
    let text = r.said();
    let state = repo::read_state(root).ok();
    if let Some(s) = state.as_ref().filter(|s| s.op == Some(op)) {
        if !s.conflicts.is_empty() {
            let mut o = refusal(
                "conflict",
                format!("The {} stopped on a conflict", op.lower()),
                "Fix the files in the Changes panel, then press Continue. Or cancel it there.",
            );
            o.detail = s.conflicts.iter().take(12).cloned().collect::<Vec<_>>().join("\n");
            o.files = s.conflicts.clone();
            return o;
        }
        let mut o = hook_kind(failure(&text), hooks);
        if o.kind == "other" {
            o.kind = "stopped";
        }
        o.hint = format!("{} The {} is waiting. Continue or cancel it in the Changes panel.", o.hint, op.lower())
            .trim()
            .to_string();
        return o;
    }
    if let Some(stop) = job_stopped(r) {
        return stop;
    }
    if !r.ok() {
        if is_dirty(&text) {
            return dirty_refusal(&text, "Your local changes are in the way");
        }
        let low = text.to_lowercase();
        if low.contains("nothing to commit") || low.contains("nothing added to commit") {
            return empty_refusal(op, branch);
        }
        return hook_kind(failure(&text), hooks);
    }
    if repo::head_sha(root).as_deref() == Some(before) {
        return empty_refusal(op, branch);
    }
    success(done)
}

fn mainline_args(parents: &[String], mainline: usize) -> Result<Vec<String>, String> {
    if parents.len() < 2 {
        return Ok(Vec::new());
    }
    if mainline < 1 || mainline > parents.len() {
        return Err(format!("Pick a parent from 1 to {}.", parents.len()));
    }
    Ok(vec!["-m".into(), mainline.to_string()])
}

fn guard(root: &Path, expect: &Expect) -> Result<(String, Option<String>), Box<Outcome>> {
    match repo::op_gate(root) {
        Ok(Some(busy)) => return Err(Box::new(busy)),
        Err(e) => return Err(Box::new(failure(&e))),
        Ok(None) => {}
    }
    let head = repo::head_sha(root).ok_or_else(unborn)?;
    let branch = repo::current_branch(root);
    if let Some(s) = stale(expect, &head, &branch) {
        return Err(Box::new(s));
    }
    Ok((head, branch))
}

pub fn cherry_pick(root: &Path, rev: &str, record_origin: bool, mainline: usize, expect: &Expect) -> Outcome {
    let Some(sha) = repo::resolve_commit(root, rev) else {
        return missing();
    };
    let (head, branch) = match guard(root, expect) {
        Ok(v) => v,
        Err(o) => return *o,
    };
    if repo::on_line(root, &sha, Some(&head)) {
        return refusal(
            "onHead",
            "This commit is already here",
            format!("{} already has it. Nothing was changed.", place_name(&branch)),
        );
    }
    let line = match mainline_args(&repo::parents_of(root, &sha), mainline) {
        Ok(l) => l,
        Err(p) => return refusal("invalid", "That parent does not exist", p),
    };
    let mut tail: Vec<&str> = Vec::new();
    if record_origin {
        tail.push("-x");
    }
    tail.extend(line.iter().map(String::as_str));
    tail.push(&sha);
    let mut args = vec!["cherry-pick", "--empty=drop"];
    args.extend(tail.iter());
    let (r, hooks) = making(root, &args);
    let done = format!("Cherry-picked {} onto {}", short_sha(&sha), place_name(&branch));
    pick_outcome(root, Op::CherryPick, &r, &hooks, &head, &branch, done)
}

pub fn revert_commit(root: &Path, rev: &str, mainline: usize, expect: &Expect) -> Outcome {
    let Some(sha) = repo::resolve_commit(root, rev) else {
        return missing();
    };
    let (head, branch) = match guard(root, expect) {
        Ok(v) => v,
        Err(o) => return *o,
    };
    if !repo::on_line(root, &sha, Some(&head)) {
        return refusal(
            "notOnHead",
            format!("This commit is not on {}", place_name(&branch)),
            "Revert undoes a commit of the branch you are on. Switch to its branch first.",
        );
    }
    let line = match mainline_args(&repo::parents_of(root, &sha), mainline) {
        Ok(l) => l,
        Err(p) => return refusal("invalid", "That parent does not exist", p),
    };
    let mut args = vec!["revert", "--no-edit"];
    args.extend(line.iter().map(String::as_str));
    args.push(&sha);
    let (r, hooks) = making(root, &args);
    let mut o = pick_outcome(root, Op::Revert, &r, &hooks, &head, &branch, format!("Reverted {}", short_sha(&sha)));
    if o.ok {
        o.hint = "A new commit undoes it. The old commit stays in the history.".into();
    }
    o
}

fn tree_leaves(text: &str) -> Vec<String> {
    split_z(text)
        .into_iter()
        .filter_map(|entry| {
            let (meta, path) = entry.split_once('\t')?;
            (meta.split(' ').nth(1) != Some("commit")).then(|| path.to_string())
        })
        .collect()
}

pub fn overwritten_untracked(root: &Path, rev: &str) -> Result<Vec<String>, String> {
    let sha = repo::resolve_commit(root, rev).ok_or("This commit is not in the repo.")?;
    let top = read_ok(root, &["rev-parse", "--show-toplevel"])?.trim().to_string();
    if top.is_empty() {
        return Err("This folder has no work tree.".into());
    }
    let top = Path::new(&top);
    let tree = read_ok(top, &["ls-tree", "-r", "-z", "--full-tree", &sha])?;
    let index = read_ok(top, &["ls-files", "-z", "--full-name"])?;
    let tracked: HashSet<String> = split_z(&index).into_iter().collect();
    let leaves: Vec<String> = tree_leaves(&tree).into_iter().filter(|p| !tracked.contains(p)).collect();
    let mut folders: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for path in &leaves {
        let mut cut = path.rfind('/');
        while let Some(c) = cut {
            if c == 0 {
                break;
            }
            let folder = &path[..c];
            if !seen.insert(folder.to_string()) {
                break;
            }
            folders.push(folder.to_string());
            cut = folder.rfind('/');
        }
    }
    let mut found = Vec::new();
    for p in &leaves {
        let full = top.join(p);
        if let Ok(m) = fs::symlink_metadata(&full) {
            if m.file_type().is_symlink() || m.is_file() {
                found.push(p.clone());
            } else if m.is_dir() && fs::read_dir(&full).map(|mut d| d.next().is_some()).unwrap_or(false) {
                found.push(format!("{p}/"));
            }
        }
    }
    for d in folders.iter().filter(|f| !tracked.contains(*f)) {
        if let Ok(m) = fs::symlink_metadata(top.join(d))
            && !m.is_dir()
        {
            found.push(d.clone());
        }
    }
    found.sort();
    found.dedup();
    Ok(found)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResetPreview {
    pub sha: String,
    pub head: String,
    pub branch: Option<String>,
    pub leaving: usize,
    pub joining: usize,
    pub only_here: usize,
    pub pushed_leaving: usize,
    pub files: Vec<String>,
    pub file_count: usize,
    pub untracked_lost: Option<Vec<String>>,
}

pub fn reset_preview(root: &Path, rev: &str) -> Result<ResetPreview, Box<Outcome>> {
    let sha = repo::resolve_commit(root, rev).ok_or_else(missing)?;
    let head = repo::head_sha(root).ok_or_else(unborn)?;
    let branch = repo::current_branch(root);
    let mut others: Vec<String> = Vec::new();
    if let Some(b) = &branch {
        others.push(format!("--exclude={b}"));
    }
    others.extend(["--branches".into(), "--tags".into(), "--remotes".into()]);
    let leaving = repo::count_revs(root, &[&format!("{sha}..{head}")]);
    let joining = repo::count_revs(root, &[&format!("{head}..{sha}")]);
    let mut only: Vec<&str> = vec![&head, "--not", &sha];
    only.extend(others.iter().map(String::as_str));
    let only_here = repo::count_revs(root, &only);
    let unpushed = repo::count_revs(root, &[&head, "--not", &sha, "--remotes"]);
    let status = read(root, &["status", "--porcelain=v1", "-z", "--untracked-files=no"]);
    if !status.ok() {
        return Err(Box::new(failure(&status.said())));
    }
    let files = repo::status_paths(&status.stdout);
    Ok(ResetPreview {
        untracked_lost: overwritten_untracked(root, &sha).ok(),
        sha,
        head,
        branch,
        leaving,
        joining,
        only_here,
        pushed_leaving: leaving.saturating_sub(unpushed),
        file_count: files.len(),
        files: files.into_iter().take(50).collect(),
    })
}

pub fn reset_to(root: &Path, rev: &str, mode: ResetMode, expect: &Expect, known_untracked: &[String]) -> Outcome {
    let Some(sha) = repo::resolve_commit(root, rev) else {
        return missing();
    };
    let (head, branch) = match guard(root, expect) {
        Ok(v) => v,
        Err(o) => return *o,
    };
    if mode == ResetMode::Hard {
        match overwritten_untracked(root, &sha) {
            Err(e) => {
                let mut o = refusal("untracked", "Cannot check the files that git does not track", "Nothing was changed.");
                o.detail = e;
                return o;
            }
            Ok(lost) => {
                let fresh: Vec<String> = lost.iter().filter(|p| !known_untracked.contains(p)).cloned().collect();
                if !fresh.is_empty() {
                    let title = if fresh.len() == 1 {
                        "A file that git does not track would be replaced".to_string()
                    } else {
                        format!("{} files that git does not track would be replaced", fresh.len())
                    };
                    let mut o = refusal("untracked", title, "Move them first, or open Reset again to see them all. Nothing was changed.");
                    o.detail = fresh.iter().take(12).cloned().collect::<Vec<_>>().join("\n");
                    o.files = fresh;
                    return o;
                }
            }
        }
    }
    let r = run::git(root, &["reset", mode.flag(), "-q", &sha, "--"]);
    if !r.ok() {
        return failure(&r.said());
    }
    let mut o = success(format!("Moved {} to {}", branch.clone().unwrap_or_else(|| "HEAD".into()), short_sha(&sha)));
    o.hint = format!("{} It was at {} before.", mode.done(), short_sha(&head));
    o
}

pub fn undo_last_commit(root: &Path, expect: &Expect) -> Outcome {
    match repo::op_gate(root) {
        Ok(Some(busy)) => return busy,
        Err(e) => return failure(&e),
        Ok(None) => {}
    }
    let Some(head) = repo::head_sha(root) else {
        return refusal("unborn", "There is no commit to undo", "");
    };
    let branch = repo::current_branch(root);
    if let Some(s) = stale(expect, &head, &branch) {
        return s;
    }
    let parents = repo::parents_of(root, &head);
    if parents.is_empty() {
        return refusal("root", "This is the first commit", "It has nothing before it, so it cannot be undone this way.");
    }
    if parents.len() > 1 {
        return refusal("merge", "The last commit is a merge", "Use Reset to here on the commit you want to go back to.");
    }
    if !repo::remote_branches_with(root, &head).is_empty() {
        return refusal(
            "pushed",
            "This commit is already on the remote",
            "Undo would make your branch differ from the remote. Use Revert instead.",
        );
    }
    let r = run::git(root, &["reset", "--soft", "-q", &parents[0], "--"]);
    if !r.ok() {
        return failure(&r.said());
    }
    let mut o = success("Last commit undone");
    o.hint = "Its changes are staged, so nothing was lost.".into();
    o
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainItem {
    pub sha: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub name: String,
    pub email: String,
    pub date: String,
    pub encoding: String,
}

pub fn parse_chain(text: &str) -> Vec<ChainItem> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\0').collect();
            let get = |i: usize| f.get(i).copied().unwrap_or_default().to_string();
            ChainItem {
                sha: get(0),
                tree: get(1),
                parents: get(2).split_whitespace().map(str::to_string).collect(),
                name: get(3),
                email: get(4),
                date: get(5).trim().to_string(),
                encoding: get(6).trim().to_string(),
            }
        })
        .collect()
}

fn raw_date(d: &str) -> bool {
    let Some((secs, zone)) = d.split_once(' ') else {
        return false;
    };
    let secs = secs.strip_prefix('-').unwrap_or(secs);
    !secs.is_empty()
        && secs.chars().all(|c| c.is_ascii_digit())
        && zone.len() == 5
        && (zone.starts_with('+') || zone.starts_with('-'))
        && zone[1..].chars().all(|c| c.is_ascii_digit())
}

fn safe_encoding(e: &str) -> bool {
    let mut c = e.chars();
    c.next().is_some_and(|f| f.is_ascii_alphanumeric()) && c.all(|x| x.is_ascii_alphanumeric() || "._:-".contains(x))
}

pub fn chain_problem(chain: &[ChainItem], sha: &str, head: &str, parents: &[String]) -> bool {
    let (Some(first), Some(last)) = (chain.first(), chain.last()) else {
        return true;
    };
    if first.sha != sha || last.sha != head || first.parents != parents {
        return true;
    }
    chain.iter().enumerate().any(|(k, c)| {
        !is_full_sha(&c.sha)
            || !is_full_sha(&c.tree)
            || !raw_date(&c.date)
            || (!c.encoding.is_empty() && !safe_encoding(&c.encoding))
            || (k > 0 && (c.parents.len() != 1 || c.parents[0] != chain[k - 1].sha))
    })
}

fn rebuild_chain(root: &Path, chain: &[ChainItem], first_file: &Path, dir: &Path, sign: bool) -> Result<String, Box<Outcome>> {
    let mut parent = chain[0].parents.first().cloned();
    for (k, c) in chain.iter().enumerate() {
        let encoding = if c.encoding.is_empty() { "UTF-8".to_string() } else { c.encoding.clone() };
        let file = if k == 0 { first_file.to_path_buf() } else { dir.join(format!("m{k}")) };
        let file_text = file.to_string_lossy().to_string();
        if k > 0 {
            let enc = format!("--encoding={encoding}");
            let output = format!("--output={file_text}");
            let out = run::git(root, &["log", "-1", "--no-show-signature", "--no-notes", "--no-color", &enc, "--pretty=format:%B", &output, &c.sha, "--"]);
            if !out.ok() {
                return Err(Box::new(unchanged(failure(&out.said()))));
            }
        }
        let date = format!("@{}", c.date);
        let env = [
            ("GIT_AUTHOR_NAME", c.name.as_str()),
            ("GIT_AUTHOR_EMAIL", c.email.as_str()),
            ("GIT_AUTHOR_DATE", date.as_str()),
        ];
        let enc_cfg = format!("i18n.commitEncoding={encoding}");
        let mut args: Vec<&str> = Vec::new();
        if k > 0 {
            args.extend(["-c", &enc_cfg]);
        }
        args.push("commit-tree");
        if sign {
            args.push("-S");
        }
        args.push(&c.tree);
        if let Some(p) = &parent {
            args.extend(["-p", p]);
        }
        args.extend(["-F", &file_text]);
        let made = run::git_env(root, &args, &env);
        let next = made.line();
        if !made.ok() || !is_full_sha(&next) {
            return Err(Box::new(unchanged(failure(&made.said()))));
        }
        parent = Some(next);
    }
    parent.ok_or_else(|| Box::new(unchanged(failure("cannot rebuild the commits"))))
}

fn message_hook(root: &Path, file: &Path) -> Option<Outcome> {
    let hooks = repo::commit_hooks(root);
    if !hooks.iter().any(|h| h == "commit-msg") {
        return None;
    }
    let path = file.to_string_lossy().to_string();
    let r = run::job(root, &["hook", "run", "--ignore-missing", "commit-msg", "--", &path], None, JOB_LIMIT);
    if let Some(mut stop) = job_stopped(&r) {
        stop.hint = "Nothing was changed.".into();
        return Some(stop);
    }
    if !r.ok() {
        return Some(unchanged(hook_kind(failure(&r.said()), &hooks)));
    }
    let after = fs::read_to_string(file).unwrap_or_default();
    if after.trim().is_empty() {
        return Some(refusal("empty", "The commit-msg hook left the message empty", "Nothing was changed."));
    }
    None
}

fn reword_older(root: &Path, sha: &str, head: &str, branch: &Option<String>, parents: &[String], file: &Path, dir: &Path) -> Result<String, Box<Outcome>> {
    let mut args: Vec<String> = ["log", "--no-show-signature", "--topo-order", "--reverse", "--date=raw", "--format=%H%x00%T%x00%P%x00%an%x00%ae%x00%ad%x00%e", head]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if !parents.is_empty() {
        args.push("--not".into());
        args.extend(parents.iter().cloned());
    }
    args.push("--".into());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let chain = parse_chain(&read_ok(root, &refs).map_err(|e| failure(&e))?);
    if chain_problem(&chain, sha, head, parents) {
        return Err(Box::new(refusal(
            "chain",
            "Cannot change this message here",
            "The commits after it could not be read in order. Nothing was changed.",
        )));
    }
    if let Some(stop) = message_hook(root, file) {
        return Err(Box::new(stop));
    }
    let sign = run::git(root, &["config", "--bool", "commit.gpgSign"]).line() == "true";
    let built = rebuild_chain(root, &chain, file, dir, sign)?;
    if &repo::current_branch(root) != branch {
        return Err(Box::new(branch_changed()));
    }
    let target = branch.as_ref().map_or("HEAD".to_string(), |b| format!("refs/heads/{b}"));
    let why = format!("edit message of {}", short_sha(sha));
    let moved = run::git(root, &["update-ref", "-m", &why, &target, &built, head]);
    if !moved.ok() {
        let text = moved.said();
        return Err(Box::new(if text.contains("but expected") || text.contains(" is at ") { head_changed() } else { unchanged(failure(&text)) }));
    }
    Ok(built)
}

pub fn reword_commit(root: &Path, rev: &str, summary: &str, body: &str, expect: &Expect) -> Outcome {
    if summary.trim().is_empty() {
        return refusal("empty", "Write a summary first", "The first line says what changed.");
    }
    let message = compose_message(summary, body);
    let Some(sha) = repo::resolve_commit(root, rev) else {
        return missing();
    };
    let (head, branch) = match guard(root, expect) {
        Ok(v) => v,
        Err(o) => return *o,
    };
    if !repo::on_line(root, &sha, Some(&head)) {
        return refusal(
            "notOnHead",
            format!("This commit is not on {}", place_name(&branch)),
            "You can only change the message of a commit on the branch you are on.",
        );
    }
    let old = read_ok(root, &["log", "-1", "--no-show-signature", "--format=%B", &sha, "--"]).unwrap_or_default();
    let (was_s, was_b) = split_message(&old);
    if compose_message(&was_s, &was_b) == message {
        return refusal("same", "The message is the same", "Nothing was changed.");
    }
    let parents = repo::parents_of(root, &sha);
    if sha != head {
        if parents.len() > 1 {
            return refusal("merge", "This commit is a merge", "Only the newest commit's message can be changed when it is a merge.");
        }
        if repo::count_revs(root, &["--min-parents=2", &format!("{sha}..{head}")]) > 0 {
            return refusal(
                "mergeBetween",
                "There is a merge after this commit",
                "Older commits cannot be changed when a merge comes after them. Nothing was changed.",
            );
        }
    }
    let pushed = !repo::remote_branches_with(root, &sha).is_empty();
    let dir = match repo::temp_dir() {
        Ok(d) => d,
        Err(e) => return failure(&e),
    };
    let file = dir.0.join("msg");
    if let Err(e) = fs::write(&file, &message) {
        return failure(&e.to_string());
    }
    if sha == head {
        let path = file.to_string_lossy().to_string();
        let (r, hooks) = making(root, &["commit", "--amend", "--only", "-F", &path]);
        if let Some(stop) = job_stopped(&r) {
            return stop;
        }
        if !r.ok() {
            return hook_kind(failure(&r.said()), &hooks);
        }
    } else if let Err(o) = reword_older(root, &sha, &head, &branch, &parents, &file, &dir.0) {
        return *o;
    }
    let mut o = success("Message changed");
    if pushed {
        o.hint = "This commit was already on the remote. You need a force push to update it there.".into();
        o.warning = true;
    }
    o
}
