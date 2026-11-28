use super::TOKEN_HEADER;
use crate::app::{App, lock};
use crate::loop_guard::{Verdict, loop_note};
use crate::turns::{ToolCall, args_of};
use crate::util::clip_note;
use axum::Router;
use axum::Extension;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub const TOOL_CHARS: usize = 12_000;

static CALLS: AtomicU64 = AtomicU64::new(0);

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/tools", get(list))
        .route("/tool", post(call))
        .route("/agent/tools", get(agent_list))
        .route("/agent/tool", post(agent_call))
}

pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn allowed(app: &App, headers: &HeaderMap) -> bool {
    let sent = headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    !app.cli.token.is_empty() && same(sent, &app.cli.token)
}

fn refused() -> Response {
    (StatusCode::UNAUTHORIZED, axum::Json(json!({"ok": false, "reason": "The Nixa tool token is missing or wrong."}))).into_response()
}

pub fn tool_list(app: &App) -> Value {
    let tools: Vec<Value> = crate::tools::specs(app)
        .into_iter()
        .map(|s| json!({"name": s.name, "description": s.description, "inputSchema": s.schema}))
        .collect();
    Value::Array(tools)
}

pub fn served_list(app: &App) -> Value {
    let tools = tool_list(app);
    let names = tools.as_array().map(|l| l.iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    crate::nixa_tools::exposure::saw_bridge(app, names);
    tools
}

async fn list(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !allowed(&app, &headers) {
        return refused();
    }
    axum::Json(json!({"ok": true, "tools": served_list(&app)})).into_response()
}

pub async fn run_tool(app: &Arc<App>, name: &str, args: &Value) -> (String, bool) {
    lock(&app.cli.live).turn_used_tools = true;
    let call = ToolCall { id: format!("bridge-{}", CALLS.fetch_add(1, Ordering::Relaxed) + 1), name: name.to_string(), args: args_of(args) };
    let verdict = lock(&app.brain).guard.check(&call.name, &call.args);
    match verdict {
        Verdict::Ok => {
            let (text, error) = crate::tools::call(app, &call).await;
            (clip_note(&text, TOOL_CHARS), error)
        }
        Verdict::Stop { warn, reason } => (loop_note(warn, &reason), true),
    }
}

async fn call(State(app): State<Arc<App>>, headers: HeaderMap, body: Option<axum::Json<Value>>) -> Response {
    if !allowed(&app, &headers) {
        return refused();
    }
    let body = body.map(|b| b.0).unwrap_or(Value::Null);
    let Some(name) = body.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) else {
        return (StatusCode::BAD_REQUEST, axum::Json(json!({"ok": false, "reason": "name is needed"}))).into_response();
    };
    let args = body.get("args").cloned().unwrap_or(Value::Null);
    let (text, error) = run_tool(&app, name, &args).await;
    axum::Json(json!({"ok": !error, "text": text})).into_response()
}

type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

pub const TERMINAL_DENIED: [&str; 2] = [crate::prompts::tools::ANSWER, crate::manage::go_live::NAME];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Caller {
    Window,
    Terminal,
}

async fn agent_caller(app: &App, headers: &HeaderMap, peer: Peer) -> Option<Caller> {
    if crate::window_key::allowed(app, headers) {
        return Some(Caller::Window);
    }
    crate::gate::from_a_terminal(peer.map(|Extension(ConnectInfo(a))| a)).await.then_some(Caller::Terminal)
}

pub fn terminal_refusal(name: &str, args: &Value) -> Option<String> {
    let word = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let risky = TERMINAL_DENIED.contains(&name)
        || (name == crate::manage::settings::NAME && word("action") == "set" && word("key") == "privacy" && word("value") != "on")
        || (name == crate::space_tools::NAME && word("action") == "lock" && args.get("on").is_some_and(|v| v != &json!(true)));
    risky.then(|| format!("{name} with this input is kept for the user. Agent terminals cannot run it. Ask the user to do it in the Nixa window."))
}

