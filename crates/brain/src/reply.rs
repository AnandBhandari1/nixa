use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const REPLY_RULE: &str = "Reply with only the JSON object {\"speak\": boolean, \"text\": string, \"heard\": \"me\" | \"unfinished\"}. No other words, no code fences. Use the tools when you need them, then give the JSON object as your last message.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentReply {
    #[serde(default)]
    pub calls: Vec<TextCall>,
    pub speak: bool,
    pub text: String,
    pub heard: String,
    #[serde(default)]
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextCall {
    pub tool: String,
    #[serde(default)]
    pub args: serde_json::Map<String, Value>,
}

pub fn calls_of(value: &serde_json::Map<String, Value>) -> Vec<TextCall> {
    let list = match value.get("calls") {
        Some(Value::Array(list)) => list.clone(),
        _ => Vec::new(),
    };
    list.into_iter()
        .filter_map(|row| {
            let row = row.as_object()?;
            let tool = row.get("tool").or_else(|| row.get("name")).and_then(Value::as_str)?.trim().to_string();
            let args = row.get("args").or_else(|| row.get("arguments")).and_then(Value::as_object).cloned().unwrap_or_default();
            (!tool.is_empty()).then_some(TextCall { tool, args })
        })
        .collect()
}

impl AgentReply {
    pub fn failed(text: impl Into<String>) -> AgentReply {
        AgentReply { calls: Vec::new(), speak: true, text: text.into(), heard: "me".into(), failed: true }
    }

    pub fn said(text: impl Into<String>) -> AgentReply {
        AgentReply { calls: Vec::new(), speak: true, text: text.into(), heard: "me".into(), failed: false }
    }
}

pub fn extract_json(text: &str) -> String {
    let mut trimmed = text.trim();
    for fence in ["```json", "```JSON", "```"] {
        if let Some(rest) = trimmed.strip_prefix(fence) {
            trimmed = rest.trim_start();
            break;
        }
    }
    if let Some(rest) = trimmed.strip_suffix("```") {
        trimmed = rest.trim_end();
    }
    match (trimmed.find('{'), trimmed.rfind('}')) {
        (Some(start), Some(end)) if end > start => trimmed[start..=end].to_string(),
        _ => trimmed.to_string(),
    }
}

pub fn parse_reply(text: &str) -> AgentReply {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(value)) => {
            let said = value.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let speak = value.get("speak").and_then(Value::as_bool) == Some(true) && !said.is_empty();
            let heard = if value.get("heard").and_then(Value::as_str) == Some("unfinished") { "unfinished" } else { "me" };
            AgentReply { calls: calls_of(&value), speak, text: said, heard: heard.into(), failed: false }
        }
        _ => {
            let said = text.trim().to_string();
            AgentReply { calls: Vec::new(), speak: !said.is_empty(), text: said, heard: "me".into(), failed: false }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_json_reply() {
        let reply = parse_reply(&extract_json("```json\n{\"speak\": true, \"text\": \" Hi \", \"heard\": \"unfinished\"}\n```"));
        assert!(reply.speak);
        assert_eq!(reply.text, "Hi");
        assert_eq!(reply.heard, "unfinished");
    }

    #[test]
    fn reads_text_tool_calls() {
        let reply = parse_reply(&extract_json("{\"calls\": [{\"tool\": \"nixa_term_list\", \"args\": {}}, {\"name\": \"nixa_task_list\", \"arguments\": {\"status\": \"todo\"}}, {\"args\": {}}]}"));
        assert_eq!(reply.calls.len(), 2);
        assert_eq!(reply.calls[0].tool, "nixa_term_list");
        assert_eq!(reply.calls[1].args.get("status").and_then(Value::as_str), Some("todo"));
        assert!(parse_reply("{\"speak\": true, \"text\": \"Hi\"}").calls.is_empty());
    }

    #[test]
    fn plain_text_is_spoken() {
        let reply = parse_reply("Just words");
        assert!(reply.speak);
        assert_eq!(reply.text, "Just words");
        let quiet = parse_reply("{\"speak\": false, \"text\": \"x\"}");
        assert!(!quiet.speak);
    }
}
