use super::{Cmd, Ctx, Kind, send};
use crate::app::{App, lock};
use crate::reply::TextCall;
use crate::turns::{ToolCall, ToolSpec};
use serde_json::Value;
use std::sync::atomic::{AtomicU32, Ordering};

pub const MAX_ROUNDS: u32 = 12;
const RESULT_MAX: usize = 6_000;
const SENTENCE_MAX: usize = 120;

static ROUNDS: AtomicU32 = AtomicU32::new(0);

pub fn uses(app: &App, kind: Kind) -> bool {
    matches!(kind, Kind::Omp | Kind::Omo) || app.cli.saved().text_tools.iter().any(|word| word == kind.word())
}

pub fn reset() {
    ROUNDS.store(0, Ordering::Relaxed);
}

fn first_sentence(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = flat.find(". ").map(|i| i + 1).unwrap_or(flat.len());
    let mut cut: String = flat[..end].chars().take(SENTENCE_MAX).collect();
    if flat[..end].chars().count() > SENTENCE_MAX {
        cut.push('…');
    }
    cut
}

fn params(spec: &ToolSpec) -> String {
    let Some(props) = spec.schema.get("properties").and_then(Value::as_object) else {
        return String::new();
    };
    let required: Vec<&str> = spec.schema.get("required").and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut names: Vec<String> = props.keys().map(|k| if required.contains(&k.as_str()) { k.clone() } else { format!("{k}?") }).collect();
    names.sort();
    names.join(", ")
}

pub fn catalog(specs: &[ToolSpec]) -> String {
    specs.iter().map(|s| format!("- {}({}): {}", s.name, params(s), first_sentence(&s.description))).collect::<Vec<_>>().join("\n")
}

pub fn rule(app: &App) -> String {
    let specs = crate::tools::specs(app);
    format!(
        "You have Nixa tools, called with text. To use tools, reply with only this JSON object and nothing else: {{\"calls\": [{{\"tool\": \"<name>\", \"args\": {{...}}}}]}}. Nixa runs them and sends the results back as the next message. Then call more tools, or give your final answer. Your final answer is only the JSON object {{\"speak\": boolean, \"text\": string, \"heard\": \"me\" | \"unfinished\"}}. Never write a calls object and a final answer together. At most {MAX_ROUNDS} rounds of tools per message. Use only tools from this list (a name ending with ? is optional):\n{}",
        catalog(&specs)
    )
}

fn result_text(index: usize, name: &str, out: &str) -> String {
    let mut body: String = out.chars().take(RESULT_MAX).collect();
    if out.chars().count() > RESULT_MAX {
        body.push_str("\n[cut]");
    }
    format!("{}. {name}:\n{body}", index + 1)
}

pub fn run(ctx: &Ctx, calls: Vec<TextCall>) {
    let round = ROUNDS.fetch_add(1, Ordering::Relaxed) + 1;
    let app = ctx.app.clone();
    if round > MAX_ROUNDS {
        reset();
        let note = "Tool limit reached for this message. Give your final answer now, as the JSON object with speak, text and heard.".to_string();
        crate::agent::set_busy(&app, true);
        if !send(&app, Cmd::Tell(note)) {
            crate::agent::set_busy(&app, false);
        }
        return;
    }
    crate::agent::set_busy(&app, true);
    lock(&app.cli.live).turn_used_tools = true;
    crate::nixa_tools::exposure::saw(&app, ctx.kind.word(), crate::tools::specs(&app).into_iter().map(|s| s.name).collect(), "text tool calls run by Nixa from the brain's reply");
    tokio::spawn(async move {
        let mut parts = Vec::new();
        for (index, call) in calls.iter().enumerate() {
            let tool = ToolCall { id: format!("text-{round}-{index}"), name: call.tool.clone(), args: call.args.clone() };
            let (out, failed) = crate::tools::call(&app, &tool).await;
            let out = if failed && !out.starts_with("Tool error") { format!("Tool error: {out}") } else { out };
            parts.push(result_text(index, &call.tool, &out));
        }
        let message = format!("Tool results:\n{}\n\nCall more tools, or give your final answer.", parts.join("\n\n"));
        if !send(&app, Cmd::Tell(message)) {
            crate::agent::set_busy(&app, false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(name: &str, description: &str, schema: Value) -> ToolSpec {
        ToolSpec { name: name.into(), description: description.into(), schema }
    }

    #[test]
    fn the_catalog_lists_names_arguments_and_one_sentence() {
        let list = vec![
            spec("nixa_term_send", "Send text to a terminal. It waits for the agent.", json!({"properties": {"terminal": {}, "text": {}, "submit": {}}, "required": ["terminal", "text"]})),
            spec("nixa_term_list", "List terminals", json!({"properties": {}})),
        ];
        let text = catalog(&list);
        assert!(text.contains("- nixa_term_list(): List terminals"), "{text}");
        assert!(text.contains("nixa_term_send(submit?, terminal, text): Send text to a terminal."), "{text}");
        assert!(!text.contains("waits for the agent"), "{text}");
    }

    #[test]
    fn results_are_cut() {
        let long = "x".repeat(RESULT_MAX + 50);
        assert!(result_text(0, "t", &long).ends_with("[cut]"));
        assert_eq!(result_text(1, "t", "ok"), "2. t:\nok");
    }
}
