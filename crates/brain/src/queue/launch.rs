use super::{asked, db, deep_seat};
use crate::app::App;
use crate::custom_agents::{How, UseInput, use_agent_with};
use crate::schedules::runner::{self, Job, Want};
use nixa_store::models::Prepared;
use crate::spaces::Space;
use nixa_protocol::{Status, TermId};
use nixa_store::tasks::{self, Task, TaskStatus};
use std::sync::Arc;

pub const CUSTOM_PREFIX: &str = "custom:";
pub const MEMBER_PREFIX: &str = "member:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seat {
    Custom(String),
    Terminal(String),
    Program(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub terminal: String,
    pub name: String,
    pub note: String,
    pub fresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Started(Started),
    Busy(String),
    Failed(String),
}

pub fn seat_of(task: &Task) -> Option<Seat> {
    let who = task.assigned.trim();
    if who.is_empty() {
        return None;
    }
    if task.assigned_kind == crate::tasks::KIND_CUSTOM {
        return Some(Seat::Custom(
            who.trim_start_matches(MEMBER_PREFIX).to_string(),
        ));
    }
    if let Some(id) = who
        .strip_prefix(CUSTOM_PREFIX)
        .or_else(|| who.strip_prefix(MEMBER_PREFIX))
    {
        return Some(Seat::Custom(id.to_string()));
    }
    if who.parse::<TermId>().is_ok() {
        return Some(Seat::Terminal(who.to_string()));
    }
    Some(Seat::Program(who.to_string()))
}

pub async fn terminal_open(terminal: &str) -> bool {
    let Ok(id) = crate::engine::term_id(terminal) else { return false };
    crate::engine::get(&id).await.is_ok_and(|info| info.running && !info.cmd.is_empty())
}

pub fn replace_gone(conn: &nixa_store::Connection, task: &Task, terminal: &str) -> Result<Task, String> {
    use nixa_store::tasks::{Actor, AuthorKind, CommentInput, CommentKind};
    let fail = |e: anyhow::Error| e.to_string();
    let by = Actor::new(AuthorKind::Nixa, "Nixa");
    let agent = nixa_store::agents::agent_by_terminal(conn, terminal).map_err(fail)?.filter(|a| !a.off);
    let line = match &agent {
        Some(agent) => {
            tasks::assign_task(conn, task.id, crate::tasks::KIND_CUSTOM, &agent.id).map_err(fail)?;
            let title = if agent.title.is_empty() { &agent.id } else { &agent.title };
            format!("Terminal {terminal} is closed, so Nixa gave the task to its agent profile {title} ({}).", agent.id)
        }
        None => {
            tasks::assign_task(conn, task.id, "", "").map_err(fail)?;
            tasks::set_status(conn, task.id, TaskStatus::Inbox, None, false).map_err(fail)?;
            format!("Terminal {terminal} is closed and has no agent profile, so Nixa moved the task to the inbox to be placed again.")
        }
    };
    tasks::add_comment(conn, task.id, &CommentInput::new(CommentKind::Note, &line), &by).map_err(fail)?;
    tasks::get_task(conn, task.id).map_err(fail)?.ok_or(format!("No task #{}.", task.id))
}

pub fn custom_of(task: &Task) -> Option<String> {
    match seat_of(task)? {
        Seat::Custom(id) => Some(id),
        _ => None,
    }
}

fn holds_other(app: &App, terminal: &str, id: i64) -> Option<i64> {
    let conn = db(app).ok()?;
    tasks::held_tasks(&conn)
        .ok()?
        .into_iter()
        .find(|t| t.terminal_id == terminal && t.id != id && t.status != TaskStatus::Review)
        .map(|t| t.id)
}

pub async fn open(
    app: &Arc<App>,
    task: &Task,
    space: &Space,
    seat: &Seat,
    prompt: &str,
    by_user: bool,
    want: Option<&Want>,
) -> Outcome {
    match seat {
        Seat::Custom(id) => custom(app, task, space, id, prompt, by_user, want).await,
        Seat::Terminal(terminal) => terminal_seat(app, task, terminal, prompt).await,
        Seat::Program(key) => program(app, task, space, key, prompt, want).await,
    }
}

fn job_of(task: &Task, space: &Space, agent: &str, prompt: &str) -> Job {
    Job {
        schedule: format!("task-{}", task.id),
        name: format!("#{} {}", task.id, super::prompt::cut(&task.title, 40)),
        agent: agent.to_string(),
        folder: super::scope::work_folder(task, &space.folder),
        prompt: prompt.to_string(),
    }
}

