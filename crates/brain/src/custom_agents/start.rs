use super::launch::{self, Plan};
use super::live::{self, AgentView, CUSTOM_LABEL, PURPOSE_LABEL, RESUME_LABEL};
use crate::app::{App, lock};
use crate::spaces::Space;
use nixa_protocol::{CreateSpec, ReadSource, Reply, Request, TermId};
use nixa_store::agents::{
    self as store, AgentState, AgentUse, CustomAgent, RESUME_PROVIDERS, session_from_label,
};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

pub const READY_MS: u64 = 90_000;
pub const SESSION_MS: u64 = 20_000;
pub const CHECK_MS: u64 = 3_000;
pub const UNSURE_WAIT_MS: u64 = 3_000;
pub const POLL_MS: u64 = 1_000;
pub const MESSAGE_MAX: usize = 8_000;
const PROBLEM_LIMIT: usize = 240;
const PROBLEM_WORDS: [&str; 10] = [
    "error",
    "not found",
    "no such",
    "denied",
    "cannot",
    "can't",
    "failed",
    "unknown",
    "invalid",
    "usage:",
];

#[derive(Debug, Clone, Default)]
pub struct UseInput {
    pub reference: String,
    pub message: String,
    pub show: bool,
    pub space: String,
    pub by_user: bool,
    pub exact: bool,
    pub copy: bool,
    pub independent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    Reused,
    Resumed,
    Started,
    Busy,
    Cannot,
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct Used {
    pub ok: bool,
    pub how: How,
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub terminal: String,
    pub show: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Off,
    Copy,
    Busy,
    Send(String),
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub argv: Vec<String>,
    pub send: String,
    pub fresh: bool,
    pub profiled: bool,
}

fn used(ok: bool, how: How, text: String, view: Option<&AgentView>) -> Used {
    Used {
        ok,
        how,
        text,
        terminal: String::new(),
        show: false,
        agent: view.cloned(),
    }
}

fn name_of(view: &AgentView) -> String {
    format!("{} ({})", view.agent.title, view.agent.id)
}

pub fn step(view: &AgentView, input: &UseInput, elsewhere: bool) -> Step {
    let busy = matches!(view.state, AgentState::Working | AgentState::Waiting);
    if view.agent.off && !input.by_user {
        return Step::Off;
    }
    if view.agent.start.is_some() && (busy || input.copy || elsewhere) {
        return Step::Copy;
    }
    if busy {
        return Step::Busy;
    }
    if view.state == AgentState::Free && !view.agent.terminal.is_empty() {
        return Step::Send(view.agent.terminal.clone());
    }
    Step::Open
}

pub(crate) fn own_space<'a>(agent: &CustomAgent, spaces: &'a [Space]) -> Option<&'a Space> {
    if agent.space.is_empty() {
        return None;
    }
    spaces
        .iter()
        .find(|s| s.id == agent.space)
        .or_else(|| {
            let folder = agent.folder.trim();
            (!folder.is_empty())
                .then(|| spaces.iter().find(|s| crate::spaces::same_folder(&s.folder, folder)))
                .flatten()
        })
        .or_else(|| {
            let name = agent.space_name.trim();
            (!name.is_empty())
                .then(|| spaces.iter().find(|s| s.name.trim().eq_ignore_ascii_case(name)))
                .flatten()
        })
        .or_else(|| {
            let key = agent.space.trim();
            spaces.iter().find(|s| s.name.trim().eq_ignore_ascii_case(key))
        })
}

pub fn space_for(agent: &CustomAgent, wanted: &str, spaces: &[Space]) -> Option<Space> {
    let key = wanted.trim().to_lowercase();
    let named = if key.is_empty() {
        None
    } else {
        spaces
            .iter()
            .find(|s| s.id.to_lowercase() == key || s.name.to_lowercase() == key)
            .or_else(|| spaces.iter().find(|s| s.folder.to_lowercase() == key))
    };
    if agent.start.is_some() && named.is_some() {
        return named.cloned();
    }
    if agent.space.is_empty() {
        return named.cloned();
    }
    own_space(agent, spaces).cloned()
}

