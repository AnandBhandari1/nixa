use nixa_protocol::TermInfo;
use std::collections::HashMap;

pub const SCHEDULE_LABEL: &str = "nixa.schedule";
pub const SHELLS: [&str; 7] = ["shell", "sh", "zsh", "bash", "fish", "login", "terminal"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub label: String,
    pub job: String,
    pub number: Option<u32>,
    pub space: String,
    pub agent: String,
}

pub fn program_of(info: &TermInfo) -> String {
    info.cmd
        .first()
        .map(|c| c.rsplit('/').next().unwrap_or(c).trim().to_lowercase())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "shell".into())
}

pub fn is_shell(program: &str) -> bool {
    let p = program.trim().trim_start_matches('-').to_lowercase();
    SHELLS.contains(&p.as_str())
}

fn schedule_task(info: &TermInfo) -> Option<String> {
    let value = info.labels.get(SCHEDULE_LABEL)?.trim();
    let n = value.strip_prefix("task-")?;
    (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).then(|| n.to_string())
}

pub fn job_of(info: &TermInfo, agent: &str) -> String {
    let last = info.name.rsplit('·').next().unwrap_or_default().trim().to_string();
    if last.to_lowercase().starts_with("deep brain") {
        return "Deep brain".into();
    }
    if let Some(n) = schedule_task(info) {
        return format!("Task {n}");
    }
    let low = last.to_lowercase();
    let program = program_of(info);
    if last.is_empty() || last.chars().all(|c| c.is_ascii_digit()) || low == agent.to_lowercase() || low == program || is_shell(&low) || low == info.id.as_str() {
        return String::new();
    }
    last
}

pub fn number_of(numbers: &HashMap<String, u32>, terminal: &str) -> Option<u32> {
    numbers
        .get(terminal)
        .copied()
        .or_else(|| {
            terminal
                .parse::<nixa_protocol::TermId>()
                .ok()?
                .own_number()
                .and_then(|n| u32::try_from(n).ok())
        })
        .filter(|n| *n > 0)
}

pub fn identity(info: &TermInfo, agent: &str, numbers: &HashMap<String, u32>, space: &str) -> Identity {
    let id = info.id.to_string();
    let number = number_of(numbers, &id);
    let label = match number {
        Some(n) => format!("Agent {n}"),
        None => format!("Agent {id}"),
    };
    Identity { label, job: job_of(info, agent), number, space: space.to_string(), agent: agent.to_string() }
}

pub fn announcement(who: &Identity, state: &str) -> String {
    let job = if who.job.is_empty() { String::new() } else { format!(": {}", who.job) };
    format!("{} {state}{job}", who.label)
}

pub fn spoken(who: &Identity, state: &str) -> String {
    let place = if who.space.is_empty() { String::new() } else { format!(" in {}", who.space) };
    format!("{}{place}", announcement(who, state))
}
