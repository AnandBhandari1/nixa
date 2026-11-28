use crate::provider::Model;
use crate::turns::{StepResult, ToolCall, ToolSpec, Turn, Usage, args_of};
use serde_json::{Map, Value, json};

pub const VERSION: &str = "2023-06-01";
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub fn is_official(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = rest.split(['/', ':', '?']).next().unwrap_or("");
    host == "api.anthropic.com"
}

pub fn messages(turns: &[Turn]) -> Vec<Value> {
    let mut out: Vec<(String, Vec<Value>)> = Vec::new();
    let mut push = |role: &str, blocks: Vec<Value>| {
        if blocks.is_empty() {
            return;
        }
        match out.last_mut() {
            Some((last, list)) if last == role => list.extend(blocks),
            _ => out.push((role.to_string(), blocks)),
        }
    };
    for turn in turns {
        match turn {
            Turn::User { text } => push("user", vec![json!({"type": "text", "text": text})]),
            Turn::Tool { id, text, error, .. } => push("user", vec![json!({"type": "tool_result", "tool_use_id": id, "content": text, "is_error": error})]),
            Turn::Assistant { text, calls } => {
                let mut blocks = Vec::new();
                if !text.trim().is_empty() {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                for call in calls {
                    blocks.push(json!({"type": "tool_use", "id": call.id, "name": call.name, "input": Value::Object(call.args.clone())}));
                }
                push("assistant", blocks);
            }
        }
    }
    out.into_iter().map(|(role, content)| json!({"role": role, "content": content})).collect()
}

pub fn tools(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            let mut schema = tool.schema.as_object().cloned().unwrap_or_default();
            schema.insert("type".into(), json!("object"));
            json!({"name": tool.name, "description": tool.description, "input_schema": Value::Object(schema)})
        })
        .collect()
}

pub struct BodyInput<'a> {
    pub system: &'a str,
    pub turns: &'a [Turn],
    pub tools: &'a [ToolSpec],
    pub model: &'a str,
    pub effort: Option<&'a str>,
    pub max_tokens: u32,
    pub stream: bool,
}

pub fn body(input: &BodyInput) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(input.model));
    body.insert("max_tokens".into(), json!(input.max_tokens));
    body.insert("system".into(), json!(input.system));
    body.insert("messages".into(), Value::Array(messages(input.turns)));
    if let Some(effort) = input.effort.filter(|e| EFFORTS.contains(e)) {
        body.insert("output_config".into(), json!({"effort": effort}));
    }
    if !input.tools.is_empty() {
        body.insert("tools".into(), Value::Array(tools(input.tools)));
    }
    if input.stream {
        body.insert("stream".into(), json!(true));
    }
    Value::Object(body)
}

pub fn usage_of(usage: &Value) -> Option<Usage> {
    let input = usage.get("input_tokens").and_then(Value::as_u64);
    let output = usage.get("output_tokens").and_then(Value::as_u64);
    let cached = usage.get("cache_read_input_tokens").and_then(Value::as_u64);
    if input.is_none() && output.is_none() && cached.is_none() {
        return None;
    }
    Some(Usage { input: input.unwrap_or(0), output: output.unwrap_or(0), cached: cached.unwrap_or(0) })
}

pub fn from_content(content: &[Value]) -> StepResult {
    let text: Vec<&str> = content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect();
    let calls = content
        .iter()
        .enumerate()
        .filter(|(_, block)| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|(index, block)| {
            let name = block.get("name").and_then(Value::as_str).filter(|n| !n.is_empty())?;
            let id = block.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("toolu_{index}"));
            Some(ToolCall { id, name: name.to_string(), args: args_of(block.get("input").unwrap_or(&Value::Null)) })
        })
        .collect();
    StepResult { text: text.join("\n"), calls, usage: None }
}

#[derive(Debug, Default)]
struct Block {
    kind: String,
    text: String,
    id: String,
    name: String,
    json: String,
}

#[derive(Debug, Default)]
pub struct Stream {
    blocks: Vec<(u64, Block)>,
    usage: Usage,
    seen_usage: bool,
}

impl Stream {
    fn block(&mut self, index: u64) -> &mut Block {
        let at = match self.blocks.iter().position(|(i, _)| *i == index) {
            Some(at) => at,
            None => {
                self.blocks.push((index, Block::default()));
                self.blocks.len() - 1
            }
        };
        &mut self.blocks[at].1
    }

