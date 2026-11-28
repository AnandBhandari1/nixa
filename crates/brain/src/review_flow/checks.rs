use regex::Regex;
use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};

pub const NPM_SCRIPTS: [&str; 4] = ["check", "typecheck", "lint", "test"];
pub const CARGO_STEPS: [&str; 3] = ["check", "clippy", "test"];
pub const CHECK_MS: u64 = 5 * 60 * 1000;
pub const CHECKS_TOTAL_MS: u64 = 10 * 60 * 1000;
pub const MAX_PACKAGES: usize = 3;
const TAIL_LINES: usize = 8;
const TAIL_CHARS: usize = 400;
const OUTPUT_CHARS: usize = 20_000;
const READ_KEEP: usize = 256 * 1024;
const COMMAND_NAME_CHARS: usize = 40;
const PIPE_WAIT_MS: u64 = 2000;
pub const SAFE_ENV: [&str; 16] = ["PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "TERM", "CI", "SSH_AUTH_SOCK", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY", "SSL_CERT_FILE", "SSL_CERT_DIR", "COLORTERM"];
pub const SHELL_CHARS: [char; 22] = [';', '&', '|', '$', '(', ')', '`', '<', '>', '\n', '\r', '\'', '"', '\\', '*', '?', '{', '}', '[', ']', '!', '#'];
pub const SHELLS: [&str; 16] = ["sh", "bash", "zsh", "dash", "ksh", "fish", "csh", "tcsh", "env", "eval", "exec", "sudo", "doas", "xargs", "osascript", "nohup"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Npm,
    Cargo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecOutcome {
    pub code: Option<i32>,
    pub output: String,
    pub timed_out: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckResult {
    pub package: String,
    pub script: String,
    pub ok: bool,
    pub ms: u64,
    pub summary: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tail: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ChecksOutcome {
    pub folder: String,
    pub results: Vec<CheckResult>,
    pub failed: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub none: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub typed: bool,
    pub dir: PathBuf,
    pub script: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    pub commands: Vec<String>,
    pub packages: Option<Vec<PathBuf>>,
    pub total_ms: Option<u64>,
    pub check_ms: Option<u64>,
}

pub fn manifest_kind(dir: &Path) -> Option<Kind> {
    if dir.join("package.json").is_file() {
        Some(Kind::Npm)
    } else if dir.join("Cargo.toml").is_file() {
        Some(Kind::Cargo)
    } else {
        None
    }
}

pub fn npm_scripts(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dir.join("package.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let scripts = value.get("scripts").and_then(|s| s.as_object());
    NPM_SCRIPTS
        .iter()
        .filter(|name| scripts.and_then(|s| s.get(**name)).and_then(|v| v.as_str()).is_some_and(|v| !v.trim().is_empty()))
        .map(|name| name.to_string())
        .collect()
}

pub fn scripts_of(dir: &Path) -> Vec<String> {
    match manifest_kind(dir) {
        Some(Kind::Npm) => npm_scripts(dir),
        Some(Kind::Cargo) => CARGO_STEPS.iter().map(|s| s.to_string()).collect(),
        None => Vec::new(),
    }
}

pub fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn inside(root: &Path, path: &Path) -> bool {
    let full = if path.is_absolute() { clean_path(path) } else { clean_path(&root.join(path)) };
    full.starts_with(clean_path(root))
}

fn child_packages(root: &Path) -> Vec<PathBuf> {
    if !root.join(".git").exists() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            !name.starts_with('.') && name != "node_modules" && name != "target"
        })
        .map(|e| e.path())
        .filter(|dir| !scripts_of(dir).is_empty())
        .collect();
    found.sort();
    found
}

pub fn safe_package(root: &Path, dir: &Path) -> bool {
    let (Ok(real_root), Ok(real_dir)) = (std::fs::canonicalize(root), std::fs::canonicalize(dir)) else {
        return false;
    };
    real_dir.starts_with(&real_root)
        && ["package.json", "Cargo.toml"].iter().map(|m| dir.join(m)).filter(|m| m.symlink_metadata().is_ok()).all(|m| std::fs::canonicalize(&m).is_ok_and(|real| real.starts_with(&real_root)))
}

pub fn packages_for(folder: &Path, changed: &[String], max: usize) -> Vec<PathBuf> {
    let root = clean_path(folder);
    let mut found: Vec<PathBuf> = Vec::new();
    for path in changed {
        let given = Path::new(path);
        let full = if given.is_absolute() { clean_path(given) } else { clean_path(&root.join(given)) };
        if full == root || !full.starts_with(&root) {
            continue;
        }
        let mut dir = full.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
        loop {
            if manifest_kind(&dir).is_some() {
                if safe_package(&root, &dir) && !found.contains(&dir) {
                    found.push(dir);
                }
                break;
            }
            if dir == root {
                break;
            }
            match dir.parent() {
                Some(up) if up.starts_with(&root) => dir = up.to_path_buf(),
                _ => break,
            }
        }
    }
    if found.is_empty() {
        if manifest_kind(&root).is_some() {
            found.push(root.clone());
        } else {
            found.extend(child_packages(&root));
        }
        found.retain(|dir| safe_package(&root, dir));
    }
    found.truncate(max);
    found
}

pub fn find_program(name: &str) -> String {
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let path_var = std::env::var("PATH").unwrap_or_default();
    crate::cli_brains::search_dirs(&path_var, &home)
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|p| crate::cli_brains::is_executable(p))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string())
}

