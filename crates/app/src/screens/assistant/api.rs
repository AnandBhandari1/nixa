use crate::links::brain::http;
use gpui::App;
use nixa_protocol::paths;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub type Call = Arc<dyn Fn(Value) -> Result<Value, String> + Send + Sync>;

pub const PATH: &str = "/assistant";
pub const WAIT: Duration = Duration::from_secs(45);

pub fn brain() -> Call {
    Arc::new(|body| http(&paths::brain_addr(), "POST", PATH, Some(&body), WAIT).map_err(|e| format!("{e:#}")))
}

#[derive(Clone, Copy)]
pub struct Host {
    pub tasks: fn(&mut App),
    pub schedules: fn(&mut App),
}

fn open_schedules(cx: &mut App) {
    crate::screens::settings::open(crate::screens::settings::parts::schedules::PART, cx);
}

pub fn host() -> Host {
    Host { tasks: crate::screens::tasks::open, schedules: open_schedules }
}
