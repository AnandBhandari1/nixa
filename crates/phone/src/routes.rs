use crate::door::{Door, Pair};
use crate::secret::is_key;
use crate::wire::MAX_MESSAGE;
use crate::{log, socket};
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::extract::ws::{WebSocketUpgrade, rejection::WebSocketUpgradeRejection};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;

pub const ROOT_PATH: &str = "/nixa-root.crt";
pub const ROOT_FILE_NAME: &str = "Nixa-root.crt";

pub(crate) fn door_router(door: Door) -> Router {
    Router::new().route("/ws", any(ws_route)).fallback(door_route).with_state(door)
}

pub(crate) fn setup_router(door: Door) -> Router {
    Router::new().fallback(setup_route).with_state(door)
}

fn text(headers: &HeaderMap, name: header::HeaderName) -> &str {
    headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
}

fn guarded(status: StatusCode, kind: &'static str, body: Body) -> Response {
    let mut res = Response::new(body);
    *res.status_mut() = status;
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    res
}

fn plain(status: StatusCode, body: &'static str) -> Response {
    guarded(status, "text/plain; charset=utf-8", Body::from(body))
}

fn not_paired() -> Response {
    plain(StatusCode::NOT_FOUND, "This phone is not paired with Nixa.")
}

fn paired(cookie: Option<String>) -> Response {
    let mut res = plain(StatusCode::OK, "Paired with Nixa.");
    if let Some(value) = cookie.and_then(|c| HeaderValue::from_str(&c).ok()) {
        res.headers_mut().insert(header::SET_COOKIE, value);
    }
    res
}

async fn ws_route(
    State(door): State<Door>,
    method: Method,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let host = text(&headers, header::HOST);
    let phone = door.phone_of(text(&headers, header::COOKIE));
    let origin_ok = text(&headers, header::ORIGIN) == format!("https://{host}");
    let (Some(phone), true, true, true) = (phone, method == Method::GET, door.has_host(host), origin_ok) else {
        log("phone", "socket refused");
        return StatusCode::FORBIDDEN.into_response();
    };
    match ws {
        Ok(ws) => ws
            .max_message_size(MAX_MESSAGE)
            .max_frame_size(MAX_MESSAGE)
            .on_upgrade(move |socket| socket::run(door, socket, phone)),
        Err(_) => not_paired(),
    }
}

async fn door_route(State(door): State<Door>, method: Method, uri: Uri, headers: HeaderMap) -> Response {
    if method != Method::GET || !door.has_host(text(&headers, header::HOST)) {
        return not_paired();
    }
    let Some(key) = uri.path().strip_prefix("/k/").filter(|key| is_key(key)) else {
        return not_paired();
    };
    match door.pair(key, text(&headers, header::COOKIE), text(&headers, header::USER_AGENT)) {
        Pair::Wrong => not_paired(),
        Pair::Already => paired(None),
        Pair::New(cookie) => paired(Some(cookie)),
    }
}

async fn setup_route(State(door): State<Door>, method: Method, uri: Uri, headers: HeaderMap) -> Response {
    let wanted = method == Method::GET && uri.path() == ROOT_PATH && door.has_setup_host(text(&headers, header::HOST));
    match (wanted, door.setup_view()) {
        (true, Some(der)) => {
            let mut res = guarded(StatusCode::OK, "application/x-x509-ca-cert", Body::from(der));
            let name = format!("attachment; filename=\"{ROOT_FILE_NAME}\"");
            if let Ok(value) = HeaderValue::from_str(&name) {
                res.headers_mut().insert(header::CONTENT_DISPOSITION, value);
            }
            res
        }
        _ => plain(StatusCode::NOT_FOUND, "Not found"),
    }
}
