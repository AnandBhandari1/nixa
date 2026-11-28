use super::open;
use crate::app::App;
use serde_json::{Value, json};

pub const KEY: &str = "models.online_scores";
pub const ACTION: &str = "online";
pub const LABEL: &str = "Update model scores from the internet";
pub const ABOUT: &str = "Off by default. When it is on, Nixa reads the public SWE-bench leaderboard from raw.githubusercontent.com once a week and when you click Refresh public scores. Nothing else is sent.";
pub const OFF_NOTE: &str = "Public scores are off. To read them from the internet, turn on Settings, Web search, Update model scores from the internet.";
pub const ON_NOTE: &str = "Public scores are on. Click Refresh public scores to read them now.";

pub fn on(app: &App) -> bool {
    open(app).ok().and_then(|conn| nixa_store::settings::setting_bool(&conn, KEY, false).ok()).unwrap_or(false)
}

pub fn set(app: &App, on: bool) -> Result<(), String> {
    let conn = open(app)?;
    nixa_store::settings::set_setting_bool(&conn, KEY, on).map_err(|e| format!("Could not save {KEY}: {e:#}"))
}

pub fn note(on: bool) -> &'static str {
    if on { "" } else { OFF_NOTE }
}

pub fn view(app: &App) -> Value {
    let on = on(app);
    json!({"on": on, "label": LABEL, "about": ABOUT, "note": note(on)})
}

pub fn change(app: &App, args: &Value) -> Result<Value, String> {
    if let Some(map) = args.as_object()
        && let Some(extra) = map.keys().find(|k| k.as_str() != "on")
    {
        return Err(format!("Unknown argument {extra}. Nothing was done."));
    }
    match args.get("on") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(wanted)) => {
            set(app, *wanted)?;
            super::changed(app);
            app.info(if *wanted { ON_NOTE } else { "Public model scores are off. Nixa makes no call for them." });
        }
        Some(_) => return Err("on must be true or false.".into()),
    }
    Ok(view(app))
}

pub fn stamp(app: &App, value: &mut Value) {
    if let Some(map) = value.as_object_mut() {
        let on = on(app);
        map.insert("scoresOnline".into(), json!(on));
        map.insert("scoresNote".into(), json!(note(on)));
    }
}
