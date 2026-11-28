pub mod agents;
pub mod job;
pub mod prompts;
pub mod repo;
pub mod report;
pub mod settings;
pub mod text;

pub use agents::{AGENTS, Agent, CHECKS_OFF, ChecksOff, Found, agent_by_id, checks_off_of, detect_agents, forget_agents, model_placeholder};
pub use job::{Cancel, cancel};
pub use report::{Report, Tone, parse_report};
pub use settings::{AiSettings, PrefixRule, SETTINGS_KEY, Style, timeout_of};
pub use text::Message;

use std::fmt;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Missing,
    Failed,
    Timeout,
    Cancelled,
    Empty,
    Nothing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiError {
    pub kind: ErrorKind,
    pub message: String,
    pub agent: Option<String>,
}

impl AiError {
    pub fn new(kind: ErrorKind, message: impl Into<String>, agent: Option<&str>) -> AiError {
        AiError {
            kind,
            message: message.into(),
            agent: agent.map(str::to_string),
        }
    }

    pub fn cancelled(&self) -> bool {
        self.kind == ErrorKind::Cancelled
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AiError {}

#[derive(Clone, Debug, Default)]
pub struct Opts {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub timeout_sec: Option<u64>,
    pub cancel: Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub agent: String,
    pub label: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitText {
    pub summary: String,
    pub body: String,
    pub agent: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrText {
    pub title: String,
    pub body: String,
    pub agent: String,
}

fn failed(message: impl Into<String>) -> AiError {
    AiError::new(ErrorKind::Failed, message, None)
}

fn nothing(message: &str) -> AiError {
    AiError::new(ErrorKind::Nothing, message, None)
}

static WORK: AtomicUsize = AtomicUsize::new(0);

struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn work_dir() -> Result<WorkDir, AiError> {
    let n = WORK.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("nixa-git-ai.{}.{n}.{nanos}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| failed(format!("Cannot make a temp folder for the prompt: {e}")))?;
    Ok(WorkDir(dir))
}

fn out_file(dir: &Path) -> Result<PathBuf, AiError> {
    let path = dir.join("answer.txt");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| failed(format!("Cannot make a temp file for the answer: {e}")))?;
    Ok(path)
}

fn failure_with_tip(agent: &Agent, checks_off: bool, out: &job::Outcome) -> String {
    let text = failure_text(agent.label, out);
    match agents::checks_off_of(agent) {
        Some(c) if !checks_off => format!(
            "{text}. If {} stopped to ask, you can turn on \"{}\" for it in AI settings.",
            agent.label, c.what
        ),
        _ => text,
    }
}

fn failure_text(label: &str, out: &job::Outcome) -> String {
    let line = match text::first_useful_line(&out.stderr) {
        l if l.is_empty() => text::first_useful_line(&out.stdout),
        l => l,
    };
    if line.is_empty() {
        let code = out.code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into());
        format!("{label} failed (exit {code})")
    } else {
        format!("{label}: {line}")
    }
}

fn pick_model(agent: &Agent, found: &Found, model: String) -> Result<String, AiError> {
    if !model.is_empty() || !agent.needs_model {
        return Ok(model);
    }
    agents::ollama_models(&found.path).into_iter().next().ok_or_else(|| {
        AiError::new(
            ErrorKind::Failed,
            format!("{} has no model to use. Start Ollama, or get one with: ollama pull llama3.2", agent.label),
            Some(agent.id),
        )
    })
}

fn missing(agent: &Agent) -> AiError {
    AiError::new(ErrorKind::Missing, format!("{} is not installed", agent.label), Some(agent.id))
}

pub type Ask<'a> = &'a dyn Fn(&str) -> Result<Answer, AiError>;

pub fn complete(settings: &AiSettings, prompt: &str, opts: &Opts) -> Result<Answer, AiError> {
    let id = opts.agent.clone().unwrap_or_else(|| settings.agent.clone());
    let agent = agent_by_id(&id).ok_or_else(|| AiError::new(ErrorKind::Missing, format!("Unknown AI agent: {id}"), Some(&id)))?;
    let found = detect_agents(false)
        .into_iter()
        .find(|f| f.id == agent.id && f.installed)
        .ok_or_else(|| missing(agent))?;
    run_found(settings, agent, &found, prompt, opts, &agents::search_path())
}

pub fn run_found(settings: &AiSettings, agent: &Agent, found: &Found, prompt: &str, opts: &Opts, path: &str) -> Result<Answer, AiError> {
    let wanted = opts.model.clone().unwrap_or_else(|| settings.model_of(agent.id).to_string());
    let model = pick_model(agent, found, wanted.trim().to_string())?;
    let timeout = timeout_of(settings, opts.timeout_sec);
    let stop = || AiError::new(ErrorKind::Cancelled, "Stopped", Some(agent.id));
    if opts.cancel.load(Ordering::SeqCst) {
        return Err(stop());
    }
    let dir = work_dir()?;
    let out_path = if agent.output_file { Some(out_file(&dir.0)?) } else { None };
    let checks_off = settings.checks_off_for(agent.id);
    let (args, stdin) = agents::command_args(agent, &model, out_path.as_deref(), checks_off, prompt);
    let mut env: Vec<(String, String)> = agents::JOB_ENV.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.push(("PATH".into(), path.to_string()));
    let spec = job::Spec {
        program: found.path.clone().into(),
        args,
        stdin,
        env,
        cwd: Some(dir.0.clone()),
        timeout: Duration::from_secs(timeout),
    };
    let out = match job::run(&spec, &opts.cancel) {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            forget_agents();
            return Err(missing(agent));
        }
        Err(e) => return Err(AiError::new(ErrorKind::Failed, format!("{} did not start: {e}", agent.label), Some(agent.id))),
    };
    if out.cancelled {
        return Err(stop());
    }
    if out.timed_out {
        return Err(AiError::new(
            ErrorKind::Timeout,
            format!("{} took longer than {timeout} s and was stopped", agent.label),
            Some(agent.id),
        ));
    }
    if out.code == Some(127) {
        forget_agents();
        return Err(missing(agent));
    }
    if out.code != Some(0) {
        return Err(AiError::new(ErrorKind::Failed, failure_with_tip(agent, checks_off, &out), Some(agent.id)));
    }
    let from_file = out_path.and_then(|p| fs::read_to_string(p).ok()).filter(|t| !t.trim().is_empty());
    let answer = text::clean_output(from_file.as_deref().unwrap_or(&out.stdout));
    if answer.is_empty() {
        return Err(AiError::new(ErrorKind::Empty, format!("{} gave no answer", agent.label), Some(agent.id)));
    }
    Ok(Answer {
        text: answer,
        agent: agent.id.into(),
        label: agent.label.into(),
        model,
    })
}

pub fn ping(settings: &AiSettings, opts: &Opts) -> Result<Answer, AiError> {
    complete(settings, prompts::PING_PROMPT, opts)
}

fn safe_rev(sha: &str) -> Result<&str, AiError> {
    let sha = sha.trim();
    if sha.is_empty() || sha.starts_with('-') {
        return Err(failed(format!("Not a commit: {sha}")));
    }
    Ok(sha)
}

pub fn commit_message(settings: &AiSettings, root: &Path, branch: Option<&str>, opts: &Opts) -> Result<CommitText, AiError> {
    commit_message_by(settings, root, branch, &|p| complete(settings, p, opts))
}

pub fn commit_message_by(settings: &AiSettings, root: &Path, branch: Option<&str>, ask: Ask) -> Result<CommitText, AiError> {
    let diff = repo::staged_diff(root).map_err(failed)?;
    if diff.trim().is_empty() {
        return Err(nothing("Nothing is staged. Stage some changes first."));
    }
    let branch = branch.map(str::to_string).or_else(|| repo::current_branch(root));
    let style = &settings.style;
    let prefix = prompts::resolve_prefix(&style.prefix_rules, branch.as_deref());
    let answer = ask(&prompts::commit_prompt(&diff, style, prefix.as_deref()))?;
    let message = match &prefix {
        Some(p) => prompts::ensure_prefix(&answer.text, p),
        None => answer.text.clone(),
    };
    let split = text::split_message(&message);
    Ok(CommitText {
        summary: split.summary,
        body: split.body,
        agent: answer.agent,
    })
}

pub fn review_staged(settings: &AiSettings, root: &Path, opts: &Opts) -> Result<String, AiError> {
    review_staged_by(settings, root, &|p| complete(settings, p, opts))
}

pub fn review_staged_by(settings: &AiSettings, root: &Path, ask: Ask) -> Result<String, AiError> {
    let diff = repo::staged_diff(root).map_err(failed)?;
    if diff.trim().is_empty() {
        return Err(nothing("Nothing is staged to review. Stage some changes first."));
    }
    let project = repo::review_rules(root);
    let prompt = prompts::review_prompt("this staged diff", &diff, &settings.review.instructions, &project, "");
    ask(&prompt).map(|a| a.text)
}

pub fn review_commit(settings: &AiSettings, root: &Path, sha: &str, opts: &Opts) -> Result<String, AiError> {
    review_commit_by(settings, root, sha, &|p| complete(settings, p, opts))
}

pub fn review_commit_by(settings: &AiSettings, root: &Path, sha: &str, ask: Ask) -> Result<String, AiError> {
    let c = repo::commit_context(root, safe_rev(sha)?).map_err(failed)?;
    if c.patch.trim().is_empty() {
        return Err(nothing("This commit has no text changes to send."));
    }
    let project = repo::review_rules(root);
    let lines = prompts::commit_context_lines(&c.oid, &c.summary, &c.files);
    let prompt = prompts::review_prompt("this commit", &c.patch, &settings.review.instructions, &project, &lines);
    ask(&prompt).map(|a| a.text)
}

pub fn explain_commit(settings: &AiSettings, root: &Path, sha: &str, opts: &Opts) -> Result<String, AiError> {
    explain_commit_by(root, sha, &|p| complete(settings, p, opts))
}

pub fn explain_commit_by(root: &Path, sha: &str, ask: Ask) -> Result<String, AiError> {
    let c = repo::commit_context(root, safe_rev(sha)?).map_err(failed)?;
    if c.patch.trim().is_empty() {
        return Err(nothing("This commit has no text changes to send."));
    }
    let lines = prompts::commit_context_lines(&c.oid, &c.summary, &c.files);
    ask(&prompts::explain_commit_prompt(&c.patch, &lines)).map(|a| a.text)
}

pub fn explain_conflict(
    settings: &AiSettings,
    root: &Path,
    file: &str,
    given: Option<(&str, &str)>,
    opts: &Opts,
) -> Result<String, AiError> {
    explain_conflict_by(root, file, given, &|p| complete(settings, p, opts))
}

pub fn explain_conflict_by(root: &Path, file: &str, given: Option<(&str, &str)>, ask: Ask) -> Result<String, AiError> {
    let sides = match given {
        Some((ours, theirs)) => prompts::Sides {
            count: 1,
            ours: ours.into(),
            theirs: theirs.into(),
        },
        None => {
            let text = fs::read_to_string(root.join(file)).map_err(|_| nothing(&format!("Cannot read {file}")))?;
            prompts::conflict_sides(&text)
        }
    };
    if sides.count == 0 {
        return Err(nothing(&format!("{file} has no conflict markers.")));
    }
    ask(&prompts::conflict_prompt(file, &sides.ours, &sides.theirs)).map(|a| a.text)
}

pub fn pr_description(settings: &AiSettings, root: &Path, base: Option<&str>, branch: Option<&str>, opts: &Opts) -> Result<PrText, AiError> {
    pr_description_by(root, base, branch, &|p| complete(settings, p, opts))
}

pub fn pr_description_by(root: &Path, base: Option<&str>, branch: Option<&str>, ask: Ask) -> Result<PrText, AiError> {
    let context = repo::pr_context(root, base, branch);
    if context.commits_text.trim().is_empty() {
        return Err(nothing("There are no commits to describe."));
    }
    let answer = ask(&prompts::pr_prompt(&context.commits_text, &context.diff_stat))?;
    Ok(PrText {
        title: context.title,
        body: answer.text,
        agent: answer.agent,
    })
}

#[cfg(test)]
mod tests {
    use super::repo::testrepo::{self, commit, run};
    use super::*;

    fn opts() -> Opts {
        Opts::default()
    }

    #[test]
    fn nothing_staged_is_said_before_any_agent_runs() {
        let repo = testrepo::new();
        let root = repo.0.as_path();
        commit(root, "a.txt", "one\n", "first");
        let s = AiSettings { agent: "nope".into(), ..AiSettings::default() };
        let e = commit_message(&s, root, None, &opts()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Nothing);
        assert_eq!(e.message, "Nothing is staged. Stage some changes first.");
        let e = review_staged(&s, root, &opts()).unwrap_err();
        assert_eq!(e.message, "Nothing is staged to review. Stage some changes first.");
        let e = explain_conflict(&s, root, "a.txt", None, &opts()).unwrap_err();
        assert_eq!(e.message, "a.txt has no conflict markers.");
        let e = explain_conflict(&s, root, "gone.txt", None, &opts()).unwrap_err();
        assert_eq!(e.message, "Cannot read gone.txt");
        let e = explain_commit(&s, root, "--help", &opts()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Failed);
    }

    #[test]
    fn unknown_agent_is_missing() {
        let repo = testrepo::new();
        let root = repo.0.as_path();
        commit(root, "a.txt", "one\n", "first");
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        run(root, &["add", "a.txt"]);
        let s = AiSettings { agent: "nope".into(), ..AiSettings::default() };
        let e = commit_message(&s, root, None, &opts()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Missing);
        assert_eq!(e.message, "Unknown AI agent: nope");
        let e = explain_commit(&s, root, "HEAD", &opts()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Missing);
    }

    #[test]
    fn failure_text_prefers_stderr() {
        let out = job::Outcome { code: Some(1), stderr: "Error: not logged in".into(), ..Default::default() };
        assert_eq!(failure_text("Codex CLI", &out), "Codex CLI: not logged in");
        let bare = job::Outcome { code: Some(2), ..Default::default() };
        assert_eq!(failure_text("Codex CLI", &bare), "Codex CLI failed (exit 2)");
    }

    fn fake(dir: &Path, name: &str, body: &str) -> Found {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        Found { id: name.into(), label: name.into(), installed: true, path: path.display().to_string() }
    }

    fn bin_dir(tag: &str) -> testrepo::Repo {
        let dir = std::env::temp_dir().join(format!("nixa-git-ai-bin-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        testrepo::Repo(dir)
    }

    #[test]
    fn cursor_gets_trust_only_when_the_user_allows_it() {
        let bin = bin_dir("cursor");
        let found = fake(&bin.0, "cursor-agent", "for a in \"$@\"; do [ \"$a\" = --trust ] && { echo trusted; exit 0; }; done; echo 'Workspace Trust Required' >&2; exit 1");
        let agent = agent_by_id("cursor").unwrap();
        let mut s = AiSettings { agent: "cursor".into(), ..AiSettings::default() };
        let e = run_found(&s, agent, &found, "p", &opts(), "/usr/bin:/bin").unwrap_err();
        assert_eq!(e.kind, ErrorKind::Failed);
        assert_eq!(
            e.message,
            "Cursor CLI: Workspace Trust Required. If Cursor CLI stopped to ask, you can turn on \"Trust the folder without asking\" for it in AI settings."
        );
        s.checks_off.push("cursor".into());
        assert_eq!(run_found(&s, agent, &found, "p", &opts(), "/usr/bin:/bin").unwrap().text, "trusted");
    }

    #[test]
    fn runs_a_stdin_agent_and_cleans_its_answer() {
        let bin = bin_dir("stdin");
        let found = fake(&bin.0, "claude", "read -r first; printf '```\\n%s\\nmodel=%s\\n```\\n' \"$first\" \"$6\"; echo noise >&2");
        let agent = agent_by_id("claude").unwrap();
        let mut s = AiSettings::default();
        s.models.insert("claude".into(), "haiku".into());
        let a = run_found(&s, agent, &found, "hello there\nmore", &opts(), "/usr/bin:/bin").unwrap();
        assert_eq!(a.text, "hello there\nmodel=haiku");
        assert_eq!(a.model, "haiku");
        assert_eq!(a.label, "Claude Code");
    }

    #[test]
    fn reads_the_answer_file_of_codex() {
        let bin = bin_dir("codex");
        let found = fake(&bin.0, "codex", "out=''; last=''; while [ $# -gt 0 ]; do [ \"$1\" = -o ] && out=$2; last=$1; shift; done; printf 'file: %s' \"$last\" > \"$out\"; echo stdout-noise");
        let agent = agent_by_id("codex").unwrap();
        let a = run_found(&AiSettings::default(), agent, &found, "the prompt", &opts(), "/usr/bin:/bin").unwrap();
        assert_eq!(a.text, "file: the prompt");
    }

    #[test]
    fn agent_errors_are_sorted() {
        let bin = bin_dir("errors");
        let agent = agent_by_id("claude").unwrap();
        let s = AiSettings::default();
        let bad = fake(&bin.0, "bad", "cat >/dev/null; echo 'Error: quota reached' >&2; exit 3");
        let e = run_found(&s, agent, &bad, "p", &opts(), "/usr/bin:/bin").unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ErrorKind::Failed, "Claude Code: quota reached"));
        let gone = fake(&bin.0, "gone", "exit 127");
        assert_eq!(run_found(&s, agent, &gone, "p", &opts(), "/bin").unwrap_err().kind, ErrorKind::Missing);
        let quiet = fake(&bin.0, "quiet", "cat >/dev/null; printf '  \\n'");
        let e = run_found(&s, agent, &quiet, "p", &opts(), "/bin").unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ErrorKind::Empty, "Claude Code gave no answer"));
        let stopped = Opts::default();
        stopped.cancel.store(true, Ordering::SeqCst);
        let e = run_found(&s, agent, &quiet, "p", &stopped, "/bin").unwrap_err();
        assert!(e.cancelled());
        let lost = Found { path: bin.0.join("nothing-here").display().to_string(), ..quiet };
        assert_eq!(run_found(&s, agent, &lost, "p", &opts(), "/bin").unwrap_err().kind, ErrorKind::Missing);
    }

