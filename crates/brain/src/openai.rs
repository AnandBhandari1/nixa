use crate::turns::{StepResult, ToolCall, ToolSpec, Turn, Usage, args_of};
use serde_json::{Map, Value, json};

pub fn messages(system: &str, turns: &[Turn]) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": system})];
    for turn in turns {
        out.push(match turn {
            Turn::User { text } => json!({"role": "user", "content": text}),
            Turn::Tool { id, text, .. } => json!({"role": "tool", "tool_call_id": id, "content": text}),
            Turn::Assistant { text, calls } if calls.is_empty() => json!({"role": "assistant", "content": text}),
            Turn::Assistant { text, calls } => json!({
                "role": "assistant",
                "content": if text.is_empty() { Value::Null } else { Value::String(text.clone()) },
                "tool_calls": calls.iter().map(|call| json!({
                    "id": call.id,
                    "type": "function",
                    "function": {"name": call.name, "arguments": Value::Object(call.args.clone()).to_string()},
                })).collect::<Vec<_>>(),
            }),
        });
    }
    out
}

pub fn tools(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| json!({"type": "function", "function": {"name": tool.name, "description": tool.description, "parameters": tool.schema}}))
        .collect()
}

pub fn body(system: &str, turns: &[Turn], specs: &[ToolSpec], model: &str, stream: bool) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("messages".into(), Value::Array(messages(system, turns)));
    if !specs.is_empty() {
        body.insert("tools".into(), Value::Array(tools(specs)));
    }
    if stream {
        body.insert("stream".into(), json!(true));
        body.insert("stream_options".into(), json!({"include_usage": true}));
    }
    Value::Object(body)
}

pub fn usage_of(usage: &Value) -> Option<Usage> {
    let input = usage.get("prompt_tokens").and_then(Value::as_u64);
    let output = usage.get("completion_tokens").and_then(Value::as_u64);
    let cached = usage.pointer("/prompt_tokens_details/cached_tokens").and_then(Value::as_u64);
    if input.is_none() && output.is_none() && cached.is_none() {
        return None;
    }
    Some(Usage { input: input.unwrap_or(0), output: output.unwrap_or(0), cached: cached.unwrap_or(0) })
}

pub fn from_message(message: &Value) -> StepResult {
    let text = message.get("content").and_then(Value::as_str).unwrap_or("").to_string();
    let calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .enumerate()
                .filter_map(|(index, call)| {
                    let kind = call.get("type").and_then(Value::as_str).unwrap_or("function");
                    let name = call.pointer("/function/name").and_then(Value::as_str)?;
                    if kind != "function" || name.is_empty() {
                        return None;
                    }
                    let id = call.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("call_{index}"));
                    let args = args_of(call.pointer("/function/arguments").unwrap_or(&Value::Null));
                    Some(ToolCall { id, name: name.to_string(), args })
                })
                .collect()
        })
        .unwrap_or_default();
    StepResult { text, calls, usage: None }
}

#[derive(Debug, Default)]
struct Partial {
    id: String,
    name: String,
    args: String,
}

#[derive(Debug, Default)]
pub struct Stream {
    text: String,
    calls: Vec<(u64, Partial)>,
    usage: Option<Usage>,
    done: bool,
}

impl Stream {
    pub fn feed(&mut self, data: &str) -> Result<Option<String>, String> {
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return Ok(None);
        }
        let value: Value = serde_json::from_str(data).map_err(|e| format!("bad stream data: {e}"))?;
        if let Some(error) = value.get("error") {
            let message = error.get("message").and_then(Value::as_str).unwrap_or("stream error");
            return Err(message.to_string());
        }
        if let Some(usage) = value.get("usage").filter(|u| !u.is_null()) {
            self.usage = usage_of(usage).or(self.usage);
        }
        let Some(delta) = value.pointer("/choices/0/delta") else { return Ok(None) };
        if let Some(list) = delta.get("tool_calls").and_then(Value::as_array) {
            for (position, call) in list.iter().enumerate() {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(position as u64);
                let slot = match self.calls.iter().position(|(i, _)| *i == index) {
                    Some(at) => at,
                    None => {
                        self.calls.push((index, Partial::default()));
                        self.calls.len() - 1
                    }
                };
                let part = &mut self.calls[slot].1;
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    part.id.push_str(id);
                }
                if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                    part.name.push_str(name);
                }
                if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                    part.args.push_str(args);
                }
            }
        }
        match delta.get("content").and_then(Value::as_str) {
            Some(text) if !text.is_empty() => {
                self.text.push_str(text);
                Ok(Some(text.to_string()))
            }
            _ => Ok(None),
        }
    }

    pub fn finish(self) -> StepResult {
        let calls = self
            .calls
            .into_iter()
            .enumerate()
            .filter(|(_, (_, part))| !part.name.is_empty())
            .map(|(index, (_, part))| ToolCall {
                id: if part.id.is_empty() { format!("call_{index}") } else { part.id },
                name: part.name,
                args: args_of(&Value::String(part.args)),
            })
            .collect();
        StepResult { text: self.text, calls, usage: self.usage }
    }
}

pub fn models_of(body: &Value) -> Vec<crate::provider::Model> {
    let mut out: Vec<crate::provider::Model> = body
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(Value::as_str)?;
                    let owner = item.get("owned_by").and_then(Value::as_str).unwrap_or("");
                    Some(crate::provider::Model {
                        id: id.to_string(),
                        name: id.to_string(),
                        description: if owner.is_empty() { String::new() } else { format!("by {owner}") },
                        efforts: Vec::new(),
                        max_tokens: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_messages_with_calls() {
        let mut args = Map::new();
        args.insert("a".into(), json!(1));
        let turns = vec![
            Turn::User { text: "hi".into() },
            Turn::Assistant { text: String::new(), calls: vec![ToolCall { id: "c1".into(), name: "t".into(), args }] },
            Turn::Tool { id: "c1".into(), name: "t".into(), text: "ok".into(), error: false },
        ];
        let out = messages("sys", &turns);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0]["role"], "system");
        assert!(out[2]["content"].is_null());
        assert_eq!(out[2]["tool_calls"][0]["function"]["arguments"], "{\"a\":1}");
        assert_eq!(out[3]["tool_call_id"], "c1");
    }

    #[test]
    fn streams_text_and_calls() {
        let mut stream = Stream::default();
        assert_eq!(stream.feed(r#"{"choices":[{"delta":{"content":"He"}}]}"#).unwrap(), Some("He".into()));
        stream.feed(r#"{"choices":[{"delta":{"content":"y"}}]}"#).unwrap();
        stream.feed(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c9","function":{"name":"nixa_term_list","arguments":"{\"a\""}}]}}]}"#).unwrap();
        stream.feed(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":2}"}}]}}]}"#).unwrap();
        stream.feed(r#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":1}}}"#).unwrap();
        stream.feed("[DONE]").unwrap();
        let result = stream.finish();
        assert_eq!(result.text, "Hey");
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].id, "c9");
        assert_eq!(result.calls[0].args["a"], 2);
        assert_eq!(result.usage, Some(Usage { input: 5, output: 3, cached: 1 }));
    }

    #[test]
    fn stream_error_is_reported() {
        let mut stream = Stream::default();
        assert!(stream.feed(r#"{"error":{"message":"bad key"}}"#).is_err());
    }

    #[test]
    fn reads_plain_message() {
        let result = from_message(&json!({"content": "x", "tool_calls": [{"function": {"name": "t", "arguments": "{}"}}]}));
        assert_eq!(result.calls[0].id, "call_0");
    }
}
