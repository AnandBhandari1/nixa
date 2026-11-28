use super::busy::{self, Busy, Evidence};
use crate::app::App;
use crate::spaces::Space;
use nixa_protocol::{Status, TermInfo};
use nixa_store::agents::{
    AgentState, CustomAgent, agent_resume_in, agent_state, cannot_resume, session_from_label,
    starts_new,
};
use serde::Serialize;
use std::collections::HashMap;

pub const CUSTOM_LABEL: &str = "nixa.custom";
pub const PURPOSE_LABEL: &str = "nixa.purpose";
pub const RESUME_LABEL: &str = "resume";
pub const WORKING_BUMP_MS: i64 = 60_000;

const SHELLS: [&str; 9] = [
    "sh", "bash", "zsh", "dash", "fish", "env", "login", "ksh", "tcsh",
];

#[derive(Debug, Clone, Default)]
pub struct Live {
    pub terms: HashMap<String, TermInfo>,
    pub numbers: HashMap<String, u32>,
    pub spaces: Vec<Space>,
    pub evidence: Evidence,
    pub gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentView {
    #[serde(flatten)]
    pub agent: CustomAgent,
    pub state: AgentState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub resume: String,
    pub can_resume: bool,
    pub starts_new: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub busy: Vec<Busy>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unsure: String,
    pub sure: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    pub live: bool,
    pub session: Option<String>,
    pub space: Option<String>,
    pub bump: bool,
}

impl Seen {
    pub fn changed(&self) -> bool {
        self.session.is_some() || self.space.is_some() || self.bump
    }
}

pub async fn gather(app: &App) -> Live {
    match crate::engine::list().await {
        Ok(list) => made(app, list, Vec::new()),
        Err(_) => cached(app),
    }
}

fn cached(app: &App) -> Live {
    let list = app.terminals().into_iter().map(|s| s.info).collect();
    made(app, list, vec!["the engine did not answer, so the terminal states come from Nixa's last copy".into()])
}

pub fn watched(app: &App) -> Live {
    let list = app.terminals().into_iter().map(|s| s.info).collect();
    made(app, list, Vec::new())
}

fn made(app: &App, terms: Vec<TermInfo>, gaps: Vec<String>) -> Live {
    let terms: HashMap<String, TermInfo> = terms.into_iter().map(|t| (t.id.to_string(), t)).collect();
    let ids: Vec<String> = terms.keys().cloned().collect();
    Live {
        evidence: busy::gather(app, &ids),
        terms,
        numbers: crate::app::lock(&app.announce.numbers).clone(),
        spaces: app.spaces(),
        gaps,
    }
}

fn base(word: &str) -> String {
    word.rsplit('/')
        .next()
        .unwrap_or(word)
        .trim_start_matches('-')
        .to_lowercase()
}

pub fn provider_word(word: &str) -> Option<&'static str> {
    Some(match base(word).as_str() {
        "claude" | "claude-code" => "claude",
        "codex" => "codex",
        "cursor" | "cursor-agent" => "cursor",
        "agy" | "agyd" | "antigravity" => "agy",
        "omp" => "omp",
        "hermes" => "hermes",
        "opencode" => "opencode",
        "jcode" => "jcode",
        "dim" | "dima" | "dimcode" => "dim",
        "crush" => "crush",
        "omo" => "omo",
        "gemini" => "gemini",
        _ => return None,
    })
}

pub fn words_of(t: &TermInfo) -> Vec<String> {
    let mut out = Vec::new();
    for (i, part) in t.cmd.iter().enumerate() {
        let script = i > 0
            && t.cmd
                .get(i - 1)
                .is_some_and(|p| p.starts_with('-') && p.ends_with('c'));
        if script {
            out.extend(part.split_whitespace().map(str::to_string));
        } else {
            out.push(part.clone());
        }
    }
    out.into_iter().take(16).collect()
}

pub fn front_program(t: &TermInfo) -> String {
    words_of(t)
        .into_iter()
        .filter(|w| !w.starts_with('-') && !w.contains('='))
        .map(|w| base(&w))
        .find(|w| !SHELLS.contains(&w.as_str()) && !w.is_empty())
        .unwrap_or_default()
}

pub fn provider_of(t: &TermInfo) -> Option<String> {
    if let Some((provider, _)) = t
        .labels
        .get(RESUME_LABEL)
        .and_then(|l| session_from_label(l))
    {
        return Some(provider);
    }
    words_of(t)
        .iter()
        .find_map(|w| provider_word(w))
        .map(str::to_string)
}

