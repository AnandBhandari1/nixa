use super::{action, agent_models};
use crate::app::App;
use crate::tools::{Args, ToolResult, schema, spec, text};
use crate::turns::ToolSpec;
use nixa_store::models::lists::LIST_AGENTS;
use nixa_store::models::rank::COSTS;
use nixa_store::models::act::{AGENT_MODELS_ABOUT, MODELS_ABOUT};
use nixa_store::models::{ACTIONS, By, CAPABILITIES};
use serde_json::{Value, json};
use std::sync::Arc;

pub const MODELS: &str = "nixa_models";
pub const AGENT_MODELS: &str = "nixa_list_agent_models";
pub const NAMES: [&str; 2] = [MODELS, AGENT_MODELS];

pub fn specs() -> Vec<ToolSpec> {
    let caps = json!({"type": "array", "items": {"type": "string", "enum": CAPABILITIES}});
    let props = json!({
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
        "plan": {"type": "string", "minLength": 1, "maxLength": 80}
    });
    let list_props = json!({
        "agent": {"type": "string", "enum": LIST_AGENTS},
        "refresh": {"type": "boolean", "description": "Read the list from the agent now instead of the saved list"}
    });
    vec![spec(MODELS, MODELS_ABOUT, schema(props, &[])), spec(AGENT_MODELS, AGENT_MODELS_ABOUT, schema(list_props, &["agent"]))]
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

pub async fn call(app: &Arc<App>, name: &str, args: &Args) -> ToolResult {
    match name {
        MODELS => action(app, &Value::Object(args.clone()), By::Nixa).await.map(|v| pretty(&v)),
        AGENT_MODELS => {
            let extra: Vec<&String> = args.keys().filter(|k| !matches!(k.as_str(), "agent" | "refresh")).collect();
            if !extra.is_empty() {
                return Err(format!("Unknown arguments: {}. Nothing was done.", extra.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")));
            }
            let agent = text(args, "agent").ok_or("agent is needed.")?;
            let refresh = args.get("refresh").and_then(Value::as_bool).unwrap_or(false);
            agent_models(app, &agent, refresh).await.map(|v| pretty(&v))
        }
        _ => Err(format!("There is no tool named {name}.")),
    }
}