fn terminal_list(app: &App) -> Value {
    let all = tool_list(app);
    let kept = all.as_array().map(|l| l.iter().filter(|t| !TERMINAL_DENIED.contains(&t["name"].as_str().unwrap_or(""))).cloned().collect()).unwrap_or_default();
    Value::Array(kept)
}

async fn agent_list(State(app): State<Arc<App>>, headers: HeaderMap, peer: Peer) -> Response {
    let tools = match agent_caller(&app, &headers, peer).await {
        Some(Caller::Window) => tool_list(&app),
        Some(Caller::Terminal) => terminal_list(&app),
        None => return crate::window_key::refused(),
    };
    axum::Json(json!({"ok": true, "tools": tools})).into_response()
}

pub async fn run_agent_tool(app: &Arc<App>, name: &str, args: &Value) -> (String, bool) {
    if !crate::tools::specs(app).iter().any(|s| s.name == name) {
        return (format!("No Nixa tool named {name}. Run nixa tool list."), true);
    }
    let call = ToolCall { id: format!("agent-{}", CALLS.fetch_add(1, Ordering::Relaxed) + 1), name: name.to_string(), args: args_of(args) };
    let (text, error) = crate::tools::call(app, &call).await;
    (clip_note(&text, TOOL_CHARS), error)
}

async fn agent_call(State(app): State<Arc<App>>, headers: HeaderMap, peer: Peer, body: Option<axum::Json<Value>>) -> Response {
    let Some(who) = agent_caller(&app, &headers, peer).await else {
        return crate::window_key::refused();
    };
    let body = body.map(|b| b.0).unwrap_or(Value::Null);
    let Some(name) = body.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) else {
        return (StatusCode::BAD_REQUEST, axum::Json(json!({"ok": false, "reason": "name is needed"}))).into_response();
    };
    let args = body.get("args").cloned().unwrap_or(Value::Null);
    if who == Caller::Terminal
        && let Some(why) = terminal_refusal(name, &args)
    {
        return axum::Json(json!({"ok": false, "refused": true, "text": why})).into_response();
    }
    let (text, error) = run_agent_tool(&app, name, &args).await;
    axum::Json(json!({"ok": !error, "text": text})).into_response()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_cannot_run_user_only_tools() {
        for (name, args) in [
            (crate::prompts::tools::ANSWER, json!({"id": "p1", "choice": "yes"})),
            (crate::manage::go_live::NAME, json!({})),
            (crate::manage::settings::NAME, json!({"action": "set", "key": "privacy", "value": "off"})),
            (crate::space_tools::NAME, json!({"action": "lock", "space": "home", "on": false})),
        ] {
            assert!(terminal_refusal(name, &args).is_some(), "{name} {args}");
        }
        for (name, args) in [
            (crate::manage::settings::NAME, json!({"action": "set", "key": "privacy", "value": "on"})),
            (crate::manage::settings::NAME, json!({"action": "get"})),
            (crate::space_tools::NAME, json!({"action": "lock", "space": "home", "on": true})),
            (crate::actions::tool::NAME, json!({"action": "list"})),
        ] {
            assert!(terminal_refusal(name, &args).is_none(), "{name} {args}");
        }
    }

    #[test]
    fn compares_tokens() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "ab"));
        assert!(!same("", "a"));
    }

    #[tokio::test]
    async fn checks_token_and_lists_tools() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let mut headers = HeaderMap::new();
        assert!(!allowed(&app, &headers));
        headers.insert(TOKEN_HEADER, "wrong".parse().unwrap());
        assert!(!allowed(&app, &headers));
        headers.insert(TOKEN_HEADER, app.cli.token.parse().unwrap());
        assert!(allowed(&app, &headers));
        let tools = tool_list(&app);
        let names: Vec<&str> = tools.as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"nixa_term_list"));
        assert!(tools[0]["inputSchema"].is_object());
        let (text, error) = run_tool(&app, "no_such_tool", &json!({})).await;
        assert!(error);
        assert!(text.contains("no tool named"), "{text}");
    }
}