fn cargo_args(step: &str) -> Vec<String> {
    let words: &[&str] = match step {
        "check" => &["check", "--all-targets"],
        "clippy" => &["clippy", "--all-targets", "--", "-D", "warnings"],
        _ => &["test"],
    };
    words.iter().map(|w| w.to_string()).collect()
}

fn command_name(command: &str) -> String {
    command.chars().take(COMMAND_NAME_CHARS).collect()
}

pub fn env_ok(name: &str, extra: &[&str]) -> bool {
    SAFE_ENV.contains(&name) || extra.contains(&name) || name.starts_with("LC_") || name.starts_with("CI_")
}

pub fn safe_env(extra: &[&str]) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    std::env::vars_os().filter(|(k, _)| k.to_str().is_some_and(|k| env_ok(k, extra))).collect()
}

pub fn env_set(word: &str) -> bool {
    word.split_once('=').is_some_and(|(k, _)| k.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

pub fn argv_of(command: &str) -> Result<Vec<String>, &'static str> {
    if command.contains(SHELL_CHARS) {
        return Err("it has shell characters such as ; | & $ or quotes");
    }
    let words: Vec<String> = command.split_whitespace().map(str::to_string).collect();
    let program = words.iter().find(|w| !env_set(w)).ok_or("it has no program")?;
    let base = program.rsplit('/').next().unwrap_or(program);
    if SHELLS.contains(&base) {
        return Err("it starts a shell or runs another command");
    }
    Ok(words)
}

pub fn typed_plan(folder: &Path, command: &str) -> Result<Planned, &'static str> {
    let words = argv_of(command)?;
    let at = words.iter().position(|w| !env_set(w)).ok_or("it has no program")?;
    let program = &words[at];
    let real = if program.contains('/') {
        let path = Path::new(program);
        if path.is_absolute() { path.to_path_buf() } else { folder.join(path) }.to_string_lossy().to_string()
    } else {
        find_program(program)
    };
    let (program, args) = if at == 0 { (real, words[1..].to_vec()) } else { ("/usr/bin/env".to_string(), words[..at].iter().cloned().chain(std::iter::once(real)).chain(words[at + 1..].iter().cloned()).collect()) };
    Ok(Planned { typed: true, dir: folder.to_path_buf(), name: command_name(command), program, args, script: command.to_string() })
}

