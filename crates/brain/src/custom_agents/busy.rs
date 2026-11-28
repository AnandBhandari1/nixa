use crate::app::{App, lock};
use nixa_protocol::Status;
use nixa_store::agent_busy::claim_text;
use nixa_store::agents::AgentState;
use nixa_store::tasks::{BlockedOn, TaskStatus};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

pub const JOB_QUIET_MS: i64 = 2 * 60 * 60 * 1000;
pub const HANDED_MS: i64 = 2 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct Evidence {
    pub claims: HashMap<String, Vec<Claim>>,
    pub claims_ok: bool,
    pub jobs: HashMap<String, u64>,
    pub handed: HashMap<String, u64>,
    pub since: HashMap<String, (Option<Status>, u64)>,
    pub asked: HashSet<String>,
    pub deep: HashSet<String>,
    pub gaps: Vec<String>,
}

impl Default for Evidence {
    fn default() -> Self {
        Evidence {
            claims: HashMap::new(),
            claims_ok: true,
            jobs: HashMap::new(),
            handed: HashMap::new(),
            since: HashMap::new(),
            asked: HashSet::new(),
            deep: HashSet::new(),
            gaps: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub task: i64,
    pub status: TaskStatus,
    pub blocked_on: Option<BlockedOn>,
    pub space: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    Task,
    Job,
    Question,
    Deep,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Busy {
    pub why: Why,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<i64>,
    pub waiting: bool,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reading {
    pub busy: Vec<Busy>,
    pub unsure: String,
    pub sure: bool,
}

pub fn gather(app: &App, terminals: &[String]) -> Evidence {
    let mut ev = Evidence::default();
    match crate::queue::db(app).and_then(|c| nixa_store::tasks::held_tasks(&c).map_err(|e| e.to_string())) {
        Ok(list) => {
            for t in list.into_iter().filter(|t| matches!(t.status, TaskStatus::Doing | TaskStatus::Blocked)) {
                ev.claims.entry(t.terminal_id.clone()).or_default().push(Claim {
                    task: t.id,
                    status: t.status,
                    blocked_on: t.blocked_on,
                    space: t.space_id,
                });
            }
        }
        Err(e) => {
            ev.claims_ok = false;
            ev.gaps.push(format!("task claims could not be read ({e})"));
        }
    }
    {
        let watch = lock(&app.watch);
        ev.jobs = watch.jobs.clone();
        ev.since = watch.world.iter().map(|(k, s)| (k.clone(), (s.info.status, s.since))).collect();
    }
    ev.handed = lock(&app.custom_agents.handed).clone();
    ev.asked = lock(&app.prompts.book).items.iter().map(|p| p.terminal.clone()).collect();
    let deep = lock(&app.deep);
    ev.deep = terminals.iter().filter(|t| deep.owns(t)).cloned().collect();
    ev
}

fn ago(since: u64, now: i64) -> String {
    match (now - since as i64).max(0) / 60_000 {
        0 => "less than a minute ago".into(),
        1 => "1 minute ago".into(),
        n => format!("{n} minutes ago"),
    }
}

fn busy(why: Why, waiting: bool, text: impl Into<String>) -> Busy {
    Busy { why, task: None, waiting, text: text.into() }
}

pub fn read(ev: &Evidence, terminal: &str, live: bool, status: Option<Status>, now: i64, gaps: &[String]) -> Reading {
    let mut notes: Vec<String> = ev.gaps.iter().chain(gaps).cloned().collect();
    let mut out = Vec::new();
    let claims: Vec<&Claim> = if terminal.is_empty() { Vec::new() } else { ev.claims.get(terminal).into_iter().flatten().collect() };
    if !live {
        if !claims.is_empty() {
            let ids: Vec<String> = claims.iter().map(|c| format!("#{}", c.task)).collect();
            notes.push(format!("its closed terminal still holds task {}", ids.join(", ")));
        }
        return Reading { busy: out, unsure: notes.join("; "), sure: ev.claims_ok && gaps.is_empty() };
    }
    for c in &claims {
        out.push(Busy {
            why: Why::Task,
            task: Some(c.task),
            waiting: c.status == TaskStatus::Blocked,
            text: claim_text(c.task, c.status, c.blocked_on, &c.space),
        });
    }
    if ev.deep.contains(terminal) {
        out.push(busy(Why::Deep, false, "it is a deep brain seat"));
    }
    if let Some(at) = ev.jobs.get(terminal) {
        let (seen, since) = ev.since.get(terminal).copied().unwrap_or((status, 0));
        let reported = since >= *at;
        let ended = reported && matches!(seen, Some(Status::Done | Status::Idle | Status::Error));
        let quiet = !reported && status != Some(Status::Working) && now - *at as i64 > JOB_QUIET_MS;
        if quiet {
            notes.push(format!("Nixa sent it a job {} and never heard back", ago(*at, now)));
        } else if !ended {
            out.push(busy(Why::Job, false, format!("Nixa sent it a job {} and it has not reported done", ago(*at, now))));
        }
    }
    if let Some(at) = ev.handed.get(terminal).filter(|at| now - **at as i64 <= HANDED_MS) {
        out.push(busy(Why::Job, false, format!("Nixa handed it work {}", ago(*at, now))));
    }
    if ev.asked.contains(terminal) {
        out.push(busy(Why::Question, true, "a question waits on its screen"));
    }
    match status {
        Some(Status::Working) => out.push(busy(Why::Status, false, "its terminal reports working")),
        Some(Status::Blocked) => out.push(busy(Why::Status, true, "its terminal reports blocked")),
        _ => {}
    }
    if status.is_none() && out.is_empty() {
        notes.push("its terminal reports no status and Nixa knows of no task, job or question for it".into());
    }
    Reading { busy: out, unsure: notes.join("; "), sure: ev.claims_ok && gaps.is_empty() }
}

pub fn handed(app: &App, terminal: &str, now: u64) {
    let mut map = lock(&app.custom_agents.handed);
    map.retain(|_, at| now.saturating_sub(*at) as i64 <= HANDED_MS);
    map.insert(terminal.to_string(), now);
}

pub fn state_of(base: AgentState, status: Option<Status>, busy: &[Busy]) -> AgentState {
    if base == AgentState::Closed {
        return base;
    }
    if status == Some(Status::Working) {
        return AgentState::Working;
    }
    if base == AgentState::Waiting || busy.iter().any(|b| b.waiting) {
        return AgentState::Waiting;
    }
    if !busy.is_empty() {
        return AgentState::Working;
    }
    base
}

pub fn free_now(state: AgentState, busy: &[Busy], off: bool, sure: bool) -> bool {
    state == AgentState::Free && busy.is_empty() && !off && sure
}

pub fn busy_text(busy: &[Busy]) -> String {
    busy.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("; ")
}
