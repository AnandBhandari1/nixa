use super::{DEFAULT_MODEL, MODELS};
use crate::app::App;
use nixa_store::settings::{get_setting, set_setting};
use serde::Serialize;
use serde_json::{Value, json};

pub const PREFIX: &str = "decide.";
pub const DEFAULT_LEVEL: f64 = 0.85;
pub const MIN_LEVEL: f64 = 0.5;
pub const MAX_LEVEL: f64 = 0.99;
pub const USES: [&str; 9] = ["questions", "review", "drones", "topic", "idle", "followup", "routing", "screens", "chat"];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Settings {
    pub on: bool,
    pub model: String,
    pub level: f64,
    pub questions: bool,
    pub review: bool,
    pub drones: bool,
    pub topic: bool,
    pub idle: bool,
    pub followup: bool,
    pub routing: bool,
    pub screens: bool,
    pub chat: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { on: false, model: DEFAULT_MODEL.to_string(), level: DEFAULT_LEVEL, questions: true, review: true, drones: true, topic: true, idle: true, followup: true, routing: true, screens: true, chat: true }
    }
}

impl Settings {
    pub fn uses(&self, name: &str) -> bool {
        self.on
            && match name {
                "questions" => self.questions,
                "review" => self.review,
                "drones" => self.drones,
                "topic" => self.topic,
                "idle" => self.idle,
                "followup" => self.followup,
                "routing" => self.routing,
                "screens" => self.screens,
                "chat" => self.chat,
                _ => false,
            }
    }

    pub fn route(&self) -> &'static str {
        if self.on { "Decision model first" } else { "Current setup" }
    }
}

fn bool_of(body: &Value, key: &str, before: bool) -> Result<bool, String> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(before),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{key} must be true or false")),
    }
}

pub fn apply(before: &Settings, body: &Value) -> Result<Settings, String> {
    let model = match body.get("model") {
        None | Some(Value::Null) => before.model.clone(),
        Some(Value::String(m)) if MODELS.contains(&m.trim()) => m.trim().to_string(),
        Some(_) => return Err(format!("model must be one of {}", MODELS.join(", "))),
    };
    let level = match body.get("level") {
        None | Some(Value::Null) => before.level,
        Some(v) => match v.as_f64() {
            Some(l) if (MIN_LEVEL..=MAX_LEVEL).contains(&l) => l,
            _ => return Err(format!("level must be a number from {MIN_LEVEL} to {MAX_LEVEL}")),
        },
    };
    Ok(Settings {
        on: bool_of(body, "on", before.on)?,
        model,
        level,
        questions: bool_of(body, "questions", before.questions)?,
        review: bool_of(body, "review", before.review)?,
        drones: bool_of(body, "drones", before.drones)?,
        topic: bool_of(body, "topic", before.topic)?,
        idle: bool_of(body, "idle", before.idle)?,
        followup: bool_of(body, "followup", before.followup)?,
        routing: bool_of(body, "routing", before.routing)?,
        screens: bool_of(body, "screens", before.screens)?,
        chat: bool_of(body, "chat", before.chat)?,
    })
}

fn store(app: &App) -> Result<nixa_store::Connection, String> {
    nixa_store::open(&app.db_file).map_err(|e| format!("The store did not open: {e:#}"))
}

pub fn from_saved(saved: &serde_json::Map<String, Value>) -> Settings {
    let mut body = serde_json::Map::new();
    for (key, value) in saved {
        if let Some(short) = key.strip_prefix(PREFIX) {
            body.insert(short.to_string(), value.clone());
        }
    }
    let defaults = Settings::default();
    let mut out = defaults.clone();
    for (key, value) in body {
        if let Ok(next) = apply(&out, &json!({key: value})) {
            out = next;
        }
    }
    out
}

pub fn load(app: &App) -> Settings {
    let Ok(conn) = store(app) else { return Settings::default() };
    let mut saved = serde_json::Map::new();
    for key in ["on", "model", "level", "questions", "review", "drones", "topic", "idle", "followup", "routing", "screens", "chat"] {
        let full = format!("{PREFIX}{key}");
        if let Ok(Some(value)) = get_setting(&conn, &full) {
            saved.insert(full, value);
        }
    }
    from_saved(&saved)
}

pub fn save(app: &App, body: &Value) -> Result<Settings, String> {
    let next = apply(&load(app), body)?;
    let conn = store(app)?;
    let values = [("on", json!(next.on)), ("model", json!(next.model)), ("level", json!(next.level)), ("questions", json!(next.questions)), ("review", json!(next.review)), ("drones", json!(next.drones)), ("topic", json!(next.topic)), ("idle", json!(next.idle)), ("followup", json!(next.followup)), ("routing", json!(next.routing)), ("screens", json!(next.screens)), ("chat", json!(next.chat))];
    for (key, value) in values {
        set_setting(&conn, &format!("{PREFIX}{key}"), &value).map_err(|e| format!("Could not save {PREFIX}{key}: {e:#}"))?;
    }
    crate::util::log("decide", &format!("settings: {}, model {}, level {}", next.route(), next.model, next.level));
    Ok(next)
}
