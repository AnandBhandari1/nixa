use regex::Regex;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub const GIT_ENV: &[(&str, &str)] = &[
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GCM_INTERACTIVE", "never"),
    ("GIT_EDITOR", "true"),
    ("GIT_MERGE_AUTOEDIT", "no"),
    ("LC_ALL", "C"),
];

pub const SSH_BATCH: &str = "ssh -o BatchMode=yes";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub cancelled: bool,
    pub timed_out: bool,
    pub failed_to_start: bool,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == Some(0) && !self.cancelled && !self.timed_out
    }

    pub fn said(&self) -> String {
        [self.stderr.trim(), self.stdout.trim()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Debug, Default)]
pub struct Opts {
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub stdin: Option<String>,
    pub timeout: Option<Duration>,
    pub cancel: Option<Arc<AtomicBool>>,
}

pub fn cancel_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

pub fn is_cancelled(flag: &Option<Arc<AtomicBool>>) -> bool {
    flag.as_ref().is_some_and(|f| f.load(Ordering::SeqCst))
}

fn read_all(mut from: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        from.read_to_end(&mut buf).ok();
        String::from_utf8_lossy(&buf).into_owned()
    })
}

fn kill_group(pid: u32) {
    Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
}

pub fn run(program: &Path, args: &[String], opts: &Opts) -> Output {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(if opts.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(dir) = &opts.cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Output {
                stderr: format!("{}: {e}", program.display()),
                failed_to_start: true,
                ..Output::default()
            };
        }
    };
    let pid = child.id();
    if let (Some(text), Some(mut input)) = (opts.stdin.clone(), child.stdin.take()) {
        thread::spawn(move || {
            input.write_all(text.as_bytes()).ok();
        });
    }
    let out = child.stdout.take().map(read_all);
    let err = child.stderr.take().map(read_all);
    let started = Instant::now();
    let mut cancelled = false;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if is_cancelled(&opts.cancel) {
            cancelled = true;
            kill_group(pid);
            child.kill().ok();
            break child.wait().ok();
        }
        if opts.timeout.is_some_and(|t| started.elapsed() >= t) {
            timed_out = true;
            kill_group(pid);
            child.kill().ok();
            break child.wait().ok();
        }
        thread::sleep(Duration::from_millis(15));
    };
    let stdout = out.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
    Output {
        code: if cancelled || timed_out { None } else { status.and_then(|s| s.code()) },
        stdout,
        stderr,
        cancelled,
        timed_out,
        failed_to_start: false,
    }
}

pub fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

pub fn shell(script: &str, args: &[&str], opts: &Opts) -> Output {
    let mut all = vec!["-c".to_string(), script.to_string(), "sh".to_string()];
    all.extend(strings(args));
    run(Path::new("/bin/sh"), &all, opts)
}

pub fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

pub fn tail(text: &str, count: usize) -> String {
    let all = lines(text);
    let from = all.len().saturating_sub(count);
    all[from..].join("\n")
}

pub fn last_line(text: &str) -> String {
    lines(text).pop().unwrap_or_default()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Said {
    pub kind: String,
    pub title: String,
    pub hint: String,
    pub detail: String,
}

const RULES: &[(&str, &str)] = &[
    ("timeout", r"(?i)took longer than \d+ ?s and was stopped|timed out waiting"),
    ("identity", r"(?i)Please tell me who you are|empty ident name|unable to auto-detect email|user\.useConfigOnly"),
    ("noUpstream", r"(?i)has no upstream branch|no upstream configured|There is no tracking information"),
    ("rejected", r"(?i)\[rejected\]|non-fast-forward|fetch first|Updates were rejected|stale info"),
    ("auth", r"(?i)Authentication failed|could not read (Username|Password)|terminal prompts disabled|Invalid username or (password|token)|Permission to .* denied to|denied to [\w-]+|returned error: 40[13]|HTTP 40[13]|Bad credentials|Permission denied \(publickey|Write access to repository not granted|requested URL returned error: 403"),
    ("notFound", r"(?i)Repository not found|repository '.*' not found|does not appear to be a git repository|returned error: 404"),
    ("network", r"(?i)Could not resolve host|Failed to connect|Connection (timed out|refused|reset)|Operation timed out|unable to access .*: (SSL|LibreSSL|Recv failure|Send failure)|The requested URL returned error: 5\d\d"),
    ("lock", r"(?i)index\.lock|Another git process seems to be running|Unable to create '.*\.lock'"),
    ("conflict", r"(?i)CONFLICT \(|Automatic merge failed|could not apply|fix conflicts and then|Resolve all conflicts|you need to resolve your current index first|unmerged files"),
    ("dirty", r"(?i)would be overwritten by (merge|checkout)|Please commit your changes or stash them|cannot (pull|rebase) with rebase: You have unstaged changes|Your local changes to the following files"),
    ("hook", r"(?i)hook (declined|failed)|pre-commit hook|commit-msg hook|pre-push hook|husky|lint-staged"),
    ("nothing", r"(?i)nothing to commit|no changes added to commit|nothing added to commit"),
];

fn words(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "timeout" => ("Git took too long", "It was stopped. Try again, or run it in a terminal."),
        "identity" => ("No git identity for this repo", "Pick a profile for this repo."),
        "noUpstream" => ("This branch has no remote branch yet", "Push once to publish it."),
        "rejected" => ("The remote has newer commits", "Pull first, or force push with lease."),
        "auth" => ("Git sign-in failed", "Check the account and token for this repo."),
        "notFound" => ("Remote repository not found", "Check the remote URL and that this account can see the repo."),
        "network" => ("Cannot reach the remote", "Check the network and try again."),
        "lock" => ("Another git command is running", "Wait a moment, then try again. If it stays, remove .git/index.lock."),
        "conflict" => ("There are conflicts", "Resolve the conflicted files, then continue."),
        "dirty" => ("Local changes are in the way", "Commit or stash your changes first."),
        "hook" => ("A git hook stopped this", "Fix what the hook says, or run without hooks."),
        "nothing" => ("Nothing to commit", "Stage some changes first."),
        _ => ("Git failed", ""),
    }
}

