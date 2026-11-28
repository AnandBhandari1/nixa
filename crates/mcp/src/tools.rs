use crate::state::{Mode, State};
use crate::{agents, gateway, models, secrets, tasks, term, trees, window};
use anyhow::{Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub struct Tool {
    pub name: &'static str,
    pub description: String,
    pub input_schema: Value,
    pub worker: bool,
}

pub fn schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

pub fn parse<T: DeserializeOwned>(args: &Value) -> Result<T> {
    let args = if args.is_null() { json!({}) } else { args.clone() };
    serde_json::from_value(args).map_err(|e| anyhow!("Bad arguments: {e}. Nothing was done."))
}

pub fn all() -> Vec<Tool> {
    let mut list = term::tools();
    list.extend(agents::tools());
    list.extend(tasks::tools());
    list.extend(models::tools());
    list.extend(window::tools());
    list.extend(trees::tools());
    list
}

pub fn visible(state: &State) -> Vec<Tool> {
    all()
        .into_iter()
        .filter(|tool| state.mode == Mode::Full || tool.worker)
        .collect()
}

pub fn list_json(state: &State) -> Value {
    if gateway::on() {
        return gateway::list_json(state);
    }
    Value::Array(
        visible(state)
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                })
            })
            .collect(),
    )
}

fn run(state: &State, name: &str, args: &Value) -> Result<Value> {
    if !visible(state).iter().any(|tool| tool.name == name) {
        if all().iter().any(|tool| tool.name == name) {
            bail!("{name} is not allowed in worker mode. The work goes on: put the step you need in your report to the terminal that started you, or ask Nixa to do it with nixa deep ask (MCP: nixa_deep_ask), then go on with your own work.");
        }
        bail!("Unknown tool: {name}");
    }
    term::run(state, name, args)
        .or_else(|| agents::run(state, name, args))
        .or_else(|| tasks::run(state, name, args))
        .or_else(|| models::run(state, name, args))
        .or_else(|| window::run(state, name, args))
        .or_else(|| trees::run(state, name, args))
        .unwrap_or_else(|| Err(anyhow!("Unknown tool: {name}")))
}

fn text_of(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => serde_json::to_string_pretty(&other).unwrap_or_default(),
    }
}

pub fn call(state: &State, name: &str, args: &Value) -> Value {
    if name == gateway::NAME && gateway::on() {
        return gateway::call(state, args);
    }
    let (text, failed) = match run(state, name, args) {
        Ok(value) => (text_of(value), false),
        Err(error) => (format!("{error:#}"), true),
    };
    let reply = json!({
        "content": [{ "type": "text", "text": secrets::hide(&text, &state.secrets) }],
        "isError": failed,
    });
    crate::gaps::note(state, name, &reply);
    reply
}
