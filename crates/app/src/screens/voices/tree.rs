use super::model::{MapView, Resolved, VoiceState};
use serde_json::{Value, json};
use std::collections::HashSet;

pub struct LevelInfo {
    pub id: &'static str,
    pub number: u8,
    pub title: &'static str,
    pub hint: &'static str,
}

pub const LEVELS: [LevelInfo; 5] = [
    LevelInfo { id: "default", number: 5, title: "Default voice", hint: "Used for anything that matches no other rule" },
    LevelInfo { id: "kind", number: 4, title: "Kind of speech", hint: "Nixa's chat, agent finished, check needed, alarm" },
    LevelInfo { id: "space", number: 3, title: "Space or domain", hint: "A project such as a space of yours" },
    LevelInfo { id: "agent", number: 2, title: "Agent program or model", hint: "All agents of one program, or one model" },
    LevelInfo { id: "terminal", number: 1, title: "Numbered agent", hint: "One agent tab. The most specific rule." },
];

pub const AFTER: &str = "finished";

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub level: &'static str,
    pub key: String,
    pub label: String,
    pub hint: String,
    pub indent: u8,
    pub model: bool,
    pub ctx: Value,
}

pub fn map_key(level: &str, key: &str) -> String {
    if level == "terminal" { key.trim().to_string() } else { key.trim().to_lowercase() }
}

pub fn explicit_voice(map: &MapView, row: &Row) -> Option<String> {
    if row.level == "default" {
        return map.default.clone();
    }
    map.section(row.level)?.get(&map_key(row.level, &row.key)).cloned()
}

pub fn voice_name(voice: &str) -> String {
    if voice == "cycle" { "random voice".into() } else { voice.to_string() }
}

fn saved(map: &MapView, level: &str) -> Vec<String> {
    map.section(level).map(|s| s.keys().cloned().collect()).unwrap_or_default()
}

fn row(id: String, level: &'static str, key: &str, label: &str, hint: &str, indent: u8, ctx: Value) -> Row {
    Row { id, level, key: key.to_string(), label: label.to_string(), hint: hint.to_string(), indent, model: level == "model", ctx }
}

pub fn build_rows(state: Option<&VoiceState>) -> Vec<Row> {
    let mut rows = vec![row("default".into(), "default", "", "Default voice", "Anything that matches no other rule", 0, json!({}))];
    let Some(state) = state else { return rows };
    let map = &state.map;
    for kind in &state.kinds {
        let ctx = if kind.id == "chat" { json!({"agent": "nixa", "kind": "chat"}) } else { json!({"kind": kind.id}) };
        rows.push(row(format!("kind:{}", kind.id), "kind", &kind.id, &kind.label, &kind.hint, 1, ctx));
    }
    let mut seen = HashSet::new();
    for name in state.live.spaces.iter().cloned().chain(saved(map, "space")) {
        let key = map_key("space", &name);
        if key.is_empty() || key == "~" || !seen.insert(key.clone()) {
            continue;
        }
        rows.push(row(format!("space:{key}"), "space", &name, &name, "Nixa speaking about this space", 2, json!({"space": name, "kind": AFTER})));
    }
    for agent in &state.agents {
        rows.push(row(format!("agent:{agent}"), "agent", agent, agent, "Every terminal of this agent", 3, json!({"agent": agent, "kind": AFTER})));
    }
    let mut seen = HashSet::new();
    let models = state.models.iter().map(|m| (m.id.clone(), m.label.clone())).chain(saved(map, "model").into_iter().map(|id| (id.clone(), id)));
    for (id, label) in models {
        let key = map_key("model", &id);
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        rows.push(row(format!("model:{key}"), "model", &id, &label, "One model, over its agent", 3, json!({"model": id, "kind": AFTER})));
    }
    let mut live = HashSet::new();
    for t in &state.live.terminals {
        if t.address.is_empty() || !live.insert(t.address.clone()) {
            continue;
        }
        let ctx = json!({"terminal": t.address, "agent": t.agent, "space": t.space, "kind": AFTER});
        rows.push(row(format!("terminal:{}", t.address), "terminal", &t.address, &t.label, &t.address, 4, ctx));
    }
    for address in saved(map, "terminal") {
        if live.contains(&address) {
            continue;
        }
        let label = format!("{address} (not open)");
        rows.push(row(format!("terminal:{address}"), "terminal", &address, &label, "Saved for an agent that is not open", 4, json!({"terminal": address, "kind": AFTER})));
    }
    rows
}

pub fn row_specs(rows: &[Row]) -> Value {
    Value::Array(rows.iter().map(|r| json!({"id": r.id, "ctx": r.ctx})).collect())
}

pub fn row_ids(rows: &[Row]) -> Vec<String> {
    rows.iter().map(|r| r.id.clone()).collect()
}

pub fn resolved_for<'a>(state: Option<&'a VoiceState>, row: &Row) -> Option<&'a Resolved> {
    state?.rows.iter().find(|r| r.id == row.id)
}

pub fn shown_rows(rows: &[Row], state: Option<&VoiceState>, level: &str, all_models: bool) -> Vec<Row> {
    let mut out: Vec<Row> = rows.iter().filter(|r| r.level == level && !r.model).cloned().collect();
    if level == "agent" {
        let map = state.map(|s| &s.map);
        out.extend(rows.iter().filter(|r| r.model && (all_models || map.is_some_and(|m| explicit_voice(m, r).is_some()))).cloned());
    }
    out
}
