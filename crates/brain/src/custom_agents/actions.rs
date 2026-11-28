use super::live::{self, AgentView, CUSTOM_LABEL, Live, PURPOSE_LABEL, RESUME_LABEL};
use super::start::{self, UseInput};
use crate::app::App;
use crate::tools::{Args, text};
use nixa_protocol::{Request, TermInfo};
use nixa_store::agents::{
    self as store, AddAgent, AgentUse, CustomAgent, EditAgent, StartProfile, session_from_label,
};
use nixa_store::text::clean_line;
use serde_json::{Value, json};
use std::sync::Arc;

pub const ACTIONS: [&str; 7] = ["list", "find", "save", "add", "edit", "use", "forget"];

#[derive(Debug, Clone, Default)]
pub struct Who {
    pub name: String,
    pub by_user: bool,
    pub caller: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub text: String,
    pub data: Value,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn conn(app: &App) -> Result<nixa_store::Connection, String> {
    nixa_store::open(&app.db_file).map_err(|e| format!("{e:#}"))
}

pub fn read_off(value: Option<&Value>) -> Result<Option<bool>, String> {
    let word = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Bool(b)) => return Ok(Some(*b)),
        Some(Value::String(s)) => s.trim().to_lowercase(),
        Some(other) => other.to_string(),
    };
    match word.as_str() {
        "" => Ok(None),
        "off" | "true" | "yes" | "bench" => Ok(Some(true)),
        "on" | "false" | "no" | "active" => Ok(Some(false)),
        _ => Err("off must be on or off.".into()),
    }
}

