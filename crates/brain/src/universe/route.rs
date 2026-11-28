use crate::app::App;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use nixa_protocol::{Reply, Request, TermId, TermInfo, client};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const ROUTE: &str = "/universe/flow";
pub const ENGINE_MS: u64 = 3000;

type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

pub fn routes() -> Router<Arc<App>> {
    Router::new().route(ROUTE, post(flow_post))
}

fn bad(code: StatusCode, error: impl Into<String>) -> Response {
    (code, Json(json!({"ok": false, "error": error.into()}))).into_response()
}

pub fn parse_note(body: &Value) -> Result<(TermId, TermId), String> {
    let word = |key: &str| body.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let from = word("terminal").ok_or("terminal is needed: run it from inside a Nixa terminal (NIXA_TERM_ID)")?;
    let to = word("to").ok_or("to is needed: the terminal the message went to")?;
    if from == super::NIXA || to == super::NIXA {
        return Err(format!("{} is not a terminal id. Use space-agent-number (like nixa-claude-12) or a legacy id like m1.", super::NIXA));
    }
    let from: TermId = from.parse().map_err(|_| format!("{from} is not a terminal id. Use space-agent-number (like nixa-claude-12) or a legacy id like m1."))?;
    let to: TermId = to.parse().map_err(|_| format!("{to} is not a terminal id. Use space-agent-number (like nixa-claude-12) or a legacy id like m1."))?;
    if from == to {
        return Err("A terminal does not send to itself.".into());
    }
    Ok((from, to))
}

async fn term(socket: PathBuf, id: TermId) -> Option<TermInfo> {
    tokio::task::spawn_blocking(move || {
        let mut c = client::connect(Path::new(&socket)).ok()?;
        c.set_timeout(Some(Duration::from_millis(ENGINE_MS))).ok()?;
        match c.call(&Request::Get { id }).ok()? {
            Reply::Term { term } => Some(term),
            _ => None,
        }
    })
    .await
    .ok()
    .flatten()
}

async fn flow_post(State(app): State<Arc<App>>, peer: Peer, Json(body): Json<Value>) -> Response {
    let (from, to) = match parse_note(&body) {
        Ok(pair) => pair,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error),
    };
    let socket = app.universe.socket();
    let Some(sender) = term(socket.clone(), from.clone()).await.filter(|t| t.running) else {
        return bad(StatusCode::FORBIDDEN, format!("Terminal {from} is not a running Nixa terminal."));
    };
    if term(socket, to.clone()).await.is_none() {
        return bad(StatusCode::NOT_FOUND, format!("Terminal {to} is not a Nixa terminal."));
    }
    let name = from.to_string();
    if let Err(error) = crate::extras::peer::from_terminal(peer.map(|Extension(ConnectInfo(a))| a), &name, sender.pid).await {
        return bad(StatusCode::FORBIDDEN, error);
    }
    let shown = super::flow(&app, &name, &to.to_string());
    (StatusCode::OK, Json(json!({"ok": true, "shown": shown}))).into_response()
}
