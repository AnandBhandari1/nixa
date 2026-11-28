#[path = "agent_tools.rs"]
mod agent_tools;

use crate::app::App;
use crate::turns::{ToolCall, ToolSpec};
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub type Args = Map<String, Value>;
pub type ToolResult = Result<String, String>;

pub fn schema(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

pub fn spec(name: &str, description: &str, schema: Value) -> ToolSpec {
    ToolSpec { name: name.to_string(), description: description.to_string(), schema }
}

pub fn text(args: &Args, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn need(args: &Args, key: &str) -> Result<String, String> {
    text(args, key).ok_or_else(|| format!("{key} is needed."))
}

pub fn number(args: &Args, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

pub fn flag(args: &Args, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

pub fn strings(args: &Args, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(list)) => list.iter().filter_map(|v| v.as_str().map(str::to_string).or_else(|| v.as_i64().map(|n| n.to_string()))).collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

pub fn active_space(app: &App) -> Option<String> {
    nixa_store::open(&app.db_file).ok().and_then(|conn| nixa_store::spaces::active_space(&conn).ok().flatten()).filter(|s| !s.is_empty())
}

pub fn specs(app: &App) -> Vec<ToolSpec> {
    let mut out = crate::term_tools::specs();
    out.extend(crate::cli_brains::direct::specs());
    out.extend(agent_tools::specs());
    out.extend(crate::nixa_tools::specs(&app.name()));
    out.extend(crate::tasks::specs());
    out.extend(crate::prompts::tools());
    out.extend(crate::soul::specs(&app.name()));
    out.extend(crate::custom_agents::specs());
    out.extend(crate::git_tools::specs());
    out.extend(crate::project_tools::specs());
    out.extend(crate::browser::specs());
    out.extend(crate::queue::specs());
    out.extend(crate::review_flow::tools());
    out.extend(crate::models::specs());
    out.extend(crate::workflows::specs());
    out.extend(crate::teams::specs());
    out.extend(crate::talk::specs());
    out.extend(crate::modes::specs(&app.name()));
    out.extend(crate::privacy::specs());
    out.extend(crate::extras::specs());
    out.extend(crate::assistant::specs());
    out.extend(crate::app_tools::specs());
    out.extend(crate::cli_tool::specs());
    out.extend(crate::space_tools::specs());
    out.extend(crate::web::specs());
    out.extend(crate::knowledge::specs(&app.name()));
    out.extend(crate::connectors::specs());
    out.extend(crate::desktop::specs());
    out.extend(crate::camera::specs());
    out.extend(crate::drones::tools::specs());
    out.extend(crate::flows::specs());
    out.extend(crate::followup::patches::specs(&app.name()));
    out.extend(crate::manage::specs());
    out.push(crate::actions::tool::spec_of());
    out
}

pub async fn call(app: &Arc<App>, call: &ToolCall) -> (String, bool) {
    let input = crate::modes::journal::DecisionInput {
        kind: "tool".into(),
        choice: call.name.clone(),
        reason: Some(format!("brain tool action: {}", text(&call.args, "action").unwrap_or_else(|| "run".into()))),
        outcome: Some("started".into()),
        ..Default::default()
    };
    let recorded = crate::modes::store(app).and_then(|conn| crate::modes::journal::record(&conn, &input, crate::util::now_ms()));
    let id = match recorded {
        Ok(id) => id,
        Err(error) => return (format!("Tool error: the action could not be journaled: {error}"), true),
    };
    let cancel = crate::app::lock(&app.brain).cancel.clone();
    let result = match cancel {
        Some(cancel) => tokio::select! {
            biased;
            _ = cancel.cancelled() => ("Tool error: stopped by the user.".into(), true),
            result = run(app, call) => result,
        },
        None => run(app, call).await,
    };
    let outcome = if result.1 { "failed or stopped" } else { "completed" };
    if let Err(error) = crate::modes::store(app).and_then(|conn| crate::modes::journal::set_outcome(&conn, id, outcome, None, crate::util::now_ms())) {
        crate::util::log("tool", &format!("could not journal action outcome: {error}"));
    }
    result
}

async fn run(app: &Arc<App>, call: &ToolCall) -> (String, bool) {
    crate::util::log("tool", &call.name);
    if !crate::settings_api::nixa_on(&app.dir) {
        return ("Tool error: Nixa is off. Turn it on in Settings before running tools.".into(), true);
    }
    if let Some(blocked) = crate::privacy::before_tool(app, &call.name, &call.args) {
        return (format!("Tool error: {blocked}"), true);
    }
    let result = if let Some(done) = crate::privacy::call(app, &call.name, &call.args) {
        done
    } else if call.name == crate::cli_brains::direct::NAME {
        crate::cli_brains::direct::call(app, &call.args).await
    } else if call.name.starts_with("nixa_term_") {
        crate::term_tools::call(app, &call.name, &call.args).await
    } else if agent_tools::NAMES.contains(&call.name.as_str()) {
        agent_tools::call(app, &call.name, &call.args).await
    } else if crate::nixa_tools::NAMES.contains(&call.name.as_str()) {
        crate::nixa_tools::call(app, &call.name, &call.args).await
    } else if crate::prompts::tools::NAMES.contains(&call.name.as_str()) {
        crate::prompts::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::soul::NAMES.contains(&call.name.as_str()) {
        crate::soul::call(app, &call.name, &call.args).unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if call.name == crate::tasks::NAME {
        crate::tasks::call(app, &call.args).await
    } else if crate::talk::NAMES.contains(&call.name.as_str()) {
        crate::talk::call(app, &call.name, &call.args).unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if call.name == crate::custom_agents::NAME {
        crate::custom_agents::call(app, &call.args).await
    } else if crate::git_tools::NAMES.contains(&call.name.as_str()) {
        crate::git_tools::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::project_tools::NAMES.contains(&call.name.as_str()) {
        crate::project_tools::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::browser::NAMES.contains(&call.name.as_str()) {
        crate::browser::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::queue::NAMES.contains(&call.name.as_str()) {
        crate::queue::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::review_flow::NAMES.contains(&call.name.as_str()) {
        crate::review_flow::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::models::NAMES.contains(&call.name.as_str()) {
        crate::models::call(app, &call.name, &call.args).await
    } else if call.name == crate::workflows::NAME {
        crate::workflows::call(app, &call.args).await
    } else if call.name == crate::teams::NAME {
        crate::teams::call(app, &call.args).await
    } else if crate::app_tools::NAMES.contains(&call.name.as_str()) {
        crate::app_tools::call(app, &call.name, &call.args).await
    } else if call.name == crate::cli_tool::NAME {
        crate::cli_tool::call(app, &call.args).await
    } else if call.name == crate::space_tools::NAME {
        crate::space_tools::call(app, &call.args).await
    } else if crate::web::NAMES.contains(&call.name.as_str()) {
        crate::web::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::assistant::NAMES.contains(&call.name.as_str()) {
        crate::assistant::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::knowledge::NAMES.contains(&call.name.as_str()) {
        crate::knowledge::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::connectors::NAMES.contains(&call.name.as_str()) {
        crate::connectors::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::modes::NAMES.contains(&call.name.as_str()) {
        crate::modes::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::extras::NAMES.contains(&call.name.as_str()) {
        crate::extras::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if crate::desktop::NAMES.contains(&call.name.as_str()) {
        crate::desktop::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if call.name == crate::camera::NAME {
        crate::camera::call(app, &call.args).await
    } else if call.name == crate::drones::tools::NAME {
        crate::drones::tools::call(app, &call.args).await
    } else if call.name == crate::flows::NAME {
        crate::flows::call(app, &call.args).await
    } else if crate::followup::patches::NAMES.contains(&call.name.as_str()) {
        crate::followup::patches::call(app, &call.args)
    } else if crate::manage::NAMES.contains(&call.name.as_str()) {
        crate::manage::call(app, &call.name, &call.args).await.unwrap_or_else(|| Err(format!("There is no tool named {}.", call.name)))
    } else if call.name == crate::actions::tool::NAME {
        crate::actions::tool::call(app, &call.args).await
    } else {
        Err(format!("There is no tool named {}.", call.name))
    };
    let result = match result {
        Ok(text) if call.name == crate::manage::go_live::NAME
            && serde_json::from_str::<Value>(&text).ok().is_some_and(|v| matches!(v["status"].as_str(), Some("staging" | "activating"))) => {
                crate::manage::go_live::wait_result(app).await
            }
        other => other,
    };
    match &result {
        Ok(text) => crate::extras::after_tool(app, &call.name, &call.args, text, false),
        Err(error) => crate::extras::after_tool(app, &call.name, &call.args, error, true),
    }
    match result {
        Ok(text) => (crate::privacy::after_tool(app, &call.name, &call.args, if text.is_empty() { "Done.".into() } else { text }), false),
        Err(error) => (format!("Tool error: {}", crate::privacy::after_tool(app, &call.name, &call.args, error)), true),
    }
}
