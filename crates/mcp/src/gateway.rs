use crate::state::State;
use crate::tools::{self, Tool, schema};
use serde_json::{Map, Value, json};

pub const ENV_GATEWAY: &str = "NIXA_MCP_GATEWAY";
pub const NAME: &str = crate::bridge::GATEWAY_TOOL;
pub const HELP: &str = "help";

pub fn on() -> bool {
    std::env::var(ENV_GATEWAY).is_ok_and(|v| v.trim() == "1")
}

pub fn tool() -> Tool {
    Tool {
        name: NAME,
        description: format!(
            "Run any Nixa tool by name, with its arguments in args. Tool \"{HELP}\" lists every tool with its arguments."
        ),
        input_schema: schema(
            json!({
                "tool": { "type": "string", "description": "Tool name, or help for the list" },
                "args": { "type": "object", "description": "Arguments for that tool", "additionalProperties": true },
            }),
            &["tool"],
        ),
        worker: true,
    }
}

pub fn list_json(state: &State) -> Value {
    let mut list = tools::visible(state);
    list.push(tool());
    Value::Array(
        list.iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": t.input_schema }))
            .collect(),
    )
}

fn row(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn type_text(spec: &Value) -> String {
    if let Some(list) = spec.get("enum").and_then(Value::as_array) {
        return list.iter().map(Value::to_string).collect::<Vec<_>>().join("|");
    }
    match spec.get("type") {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("|"),
        Some(Value::String(kind)) if kind == "array" => {
            let item = spec.get("items").map(type_text).filter(|t| !t.is_empty());
            format!("{}[]", item.unwrap_or_else(|| "item".into()))
        }
        Some(Value::String(kind)) => kind.clone(),
        _ => spec
            .get("anyOf")
            .and_then(Value::as_array)
            .map(|list| list.iter().map(type_text).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("|"))
            .unwrap_or_default(),
    }
}

pub fn args_text(spec: &Value) -> String {
    let required: Vec<&str> = spec
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut props: Vec<(String, Value)> = row(spec.get("properties").unwrap_or(&Value::Null)).into_iter().collect();
    props.sort_by(|a, b| a.0.cmp(&b.0));
    props
        .iter()
        .map(|(key, value)| {
            let mark = if required.contains(&key.as_str()) { "" } else { "?" };
            let kind = type_text(value);
            format!("{key}{mark}: {}", if kind.is_empty() { "any".into() } else { kind })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn first_sentence(text: &str) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let bytes = one.as_bytes();
    for (at, b) in bytes.iter().enumerate() {
        if matches!(b, b'.' | b'!' | b'?') && bytes.get(at + 1).is_none_or(|n| *n == b' ') {
            return one[..=at].to_string();
        }
    }
    one
}

pub fn help_text(list: &[Tool]) -> String {
    list.iter()
        .map(|t| format!("{}({}): {}", t.name, args_text(&t.input_schema), first_sentence(&t.description)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn reply(text: String, failed: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": failed })
}

pub fn call(state: &State, args: &Value) -> Value {
    let args = row(args);
    let target = args.get("tool").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let list = tools::visible(state);
    if target.is_empty() || target == HELP {
        return reply(help_text(&list), false);
    }
    if target == NAME {
        return reply(format!("{NAME} cannot call itself."), true);
    }
    if !list.iter().any(|t| t.name == target) {
        return reply(
            format!("There is no Nixa tool named {target}. Call {NAME} with tool \"{HELP}\" to see the list."),
            true,
        );
    }
    let inner = args.get("args").cloned().unwrap_or_else(|| json!({}));
    tools::call(state, &target, &inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Mode;
    use std::path::PathBuf;

    fn state(mode: Mode) -> State {
        State::new(mode, PathBuf::from("/nonexistent/mg.sock"), PathBuf::from("/nonexistent"), None)
    }

    fn text(value: &Value) -> String {
        value["content"][0]["text"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn args_and_sentences_are_short() {
        let spec = json!({"properties": {"terminal": {"type": ["string", "integer"]}, "keys": {"type": "array", "items": {"type": "string"}}, "place": {"enum": ["tab", "right"]}, "x": {}}, "required": ["terminal"]});
        assert_eq!(args_text(&spec), "keys?: string[], place?: \"tab\"|\"right\", terminal: string|integer, x?: any");
        assert_eq!(first_sentence("Run it now.  Then   more. Last"), "Run it now.");
        assert_eq!(first_sentence("v1.2 is out"), "v1.2 is out");
    }

    #[test]
    fn help_lists_tools_and_bad_names_are_errors() {
        let s = state(Mode::Full);
        let help = call(&s, &json!({"tool": "help"}));
        assert_eq!(help["isError"], false);
        assert!(text(&help).contains("nixa_term_list(label?: string, parent?: string): List Nixa terminals"));
        assert!(text(&call(&s, &json!({}))).contains("nixa_app_summary()"));
        assert_eq!(call(&s, &json!({"tool": NAME}))["isError"], true);
        let none = call(&s, &json!({"tool": "nixa_nope"}));
        assert!(text(&none).contains("There is no Nixa tool named nixa_nope"));
    }

    #[test]
    fn calls_pass_through_and_keep_worker_limits() {
        let worker = state(Mode::Worker);
        let blocked = call(&worker, &json!({"tool": "nixa_term_send", "args": {"terminal": "m1", "message": "hi"}}));
        assert_eq!(blocked["isError"], true);
        assert!(!text(&call(&worker, &json!({"tool": "help"}))).contains("nixa_term_send"));
        let passed = call(&worker, &json!({"tool": "nixa_term_get", "args": {"terminal": "nope"}}));
        assert!(text(&passed).contains("is not a terminal id"));
        let listed = list_json(&worker);
        let names: Vec<&str> = listed.as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names.last(), Some(&NAME));
        assert!(names.contains(&"nixa_app_summary"));
        assert!(!names.contains(&"nixa_app_run"));
    }
}
