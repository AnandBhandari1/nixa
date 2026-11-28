use crate::app::App;
use serde_json::{Value, json};

pub fn snapshot(app: &App) -> Value {
    let mut out = crate::phone_door::snapshot(app);
    if let (Some(map), Value::Object(view)) = (out.as_object_mut(), crate::settings_api::view(app)) {
        map.extend(view);
        map.insert("watchMinutes".into(), json!(app.watch_minutes()));
        map.insert("phone".into(), crate::phone_door::view(app)["phone"].clone());
        let mut prompts = crate::prompts::snapshot(app);
        if let Some(prompts) = prompts.as_object_mut() {
            prompts.remove("type");
        }
        map.insert("prompts".into(), prompts);
    }
    out
}

pub fn send_snapshot(app: &App) -> Value {
    let snap = snapshot(app);
    app.send_out(json!({"type": "snapshot", "snapshot": snap}));
    crate::settings_api::send_state(app);
    crate::settings_api::send_work(app);
    crate::prompts::broadcast(app);
    snap
}
