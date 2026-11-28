use crate::state::State;
use crate::tools::{Tool, parse, schema};
use anyhow::{Result, anyhow};
use nixa_store::Connection;
use nixa_store::models::act::{AGENT_MODELS_ABOUT, MODELS_ABOUT};
use nixa_store::models::lists::{LIST_AGENTS, LIST_TIMEOUT};
use nixa_store::models::rank::COSTS;
use nixa_store::models::{ACTIONS, By, CAPABILITIES, act, action_name, agent_rows, model_list, parse_args, refresh_list, run_list_command, want_public};
use nixa_store::time::now_ms;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

pub const MODELS: &str = "nixa_models";
pub const AGENT_MODELS: &str = "nixa_list_agent_models";
pub const QUEUED: &str = "The brain fetches the public scores within a minute. Call list after that to see them.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    agent: String,
    refresh: Option<bool>,
}

fn db_file(state: &State) -> PathBuf {
    state.data_dir.join(nixa_store::DB_FILE)
}

fn open(state: &State) -> Result<Connection> {
    nixa_store::open(&db_file(state))
}

pub fn tools() -> Vec<Tool> {
    let caps = json!({"type": "array", "items": {"type": "string", "enum": CAPABILITIES}});
    vec![
        Tool {
            name: MODELS,
            description: MODELS_ABOUT.into(),
            input_schema: schema(
                json!({
                    "action": {"type": "string", "enum": ACTIONS},
                    "task": {"type": "string", "maxLength": 2000},
                    "capabilities": caps,
                    "named": {"type": "string", "maxLength": 200},
                    "agent": {"type": "string", "maxLength": 40},
                    "from": {"type": "string", "maxLength": 200},
                    "id": {"type": "string", "maxLength": 200},
                    "rank": {"type": "integer", "minimum": 1},
                    "secondary": {"type": "string", "maxLength": 200},
                    "cost": {"type": "string", "enum": COSTS},
                    "disabled": {"type": "boolean"},
                    "pinned": {"type": "boolean"},
                    "pins": caps,
                    "public_names": {"type": "array", "maxItems": 10, "items": {"type": "string", "minLength": 1, "maxLength": 120}},
                    "company": {"type": "string", "minLength": 1, "maxLength": 80},
                    "plan": {"type": "string", "minLength": 1, "maxLength": 80},
                }),
                &[],
            ),
            worker: false,
        },
        Tool {
            name: AGENT_MODELS,
            description: AGENT_MODELS_ABOUT.into(),
            input_schema: schema(
                json!({
                    "agent": {"type": "string", "enum": LIST_AGENTS},
                    "refresh": {"type": "boolean", "description": "Read the list from the agent now instead of the saved list"},
                }),
                &["agent"],
            ),
            worker: true,
        },
    ]
}

pub fn run(state: &State, name: &str, args: &Value) -> Option<Result<Value>> {
    let result = match name {
        MODELS => models(state, args),
        AGENT_MODELS => parse::<ListArgs>(args).and_then(|a| agent_models(state, &a.agent, a.refresh.unwrap_or(false))),
        _ => return None,
    };
    Some(result)
}

fn models(state: &State, args: &Value) -> Result<Value> {
    let parsed = parse_args(args)?;
    let conn = open(state)?;
    if action_name(&parsed) == "refresh" {
        want_public(&conn)?;
        return Ok(json!({"refreshed": false, "queued": true, "note": QUEUED}));
    }
    act(&conn, &parsed, By::Nixa)
}

fn run_command(argv: &[&str]) -> Result<String> {
    run_list_command(argv, LIST_TIMEOUT)
}

