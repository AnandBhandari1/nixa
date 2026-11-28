use super::proc::{self, Event, Spec};
use super::opencode::{Api, wait_up};
use super::{Kind, find_bin};
use crate::app::{App, lock};
use crate::util::{log, now_ms};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub const IDLE_MS: u64 = 600_000;
const CHECK_MS: u64 = 15_000;

pub fn config() -> Value {
    json!({
        "share": "disabled",
        "update": "disable",
        "snapshots": false,
        "lsp": false,
        "formatter": false,
        "websearch": false,
        "agents": {
            "nixa-decide": {
                "mode": "primary",
                "description": "Nixa decisions only",
                "permissions": [{"action": "*", "resource": "*", "effect": "deny"}]
            }
        },
        "default_agent": "nixa-decide"
    })
}

pub fn should_stop(idle_ms: u64, chat_has_server: bool) -> bool {
    chat_has_server || idle_ms >= IDLE_MS
}

pub fn running(app: &App) -> Option<(String, String)> {
    lock(&app.cli.live).decide_server.clone()
}

fn touch(app: &App) {
    lock(&app.cli.live).decide_used = now_ms();
}

pub async fn get(app: &Arc<App>) -> Result<(String, String), String> {
    if let Some(server) = running(app) {
        touch(app);
        return Ok(server);
    }
    let _guard = app.cli.decide_start.lock().await;
    if let Some(server) = running(app) {
        touch(app);
        return Ok(server);
    }
    start(app).await
}

async fn start(app: &Arc<App>) -> Result<(String, String), String> {
    let saved = app.cli.saved();
    let bin = find_bin(Kind::Opencode, &saved).ok_or_else(|| format!("off: the OpenCode command ({}) is not installed on this Mac", Kind::Opencode.bin_name()))?;
    nixa_protocol::paths::ensure_private_dir(&app.cli.work_dir).map_err(|e| e.to_string())?;
    let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())?;
    let password = super::new_token();
    let env = vec![
        ("OPENCODE_SERVER_PASSWORD".to_string(), password.clone()),
        ("OPENCODE_CONFIG_CONTENT".to_string(), config().to_string()),
        ("OPENCODE_DISABLE_CLAUDE_CODE".to_string(), "1".to_string()),
        ("OPENCODE_DISABLE_PROJECT_CONFIG".to_string(), "1".to_string()),
        ("OPENCODE_DISABLE_AUTOUPDATE".to_string(), "1".to_string()),
    ];
    let spec = Spec { bin, args: vec!["serve".into(), "--port".into(), port.to_string(), "--hostname".into(), "127.0.0.1".into()], cwd: app.cli.work_dir.clone(), env, stdin: false };
    let mut p = proc::spawn(&app.cli.groups, "OpenCode decisions", &spec, false)?;
    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).build().unwrap_or_default();
    let api = Api { http, base: format!("http://127.0.0.1:{port}"), password };
    if let Err(e) = wait_up(&api, &mut p).await {
        p.stop();
        log("decide", &format!("the decisions OpenCode server did not start: {e}"));
        return Err(format!("off: {e}"));
    }
    let server = (api.base.clone(), api.password.clone());
    {
        let mut live = lock(&app.cli.live);
        live.decide_server = Some(server.clone());
        live.decide_used = now_ms();
    }
    log("decide", &format!("started a small OpenCode server for decisions on port {port}"));
    tokio::spawn(watch(app.clone(), p));
    Ok(server)
}

async fn watch(app: Arc<App>, mut p: proc::Proc) {
    let why = loop {
        tokio::time::sleep(Duration::from_millis(CHECK_MS)).await;
        if let Ok(Event::Ended(code)) = p.events.try_recv() {
            break format!("it stopped by itself (code {code:?})");
        }
        let (used, chat) = {
            let live = lock(&app.cli.live);
            (live.decide_used, live.opencode.is_some())
        };
        if should_stop(now_ms().saturating_sub(used), chat) {
            break if chat { "the OpenCode chat brain has its own server".to_string() } else { "it was not used for 10 minutes".to_string() };
        }
    };
    lock(&app.cli.live).decide_server = None;
    p.stop();
    log("decide", &format!("stopped the decisions OpenCode server: {why}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_when_idle_or_when_the_chat_brain_has_a_server() {
        assert!(!should_stop(0, false));
        assert!(!should_stop(IDLE_MS - 1, false));
        assert!(should_stop(IDLE_MS, false));
        assert!(should_stop(0, true));
    }

    #[test]
    fn config_denies_every_tool_and_has_no_mcp() {
        let c = config();
        assert!(c.get("mcp").is_none());
        assert_eq!(c["agents"]["nixa-decide"]["permissions"][0]["effect"], "deny");
        assert_eq!(c["default_agent"], "nixa-decide");
    }
}
