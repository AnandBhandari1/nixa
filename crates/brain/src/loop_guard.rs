use crate::util::short_hash;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

pub const MAX_IDENTICAL: usize = 3;
pub const TOOL_BUDGET: usize = 40;
pub const WINDOW: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Stop { warn: bool, reason: String },
}

#[derive(Debug, Default)]
pub struct LoopGuard {
    counts: HashMap<String, usize>,
    per_tool: HashMap<String, usize>,
    sequence: Vec<String>,
    flagged: HashSet<String>,
}

pub fn call_key(name: &str, args: &Map<String, Value>) -> String {
    let body = serde_json::to_string(args).unwrap_or_default();
    short_hash(&format!("{name}:{body}"))
}

fn cycle_of(sequence: &[String]) -> Option<String> {
    for period in [2usize, 3] {
        if sequence.len() < period * 2 {
            continue;
        }
        let tail = &sequence[sequence.len() - period * 2..];
        if tail.iter().enumerate().all(|(index, key)| *key == tail[index % period]) {
            let mut head: Vec<&String> = tail[..period].iter().collect();
            head.sort();
            head.dedup();
            let joined: Vec<&str> = head.iter().map(|s| s.as_str()).collect();
            return Some(format!("{period}:{}", joined.join(",")));
        }
    }
    None
}

impl LoopGuard {
    pub fn reset(&mut self) {
        *self = LoopGuard::default();
    }

    pub fn check(&mut self, name: &str, args: &Map<String, Value>) -> Verdict {
        let key = call_key(name, args);
        let same = self.counts.get(&key).copied().unwrap_or(0) + 1;
        self.counts.insert(key.clone(), same);
        let ran = self.per_tool.get(name).copied().unwrap_or(0);
        self.sequence.push(key.clone());
        if self.sequence.len() > WINDOW {
            self.sequence.remove(0);
        }
        let problem = if same > MAX_IDENTICAL {
            Some((format!("same:{key}"), format!("{name} was called {same} times with the same input.")))
        } else if ran + 1 > TOOL_BUDGET {
            Some((format!("budget:{name}"), format!("{name} was called {} times this turn, over the limit of {TOOL_BUDGET}.", ran + 1)))
        } else {
            cycle_of(&self.sequence).map(|cycle| (format!("cycle:{cycle}"), "The last tool calls repeat in a cycle.".to_string()))
        };
        match problem {
            None => {
                self.per_tool.insert(name.to_string(), ran + 1);
                Verdict::Ok
            }
            Some((id, reason)) => {
                let warn = self.flagged.insert(id);
                Verdict::Stop { warn, reason }
            }
        }
    }
}

pub fn loop_note(warn: bool, reason: &str) -> String {
    if warn {
        format!("{reason} This call was not run. Do not repeat it. Answer with what you have, or try a different step.")
    } else {
        format!("Blocked: {reason} This call was not run again. Answer with what you have.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn stops_same_calls() {
        let mut guard = LoopGuard::default();
        let a = args(json!({"x": 1}));
        for _ in 0..MAX_IDENTICAL {
            assert_eq!(guard.check("t", &a), Verdict::Ok);
        }
        assert!(matches!(guard.check("t", &a), Verdict::Stop { .. }));
    }

    #[test]
    fn spots_cycles_and_warns_once() {
        let mut guard = LoopGuard::default();
        let a = args(json!({"x": 1}));
        let b = args(json!({"x": 2}));
        assert_eq!(guard.check("t", &a), Verdict::Ok);
        assert_eq!(guard.check("t", &b), Verdict::Ok);
        assert_eq!(guard.check("t", &a), Verdict::Ok);
        assert_eq!(guard.check("t", &b), Verdict::Stop { warn: true, reason: "The last tool calls repeat in a cycle.".into() });
        guard.reset();
        assert_eq!(guard.check("t", &a), Verdict::Ok);
    }
}
