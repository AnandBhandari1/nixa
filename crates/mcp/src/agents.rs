use crate::state::State;
use crate::term;
use crate::tools::{Tool, parse, schema};
use anyhow::{Result, bail};
use nixa_protocol::{CreateSpec, Reply, Request};
use nixa_store::agent_catalog::{self, AGENTS_FILE, StartOptions, StartPlan};
use nixa_store::models::{Prepared, record_start};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

pub use nixa_store::agent_catalog::load;

const START_WAIT: Duration = Duration::from_secs(20);
const TASK_MAX: usize = 20_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "nixa_agent_list",
            description: format!(
                "List the coding agents Nixa can start: key, name, the program it runs and whether it is installed. Your own entries go in {AGENTS_FILE} in the Nixa data folder. Use this before nixa_agent_start."
            ),
            input_schema: schema(json!({}), &[]),
            worker: true,
        },
        Tool {
            name: "nixa_agent_start",
            description: "Start an agent from nixa_agent_list in a new Nixa terminal, with an optional folder and name. When this runs inside a top-level Nixa terminal, the new one is its child. A child terminal cannot start another child. The model follows the shared pick: the named model, then capability pins, then the saved rank. When the agent is not installed or has no free model, the next installed agent in the fall over order starts and the reply has a note. A task (first message) is given to the agent as its last argument, and it runs at once.".into(),
            input_schema: schema(
                json!({
                    "agent": { "type": "string", "description": "Agent key or name from nixa_agent_list, for example claude or codex" },
                    "model": { "type": "string", "description": "Optional exact model for this agent, from nixa_models. A named model is never swapped" },
                    "cwd": { "type": "string", "description": "Folder the agent starts in" },
                    "name": { "type": "string", "maxLength": 80, "description": "Terminal name. Default: the agent name" },
                    "task": { "type": "string", "description": "Optional first message for the agent" },
                }),
                &["agent"],
            ),
            worker: false,
        },
    ]
}

pub fn run(state: &State, name: &str, args: &Value) -> Option<Result<Value>> {
    let result = match name {
        "nixa_agent_list" => parse::<NoArgs>(args).map(|_| json!({ "agents": agent_catalog::list(&state.data_dir) })),
        "nixa_agent_start" => parse(args).and_then(|a| start(state, a)),
        _ => return None,
    };
    Some(result)
}

fn start(state: &State, args: StartOptions) -> Result<Value> {
    let parent = term::start_parent(state)?;
    let StartPlan { spec, task, prepared, note, .. } = agent_catalog::plan(&state.data_dir, args, parent)?;
    let mut value = match (prepared, task) {
        (Some(prepared), Some(task)) => start_picked(state, spec, task, prepared)?,
        (Some(prepared), None) => start_plain(state, spec, prepared)?,
        (None, task) => term::start(state, spec, task)?,
    };
    if !note.is_empty() {
        value["note"] = json!(note);
    }
    Ok(value)
}

fn db_file(state: &State) -> PathBuf {
    state.data_dir.join(nixa_store::DB_FILE)
}

fn start_plain(state: &State, spec: CreateSpec, prepared: Prepared) -> Result<Value> {
    let folder = spec.cwd.clone().unwrap_or_default();
    let term = match term::engine(&state.socket, &Request::Create { spec }, START_WAIT)? {
        Reply::Term { term } => term,
        other => bail!("nixa-engine gave an unexpected reply: {other:?}"),
    };
    if let Ok(conn) = nixa_store::open(&db_file(state)) {
        let _ = record_start(&conn, &prepared, term.id.as_str(), "", &folder, "", &term.name);
    }
    let mut value = term::summary(&term);
    value["started"] = json!(true);
    value["first_message"] = json!(false);
    value["model"] = json!(prepared.row.id);
    Ok(value)
}

fn start_picked(state: &State, mut spec: CreateSpec, task: String, prepared: Prepared) -> Result<Value> {
    if task.chars().count() > TASK_MAX {
        bail!("The task is longer than {TASK_MAX} characters. Nothing was started.");
    }
    let model = prepared.row.id.clone();
    spec.cmd.push(task.clone());
    let folder = spec.cwd.clone().unwrap_or_default();
    let term = match term::engine(&state.socket, &Request::Create { spec }, START_WAIT)? {
        Reply::Term { term } => term,
        other => bail!("nixa-engine gave an unexpected reply: {other:?}"),
    };
    if let Ok(conn) = nixa_store::open(&db_file(state)) {
        let _ = record_start(&conn, &prepared, term.id.as_str(), &task, &folder, "", &term.name);
    }
    let mut value = term::summary(&term);
    value["started"] = json!(true);
    value["first_message"] = json!(true);
    value["model"] = json!(model);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_task_start_picks_a_model_and_runs_at_once() {
        let dir = std::env::temp_dir().join(format!("nixa-mcp-agents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let program = dir.join("bin").join("claude");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = json!({"agents": {"claude": {"run": [program.display().to_string()]}, "mine": {"name": "Mine", "run": ["/bin/sh"]}}});
        std::fs::write(dir.join(AGENTS_FILE), file.to_string()).unwrap();
        let state = State::new(crate::state::Mode::Full, dir.join("e.sock"), dir.clone(), None);
        crate::models::seed_fixed(&state);
        assert!(agent_catalog::list(&dir).iter().any(|agent| agent.key == "claude" && agent.installed));
        let bad_model = StartOptions { agent: "claude".into(), model: Some("no-such-model".into()), cwd: None, name: None, task: None };
        assert!(start(&state, bad_model).unwrap_err().to_string().contains("no-such-model"));
        let conn = nixa_store::open(&db_file(&state)).unwrap();
        nixa_store::settings::set_setting_text(&conn, "decision.mode", "manual").unwrap();
        drop(conn);
        let args = StartOptions { agent: "claude".into(), model: None, cwd: None, name: None, task: Some("fix the menu".into()) };
        assert!(start(&state, args).is_err());
        let args = StartOptions { agent: "mine".into(), model: None, cwd: None, name: None, task: Some("say hi".into()) };
        assert!(start(&state, args).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
