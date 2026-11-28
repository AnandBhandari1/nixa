mod actions;
pub mod online;
pub mod public;
mod routes;
mod start;
#[cfg(test)]
mod tests;
pub mod tick;
mod tool;

pub use actions::{action, agent_models, launch_result, refresh_lists};
pub use start::{pick as pick_start, record as record_start};
pub use public::refresh;
pub use routes::{WS_KIND, is_ws, routes, ws_request};
pub use start::{prepare, programs, record};
pub use tick::{inventory, run};
pub use tool::{AGENT_MODELS, MODELS, NAMES, call, specs};

use crate::app::App;
use nixa_store::Connection;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Mutex;

pub const CHANGED: &str = "models_changed";

#[derive(Default)]
pub struct State {
    pub listing: Mutex<HashSet<String>>,
    pub refreshing: tokio::sync::Mutex<()>,
}

pub fn changed(app: &App) {
    app.send_out(json!({"type": CHANGED}));
}

pub fn open(app: &App) -> Result<Connection, String> {
    nixa_store::open(&app.db_file).map_err(|e| format!("could not open the store: {e:#}"))
}
