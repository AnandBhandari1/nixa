use crate::errors::describe;
use crate::outcome::{Outcome, failure, refusal, stopped, success};
use crate::run::{self, Job, Out, git, git_ok, read};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

pub const OPS: [&str; 4] = ["merge", "rebase", "cherry-pick", "revert"];

pub const SKIP_OPS: [&str; 3] = ["rebase", "cherry-pick", "revert"];

pub const HOOK_NAMES: [&str; 4] = ["pre-commit", "prepare-commit-msg", "commit-msg", "post-commit"];

static MARKERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(?:<{7}|>{7})(?: |\r?$)").expect("markers"));

static OPEN_MARKER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^<{7}(?: |$)").expect("open marker"));

static EMPTY_STEP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)No changes - did you forget to use 'git add'|nothing to commit|The previous cherry-pick is now empty").expect("empty step")
});

pub fn op_word(op: &str) -> &'static str {
    match op {
        "merge" => "Merge",
        "rebase" => "Rebase",
        "cherry-pick" => "Cherry-pick",
        "revert" => "Revert",
        _ => "Working",
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub n: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub op: Option<String>,
    pub step: Option<Step>,
    pub conflicts: Vec<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Identity {
    #[default]
    Unchecked,
    Ready {
        status: String,
        profile: Option<String>,
        reason: String,
        warnings: Vec<String>,
    },
    Ask {
        warnings: Vec<String>,
    },
    Failed(String),
}

pub fn identity_gate(identity: &Identity) -> Result<Value, Box<Outcome>> {
    match identity {
        Identity::Unchecked => Ok(Value::Null),
        Identity::Ready { status, profile, reason, warnings } => {
            Ok(json!({ "status": status, "profile": profile, "reason": reason, "warnings": warnings }))
        }
        Identity::Ask { warnings } => Err(Box::new(refusal(
            "identity",
            "Pick a profile for this repo first",
            "A profile sets the name and email on your commits. Then try again.",
        )
        .detail(&warnings.join("\n"))
        .with("ask", true.into()))),
        Identity::Failed(error) => Err(Box::new(refusal("identityError", "Cannot check the git identity", "Nothing was done.").detail(error))),
    }
}

pub fn lines(text: &str) -> Vec<String> {
    text.split('\n').map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

pub fn split_z(text: &str) -> Vec<String> {
    text.split('\0').filter(|p| !p.is_empty()).map(String::from).collect()
}

pub fn clean_path(path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains('\0') || path.starts_with('/') || path.split('/').any(|part| part == "..") {
        return Err(format!("not a repo file path: {path}"));
    }
    Ok(path.to_string())
}

pub fn full_path(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

pub fn branch_name_problem(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        return Some("Type a name.");
    }
    if name.chars().any(char::is_whitespace) {
        return Some("Spaces are not allowed.");
    }
    if name.starts_with('-') {
        return Some("It cannot start with a dash.");
    }
    if name == "@" {
        return Some("A lone @ is not allowed.");
    }
    if name == "HEAD" {
        return Some("HEAD is not allowed.");
    }
    if name.chars().any(|c| (c as u32) < 0x20 || c == '\x7f' || "~^:?*[\\".contains(c)) {
        return Some("These signs are not allowed: ~ ^ : ? * [ \\");
    }
    if name.contains("..") {
        return Some("Two dots in a row are not allowed.");
    }
    if name.contains("@{") {
        return Some("@{ is not allowed.");
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Some("A slash must sit between two words.");
    }
    if name.ends_with('.') {
        return Some("It cannot end with a dot.");
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.iter().any(|p| p.starts_with('.')) {
        return Some("A part cannot start with a dot.");
    }
    if parts.iter().any(|p| p.ends_with(".lock")) {
        return Some("A part cannot end with .lock.");
    }
    None
}

pub fn remote_name_problem(name: &str) -> Option<&'static str> {
    branch_name_problem(name)
}

pub fn web_url(url: &str) -> Option<String> {
    static TRANSPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9+.-]+::").expect("transport"));
    static FULL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^(ssh|git|https?|git\+ssh|ssh\+git)://(?:[^/?#]*@)?([^/:@?#]+)(?::\d*)?/([^?#]+)").expect("full url")
    });
    static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z][a-z0-9+.-]*://").expect("scheme"));
    static SCP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[^@/:]+@)?([^/:@]+):([^?#]+)").expect("scp"));
    static HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._-]+$").expect("host"));
    static ALIAS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(github\.com|gitlab\.com|bitbucket\.org)[-_].+$").expect("alias"));
    let u = url.trim();
    if u.is_empty() || TRANSPORT.is_match(u) || u.chars().any(|c| c.is_whitespace() || (c as u32) < 0x20 || c == '\x7f') {
        return None;
    }
    let mut scheme = "https";
    let (host, path) = if let Some(m) = FULL.captures(u) {
        if m[1].eq_ignore_ascii_case("http") {
            scheme = "http";
        }
        (m[2].to_string(), m[3].to_string())
    } else if !SCHEME.is_match(u) {
        let m = SCP.captures(u)?;
        if m[2].starts_with('/') {
            return None;
        }
        (m[1].to_string(), m[2].to_string())
    } else {
        return None;
    };
    if !HOST.is_match(&host) || host.starts_with('-') {
        return None;
    }
    let host = ALIAS.replace(&host, "$1").to_lowercase();
    let mut path = path.trim_start_matches('/').trim_end_matches('/').to_string();
    if path.to_lowercase().ends_with(".git") {
        path.truncate(path.len() - 4);
    }
    let path = path.trim_end_matches('/').to_string();
    if path.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}/{path}"))
}

