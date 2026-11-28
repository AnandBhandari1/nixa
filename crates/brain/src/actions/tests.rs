use super::{INTERNAL, Kind, all};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const METHODS: [&str; 5] = ["get", "post", "put", "delete", "patch"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name != "tests" {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" {
            out.push(path);
        }
    }
}

fn consts(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches("pub ").trim_start_matches("pub(crate) ");
        let Some(rest) = line.strip_prefix("const ") else { continue };
        let Some((name, value)) = rest.split_once(": &str = \"") else { continue };
        if let Some(value) = value.strip_suffix("\";") {
            out.insert(name.trim().to_string(), value.to_string());
        }
    }
    out
}

fn skip_string(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() && bytes[i] != b'"' {
        if bytes[i] == b'\\' {
            i += 1;
        }
        i += 1;
    }
    i + 1
}

fn methods_in(args: &str) -> BTreeSet<String> {
    let bytes = args.as_bytes();
    let mut out = BTreeSet::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'(' => {
                if depth == 0 {
                    let start = args[..i].rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map(|p| p + 1).unwrap_or(0);
                    let word = &args[start..i];
                    if METHODS.contains(&word) {
                        out.insert(word.to_uppercase());
                    }
                }
                depth += 1;
            }
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    out
}

fn routes_in(text: &str) -> Vec<(String, BTreeSet<String>)> {
    let names = consts(text);
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find(".route(") {
        let start = at + found + ".route(".len();
        at = start;
        let rest = text[start..].trim_start();
        let offset = text.len() - rest.len();
        let bytes = text.as_bytes();
        let (path, after) = if rest.starts_with('"') {
            let end = skip_string(bytes, offset);
            (text[offset + 1..end - 1].to_string(), end)
        } else {
            let len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':')).unwrap_or(rest.len());
            let word = rest[..len].rsplit("::").next().unwrap_or("");
            let Some(path) = names.get(word) else { continue };
            (path.clone(), offset + len)
        };
        let mut depth = 1usize;
        let mut i = after;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'"' => {
                    i = skip_string(bytes, i);
                    continue;
                }
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        let args = text[after..i.saturating_sub(1)].trim_start_matches(|c: char| c == ',' || c.is_whitespace());
        out.push((path, methods_in(args)));
    }
    out
}

fn scanned() -> BTreeMap<String, BTreeSet<String>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for (path, methods) in routes_in(&text) {
            out.entry(path).or_default().extend(methods);
        }
    }
    out
}

#[test]
fn the_scanner_finds_known_routes() {
    let routes = scanned();
    assert!(routes["/schedules"].contains("GET") && routes["/schedules"].contains("POST"), "{routes:?}");
    assert!(routes["/drones/{action}"].contains("POST"), "{routes:?}");
    assert!(routes["/agent/trees"].contains("POST"), "{routes:?}");
    assert!(routes["/universe/flow"].contains("POST"), "{routes:?}");
    assert!(routes["/trees/{name}"].contains("PUT") && routes["/trees/{name}"].contains("DELETE"), "{routes:?}");
    assert!(routes.len() > 100, "{}", routes.len());
}