pub fn plan(folder: &Path, changed: &[String], options: &RunOptions) -> Result<Vec<Planned>, String> {
    let commands: Vec<String> = options.commands.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).take(MAX_PACKAGES).collect();
    if !commands.is_empty() {
        return commands.iter().map(|command| typed_plan(folder, command).map_err(|why| format!("\"{command}\" was not run: {why}."))).collect();
    }
    let packages = options.packages.clone().unwrap_or_else(|| packages_for(folder, changed, MAX_PACKAGES));
    let mut out = Vec::new();
    let mut npm = String::new();
    let mut cargo = String::new();
    for dir in &packages {
        match manifest_kind(dir) {
            Some(Kind::Npm) => {
                for script in npm_scripts(dir) {
                    if npm.is_empty() {
                        npm = find_program("npm");
                    }
                    out.push(Planned { typed: false, dir: dir.clone(), name: script.clone(), program: npm.clone(), args: vec!["run".into(), script.clone()], script });
                }
            }
            Some(Kind::Cargo) => {
                for step in CARGO_STEPS {
                    if cargo.is_empty() {
                        cargo = find_program("cargo");
                    }
                    out.push(Planned { typed: false, dir: dir.clone(), name: format!("cargo {step}"), program: cargo.clone(), args: cargo_args(step), script: step.to_string() });
                }
            }
            None => {}
        }
    }
    if out.is_empty() {
        return Err(if packages.is_empty() {
            "no package.json or Cargo.toml found".to_string()
        } else {
            "no check, typecheck, lint or test script found".to_string()
        });
    }
    Ok(out)
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").expect("ansi pattern"));
static NODE_PASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*(?:#|ℹ)\s*pass (\d+)").expect("pattern"));
static NODE_FAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*(?:#|ℹ)\s*fail (\d+)").expect("pattern"));
static JEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Tests:\s+(?:(\d+) failed,\s*)?(?:\d+ skipped,\s*)?(?:(\d+) passed,\s*)?(\d+) total").expect("pattern")
});
static VITEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Tests\s+(?:(\d+) failed\s*\|\s*)?(?:\d+ passed)?\s*\((\d+)\)").expect("pattern"));
static CARGO_TEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"test result: \w+\. (\d+) passed; (\d+) failed").expect("pattern"));
static TS_ERROR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"error TS\d+").expect("pattern"));
static RUST_ERROR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^error(?:\[E\d+\])?: ").expect("pattern"));
static RUST_ERROR_TOTAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^error: (?:could not compile|aborting)").expect("pattern"));

pub fn strip(text: &str) -> String {
    ANSI.replace_all(text, "").to_string()
}

fn number(m: Option<regex::Match>) -> u64 {
    m.and_then(|m| m.as_str().parse().ok()).unwrap_or(0)
}

fn counted(script: &str, ok: bool, fail: u64, total: u64) -> String {
    if ok {
        format!("{script} passed ({total} of {total})")
    } else {
        format!("{script} failed ({fail} of {total})")
    }
}

pub fn summarize(script: &str, output: &str, ok: bool, timed_out: bool) -> String {
    if timed_out {
        return format!("{script} ran out of time");
    }
    let clean = strip(output);
    if let (Some(pass), Some(fail)) = (NODE_PASS.captures(&clean), NODE_FAIL.captures(&clean)) {
        let fail = number(fail.get(1));
        return counted(script, ok, fail, number(pass.get(1)) + fail);
    }
    if let Some(found) = JEST.captures(&clean) {
        return counted(script, ok, number(found.get(1)), number(found.get(3)));
    }
    if let Some(found) = VITEST.captures(&clean) {
        return counted(script, ok, number(found.get(1)), number(found.get(2)));
    }
    let mut cargo_seen = false;
    let (mut passed, mut failed) = (0, 0);
    for found in CARGO_TEST.captures_iter(&clean) {
        cargo_seen = true;
        passed += number(found.get(1));
        failed += number(found.get(2));
    }
    if cargo_seen {
        return counted(script, ok, failed, passed + failed);
    }
    if !ok {
        let ts = TS_ERROR.find_iter(&clean).count();
        if ts > 0 {
            return format!("{script} failed ({ts} type error{})", if ts == 1 { "" } else { "s" });
        }
        let rust = RUST_ERROR.find_iter(&clean).count().saturating_sub(RUST_ERROR_TOTAL.find_iter(&clean).count());
        if rust > 0 {
            return format!("{script} failed ({rust} error{})", if rust == 1 { "" } else { "s" });
        }
    }
    format!("{script} {}", if ok { "passed" } else { "failed" })
}

fn last_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    text.chars().skip(count - max).collect()
}

pub fn tail_of(output: &str) -> String {
    let clean = nixa_store::text::hide_secrets(&strip(output), &[]);
    let lines: Vec<&str> = clean.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    let kept = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
    last_chars(&kept, TAIL_CHARS)
}

async fn read_tail<R: AsyncRead + Unpin>(mut reader: R) -> Vec<u8> {
    let mut kept: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                kept.extend_from_slice(&buf[..n]);
                if kept.len() > READ_KEEP * 2 {
                    kept.drain(..kept.len() - READ_KEEP);
                }
            }
        }
    }
    if kept.len() > READ_KEEP {
        kept.drain(..kept.len() - READ_KEEP);
    }
    kept
}

