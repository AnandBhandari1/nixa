use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MapView {
    pub default: Option<String>,
    pub terminals: BTreeMap<String, String>,
    pub models: BTreeMap<String, String>,
    pub agents: BTreeMap<String, String>,
    pub spaces: BTreeMap<String, String>,
    pub kinds: BTreeMap<String, String>,
}

impl MapView {
    pub fn section(&self, level: &str) -> Option<&BTreeMap<String, String>> {
        match level {
            "terminal" => Some(&self.terminals),
            "model" => Some(&self.models),
            "agent" => Some(&self.agents),
            "space" => Some(&self.spaces),
            "kind" => Some(&self.kinds),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Kind {
    pub id: String,
    pub label: String,
    pub hint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ModelItem {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RuleView {
    pub tier: String,
    pub level: u8,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Resolved {
    pub id: String,
    pub voice: String,
    pub rule: RuleView,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct LiveTerm {
    pub address: String,
    pub agent: String,
    pub space: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Live {
    pub spaces: Vec<String>,
    pub terminals: Vec<LiveTerm>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct VoiceState {
    pub voices: Vec<String>,
    pub normal: Vec<String>,
    pub source: String,
    #[serde(alias = "voiceOn")]
    pub voice_on: Option<bool>,
    pub note: String,
    pub sample: String,
    pub humor: String,
    pub map: MapView,
    pub kinds: Vec<Kind>,
    pub agents: Vec<String>,
    pub models: Vec<ModelItem>,
    pub rows: Vec<Resolved>,
    pub live: Live,
}

impl VoiceState {
    pub fn humor_on(&self) -> bool {
        self.humor == "light"
    }

    pub fn listed_by_tool(&self) -> bool {
        self.source == "speak"
    }

    pub fn is_normal(&self, voice: &str) -> bool {
        self.normal.iter().any(|n| n.eq_ignore_ascii_case(voice))
    }
}

pub fn parse(value: &Value) -> Option<VoiceState> {
    let data = value.get("data").filter(|d| d.is_object()).unwrap_or(value);
    data.get("voices")?;
    serde_json::from_value(data.clone()).ok()
}
