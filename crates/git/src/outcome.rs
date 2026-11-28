use crate::errors::describe;
use crate::run::Out;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub ok: bool,
    pub kind: String,
    pub title: String,
    pub hint: String,
    pub detail: String,
    pub stderr: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Outcome {
    pub fn with(mut self, key: &str, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = hint.into();
        self
    }

    pub fn detail(mut self, detail: &str) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn stderr(mut self, stderr: &str) -> Self {
        self.stderr = stderr.into();
        self
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.extra.get(key)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.extra.get(key).and_then(Value::as_str)
    }

    pub fn flag(&self, key: &str) -> bool {
        self.extra.get(key).and_then(Value::as_bool).unwrap_or(false)
    }
}

pub fn success(title: &str) -> Outcome {
    Outcome { ok: true, title: title.into(), ..Outcome::default() }
}

pub fn failure(text: &str) -> Outcome {
    let words = describe(text);
    Outcome {
        ok: false,
        kind: words.kind,
        title: words.title,
        hint: words.hint,
        detail: words.detail,
        stderr: text.into(),
        extra: Map::new(),
    }
}

pub fn refusal(kind: &str, title: &str, hint: &str) -> Outcome {
    Outcome { ok: false, kind: kind.into(), title: title.into(), hint: hint.into(), ..Outcome::default() }
}

pub fn described(text: &str) -> Outcome {
    let words = describe(text);
    Outcome { ok: false, kind: words.kind, title: words.title, hint: words.hint, ..Outcome::default() }
}

pub fn stopped(out: &Out) -> Option<Outcome> {
    if out.cancelled {
        return Some(refusal("cancelled", "Stopped", "Nothing more was sent.").with("cancelled", true.into()));
    }
    if out.timed_out {
        return Some(described("timed out waiting").with("timedOut", true.into()));
    }
    None
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_flattens_extra_fields() {
        let out = success("Committed").with("sha", "abc1234".into());
        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(json["sha"], "abc1234");
        assert_eq!(json["ok"], true);
        let back: Outcome = serde_json::from_value(json).unwrap();
        assert_eq!(back.text("sha"), Some("abc1234"));
    }

    #[test]
    fn stopped_reports_cancel_and_timeout() {
        let out = Out { cancelled: true, ..Out::default() };
        assert_eq!(stopped(&out).unwrap().kind, "cancelled");
        let out = Out { timed_out: true, ..Out::default() };
        let s = stopped(&out).unwrap();
        assert_eq!(s.kind, "timeout");
        assert!(s.flag("timedOut"));
        assert!(stopped(&Out::default()).is_none());
        assert_eq!(plural(1, "file", "files"), "1 file");
    }
}