pub fn masked_url(url: &str) -> String {
    static FULL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^([a-z][a-z0-9+.-]*://)([^/?#]*)(.*)$").expect("mask"));
    let u = url.trim();
    if let Some(at) = u.find("::")
        && u[..at].chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        && at > 0
    {
        return format!("{}:: link", &u[..at]);
    }
    let Some(m) = FULL.captures(u) else { return u.to_string() };
    let auth = &m[2];
    let host = auth.rsplit('@').next().unwrap_or("");
    let rest = m[3].split(['?', '#']).next().unwrap_or("");
    format!("{}{host}{rest}", &m[1])
}

pub fn stash_ref(reference: &str) -> Result<String, String> {
    static NAMED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^stash@\{\d+\}$").expect("stash"));
    static SHA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{7,64}$").expect("sha"));
    if NAMED.is_match(reference) || SHA.is_match(reference) {
        return Ok(reference.to_string());
    }
    if !reference.is_empty() && reference.chars().all(|c| c.is_ascii_digit()) {
        return Ok(format!("stash@{{{reference}}}"));
    }
    Err(format!("not a stash: {reference}"))
}

pub fn compose_message(summary: &str, body: &str) -> String {
    static GAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\n\s*").expect("gap"));
    static LEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\n").expect("lead"));
    let head = GAP.replace_all(summary, " ").trim().to_string();
    let rest = body.replace("\r\n", "\n");
    let rest = LEAD.replace(&rest, "").trim_end().to_string();
    if head.is_empty() && rest.is_empty() {
        return String::new();
    }
    if rest.is_empty() { format!("{head}\n") } else { format!("{head}\n\n{rest}\n") }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub summary: String,
    pub body: String,
}

pub fn split_message_text(text: &str) -> Message {
    let text = text.replace("\r\n", "\n");
    let mut all: Vec<&str> = text.trim_end().split('\n').collect();
    let summary = if all.is_empty() { String::new() } else { all.remove(0).trim().to_string() };
    while all.first().is_some_and(|l| l.trim().is_empty()) {
        all.remove(0);
    }
    Message { summary, body: all.join("\n") }
}

fn trimmed(out: &Out) -> Option<String> {
    let text = out.stdout.trim();
    if out.ok() && !text.is_empty() { Some(text.to_string()) } else { None }
}

pub fn current_branch(root: &Path) -> Option<String> {
    trimmed(&git(root, &["symbolic-ref", "-q", "--short", "HEAD"]))
}

pub fn upstream_of(root: &Path) -> Option<String> {
    trimmed(&git(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]))
}

pub fn head_sha(root: &Path) -> Option<String> {
    trimmed(&git(root, &["rev-parse", "-q", "--verify", "HEAD"]))
}

