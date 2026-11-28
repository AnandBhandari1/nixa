use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SETTINGS_KEY: &str = "git.ai";
pub const DEFAULT_AGENT: &str = "claude";
pub const DEFAULT_TIMEOUT_SEC: u64 = 240;
pub const MIN_TIMEOUT_SEC: u64 = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrefixRule {
    pub pattern: String,
    pub prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub preset: String,
    pub instructions: String,
    pub prefix_rules: Vec<PrefixRule>,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            preset: "conventional".into(),
            instructions: String::new(),
            prefix_rules: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Review {
    pub instructions: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub agent: String,
    pub models: BTreeMap<String, String>,
    pub timeout_sec: u64,
    pub style: Style,
    pub review: Review,
    pub checks_off: Vec<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            agent: DEFAULT_AGENT.into(),
            models: BTreeMap::new(),
            timeout_sec: DEFAULT_TIMEOUT_SEC,
            style: Style::default(),
            review: Review::default(),
            checks_off: Vec::new(),
        }
    }
}

impl AiSettings {
    pub fn from_value(value: Option<&Value>) -> AiSettings {
        let mut out = value
            .and_then(|v| serde_json::from_value::<AiSettings>(v.clone()).ok())
            .unwrap_or_default();
        if out.agent.trim().is_empty() {
            out.agent = DEFAULT_AGENT.into();
        }
        out
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn model_of(&self, agent: &str) -> &str {
        self.models.get(agent).map(|m| m.trim()).unwrap_or("")
    }

    pub fn checks_off_for(&self, agent: &str) -> bool {
        self.checks_off.iter().any(|a| a == agent)
    }
}

pub fn timeout_of(settings: &AiSettings, wanted: Option<u64>) -> u64 {
    let value = wanted.unwrap_or(settings.timeout_sec);
    let value = if value == 0 { DEFAULT_TIMEOUT_SEC } else { value };
    value.max(MIN_TIMEOUT_SEC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_fields_take_defaults() {
        let s = AiSettings::from_value(Some(&json!({"agent": "codex", "models": {"codex": " gpt-5 "}})));
        assert_eq!(s.agent, "codex");
        assert_eq!(s.model_of("codex"), "gpt-5");
        assert_eq!(s.timeout_sec, 240);
        assert_eq!(s.style.preset, "conventional");
        assert!(!s.checks_off_for("cursor"));
        assert_eq!(AiSettings::from_value(None), AiSettings::default());
        assert_eq!(AiSettings::from_value(Some(&json!(3))), AiSettings::default());
        assert_eq!(AiSettings::from_value(Some(&json!({"agent": ""}))).agent, "claude");
    }

    #[test]
    fn round_trips_as_snake_case() {
        let mut s = AiSettings::default();
        s.style.prefix_rules.push(PrefixRule { pattern: "f/*".into(), prefix: "x".into() });
        let v = s.to_value();
        assert_eq!(v["timeout_sec"], 240);
        assert_eq!(v["style"]["prefix_rules"][0]["pattern"], "f/*");
        assert_eq!(v["checks_off"], serde_json::json!([]));
        assert_eq!(AiSettings::from_value(Some(&v)), s);
        s.checks_off.push("cursor".into());
        assert!(AiSettings::from_value(Some(&s.to_value())).checks_off_for("cursor"));
    }

    #[test]
    fn timeout_has_a_floor() {
        let s = AiSettings::default();
        assert_eq!(timeout_of(&s, None), 240);
        assert_eq!(timeout_of(&s, Some(3)), 10);
        assert_eq!(timeout_of(&s, Some(0)), 240);
        assert_eq!(timeout_of(&s, Some(90)), 90);
    }
}