fn raw(args: &Args, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

pub fn profile_of(args: &Args) -> Option<StartProfile> {
    let agent = text(args, "agent")?.to_lowercase();
    Some(StartProfile {
        agent,
        model: text(args, "model").unwrap_or_default(),
        effort: text(args, "effort").unwrap_or_default(),
        args: Vec::new(),
    })
}

pub fn edit_of(args: &Args) -> Result<EditAgent, String> {
    Ok(EditAgent {
        title: text(args, "title"),
        purpose: text(args, "purpose"),
        instructions: raw(args, "instructions"),
        session: text(args, "session"),
        role: raw(args, "role"),
        job: raw(args, "job"),
        workflow: raw(args, "workflow"),
        profile_agent: text(args, "agent").map(|a| a.to_lowercase()),
        profile_model: text(args, "model"),
        profile_effort: text(args, "effort"),
        off: read_off(args.get("off"))?,
        skills: raw(args, "skills").map(|s| vec![s]),
    })
}

pub fn add_of(args: &Args, spaces: &[crate::spaces::Space]) -> Result<AddAgent, String> {
    let wanted = clean_line(&text(args, "space").unwrap_or_default(), 120);
    let space = if wanted.is_empty() {
        None
    } else {
        let found = spaces
            .iter()
            .find(|s| s.id == wanted || s.name.eq_ignore_ascii_case(&wanted));
        let names: Vec<&str> = spaces.iter().map(|s| s.name.as_str()).collect();
        let open = if names.is_empty() {
            "none".to_string()
        } else {
            names.join(", ")
        };
        Some(found.ok_or_else(|| format!("No space matches \"{wanted}\". Open spaces: {open}."))?)
    };
    Ok(AddAgent {
        title: text(args, "title").unwrap_or_default(),
        purpose: text(args, "purpose").unwrap_or_default(),
        instructions: raw(args, "instructions").unwrap_or_default(),
        provider: text(args, "provider").unwrap_or_default(),
        session: text(args, "session").unwrap_or_default(),
        space: space.map(|s| s.id.clone()).unwrap_or_default(),
        space_name: space.map(|s| s.name.clone()).unwrap_or_default(),
        folder: space.map(|s| s.folder.clone()).unwrap_or_default(),
        role: raw(args, "role").unwrap_or_default(),
        job: raw(args, "job").unwrap_or_default(),
        workflow: raw(args, "workflow").unwrap_or_default(),
        start: profile_of(args),
        off: read_off(args.get("off"))?.unwrap_or(false),
        migrated_from: String::new(),
        skills: raw(args, "skills").map(|s| vec![s]).unwrap_or_default(),
    })
}

pub fn edit_line(agent: &CustomAgent) -> String {
    let mut line = format!(
        "Changed {}: \"{}\" ({})",
        agent.id, agent.title, agent.purpose
    );
    line.push_str(if agent.instructions.is_empty() {
        ", no instructions"
    } else {
        ", with instructions"
    });
    if agent.role.is_empty() {
        line.push_str(", no role");
    } else {
        line.push_str(&format!(", role {}", agent.role));
    }
    if !agent.job.is_empty() {
        line.push_str(&format!(", job {}", agent.job));
    }
    if !agent.workflow.is_empty() {
        line.push_str(&format!(", default workflow {}", agent.workflow));
    }
    if let Some(start) = &agent.start {
        line.push_str(&format!(", starts as {}", store::profile_text(start)));
    }
    if agent.off {
        line.push_str(", off");
    }
    line.push('.');
    line
}

fn lines(views: &[AgentView], empty: &str) -> String {
    if views.is_empty() {
        return empty.to_string();
    }
    views
        .iter()
        .map(live::view_line)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn providers() -> Vec<&'static str> {
    let mut all: Vec<&str> = store::RESUME_PROVIDERS.to_vec();
    for p in store::START_PROVIDERS {
        if !all.contains(&p) {
            all.push(p);
        }
    }
    all
}

pub fn persist(app: &App, before: &CustomAgent, seen: &live::Seen) {
    if !seen.changed() {
        return;
    }
    let change = AgentUse {
        session: seen.session.clone(),
        space: seen.space.clone(),
        ..AgentUse::default()
    };
    let saved = conn(app).and_then(|c| {
        if let Some(learned) = &seen.session {
            super::start::attach_session(&c, before, learned);
        }
        store::use_agent(&c, &before.id, &change).map_err(err)
    });
    if let Err(e) = saved {
        crate::util::log(
            "custom agents",
            &format!("could not save what was seen for {}: {e}", before.id),
        );
    }
}

async fn tag(term: &str, agent: Option<&CustomAgent>) {
    let Ok(id) = crate::engine::term_id(term) else {
        return;
    };
    let custom = agent.map(|a| a.id.as_str());
    let purpose = agent.map(|a| a.purpose.as_str());
    let _ = crate::engine::label(&id, CUSTOM_LABEL, custom).await;
    let _ = crate::engine::label(&id, PURPOSE_LABEL, purpose).await;
}

async fn rename(term: &str, name: &str) {
    if let Ok(id) = crate::engine::term_id(term) {
        let _ = crate::engine::quick(Request::Rename {
            id,
            name: name.to_string(),
        })
        .await;
    }
}

pub async fn views(app: &App) -> Result<(Vec<AgentView>, Live), String> {
    let agents = store::list_agents(&conn(app)?).map_err(err)?;
    let live = live::gather(app).await;
    let now = nixa_store::time::now_ms();
    let mut out = Vec::new();
    for agent in &agents {
        let (view, seen) = live::view_of(&app.dir, agent, &live, now);
        persist(app, agent, &seen);
        if seen.live
            && let Some(t) = live.terms.get(&agent.terminal)
            && (t.labels.get(CUSTOM_LABEL) != Some(&agent.id)
                || t.labels.get(PURPOSE_LABEL) != Some(&agent.purpose))
        {
            tag(&agent.terminal, Some(agent)).await;
        }
        out.push(view);
    }
    out.sort_by_key(|v| std::cmp::Reverse(v.agent.last_used));
    Ok((out, live))
}

pub fn list_data(views: &[AgentView], live: &Live) -> Value {
    let spaces: Vec<Value> = live
        .spaces
        .iter()
        .map(|s| json!({"id": s.id, "name": if s.name.is_empty() { &s.id } else { &s.name }}))
        .collect();
    json!({"agents": views, "free": free_ids(views), "unsure": live.evidence.gaps.iter().chain(&live.gaps).collect::<Vec<_>>(), "spaces": spaces, "providers": providers(), "profile_agents": store::PROFILE_AGENTS, "teams": [], "workflows": []})
}

pub fn views_now(app: &App, agents: &[CustomAgent]) -> Vec<AgentView> {
    let live = live::watched(app);
    let now = nixa_store::time::now_ms();
    agents.iter().map(|a| live::view_of(&app.dir, a, &live, now).0).collect()
}

pub fn free_ids(views: &[AgentView]) -> Vec<String> {
    views
        .iter()
        .filter(|v| super::busy::free_now(v.state, &v.busy, v.agent.off, v.sure))
        .map(|v| v.agent.id.clone())
        .collect()
}

fn free_line(views: &[AgentView]) -> String {
    let free: Vec<String> = views
        .iter()
        .filter(|v| super::busy::free_now(v.state, &v.busy, v.agent.off, v.sure))
        .map(|v| format!("{} ({})", v.agent.id, v.agent.title))
        .collect();
    if free.is_empty() {
        "Free now: none. Working, waiting, off and closed agents are not free.".into()
    } else {
        format!("Free now: {}.", free.join(", "))
    }
}

fn with_links(app: &App, mut data: Value) -> Value {
    data["teams"] = team_cards(app);
    if let Ok(conn) = crate::workflows::open_db(app)
        && let Ok(list) = crate::workflows::routes::list_json(&conn)
    {
        data["workflows"] = list["workflows"].clone();
    }
    data
}

fn team_cards(app: &App) -> Value {
    let teams = crate::teams::runs::teams_file(app).map(|f| f.teams).unwrap_or_default();
    json!(teams
        .iter()
        .map(|t| json!({
            "id": format!("{}{}", crate::schedules::runner::TEAM_PREFIX, t.id),
            "name": t.name,
            "purpose": t.purpose,
            "flow": t.flow,
            "steps": t.steps.iter().map(|s| json!({"agent": s.agent})).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>())
}

fn find_order(views: Vec<AgentView>, query: &str, space: &str) -> Vec<AgentView> {
    let mut scored: Vec<(i64, AgentView)> = views
        .into_iter()
        .map(|v| (store::match_score(&v.agent, query), v))
        .filter(|(score, v)| {
            *score > 0
                && (space.is_empty()
                    || v.agent.space == space
                    || v.agent.space_name.eq_ignore_ascii_case(space))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.state.order().cmp(&b.1.state.order()))
            .then(b.1.agent.last_used.cmp(&a.1.agent.last_used))
    });
    scored.into_iter().map(|(_, v)| v).collect()
}

pub fn term_provider(t: &TermInfo) -> String {
    live::provider_of(t).unwrap_or_else(|| live::front_program(t))
}

async fn save(app: &App, args: &Args, who: &Who) -> Result<Outcome, String> {
    let terminal = text(args, "terminal")
        .or_else(|| who.caller.clone())
        .ok_or("save needs terminal.")?;
    let id = crate::engine::term_id(&terminal).map_err(err)?;
    let t = crate::engine::get(&id)
        .await
        .map_err(|_| format!("Terminal {terminal} is not open."))?;
    if !t.running {
        return Err(format!("Terminal {terminal} is not open."));
    }
    if !live::agent_running(&t) {
        return Err(format!(
            "No agent runs in terminal {terminal} now. Start the agent there first."
        ));
    }
    let provider = text(args, "provider")
        .map(|p| p.to_lowercase())
        .unwrap_or_else(|| term_provider(&t));
    let label = t
        .labels
        .get(RESUME_LABEL)
        .and_then(|l| session_from_label(l));
    let session = text(args, "session")
        .or_else(|| label.filter(|(p, _)| *p == provider).map(|(_, s)| s))
        .unwrap_or_default();
    let space_id = live::space_label(&t)
        .ok_or(format!("Terminal {terminal} is not in a space."))?
        .to_string();
    let spaces = app.spaces();
    let space = spaces.iter().find(|s| s.id == space_id);
    let c = conn(app)?;
    let all = store::list_agents(&c).map_err(err)?;
    let same = all
        .iter()
        .find(|a| a.terminal == t.id.as_str() && a.provider == provider)
        .or_else(|| {
            all.iter()
                .find(|a| !session.is_empty() && a.provider == provider && a.session == session)
        })
        .cloned();
    let agent = match same {
        Some(found) => {
            let mut change = edit_of(args)?;
            change.session = Some(session.clone()).filter(|s| !s.is_empty());
            change.profile_agent = None;
            change.profile_model = None;
            change.profile_effort = None;
            store::edit_agent(&c, &found.id, &change).unwrap_or(found)
        }
        None => {
            let mut input = add_of(args, &[])?;
            input.provider = provider.clone();
            input.session = session.clone();
            input.space = space_id.clone();
            input.space_name = space.map(|s| s.name.clone()).unwrap_or_default();
            input.folder = t.cwd.clone();
            input.start = None;
            store::add_agent(&c, &input, &who.name).map_err(err)?
        }
    };
    let agent = store::use_agent(
        &c,
        &agent.id,
        &AgentUse {
            terminal: Some(t.id.to_string()),
            space: Some(space_id),
            folder: Some(t.cwd.clone()),
            ..AgentUse::default()
        },
    )
    .map_err(err)?;
    drop(c);
    rename(t.id.as_str(), &agent.title).await;
    tag(t.id.as_str(), Some(&agent)).await;
    let live = live::gather(app).await;
    let (view, _) = live::view_of(&app.dir, &agent, &live, nixa_store::time::now_ms());
    Ok(Outcome {
        text: format!("Saved. {}", live::view_line(&view)),
        data: json!({"agent": view}),
    })
}

pub async fn action(app: &Arc<App>, name: &str, args: &Args, who: &Who) -> Result<Outcome, String> {
    let id = text(args, "id").or_else(|| text(args, "ref"));
    let out = match name {
        "list" => {
            let (views, live) = views(app).await?;
            return Ok(Outcome {
                text: lines(&views, "No custom agents are saved yet."),
                data: with_links(app, list_data(&views, &live)),
            });
        }
        "find" => {
            let query = text(args, "query")
                .or_else(|| text(args, "purpose"))
                .ok_or("find needs query, the purpose to look for.")?;
            let (views, _) = views(app).await?;
            let found = find_order(views, &query, &text(args, "space").unwrap_or_default());
            let empty =
                format!("No saved custom agent fits \"{query}\". Start a new agent and save it.");
            let text = if found.is_empty() { empty } else { format!("{}\n{}", lines(&found, &empty), free_line(&found)) };
            return Ok(Outcome {
                text,
                data: json!({"agents": found, "free": free_ids(&found)}),
            });
        }
        "save" => save(app, args, who).await?,
        "add" => {
            let input = add_of(args, &app.spaces())?;
            let agent = store::add_agent(&conn(app)?, &input, &who.name).map_err(err)?;
            let live = live::gather(app).await;
            let (view, _) = live::view_of(&app.dir, &agent, &live, nixa_store::time::now_ms());
            Outcome {
                text: format!("Added. {}", live::view_line(&view)),
                data: json!({"agent": view}),
            }
        }
        "edit" => {
            let id = id.ok_or("edit needs id.")?;
            let change = edit_of(args)?;
            let agent = store::edit_agent(&conn(app)?, &id, &change).map_err(err)?;
            if change.title.is_some() && !agent.terminal.is_empty() {
                rename(&agent.terminal, &agent.title).await;
            }
            if change.purpose.is_some() && !agent.terminal.is_empty() {
                tag(&agent.terminal, Some(&agent)).await;
            }
            Outcome {
                text: edit_line(&agent),
                data: json!({"agent": agent}),
            }
        }
        "forget" => {
            let id = id.ok_or("forget needs id.")?;
            let agent = store::forget_agent(&conn(app)?, &id).map_err(err)?;
            if !agent.terminal.is_empty() {
                tag(&agent.terminal, None).await;
            }
            Outcome {
                text: format!(
                    "Forgot {} \"{}\". Its terminal, if open, stays open.",
                    agent.id, agent.title
                ),
                data: json!({"agent": agent}),
            }
        }
        "use" => {
            let reference = id
                .clone()
                .or_else(|| text(args, "query"))
                .or_else(|| text(args, "purpose"))
                .ok_or("use needs id, a title or a purpose.")?;
            let input = UseInput {
                reference,
                message: text(args, "message").unwrap_or_default(),
                show: crate::tools::flag(args, "show") == Some(true),
                space: text(args, "space").unwrap_or_default(),
                by_user: who.by_user,
                exact: id.is_some() && who.by_user,
                copy: crate::tools::flag(args, "copy") == Some(true),
                independent: crate::tools::flag(args, "independent") == Some(true),
            };
            let used = start::use_agent(app, input).await?;
            Outcome {
                text: used.text.clone(),
                data: serde_json::to_value(&used).map_err(err)?,
            }
        }
        _ => return Err(format!("Choose one of: {}.", ACTIONS.join(", "))),
    };
    super::changed(app);
    Ok(out)
}