pub fn remote_names(root: &Path) -> Vec<String> {
    lines(&git(root, &["remote"]).stdout)
}

pub fn pick_remote(root: &Path, branch: Option<&str>, purpose: &str) -> Option<String> {
    let names = remote_names(root);
    if names.is_empty() {
        return None;
    }
    let out = git(root, &["config", "-z", "--get-regexp", r"^(remote\.pushdefault|branch\..*\.(pushremote|remote))$"]);
    let mut values = std::collections::HashMap::new();
    for entry in out.stdout.split('\0') {
        if let Some(cut) = entry.find('\n')
            && cut > 0
        {
            values.insert(entry[..cut].to_string(), entry[cut + 1..].to_string());
        }
    }
    let keys: Vec<Option<String>> = if purpose == "push" {
        vec![
            branch.map(|b| format!("branch.{b}.pushremote")),
            Some("remote.pushdefault".into()),
            branch.map(|b| format!("branch.{b}.remote")),
        ]
    } else {
        vec![branch.map(|b| format!("branch.{b}.remote"))]
    };
    for key in keys.into_iter().flatten() {
        if let Some(value) = values.get(&key)
            && names.contains(value)
        {
            return Some(value.clone());
        }
    }
    if names.iter().any(|n| n == "origin") { Some("origin".into()) } else { names.first().cloned() }
}

fn decide_op(has: &[bool; 6], am: bool, seq: &str) -> Option<String> {
    let op = if (has[1] || has[2]) && !am {
        Some("rebase")
    } else if has[1] || has[2] {
        None
    } else if has[0] {
        Some("merge")
    } else if has[3] {
        Some("cherry-pick")
    } else if has[4] {
        Some("revert")
    } else if has[5] {
        match seq {
            "revert" => Some("revert"),
            "pick" | "p" => Some("cherry-pick"),
            _ => None,
        }
    } else {
        None
    };
    op.map(String::from)
}

fn read_trim(path: &Path) -> String {
    fs::read_to_string(path).map(|t| t.trim().to_string()).unwrap_or_default()
}

pub fn git_path(root: &Path, name: &str) -> Option<PathBuf> {
    let out = git(root, &["rev-parse", "--git-path", name]);
    let rel = trimmed(&out)?;
    let path = PathBuf::from(&rel);
    Some(if path.is_absolute() { path } else { root.join(path) })
}

