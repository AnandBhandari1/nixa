use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const FALLBACK_PROVIDERS: [&str; 8] = [
    "claude", "codex", "cursor", "agy", "omp", "hermes", "dim", "opencode",
];
pub const EMPTY_TEXT: &str = "No custom agents yet. Any agent in the sidebar can become one: ask Nixa, for example: save terminal 9 as the news researcher. Or press Add agent and write one by hand.";
pub const CONNECTING_TEXT: &str = "Connecting to Nixa…";
pub const ABOUT_TEXT: &str = "These are your normal agents from the left sidebar, with a name tag, a purpose and standing instructions, or entries you write here by hand. This list keeps their space and their session, so Nixa and the deep brains can reuse them, or resume them when they are closed. It is not the Agent map, which picks a model for new work.";
pub const FOOT_TEXT: &str = "A tagged agent shows its purpose next to its name in the sidebar. Free agents are reused, closed ones are resumed in their own space, busy ones are never copied. Standing instructions tell Nixa and the deep brains how to direct the agent. A reviewer takes finished code work of its space once, by itself.";
pub const ADD_ABOUT: &str = "No open terminal is needed. With a provider and a session id Nixa can resume it later; without them it stays a definition until a session is added with Edit.";
pub const TEAMS_ABOUT: &str = "A team is custom agents that work one after another. To run one, tell Nixa, for example: give it to the Feature team in nixa.";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub agent: String,
    pub model: String,
    pub effort: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Agent {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub instructions: String,
    pub role: String,
    pub job: String,
    pub workflow: String,
    pub start: Option<Profile>,
    pub off: bool,
    pub provider: String,
    pub session: String,
    pub space: String,
    pub space_name: String,
    pub terminal: String,
    pub last_used: i64,
    pub state: String,
    pub number: Option<u32>,
    pub can_resume: bool,
    pub starts_new: bool,
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SpaceRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Workflow {
    pub name: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Step {
    pub agent: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub flow: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct List {
    pub agents: Vec<Agent>,
    pub spaces: Vec<SpaceRef>,
    pub providers: Vec<String>,
    pub profile_agents: Vec<String>,
    pub workflows: Vec<Workflow>,
    pub teams: Vec<Team>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Open,
    Start,
    Resume,
    Edit,
    TurnOn,
    TurnOff,
    MakeReviewer,
    StopReviewing,
    Workflow,
    Forget,
    ForgetYes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Good,
    Danger,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Form {
    pub title: String,
    pub purpose: String,
    pub instructions: String,
    pub session: String,
    pub model: String,
    pub effort: String,
}

pub fn parse_list(value: &Value) -> List {
    let data = value.get("data").unwrap_or(value);
    let mut list: List = serde_json::from_value(data.clone()).unwrap_or_default();
    list.agents.retain(|a| !a.id.is_empty());
    list.spaces.retain(|s| !s.id.is_empty());
    list.workflows.retain(|w| !w.name.is_empty());
    list
}

pub fn reply_text(value: &Value) -> String {
    value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn providers(list: &List) -> Vec<String> {
    if list.providers.is_empty() {
        return FALLBACK_PROVIDERS.iter().map(|p| p.to_string()).collect();
    }
    list.providers.clone()
}

pub fn profile_agents(list: &List) -> Vec<String> {
    if list.profile_agents.is_empty() {
        return nixa_store::agents::PROFILE_AGENTS
            .iter()
            .map(|p| p.to_string())
            .collect();
    }
    list.profile_agents.clone()
}

pub fn closed(a: &Agent) -> bool {
    a.state == "closed" || a.state.is_empty()
}

pub fn reviewer(a: &Agent) -> bool {
    a.role == nixa_store::agents::REVIEWER_ROLE
}

pub fn state_text(state: &str) -> String {
    match state {
        "free" => "Free".into(),
        "working" => "Working".into(),
        "waiting" => "Waiting for an answer".into(),
        "closed" | "" => "Closed".into(),
        other => other.to_string(),
    }
}

pub fn tone_of_state(state: &str) -> &'static str {
    match state {
        "free" => "green",
        "working" => "blue",
        "waiting" => "yellow",
        _ => "dim",
    }
}

pub fn ago(now_ms: i64, then_ms: i64) -> String {
    if then_ms <= 0 {
        return "never".into();
    }
    let minutes = ((now_ms - then_ms) as f64 / 60_000.0).round() as i64;
    if minutes < 1 {
        return "just now".into();
    }
    if minutes < 60 {
        return format!("{minutes} min ago");
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours < 48 {
        return format!("{hours} h ago");
    }
    format!("{} days ago", (hours as f64 / 24.0).round() as i64)
}

pub fn profile_text(p: &Profile) -> String {
    let model = if p.model.is_empty() {
        "default model"
    } else {
        &p.model
    };
    let effort = if p.effort.is_empty() {
        "default effort"
    } else {
        &p.effort
    };
    format!("{} · {model} · {effort}", p.agent)
}

pub fn space_text(a: &Agent) -> String {
    if a.space_name.is_empty() {
        a.space.clone()
    } else {
        a.space_name.clone()
    }
}

pub fn where_text(a: &Agent) -> String {
    if a.terminal.is_empty() || closed(a) {
        return "Not in the sidebar now".into();
    }
    match a.number {
        Some(n) => format!("In the sidebar as #{n}"),
        None => format!("In the sidebar as {}", a.terminal),
    }
}

pub fn job_line(a: &Agent) -> Option<String> {
    if a.job.is_empty() && a.start.is_none() {
        return None;
    }
    let job = if a.job.is_empty() {
        "No job".to_string()
    } else {
        format!("Job {}", a.job)
    };
    let start = a
        .start
        .as_ref()
        .map(|p| format!(" · starts as {}", profile_text(p)))
        .unwrap_or_default();
    Some(format!("{job}{start}"))
}

pub fn place_line(a: &Agent) -> String {
    let provider = if a.provider.is_empty() {
        "no provider"
    } else {
        &a.provider
    };
    let space = if !a.space.is_empty() {
        format!("space {}", space_text(a))
    } else if a.start.is_some() {
        "starts in the task's space".into()
    } else {
        "no space".into()
    };
    format!("{provider} · {space}")
}

pub fn warning(a: &Agent) -> Option<String> {
    (closed(a) && !a.can_resume && !a.starts_new).then(|| {
        format!(
            "Cannot resume: {}",
            if a.why.is_empty() { "unknown" } else { &a.why }
        )
    })
}

pub fn acts(a: &Agent, forgetting: bool) -> Vec<Act> {
    let mut out = Vec::new();
    if !closed(a) {
        out.push(Act::Open);
    }
    let profile = a.start.is_some();
    if profile && !a.off {
        out.push(Act::Start);
    } else if closed(a) && (a.can_resume || a.starts_new) && !a.off {
        out.push(Act::Resume);
    }
    out.push(Act::Edit);
    if profile {
        out.push(if a.off { Act::TurnOn } else { Act::TurnOff });
    }
    out.push(if reviewer(a) {
        Act::StopReviewing
    } else {
        Act::MakeReviewer
    });
    out.push(Act::Workflow);
    out.push(if forgetting {
        Act::ForgetYes
    } else {
        Act::Forget
    });
    out
}

pub fn act_key(act: Act) -> &'static str {
    match act {
        Act::Open => "open",
        Act::Start => "start",
        Act::Resume => "resume",
        Act::Edit => "edit",
        Act::TurnOn => "on",
        Act::TurnOff => "off",
        Act::MakeReviewer => "review-on",
        Act::StopReviewing => "review-off",
        Act::Workflow => "workflow",
        Act::Forget => "forget",
        Act::ForgetYes => "forget-yes",
    }
}

pub fn act_label(act: Act, a: &Agent, busy: bool) -> String {
    match act {
        Act::Open => "Open".into(),
        Act::Start if busy => "Starting…".into(),
        Act::Start if closed(a) => "Start".into(),
        Act::Start => "New tab".into(),
        Act::Resume if busy => "Resuming…".into(),
        Act::Resume if a.starts_new && !a.can_resume => "Start".into(),
        Act::Resume => "Resume".into(),
        Act::Edit => "Edit".into(),
        Act::TurnOn => "Turn on".into(),
        Act::TurnOff => "Turn off".into(),
        Act::MakeReviewer => "Make reviewer".into(),
        Act::StopReviewing => "Stop reviewing".into(),
        Act::Workflow if a.workflow.is_empty() => "Default workflow".into(),
        Act::Workflow => format!("Workflow: {}", a.workflow),
        Act::Forget => "Forget".into(),
        Act::ForgetYes => "Yes, forget".into(),
    }
}

pub fn act_tone(act: Act) -> Tone {
    match act {
        Act::Start | Act::Resume | Act::TurnOn => Tone::Good,
        Act::ForgetYes => Tone::Danger,
        _ => Tone::Plain,
    }
}

pub fn act_tip(act: Act, a: &Agent) -> String {
    let space = if a.space.is_empty() {
        "its space".to_string()
    } else {
        space_text(a)
    };
    let profile = a.start.as_ref().map(profile_text).unwrap_or_default();
    match act {
        Act::Open => "Show this agent in the sidebar and its terminal".into(),
        Act::Start if closed(a) => format!("Start {} as {profile} in the shown space", a.title),
        Act::Start => format!("{} is open; start one more tab of it in the shown space", a.title),
        Act::Resume if a.starts_new && !a.can_resume => format!("Open a new tab in {space} and start a new {} session", a.provider),
        Act::Resume => format!("Open a new tab in {space} and resume the saved {} session", a.provider),
        Act::Edit if a.start.is_some() => "Change the title, purpose, instructions, agent, model or effort".into(),
        Act::Edit => "Change the title, purpose, instructions or session".into(),
        Act::TurnOn => "Nixa may pick and start this agent again".into(),
        Act::TurnOff => "Nixa does not pick or start this agent while it is off".into(),
        Act::MakeReviewer => format!("Make this the review agent of {space}. When another agent finishes code work there, Nixa passes the task to it once. It only reads and reports its findings."),
        Act::StopReviewing => "This agent no longer takes finished code work by itself".into(),
        Act::Workflow => "Pick the workflow a task follows when it is given to this agent and has no workflow of its own".into(),
        Act::Forget => "Remove from this list".into(),
        Act::ForgetYes => "Remove from this list. The terminal stays open.".into(),
    }
}

pub fn act_request(act: Act, a: &Agent, space: Option<&str>) -> Option<(&'static str, Value)> {
    let id = a.id.clone();
    Some(match act {
        Act::Start => {
            let mut body = json!({"id": id, "show": true});
            if let Some(space) = space.filter(|s| !s.is_empty()) {
                body["space"] = json!(space);
            }
            if !closed(a) {
                body["copy"] = json!(true);
            }
            ("use", body)
        }
        Act::Resume => ("use", json!({"id": id, "show": true})),
        Act::TurnOn => ("edit", json!({"id": id, "off": "on"})),
        Act::TurnOff => ("edit", json!({"id": id, "off": "off"})),
        Act::MakeReviewer => ("edit", json!({"id": id, "role": "reviewer"})),
        Act::StopReviewing => ("edit", json!({"id": id, "role": "none"})),
        Act::ForgetYes => ("forget", json!({"id": id})),
        Act::Open | Act::Edit | Act::Workflow | Act::Forget => return None,
    })
}

pub fn workflow_request(a: &Agent, name: Option<&str>) -> (&'static str, Value) {
    (
        "edit",
        json!({"id": a.id, "workflow": name.unwrap_or("none")}),
    )
}

pub fn form_of(a: &Agent) -> Form {
    let p = a.start.clone().unwrap_or_default();
    Form {
        title: a.title.clone(),
        purpose: a.purpose.clone(),
        instructions: a.instructions.clone(),
        session: a.session.clone(),
        model: p.model,
        effort: p.effort,
    }
}

pub fn edit_body(a: &Agent, form: &Form, profile_agent: &str) -> Value {
    let mut body = Map::new();
    body.insert("id".into(), json!(a.id));
    body.insert("title".into(), json!(form.title.trim()));
    body.insert("purpose".into(), json!(form.purpose.trim()));
    body.insert("instructions".into(), json!(form.instructions.trim()));
    if !form.session.trim().is_empty() {
        body.insert("session".into(), json!(form.session.trim()));
    }
    if a.start.is_some() {
        if !profile_agent.is_empty() {
            body.insert("agent".into(), json!(profile_agent));
        }
        if !form.model.trim().is_empty() {
            body.insert("model".into(), json!(form.model.trim()));
        }
        let effort = form.effort.trim();
        body.insert(
            "effort".into(),
            json!(if effort.is_empty() { "default" } else { effort }),
        );
    }
    Value::Object(body)
}

pub fn add_body(form: &Form, provider: &str, space: &str) -> Result<Value, String> {
    let title = form.title.trim();
    let purpose = form.purpose.trim();
    if title.is_empty() || purpose.is_empty() {
        return Err("A custom agent needs a title and a purpose.".into());
    }
    Ok(json!({
        "title": title,
        "purpose": purpose,
        "instructions": form.instructions.trim(),
        "provider": provider,
        "session": form.session.trim(),
        "space": space,
    }))
}

pub fn team_line(team: &Team, agents: &[Agent]) -> String {
    team.steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let title = agents
                .iter()
                .find(|a| a.id == s.agent)
                .map(|a| a.title.as_str())
                .unwrap_or(&s.agent);
            format!("{}. {title}", i + 1)
        })
        .collect::<Vec<_>>()
        .join("  →  ")
}

pub fn used_terminal(value: &Value) -> Option<String> {
    let data = value.get("data")?;
    if data.get("show").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    data.get("terminal")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}