pub fn launch_for(dir: &std::path::Path, view: &AgentView, message: &str) -> Option<Launch> {
    let fresh = view.starts_new && view.resume.is_empty();
    let profiled = fresh && view.agent.start.is_some();
    if profiled {
        let Plan { argv, send } = launch::profile_plan(dir, &view.agent, message)?;
        return Some(Launch {
            argv: launch::shell(dir, &launch::join(&argv)),
            send: send.unwrap_or_default(),
            fresh,
            profiled,
        });
    }
    if fresh {
        let command = nixa_store::agents::start_command_in(dir, &view.agent.provider)?;
        return Some(Launch {
            argv: launch::shell(dir, command),
            send: launch::first_message(&view.agent, message),
            fresh,
            profiled,
        });
    }
    if !view.can_resume || view.resume.is_empty() {
        return None;
    }
    let send = if message.is_empty() {
        String::new()
    } else {
        launch::with_instructions(&view.agent, message)
    };
    Some(Launch {
        argv: launch::shell(dir, &view.resume),
        send,
        fresh,
        profiled,
    })
}

pub fn start_problem(screen: &str) -> String {
    let lines: Vec<String> = screen
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with(['╭', '╰', '│', '─']))
        .map(str::to_string)
        .collect();
    let found = lines
        .iter()
        .find(|l| {
            let low = l.to_lowercase();
            PROBLEM_WORDS.iter().any(|w| low.contains(w))
        })
        .cloned()
        .unwrap_or_else(|| {
            lines
                .iter()
                .rev()
                .take(2)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
        });
    let flat = nixa_store::text::one_line(&found);
    nixa_store::text::clip_dots(&flat, PROBLEM_LIMIT)
}

pub fn not_started(name: &str, terminal: &str, problem: &str) -> String {
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(" ({problem})")
    };
    format!(
        "{name} did not start in terminal {terminal}: its terminal closed{problem}. Nothing was typed into a shell, and Nixa closed that terminal."
    )
}

struct Opening<'a> {
    app: &'a App,
    id: String,
}

impl Drop for Opening<'_> {
    fn drop(&mut self) {
        lock(&self.app.custom_agents.opening).remove(&self.id);
    }
}

fn opening<'a>(app: &'a App, id: &str) -> Option<Opening<'a>> {
    lock(&app.custom_agents.opening)
        .insert(id.to_string())
        .then(|| Opening {
            app,
            id: id.to_string(),
        })
}

fn remember(app: &App, id: &str, change: &AgentUse) {
    let saved = nixa_store::open(&app.db_file).and_then(|conn| store::use_agent(&conn, id, change));
    if let Err(e) = saved {
        crate::util::log(
            "custom agents",
            &format!("could not save the use of {id}: {e:#}"),
        );
    }
}

pub fn tab_id(agent: &str) -> String {
    format!("c{agent}{}", crate::util::now_ms())
}

pub fn tab_spec(agent: &CustomAgent, space: &Space, argv: Vec<String>, parent: Option<TermId>) -> CreateSpec {
    let folder = [agent.folder.as_str(), space.folder.as_str()]
        .into_iter()
        .find(|f| !f.is_empty() && std::path::Path::new(f).is_dir())
        .map(str::to_string);
    let mut spec = CreateSpec {
        cmd: argv,
        cwd: folder,
        name: Some(agent.title.clone()),
        parent,
        ..CreateSpec::default()
    };
    for key in crate::announce::SPACE_LABELS {
        spec.labels.insert(key.into(), space.id.clone());
    }
    spec.labels.insert(nixa_protocol::id::SPACE_SLUG_LABEL.into(), space.name.clone());
    spec.labels.insert("tab".into(), tab_id(&agent.id));
    spec.labels.insert(CUSTOM_LABEL.into(), agent.id.clone());
    spec.labels
        .insert(PURPOSE_LABEL.into(), agent.purpose.clone());
    spec
}

