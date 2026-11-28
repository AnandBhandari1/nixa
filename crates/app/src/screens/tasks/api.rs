use super::model::NOT_CONNECTED;
use crate::links::brain::Link;
use gpui::App;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub type Call = Arc<dyn Fn(&str, Value) -> Result<Value, String> + Send + Sync>;

pub const KIND: &str = "tasks";
pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const STOP_ACTION: &str = "speak_stop";


pub fn wait_of(action: &str) -> Duration {
    let secs = match action {
        "start" => 60,
        "continue" => 120,
        "release" => 60,
        "speak" | "speak_wait" => 900,
        _ => return CALL_WAIT,
    };
    Duration::from_secs(secs)
}

pub fn body(action: &str, args: Value) -> Value {
    json!({"action": action, "args": args})
}

pub fn link_call(link: Option<Link>) -> Call {
    Arc::new(move |action, args| {
        let Some(link) = &link else {
            return Err(NOT_CONNECTED.to_string());
        };
        link.request_for(KIND, body(action, args), wait_of(action))
            .map_err(|e| format!("{e:#}"))
    })
}

#[derive(Clone, Copy)]
pub struct Host {
    pub focus: fn(&str, &mut App) -> bool,
    pub active_space: fn(&App) -> String,
    pub settings: fn(&str, &mut App),
    pub open_url: fn(&str, &mut App),
}

pub fn open_url(url: &str, cx: &mut App) {
    cx.open_url(url);
}

pub fn host() -> Host {
    Host {
        focus: super::place::focus,
        active_space: super::place::active_space,
        settings: super::place::settings,
        open_url,
    }
}
