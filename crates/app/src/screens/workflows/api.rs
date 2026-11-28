use crate::links::brain::{Link, http};
use nixa_protocol::paths;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Call = Arc<dyn Fn(&str, Value) -> Result<Value, String> + Send + Sync>;

pub const KIND: &str = "workflows";
pub const PATH: &str = "/workflows";
pub const WAIT: Duration = Duration::from_secs(15);

static LINK: Mutex<Option<Link>> = Mutex::new(None);

pub fn set_link(link: Link) {
    if let Ok(mut slot) = LINK.lock() {
        *slot = Some(link);
    }
}

fn link() -> Option<Link> {
    LINK.lock().ok().and_then(|slot| slot.clone())
}

pub fn args(action: &str, extra: Value) -> Value {
    let mut map = match extra {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.insert("action".into(), Value::String(action.to_string()));
    Value::Object(map)
}

pub fn brain() -> Call {
    Arc::new(|action, extra| {
        let body = args(action, extra);
        match link() {
            Some(link) if link.connected() => link
                .request_for(KIND, json!({ "args": body }), WAIT)
                .map_err(|e| format!("{e:#}")),
            _ => http(&paths::brain_addr(), "POST", PATH, Some(&body), WAIT)
                .map_err(|e| format!("{e:#}")),
        }
    })
}
