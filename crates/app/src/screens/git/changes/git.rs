use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const GIT_ENV: [(&str, &str); 4] = [
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GCM_INTERACTIVE", "never"),
    ("GIT_EDITOR", "true"),
    ("GIT_MERGE_AUTOEDIT", "no"),
];

pub const READ: [&str; 6] = [
    "--literal-pathspecs",
    "--no-optional-locks",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "diff.autoRefreshIndex=false",
];

pub const MAX_FILES: usize = 1000;
pub const MAX_REPOS: usize = 24;
pub const JOB_LIMIT: Duration = Duration::from_secs(600);
pub const SWITCH_TAG: &str = "nixa-switch:";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Out {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub cancelled: bool,
    pub timed_out: bool,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    pub fn said(&self) -> String {
        [self.stderr.trim(), self.stdout.trim()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn command(root: &str, args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in GIT_ENV {
        c.env(k, v);
    }
    c
}

fn broken(e: std::io::Error) -> Out {
    Out {
        code: None,
        stderr: e.to_string(),
        ..Out::default()
    }
}

pub fn run(root: &str, args: &[&str]) -> Out {
    match command(root, args).output() {
        Ok(o) => Out {
            code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
            ..Out::default()
        },
        Err(e) => broken(e),
    }
}

pub fn run_in(root: &str, args: &[&str], input: &str) -> Out {
    let mut c = command(root, args);
    c.stdin(Stdio::piped());
    let mut child = match c.spawn() {
        Ok(child) => child,
        Err(e) => return broken(e),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let text = input.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    match child.wait_with_output() {
        Ok(o) => Out {
            code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
            ..Out::default()
        },
        Err(e) => broken(e),
    }
}

pub fn read(root: &str, args: &[&str]) -> Out {
    let mut all: Vec<&str> = READ.to_vec();
    all.extend_from_slice(args);
    run(root, &all)
}

pub fn owned(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

fn pipe_text(mut pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(p) = pipe.as_mut() {
            let _ = p.read_to_string(&mut text);
        }
        text
    })
}

fn stop_child(child: &mut Child) {
    let pid = child.id();
    let _ = Command::new("/bin/kill")
        .args(["-9", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

pub fn job(root: &str, args: &[&str], cancel: &AtomicBool, limit: Duration) -> Out {
    use std::os::unix::process::CommandExt;
    let mut c = command(root, args);
    c.process_group(0);
    let own_ssh = std::env::var_os("GIT_SSH_COMMAND").is_some()
        || std::env::var_os("GIT_SSH").is_some()
        || run(root, &["config", "--get", "core.sshCommand"]).ok();
    if !own_ssh {
        c.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let mut child = match c.spawn() {
        Ok(child) => child,
        Err(e) => return broken(e),
    };
    let out = pipe_text(child.stdout.take());
    let err = pipe_text(child.stderr.take());
    let start = Instant::now();
    let mut result = Out::default();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                result.code = status.code();
                break;
            }
            Ok(None) => {}
            Err(_) => break,
        }
        if cancel.load(Ordering::Relaxed) {
            stop_child(&mut child);
            result.cancelled = true;
            break;
        }
        if start.elapsed() > limit {
            stop_child(&mut child);
            result.timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    result.stdout = out.join().unwrap_or_default();
    result.stderr = err.join().unwrap_or_default();
    result
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    None,
    Timeout,
    Identity,
    NoUpstream,
    Rejected,
    Auth,
    NotFound,
    Network,
    Lock,
    Conflict,
    Dirty,
    Hook,
    Nothing,
    Other,
    Cancelled,
    Detached,
    BranchChanged,
    NoRemote,
    Invalid,
    Empty,
    HeadChanged,
    Diverged,
    Markers,
    Apply,
    StashMoved,
}

const RULES: [(Kind, &[&str]); 12] = [
    (Kind::Timeout, &["took longer than", "timed out waiting"]),
    (
        Kind::Identity,
        &[
            "please tell me who you are",
            "empty ident name",
            "unable to auto-detect email",
            "user.useconfigonly",
        ],
    ),
    (
        Kind::NoUpstream,
        &[
            "has no upstream branch",
            "no upstream configured",
            "there is no tracking information",
        ],
    ),
    (
        Kind::Rejected,
        &[
            "[rejected]",
            "non-fast-forward",
            "fetch first",
            "updates were rejected",
            "stale info",
        ],
    ),
    (
        Kind::Auth,
        &[
            "authentication failed",
            "could not read username",
            "could not read password",
            "terminal prompts disabled",
            "invalid username or password",
            "invalid username or token",
            "denied to ",
            "returned error: 401",
            "returned error: 403",
            "http 401",
            "http 403",
            "bad credentials",
            "permission denied (publickey",
            "write access to repository not granted",
        ],
    ),
    (
        Kind::NotFound,
        &[
            "repository not found",
            "does not appear to be a git repository",
            "returned error: 404",
        ],
    ),
    (
        Kind::Network,
        &[
            "could not resolve host",
            "failed to connect",
            "connection timed out",
            "connection refused",
            "connection reset",
            "operation timed out",
            "recv failure",
            "send failure",
            "the requested url returned error: 5",
        ],
    ),
    (
        Kind::Lock,
        &["index.lock", "another git process seems to be running"],
    ),
    (
        Kind::Conflict,
        &[
            "conflict (",
            "automatic merge failed",
            "could not apply",
            "fix conflicts and then",
            "resolve all conflicts",
            "you need to resolve your current index first",
            "unmerged files",
        ],
    ),
    (
        Kind::Dirty,
        &[
            "would be overwritten by merge",
            "would be overwritten by checkout",
            "please commit your changes or stash them",
            "you have unstaged changes",
            "your local changes to the following files",
        ],
    ),
    (
        Kind::Hook,
        &[
            "hook declined",
            "hook failed",
            "pre-commit hook",
            "commit-msg hook",
            "pre-push hook",
            "husky",
            "lint-staged",
        ],
    ),
    (
        Kind::Nothing,
        &[
            "nothing to commit",
            "no changes added to commit",
            "nothing added to commit",
        ],
    ),
];

pub fn classify(text: &str) -> Kind {
    let low = text.to_lowercase();
    RULES
        .iter()
        .find(|(_, words)| words.iter().any(|w| low.contains(w)))
        .map(|(k, _)| *k)
        .unwrap_or(Kind::Other)
}

pub fn words(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Timeout => (
            "Git took too long",
            "It was stopped. Try again, or run it in a terminal.",
        ),
        Kind::Identity => ("No git identity for this repo", "Pick a profile for this repo."),
        Kind::NoUpstream => (
            "This branch has no remote branch yet",
            "Push once to publish it.",
        ),
        Kind::Rejected => (
            "The remote has newer commits",
            "Pull first, or force push with lease.",
        ),
        Kind::Auth => (
            "Git sign-in failed",
            "Check the account and token for this repo.",
        ),
        Kind::NotFound => (
            "Remote repository not found",
            "Check the remote URL and that this account can see the repo.",
        ),
        Kind::Network => ("Cannot reach the remote", "Check the network and try again."),
        Kind::Lock => (
            "Another git command is running",
            "Wait a moment, then try again. If it stays, remove .git/index.lock.",
        ),
        Kind::Conflict => (
            "There are conflicts",
            "Resolve the conflicted files, then continue.",
        ),
        Kind::Dirty => (
            "Local changes are in the way",
            "Commit or stash your changes first.",
        ),
        Kind::Hook => (
            "A git hook stopped this",
            "Fix what the hook says, or run without hooks.",
        ),
        Kind::Nothing => ("Nothing to commit", "Stage some changes first."),
        _ => ("Git failed", ""),
    }
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if ('@'..='~').contains(&d) {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

pub fn detail_of(text: &str) -> String {
    let clean = strip_ansi(text);
    let lines: Vec<String> = clean
        .trim()
        .lines()
        .filter(|l| !l.trim_start().to_lowercase().starts_with("hint:"))
        .map(|l| {
            let t = l.trim();
            let low = t.to_lowercase();
            for p in ["error:", "fatal:", "remote:"] {
                if low.starts_with(p) {
                    return t[p.len()..].trim().to_string();
                }
            }
            t.to_string()
        })
        .filter(|l| !l.is_empty())
        .collect();
    let from = lines.len().saturating_sub(4);
    lines[from..].join("\n")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Done {
    pub ok: bool,
    pub kind: Kind,
    pub title: String,
    pub hint: String,
    pub detail: String,
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub published: bool,
    pub up_to_date: bool,
    pub stale: bool,
    pub stash_conflict: bool,
    pub tags: Vec<String>,
    pub state: Option<RepoState>,
    pub check: Option<PullCheck>,
}

impl Done {
    pub fn body(&self) -> String {
        [self.hint.as_str(), self.detail.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn success(title: impl Into<String>) -> Done {
    Done {
        ok: true,
        title: title.into(),
        ..Done::default()
    }
}

pub fn refusal(kind: Kind, title: impl Into<String>, hint: impl Into<String>) -> Done {
    Done {
        kind,
        title: title.into(),
        hint: hint.into(),
        ..Done::default()
    }
}

pub fn failure(text: &str) -> Done {
    let kind = classify(text);
    let (title, hint) = words(kind);
    Done {
        kind,
        title: title.to_string(),
        hint: hint.to_string(),
        detail: detail_of(text),
        ..Done::default()
    }
}

fn stopped(out: &Out, what: &str) -> Option<Done> {
    if out.cancelled {
        return Some(refusal(Kind::Cancelled, format!("{what} stopped"), ""));
    }
    if out.timed_out {
        return Some(failure("took longer than 600 s and was stopped"));
    }
    None
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn base_name(path: &str) -> &str {
    let t = path.trim_end_matches('/');
    t.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(t)
}

pub fn folder_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub add: u32,
    pub del: u32,
    pub binary: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub folder: String,
    pub status: char,
    pub fresh: bool,
    pub conflict: bool,
    pub x: char,
    pub y: char,
    pub staged: bool,
    pub add: u32,
    pub del: u32,
    pub binary: bool,
}

pub fn status_file(entry: &str, stats: &HashMap<String, Stat>) -> Option<FileEntry> {
    if entry.len() < 4 || !entry.is_char_boundary(3) {
        return None;
    }
    let xy = &entry[..2];
    let mut chars = xy.chars();
    let x = chars.next()?;
    let y = chars.next()?;
    let path = entry[3..].to_string();
    let fresh = xy == "??";
    let conflict = xy.contains('U') || xy == "AA" || xy == "DD";
    let status = if conflict {
        'U'
    } else if xy.contains('D') {
        'D'
    } else if fresh || xy.contains('A') {
        'A'
    } else {
        'M'
    };
    let stat = stats.get(&path).copied().unwrap_or_default();
    Some(FileEntry {
        folder: folder_of(&path).to_string(),
        status,
        fresh,
        conflict,
        x,
        y,
        staged: !conflict && x != ' ' && x != '?',
        add: stat.add,
        del: stat.del,
        binary: stat.binary,
        path,
    })
}

pub fn add_numstat(text: &str, stats: &mut HashMap<String, Stat>) {
    for entry in text.split('\0') {
        let mut parts = entry.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let a = a.trim_start_matches('\n');
        let old = stats.get(path).copied().unwrap_or_default();
        stats.insert(
            path.to_string(),
            Stat {
                add: old.add + a.parse::<u32>().unwrap_or(0),
                del: old.del + d.parse::<u32>().unwrap_or(0),
                binary: a == "-",
            },
        );
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub branch: Option<String>,
    pub detached: bool,
    pub unborn: bool,
    pub ahead: u32,
    pub behind: u32,
    pub upstream: Option<String>,
}

pub fn parse_header(line: Option<&str>) -> Header {
    let mut h = Header::default();
    let Some(text) = line else { return h };
    if let Some(rest) = text.strip_prefix("No commits yet on ") {
        h.unborn = true;
        h.branch = Some(rest.trim().to_string());
        return h;
    }
    if text.starts_with("HEAD (no branch)") {
        h.detached = true;
        return h;
    }
    if let (Some(a), Some(b)) = (text.find('['), text.rfind(']'))
        && a < b {
            let track = &text[a + 1..b];
            for part in track.split(',') {
                let part = part.trim();
                if let Some(n) = part.strip_prefix("ahead ") {
                    h.ahead = n.trim().parse().unwrap_or(0);
                }
                if let Some(n) = part.strip_prefix("behind ") {
                    h.behind = n.trim().parse().unwrap_or(0);
                }
            }
        }
    match text.find("...") {
        Some(dot) => {
            h.branch = Some(text[..dot].trim().to_string());
            h.upstream = text[dot + 3..]
                .split(' ')
                .next()
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        None => h.branch = Some(text.trim().to_string()),
    }
    h
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Op {
    pub fn word(self) -> &'static str {
        match self {
            Op::Merge => "Merge",
            Op::Rebase => "Rebase",
            Op::CherryPick => "Cherry-pick",
            Op::Revert => "Revert",
        }
    }

    pub fn doing(self) -> &'static str {
        match self {
            Op::Merge => "Merging",
            Op::Rebase => "Rebasing",
            Op::CherryPick => "Cherry-picking",
            Op::Revert => "Reverting",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Op::Merge => "merge",
            Op::Rebase => "rebase",
            Op::CherryPick => "cherry-pick",
            Op::Revert => "revert",
        }
    }

    pub fn can_skip(self) -> bool {
        self != Op::Merge
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoState {
    pub op: Option<Op>,
    pub step: Option<(u32, u32)>,
    pub conflicts: Vec<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
}

fn read_trim(path: &Path) -> String {
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn read_state(root: &str) -> Option<RepoState> {
    let out = run(
        root,
        &[
            "rev-parse",
            "--git-path",
            "MERGE_HEAD",
            "--git-path",
            "rebase-merge",
            "--git-path",
            "rebase-apply",
            "--git-path",
            "CHERRY_PICK_HEAD",
            "--git-path",
            "REVERT_HEAD",
            "--git-path",
            "sequencer/todo",
        ],
    );
    if !out.ok() {
        return None;
    }
    let paths: Vec<PathBuf> = out
        .stdout
        .lines()
        .map(|l| {
            let p = PathBuf::from(l);
            if p.is_absolute() { p } else { Path::new(root).join(p) }
        })
        .collect();
    if paths.len() < 6 {
        return None;
    }
    let has = |i: usize| paths[i].exists();
    let dir = if paths[1].is_dir() {
        Some(&paths[1])
    } else if paths[2].is_dir() {
        Some(&paths[2])
    } else {
        None
    };
    let applying = dir.map(|d| d.join("applying").exists()).unwrap_or(false);
    let seq = if has(5) {
        std::fs::read_to_string(&paths[5])
            .unwrap_or_default()
            .lines()
            .find(|l| !l.starts_with('#') && !l.trim().is_empty())
            .and_then(|l| l.split_whitespace().next())
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    let op = if (has(1) || has(2)) && !applying {
        Some(Op::Rebase)
    } else if has(1) || has(2) {
        None
    } else if has(0) {
        Some(Op::Merge)
    } else if has(3) {
        Some(Op::CherryPick)
    } else if has(4) {
        Some(Op::Revert)
    } else if has(5) {
        match seq.as_str() {
            "revert" => Some(Op::Revert),
            "pick" | "p" => Some(Op::CherryPick),
            _ => None,
        }
    } else {
        None
    };
    let step = match (op, dir) {
        (Some(Op::Rebase), Some(d)) => {
            let merge = d == &paths[1];
            let (a, b) = if merge { ("msgnum", "end") } else { ("next", "last") };
            let n: u32 = read_trim(&d.join(a)).parse().unwrap_or(0);
            let total: u32 = read_trim(&d.join(b)).parse().unwrap_or(0);
            (total > 0).then_some((n, total))
        }
        _ => None,
    };
    let head_name = dir.map(|d| read_trim(&d.join("head-name"))).unwrap_or_default();
    let conflicts = read(
        root,
        &["-c", "core.quotepath=off", "diff", "--name-only", "--diff-filter=U", "-z"],
    )
    .stdout
    .split('\0')
    .filter(|s| !s.is_empty())
    .map(str::to_string)
    .collect();
    let branch_now = run(root, &["symbolic-ref", "-q", "--short", "HEAD"])
        .stdout
        .trim()
        .to_string();
    let from_rebase = (op == Some(Op::Rebase))
        .then(|| head_name.strip_prefix("refs/heads/").map(str::to_string))
        .flatten();
    let head = run(root, &["rev-parse", "-q", "--verify", "--short", "HEAD"])
        .stdout
        .trim()
        .to_string();
    Some(RepoState {
        op,
        step,
        conflicts,
        branch: from_rebase.or((!branch_now.is_empty()).then_some(branch_now)),
        head: (!head.is_empty()).then_some(head),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeftStash {
    pub reference: String,
    pub sha: String,
    pub to: String,
}

pub fn left_stash_for(text: &str, branch: &str) -> Option<LeftStash> {
    for line in text.lines() {
        let mut parts = line.split('\0');
        let (Some(reference), Some(sha), Some(subject)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Some(at) = subject.find(": ") else { continue };
        let Some(rest) = subject[at + 2..].trim().strip_prefix(SWITCH_TAG) else {
            continue;
        };
        let Some(rest) = rest.trim().strip_prefix("left ") else { continue };
        let Some((from, to)) = rest.split_once(" for ") else { continue };
        if from == branch && !reference.is_empty() && !to.contains(' ') {
            return Some(LeftStash {
                reference: reference.to_string(),
                sha: sha.to_string(),
                to: to.to_string(),
            });
        }
    }
    None
}

pub fn read_left_stash(root: &str, branch: &str) -> Option<LeftStash> {
    let out = read(root, &["stash", "list", "--format=%gd%x00%H%x00%gs"]);
    out.ok().then(|| left_stash_for(&out.stdout, branch)).flatten()
}

fn stash_pop(root: &str, reference: &str, index: bool) -> Done {
    let mut args = vec!["stash", "pop"];
    if index {
        args.push("--index");
    }
    args.push(reference);
    let out = run(root, &args);
    if out.ok() {
        return success("Changes brought back");
    }
    let mut done = failure(&out.said());
    if done.kind == Kind::Conflict {
        done.title = "Your changes clashed with the files".into();
        done.hint = "The clashing files are marked. The stash is kept.".into();
    }
    done
}

pub fn bring_back(root: &str, stash: &LeftStash) -> Done {
    let now = run(root, &["rev-parse", "-q", "--verify", &stash.reference]);
    if now.stdout.trim() != stash.sha {
        return refusal(
            Kind::StashMoved,
            "The stash list changed",
            "Open the stash window and bring the changes back from there.",
        );
    }
    let first = stash_pop(root, &stash.reference, true);
    if first.ok || first.kind == Kind::Conflict {
        return first;
    }
    stash_pop(root, &stash.reference, false)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub root: String,
    pub files: Vec<FileEntry>,
    pub line_totals: (u64, u64),
    pub limited: bool,
    pub has_head: bool,
    pub has_remote: bool,
    pub header: Header,
    pub head_sha: Option<String>,
    pub state: Option<RepoState>,
    pub left: Option<LeftStash>,
}

pub fn snapshot(root: &str) -> Result<Snapshot, String> {
    let status = read(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "-b",
            "--no-renames",
            "--untracked-files=all",
        ],
    );
    if !status.ok() {
        return Err(detail_of(&status.said()));
    }
    let head = run(root, &["rev-parse", "--verify", "-q", "HEAD"]);
    let remote = run(root, &["remote"]);
    let state = read_state(root);
    let mut stats = HashMap::new();
    let outs = if head.ok() {
        vec![read(root, &["diff", "HEAD", "--numstat", "-z", "--no-renames"])]
    } else {
        vec![
            read(root, &["diff", "--cached", "--numstat", "-z", "--no-renames"]),
            read(root, &["diff", "--numstat", "-z", "--no-renames"]),
        ]
    };
    for out in outs {
        if !out.ok() {
            return Err(detail_of(&out.said()));
        }
        add_numstat(&out.stdout, &mut stats);
    }
    let mut entries: Vec<&str> = status.stdout.split('\0').filter(|e| !e.is_empty()).collect();
    let mut header = None;
    if entries.first().is_some_and(|e| e.starts_with("## ")) {
        header = Some(entries.remove(0)[3..].to_string());
    }
    let line_totals = stats.values().fold((0, 0), |(add, del), stat| {
        (add + u64::from(stat.add), del + u64::from(stat.del))
    });
    let limited = entries.len() > MAX_FILES;
    let files = entries
        .iter()
        .take(MAX_FILES)
        .filter_map(|e| status_file(e, &stats))
        .collect();
    let header = parse_header(header.as_deref());
    let left = match (&header.branch, header.detached) {
        (Some(b), false) => read_left_stash(root, b),
        _ => None,
    };
    Ok(Snapshot {
        root: root.to_string(),
        files,
        line_totals,
        limited,
        has_head: head.ok(),
        has_remote: !remote.stdout.trim().is_empty(),
        head_sha: head.ok().then(|| head.stdout.trim().to_string()),
        header,
        state,
        left,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repo {
    pub name: String,
    pub path: String,
    pub count: usize,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    a == b
}

fn repo_at(dir: &Path, name: String) -> Option<Repo> {
    let path = dir.to_string_lossy().to_string();
    let top = run(&path, &["rev-parse", "--show-toplevel"]);
    if !top.ok() || !same_dir(Path::new(top.stdout.trim()), dir) {
        return None;
    }
    let count = read(&path, &["status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all"])
        .stdout
        .split('\0')
        .filter(|l| !l.is_empty())
        .count();
    Some(Repo { name, path, count })
}

pub fn skip_child(name: &str) -> bool {
    name.starts_with('.')
        || matches!(name, "node_modules" | "vendor" | "target" | "dist" | "build")
}

pub fn discover(root: &str) -> Vec<Repo> {
    let base = Path::new(root);
    let mut out = Vec::new();
    if !base.is_dir() {
        return out;
    }
    if let Some(r) = repo_at(base, base_name(root).to_string()) {
        out.push(r);
    }
    let mut kids: Vec<PathBuf> = std::fs::read_dir(base)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    kids.sort();
    let mut n = 0;
    for kid in kids {
        let name = kid
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if skip_child(&name) {
            continue;
        }
        if let Some(r) = repo_at(&kid, name) {
            out.push(r);
        }
        n += 1;
        if n >= MAX_REPOS {
            break;
        }
    }
    out
}

pub fn top_of(dir: &str) -> Option<String> {
    let out = run(dir, &["rev-parse", "--show-toplevel"]);
    out.ok().then(|| out.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn inside(path: &str, folder: &str) -> bool {
    let f = folder.trim_end_matches('/');
    path == f || path.starts_with(&format!("{f}/"))
}

pub fn scan_root(cwd: &str, folder: Option<&str>, home: &str) -> String {
    match folder.map(|f| f.trim_end_matches('/')) {
        Some(f) if !f.is_empty() && f != home && inside(cwd, f) => f.to_string(),
        _ => cwd.to_string(),
    }
}

pub fn location(cwd: Option<&str>, folder: Option<&str>, home: &str) -> Option<String> {
    let folder = folder.map(|f| f.trim_end_matches('/')).filter(|f| !f.is_empty());
    match (cwd, folder) {
        (Some(c), Some(f)) if f != home && !inside(c, f) => Some(f.to_string()),
        (Some(c), _) => Some(c.to_string()),
        (None, Some(f)) => Some(f.to_string()),
        (None, None) => None,
    }
}

pub fn choose_root(cwd: &str, repos: &[Repo], pick: Option<&str>) -> Option<String> {
    if let Some(p) = pick.filter(|p| repos.iter().any(|r| r.path == *p)) {
        return Some(p.to_string());
    }
    repos
        .iter()
        .filter(|r| inside(cwd, &r.path))
        .max_by_key(|r| r.path.len())
        .or(repos.first())
        .map(|r| r.path.clone())
}

pub fn has_hooks(root: &str, names: &[&str]) -> bool {
    let out = run(root, &["rev-parse", "--git-path", "hooks"]);
    if !out.ok() {
        return false;
    }
    let dir = PathBuf::from(out.stdout.trim());
    let dir = if dir.is_absolute() { dir } else { Path::new(root).join(dir) };
    names.iter().any(|n| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(dir.join(n))
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

pub const COMMIT_HOOKS: [&str; 4] = ["pre-commit", "prepare-commit-msg", "commit-msg", "post-commit"];

pub fn compose_message(summary: &str, body: &str) -> String {
    let head = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    let rest = body.replace("\r\n", "\n");
    let rest = rest.trim_end();
    let rest = rest.trim_start_matches(['\n', ' ', '\t']);
    if head.is_empty() && rest.is_empty() {
        return String::new();
    }
    if rest.is_empty() {
        format!("{head}\n")
    } else {
        format!("{head}\n\n{rest}\n")
    }
}

pub fn split_message(text: &str) -> (String, String) {
    let text = text.replace("\r\n", "\n");
    let mut lines = text.trim_end().lines();
    let summary = lines.next().unwrap_or("").trim().to_string();
    let rest: Vec<&str> = lines.skip_while(|l| l.trim().is_empty()).collect();
    (summary, rest.join("\n"))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitOpts {
    pub summary: String,
    pub body: String,
    pub amend: bool,
    pub expect_head: Option<String>,
    pub no_verify: bool,
    pub all: bool,
}

pub fn unmerged(root: &str) -> Vec<String> {
    read(root, &["-c", "core.quotepath=off", "diff", "--name-only", "--diff-filter=U", "-z"])
        .stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn commit(root: &str, o: &CommitOpts) -> Done {
    let message = compose_message(&o.summary, &o.body);
    if !message.is_empty() && o.summary.trim().is_empty() {
        return refusal(Kind::Empty, "Write a summary first", "The first line says what changed.");
    }
    let merging = run(root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).ok();
    if message.is_empty() && !o.amend && !merging {
        return refusal(Kind::Empty, "Write a summary first", "The first line says what changed.");
    }
    if o.amend
        && let Some(want) = &o.expect_head {
            let now = run(root, &["rev-parse", "-q", "--verify", "HEAD"]);
            if now.stdout.trim() != want {
                return refusal(
                    Kind::HeadChanged,
                    "The last commit changed",
                    "Amend was not done. Tick Amend again to load the new last commit.",
                );
            }
        }
    if o.all && !unmerged(root).is_empty() {
        return refusal(
            Kind::Conflict,
            "Fix the conflicted files first",
            "Commit all would save the conflict markers. Pick a side or mark each file resolved, then commit.",
        );
    }
    if o.all {
        let added = run(root, &["add", "-A"]);
        if !added.ok() {
            return failure(&added.said());
        }
    }
    let hooks = !o.no_verify && has_hooks(root, &COMMIT_HOOKS);
    let mut args = vec!["commit"];
    if o.amend {
        args.push("--amend");
    }
    if o.no_verify {
        args.push("--no-verify");
    }
    let out = if message.is_empty() {
        args.push("--no-edit");
        run(root, &args)
    } else {
        args.extend(["-F", "-"]);
        run_in(root, &args, &message)
    };
    if !out.ok() {
        let mut done = failure(&out.said());
        if hooks && matches!(done.kind, Kind::Other | Kind::Hook) {
            let (title, hint) = words(Kind::Hook);
            done.kind = Kind::Hook;
            done.title = title.into();
            done.hint = hint.into();
        }
        return done;
    }
    success(if o.amend { "Last commit updated" } else { "Committed" })
}

pub fn head_message(root: &str) -> Option<(String, String, String)> {
    let out = run(root, &["log", "-1", "--format=%H%n%B", "HEAD"]);
    if !out.ok() {
        return None;
    }
    let (sha, rest) = out.stdout.split_once('\n').unwrap_or((out.stdout.as_str(), ""));
    let (summary, body) = split_message(rest);
    Some((sha.trim().to_string(), summary, body))
}

pub fn run_all(root: &str, commands: &[Vec<String>]) -> Result<(), String> {
    for cmd in commands {
        let mut args = vec!["--literal-pathspecs"];
        args.extend(owned(cmd));
        let out = run(root, &args);
        if !out.ok() {
            let said = out.said();
            let lines: Vec<&str> = said.lines().collect();
            let from = lines.len().saturating_sub(3);
            return Err(lines[from..].join(" "));
        }
    }
    Ok(())
}

pub fn stage_commands(paths: &[String]) -> Vec<Vec<String>> {
    let mut cmd = vec!["add".to_string(), "-A".into(), "--".into()];
    cmd.extend(paths.iter().cloned());
    vec![cmd]
}

pub fn unstage_commands(paths: &[String], has_head: bool) -> Vec<Vec<String>> {
    let mut cmd: Vec<String> = if has_head {
        vec!["reset".into(), "-q".into(), "--".into()]
    } else {
        vec!["rm".into(), "--cached".into(), "-q".into(), "-r".into(), "--".into()]
    };
    cmd.extend(paths.iter().cloned());
    vec![cmd]
}

pub fn discard_commands(files: &[FileEntry], paths: &[String], has_head: bool) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for path in paths {
        let Some(f) = files.iter().find(|f| &f.path == path) else { continue };
        let p = path.clone();
        if f.conflict && has_head {
            out.push(vec!["restore".into(), "--source=HEAD".into(), "--staged".into(), "--worktree".into(), "--".into(), p]);
        } else if f.staged || f.conflict {
            out.extend(unstage_commands(std::slice::from_ref(&p), has_head));
            if f.x == 'A' || f.fresh {
                out.push(vec!["clean".into(), "-fd".into(), "--".into(), p]);
            } else {
                out.push(vec!["checkout".into(), "--".into(), p]);
            }
        } else if f.fresh {
            out.push(vec!["clean".into(), "-fd".into(), "--".into(), p]);
        } else {
            out.push(vec!["checkout".into(), "--".into(), p]);
        }
    }
    out
}

pub fn discard_all_commands(files: &[FileEntry], size: usize, has_head: bool) -> Vec<Vec<String>> {
    let mut resets = Vec::new();
    let mut cleans = Vec::new();
    let mut checkouts = Vec::new();
    for f in files {
        let indexed = f.staged || f.conflict;
        if indexed {
            resets.push(f.path.clone());
        }
        if f.fresh || (indexed && f.x == 'A') {
            cleans.push(f.path.clone());
        } else {
            checkouts.push(f.path.clone());
        }
    }
    let chunks = |verb: Vec<String>, paths: &[String]| -> Vec<Vec<String>> {
        paths
            .chunks(size.max(1))
            .map(|part| {
                let mut c = verb.clone();
                c.push("--".into());
                c.extend(part.iter().cloned());
                c
            })
            .collect()
    };
    let reset_verb: Vec<String> = if has_head {
        vec!["reset".into(), "-q".into()]
    } else {
        vec!["rm".into(), "--cached".into(), "-q".into(), "-r".into()]
    };
    let mut out = chunks(reset_verb, &resets);
    out.extend(chunks(vec!["clean".into(), "-fd".into()], &cleans));
    out.extend(chunks(vec!["checkout".into()], &checkouts));
    out
}

pub fn changed_paths(root: &str) -> Vec<String> {
    let out = read(root, &["status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all"]);
    if !out.ok() {
        return Vec::new();
    }
    out.stdout
        .split('\0')
        .filter(|e| e.len() > 3 && !e.starts_with("## "))
        .map(|e| e[3..].to_string())
        .collect()
}

pub fn clean_path(path: &str) -> Result<String, String> {
    let p = path.trim_start_matches('/');
    if p.is_empty() || p.contains('\0') || p.split('/').any(|s| s == "..") {
        return Err("That file path is not valid".into());
    }
    Ok(p.to_string())
}

pub fn has_markers(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim_end_matches('\r');
        (l.starts_with("<<<<<<<") || l.starts_with(">>>>>>>"))
            && (l.len() == 7 || l.as_bytes().get(7) == Some(&b' '))
    })
}

pub fn mark_resolved(root: &str, path: &str, refuse_markers: bool) -> Done {
    let p = match clean_path(path) {
        Ok(p) => p,
        Err(e) => return refusal(Kind::Invalid, e, ""),
    };
    if refuse_markers
        && let Ok(text) = std::fs::read_to_string(Path::new(root).join(&p))
            && has_markers(&text) {
                return refusal(
                    Kind::Markers,
                    format!("{p} still has conflict markers"),
                    "Fix the <<<<<<< and >>>>>>> lines first, or open the resolver.",
                );
            }
    let out = run(root, &["--literal-pathspecs", "add", "-A", "--", &p]);
    if !out.ok() {
        return failure(&out.said());
    }
    success(format!("Marked {p} as resolved"))
}

pub fn resolve_file(root: &str, path: &str, ours: bool) -> Done {
    let p = match clean_path(path) {
        Ok(p) => p,
        Err(e) => return refusal(Kind::Invalid, e, ""),
    };
    let side = if ours { "--ours" } else { "--theirs" };
    let out = run(root, &["--literal-pathspecs", "checkout", side, "--", &p]);
    if !out.ok() {
        let said = out.said();
        if !said.contains("does not have our version") && !said.contains("does not have their version") {
            return failure(&said);
        }
        let gone = run(root, &["--literal-pathspecs", "rm", "--quiet", "--", &p]);
        if !gone.ok() {
            return failure(&gone.said());
        }
        return success(format!("Removed {p}, as that side deleted it"));
    }
    let added = mark_resolved(root, &p, false);
    if !added.ok {
        return added;
    }
    success(if ours {
        format!("Kept the current side of {p}")
    } else {
        format!("Took the incoming side of {p}")
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Continue,
    Abort,
    Skip,
}

pub fn sequence(root: &str, op: Op, verb: Verb) -> Done {
    let word = match verb {
        Verb::Continue => "--continue",
        Verb::Abort => "--abort",
        Verb::Skip => "--skip",
    };
    if verb == Verb::Skip && !op.can_skip() {
        let mut d = refusal(Kind::Other, "A merge cannot skip", "Continue or cancel it instead.");
        d.state = read_state(root);
        return d;
    }
    let mut out = run(root, &[op.noun(), word]);
    if verb == Verb::Continue
        && op == Op::Merge
        && !out.ok()
        && (out.said().contains("unknown option") || out.said().contains("usage: git merge"))
    {
        out = run(root, &["commit", "--no-edit"]);
    }
    let state = read_state(root);
    let mut done = sequence_result(op, verb, &out, state.as_ref());
    done.state = state;
    done
}

fn step_title(state: Option<&RepoState>) -> String {
    match state.and_then(|s| s.step) {
        Some((n, total)) => format!("Step {n} of {total} has conflicts"),
        None => "There are conflicts".into(),
    }
}

pub fn sequence_result(op: Op, verb: Verb, out: &Out, state: Option<&RepoState>) -> Done {
    let text = out.said();
    let word = op.word();
    let busy = state.and_then(|s| s.op).is_some();
    let conflicts = state.map(|s| !s.conflicts.is_empty()).unwrap_or(false);
    if out.ok() {
        if busy && conflicts {
            return refusal(Kind::Conflict, step_title(state), "Resolve the files, then continue.");
        }
        if busy {
            let mut d = success(format!("{word} paused"));
            d.hint = "Check the files, then continue.".into();
            return d;
        }
        return success(match verb {
            Verb::Continue => format!("{word} finished"),
            Verb::Abort => format!("{word} cancelled. Your branch is back as it was."),
            Verb::Skip => format!("Skipped. {word} finished"),
        });
    }
    let low = text.to_lowercase();
    if (low.contains("no changes - did you forget to use 'git add'")
        || low.contains("nothing to commit")
        || low.contains("the previous cherry-pick is now empty"))
        && !conflicts
    {
        let mut d = refusal(
            Kind::Empty,
            "This step has no changes left",
            "Skip this commit, or stage your fix first.",
        );
        d.detail = detail_of(&text);
        return d;
    }
    if conflicts {
        let mut d = failure(&text);
        d.kind = Kind::Conflict;
        d.title = step_title(state);
        d.hint = words(Kind::Conflict).1.into();
        return d;
    }
    failure(&text)
}

pub fn current_branch(root: &str) -> Option<String> {
    let out = run(root, &["symbolic-ref", "-q", "--short", "HEAD"]);
    out.ok().then(|| out.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn upstream_of(root: &str) -> Option<String> {
    let out = run(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]);
    out.ok().then(|| out.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn remote_names(root: &str) -> Vec<String> {
    run(root, &["remote"])
        .stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn config_value(root: &str, key: &str) -> Option<String> {
    let out = run(root, &["config", "--get", key]);
    out.ok().then(|| out.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn pick_remote(root: &str, branch: Option<&str>, push: bool) -> Option<String> {
    let names = remote_names(root);
    if names.is_empty() {
        return None;
    }
    let mut keys = Vec::new();
    if let Some(b) = branch {
        if push {
            keys.push(format!("branch.{b}.pushRemote"));
            keys.push("remote.pushDefault".to_string());
        }
        keys.push(format!("branch.{b}.remote"));
    } else if push {
        keys.push("remote.pushDefault".to_string());
    }
    for key in keys {
        if let Some(v) = config_value(root, &key)
            && names.contains(&v) {
                return Some(v);
            }
    }
    if names.iter().any(|n| n == "origin") {
        Some("origin".into())
    } else {
        names.first().cloned()
    }
}

pub fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("Type a name.");
    }
    if name.starts_with('-') || name.starts_with('/') || name.ends_with('/') || name.ends_with(".lock") {
        return Some("The name cannot start with - or /, or end with / or .lock.");
    }
    if name.contains("..")
        || name.contains("@{")
        || name.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
    {
        return Some("The name has a character git does not allow.");
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushRef {
    pub flag: char,
    pub to: String,
    pub summary: String,
}

pub fn parse_push(text: &str) -> Vec<PushRef> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let flag = parts.next()?;
            let refs = parts.next()?;
            let summary = parts.next()?;
            let flag = flag.chars().next().filter(|_| flag.chars().count() == 1)?;
            if !" +-*!=".contains(flag) {
                return None;
            }
            let to = refs.split_once(':').map(|(_, t)| t).unwrap_or(refs);
            Some(PushRef {
                flag,
                to: to.to_string(),
                summary: summary.to_string(),
            })
        })
        .collect()
}

fn short_ref(r: &str) -> String {
    for p in ["refs/heads/", "refs/tags/", "refs/remotes/"] {
        if let Some(rest) = r.strip_prefix(p) {
            return rest.to_string();
        }
    }
    r.to_string()
}

pub fn tag_names(refs: &[PushRef]) -> Vec<String> {
    refs.iter()
        .filter(|r| r.to.starts_with("refs/tags/") && r.flag != '=' && r.flag != '!')
        .map(|r| short_ref(&r.to))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PushOpts {
    pub force_lease: bool,
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub tags: bool,
}

pub fn push(root: &str, o: &PushOpts, cancel: &AtomicBool) -> Done {
    let Some(branch) = current_branch(root) else {
        return refusal(Kind::Detached, "Not on a branch", "Check out a branch, then push.");
    };
    if let Some(want) = &o.branch
        && want != &branch {
            let mut d = refusal(
                Kind::BranchChanged,
                format!("You are on {branch} now, not {want}"),
                format!("Nothing was pushed. Switch back to {want}, then push again."),
            );
            d.branch = Some(branch);
            return d;
        }
    if o.force_lease && o.branch.is_none() {
        return refusal(
            Kind::BranchChanged,
            "Pick the branch to force push",
            "Nothing was pushed. Push again from the Changes panel.",
        );
    }
    if let Some(problem) = o.remote.as_deref().and_then(name_problem) {
        return refusal(Kind::Invalid, "That remote name is not valid", problem);
    }
    let upstream = upstream_of(root);
    let remote = o
        .remote
        .clone()
        .or_else(|| upstream.is_none().then(|| pick_remote(root, Some(&branch), true)).flatten());
    if upstream.is_none() && remote.is_none() {
        return refusal(Kind::NoRemote, "This repo has no remote", "Add a remote in the repo settings first.");
    }
    let publish = upstream.is_none();
    let mut args: Vec<String> = vec!["push".into(), "--porcelain".into()];
    if o.tags {
        args.push("--follow-tags".into());
    }
    if o.force_lease {
        args.push("--force-with-lease".into());
        args.push("--force-if-includes".into());
    }
    if let Some(r) = &remote {
        if publish {
            args.push("-u".into());
        }
        args.push("--end-of-options".into());
        args.push(r.clone());
        args.push(if o.force_lease { format!("refs/heads/{branch}") } else { "HEAD".into() });
    }
    let out = job(root, &owned(&args), cancel, JOB_LIMIT);
    if let Some(d) = stopped(&out, "Push") {
        return d;
    }
    let refs = parse_push(&out.stdout);
    let where_to = remote
        .clone()
        .or_else(|| upstream.as_deref().and_then(|u| u.split('/').next()).map(str::to_string));
    let bad: Vec<&PushRef> = refs.iter().filter(|r| r.flag == '!').collect();
    if !bad.is_empty() || (!out.ok() && classify(&out.said()) == Kind::Rejected) {
        let mut d = failure(&out.said());
        d.kind = Kind::Rejected;
        d.stale = bad.iter().any(|r| r.summary.contains("stale info")) || out.said().contains("stale info");
        d.branch = Some(branch);
        d.remote = where_to;
        return d;
    }
    if !out.ok() {
        let mut d = failure(&out.said());
        if d.kind == Kind::Other && has_hooks(root, &["pre-push"]) {
            d.kind = Kind::Hook;
            d.title = words(Kind::Hook).0.into();
            d.hint = "Fix what the pre-push hook says, then push again.".into();
        }
        d.branch = Some(branch);
        d.remote = where_to;
        return d;
    }
    let up_to_date = !refs.is_empty() && refs.iter().all(|r| r.flag == '=');
    let elsewhere = match (&o.remote, &upstream) {
        (Some(r), Some(u)) => !u.starts_with(&format!("{r}/")),
        _ => false,
    };
    let place = where_to.clone().unwrap_or_default();
    let title = if up_to_date {
        "Already up to date".to_string()
    } else if publish {
        format!("Published {branch} to {place}")
    } else if o.force_lease {
        format!("Force pushed {branch}")
    } else if elsewhere {
        format!("Pushed {branch} to {place}")
    } else {
        format!("Pushed {branch}")
    };
    let mut d = success(title);
    d.published = publish;
    d.up_to_date = up_to_date;
    d.branch = Some(branch);
    d.remote = where_to;
    if o.tags {
        d.tags = tag_names(&refs);
    }
    d
}

pub fn push_tags(root: &str, remote: Option<&str>, cancel: &AtomicBool) -> Done {
    if let Some(problem) = remote.and_then(name_problem) {
        return refusal(Kind::Invalid, "That remote name is not valid", problem);
    }
    let remote = match remote {
        Some(r) => r.to_string(),
        None => match pick_remote(root, current_branch(root).as_deref(), true) {
            Some(r) => r,
            None => {
                return refusal(Kind::NoRemote, "This repo has no remote", "Add a remote in the repo settings first.");
            }
        },
    };
    let local = run(root, &["for-each-ref", "--count=1", "--format=%(refname)", "refs/tags"]);
    if local.ok() && local.stdout.trim().is_empty() {
        let mut d = refusal(Kind::Nothing, "This repo has no tags", "Make a tag first, then push it.");
        d.remote = Some(remote);
        return d;
    }
    let out = job(root, &["push", "--porcelain", "--tags", "--end-of-options", &remote], cancel, JOB_LIMIT);
    if let Some(d) = stopped(&out, "Push") {
        return d;
    }
    let refs: Vec<PushRef> = parse_push(&out.stdout)
        .into_iter()
        .filter(|r| r.to.starts_with("refs/tags/"))
        .collect();
    let pushed = tag_names(&refs);
    let refused: Vec<&PushRef> = refs.iter().filter(|r| r.flag == '!').collect();
    if !refused.is_empty() {
        let title = if refused.len() == 1 {
            format!("{remote} refused the tag {}", short_ref(&refused[0].to))
        } else {
            format!("{remote} refused {} tags", refused.len())
        };
        let mut hint = format!("{remote} already has a tag with the same name on another commit. Those tags were not changed.");
        if !pushed.is_empty() {
            hint.push_str(&format!(" {} pushed.", plural(pushed.len(), "other tag was", "other tags were")));
        }
        let mut d = refusal(Kind::Rejected, title, hint);
        d.detail = refused
            .iter()
            .take(6)
            .map(|r| format!("{}: {}", short_ref(&r.to), r.summary))
            .collect::<Vec<_>>()
            .join("\n");
        d.tags = pushed;
        d.remote = Some(remote);
        return d;
    }
    if !out.ok() {
        let mut d = failure(&out.said());
        d.remote = Some(remote);
        return d;
    }
    let title = if pushed.is_empty() {
        format!("All tags are already on {remote}")
    } else {
        format!("Pushed {} to {remote}", plural(pushed.len(), "tag", "tags"))
    };
    let mut d = success(title);
    d.up_to_date = pushed.is_empty();
    d.tags = pushed;
    d.remote = Some(remote);
    d
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PullMode {
    #[default]
    Rebase,
    Merge,
    FfOnly,
}

impl PullMode {
    pub fn args(self) -> &'static [&'static str] {
        match self {
            PullMode::Rebase => &["--rebase", "--autostash"],
            PullMode::Merge => &["--no-rebase"],
            PullMode::FfOnly => &["--ff-only"],
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            PullMode::Rebase => "rebase",
            PullMode::Merge => "merge",
            PullMode::FfOnly => "fast-forward only",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            PullMode::Rebase => "Pull with rebase",
            PullMode::Merge => "Pull with merge",
            PullMode::FfOnly => "Pull, fast-forward only",
        }
    }
}

pub fn pull(root: &str, mode: PullMode, cancel: &AtomicBool) -> Done {
    let Some(branch) = current_branch(root) else {
        return refusal(Kind::Detached, "Not on a branch", "Check out a branch, then pull.");
    };
    let mut args: Vec<String> = vec!["pull".into()];
    args.extend(mode.args().iter().map(|s| s.to_string()));
    if upstream_of(root).is_none() {
        let Some(remote) = pick_remote(root, Some(&branch), false) else {
            return refusal(Kind::NoRemote, "This repo has no remote", "Add a remote in the repo settings first.");
        };
        let known = run(root, &["rev-parse", "-q", "--verify", &format!("refs/remotes/{remote}/{branch}")]);
        if !known.ok() {
            let (title, hint) = words(Kind::NoUpstream);
            return refusal(Kind::NoUpstream, title, hint);
        }
        args.extend(["--end-of-options".into(), remote, branch.clone()]);
    }
    let out = job(root, &owned(&args), cancel, JOB_LIMIT);
    if let Some(d) = stopped(&out, "Pull") {
        return d;
    }
    let text = out.said();
    if !out.ok() {
        if mode == PullMode::FfOnly
            && (text.contains("Not possible to fast-forward") || text.contains("Diverging branches can't be fast-forwarded"))
        {
            let mut d = refusal(
                Kind::Diverged,
                "Your branch and the remote both have new commits",
                "Pull with rebase or merge instead.",
            );
            d.detail = detail_of(&text);
            return d;
        }
        return failure(&text);
    }
    if text.contains("Applying autostash resulted in conflicts") {
        let mut d = success("Pulled, but your changes clashed");
        d.stash_conflict = true;
        d.hint = "Your own changes did not fit with the new commits. They are kept in the stash, and the clashing files are marked. Fix those files, or open the stash window.".into();
        return d;
    }
    let up = text.contains("Already up to date") || text.contains("is up to date");
    let mut d = success(if up { "Already up to date" } else { "Pulled" });
    d.up_to_date = up;
    d
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchOpts {
    pub all: bool,
    pub prune: bool,
    pub tags: bool,
    pub remote: Option<String>,
}

pub fn fetch(root: &str, o: &FetchOpts, cancel: &AtomicBool) -> Done {
    let mut args: Vec<String> = vec!["fetch".into()];
    if o.all {
        args.push("--all".into());
    }
    if o.prune {
        args.push("--prune".into());
    }
    if o.tags {
        args.push("--tags".into());
    }
    let with_tags = if o.tags { ", with tags" } else { "" };
    let label = if o.all {
        format!("Fetched all remotes{with_tags}")
    } else {
        if let Some(problem) = o.remote.as_deref().and_then(name_problem) {
            return refusal(Kind::Invalid, "That remote name is not valid", problem);
        }
        let remote = o
            .remote
            .clone()
            .or_else(|| pick_remote(root, current_branch(root).as_deref(), false));
        let Some(remote) = remote else {
            return refusal(Kind::NoRemote, "This repo has no remote", "Add a remote in the repo settings first.");
        };
        args.extend(["--end-of-options".into(), remote.clone()]);
        format!("Fetched {remote}{with_tags}")
    };
    let out = job(root, &owned(&args), cancel, JOB_LIMIT);
    if let Some(d) = stopped(&out, "Fetch") {
        return d;
    }
    if !out.ok() {
        return failure(&out.said());
    }
    success(label)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullCheck {
    pub mode: PullMode,
    pub ahead: u32,
    pub behind: u32,
    pub clean: bool,
    pub diverged: bool,
    pub conflicts: Vec<String>,
    pub local: Vec<String>,
    pub upstream: String,
    pub step: Option<(usize, usize, String, String)>,
}

fn z_list(text: &str) -> Vec<String> {
    text.split('\0')
        .map(|p| p.trim_matches('\n').to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn merge_tree(root: &str, args: &[&str]) -> Result<(bool, String, Vec<String>), String> {
    let mut all = vec!["merge-tree", "--write-tree", "--name-only", "--no-messages", "-z"];
    all.extend_from_slice(args);
    let out = run(root, &all);
    if out.code != Some(0) && out.code != Some(1) {
        return Err(detail_of(&out.said()));
    }
    let mut list = z_list(&out.stdout);
    let tree = if list.is_empty() { String::new() } else { list.remove(0) };
    list.sort();
    list.dedup();
    Ok((out.ok(), tree.trim().to_string(), list))
}

pub fn status_paths(text: &str) -> Vec<String> {
    let parts: Vec<&str> = text.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let e = parts[i];
        if e.len() >= 4 {
            let b = e.as_bytes();
            out.push(e[3..].to_string());
            if b[0] == b'R' || b[0] == b'C' || b[1] == b'R' || b[1] == b'C' {
                if let Some(next) = parts.get(i + 1).filter(|s| !s.is_empty()) {
                    out.push(next.to_string());
                }
                i += 1;
            }
        }
        i += 1;
    }
    out
}

pub fn touched(local: &[String], incoming: &[String]) -> Vec<String> {
    let mut hit: Vec<String> = Vec::new();
    for path in local {
        if path.ends_with('/') {
            hit.extend(incoming.iter().filter(|o| o.starts_with(path.as_str())).cloned());
        } else if incoming.contains(path) {
            hit.push(path.clone());
        }
    }
    hit.sort();
    hit.dedup();
    hit
}

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

type Stop = (usize, usize, String, String);

type Replayed = Option<(Vec<String>, Option<Stop>)>;

fn replay(root: &str) -> Result<Replayed, String> {
    let out = read(
        root,
        &["rev-list", "--reverse", "--no-merges", "--right-only", "--cherry-pick", "--parents", "@{u}...HEAD"],
    );
    if !out.ok() {
        return Err(detail_of(&out.said()));
    }
    let commits: Vec<Vec<String>> = out
        .stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split(' ').map(str::to_string).collect())
        .collect();
    if commits.len() > 300 {
        return Ok(None);
    }
    let mut onto = read(root, &["rev-parse", "@{u}^{tree}"]).stdout.trim().to_string();
    for (i, c) in commits.iter().enumerate() {
        let sha = &c[0];
        let parent = c.get(1).map(String::as_str).unwrap_or(EMPTY_TREE);
        let base = format!("--merge-base={parent}");
        let (clean, tree, conflicts) = merge_tree(root, &[&base, &onto, sha])?;
        if !clean {
            let info = run(root, &["log", "-1", "--format=%h%x00%s", sha]);
            let mut parts = info.stdout.trim().splitn(2, '\0');
            let short = parts.next().unwrap_or("").to_string();
            let subject = parts.next().unwrap_or("").to_string();
            return Ok(Some((conflicts, Some((i + 1, commits.len(), short, subject)))));
        }
        onto = tree;
    }
    Ok(Some((Vec::new(), None)))
}

pub fn preview_pull(root: &str, mode: PullMode) -> Done {
    if current_branch(root).is_none() {
        return refusal(Kind::Detached, "Not on a branch", "Check out a branch, then check again.");
    }
    let Some(upstream) = upstream_of(root) else {
        let (title, hint) = words(Kind::NoUpstream);
        return refusal(Kind::NoUpstream, title, hint);
    };
    let counts = read(root, &["rev-list", "--left-right", "--count", "HEAD...@{u}"]);
    if !counts.ok() {
        return failure(&counts.said());
    }
    let nums: Vec<u32> = counts.stdout.split_whitespace().map(|n| n.parse().unwrap_or(0)).collect();
    let ahead = nums.first().copied().unwrap_or(0);
    let behind = nums.get(1).copied().unwrap_or(0);
    let mut check = PullCheck {
        mode,
        ahead,
        behind,
        clean: true,
        diverged: ahead > 0 && behind > 0,
        upstream: upstream.clone(),
        ..PullCheck::default()
    };
    if behind > 0 {
        let status = read(root, &["-c", "core.quotepath=off", "status", "--porcelain=v1", "-z", "--untracked-files=normal"]);
        let incoming = read(root, &["-c", "core.quotepath=off", "diff", "--name-only", "-z", "--no-renames", "HEAD...@{u}"]);
        check.local = touched(&status_paths(&status.stdout), &z_list(&incoming.stdout));
        if ahead > 0 {
            let replayed = if mode == PullMode::Rebase { replay(root) } else { Ok(None) };
            match replayed {
                Ok(Some((conflicts, step))) => {
                    check.conflicts = conflicts;
                    check.step = step;
                }
                Ok(None) => match merge_tree(root, &["HEAD", "@{u}"]) {
                    Ok((_, _, conflicts)) => check.conflicts = conflicts,
                    Err(e) => return failure(&e),
                },
                Err(e) => return failure(&e),
            }
        }
        check.clean = check.conflicts.is_empty() && !(mode == PullMode::FfOnly && check.diverged);
    }
    let (title, hint) = check_words(&check);
    let mut d = success(title);
    d.hint = hint;
    d.check = Some(check);
    d
}

pub fn check_words(c: &PullCheck) -> (String, String) {
    if c.behind == 0 {
        let hint = if c.ahead > 0 {
            format!("You have {} to push.", plural(c.ahead as usize, "commit", "commits"))
        } else {
            format!("This branch matches {}.", c.upstream)
        };
        return ("Nothing new to pull".into(), hint);
    }
    let news = plural(c.behind as usize, "new commit", "new commits");
    let mut notes = Vec::new();
    let (title, hint) = if c.mode == PullMode::FfOnly && c.diverged {
        if !c.conflicts.is_empty() {
            notes.push(format!(
                "A merge would have conflicts in {}.",
                plural(c.conflicts.len(), "file", "files")
            ));
        }
        (
            "Fast-forward only will not work".to_string(),
            format!("Your branch and {} both have new commits. Pull with rebase or merge instead.", c.upstream),
        )
    } else if !c.conflicts.is_empty() {
        (
            format!("The pull will stop on conflicts in {}", plural(c.conflicts.len(), "file", "files")),
            match &c.step {
                Some((_, _, short, subject)) => format!(
                    "It stops at your commit {short} \"{subject}\". You can fix the files in the Changes panel."
                ),
                None => "You can fix the files in the Changes panel after the pull.".into(),
            },
        )
    } else {
        let hint = if c.ahead == 0 {
            "Your branch will move forward.".to_string()
        } else if c.mode == PullMode::Merge {
            "Git will make a merge commit.".to_string()
        } else {
            format!(
                "Your {} on top of the new ones.",
                plural(c.ahead as usize, "commit goes", "commits go")
            )
        };
        (format!("{news}. No conflicts."), hint)
    };
    if !c.local.is_empty() {
        let n = plural(c.local.len(), "file", "files");
        notes.push(if c.mode == PullMode::Rebase {
            format!("Your uncommitted changes touch {n} that the pull also changes. They are put away and brought back, and may clash.")
        } else {
            format!("Your uncommitted changes touch {n} that the pull also changes. Git may stop and ask you to commit or stash them first.")
        });
    }
    let mut all = vec![hint];
    all.extend(notes);
    (title, all.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" "))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
    pub host: Option<String>,
    pub token_in_url: bool,
}

pub fn host_of(url: &str) -> (Option<String>, bool) {
    if let Some((_, rest)) = url.split_once("://") {
        let auth_end = rest.find('/').unwrap_or(rest.len());
        let auth = &rest[..auth_end];
        let (user, host) = match auth.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, auth),
        };
        let host = host.split(':').next().unwrap_or(host).to_lowercase();
        let token = user.map(|u| u.contains(':')).unwrap_or(false);
        return ((!host.is_empty()).then_some(host), token);
    }
    if let Some((left, _)) = url.split_once(':') {
        let host = left.rsplit('@').next().unwrap_or(left).to_lowercase();
        if !host.is_empty() && !host.contains('/') {
            return (Some(host), false);
        }
    }
    (None, false)
}

pub fn masked_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let auth_end = rest.find('/').unwrap_or(rest.len());
    match rest[..auth_end].rsplit_once('@') {
        Some((_, host)) => format!("{scheme}://***@{host}{}", &rest[auth_end..]),
        None => url.to_string(),
    }
}

pub fn list_remotes(root: &str) -> Vec<Remote> {
    let out = read(root, &["remote", "-v"]);
    let mut list: Vec<Remote> = Vec::new();
    for line in out.stdout.lines() {
        let Some((name, rest)) = line.split_once('\t') else { continue };
        let url = rest
            .strip_suffix(" (fetch)")
            .or_else(|| rest.strip_suffix(" (push)"))
            .unwrap_or(rest);
        if list.iter().any(|r| r.name == name) {
            continue;
        }
        let (host, token) = host_of(url);
        list.push(Remote {
            name: name.to_string(),
            url: masked_url(url),
            host,
            token_in_url: token,
        });
    }
    list
}

pub fn remote_of_upstream(upstream: Option<&str>, remotes: &[Remote]) -> Option<String> {
    let text = upstream?;
    remotes
        .iter()
        .map(|r| r.name.as_str())
        .filter(|n| text.starts_with(&format!("{n}/")))
        .max_by_key(|n| n.len())
        .map(str::to_string)
        .or_else(|| text.split_once('/').map(|(a, _)| a.to_string()))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub local_name: Option<String>,
    pub local_email: Option<String>,
    pub remotes: Vec<Remote>,
    pub default_branch: Option<String>,
}

impl Identity {
    pub fn ready(&self) -> bool {
        self.local_name.as_deref().is_some_and(|s| !s.trim().is_empty())
            && self.local_email.as_deref().is_some_and(|s| !s.trim().is_empty())
    }

    pub fn has_github(&self) -> bool {
        self.remotes
            .iter()
            .any(|r| r.host.as_deref().is_some_and(|h| h.contains("github")))
    }
}

fn local_value(root: &str, key: &str) -> Option<String> {
    let out = run(root, &["config", "--local", "--get", key]);
    out.ok().then(|| out.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn read_identity(root: &str) -> Identity {
    let remotes = list_remotes(root);
    let main = if remotes.iter().any(|r| r.name == "origin") {
        Some("origin".to_string())
    } else {
        remotes.first().map(|r| r.name.clone())
    };
    let default_branch = main.and_then(|m| {
        let out = run(root, &["symbolic-ref", "-q", "--short", &format!("refs/remotes/{m}/HEAD")]);
        out.stdout
            .trim()
            .strip_prefix(&format!("{m}/"))
            .map(str::to_string)
    });
    Identity {
        local_name: local_value(root, "user.name"),
        local_email: local_value(root, "user.email"),
        remotes,
        default_branch,
    }
}

pub fn email_problem(email: &str) -> Option<&'static str> {
    let e = email.trim();
    let Some((user, host)) = e.split_once('@') else {
        return Some("Type an email like you@company.com.");
    };
    if user.is_empty() || !host.contains('.') || e.contains(char::is_whitespace) || e.contains(['<', '>']) {
        return Some("Type an email like you@company.com.");
    }
    None
}

pub fn set_identity(root: &str, name: &str, email: &str) -> Result<(), String> {
    let name = name.trim();
    let email = email.trim();
    if name.is_empty() || name.contains(['<', '>', '\n']) {
        return Err("Type a name for commits.".into());
    }
    if let Some(p) = email_problem(email) {
        return Err(p.into());
    }
    for (k, v) in [("user.name", name), ("user.email", email)] {
        let out = run(root, &["config", "--local", k, v]);
        if !out.ok() {
            return Err(detail_of(&out.said()));
        }
    }
    Ok(())
}

pub fn exact_pattern(path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains(['\r', '\n', '\0']) {
        return Err("this name cannot go in an ignore file".into());
    }
    let body: String = path
        .trim_start_matches('/')
        .chars()
        .flat_map(|c| {
            if "\\*?[".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect();
    let kept = body.trim_end_matches(' ');
    let spaces = body.len() - kept.len();
    Ok(format!("/{kept}{}", "\\ ".repeat(spaces)))
}

pub fn exclude_locally(root: &str, path: &str) -> Done {
    let pattern = match exact_pattern(path) {
        Ok(p) => p,
        Err(e) => return refusal(Kind::Invalid, "This cannot be ignored", e),
    };
    let out = run(root, &["rev-parse", "--path-format=absolute", "--git-path", "info/exclude"]);
    if !out.ok() {
        return failure(&out.said());
    }
    let file = PathBuf::from(out.stdout.trim());
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    if text.lines().any(|l| l == pattern) {
        return success("Already ignored on this computer");
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut add = String::new();
    if !text.is_empty() && !text.ends_with('\n') {
        add.push('\n');
    }
    add.push_str(&pattern);
    add.push('\n');
    let wrote = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .and_then(|mut f| f.write_all(add.as_bytes()));
    match wrote {
        Ok(()) => {
            let mut d = success("Ignored on this computer");
            d.hint = "Only this copy of the repo changed. No shared file changed.".into();
            d
        }
        Err(e) => failure(&format!("could not write {}: {e}", file.display())),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub tracked: bool,
    pub in_head: bool,
}

pub fn file_facts(root: &str, file: &str) -> Facts {
    let index = read(root, &["ls-files", "-z", "--", file]);
    let head = run(root, &["--no-optional-locks", "cat-file", "-e", &format!("HEAD:{file}")]);
    Facts {
        tracked: index.ok() && !index.stdout.is_empty(),
        in_head: head.ok(),
    }
}

pub fn stamp(root: &str) -> Vec<Option<std::time::SystemTime>> {
    let out = run(root, &["rev-parse", "--absolute-git-dir"]);
    let dir = PathBuf::from(out.stdout.trim());
    let mtime = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    vec![
        mtime(dir.join("index")),
        mtime(dir.join("HEAD")),
        mtime(dir.join("refs/heads")),
        mtime(dir.join("FETCH_HEAD")),
        mtime(PathBuf::from(root)),
    ]
}