    pub fn feed(&mut self, data: &str) -> Result<Option<String>, String> {
        let value: Value = serde_json::from_str(data.trim()).map_err(|e| format!("bad stream data: {e}"))?;
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
        match kind {
            "error" => {
                let message = value.pointer("/error/message").and_then(Value::as_str).unwrap_or("stream error");
                Err(message.to_string())
            }
            "message_start" => {
                if let Some(usage) = value.pointer("/message/usage").and_then(usage_of) {
                    self.usage.input = usage.input;
                    self.usage.cached = usage.cached;
                    self.usage.output = self.usage.output.max(usage.output);
                    self.seen_usage = true;
                }
                Ok(None)
            }
            "message_delta" => {
                if let Some(output) = value.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                    self.usage.output = output;
                    self.seen_usage = true;
                }
                Ok(None)
            }
            "content_block_start" => {
                let start = value.get("content_block").cloned().unwrap_or(Value::Null);
                let block = self.block(index);
                block.kind = start.get("type").and_then(Value::as_str).unwrap_or("").to_string();
                block.id = start.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                block.name = start.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let text = start.get("text").and_then(Value::as_str).unwrap_or("").to_string();
                block.text.push_str(&text);
                Ok(if text.is_empty() { None } else { Some(text) })
            }
            "content_block_delta" => {
                let delta = value.get("delta").cloned().unwrap_or(Value::Null);
                let block = self.block(index);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let text = delta.get("text").and_then(Value::as_str).unwrap_or("").to_string();
                        block.text.push_str(&text);
                        Ok(if text.is_empty() { None } else { Some(text) })
                    }
                    Some("input_json_delta") => {
                        block.json.push_str(delta.get("partial_json").and_then(Value::as_str).unwrap_or(""));
                        Ok(None)
                    }
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    pub fn finish(mut self) -> StepResult {
        self.blocks.sort_by_key(|(index, _)| *index);
        let text: Vec<String> = self.blocks.iter().filter(|(_, b)| b.kind == "text").map(|(_, b)| b.text.clone()).collect();
        let calls = self
            .blocks
            .iter()
            .filter(|(_, b)| b.kind == "tool_use" && !b.name.is_empty())
            .map(|(index, b)| ToolCall {
                id: if b.id.is_empty() { format!("toolu_{index}") } else { b.id.clone() },
                name: b.name.clone(),
                args: args_of(&Value::String(if b.json.trim().is_empty() { "{}".into() } else { b.json.clone() })),
            })
            .collect();
        StepResult { text: text.join("\n"), calls, usage: self.seen_usage.then_some(self.usage) }
    }
}

pub fn models_of(body: &Value) -> Vec<Model> {
    body.get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(Value::as_str)?;
                    let name = item.get("display_name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(id);
                    let effort = item.pointer("/capabilities/effort");
                    let supported = effort.and_then(|e| e.get("supported")).and_then(Value::as_bool) == Some(true);
                    let efforts = if supported {
                        EFFORTS
                            .iter()
                            .filter(|level| effort.and_then(|e| e.get(**level)).and_then(|l| l.get("supported")).and_then(Value::as_bool) == Some(true))
                            .map(|s| s.to_string())
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let max_tokens = item.get("max_tokens").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0);
                    Some(Model { id: id.to_string(), name: name.to_string(), description: String::new(), efforts, max_tokens })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_keep_their_output_limit() {
        let body = serde_json::json!({"data": [{"id": "a", "max_tokens": 128000}, {"id": "b", "max_tokens": null}, {"id": "c"}]});
        let models = models_of(&body);
        assert_eq!(crate::provider::limit_of(&models, "a"), Some(128_000));
        assert_eq!(crate::provider::limit_of(&models, "b"), None);
        assert_eq!(crate::provider::limit_of(&models, "c"), None);
        assert_eq!(crate::provider::limit_of(&models, "d"), None);
    }

    #[test]
    fn merges_tool_results_into_user_turns() {
        let turns = vec![
            Turn::User { text: "hi".into() },
            Turn::Assistant { text: " ".into(), calls: vec![ToolCall { id: "u1".into(), name: "t".into(), args: Map::new() }] },
            Turn::Tool { id: "u1".into(), name: "t".into(), text: "ok".into(), error: false },
            Turn::User { text: "more".into() },
        ];
        let out = messages(&turns);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1]["content"].as_array().unwrap().len(), 1);
        assert_eq!(out[2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(out[2]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn streams_blocks() {
        let mut s = Stream::default();
        s.feed(r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":4,"output_tokens":1}}}"#).unwrap();
        s.feed(r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#).unwrap();
        assert_eq!(s.feed(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#).unwrap(), Some("Hi".into()));
        s.feed(r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"nixa_chat"}}"#).unwrap();
        s.feed(r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"action\":"}}"#).unwrap();
        s.feed(r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"list\"}"}}"#).unwrap();
        s.feed(r#"{"type":"message_delta","delta":{},"usage":{"output_tokens":7}}"#).unwrap();
        let result = s.finish();
        assert_eq!(result.text, "Hi");
        assert_eq!(result.calls[0].args["action"], "list");
        assert_eq!(result.usage, Some(Usage { input: 10, output: 7, cached: 4 }));
    }

    #[test]
    fn body_has_effort_only_when_known() {
        let input = BodyInput { system: "s", turns: &[], tools: &[], model: "m", effort: Some("high"), max_tokens: 10, stream: true };
        assert_eq!(body(&input)["output_config"]["effort"], "high");
        let input = BodyInput { effort: Some("turbo"), ..input };
        assert!(body(&input).get("output_config").is_none());
    }

    #[test]
    fn knows_official_host() {
        assert!(is_official("https://api.anthropic.com"));
        assert!(!is_official("https://proxy.example.com/api.anthropic.com"));
    }

    #[test]
    fn reads_models_with_efforts() {
        let body = json!({"data": [{"id": "m1", "display_name": "M One", "capabilities": {"effort": {"supported": true, "low": {"supported": true}, "max": {"supported": false}}}}]});
        let models = models_of(&body);
        assert_eq!(models[0].name, "M One");
        assert_eq!(models[0].efforts, vec!["low".to_string()]);
    }
}