pub async fn deep_parent(app: &App, space: &str, independent: bool) -> Option<TermId> {
    if independent {
        return None;
    }
    let terminal = crate::app::lock(&app.deep).parent_in(space)?;
    let id = crate::engine::term_id(&terminal).ok()?;
    let info = crate::engine::get(&id).await.ok()?;
    (info.running && info.parent.is_none()).then_some(id)
}

async fn screen(id: &TermId) -> String {
    match crate::engine::quick(Request::Read {
        id: id.clone(),
        source: ReadSource::History,
        lines: Some(40),
    })
    .await
    {
        Ok(Reply::Text { text, .. }) => text,
        _ => String::new(),
    }
}

async fn ended(id: &TermId) -> bool {
    !crate::engine::get(id)
        .await
        .map(|t| t.running)
        .unwrap_or(false)
}

async fn failed(id: &TermId, name: &str) -> String {
    let problem = start_problem(&screen(id).await);
    let _ = crate::engine::remove(id).await;
    not_started(name, id.as_str(), &problem)
}

async fn check_started(id: &TermId, ms: u64) -> bool {
    let mut waited = 0;
    while waited < ms {
        tokio::time::sleep(Duration::from_millis(POLL_MS.min(ms))).await;
        waited += POLL_MS.min(ms);
        if ended(id).await {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ready {
    Yes,
    Ended,
    Asking,
    Late,
}

pub fn asks(screen: &str) -> bool {
    crate::prompts::dialog::YES_NO.is_match(screen)
        || crate::prompts::dialog::parse_dialog(screen).kind
            != crate::prompts::dialog::DialogKind::None
}

pub fn not_sent(ready: Ready) -> &'static str {
    match ready {
        Ready::Asking => {
            " The message was not sent: the agent shows a question on its screen, so Nixa did not type into it. Answer it first."
        }
        Ready::Ended => " The message was not sent because the agent ended.",
        _ => " The message was not sent because the agent did not get ready in time.",
    }
}

async fn wait_ready(id: &TermId) -> Ready {
    let mut waited = 0;
    let mut asking = false;
    while waited < READY_MS {
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
        waited += POLL_MS;
        let Ok(term) = crate::engine::get(id).await else {
            return Ready::Ended;
        };
        if !term.running {
            return Ready::Ended;
        }
        let text = screen(id).await;
        asking = asks(&text);
        if asking {
            continue;
        }
        if crate::term_tools::ready(false, term.status, waited, !text.trim().is_empty()) {
            return Ready::Yes;
        }
    }
    if asking { Ready::Asking } else { Ready::Late }
}

async fn deliver(id: &TermId, text: &str) -> Ready {
    match wait_ready(id).await {
        Ready::Yes if crate::engine::send(id, text, true, false).await.is_ok() => Ready::Yes,
        Ready::Yes => Ready::Late,
        other => other,
    }
}

async fn wait_session(app: &App, agent: &CustomAgent, id: &TermId) -> Option<String> {
    let mut waited = 0;
    while waited < SESSION_MS {
        let labels = crate::engine::get(id)
            .await
            .map(|t| t.labels)
            .unwrap_or_default();
        if let Some((provider, session)) =
            labels.get(RESUME_LABEL).and_then(|l| session_from_label(l))
            && provider == agent.provider
        {
            remember(
                app,
                &agent.id,
                &AgentUse {
                    session: Some(session.clone()),
                    ..AgentUse::default()
                },
            );
            return Some(session);
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS * 2)).await;
        waited += POLL_MS * 2;
    }
    None
}

fn number_text(app: &App, terminal: &str) -> String {
    lock(&app.announce.numbers)
        .get(terminal)
        .map(|n| n.to_string())
        .unwrap_or_else(|| terminal.to_string())
}

fn reasons(view: &AgentView) -> String {
    if view.busy.is_empty() {
        String::new()
    } else {
        format!(": {}", super::busy::busy_text(&view.busy))
    }
}

async fn recheck(app: &App, view: &AgentView) -> AgentView {
    let live = live::gather(app).await;
    live::view_of(&app.dir, &view.agent, &live, nixa_store::time::now_ms()).0
}

pub async fn use_agent(app: &Arc<App>, input: UseInput) -> Result<Used, String> {
    use_agent_with(app, input, None).await
}

pub fn with_model(mut agent: CustomAgent, want: Option<&crate::schedules::runner::Want>) -> CustomAgent {
    let (Some(want), Some(start)) = (want, agent.start.as_mut()) else {
        return agent;
    };
    let model = match want.model.split_once("::") {
        Some((owner, model)) if owner == start.agent => model.to_string(),
        Some(_) => String::new(),
        None => want.model.clone(),
    };
    if !model.is_empty() {
        start.model = model;
    }
    if !want.effort.is_empty() {
        start.effort = want.effort.clone();
    }
    agent
}

pub const SESSION_MODEL_KEY: &str = "custom_agent.session_model:";

pub fn session_model(conn: &nixa_store::Connection, agent: &CustomAgent) -> Option<(String, String)> {
    let raw = nixa_store::tasks::task_meta(conn, &format!("{SESSION_MODEL_KEY}{}", agent.id)).ok().flatten()?;
    let saved: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let text = |key: &str| saved.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    let (session, terminal, base) = (text("session"), text("terminal"), text("base"));
    let same = if session.is_empty() { !terminal.is_empty() && terminal == agent.terminal && base.is_empty() && agent.session.is_empty() } else { session == agent.session };
    same.then(|| (text("model"), text("effort")))
}

pub fn save_session_model(conn: &nixa_store::Connection, agent: &CustomAgent, terminal: &str, session: &str) {
    let Some(start) = agent.start.as_ref() else {
        return;
    };
    let record = serde_json::json!({"session": session, "terminal": terminal, "base": agent.session, "model": start.model, "effort": start.effort});
    let _ = nixa_store::tasks::set_task_meta(conn, &format!("{SESSION_MODEL_KEY}{}", agent.id), &record.to_string());
}

pub fn attach_session(conn: &nixa_store::Connection, agent: &CustomAgent, session: &str) {
    let Ok(Some(raw)) = nixa_store::tasks::task_meta(conn, &format!("{SESSION_MODEL_KEY}{}", agent.id)) else {
        return;
    };
    let Ok(mut saved) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let terminal = saved.get("terminal").and_then(serde_json::Value::as_str).unwrap_or_default();
    let known = saved.get("session").and_then(serde_json::Value::as_str).unwrap_or_default();
    if !known.is_empty() || terminal.is_empty() || terminal != agent.terminal || session.is_empty() {
        return;
    }
    saved["session"] = serde_json::Value::String(session.to_string());
    let _ = nixa_store::tasks::set_task_meta(conn, &format!("{SESSION_MODEL_KEY}{}", agent.id), &saved.to_string());
}

pub fn model_changes(agent: &CustomAgent, want: Option<&crate::schedules::runner::Want>, session: Option<&(String, String)>) -> bool {
    let Some(start) = agent.start.as_ref() else {
        return false;
    };
    if want.is_none() {
        return false;
    }
    let asked = with_model(agent.clone(), want).start.unwrap_or_default();
    let (model, effort) = session.cloned().unwrap_or_else(|| (start.model.clone(), start.effort.clone()));
    asked.model != model || asked.effort != effort
}

pub fn for_model(mut view: AgentView, step: Step, changed: bool) -> (AgentView, Step) {
    if !changed {
        return (view, step);
    }
    view.resume.clear();
    view.starts_new = true;
    let step = match step {
        Step::Send(_) => Step::Copy,
        other => other,
    };
    (view, step)
}

pub async fn use_agent_with(app: &Arc<App>, input: UseInput, want: Option<&crate::schedules::runner::Want>) -> Result<Used, String> {
    let picked = {
        let conn = nixa_store::open(&app.db_file).map_err(|e| e.to_string())?;
        if input.exact {
            store::get_agent(&conn, &input.reference)
        } else {
            store::pick_agent(&conn, &input.reference, &input.space)
        }
        .map_err(|e| e.to_string())?
    };
    let saved = picked.clone();
    let Some(agent) = picked.map(|agent| with_model(agent, want)) else {
        return Ok(used(
            false,
            How::None,
            format!("No saved custom agent matches \"{}\".", input.reference),
            None,
        ));
    };
    let live = live::gather(app).await;
    let now = nixa_store::time::now_ms();
    let (view, seen) = live::view_of(&app.dir, &agent, &live, now);
    if seen.changed() {
        remember(
            app,
            &agent.id,
            &AgentUse {
                session: seen.session.clone(),
                space: seen.space.clone(),
                ..AgentUse::default()
            },
        );
    }
    let session = nixa_store::open(&app.db_file).ok().and_then(|conn| {
        if let Some(learned) = &seen.session {
            attach_session(&conn, &view.agent, learned);
        }
        session_model(&conn, &view.agent)
    });
    let changed = saved.as_ref().is_some_and(|agent| model_changes(agent, want, session.as_ref()));
    let message = nixa_store::text::clip(input.message.trim(), MESSAGE_MAX);
    let wanted = space_for(&view.agent, &input.space, &live.spaces);
    let own = own_space(&view.agent, &live.spaces)
        .map(|s| s.id.as_str())
        .unwrap_or(view.agent.space.as_str());
    let elsewhere = view.agent.start.is_some()
        && !view.agent.space.is_empty()
        && !input.space.trim().is_empty()
        && wanted.as_ref().map(|s| s.id.as_str()) != Some(own);
    let first = step(&view, &input, elsewhere);
    let (view, next) = for_model(view, first, changed);
    match next {
        Step::Off => Ok(used(
            false,
            How::Cannot,
            format!(
                "{} is off. Turn it on first, or pick another agent.",
                name_of(&view)
            ),
            Some(&view),
        )),
        Step::Busy => {
            let what = if view.state == AgentState::Working {
                "busy"
            } else {
                "waiting for an answer"
            };
            let text = format!(
                "{} is {what} in terminal {}{}. Nothing was sent and no copy was started.",
                name_of(&view),
                number_text(app, &view.agent.terminal),
                reasons(&view)
            );
            Ok(used(false, How::Busy, text, Some(&view)))
        }
        Step::Copy => {
            let Some(_guard) = opening(app, &view.agent.id) else {
                return Ok(used(
                    false,
                    How::Busy,
                    format!("{} is being started already.", name_of(&view)),
                    Some(&view),
                ));
            };
            start_copy(app, &view, &message, &input, wanted).await
        }
        Step::Send(terminal) => reuse(app, &view, &message, &input, &terminal, wanted).await,
        Step::Open => open(app, &view, &message, &input, wanted).await,
    }
}

async fn reuse(
    app: &Arc<App>,
    view: &AgentView,
    message: &str,
    input: &UseInput,
    terminal: &str,
    wanted: Option<Space>,
) -> Result<Used, String> {
    let id = crate::engine::term_id(terminal).map_err(|e| e.to_string())?;
    let Some(_guard) = opening(app, &view.agent.id) else {
        return Ok(used(
            false,
            How::Busy,
            format!("{} is taking another handoff right now. Nothing was sent.", name_of(view)),
            Some(view),
        ));
    };
    let mut fresh = recheck(app, view).await;
    if !fresh.sure && !input.by_user {
        tokio::time::sleep(Duration::from_millis(UNSURE_WAIT_MS)).await;
        fresh = recheck(app, view).await;
    }
    if fresh.state != AgentState::Free || !fresh.busy.is_empty() {
        let text = format!(
            "{} became busy before the handoff in terminal {}{}. Nothing was sent.",
            name_of(view),
            number_text(app, terminal),
            reasons(&fresh)
        );
        return Ok(used(false, How::Busy, text, Some(&fresh)));
    }
    if !fresh.sure && !input.by_user && view.agent.start.is_some() && wanted.is_some() {
        crate::modes::note(app, "job", &format!("{}: Nixa started a fresh copy", view.agent.title), Some(terminal), Some(&format!("Nixa could not tell if it was free: {}", fresh.unsure)));
        return start_copy(app, view, message, input, wanted).await;
    }
    if !fresh.sure && !input.by_user {
        crate::modes::note(app, "job", &format!("{}: Nixa sent the handoff although it was not sure the agent was free", view.agent.title), Some(terminal), Some(&fresh.unsure));
    }
    if !message.is_empty() {
        super::busy::handed(app, terminal, crate::util::now_ms());
    }
    let sent = if message.is_empty() {
        true
    } else {
        crate::engine::send(
            &id,
            &launch::with_instructions(&view.agent, message),
            true,
            false,
        )
        .await
        .is_ok()
    };
    if !sent {
        lock(&app.custom_agents.handed).remove(terminal);
    } else if !message.is_empty() {
        super::busy::handed(app, terminal, crate::util::now_ms());
    }
    remember(app, &view.agent.id, &AgentUse::default());
    let what = match (message.is_empty(), sent) {
        (true, _) => "",
        (false, true) => " Message sent.",
        (false, false) => " The message could not be sent.",
    };
    let shown = if input.show { " Shown." } else { "" };
    let unsure = if fresh.unsure.is_empty() { String::new() } else { format!(" Not sure it was free: {}.", fresh.unsure) };
    let text = format!(
        "{} is open and free in terminal {}.{what}{shown}{unsure}",
        name_of(view),
        number_text(app, terminal)
    );
    Ok(Used {
        ok: sent,
        how: How::Reused,
        text,
        terminal: terminal.to_string(),
        show: input.show,
        agent: Some(view.clone()),
    })
}

async fn start_copy(
    app: &Arc<App>,
    view: &AgentView,
    message: &str,
    input: &UseInput,
    space: Option<Space>,
) -> Result<Used, String> {
    let Some(space) = space else {
        return Ok(used(
            false,
            How::Cannot,
            format!(
                "{} is busy and Nixa has no space to start a fresh tab of it in. Give the space.",
                name_of(view)
            ),
            Some(view),
        ));
    };
    let Some(plan) = launch::profile_plan(&app.dir, &view.agent, message) else {
        return Ok(used(
            false,
            How::Cannot,
            format!("{} has no start profile Nixa can run.", name_of(view)),
            Some(view),
        ));
    };
    let argv = launch::shell(&app.dir, &launch::join(&plan.argv));
    let mut copy = view.agent.clone();
    copy.folder = String::new();
    let parent = deep_parent(app, &space.id, input.independent).await;
    let term = crate::engine::create(tab_spec(&copy, &space, argv, parent))
        .await
        .map_err(|e| format!("Nixa could not open a fresh tab of {}: {e}", name_of(view)))?;
    let id = term.id.clone();
    remember(app, &view.agent.id, &AgentUse::default());
    if !check_started(&id, CHECK_MS).await {
        return Ok(used(
            false,
            How::Cannot,
            failed(&id, &view.agent.title).await,
            Some(view),
        ));
    }
    let ready = match plan.send.as_deref() {
        Some(text) => deliver(&id, text).await,
        None => Ready::Yes,
    };
    let sent = ready == Ready::Yes;
    let profile = view
        .agent
        .start
        .as_ref()
        .map(store::profile_text)
        .unwrap_or_default();
    let with = if message.is_empty() {
        ""
    } else if sent {
        " Message sent with the start."
    } else {
        not_sent(ready)
    };
    let text = format!(
        "{} was busy, so Nixa started a fresh {profile} tab of it in terminal {} in {}.{with}",
        name_of(view),
        number_text(app, id.as_str()),
        space.name
    );
    Ok(Used {
        ok: sent,
        how: How::Started,
        text,
        terminal: id.to_string(),
        show: input.show,
        agent: Some(view.clone()),
    })
}

async fn open(
    app: &Arc<App>,
    view: &AgentView,
    message: &str,
    input: &UseInput,
    space: Option<Space>,
) -> Result<Used, String> {
    let Some(launch) = launch_for(&app.dir, view, message) else {
        let why = if view.why.is_empty() {
            "no resume command"
        } else {
            &view.why
        };
        return Ok(used(
            false,
            How::Cannot,
            format!("{} is closed and cannot be resumed: {why}.", name_of(view)),
            Some(view),
        ));
    };
    let doing = if launch.fresh { "start" } else { "resume" };
    let Some(space) = space else {
        let text = if view.agent.space.is_empty() {
            format!(
                "{} has no space saved and no space was given, so Nixa cannot {doing} it. Give the space.",
                name_of(view)
            )
        } else {
            format!(
                "{} belongs to space {}, which is not open. Open that space first.",
                name_of(view),
                if view.agent.space_name.is_empty() {
                    &view.agent.space
                } else {
                    &view.agent.space_name
                }
            )
        };
        return Ok(used(false, How::Cannot, text, Some(view)));
    };
    let Some(_guard) = opening(app, &view.agent.id) else {
        return Ok(used(
            false,
            How::Busy,
            format!("{} is being {doing}ed already.", name_of(view)),
            Some(view),
        ));
    };
    let parent = deep_parent(app, &space.id, input.independent).await;
    let term = crate::engine::create(tab_spec(&view.agent, &space, launch.argv.clone(), parent))
        .await
        .map_err(|e| {
            format!(
                "Nixa could not open a tab to {doing} {}: {e}",
                name_of(view)
            )
        })?;
    let id = term.id.clone();
    let rebound = view.agent.space != space.id;
    let stale_folder =
        view.agent.folder.is_empty() || !std::path::Path::new(&view.agent.folder).is_dir();
    remember(
        app,
        &view.agent.id,
        &AgentUse {
            terminal: Some(id.to_string()),
            space: rebound.then(|| space.id.clone()),
            space_name: rebound.then(|| space.name.clone()),
            folder: (rebound && stale_folder).then(|| space.folder.clone()),
            ..AgentUse::default()
        },
    );
    if !check_started(&id, CHECK_MS).await {
        remember(
            app,
            &view.agent.id,
            &AgentUse {
                terminal: Some(String::new()),
                ..AgentUse::default()
            },
        );
        return Ok(used(
            false,
            How::Cannot,
            failed(&id, &view.agent.title).await,
            Some(view),
        ));
    }
    let ready = if launch.send.is_empty() {
        Ready::Yes
    } else {
        deliver(&id, &launch.send).await
    };
    let sent = ready == Ready::Yes;
    let session = if launch.fresh && RESUME_PROVIDERS.contains(&view.agent.provider.as_str()) {
        wait_session(app, &view.agent, &id).await
    } else {
        Some(view.agent.session.clone()).filter(|s| !s.is_empty())
    };
    if launch.fresh
        && launch.profiled
        && let Ok(conn) = nixa_store::open(&app.db_file)
    {
        save_session_model(&conn, &view.agent, id.as_str(), session.as_deref().unwrap_or_default());
    }
    let number = number_text(app, id.as_str());
    let provider = if view.agent.provider.is_empty() {
        "new"
    } else {
        &view.agent.provider
    };
    let what = if launch.fresh {
        match &session {
            Some(s) => format!(
                "{} started a new {provider} session in terminal {number} and Nixa saved its session id {s}, so later uses resume it.",
                name_of(view)
            ),
            None => format!(
                "{} started a new {provider} session in terminal {number}. Its session id is not known yet; Nixa saves it when the terminal shows it.",
                name_of(view)
            ),
        }
    } else {
        format!(
            "{} resumed its {provider} session in terminal {number}.",
            name_of(view)
        )
    };
    let after = match (launch.profiled, launch.send.is_empty(), sent) {
        (true, _, _) if !message.is_empty() => {
            " Its instructions and the message went with the start."
        }
        (true, _, _) => " Its instructions went with the start.",
        (false, true, _) => "",
        (false, false, false) => not_sent(ready),
        (false, false, true) if launch.fresh && message.is_empty() => {
            " Its standing instructions were sent."
        }
        (false, false, true) => " Message sent.",
    };
    let how = if launch.fresh {
        How::Started
    } else {
        How::Resumed
    };
    Ok(Used {
        ok: sent,
        how,
        text: format!("{what}{after}"),
        terminal: id.to_string(),
        show: input.show,
        agent: Some(view.clone()),
    })
}
