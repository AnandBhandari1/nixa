use crate::app::App;
use crate::tools::{Args, ToolResult, need, schema, spec, text};
use crate::turns::ToolSpec;
use serde_json::{Value, json};
use std::sync::Arc;

pub const NAME: &str = "nixa_action";
pub const ACTIONS: [&str; 3] = ["list", "show", "run"];

pub fn spec_of() -> ToolSpec {
    spec(
        NAME,
        "Every Nixa app action in one list, the same ones the Nixa window uses. list shows each action name, kind and one line, or only those that match word. show gives one action with its inputs. run runs the action name with input, a JSON object.",
        schema(
            json!({
                "action": {"type": "string", "enum": ACTIONS},
                "word": {"type": "string", "description": "For list: only actions that match this word."},
                "name": {"type": "string", "description": "Action name from list, like repeat.save. Never invent names."},
                "input": {"type": "object", "description": "For run: the action input, as show describes it."}
            }),
            &["action"],
        ),
    )
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let action = need(args, "action")?;
    match action.as_str() {
        "list" => {
            let list: Vec<Value> = super::matching(text(args, "word").as_deref())
                .iter()
                .map(|a| json!({"name": a.name, "kind": a.kind.word(), "help": a.help}))
                .collect();
            Ok(json!({"actions": list}).to_string())
        }
        "show" => {
            let name = need(args, "name")?;
            super::find(&name).map(|a| a.view().to_string()).ok_or_else(|| format!("No action named {name}. Use list."))
        }
        "run" => {
            let name = need(args, "name")?;
            let input = args.get("input").cloned().unwrap_or(Value::Null);
            if let Some(why) = super::run::user_only(&name, &input) {
                return Ok(json!({"ok": false, "refused": true, "name": name, "error": why}).to_string());
            }
            let (_, out) = super::run::run(app, &name, &input).await;
            let done = out.get("ok").and_then(Value::as_bool) == Some(true) || out.get("refused").and_then(Value::as_bool) == Some(true);
            if done {
                Ok(out.to_string())
            } else {
                Err(out.get("error").and_then(Value::as_str).unwrap_or("The action failed.").to_string())
            }
        }
        other => Err(format!("action {other} is not known. Use one of: {}.", ACTIONS.join(", "))),
    }
}
