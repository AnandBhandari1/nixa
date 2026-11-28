use crate::links::brain::http;
use nixa_protocol::paths;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub type Call = Arc<dyn Fn(&str, Option<Value>) -> Result<Value, String> + Send + Sync>;

pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const USE_WAIT: Duration = Duration::from_secs(150);
pub const LIST_PATH: &str = "/custom-agents";

pub fn route(action: &str) -> (&'static str, String, Duration) {
    match action {
        "list" => ("GET", LIST_PATH.to_string(), CALL_WAIT),
        "use" => ("POST", format!("{LIST_PATH}/use"), USE_WAIT),
        other => ("POST", format!("{LIST_PATH}/{other}"), CALL_WAIT),
    }
}

pub fn brain() -> Call {
    Arc::new(|action, body| {
        let (method, path, wait) = route(action);
        http(&paths::brain_addr(), method, &path, body.as_ref(), wait).map_err(|e| format!("{e:#}"))
    })
}
