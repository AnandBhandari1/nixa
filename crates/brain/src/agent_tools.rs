use crate::app::{App, lock};
use crate::tools::{Args, ToolResult, need, schema, spec, text};
use crate::turns::ToolSpec;
use nixa_protocol::{Reply, Request};
use nixa_store::agent_catalog::{self, StartOptions, StartPlan};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
#[path = "agent_tools/tests.rs"]
mod tests;

const LIST: &str = "nixa_agent_list";
const START: &str = "nixa_agent_start";
const START_WAIT: Duration = Duration::from_secs(20);
const TASK_MAX: usize = 20_000;
pub(super) const NAMES: [&str; 2] = [LIST, START];

pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        spec(
            LIST,
            "List the coding agents Nixa can start, with keys, names, programs and whether they are installed. Use before nixa_agent_start.",
            schema(json!({}), &[]),
        ),
        spec(
            START,
            "Start a coding agent from nixa_agent_list in a new Nixa terminal. Give its key, space ID, folder and an optional first task. The model follows the shared pick and an exact model never switches. Use parent only when starting a child of an existing top-level terminal.",
            schema(json!({
                "agent": {"type": "string", "description": "Coding agent key from nixa_agent_list"},
                "model": {"type": "string", "description": "Optional exact model or model ID"},
                "space": {"type": "string", "description": "Space ID"},
                "cwd": {"type": "string", "description": "Starting folder. Defaults to the space folder."},
                "name": {"type": "string", "description": "Terminal name. Defaults to the agent name."},
                "task": {"type": "string", "description": "Optional first message to give the agent"},
                "parent": {"type": "string", "description": "Optional top-level parent terminal ID"}
            }), &["agent"]),
        ),
    ]
}

pub(super) async fn call(app: &Arc<App>, name: &str, args: &Args) -> ToolResult {
    match name {
        LIST => {
            let agents: Vec<_> = agent_catalog::list(&app.dir)
                .into_iter()
                .map(|agent| json!({
                    "key": agent.key,
                    "name": agent.name,
                    "program": agent.run.first(),
                    "installed": agent.installed,
                }))
                .collect();
            Ok(json!({"agents": agents}).to_string())
        }
        START => start(app, args).await,
        _ => Err(format!("There is no tool named {name}.")),
    }
}

async fn start(app: &Arc<App>, args: &Args) -> ToolResult {
    let spaces = app.spaces();
    let space = text(args, "space")
        .map(|id| crate::spaces::find(&spaces, &id).ok_or_else(|| format!("No space named {id}.")))
        .transpose()?;
    let options = StartOptions {
        agent: need(args, "agent")?,
        model: text(args, "model"),
        cwd: text(args, "cwd").or_else(|| space.map(|space| space.folder.clone())),
        name: text(args, "name"),
        task: text(args, "task"),
    };
    if let Some(task) = &options.task {
        if task.chars().count() > TASK_MAX {
            return Err(format!("The task is longer than {TASK_MAX} characters. Nothing was started."));
        }
        if task.starts_with('-') {
            return Err("Start the first message with words so the agent can read it as a task. Nothing was started.".into());
        }
    }
    let space = space.or_else(|| options.cwd.as_deref().and_then(|cwd| crate::spaces::for_folder(&spaces, cwd)));
    let link = app.announce.link();
    let parent = if let Some(id) = text(args, "parent") {
        let term = link.get(&id).await.map_err(|error| error.to_string())?;
        nixa_protocol::term::check_start_parent(&term.id, term.parent.as_ref())?;
        Some(term.id)
    } else {
        None
    };
    let named = options.model.clone();
    let StartPlan { mut spec, task, prepared, note, .. } =
        agent_catalog::plan(&app.dir, options, parent).map_err(|error| format!("{error:#}"))?;
    let task = task.map(|task| format!("{task}\n\n{}", crate::status_rule::rule()));
    if let Some(model) = named
        && prepared.is_none()
    {
        return Err(format!("Could not select the model {model}. Nothing was started."));
    }
    let space_id = space.map(|space| space.id.as_str());
    if let Some(id) = space_id {
        spec.labels.insert(crate::deep::SPACE_LABEL.into(), id.into());
    }
    let whole = crate::term_tools::start_text(&spec.cmd, task.as_deref());
    if let Some(held) = crate::term_tools::work_hold(app, "start-agent", &whole, None)
        .or_else(|| crate::term_tools::same_work(app, &whole, None))
    {
        return Ok(held);
    }
    if let Some(task) = &task {
        spec.cmd.push(task.clone());
    }
    let reply = link.call(Request::Create { spec }, START_WAIT).await.map_err(|error| error.to_string())?;
    let Reply::Term { term } = reply else {
        return Err(format!("nixa-engine gave an unexpected reply: {reply:?}"));
    };
    let id = term.id.as_str();
    if let Some(prepared) = &prepared
        && let Err(error) = crate::models::record_start(app, prepared, id, task.as_deref().unwrap_or_default(), &term.cwd, space_id.unwrap_or_default(), &term.name)
    {
        crate::util::log("models", &format!("the model start of {id} was not saved: {error}"));
    }
    if let Some(task) = &task {
        crate::followup::flow::job_started(app, id, task);
        lock(&app.watch).jobs.insert(id.into(), crate::util::now_ms());
    }
    Ok(json!({
        "started": true,
        "terminal": id,
        "name": term.name,
        "cwd": term.cwd,
        "model": prepared.map(|model| model.row.id),
        "first_message": task.is_some(),
        "note": note,
    }).to_string())
}