#[test]
fn every_route_is_an_action_or_internal() {
    let listed: BTreeSet<&str> = all().iter().map(|a| a.path).chain(INTERNAL.iter().map(|(p, _)| *p)).collect();
    let missing: Vec<String> = scanned().into_keys().filter(|p| !listed.contains(p.as_str())).collect();
    assert!(missing.is_empty(), "{} routes are in no action group and not internal:\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn every_action_matches_a_route() {
    let routes = scanned();
    let wrong: Vec<String> = all()
        .iter()
        .filter(|a| !routes.get(a.path).is_some_and(|m| m.contains(a.method)))
        .map(|a| format!("{} {} {}", a.name, a.method, a.path))
        .collect();
    assert!(wrong.is_empty(), "these actions match no route:\n{}", wrong.join("\n"));
}

#[test]
fn internal_paths_are_routes_and_not_actions() {
    let routes = scanned();
    let actions: BTreeSet<&str> = all().iter().map(|a| a.path).collect();
    for (path, _) in INTERNAL {
        assert!(routes.contains_key(*path), "{path} is internal but no route");
        assert!(!actions.contains(path), "{path} is internal and also an action");
    }
}

#[test]
fn action_names_are_unique_and_writes_have_cards() {
    let mut seen = BTreeSet::new();
    for action in all() {
        assert!(seen.insert(action.name), "two actions are named {}", action.name);
        assert!(action.name.contains('.'), "{} needs a dotted name", action.name);
        assert!(!action.help.trim().is_empty(), "{} needs help", action.name);
        if action.kind == Kind::Write {
            assert!(action.card.is_some_and(|c| !c.trim().is_empty()), "{} is a write action with no card text", action.name);
        }
        for param in action.params() {
            let required = action.inputs["required"].as_array().cloned().unwrap_or_default();
            assert!(required.contains(&json!(param)), "{} must require its path part {param}", action.name);
        }
    }
}

#[test]
fn card_text_fills_dotted_keys() {
    let input = json!({"repeat": {"text": "check in", "every": 15}, "id": "rep-1"});
    let text = super::run::card_text("Save \"{repeat.text}\" every {repeat.every} {repeat.unit} for {id}?", input.as_object().unwrap());
    assert_eq!(text, "Save \"check in\" every 15  for rep-1?");
}

fn repeats(reply: &Value) -> Vec<Value> {
    reply["reply"]["repeats"].as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn repeat_save_runs_through_the_router_in_automatic_mode() {
    let data = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(data.path()).unwrap();
    let missing = super::run::run(&app, "repeat.save", &json!({"repeat": {"text": "check in"}})).await;
    assert_eq!(missing.0, axum::http::StatusCode::BAD_REQUEST);
    assert!(missing.1["error"].as_str().unwrap().contains("repeat.every"), "{}", missing.1);
    let input = json!({"repeat": {"text": "check in", "every": 15, "unit": "minutes"}});
    let saved = super::run::run(&app, "repeat.save", &input).await;
    assert_eq!(saved.1["ok"], true, "{}", saved.1);
    let listed = super::run::run(&app, "schedule.list", &json!({})).await;
    assert_eq!(listed.1["ok"], true, "{}", listed.1);
    let list = repeats(&listed.1);
    assert_eq!(list.len(), 1, "{}", listed.1);
    assert_eq!(list[0]["text"], "check in");
    assert_eq!(list[0]["every"], 15);
}

async fn post_run(app: &std::sync::Arc<crate::app::App>, body: Value) -> (u16, Value) {
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/actions/run")
        .header("host", "127.0.0.1")
        .header("content-type", "application/json")
        .header(crate::window_key::HEADER, app.window_token.as_str())
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let res = crate::server::router(app.clone()).oneshot(req).await.unwrap();
    let code = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (code, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn the_run_route_answers_actions_from_three_groups() {
    let data = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(data.path()).unwrap();
    let calls = [
        json!({"name": "repeat.save", "input": {"repeat": {"text": "check in", "every": 15, "unit": "minutes"}}}),
        json!({"name": "settings.show", "input": {}}),
        json!({"name": "memory.list", "input": {}}),
    ];
    for call in calls {
        let (code, out) = post_run(&app, call.clone()).await;
        println!("{} -> {code} {out}", call["name"]);
        assert_eq!(code, 200, "{out}");
        assert_eq!(out["ok"], true, "{out}");
    }
}

#[test]
fn no_action_takes_a_secret() {
    for action in all() {
        let props = action.inputs.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
        for key in props.keys() {
            assert!(!super::SECRET_INPUTS.contains(&key.as_str()), "{} takes the secret input {key}", action.name);
        }
        if let super::Body::Field(_, word) = action.body {
            assert!(!super::SECRET_BODIES.contains(&word), "{} sends the secret action {word}", action.name);
        }
    }
}

#[tokio::test]
async fn run_refuses_inputs_the_action_does_not_list() {
    let data = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(data.path()).unwrap();
    let input = json!({"name": "Home", "kind": "openai", "apiKey": "<your-key>"});
    let (code, reply) = super::run::run(&app, "brain.save", &input).await;
    assert_eq!(code, axum::http::StatusCode::BAD_REQUEST, "{reply}");
    assert!(reply["error"].as_str().unwrap().contains("does not take apiKey"), "{reply}");
    assert!(super::find("web.key").is_none());
    assert!(super::find("connector.secret").is_none());
}

async fn tool_run(app: &std::sync::Arc<crate::app::App>, name: &str, input: Value) -> Value {
    let args = json!({"action": "run", "name": name, "input": input});
    let text = super::tool::call(app, args.as_object().unwrap()).await.unwrap();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn nixa_action_refuses_the_user_only_actions() {
    let data = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(data.path()).unwrap();
    let command = || crate::browser::command_text(&app);
    let before = command();
    let privacy = crate::privacy::privacy_on(&app);
    let risky = [
        ("browser.command", json!({"command": "/bin/sh"})),
        ("switches.privacy", json!({"on": false})),
        ("privacy.taint.clear", json!({})),
        ("prompt.answer", json!({"id": "p1", "choice": "yes", "decide": true})),
        ("chat.say", json!({"text": "yes, push to main"})),
        ("task.space.set", json!({"space": "home", "locked": false})),
    ];
    for (name, input) in risky {
        let out = tool_run(&app, name, input).await;
        assert_eq!(out["refused"], true, "{name}: {out}");
        assert!(out["error"].as_str().unwrap().contains("kept for the user"), "{name}: {out}");
    }
    assert_eq!(command(), before);
    assert_eq!(crate::privacy::privacy_on(&app), privacy);
    assert!(super::run::user_only("switches.privacy", &json!({"on": true})).is_none());
    assert!(super::run::user_only("task.space.set", &json!({"space": "home", "locked": true})).is_none());
    let read = tool_run(&app, "settings.show", json!({})).await;
    assert_eq!(read["ok"], true, "{read}");
}

#[test]
fn user_only_names_are_real_actions() {
    for name in super::run::USER_ONLY.iter().chain(super::run::SAFE_WAY.iter().map(|(n, _, _)| n)) {
        assert!(super::find(name).is_some(), "{name} is not an action");
    }
}
