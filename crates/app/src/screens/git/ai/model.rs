use nixa_git::ai::{self, AiError, AiSettings, Found, Opts, SETTINGS_KEY};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

pub const POLL: Duration = Duration::from_millis(100);
pub const STOPPED: &str = "Stopped.";
pub const NO_REPO: &str = "No repo was given.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Review,
    Explain,
    Conflict,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub kind: Option<Kind>,
    pub repo: Option<PathBuf>,
    pub sha: Option<String>,
    pub file: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

impl Request {
    pub fn review_staged(repo: impl Into<PathBuf>) -> Request {
        Request {
            kind: Some(Kind::Review),
            repo: Some(repo.into()),
            ..Request::default()
        }
    }

    pub fn review_commit(repo: impl Into<PathBuf>, sha: &str) -> Request {
        Request {
            sha: Some(sha.into()),
            ..Request::review_staged(repo)
        }
    }

    pub fn explain(repo: impl Into<PathBuf>, sha: &str) -> Request {
        Request {
            kind: Some(Kind::Explain),
            repo: Some(repo.into()),
            sha: Some(sha.into()),
            ..Request::default()
        }
    }

    pub fn conflict(repo: impl Into<PathBuf>, file: &str, sides: Option<(&str, &str)>) -> Request {
        Request {
            kind: Some(Kind::Conflict),
            repo: Some(repo.into()),
            file: Some(file.into()),
            ours: sides.map(|s| s.0.to_string()),
            theirs: sides.map(|s| s.1.to_string()),
            ..Request::default()
        }
    }
}

pub fn heading(kind: Option<Kind>) -> &'static str {
    match kind {
        Some(Kind::Review) => "AI review",
        Some(Kind::Explain) => "AI explains a commit",
        Some(Kind::Conflict) => "AI explains a conflict",
        None => "AI",
    }
}

pub fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

pub fn target_text(r: &Request) -> String {
    if r.kind == Some(Kind::Conflict) {
        return match r.file.as_deref() {
            Some(f) if !f.is_empty() => format!("Conflict in {f}"),
            _ => "Conflict".into(),
        };
    }
    match r.sha.as_deref() {
        Some(sha) if !sha.is_empty() => format!("Commit {}", short_sha(sha)),
        _ if r.kind == Some(Kind::Review) => "Staged changes".into(),
        _ => "Last commit".into(),
    }
}

pub fn parse_show(spec: &str) -> Option<Request> {
    let (head, path) = spec.split_once(':')?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let (kind, arg) = match head.split_once('@') {
        Some((k, a)) => (k, Some(a.trim()).filter(|a| !a.is_empty())),
        None => (head, None),
    };
    match (kind.trim(), arg) {
        ("review", None) => Some(Request::review_staged(path)),
        ("review", Some(sha)) => Some(Request::review_commit(path, sha)),
        ("explain", sha) => Some(Request::explain(path, sha.unwrap_or("HEAD"))),
        ("conflict", Some(file)) => Some(Request::conflict(path, file, None)),
        _ => None,
    }
}

pub fn error_text(e: &AiError) -> String {
    if e.cancelled() { STOPPED.into() } else { e.message.clone() }
}

pub fn error_hint(missing: bool) -> &'static str {
    if missing {
        "Pick another agent with Change agent."
    } else {
        "Press Retry, or pick another agent."
    }
}

pub fn waiting_text(agent: &str, stopping: bool) -> String {
    if stopping {
        return "Stopping…".into();
    }
    let who = if agent.is_empty() { "the AI" } else { agent };
    format!("Asking {who}… This can take a minute.")
}

pub fn agent_label(settings: &AiSettings) -> (String, String) {
    let label = ai::agent_by_id(&settings.agent)
        .map(|a| a.label.to_string())
        .unwrap_or_else(|| settings.agent.clone());
    (label, settings.model_of(&settings.agent).to_string())
}

pub fn chip_text(agent: &str, model: &str) -> String {
    if model.is_empty() { agent.into() } else { format!("{agent} · {model}") }
}

pub fn took_secs(ms: u128) -> u64 {
    ((ms as f64 / 1000.0).round() as u64).max(1)
}

pub fn model_text(settings: &AiSettings, id: &str) -> String {
    let model = settings.model_of(id);
    if !model.is_empty() {
        return format!("Model: {model}");
    }
    if ai::agent_by_id(id).is_some_and(|a| a.needs_model) {
        "Model: first one in ollama list".into()
    } else {
        "Model: the agent's default".into()
    }
}

pub type AskFn = Arc<dyn Fn(&Request, &AiSettings, &Opts) -> Result<String, AiError> + Send + Sync>;
pub type LoadFn = Arc<dyn Fn() -> AiSettings + Send + Sync>;
pub type DetectFn = Arc<dyn Fn() -> Vec<Found> + Send + Sync>;
pub type PickFn = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub struct Deps {
    pub ask: AskFn,
    pub load: LoadFn,
    pub detect: DetectFn,
    pub pick: PickFn,
}

pub fn ask(r: &Request, s: &AiSettings, opts: &Opts) -> Result<String, AiError> {
    let Some(repo) = r.repo.as_deref() else {
        return Err(AiError::new(ai::ErrorKind::Nothing, NO_REPO, None));
    };
    let sha = r.sha.as_deref();
    match r.kind {
        Some(Kind::Review) => match sha {
            Some(sha) => ai::review_commit(s, repo, sha, opts),
            None => ai::review_staged(s, repo, opts),
        },
        Some(Kind::Explain) => ai::explain_commit(s, repo, sha.unwrap_or("HEAD"), opts),
        Some(Kind::Conflict) => {
            let sides = r.ours.as_deref().zip(r.theirs.as_deref());
            ai::explain_conflict(s, repo, r.file.as_deref().unwrap_or(""), sides, opts)
        }
        None => Err(AiError::new(ai::ErrorKind::Nothing, "Unknown AI request: none", None)),
    }
}

pub fn load_settings() -> AiSettings {
    let value = nixa_store::open_default()
        .ok()
        .and_then(|conn| nixa_store::settings::get_setting(&conn, SETTINGS_KEY).ok().flatten());
    AiSettings::from_value(value.as_ref())
}

pub fn save_agent(id: &str) -> Result<(), String> {
    let conn = nixa_store::open_default().map_err(|e| format!("{e:#}"))?;
    let value = nixa_store::settings::get_setting(&conn, SETTINGS_KEY).map_err(|e| format!("{e:#}"))?;
    let mut settings = AiSettings::from_value(value.as_ref());
    settings.agent = id.to_string();
    nixa_store::settings::set_setting(&conn, SETTINGS_KEY, &settings.to_value()).map_err(|e| format!("{e:#}"))
}

pub fn real() -> Deps {
    Deps {
        ask: Arc::new(ask),
        load: Arc::new(load_settings),
        detect: Arc::new(|| ai::detect_agents(false)),
        pick: Arc::new(save_agent),
    }
}

pub fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx
}

pub enum Poll<T> {
    Ready(T),
    Waiting,
    Gone,
}

pub fn poll<T>(rx: &Receiver<T>) -> Poll<T> {
    match rx.try_recv() {
        Ok(v) => Poll::Ready(v),
        Err(TryRecvError::Empty) => Poll::Waiting,
        Err(TryRecvError::Disconnected) => Poll::Gone,
    }
}
