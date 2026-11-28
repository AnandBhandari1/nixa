use super::map::{Level, VoiceMap, cut, set, set_default};
use super::resolve::{Ctx, Deps, Resolved, resolve};
use serde::Serialize;
use serde_json::{Value, json};

pub const SAMPLE_LINE: &str = "Hello, this is how I sound.";
pub const LIST_MS: u64 = 5 * 60 * 1000;
pub const OFF_LINE: &str = "Voice is off. Turn voice on in Settings to list and hear the voices of the voice tool.";
pub const OFF_PLAY: &str = "Voice is off. Turn voice on in Settings to hear a sample.";
pub const MAX_ROWS: usize = 400;
pub const MAX_ROW_ID: usize = 300;

pub const KINDS: [(&str, &str, &str); 8] = [
    ("chat", "Nixa chat replies", "Nixa's own spoken answers"),
    ("deep", "Deep brain lines", "what Nixa says after the deep brain decides; manual rules win, otherwise the chat reply voice"),
    ("finished", "Agent finished", "an agent is done with its work"),
    ("attention", "Agent needs attention", "an agent is blocked, or Nixa asks you something"),
    ("check", "Check needed", "check lines"),
    ("alarm", "Alarm or reminder", "alarms and reminders"),
    ("update", "Task and team updates", "team runs, PR flow and task lines"),
    ("other", "Anything else", "any other spoken line"),
];

pub const AGENTS: [&str; 9] = ["claude", "codex", "cursor", "agy", "omp", "kimi", "opencode", "gemini", "unknown"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Speak,
    BuiltIn,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceList {
    pub voices: Vec<String>,
    pub normal: Vec<String>,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub ctx: Ctx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Humor {
    Light,
    Off,
}

impl Humor {
    pub fn parse(text: &str) -> Option<Humor> {
        match text.trim() {
            "light" => Some(Humor::Light),
            "off" => Some(Humor::Off),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Humor::Light => "light",
            Humor::Off => "off",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    State,
    Humor(Humor),
    Set { level: Option<Level>, key: String, voice: String },
    Clear { level: Option<Level>, key: String },
    Reset,
    Play { voice: Option<String>, ctx: Ctx },
    Stop,
}

pub fn builtin_list() -> Vec<String> {
    nixa_voice::CYCLE_VOICES.iter().map(|v| v.to_string()).collect()
}

pub fn off_list(known: Option<VoiceList>) -> VoiceList {
    let list = known.unwrap_or_else(|| list_from(Vec::new()));
    VoiceList { source: Source::Off, ..list }
}

pub fn list_from(names: Vec<String>) -> VoiceList {
    let normal_all = builtin_list();
    if names.is_empty() || names == normal_all {
        return VoiceList { voices: normal_all.clone(), normal: normal_all, source: Source::BuiltIn };
    }
    let normal = names.iter().filter(|n| normal_all.iter().any(|c| c.eq_ignore_ascii_case(n))).cloned().collect();
    VoiceList { voices: names, normal, source: Source::Speak }
}

pub fn rows_of(value: Option<&Value>) -> Vec<Row> {
    let Some(list) = value.and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?;
            Some(Row { id: id.chars().take(MAX_ROW_ID).collect(), ctx: row.get("ctx").map(Ctx::of).unwrap_or_default() })
        })
        .take(MAX_ROWS)
        .collect()
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).map(|v| cut(v, 300)).unwrap_or_default()
}

fn level_of(value: &Value) -> Result<Option<Level>, String> {
    let word = text(value, "level");
    if word == "default" {
        return Ok(None);
    }
    Level::parse(&word).map(Some).ok_or_else(|| "Unknown level.".to_string())
}

pub fn parse(args: &Value) -> Result<Action, String> {
    let action = text(args, "action");
    match action.as_str() {
        "state" | "" => Ok(Action::State),
        "humor" => Humor::parse(&text(args, "value")).map(Action::Humor).ok_or_else(|| "Humor must be light or off.".to_string()),
        "set" => Ok(Action::Set { level: level_of(args)?, key: text(args, "key"), voice: text(args, "voice") }),
        "clear" => Ok(Action::Clear { level: level_of(args)?, key: text(args, "key") }),
        "reset" => Ok(Action::Reset),
        "play" => Ok(Action::Play {
            voice: args.get("voice").and_then(Value::as_str).map(|v| cut(v, 100)),
            ctx: args.get("ctx").map(Ctx::of).unwrap_or_default(),
        }),
        "stop" => Ok(Action::Stop),
        other => Err(format!("Unknown voices action \"{other}\".")),
    }
}

pub fn check_voice(list: &[String], value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() {
        return Err("Pick a voice first.".into());
    }
    list.iter().find(|v| v.eq_ignore_ascii_case(name)).cloned().ok_or_else(|| format!("\"{name}\" is not one of the voices."))
}

pub fn change(map: &mut VoiceMap, action: &Action, list: &[String]) -> Result<bool, String> {
    match action {
        Action::Set { level, key, voice } => {
            let voice = check_voice(list, voice)?;
            match level {
                None => set_default(map, Some(&voice)),
                Some(level) => {
                    if !set(map, *level, key, Some(&voice)) {
                        return Err("Pick what the voice is for.".into());
                    }
                }
            }
            Ok(true)
        }
        Action::Clear { level, key } => {
            match level {
                None => set_default(map, None),
                Some(level) => {
                    set(map, *level, key, None);
                }
            }
            Ok(true)
        }
        Action::Reset => {
            *map = VoiceMap::default();
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub fn kinds_json() -> Value {
    Value::Array(KINDS.iter().map(|(id, label, hint)| json!({"id": id, "label": label, "hint": hint})).collect())
}

pub struct Facts<'a> {
    pub map: &'a VoiceMap,
    pub list: &'a VoiceList,
    pub deps: &'a Deps,
    pub humor: Humor,
    pub models: Vec<(String, String)>,
    pub voice_on: bool,
}

pub fn state(facts: &Facts, rows: &[Row]) -> Value {
    let resolved = |ctx: &Ctx| -> Resolved { resolve(facts.map, ctx, facts.deps) };
    let builtin: serde_json::Map<String, Value> = AGENTS
        .iter()
        .chain(std::iter::once(&"nixa"))
        .map(|a| (a.to_string(), json!(super::resolve::builtin(facts.deps, a))))
        .collect();
    let rows: Vec<Value> = rows
        .iter()
        .map(|row| {
            let r = resolved(&row.ctx);
            json!({"id": row.id, "voice": r.voice, "rule": r.rule})
        })
        .collect();
    json!({
        "voices": facts.list.voices,
        "normal": facts.list.normal,
        "source": facts.list.source,
        "voiceOn": facts.voice_on,
        "note": if facts.voice_on { "" } else { OFF_LINE },
        "sample": SAMPLE_LINE,
        "humor": facts.humor,
        "map": facts.map,
        "kinds": kinds_json(),
        "agents": AGENTS,
        "models": facts.models.iter().map(|(id, label)| json!({"id": id, "label": label})).collect::<Vec<_>>(),
        "builtin": builtin,
        "fallback": resolved(&Ctx::default()),
        "rows": rows,
    })
}

pub fn models_of(rows: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (agent, model) in rows {
        let model = model.trim();
        if model.is_empty() || out.iter().any(|(id, _)| id.eq_ignore_ascii_case(model)) {
            continue;
        }
        let label = if agent.trim().is_empty() { model.to_string() } else { format!("{model} ({})", agent.trim()) };
        out.push((model.to_string(), label));
    }
    out
}
