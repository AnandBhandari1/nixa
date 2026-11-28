use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const MAX_KEY: usize = 200;
pub const MAX_VOICE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Terminal,
    Model,
    Agent,
    Space,
    Kind,
}

impl Level {
    pub const ALL: [Level; 5] = [Level::Terminal, Level::Model, Level::Agent, Level::Space, Level::Kind];

    pub fn parse(text: &str) -> Option<Level> {
        match text.trim() {
            "terminal" => Some(Level::Terminal),
            "model" => Some(Level::Model),
            "agent" => Some(Level::Agent),
            "space" => Some(Level::Space),
            "kind" => Some(Level::Kind),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Level::Terminal => "terminal",
            Level::Model => "model",
            Level::Agent => "agent",
            Level::Space => "space",
            Level::Kind => "kind",
        }
    }

    pub fn section(self) -> &'static str {
        match self {
            Level::Terminal => "terminals",
            Level::Model => "models",
            Level::Agent => "agents",
            Level::Space => "spaces",
            Level::Kind => "kinds",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    pub terminals: BTreeMap<String, String>,
    pub models: BTreeMap<String, String>,
    pub agents: BTreeMap<String, String>,
    pub spaces: BTreeMap<String, String>,
    pub kinds: BTreeMap<String, String>,
}

impl VoiceMap {
    pub fn section(&self, level: Level) -> &BTreeMap<String, String> {
        match level {
            Level::Terminal => &self.terminals,
            Level::Model => &self.models,
            Level::Agent => &self.agents,
            Level::Space => &self.spaces,
            Level::Kind => &self.kinds,
        }
    }

    fn section_mut(&mut self, level: Level) -> &mut BTreeMap<String, String> {
        match level {
            Level::Terminal => &mut self.terminals,
            Level::Model => &mut self.models,
            Level::Agent => &mut self.agents,
            Level::Space => &mut self.spaces,
            Level::Kind => &mut self.kinds,
        }
    }

    pub fn get(&self, level: Level, key: &str) -> Option<&String> {
        let key = voice_key(level, key);
        if key.is_empty() {
            return None;
        }
        self.section(level).get(&key)
    }
}

pub fn cut(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect::<String>().trim().to_string()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn starts_with_word(text: &str, word: &str) -> bool {
    text.strip_prefix(word).is_some_and(|rest| !rest.starts_with(is_word))
}

pub fn normalize_agent(value: &str) -> String {
    let text = cut(value, MAX_KEY).to_lowercase();
    if text.is_empty() {
        return text;
    }
    let found = [("claude", "claude"), ("codex", "codex"), ("cursor", "cursor"), ("agy", "agy"), ("antigravity", "agy"), ("opencode", "opencode"), ("kimi", "kimi"), ("gemini", "gemini")]
        .iter()
        .find(|(part, _)| text.contains(part))
        .map(|(_, name)| name.to_string());
    if let Some(name) = found {
        return name;
    }
    if starts_with_word(&text, "omp") || text.contains("oh my pi") || text == "pi" {
        return "omp".into();
    }
    text
}

pub fn voice_key(level: Level, key: &str) -> String {
    let text = cut(key, MAX_KEY);
    match level {
        Level::Terminal => text,
        Level::Agent => normalize_agent(&text),
        _ => text.to_lowercase(),
    }
}

fn clean_voice(value: &Value) -> Option<String> {
    value.as_str().map(|v| cut(v, MAX_VOICE)).filter(|v| !v.is_empty())
}

fn clean_section(level: Level, raw: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(raw) = raw.and_then(Value::as_object) else { return out };
    for (key, value) in raw {
        let name = voice_key(level, key);
        if let Some(voice) = clean_voice(value)
            && !name.is_empty()
        {
            out.insert(name, voice);
        }
    }
    out
}

pub fn clean(raw: &Value) -> VoiceMap {
    VoiceMap {
        default: raw.get("default").and_then(clean_voice),
        terminals: clean_section(Level::Terminal, raw.get("terminals")),
        models: clean_section(Level::Model, raw.get("models")),
        agents: clean_section(Level::Agent, raw.get("agents")),
        spaces: clean_section(Level::Space, raw.get("spaces")),
        kinds: clean_section(Level::Kind, raw.get("kinds")),
    }
}

pub fn is_empty(map: &VoiceMap) -> bool {
    map.default.is_none() && Level::ALL.iter().all(|level| map.section(*level).is_empty())
}

pub fn set(map: &mut VoiceMap, level: Level, key: &str, voice: Option<&str>) -> bool {
    let name = voice_key(level, key);
    if name.is_empty() {
        return false;
    }
    let section = map.section_mut(level);
    match voice.map(|v| cut(v, MAX_VOICE)).filter(|v| !v.is_empty()) {
        Some(voice) => {
            section.insert(name, voice);
        }
        None => {
            section.remove(&name);
        }
    }
    true
}

pub fn set_default(map: &mut VoiceMap, voice: Option<&str>) {
    map.default = voice.map(|v| cut(v, MAX_VOICE)).filter(|v| !v.is_empty());
}