async fn custom(
    app: &Arc<App>,
    task: &Task,
    space: &Space,
    id: &str,
    prompt: &str,
    by_user: bool,
    want: Option<&Want>,
) -> Outcome {
    let agent = match db(app)
        .and_then(|c| nixa_store::agents::get_agent(&c, id).map_err(|e| e.to_string()))
    {
        Ok(Some(agent)) => agent,
        Ok(None) => return Outcome::Failed(format!("No saved custom agent {id}.")),
        Err(error) => return Outcome::Failed(error),
    };
    let title = if agent.title.is_empty() {
        agent.id.clone()
    } else {
        agent.title.clone()
    };
    if agent.off {
        return Outcome::Failed(format!("{title} ({id}) is off."));
    }
    if let Some(away) = agent.start.as_ref().and_then(|s| crate::leave::away_text(app, &s.agent)) {
        return Outcome::Busy(format!("{title} ({id}) waits: {away}"));
    }
    if !agent.terminal.is_empty() && deep_seat(app, &agent.terminal) {
        return Outcome::Failed(format!(
            "{title} ({id}) is a deep brain now, so it takes no task."
        ));
    }
    if !agent.terminal.is_empty() {
        if let Some(other) = holds_other(app, &agent.terminal, task.id) {
            return Outcome::Busy(format!("{title} ({id}) holds task #{other}"));
        }
        if asked(app, &agent.terminal) {
            return Outcome::Busy(format!("{title} ({id}) waits on a question"));
        }
    }
    let input = UseInput {
        reference: id.to_string(),
        message: prompt.to_string(),
        show: false,
        space: space.id.clone(),
        by_user,
        exact: true,
        copy: false,
        independent: false,
    };
    let session = db(app).ok().and_then(|c| crate::custom_agents::session_model(&c, &agent));
    let fresh_model = crate::custom_agents::model_changes(&agent, want, session.as_ref());
    let used = match use_agent_with(app, input, want).await {
        Ok(used) => used,
        Err(error) => return Outcome::Failed(error),
    };
    if used.ok && !used.terminal.is_empty() {
        let note = match used.how {
            How::Started if fresh_model => "Started a fresh session with the stage model and effort, because its last session used another model or effort.",
            How::Resumed => "Resumed its saved session.",
            How::Started => "Started its first session.",
            _ => "Reused the open custom agent.",
        };
        return Outcome::Started(Started {
            terminal: used.terminal,
            name: title,
            note: note.into(),
            fresh: false,
        });
    }
    match used.how {
        How::Busy => Outcome::Busy(used.text),
        _ => Outcome::Failed(used.text),
    }
}

async fn terminal_seat(app: &Arc<App>, task: &Task, terminal: &str, prompt: &str) -> Outcome {
    let Ok(id) = crate::engine::term_id(terminal) else {
        return Outcome::Failed(format!("{terminal} is not a terminal id."));
    };
    let info = match crate::engine::get(&id).await {
        Ok(info) => info,
        Err(_) => return Outcome::Failed(format!("Terminal {terminal} is not open.")),
    };
    if !info.running || info.cmd.is_empty() {
        return Outcome::Failed(format!("Terminal {terminal} runs no agent now."));
    }
    if deep_seat(app, terminal) {
        return Outcome::Failed(format!(
            "Terminal {terminal} is a deep brain now, so it takes no task."
        ));
    }
    if let Some(away) = crate::leave::away_text(app, &crate::prompts::scan::agent_of(&info)) {
        return Outcome::Busy(format!("terminal {terminal} waits: {away}"));
    }
    if let Some(other) = holds_other(app, terminal, task.id) {
        return Outcome::Busy(format!("terminal {terminal} holds task #{other}"));
    }
    if asked(app, terminal) {
        return Outcome::Busy(format!("terminal {terminal} waits on a question"));
    }
    if matches!(info.status, Some(Status::Working) | Some(Status::Blocked)) {
        return Outcome::Busy(format!("terminal {terminal} is busy"));
    }
    match crate::engine::send(&id, prompt, true, false).await {
        Ok(_) => Outcome::Started(Started {
            terminal: terminal.to_string(),
            name: if info.name.is_empty() {
                terminal.to_string()
            } else {
                info.name
            },
            note: "Sent to the open agent.".into(),
            fresh: false,
        }),
        Err(error) => Outcome::Busy(format!(
            "terminal {terminal} did not take the task: {error}"
        )),
    }
}

async fn program(
    app: &Arc<App>,
    task: &Task,
    space: &Space,
    key: &str,
    prompt: &str,
    want: Option<&Want>,
) -> Outcome {
    if let Some(away) = crate::leave::away_text(app, key) {
        return Outcome::Busy(away);
    }
    let job = job_of(task, space, key, prompt);
    let (plan, prepared) = match runner::launch_plan(app, &job, want) {
        Ok(found) => found,
        Err(error) => return Outcome::Failed(error.to_string()),
    };
    let label = plan.label.clone();
    let note = prepared.as_ref().map(model_note).unwrap_or_default();
    match runner::open_plan(app, &job, plan, prepared.as_ref()).await {
        Ok((terminal, _)) => {
            if let Ok(id) = crate::engine::term_id(&terminal) {
                let _ =
                    crate::engine::label(&id, super::TASK_LABEL, Some(&task.id.to_string())).await;
            }
            Outcome::Started(Started {
                terminal,
                name: label,
                note,
                fresh: true,
            })
        }
        Err(error) => Outcome::Failed(error.to_string()),
    }
}

fn model_note(prepared: &Prepared) -> String {
    if prepared.note.is_empty() {
        format!("Model: {}.", prepared.row.id)
    } else {
        format!("Model: {}. {}", prepared.row.id, prepared.note)
    }
}

pub async fn stop(terminal: &str) {
    if let Ok(id) = crate::engine::term_id(terminal) {
        runner::kill(&id).await;
    }
}

pub fn saved_seat(app: &App, terminal: &str) -> bool {
    db(app)
        .ok()
        .and_then(|c| nixa_store::agents::list_agents(&c).ok())
        .is_some_and(|list| list.iter().any(|a| a.terminal == terminal))
}
