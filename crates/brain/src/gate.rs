use crate::app::App;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;

pub const PUBLIC: [&str; 1] = ["/health"];
pub const HEADER: &str = nixa_protocol::paths::GATE_TOKEN_HEADER;
pub const FILE: &str = nixa_protocol::paths::GATE_TOKEN_FILE;
const LOCAL_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

pub fn local_host(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    let split = match value.find(']') {
        Some(end) if value.starts_with('[') => end + 1,
        _ => value.find(':').unwrap_or(value.len()),
    };
    let (name, rest) = value.split_at(split);
    let port_ok = rest.is_empty()
        || rest.strip_prefix(':').is_some_and(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    LOCAL_HOSTS.contains(&name) && port_ok
}

fn text(headers: &HeaderMap, name: impl axum::http::header::AsHeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

pub fn origin_ok(headers: &HeaderMap) -> bool {
    if !headers.contains_key(header::ORIGIN) {
        return true;
    }
    let host = text(headers, header::HOST).unwrap_or("");
    let own = format!("http://{host}");
    !host.is_empty() && headers.get_all(header::ORIGIN).iter().all(|v| v.to_str().is_ok_and(|o| o.eq_ignore_ascii_case(&own)))
}

pub fn open(dir: &std::path::Path) -> String {
    let token = crate::cli_brains::new_token();
    if let Err(e) = crate::util::atomic_write(&dir.join(FILE), &token) {
        crate::util::log("brain", &format!("the gate token file was not written: {e}"));
    }
    token
}

pub fn allowed(app: &App, headers: &HeaderMap) -> bool {
    let gate = text(headers, HEADER).unwrap_or("");
    !app.gate_token.is_empty() && crate::cli_brains::door::same(gate, &app.gate_token)
}

pub fn has_token(app: &App, headers: &HeaderMap) -> bool {
    allowed(app, headers) || crate::window_key::allowed(app, headers) || crate::cli_brains::door::allowed(app, headers)
}

pub async fn from_a_terminal(peer: Option<SocketAddr>) -> bool {
    use crate::extras::peer;
    let Some(addr) = peer.filter(|a| a.ip().is_loopback()) else { return false };
    let Some(pid) = peer::peer_pid(u32::from(addr.port()), std::process::id()).await else { return false };
    let Ok(terms) = crate::engine::list().await else { return false };
    let roots: Vec<u32> = terms.iter().filter(|t| t.running).filter_map(|t| t.pid).filter(|p| *p > 1).collect();
    if roots.is_empty() {
        return false;
    }
    peer::parents().await.is_some_and(|table| peer::chain_of(&table, pid).iter().any(|p| roots.contains(p)))
}

fn refuse(code: StatusCode, message: &str) -> Response {
    (code, axum::Json(json!({"ok": false, "error": message}))).into_response()
}

pub async fn check(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    if !text(headers, header::HOST).is_some_and(local_host) {
        return refuse(StatusCode::FORBIDDEN, "Nixa answers only calls made to 127.0.0.1 or localhost.");
    }
    if !origin_ok(headers) {
        return refuse(StatusCode::FORBIDDEN, "Nixa does not answer calls from web pages.");
    }
    if PUBLIC.contains(&request.uri().path()) || has_token(&app, headers) {
        return next.run(request).await;
    }
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    if from_a_terminal(peer).await {
        return next.run(request).await;
    }
    refuse(StatusCode::UNAUTHORIZED, "A Nixa token or a call from a Nixa terminal is needed.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    const ROUTES: [(&str, &str); 96] = [
        ("GET", "/phone"),
        ("POST", "/phone"),
        ("GET", "/health"),
        ("GET", "/chat"),
        ("POST", "/chat"),
        ("POST", "/chat/interrupt"),
        ("POST", "/chat/clean"),
        ("GET", "/context"),
        ("POST", "/compact"),
        ("GET", "/brains"),
        ("POST", "/brains"),
        ("DELETE", "/brains/1"),
        ("POST", "/brain/choice"),
        ("GET", "/brain/models"),
        ("POST", "/brain/model"),
        ("GET", "/decision"),
        ("POST", "/decision"),
        ("POST", "/decision/check"),
        ("GET", "/memory"),
        ("POST", "/memory"),
        ("DELETE", "/memory/1"),
        ("GET", "/deep"),
        ("POST", "/deep/join"),
        ("POST", "/deep/leave"),
        ("POST", "/deep/start"),
        ("POST", "/deep/ask"),
        ("POST", "/deep/digest"),
        ("POST", "/deep/1/reply"),
        ("GET", "/trees"),
        ("GET", "/trees/x"),
        ("PUT", "/trees/x"),
        ("DELETE", "/trees/x"),
        ("POST", "/trees/x/undo"),
        ("POST", "/watch/dry"),
        ("GET", "/settings"),
        ("POST", "/settings"),
        ("GET", "/ws"),
        ("POST", "/brain/ollama"),
        ("POST", "/agent/trees"),
        ("GET", "/decide"),
        ("POST", "/decide"),
        ("POST", "/decide/test"),
        ("GET", "/modes"),
        ("GET", "/modes/journal"),
        ("GET", "/modes/changes"),
        ("GET", "/modes/scores"),
        ("GET", "/modes/records"),
        ("GET", "/modes/complaints"),
        ("GET", "/workflows"),
        ("POST", "/workflows"),
        ("GET", "/desktop"),
        ("POST", "/desktop/answer"),
        ("POST", "/desktop/reads"),
        ("GET", "/content/x"),
        ("POST", "/content/x"),
        ("GET", "/answer-rules"),
        ("POST", "/answer-rules"),
        ("POST", "/answer-rules/check"),
        ("POST", "/answer-rules/1/remove"),
        ("GET", "/models"),
        ("GET", "/models/view"),
        ("POST", "/models/x"),
        ("GET", "/tools"),
        ("POST", "/tool"),
        ("GET", "/agent/tools"),
        ("POST", "/agent/tool"),
        ("GET", "/voice"),
        ("POST", "/voice"),
        ("GET", "/voice/devices"),
        ("GET", "/teams"),
        ("POST", "/teams"),
        ("GET", "/actions"),
        ("POST", "/actions/run"),
        ("GET", "/tasks/queue"),
        ("GET", "/tasks/next"),
        ("GET", "/tasks/spaces"),
        ("POST", "/tasks/spaces/x"),
        ("POST", "/tasks/resume"),
        ("POST", "/tasks/changed"),
        ("POST", "/tasks/1/start"),
        ("POST", "/tasks/1/continue"),
        ("POST", "/tasks/1/checkpoint"),
        ("GET", "/tasks/1/plan"),
        ("POST", "/tasks/1/plan"),
        ("GET", "/custom-agents"),
        ("POST", "/custom-agents/x"),
        ("GET", "/prompts"),
        ("POST", "/prompts/event"),
        ("POST", "/prompts/1/answer"),
        ("GET", "/drones"),
        ("GET", "/drones/x"),
        ("POST", "/drones/x"),
        ("POST", "/universe/flow"),
        ("GET", "/agents/leave"),
        ("POST", "/agents/limit"),
        ("POST", "/agents/handover"),
    ];

    async fn status(app: &Arc<App>, request: HttpRequest<Body>) -> u16 {
        crate::server::router(app.clone()).oneshot(request).await.unwrap().status().as_u16()
    }

    fn ask(method: &str, path: &str) -> axum::http::request::Builder {
        HttpRequest::builder().method(method).uri(path).header("host", "127.0.0.1:7421").header("content-type", "application/json")
    }

    #[tokio::test]
    async fn the_gate_token_is_its_own_and_gives_no_window_power() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        assert!(app.gate_token.len() >= 32);
        assert_ne!(app.gate_token, app.window_token);
        assert_eq!(nixa_protocol::paths::token_in(dir.path(), FILE).as_deref(), Some(app.gate_token.as_str()));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.path().join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        let old = ask("GET", "/chat").header(HEADER, &app.window_token).body(Body::empty()).unwrap();
        assert_eq!(status(&app, old).await, 401);
        let window_only = ask("GET", "/actions").header(HEADER, &app.gate_token).body(Body::empty()).unwrap();
        assert_eq!(status(&app, window_only).await, 401);
        let window_run = ask("POST", "/actions/run").header(HEADER, &app.gate_token).body(Body::from(r#"{"name":"browser.command","input":{"command":"/bin/sh"}}"#)).unwrap();
        assert_eq!(status(&app, window_run).await, 401);
        let agent = ask("GET", "/agent/tools").header(HEADER, &app.gate_token).body(Body::empty()).unwrap();
        assert_eq!(status(&app, agent).await, 401);
    }

    #[test]
    fn only_local_hosts_pass() {
        for good in ["127.0.0.1:7421", "localhost:7421", "LOCALHOST", "[::1]:7421", "127.0.0.1"] {
            assert!(local_host(good), "{good}");
        }
        for bad in ["evil.example:7421", "127.0.0.1.evil.example", "localhost.evil:7421", "127.0.0.1:", "127.0.0.1:x", "[::2]:7421", ""] {
            assert!(!local_host(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn every_route_needs_a_token_except_health() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        for (method, path) in ROUTES {
            let code = status(&app, ask(method, path).body(Body::from("{}")).unwrap()).await;
            if PUBLIC.contains(&path) {
                assert_eq!(code, 200, "{method} {path}");
            } else {
                assert!(code == 401 || code == 403, "{method} {path} gave {code}");
            }
        }
        let answer = ask("POST", "/prompts/p1/answer").body(Body::from(r#"{"choice":"yes","decide":true,"reason":"x"}"#)).unwrap();
        assert_eq!(status(&app, answer).await, 401);
        assert_eq!(status(&app, ask("GET", "/no/such/route").body(Body::empty()).unwrap()).await, 401);
    }

    #[tokio::test]
    async fn a_token_opens_the_gate_but_a_bad_host_or_origin_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let token = app.window_token.clone();
        let with = |b: axum::http::request::Builder| b.header(crate::window_key::HEADER, &token).body(Body::empty()).unwrap();
        assert_eq!(status(&app, with(ask("GET", "/chat"))).await, 200);
        let gate = ask("GET", "/chat").header(HEADER, &app.gate_token).body(Body::empty()).unwrap();
        assert_eq!(status(&app, gate).await, 200);
        let tool = ask("GET", "/chat").header(crate::cli_brains::TOKEN_HEADER, &app.cli.token).body(Body::empty()).unwrap();
        assert_eq!(status(&app, tool).await, 200);
        let wrong = ask("GET", "/chat").header(HEADER, "wrong").body(Body::empty()).unwrap();
        assert_eq!(status(&app, wrong).await, 401);
        let rebound = HttpRequest::builder().method("POST").uri("/chat/interrupt").header("host", "evil.example:7421");
        assert_eq!(status(&app, with(rebound)).await, 403);
        let no_host = HttpRequest::builder().method("GET").uri("/health");
        assert_eq!(status(&app, no_host.body(Body::empty()).unwrap()).await, 403);
        let foreign = ask("POST", "/compact").header("origin", "https://evil.example");
        assert_eq!(status(&app, with(foreign)).await, 403);
        let null = ask("POST", "/compact").header("origin", "null");
        assert_eq!(status(&app, with(null)).await, 403);
        let other_port = ask("GET", "/chat").header("origin", "http://localhost:3000");
        assert_eq!(status(&app, with(other_port)).await, 403);
        let same = ask("GET", "/chat").header("origin", "http://127.0.0.1:7421");
        assert_eq!(status(&app, with(same)).await, 200);
        let two = ask("GET", "/chat").header("origin", "http://127.0.0.1:7421").header("origin", "https://evil.example");
        assert_eq!(status(&app, with(two)).await, 403);
    }
}