fn kill_group(pid: u32) {
    let _ = std::process::Command::new("/bin/kill")
        .args(["-KILL", &format!("-{pid}")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub async fn exec(program: &str, args: &[String], cwd: &Path, ms: u64) -> ExecOutcome {
    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("exec \"$0\" \"$@\" 2>&1")
        .arg(program)
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(safe_env(&[]))
        .env("CI", "1")
        .env("FORCE_COLOR", "0")
        .env("NO_COLOR", "1")
        .env("CARGO_TERM_COLOR", "never")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return ExecOutcome { code: None, output: format!("could not start {program}: {e}"), timed_out: false },
    };
    let pid = child.id();
    let out = child.stdout.take().map(|s| tokio::spawn(read_tail(s)));
    let err = child.stderr.take().map(|s| tokio::spawn(read_tail(s)));    let waited = tokio::time::timeout(Duration::from_millis(ms), child.wait()).await;
    let (code, timed_out) = match waited {
        Ok(Ok(status)) => (Some(status.code().unwrap_or(1)), false),
        Ok(Err(_)) => (None, false),
        Err(_) => {
            if let Some(pid) = pid {
                kill_group(pid);
            }
            let _ = child.kill().await;
            (None, true)
        }
    };
    if !timed_out && let Some(pid) = pid {
        kill_group(pid);
    }
    let mut text = String::new();
    for task in [out, err].into_iter().flatten() {
        if let Ok(Ok(bytes)) = tokio::time::timeout(Duration::from_millis(PIPE_WAIT_MS), task).await {
            text.push_str(&String::from_utf8_lossy(&bytes));
            text.push('\n');
        }
    }
    ExecOutcome { code, output: last_chars(&text, OUTPUT_CHARS), timed_out }
}

fn package_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| dir.to_string_lossy().to_string())
}

pub async fn run_plan(folder: &Path, plan: Vec<Planned>, options: &RunOptions) -> ChecksOutcome {
    let total = options.total_ms.unwrap_or(CHECKS_TOTAL_MS);
    let each = options.check_ms.unwrap_or(CHECK_MS);
    let started = Instant::now();
    let mut results = Vec::new();
    for item in plan {
        let spent = started.elapsed().as_millis() as u64;
        let left = total.saturating_sub(spent);
        let package = package_name(&item.dir);
        if left == 0 {
            results.push(CheckResult { package, summary: format!("{} not run, out of time", item.name), script: item.script, ok: false, ms: 0, tail: String::new(), timed_out: true });
            continue;
        }
        let at = Instant::now();
        let run = exec(&item.program, &item.args, &item.dir, each.min(left)).await;
        let ok = run.code == Some(0) && !run.timed_out;
        results.push(CheckResult {
            package,
            summary: summarize(&item.name, &run.output, ok, run.timed_out),
            script: item.script,
            ok,
            ms: at.elapsed().as_millis() as u64,
            tail: if ok { String::new() } else { tail_of(&run.output) },
            timed_out: run.timed_out,
        });
    }
    let failed = results.iter().any(|r| !r.ok);
    ChecksOutcome { folder: folder.to_string_lossy().to_string(), results, failed, none: String::new() }
}

pub async fn run_checks(folder: &Path, changed: &[String], options: &RunOptions) -> ChecksOutcome {
    match plan(folder, changed, options) {
        Ok(planned) => run_plan(folder, planned, options).await,
        Err(none) => ChecksOutcome { folder: folder.to_string_lossy().to_string(), results: Vec::new(), failed: false, none },
    }
}

pub fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace(['"', '“', '”'], "'"))
}

pub fn short_checks(outcome: &ChecksOutcome) -> String {
    if !outcome.none.is_empty() {
        return format!("none ({})", outcome.none);
    }
    let mut packages: Vec<&str> = Vec::new();
    for r in &outcome.results {
        if !packages.contains(&r.package.as_str()) {
            packages.push(&r.package);
        }
    }
    packages
        .iter()
        .map(|name| {
            let parts: Vec<&str> = outcome.results.iter().filter(|r| r.package == *name).map(|r| r.summary.as_str()).collect();
            format!("in {name}: {}", parts.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn describe_checks(outcome: &ChecksOutcome) -> String {
    if !outcome.none.is_empty() {
        return format!("Checks run by Nixa: none ({}), so judge from the reply, the screen and the changed files.", outcome.none);
    }
    let tails: String = outcome
        .results
        .iter()
        .filter(|r| !r.ok && !r.tail.is_empty())
        .map(|r| format!(" Last lines of {} {}: {}.", r.package, r.script, quoted(&crate::util::flat(&r.tail))))
        .collect();
    let failed = if outcome.failed { " The job is not done while a check fails." } else { "" };
    format!("Checks run by Nixa {}.{tails}{failed}", short_checks(outcome))
}