    fn answer(text: &str) -> Result<Answer, AiError> {
        Ok(Answer { text: text.into(), agent: "claude".into(), label: "Claude Code".into(), model: String::new() })
    }

    #[test]
    fn commit_message_adds_the_branch_prefix() {
        let repo = testrepo::new();
        let root = repo.0.as_path();
        commit(root, "a.txt", "one\n", "first");
        run(root, &["checkout", "-q", "-b", "feature/ABC-7-login"]);
        fs::write(root.join("a.txt"), "two\n").unwrap();
        run(root, &["add", "a.txt"]);
        let mut s = AiSettings::default();
        s.style.prefix_rules.push(PrefixRule { pattern: "feature/*".into(), prefix: "[{ticket}]".into() });
        let seen = std::cell::RefCell::new(String::new());
        let ask = |p: &str| {
            *seen.borrow_mut() = p.to_string();
            answer("feat: add login\n\nWhy.")
        };
        let m = commit_message_by(&s, root, None, &ask).unwrap();
        assert_eq!(m.summary, "[ABC-7] feat: add login");
        assert_eq!(m.body, "Why.");
        assert!(seen.borrow().contains("Start the first line with exactly \"[ABC-7] \""));
        assert!(seen.borrow().contains("+two"));
    }

    #[test]
    fn reviews_and_explains_send_the_right_text() {
        let repo = testrepo::new();
        let root = repo.0.as_path();
        commit(root, "a.txt", "one\n", "first");
        commit(root, "a.txt", "two\n", "second");
        fs::create_dir_all(root.join(".nixa")).unwrap();
        fs::write(root.join(".nixa/review.md"), "No unwrap.").unwrap();
        fs::write(root.join("a.txt"), "three\n").unwrap();
        run(root, &["add", "a.txt"]);
        let seen = std::cell::RefCell::new(Vec::new());
        let ask = |p: &str| {
            seen.borrow_mut().push(p.to_string());
            answer("**Verdict**\nLooks safe")
        };
        let s = AiSettings::default();
        assert_eq!(review_staged_by(&s, root, &ask).unwrap(), "**Verdict**\nLooks safe");
        review_commit_by(&s, root, "HEAD", &ask).unwrap();
        explain_commit_by(root, "HEAD~1", &ask).unwrap();
        explain_conflict_by(root, "x.rs", Some(("mine", "theirs")), &ask).unwrap();
        let pr = pr_description_by(root, None, None, &ask).unwrap();
        assert_eq!(pr.title, "second");
        let seen = seen.borrow();
        assert!(seen[0].contains("Review this staged diff.") && seen[0].contains("No unwrap.") && seen[0].contains("+three"));
        assert!(seen[1].contains("Review this commit.") && seen[1].contains("(\"second\")") && seen[1].contains("+two"));
        assert!(seen[2].contains("Explain this commit.") && seen[2].contains("(\"first\")"));
        assert!(seen[3].contains("CURRENT (ours):\nmine") && seen[3].contains("INCOMING (theirs):\ntheirs"));
        assert!(seen[4].contains("Write a pull request description") && seen[4].contains("second"));
    }

    #[test]
    fn work_dir_is_private_and_removed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = work_dir().unwrap();
        let path = dir.0.clone();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
        let f = out_file(&path).unwrap();
        assert_eq!(fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        drop(dir);
        assert!(!path.exists());
    }
}
