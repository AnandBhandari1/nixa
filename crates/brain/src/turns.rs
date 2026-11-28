use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Turn {
    User {
        text: String,
    },
    Assistant {
        text: String,
        calls: Vec<ToolCall>,
    },
    Tool {
        id: String,
        name: String,
        text: String,
        error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepResult {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
}

pub fn args_of(raw: &Value) -> Map<String, Value> {
    match raw {
        Value::Object(map) => map.clone(),
        Value::String(text) if !text.trim().is_empty() => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        },
        _ => Map::new(),
    }
}

pub fn turn_size(turn: &Turn) -> usize {
    serde_json::to_string(turn).map(|s| s.len()).unwrap_or(0)
}

pub fn turns_size(turns: &[Turn]) -> usize {
    turns.iter().map(turn_size).sum()
}

pub fn trim_turns(turns: &[Turn], max_chars: usize) -> Vec<Turn> {
    let mut kept: &[Turn] = turns;
    while turns_size(kept) > max_chars {
        let next = kept
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, turn)| matches!(turn, Turn::User { .. }))
            .map(|(index, _)| index);
        match next {
            Some(index) => kept = &kept[index..],
            None => break,
        }
    }
    kept.to_vec()
}

pub fn close_calls(turns: &[Turn], note: &str) -> Vec<Turn> {
    let answered: Vec<&str> = turns
        .iter()
        .filter_map(|turn| match turn {
            Turn::Tool { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let mut out = turns.to_vec();
    for turn in turns {
        if let Turn::Assistant { calls, .. } = turn {
            for call in calls {
                if !answered.contains(&call.id.as_str()) {
                    out.push(Turn::Tool {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        text: note.to_string(),
                        error: true,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(text: &str) -> Turn {
        Turn::User { text: text.into() }
    }

    #[test]
    fn parses_args() {
        assert_eq!(args_of(&json!("{\"a\":1}"))["a"], 1);
        assert!(args_of(&json!("nope")).is_empty());
        assert!(args_of(&json!([1])).is_empty());
        assert_eq!(args_of(&json!({"b": 2}))["b"], 2);
    }

    #[test]
    fn trims_from_user_turns() {
        let turns = vec![user(&"a".repeat(100)), user(&"b".repeat(100)), user("c")];
        let kept = trim_turns(&turns, turns_size(&turns[1..]));
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0], turns[1]);
    }

    #[test]
    fn closes_open_calls() {
        let call = ToolCall { id: "c1".into(), name: "x".into(), args: Map::new() };
        let turns = vec![user("hi"), Turn::Assistant { text: String::new(), calls: vec![call] }];
        let closed = close_calls(&turns, "Stopped");
        assert_eq!(closed.len(), 3);
        assert!(matches!(&closed[2], Turn::Tool { id, error: true, .. } if id == "c1"));
        assert_eq!(close_calls(&closed, "Stopped").len(), 3);
    }
}