pub fn agent_running(t: &TermInfo) -> bool {
    t.running && (t.status.is_some() || !front_program(t).is_empty())
}

pub fn space_label(t: &TermInfo) -> Option<&str> {
    crate::announce::SPACE_LABELS
        .iter()
        .find_map(|k| t.labels.get(*k))
        .map(String::as_str)
        .filter(|s| !s.is_empty())
}

pub fn attach(agent: &CustomAgent, t: Option<&TermInfo>, now: i64) -> Seen {
    let Some(t) = t.filter(|t| agent_running(t)) else {
        return Seen::default();
    };
    if let Some(provider) = provider_of(t)
        && !agent.provider.is_empty()
        && provider != agent.provider
    {
        return Seen::default();
    }
    let mut seen = Seen {
        live: true,
        ..Seen::default()
    };
    if let Some((provider, session)) = t
        .labels
        .get(RESUME_LABEL)
        .and_then(|l| session_from_label(l))
        && provider == agent.provider
        && session != agent.session
    {
        seen.session = Some(session);
    }
    if let Some(space) = space_label(t)
        && space != agent.space
    {
        seen.space = Some(space.to_string());
    }
    seen.bump = t.status == Some(Status::Working) && now - agent.last_used > WORKING_BUMP_MS;
    seen
}

pub fn apply(agent: &mut CustomAgent, seen: &Seen, now: i64) {
    if let Some(session) = &seen.session {
        agent.session = session.clone();
    }
    if let Some(space) = &seen.space {
        agent.space = space.clone();
    }
    if seen.bump {
        agent.last_used = now;
    }
}

pub fn view_of(dir: &std::path::Path, agent: &CustomAgent, live: &Live, now: i64) -> (AgentView, Seen) {
    let term = (!agent.terminal.is_empty())
        .then(|| live.terms.get(&agent.terminal))
        .flatten();
    let seen = attach(agent, term, now);
    let mut agent = agent.clone();
    apply(&mut agent, &seen, now);
    if let Some(space) = live.spaces.iter().find(|s| s.id == agent.space) {
        agent.space_name = space.name.clone();
    }
    let status = term.filter(|_| seen.live).and_then(|t| t.status);
    let reading = busy::read(&live.evidence, &agent.terminal, seen.live, status, now, &live.gaps);
    let state = busy::state_of(agent_state(seen.live, status), status, &reading.busy);
    let number = if seen.live {
        live.numbers.get(&agent.terminal).copied()
    } else {
        None
    };
    let why = cannot_resume(&agent).unwrap_or_default();
    let view = AgentView {
        resume: agent_resume_in(dir, &agent).unwrap_or_default(),
        can_resume: why.is_empty(),
        starts_new: starts_new(&agent),
        why,
        number,
        state,
        busy: reading.busy,
        unsure: reading.unsure,
        sure: reading.sure,
        agent,
    };
    (view, seen)
}

pub fn where_line(view: &AgentView) -> String {
    let terminal = &view.agent.terminal;
    match view.number {
        Some(n) => format!("terminal {n} ({terminal})"),
        None if terminal.is_empty() => "no terminal".into(),
        None => format!("terminal {terminal}"),
    }
}

pub fn view_line(view: &AgentView) -> String {
    let state = match view.state {
        AgentState::Closed if view.can_resume => "closed, can resume".to_string(),
        AgentState::Closed if view.starts_new => {
            "not started yet, use starts a new session".to_string()
        }
        AgentState::Closed => format!(
            "closed, cannot resume: {}",
            if view.why.is_empty() {
                "unknown"
            } else {
                &view.why
            }
        ),
        AgentState::Free => format!("free in {}", where_line(view)),
        AgentState::Waiting => format!("waiting for an answer in {}", where_line(view)),
        AgentState::Working => format!("working in {}", where_line(view)),
    };
    let line = nixa_store::agents::agent_line(&view.agent, view.state);
    let cut = line.rfind(" · ").map(|at| &line[..at]).unwrap_or(&line);
    let why = if view.busy.is_empty() { String::new() } else { format!(": {}", busy::busy_text(&view.busy)) };
    let unsure = if view.unsure.is_empty() { String::new() } else { format!(" (not sure: {})", view.unsure) };
    format!("{cut} · {state}{why}{unsure}")
}