pub fn read_state(root: &Path) -> Result<State, String> {
    let names = ["MERGE_HEAD", "rebase-merge", "rebase-apply", "CHERRY_PICK_HEAD", "REVERT_HEAD", "sequencer/todo"];
    let mut args = vec!["rev-parse"];
    for name in names {
        args.push("--git-path");
        args.push(name);
    }
    let out = git_ok(root, &args)?;
    let paths: Vec<PathBuf> = out
        .lines()
        .map(|line| {
            let p = PathBuf::from(line);
            if p.is_absolute() { p } else { root.join(p) }
        })
        .collect();
    if paths.len() < 6 {
        return Err("cannot read the repo state".into());
    }
    let has = [0, 1, 2, 3, 4, 5].map(|i| paths[i].exists());
    let dir = if paths[1].is_dir() {
        Some(&paths[1])
    } else if paths[2].is_dir() {
        Some(&paths[2])
    } else {
        None
    };
    let (step_text, head_name, am) = match dir {
        Some(d) => {
            let (a, b) = if d == &paths[1] { ("msgnum", "end") } else { ("next", "last") };
            (format!("{} {}", read_trim(&d.join(a)), read_trim(&d.join(b))), read_trim(&d.join("head-name")), d.join("applying").exists())
        }
        None => (String::new(), String::new(), false),
    };
    let seq = if has[5] {
        fs::read_to_string(&paths[5])
            .ok()
            .and_then(|t| t.lines().find(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.split_whitespace().next().unwrap_or("").to_string()))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let op = decide_op(&has, am, &seq);
    let nums: Vec<u32> = step_text.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    let step = match (op.as_deref(), nums.as_slice()) {
        (Some("rebase"), [n, total]) if *total > 0 => Some(Step { n: *n, total: *total }),
        _ => None,
    };
    let from_rebase = if op.as_deref() == Some("rebase") { head_name.strip_prefix("refs/heads/").map(String::from) } else { None };
    let branch = from_rebase.or_else(|| current_branch(root));
    let head = trimmed(&git(root, &["rev-parse", "-q", "--verify", "--short", "HEAD"]));
    let conflicts = unmerged_files(root);
    Ok(State { op, step, conflicts, branch, head })
}

pub fn unmerged_files(root: &Path) -> Vec<String> {
    let out = read(root, &["-c", "core.quotepath=off", "diff", "--name-only", "--diff-filter=U", "-z"]);
    if out.ok() { split_z(&out.stdout) } else { Vec::new() }
}

pub fn find_hooks(root: &Path, names: &[&str]) -> Vec<String> {
    let Some(dir) = git_path(root, "hooks") else { return Vec::new() };
    names
        .iter()
        .filter(|name| {
            fs::metadata(dir.join(name)).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
        })
        .map(|name| name.to_string())
        .collect()
}

pub fn commit_hooks(root: &Path) -> Vec<String> {
    find_hooks(root, &HOOK_NAMES)
}

pub fn temp_text(prefix: &str, text: &str) -> Result<tempfile::NamedTempFile, String> {
    let mut file = tempfile::Builder::new().prefix(prefix).tempfile().map_err(|e| format!("cannot write the message file: {e}"))?;
    file.write_all(text.as_bytes()).map_err(|e| format!("cannot write the message file: {e}"))?;
    file.flush().map_err(|e| e.to_string())?;
    Ok(file)
}

pub fn run_maybe_job(root: &Path, args: &[&str], slow: bool, job: &Job) -> Out {
    if slow { run::job(root, args, job) } else { git_env(root, args, &job.env) }
}

fn git_env(root: &Path, args: &[&str], env: &[(String, String)]) -> Out {
    run::git_env(root, args, env)
}

#[derive(Debug, Clone, Default)]
pub struct CommitOpts {
    pub summary: String,
    pub body: String,
    pub amend: bool,
    pub no_verify: bool,
    pub all: bool,
    pub expect_head: Option<String>,
    pub job: Job,
}

fn merge_in_progress(root: &Path) -> bool {
    git(root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).ok()
}

pub fn commit(root: &Path, opts: &CommitOpts) -> Outcome {
    let message = compose_message(&opts.summary, &opts.body);
    let empty = || refusal("empty", "Write a summary first", "The first line says what changed.");
    if !message.is_empty() && opts.summary.trim().is_empty() {
        return empty();
    }
    let use_merge = message.is_empty() && !opts.amend && merge_in_progress(root);
    if message.is_empty() && !opts.amend && !use_merge {
        return empty();
    }
    if opts.amend
        && let Some(expect) = &opts.expect_head
        && head_sha(root).as_deref() != Some(expect.as_str())
    {
        return refusal("headChanged", "The last commit changed", "Amend was not done. Tick Amend again to load the new last commit.");
    }
    if opts.all && !unmerged_files(root).is_empty() {
        return refusal(
            "conflict",
            "Fix the conflicted files first",
            "Commit all would save the conflict markers. Pick a side or mark each file resolved, then commit.",
        );
    }
    if opts.all {
        let added = git(root, &["add", "-A"]);
        if !added.ok() {
            return failure(&added.said());
        }
    }
    let hooks = if opts.no_verify { Vec::new() } else { commit_hooks(root) };
    let file = if message.is_empty() {
        None
    } else {
        match temp_text("nixa-git-msg.", &message) {
            Ok(f) => Some(f),
            Err(e) => return failure(&e),
        }
    };
    let path = file.as_ref().map(|f| f.path().to_string_lossy().into_owned());
    let mut args = vec!["commit"];
    if opts.amend {
        args.push("--amend");
    }
    if opts.no_verify {
        args.push("--no-verify");
    }
    match &path {
        Some(p) => {
            args.push("-F");
            args.push(p);
        }
        None => args.push("--no-edit"),
    }
    let out = run_maybe_job(root, &args, !hooks.is_empty(), &opts.job);
    let hook_list = json!(hooks);
    if let Some(stop) = stopped(&out) {
        return stop.with("hooks", hook_list);
    }
    if !out.ok() {
        let result = failure(&out.said()).with("hooks", hook_list);
        if !hooks.is_empty() && (result.kind == "other" || result.kind == "hook") {
            let words = describe("pre-commit hook");
            return Outcome { kind: "hook".into(), title: words.title, hint: words.hint, ..result };
        }
        return result;
    }
    let sha = git(root, &["rev-parse", "--short", "HEAD"]).stdout.trim().to_string();
    success(if opts.amend { "Last commit updated" } else { "Committed" })
        .with("sha", sha.into())
        .with("hooks", hook_list)
        .stderr(&out.stderr)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadMessage {
    pub summary: String,
    pub body: String,
    pub sha: Option<String>,
}

pub fn head_message(root: &Path) -> HeadMessage {
    let out = git(root, &["log", "-1", "--format=%H%n%B", "HEAD"]);
    if !out.ok() {
        return HeadMessage::default();
    }
    let (sha, rest) = out.stdout.split_once('\n').unwrap_or((out.stdout.as_str(), ""));
    let msg = split_message_text(rest);
    let sha = sha.trim();
    HeadMessage { summary: msg.summary, body: msg.body, sha: if sha.is_empty() { None } else { Some(sha.into()) } }
}

pub fn merge_message(root: &Path) -> Option<Message> {
    let path = git_path(root, "MERGE_MSG")?;
    let text = fs::read_to_string(path).ok()?;
    let kept: Vec<&str> = text.split('\n').filter(|l| !l.starts_with('#')).collect();
    let kept = kept.join("\n");
    let kept = kept.trim();
    if kept.is_empty() { None } else { Some(split_message_text(kept)) }
}

fn state_value(state: &Option<State>) -> Value {
    serde_json::to_value(state).unwrap_or(Value::Null)
}

pub fn step_title(state: &Option<State>) -> String {
    match state.as_ref().and_then(|s| s.step) {
        Some(step) => format!("Step {} of {} has conflicts", step.n, step.total),
        None => "There are conflicts".into(),
    }
}

fn sequence_result(op: &str, verb: &str, out: &Out, state: Option<State>) -> Outcome {
    let text = out.said();
    let words = op_word(op);
    let conflicts = state.as_ref().map(|s| !s.conflicts.is_empty()).unwrap_or(false);
    let active = state.as_ref().is_some_and(|s| s.op.is_some());
    if out.ok() {
        if active && conflicts {
            return refusal("conflict", &step_title(&state), "Resolve the files, then continue.")
                .stderr(&out.stderr)
                .with("state", state_value(&state));
        }
        if active {
            return success(&format!("{words} paused"))
                .hint("Check the files, then continue.")
                .stderr(&out.stderr)
                .with("state", state_value(&state));
        }
        let done = match verb {
            "continue" => format!("{words} finished"),
            "abort" => format!("{words} cancelled. Your branch is back as it was."),
            _ => format!("Skipped. {words} finished"),
        };
        return success(&done).stderr(&out.stderr).with("state", state_value(&state));
    }
    if EMPTY_STEP.is_match(&text) && !conflicts {
        return refusal("empty", "This step has no changes left", "Skip this commit, or stage your fix first.")
            .stderr(&out.stderr)
            .detail(&describe(&text).detail)
            .with("state", state_value(&state));
    }
    if conflicts {
        let hint = describe("CONFLICT (").hint;
        let base = failure(&text).with("state", state_value(&state));
        return Outcome { kind: "conflict".into(), title: step_title(&state), hint, ..base };
    }
    failure(&text).with("state", state_value(&state))
}

fn sequence(root: &Path, op: Option<&str>, verb: &str, job: &Job) -> Outcome {
    let current = match op.filter(|o| OPS.contains(o)) {
        Some(o) => Some(o.to_string()),
        None => match read_state(root) {
            Ok(s) => s.op,
            Err(e) => return failure(&e),
        },
    };
    let Some(current) = current else {
        return refusal("idle", "Nothing is in progress", "").with("state", state_value(&read_state(root).ok()));
    };
    if verb == "skip" && !SKIP_OPS.contains(&current.as_str()) {
        return refusal(
            "other",
            &format!("A {} cannot skip", op_word(&current).to_lowercase()),
            "Continue or cancel it instead.",
        )
        .with("state", state_value(&read_state(root).ok()));
    }
    let flag = format!("--{verb}");
    let args = [current.as_str(), flag.as_str()];
    let slow = verb != "abort" && !commit_hooks(root).is_empty();
    let mut out = run_maybe_job(root, &args, slow, job);
    if let Some(stop) = stopped(&out) {
        return stop.with("state", state_value(&read_state(root).ok()));
    }
    static OLD_MERGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)unknown option|usage: git merge").expect("old merge"));
    if verb == "continue" && current == "merge" && !out.ok() && OLD_MERGE.is_match(&out.said()) {
        out = run_maybe_job(root, &["commit", "--no-edit"], slow, job);
    }
    sequence_result(&current, verb, &out, read_state(root).ok())
}

pub fn continue_op(root: &Path, op: Option<&str>, job: &Job) -> Outcome {
    sequence(root, op, "continue", job)
}

pub fn abort_op(root: &Path, op: Option<&str>, job: &Job) -> Outcome {
    sequence(root, op, "abort", job)
}

pub fn skip_op(root: &Path, op: Option<&str>, job: &Job) -> Outcome {
    sequence(root, op, "skip", job)
}

pub fn has_markers(text: &str) -> bool {
    MARKERS.is_match(text)
}

pub fn mark_resolved(root: &Path, path: &str, refuse_markers: bool) -> Outcome {
    let p = match clean_path(path) {
        Ok(p) => p,
        Err(e) => return failure(&e),
    };
    if refuse_markers
        && let Ok(text) = fs::read_to_string(full_path(root, &p))
        && has_markers(&text)
    {
        return refusal(
            "markers",
            &format!("{p} still has conflict markers"),
            "Fix the <<<<<<< and >>>>>>> lines first, or open the resolver.",
        )
        .with("path", p.into());
    }
    let out = git(root, &["--literal-pathspecs", "add", "-A", "--", &p]);
    if !out.ok() {
        return failure(&out.said()).with("path", p.into());
    }
    success(&format!("Marked {p} as resolved")).with("path", p.into())
}

pub fn resolve_file(root: &Path, path: &str, side: &str) -> Outcome {
    if side != "ours" && side != "theirs" {
        return failure("side must be ours or theirs");
    }
    let p = match clean_path(path) {
        Ok(p) => p,
        Err(e) => return failure(&e),
    };
    let flag = format!("--{side}");
    let out = git(root, &["--literal-pathspecs", "checkout", &flag, "--", &p]);
    if !out.ok() {
        static NO_SIDE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)does not have (our|their) version").expect("side"));
        if !NO_SIDE.is_match(&out.said()) {
            return failure(&out.said()).with("path", p.into());
        }
        let removed = git(root, &["--literal-pathspecs", "rm", "--quiet", "--", &p]);
        if !removed.ok() {
            return failure(&removed.said()).with("path", p.into());
        }
        return success(&format!("Removed {p}, as that side deleted it")).with("path", p.into()).with("removed", true.into());
    }
    let added = mark_resolved(root, &p, false);
    if !added.ok {
        return added;
    }
    let title = if side == "ours" { format!("Kept the current side of {p}") } else { format!("Took the incoming side of {p}") };
    success(&title).with("path", p.into()).with("side", side.into())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictFile {
    pub path: String,
    pub file: String,
    pub text: Option<String>,
    pub has_markers: bool,
    pub deleted: bool,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

pub fn read_conflict_file(root: &Path, path: &str) -> Result<ConflictFile, String> {
    let p = clean_path(path)?;
    let full = full_path(root, &p);
    let text = fs::read(&full).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    let stage = |n: u8| {
        let spec = format!(":{n}:{p}");
        let out = git(root, &["show", &spec]);
        if out.ok() { Some(out.stdout) } else { None }
    };
    Ok(ConflictFile {
        path: p.clone(),
        file: full.to_string_lossy().into_owned(),
        has_markers: text.as_deref().is_some_and(|t| OPEN_MARKER.is_match(t)),
        deleted: text.is_none(),
        text,
        base: stage(1),
        ours: stage(2),
        theirs: stage(3),
    })
}

pub fn write_resolved(root: &Path, path: &str, text: &str) -> Outcome {
    let p = match clean_path(path) {
        Ok(p) => p,
        Err(e) => return failure(&e),
    };
    if let Err(e) = fs::write(full_path(root, &p), text) {
        return failure(&format!("cannot write the file: {e}")).with("path", p.into());
    }
    mark_resolved(root, &p, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrepo;

    #[test]
    fn branch_names_follow_git_rules() {
        assert_eq!(branch_name_problem("feature/x"), None);
        assert_eq!(branch_name_problem(""), Some("Type a name."));
        assert_eq!(branch_name_problem("a b"), Some("Spaces are not allowed."));
        assert_eq!(branch_name_problem("-x"), Some("It cannot start with a dash."));
        assert_eq!(branch_name_problem("a..b"), Some("Two dots in a row are not allowed."));
        assert_eq!(branch_name_problem("a/.b"), Some("A part cannot start with a dot."));
        assert_eq!(branch_name_problem("a.lock"), Some("A part cannot end with .lock."));
        assert_eq!(branch_name_problem("a~1"), Some("These signs are not allowed: ~ ^ : ? * [ \\"));
        assert_eq!(branch_name_problem("a//b"), Some("A slash must sit between two words."));
        assert_eq!(branch_name_problem("HEAD"), Some("HEAD is not allowed."));
    }

    #[test]
    fn web_urls_from_remote_urls() {
        assert_eq!(web_url("git@github.com:me/app.git").as_deref(), Some("https://github.com/me/app"));
        assert_eq!(web_url("https://user@gitlab.com/g/p.git/").as_deref(), Some("https://gitlab.com/g/p"));
        assert_eq!(web_url("ssh://git@github.com-work:22/me/app.git").as_deref(), Some("https://github.com/me/app"));
        assert_eq!(web_url("http://host.local/x").as_deref(), Some("http://host.local/x"));
        assert_eq!(web_url("ext::sh -c x"), None);
        assert_eq!(web_url("/local/path"), None);
        assert_eq!(masked_url("https://user:secret@github.com/me/app.git?x=1"), "https://github.com/me/app.git");
        assert_eq!(masked_url("ext::sh -c evil"), "ext:: link");
    }

    #[test]
    fn messages_and_stash_refs() {
        assert_eq!(compose_message("  fix \n it ", "\n\nbody\r\nmore  \n"), "fix it\n\nbody\nmore\n");
        assert_eq!(compose_message("only", ""), "only\n");
        assert_eq!(compose_message(" ", " "), "");
        let m = split_message_text("Title\n\n\nBody line\n");
        assert_eq!(m.summary, "Title");
        assert_eq!(m.body, "Body line");
        assert_eq!(stash_ref("2").unwrap(), "stash@{2}");
        assert_eq!(stash_ref("stash@{0}").unwrap(), "stash@{0}");
        assert!(stash_ref("main").is_err());
        assert!(clean_path("../x").is_err());
        assert!(clean_path("/abs").is_err());
        assert!(clean_path("a/b.txt").is_ok());
    }

    #[test]
    fn decide_op_reads_flags() {
        assert_eq!(decide_op(&[false, true, false, false, false, false], false, ""), Some("rebase".into()));
        assert_eq!(decide_op(&[false, false, true, false, false, false], true, ""), None);
        assert_eq!(decide_op(&[true, false, false, false, false, false], false, ""), Some("merge".into()));
        assert_eq!(decide_op(&[false, false, false, false, false, true], false, "revert"), Some("revert".into()));
        assert_eq!(decide_op(&[false, false, false, false, false, true], false, "p"), Some("cherry-pick".into()));
    }

    #[test]
    fn commit_amend_and_hooks() {
        let repo = testrepo::new();
        let root = repo.root();
        let none = commit(root, &CommitOpts::default());
        assert_eq!(none.kind, "empty");
        repo.write("a.txt", "changed\n");
        let out = commit(root, &CommitOpts { summary: "second".into(), body: "why".into(), all: true, ..CommitOpts::default() });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Committed");
        let head = head_message(root);
        assert_eq!(head.summary, "second");
        assert_eq!(head.body, "why");
        let wrong = commit(
            root,
            &CommitOpts { summary: "x".into(), amend: true, expect_head: Some("0000".into()), ..CommitOpts::default() },
        );
        assert_eq!(wrong.kind, "headChanged");
        let amended = commit(
            root,
            &CommitOpts { summary: "second again".into(), amend: true, expect_head: head.sha.clone(), ..CommitOpts::default() },
        );
        assert_eq!(amended.title, "Last commit updated");
        let hook = repo.path(".git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\necho lint broke >&2\nexit 1\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(commit_hooks(root), vec!["pre-commit".to_string()]);
        repo.write("a.txt", "again\n");
        let blocked = commit(root, &CommitOpts { summary: "third".into(), all: true, ..CommitOpts::default() });
        assert_eq!(blocked.kind, "hook");
        let skipped = commit(root, &CommitOpts { summary: "third".into(), all: true, no_verify: true, ..CommitOpts::default() });
        assert!(skipped.ok, "{skipped:?}");
    }

    #[test]
    fn merge_conflict_state_and_resolve() {
        let repo = testrepo::new();
        let root = repo.root();
        repo.git(&["switch", "-q", "-c", "side"]);
        repo.commit_file("a.txt", "side\n", "side");
        repo.git(&["switch", "-q", "main"]);
        repo.commit_file("a.txt", "main\n", "main");
        let _ = git(root, &["merge", "side"]);
        let state = read_state(root).unwrap();
        assert_eq!(state.op.as_deref(), Some("merge"));
        assert_eq!(state.conflicts, vec!["a.txt".to_string()]);
        assert_eq!(state.branch.as_deref(), Some("main"));
        let file = read_conflict_file(root, "a.txt").unwrap();
        assert!(file.has_markers);
        assert_eq!(file.ours.as_deref(), Some("main\n"));
        assert_eq!(file.theirs.as_deref(), Some("side\n"));
        assert_eq!(mark_resolved(root, "a.txt", true).kind, "markers");
        let skip = skip_op(root, None, &Job::default());
        assert_eq!(skip.kind, "other");
        let r = resolve_file(root, "a.txt", "theirs");
        assert!(r.ok, "{r:?}");
        assert_eq!(repo.read("a.txt"), "side\n");
        let msg = merge_message(root).unwrap();
        assert!(msg.summary.starts_with("Merge branch 'side'"));
        let done = continue_op(root, None, &Job::default());
        assert!(done.ok, "{done:?}");
        assert_eq!(done.title, "Merge finished");
        assert_eq!(read_state(root).unwrap().op, None);
        assert_eq!(abort_op(root, None, &Job::default()).kind, "idle");
    }

    #[test]
    fn abort_merge_and_write_resolved() {
        let repo = testrepo::new();
        let root = repo.root();
        repo.git(&["switch", "-q", "-c", "side"]);
        repo.commit_file("a.txt", "side\n", "side");
        repo.git(&["switch", "-q", "main"]);
        repo.commit_file("a.txt", "main\n", "main");
        let _ = git(root, &["merge", "side"]);
        let w = write_resolved(root, "a.txt", "both\n");
        assert!(w.ok);
        assert!(unmerged_files(root).is_empty());
        let a = abort_op(root, Some("merge"), &Job::default());
        assert!(a.ok, "{a:?}");
        assert_eq!(repo.read("a.txt"), "main\n");
    }

    #[test]
    fn pick_remote_prefers_config() {
        let repo = testrepo::new();
        let root = repo.root();
        assert_eq!(pick_remote(root, Some("main"), "push"), None);
        repo.git(&["remote", "add", "up", "/tmp/none"]);
        repo.git(&["remote", "add", "origin", "/tmp/none2"]);
        assert_eq!(pick_remote(root, Some("main"), "push").as_deref(), Some("origin"));
        repo.git(&["config", "remote.pushdefault", "up"]);
        assert_eq!(pick_remote(root, Some("main"), "push").as_deref(), Some("up"));
        assert_eq!(pick_remote(root, Some("main"), "fetch").as_deref(), Some("origin"));
        assert!(identity_gate(&Identity::Ask { warnings: vec![] }).is_err());
        assert!(identity_gate(&Identity::Unchecked).unwrap().is_null());
    }
}
