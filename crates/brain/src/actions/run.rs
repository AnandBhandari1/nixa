use super::{Action, Body};
use crate::app::App;
use axum::body::Body as HttpBody;
use axum::http::{Request, StatusCode};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use tower::ServiceExt;

pub const MAX_REPLY: usize = 16 * 1024 * 1024;

pub const USER_ONLY: [&str; 17] = [
    "browser.command",
    "browser.download",
    "privacy.taint.clear",
    "privacy.check.save",
    "privacy.owner.save",
    "decide.save",
    "prompt.answer",
    "desktop.answer",
    "connector.answer",
    "connector.trust",
    "workflow.answer",
    "chat.say",
    "brain.save",
    "phone.new_key",
    "phone.status",
    "web.set",
    "switches.keyword_clear",
];

pub const SAFE_WAY: [(&str, &str, bool); 5] = [
    ("switches.privacy", "on", true),
    ("browser.on", "on", false),
    ("phone.door", "on", false),
    ("phone.mobile", "on", false),
    ("task.space.set", "locked", true),
];

pub fn user_only(name: &str, input: &Value) -> Option<String> {
    let risky = USER_ONLY.contains(&name)
        || SAFE_WAY.iter().any(|(action, key, safe)| *action == name && input.get(*key).is_some_and(|v| v != &json!(*safe)));
    risky.then(|| {
        format!("{name} is kept for the user: it changes privacy, consent, a yes card or what runs on this Mac. Nixa tools do not run it. Ask the user to do it in the Nixa window.")
    })
}

pub async fn run(app: &Arc<App>, name: &str, input: &Value) -> (StatusCode, Value) {
    let Some(action) = super::find(name) else {
        return fail(StatusCode::NOT_FOUND, format!("No action named {name}. Run nixa do list."));
    };
    let input = match input {
        Value::Null => Map::new(),
        Value::Object(map) => map.clone(),
        _ => return fail(StatusCode::BAD_REQUEST, "The input must be a JSON object."),
    };
    let unknown = unknown(&action.inputs, &input);
    if !unknown.is_empty() {
        return fail(StatusCode::BAD_REQUEST, format!("{name} does not take {}. Run nixa do show {name}.", unknown.join(", ")));
    }
    let missing = missing(&action.inputs, &input, "");
    if !missing.is_empty() {
        return fail(StatusCode::BAD_REQUEST, format!("Give {}. Run nixa do show {name}.", missing.join(", ")));
    }
    let (path, rest) = match fill_path(&action, &input) {
        Ok(done) => done,
        Err(error) => return fail(StatusCode::BAD_REQUEST, error),
    };
    let request = match request(app, &action, &path, rest) {
        Ok(request) => request,
        Err(error) => return fail(StatusCode::BAD_REQUEST, error),
    };
    let response = match crate::server::router(app.clone()).oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    };
    let code = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), MAX_REPLY).await.unwrap_or_default();
    let reply = serde_json::from_slice::<Value>(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()));
    let ok = code.is_success() && reply.get("ok").and_then(Value::as_bool) != Some(false);
    let mut out = json!({"ok": ok, "name": name, "status": code.as_u16(), "reply": reply});
    if !ok {
        out["error"] = json!(reply.get("error").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("{name} answered {}", code.as_u16())));
    }
    (StatusCode::OK, out)
}

fn fail(code: StatusCode, error: impl Into<String>) -> (StatusCode, Value) {
    (code, json!({"ok": false, "error": error.into()}))
}

pub fn unknown(schema: &Value, input: &Map<String, Value>) -> Vec<String> {
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(true) {
        return Vec::new();
    }
    let props = schema.get("properties").and_then(Value::as_object);
    input.keys().filter(|k| props.is_none_or(|p| !p.contains_key(k.as_str()))).cloned().collect()
}

pub fn missing(schema: &Value, input: &Map<String, Value>, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let required = schema.get("required").and_then(Value::as_array).cloned().unwrap_or_default();
    for key in required.iter().filter_map(Value::as_str) {
        if input.get(key).is_none_or(Value::is_null) {
            out.push(format!("{prefix}{key}"));
        }
    }
    let props = schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
    for (key, inner) in props {
        if let Some(Value::Object(given)) = input.get(&key) {
            out.extend(missing(&inner, given, &format!("{prefix}{key}.")));
        }
    }
    out
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn lookup<'a>(input: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    let mut parts = key.split('.');
    let mut at = input.get(parts.next()?)?;
    for part in parts {
        at = at.get(part)?;
    }
    Some(at)
}

pub fn card_text(template: &str, input: &Map<String, Value>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let key = after[..close].trim();
        out.push_str(&lookup(input, key).map(text_of).unwrap_or_default());
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn fill_path(action: &Action, input: &Map<String, Value>) -> Result<(String, Map<String, Value>), String> {
    let mut rest = input.clone();
    let mut path = action.path.to_string();
    for param in action.params() {
        let value = rest.remove(param).map(|v| text_of(&v)).filter(|v| !v.trim().is_empty()).ok_or_else(|| format!("Give {param}."))?;
        path = path.replace(&format!("{{{param}}}"), &encode(value.trim()));
    }
    Ok((path, rest))
}

fn body_of(body: Body, rest: Map<String, Value>) -> Value {
    match body {
        Body::Raw => Value::Object(rest),
        Body::Payload(action) => json!({"action": action, "payload": rest}),
        Body::Field(key, value) => {
            let mut out = rest;
            out.insert(key.to_string(), json!(value));
            Value::Object(out)
        }
    }
}

fn query_of(body: &Value) -> String {
    let Some(map) = body.as_object() else { return String::new() };
    map.iter().map(|(k, v)| format!("{}={}", encode(k), encode(&text_of(v)))).collect::<Vec<_>>().join("&")
}

fn request(app: &App, action: &Action, path: &str, rest: Map<String, Value>) -> Result<Request<HttpBody>, String> {
    let body = body_of(action.body, rest);
    let in_query = matches!(action.method, "GET" | "DELETE");
    let query = if in_query { query_of(&body) } else { String::new() };
    let uri = if query.is_empty() { path.to_string() } else { format!("{path}?{query}") };
    let data = if in_query { HttpBody::empty() } else { HttpBody::from(body.to_string()) };
    Request::builder()
        .method(action.method)
        .uri(uri)
        .header("host", "127.0.0.1")
        .header("content-type", "application/json")
        .header(crate::window_key::HEADER, app.window_token.as_str())
        .body(data)
        .map_err(|e| format!("The request could not be built: {e}"))
}
