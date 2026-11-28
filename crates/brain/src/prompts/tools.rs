use super::answer::answer_by_tool;
use super::book::{Answer, Choice};
use super::card::describe;
use crate::app::{App, lock};
use crate::tools::{Args, ToolResult, flag, need, schema, spec, text};
use crate::turns::ToolSpec;
use serde_json::{Value, json};
use std::sync::Arc;

pub const LIST: &str = "nixa_prompts";
pub const ANSWER: &str = "nixa_answer_prompt";
pub const NAMES: [&str; 2] = [LIST, ANSWER];

pub fn tools() -> Vec<ToolSpec> {
    vec![
        spec(LIST, "List the open agent prompts (questions, permissions and plans) with their ids and options.", schema(json!({}), &[])),
        spec(
            ANSWER,
            "Answer an open agent prompt (question, permission or plan) with what the user said. Use choice yes, always or no for permissions and plans. Use option numbers (1 is the first option) or text for questions. A plan rejection may carry text feedback. When the prompt event says you decide it yourself, read the terminal screen, the job and every option first, never blindly pick option 1 or the one marked Recommended, then call this with decide true and a short reason (ask nixa_deep_ask first if unclear). When the user answered the prompt, use their answer without decide.",
            schema(
                json!({
                    "id": {"type": "string", "description": "Prompt id, like p3"},
                    "choice": {"type": "string", "enum": ["yes", "always", "no"]},
                    "options": {"type": "array", "items": {"type": "integer", "minimum": 1}},
                    "text": {"type": "string"},
                    "decide": {"type": "boolean", "description": "true only when the prompt event lets you decide it yourself"},
                    "reason": {"type": "string", "description": "Needed with decide true: one short reason why this option fits best"}
                }),
                &["id"],
            ),
        ),
    ]
}

pub fn list_text(app: &App) -> String {
    let book = lock(&app.prompts.book);
    if book.items.is_empty() {
        return "No open prompts.".to_string();
    }
    let lines: Vec<String> = book.items.iter().map(|p| format!("- {}", describe(p))).collect();
    format!("Open prompts:\n{}", lines.join("\n"))
}

pub fn answer_of(args: &Args) -> Result<Answer, String> {
    let choice = match text(args, "choice") {
        Some(word) => Some(Choice::parse(&word).ok_or("choice is yes, always or no.")?),
        None => None,
    };
    let options = match args.get("options") {
        Some(Value::Array(list)) => list.iter().map(|v| v.as_u64().filter(|n| *n >= 1).map(|n| n as usize).ok_or("options are numbers from 1.")).collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("options is a list of numbers.".into()),
        None => Vec::new(),
    };
    Ok(Answer { choice, options, text: text(args, "text") })
}

pub async fn call(app: &Arc<App>, name: &str, args: &Args) -> Option<ToolResult> {
    match name {
        LIST => Some(Ok(list_text(app))),
        ANSWER => {
            let run = async {
                let id = need(args, "id")?;
                let given = answer_of(args)?;
                let link = app.prompts.link();
                answer_by_tool(app, &link, &id, given, flag(args, "decide") == Some(true), text(args, "reason")).await
            };
            Some(run.await)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_answers() {
        let args: Args = serde_json::from_value(json!({"id": "p1", "choice": "Yes", "options": [1, 3], "text": " hi "})).unwrap();
        let a = answer_of(&args).unwrap();
        assert_eq!(a.choice, Some(Choice::Yes));
        assert_eq!(a.options, vec![1, 3]);
        assert_eq!(a.text.as_deref(), Some("hi"));
        let bad: Args = serde_json::from_value(json!({"options": [0]})).unwrap();
        assert!(answer_of(&bad).is_err());
        let bad: Args = serde_json::from_value(json!({"choice": "maybe"})).unwrap();
        assert!(answer_of(&bad).is_err());
        assert_eq!(tools().len(), 2);
    }
}