pub fn strip_ansi(text: &str) -> String {
    let re = Regex::new(r"\x1b(?:\[[0-9;?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-Z\\-_])").expect("ansi pattern");
    re.replace_all(text, "").into_owned()
}

pub fn classify(text: &str) -> &'static str {
    for (kind, pattern) in RULES {
        if Regex::new(pattern).expect("git error pattern").is_match(text) {
            return kind;
        }
    }
    "other"
}

pub fn describe(text: &str) -> Said {
    let kind = classify(text);
    let prefix = Regex::new(r"(?i)^(error|fatal|remote):\s*").expect("prefix pattern");
    let hint = Regex::new(r"(?i)^\s*hint:").expect("hint pattern");
    let kept: Vec<String> = strip_ansi(text)
        .trim()
        .lines()
        .filter(|l| !hint.is_match(l))
        .map(|l| prefix.replace(l, "").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let from = kept.len().saturating_sub(4);
    let (title, hint) = words(kind);
    Said {
        kind: kind.to_string(),
        title: title.to_string(),
        hint: hint.to_string(),
        detail: kept[from..].join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_reads_output_and_code() {
        let out = shell("printf hi; printf oops >&2; exit 3", &[], &Opts::default());
        assert_eq!(out.code, Some(3));
        assert_eq!(out.stdout, "hi");
        assert_eq!(out.stderr, "oops");
        assert!(!out.ok());
    }

    #[test]
    fn run_feeds_stdin() {
        let opts = Opts {
            stdin: Some("abc\n".into()),
            ..Opts::default()
        };
        let out = shell("read x; printf %s \"$x\"", &[], &opts);
        assert_eq!(out.stdout, "abc");
        assert!(out.ok());
    }

    #[test]
    fn run_stops_on_timeout_and_cancel() {
        let opts = Opts {
            timeout: Some(Duration::from_millis(200)),
            ..Opts::default()
        };
        let started = Instant::now();
        let out = shell("sleep 5", &[], &opts);
        assert!(out.timed_out);
        assert!(started.elapsed() < Duration::from_secs(3));
        let flag = cancel_flag();
        flag.store(true, Ordering::SeqCst);
        let out = shell(
            "sleep 5",
            &[],
            &Opts {
                cancel: Some(flag),
                ..Opts::default()
            },
        );
        assert!(out.cancelled);
        assert_eq!(out.code, None);
    }

    #[test]
    fn missing_program_fails_to_start() {
        let out = run(Path::new("/no/such/program"), &[], &Opts::default());
        assert!(out.failed_to_start);
    }

    #[test]
    fn describe_names_the_kind() {
        let said = describe("remote: Invalid username or password.\nfatal: Authentication failed for 'https://github.com/a/b.git/'");
        assert_eq!(said.kind, "auth");
        assert_eq!(said.title, "Git sign-in failed");
        assert!(said.detail.starts_with("Invalid username"));
        assert_eq!(describe("fatal: repository 'x' not found").kind, "notFound");
        assert_eq!(describe("something else").kind, "other");
    }
}
