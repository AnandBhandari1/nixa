use crate::links::brain::http;
use nixa_protocol::paths;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub type Call = Arc<dyn Fn(&str, Option<Value>) -> Result<Value, String> + Send + Sync>;

pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const REFRESH_WAIT: Duration = Duration::from_secs(45);
pub const LIST_WAIT: Duration = Duration::from_secs(75);
pub const PATH: &str = "/models";

pub fn route(action: &str) -> (&'static str, String, Duration) {
    match action {
        "list" => ("GET", PATH.to_string(), CALL_WAIT),
        "view" => ("GET", format!("{PATH}/view"), CALL_WAIT),
        "refresh" => ("POST", format!("{PATH}/refresh"), REFRESH_WAIT),
        "agent_models" => ("POST", format!("{PATH}/agent_models"), LIST_WAIT),
        other => ("POST", format!("{PATH}/{other}"), CALL_WAIT),
    }
}

pub fn brain() -> Call {
    Arc::new(|action, body| {
        let (method, path, wait) = route(action);
        http(&paths::brain_addr(), method, &path, body.as_ref(), wait).map_err(|e| format!("{e:#}"))
    })
}