fn agent_models(state: &State, agent: &str, refresh: bool) -> Result<Value> {
    let agent = agent.trim();
    let conn = open(state)?;
    let list = model_list(&conn, agent, refresh, now_ms(), &run_command)?;
    if list.stale && !refresh && list.error.is_empty() {
        let (db, agent) = (db_file(state), agent.to_string());
        std::thread::spawn(move || {
            if let Ok(conn) = nixa_store::open(&db) {
                let _ = refresh_list(&conn, &agent, now_ms(), &run_command);
            }
        });
    }
    let mut value = serde_json::to_value(&list)?;
    value["leaderboard"] = serde_json::to_value(agent_rows(&conn, agent)?).map_err(|e| anyhow!(e))?;
    Ok(value)
}

#[cfg(test)]
pub fn seed_fixed(state: &State) {
    let conn = open(state).unwrap();
    nixa_store::models::update(&conn, |cache| {
        cache.ranking = None;
        nixa_store::models::pick::ensure_map(cache, now_ms(), &nixa_store::models::lists::fixed_models())
    })
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Mode;

    fn state() -> (tempfile_dir::Dir, State) {
        let dir = tempfile_dir::Dir::new();
        let state = State::new(Mode::Full, dir.0.join("e.sock"), dir.0.clone(), None);
        (dir, state)
    }

    mod tempfile_dir {
        pub struct Dir(pub std::path::PathBuf);

        impl Dir {
            pub fn new() -> Dir {
                use std::sync::atomic::{AtomicU64, Ordering};
                static COUNT: AtomicU64 = AtomicU64::new(0);
                let n = COUNT.fetch_add(1, Ordering::SeqCst);
                let path = std::env::temp_dir().join(format!("nixa-mcp-models-{}-{n}", std::process::id()));
                std::fs::create_dir_all(&path).unwrap();
                Dir(path)
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn models_tool_lists_edits_and_queues_refresh() {
        let (_dir, state) = state();
        seed_fixed(&state);
        let list = run(&state, MODELS, &json!({})).unwrap().unwrap();
        assert!(list["rows"].as_array().unwrap().iter().any(|r| r["id"] == "claude::claude-sonnet-5-5"));
        let pin = run(&state, MODELS, &json!({"action": "edit", "id": "claude::claude-sonnet-5-5", "pins": ["ui"]})).unwrap();
        assert!(pin.is_ok());
        nixa_store::settings::set_setting_text(&open(&state).unwrap(), "decision.mode", "manual").unwrap();
        let pin = run(&state, MODELS, &json!({"action": "edit", "id": "claude::claude-sonnet-5-5", "pins": ["ui"]})).unwrap();
        assert!(pin.is_ok());
        let edit = run(&state, MODELS, &json!({"action": "edit", "id": "claude::claude-sonnet-5-5", "disabled": true})).unwrap().unwrap();
        assert!(edit["rows"].as_array().unwrap().iter().any(|r| r["id"] == "claude::claude-sonnet-5-5" && r["disabled"] == true));
        let queued = run(&state, MODELS, &json!({"action": "refresh"})).unwrap().unwrap();
        assert_eq!(queued["queued"], true);
        let conn = open(&state).unwrap();
        assert!(nixa_store::models::take_public_wish(&conn).unwrap());
        let view = run(&state, MODELS, &json!({"action": "view"})).unwrap().unwrap();
        assert_eq!(view["choices"].as_array().unwrap().len(), 5);
        assert!(run(&state, "nixa_other", &json!({})).is_none());
    }

    #[test]
    fn agent_models_tool_checks_args() {
        let (_dir, state) = state();
        let claude = run(&state, AGENT_MODELS, &json!({"agent": "claude"})).unwrap().unwrap();
        assert_eq!(claude["models"].as_array().unwrap().len(), 3);
        assert_eq!(claude["leaderboard"].as_array().unwrap().len(), 3);
        assert!(run(&state, AGENT_MODELS, &json!({"agent": "claude", "x": 1})).unwrap().is_err());
        assert!(run(&state, AGENT_MODELS, &json!({"agent": "nobody"})).unwrap().is_err());
        assert!(tools().iter().any(|t| t.name == AGENT_MODELS && t.worker));
    }
}
