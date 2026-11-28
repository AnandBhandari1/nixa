use crate::app::{App, lock};
use crate::util::now_ms;
use nixa_protocol::Status;

pub const CONTEXT_AGENTS: usize = 8;
pub const CONTEXT_CHARS: usize = 900;
pub const CONTEXT_WAITING: usize = 3;
pub const JOB_WORDS: usize = 50;
pub const NOTE_CHARS: usize = 60;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldAgent {
    pub terminal: String,
    pub number: Option<u32>,
    pub agent: String,
    pub place: String,
    pub folder: String,
    pub state: String,
    pub note: String,
    pub since: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldJob {
    pub terminal: String,
    pub number: Option<u32>,
    pub work: String,
    pub rounds: u32,
    pub waits: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldInput {
    pub at: u64,
    pub mode: String,
    pub clock: String,
    pub agents: Vec<WorldAgent>,
    pub jobs: Vec<WorldJob>,
    pub prompts: Vec<String>,
    pub presence: String,
    pub waiting: Vec<String>,
    pub work: String,
}

fn state_word(state: &str) -> String {
    match state {
        "idle" => "idle".into(),
        "working" => "working".into(),
        "done" => "finished".into(),
        "blocked" => "waiting on a question".into(),
        "error" => "error".into(),
        "none" | "" => "unknown".into(),
        other => clean(other, 20),
    }
}

pub fn clean(text: &str, max: usize) -> String {
    let flat: String = text.chars().map(|c| if matches!(c, '\r' | '\n' | '"') { ' ' } else { c }).collect();
    flat.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

pub fn ago(ms: u64) -> String {
    if ms < 60_000 {
        return "just now".into();
    }
    if ms < 3_600_000 {
        return format!("{} min", (ms as f64 / 60_000.0).round() as u64);
    }
    let hours = (ms as f64 / 3_600_000.0 * 10.0).round() / 10.0;
    if hours.fract() == 0.0 { format!("{} h", hours as u64) } else { format!("{hours:.1} h") }
}

fn base_name(folder: &str) -> String {
    folder.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string()
}

fn agent_line(agent: &WorldAgent, at: u64) -> String {
    let place = if !clean(&agent.place, 40).is_empty() { clean(&agent.place, 40) } else { clean(&base_name(&agent.folder), 40) };
    let since = match agent.since {
        Some(since) if agent.state != "none" && !agent.state.is_empty() => format!(" {}", ago(at.saturating_sub(since))),
        _ => String::new(),
    };
    let name = clean(&agent.agent, 24);
    let number = agent.number.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
    let place = if place.is_empty() { String::new() } else { format!(" in {place}") };
    let note = clean(&agent.note, NOTE_CHARS);
    let note = if note.is_empty() { String::new() } else { format!(" ({note})") };
    format!("terminal {number} ({}) {}{place}: {}{since}{note}", agent.terminal, if name.is_empty() { "an agent".into() } else { name }, state_word(&agent.state))
}

fn job_line(job: &WorldJob) -> String {
    let mut notes = Vec::new();
    if !job.work.trim().is_empty() {
        notes.push(format!("\"{}\"", clean(&job.work, JOB_WORDS)));
    }
    if job.rounds > 0 {
        notes.push(format!("round {}", job.rounds));
    }
    if job.waits {
        notes.push("waits for the user".into());
    }
    let who = job.number.map(|n| n.to_string()).unwrap_or_else(|| job.terminal.clone());
    if notes.is_empty() { format!("terminal {who}") } else { format!("terminal {who} ({})", notes.join(", ")) }
}

pub fn describe_world(input: &WorldInput) -> String {
    let agents: Vec<String> = input.agents.iter().map(|a| agent_line(a, input.at)).collect();
    let shown: Vec<String> = agents.iter().take(CONTEXT_AGENTS).cloned().collect();
    let more = agents.len().saturating_sub(shown.len());
    let jobs = if input.jobs.is_empty() { "none".into() } else { input.jobs.iter().map(job_line).collect::<Vec<_>>().join(", ") };
    let prompts = if input.prompts.is_empty() { "none".into() } else { input.prompts.join(", ") };
    let presence = if input.presence.trim().is_empty() { String::new() } else { format!(" User: {}.", clean(&input.presence, 60)) };
    let waiting = if input.waiting.is_empty() {
        String::new()
    } else {
        let first: Vec<String> = input.waiting.iter().take(CONTEXT_WAITING).map(|w| clean(w, 120)).collect();
        let extra = input.waiting.len().saturating_sub(CONTEXT_WAITING);
        format!(" Tasks waiting for the user: {}{}.", first.join("; "), if extra > 0 { format!("; and {extra} more") } else { String::new() })
    };
    let work = if input.work.trim().is_empty() { String::new() } else { format!(" {}", clean(&input.work, 200)) };
    let list = if shown.is_empty() { "none".into() } else { shown.join("; ") };
    let more = if more > 0 { format!("; and {more} more") } else { String::new() };
    let line = format!("Context: {}, {}.{work} Agents: {list}{more}. Jobs: {jobs}. Prompts: {prompts}.{presence}", input.clock, input.mode);
    let line = if line.chars().count() > CONTEXT_CHARS { format!("{}\u{2026}", line.chars().take(CONTEXT_CHARS - 1).collect::<String>()) } else { line };
    format!("{line}{waiting}")
}

fn status_text(status: Option<Status>) -> String {
    status.map(|s| s.as_str().to_string()).unwrap_or_else(|| "none".into())
}

pub fn gather(app: &App) -> WorldInput {
    let at = now_ms();
    let numbers = lock(&app.announce.numbers).clone();
    let spaces = app.spaces();
    let deep = lock(&app.deep);
    let mut agents = Vec::new();
    let mut prompts = Vec::new();
    for seen in app.terminals() {
        let info = &seen.info;
        if deep.owns(info.id.as_str()) || !info.running {
            continue;
        }
        let terminal = info.id.to_string();
        let number = numbers.get(&terminal).copied();
        if info.status == Some(Status::Blocked) {
            prompts.push(format!("terminal {}", number.map(|n| n.to_string()).unwrap_or_else(|| terminal.clone())));
        }
        let place = info
            .labels
            .get(crate::deep::SPACE_LABEL)
            .and_then(|id| crate::spaces::find(&spaces, id))
            .or_else(|| crate::spaces::for_folder(&spaces, &info.cwd))
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let agent = info.labels.get("agent").cloned().unwrap_or_else(|| info.name.clone());
        agents.push(WorldAgent {
            terminal,
            number,
            agent,
            place,
            folder: info.cwd.clone(),
            state: status_text(info.status),
            note: info.status_note.clone().unwrap_or_default(),
            since: Some(seen.since),
        });
    }
    drop(deep);
    let jobs: Vec<WorldJob> = lock(&app.followup.jobs)
        .values()
        .map(|job| WorldJob { terminal: job.terminal.clone(), number: numbers.get(&job.terminal).copied(), work: job.request.clone(), rounds: job.rounds, waits: false })
        .collect();
    let presence = {
        let p = lock(&app.talk.presence);
        if p.quiet(at) { "quiet".to_string() } else { String::new() }
    };
    WorldInput {
        at,
        mode: app.decision.name(),
        clock: crate::prompt::clock(at),
        agents,
        jobs,
        prompts,
        presence,
        waiting: waiting_tasks(app),
        work: crate::modes::world_line(app),
    }
}

fn waiting_tasks(app: &App) -> Vec<String> {
    let Ok(conn) = nixa_store::open(&app.db_file) else { return Vec::new() };
    use nixa_store::tasks::TaskStatus;
    let filter = nixa_store::tasks::TaskFilter { status: vec![TaskStatus::Inbox, TaskStatus::Todo, TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review], ..nixa_store::tasks::TaskFilter::default() };
    nixa_store::tasks::list_tasks(&conn, &filter)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.needs_user)
        .map(|t| format!("#{} {}", t.id, t.title))
        .collect()
}

pub fn more_line(input: &WorldInput) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !input.work.trim().is_empty() {
        parts.push(clean(&input.work, 200));
    }
    if !input.jobs.is_empty() {
        parts.push(format!("Jobs: {}.", input.jobs.iter().map(job_line).collect::<Vec<_>>().join(", ")));
    }
    if !input.presence.trim().is_empty() {
        parts.push(format!("User: {}.", clean(&input.presence, 60)));
    }
    if !input.waiting.is_empty() {
        let first: Vec<String> = input.waiting.iter().take(CONTEXT_WAITING).map(|w| clean(w, 120)).collect();
        let extra = input.waiting.len().saturating_sub(CONTEXT_WAITING);
        parts.push(format!("Tasks waiting for the user: {}{}.", first.join("; "), if extra > 0 { format!("; and {extra} more") } else { String::new() }));
    }
    let line = parts.join(" ");
    if line.chars().count() > CONTEXT_CHARS { format!("{}\u{2026}", line.chars().take(CONTEXT_CHARS - 1).collect::<String>()) } else { line }
}

pub fn context(app: &App) -> String {
    describe_world(&gather(app))
}
