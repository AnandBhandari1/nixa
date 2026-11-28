mod actions;
mod busy;
#[cfg(test)]
mod busy_tests;
mod job;
mod launch;
mod live;
mod routes;
mod start;
#[cfg(test)]
mod tests;
mod tool;

pub use actions::{ACTIONS, Outcome, Who, action, free_ids, views_now};
pub use busy::{busy_text, free_now};
pub use live::{AgentView, CUSTOM_LABEL, PURPOSE_LABEL};
pub use routes::{WS_KIND, is_ws, routes, ws_request};
pub(crate) use start::own_space;
pub use start::{How, UseInput, Used, for_model, model_changes, attach_session, save_session_model, session_model, use_agent, use_agent_with, with_model};
pub use tool::{NAME, call, specs};

use crate::app::App;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub const CHANGED: &str = "custom_agents_changed";

#[derive(Default)]
pub struct State {
    pub opening: Mutex<HashSet<String>>,
    pub handed: Mutex<HashMap<String, u64>>,
}

pub fn changed(app: &App) {
    app.send_out(json!({"type": CHANGED}));
}
