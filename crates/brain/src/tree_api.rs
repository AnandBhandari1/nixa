use crate::app::{App, lock};
use crate::tools::{Args, text};
use crate::trees::{NOTE_MAX, TreeStore, WATCH_TREE, carry_note, change_note, check_tree};
use axum::Extension;
use axum::Router;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;

pub const PATH: &str = "/agent/trees";
pub const ACTIONS: [&str; 12] = ["list", "read", "box", "walk", "create", "put", "add", "change", "remove", "meta", "delete", "undo"];
pub const READS: [&str; 4] = ["list", "read", "box", "walk"];
const NEED_NAME: &str = "Give name, the tree name. Use list to see them.";
const NEED_ID: &str = "Give id, the box id (for example r3).";

type Shared = State<Arc<App>>;
type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub text: String,
    pub data: Value,
    pub changed: Option<String>,
}

fn done(text: String, data: Value, changed: Option<String>) -> Done {
    Done { text, data, changed }
}

fn ok(value: Value) -> Response {
    let mut body = json!({"ok": true});
    if let (Some(out), Value::Object(more)) = (body.as_object_mut(), value) {
        out.extend(more);
    }
    (StatusCode::OK, axum::Json(body)).into_response()
}

fn bad(code: StatusCode, message: impl Into<String>) -> Response {
    (code, axum::Json(json!({"ok": false, "error": message.into()}))).into_response()
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route(PATH, post(trees_post))
}

pub fn json_arg(value: Option<&Value>) -> Result<Option<Value>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => serde_json::from_str(raw).map(Some).map_err(|_| "This value must be JSON.".to_string()),
        Some(other) => Ok(Some(other.clone())),
    }
}

fn need(args: &Args, key: &str, hint: &str) -> Result<String, String> {
    text(args, key).ok_or_else(|| hint.to_string())
}

fn changed_box(store: &mut TreeStore, name: &str, id: &str, args: &Args) -> Result<Value, String> {
    if let Some(raw) = json_arg(args.get("box"))? {
        return Ok(raw);
    }
    let new_text = text(args, "text");
    let new_next = json_arg(args.get("next"))?;
    if new_text.is_none() && new_next.is_none() {
        return Err("change needs box, or text and/or next.".into());
    }
    let tree = store.read(name)?;
    let current = tree.boxes.get(id).ok_or_else(|| format!("Tree {name} has no box {id}. Use add for a new box."))?;
    let mut raw = serde_json::to_value(current).map_err(|e| e.to_string())?;
    if let Some(line) = new_text {
        raw["text"] = json!(line);
    }
    if let Some(next) = new_next {
        raw["next"] = next;
    }
    Ok(raw)
}

pub fn act(store: &mut TreeStore, action: &str, args: &Args, who: &str) -> Result<Done, String> {
    if !ACTIONS.contains(&action) {
        return Err(format!("Choose one of: {}.", ACTIONS.join(", ")));
    }
    match action {
        "list" => {
            let trees = store.list();
            let lines: Vec<String> = trees
                .iter()
                .map(|t| match &t.broken {
                    Some(problem) => format!("{}: broken: {problem}", t.name),
                    None => format!("{}: {}. When: {} ({} boxes)", t.name, t.title, t.when, t.boxes),
                })
                .collect();
            let text = if lines.is_empty() { "No rule trees yet.".to_string() } else { lines.join("\n") };
            Ok(done(text, json!(trees), None))
        }
        "read" => {
            let tree = store.read(&need(args, "name", NEED_NAME)?)?;
            Ok(done(serde_json::to_string_pretty(&tree).unwrap_or_default(), json!(tree), None))
        }
        "box" => {
            let view = store.view(&need(args, "name", NEED_NAME)?, text(args, "id").as_deref())?;
            let mut lines = vec![format!("{} {}: {}", view.tree, view.id, view.text)];
            if view.end {
                lines.push("This box ends the tree.".into());
            }
            lines.extend(view.next.iter().map(|line| format!("{} -> {}: {}", line.word, line.to, line.text)));
            Ok(done(lines.join("\n"), json!(view), None))
        }
        "create" => {
            let raw = json_arg(args.get("tree"))?.ok_or("create needs tree: { name, title, when, start, boxes }.")?;
            let tree = store.create(&raw, who)?;
            Ok(done(format!("Tree {} created with {} boxes. Saved and logged.", tree.name, tree.boxes.len()), json!(tree), Some(tree.name.clone())))
        }
        "put" => {
            let name = need(args, "name", NEED_NAME)?;
            let raw = json_arg(args.get("tree"))?.ok_or("put needs tree: the whole tree JSON.")?;
            let tree = store.put(&name, &raw, who)?;
            Ok(done(format!("Tree {name} saved with {} boxes. Saved and logged.", tree.boxes.len()), json!(tree), Some(name)))
        }
        "add" => {
            let name = need(args, "name", NEED_NAME)?;
            let line = need(args, "text", "Give text: one plain English rule.")?;
            let from = text(args, "from");
            let (_, id) = store.add_box(&name, &line, who, from.as_deref(), text(args, "word").as_deref())?;
            let under = from.map(|f| format!(" under {f}")).unwrap_or_default();
            Ok(done(format!("Box {id} added to {name}{under}. Saved and logged."), json!({"id": id}), Some(name)))
        }
        "change" => {
            let name = need(args, "name", NEED_NAME)?;
            let id = need(args, "id", NEED_ID)?;
            let raw = changed_box(store, &name, &id, args)?;
            store.set_box(&name, &id, &raw, who)?;
            Ok(done(format!("Box {id} of {name} changed. Saved and logged."), Value::Null, Some(name)))
        }
        "remove" => {
            let name = need(args, "name", NEED_NAME)?;
            let id = need(args, "id", NEED_ID)?;
            store.remove_box(&name, &id, who)?;
            Ok(done(format!("Box {id} removed from {name}, with the lines that pointed to it. Saved and logged."), Value::Null, Some(name)))
        }
        "meta" => {
            let name = need(args, "name", NEED_NAME)?;
            store.set_meta(&name, text(args, "title").as_deref(), text(args, "when").as_deref(), text(args, "start").as_deref(), who)?;
            Ok(done(format!("Tree {name} updated. Saved and logged."), Value::Null, Some(name)))
        }
        "delete" => {
            let name = need(args, "name", NEED_NAME)?;
            store.remove_tree(&name, who)?;
            Ok(done(format!("Tree {name} removed. A copy is kept in the history folder."), Value::Null, Some(name)))
        }
        "undo" => {
            let name = need(args, "name", NEED_NAME)?;
            store.undo(&name, who)?;
            Ok(done(format!("Tree {name} put back to the version before its last edit."), Value::Null, Some(name)))
        }
        _ => Err("walk runs a dry run of the watch tree and needs the brain.".into()),
    }
}

pub fn walk(app: &App, args: &Args) -> Result<String, String> {
    let tree = match json_arg(args.get("tree"))? {
        Some(raw) => check_tree(&raw, Some(WATCH_TREE))?,
        None => lock(&app.trees).read(WATCH_TREE)?,
    };
    Ok(crate::walk::dry_run(&tree, &crate::watch::subjects(app)))
}

pub fn changed(app: &App, name: &str, by: &str) {
    let note = change_note(&mut lock(&app.trees), name, by, NOTE_MAX);
    let mut brain = lock(&app.brain);
    brain.carry = carry_note(&brain.carry, name, &note);
    drop(brain);
    app.send_out(json!({"type": "trees_changed", "name": name}));
}

async fn caller(terminal: &str, peer: Option<SocketAddr>) -> Result<String, String> {
    let id = crate::engine::term_id(terminal).map_err(|e| e.to_string())?;
    match crate::engine::get(&id).await {
        Ok(term) if term.running => {
            let name = term.id.to_string();
            crate::extras::peer::from_terminal(peer, &name, term.pid).await?;
            Ok(name)
        }
        Ok(_) => Err(format!("Terminal {terminal} does not run.")),
        Err(_) => Err(format!("Terminal {terminal} is not a Nixa terminal.")),
    }
}

async fn who_of(app: &App, headers: &HeaderMap, args: &Args, peer: Option<SocketAddr>) -> Result<String, Option<String>> {
    if let Some(terminal) = text(args, "terminal") {
        return caller(&terminal, peer).await.map(|t| format!("agent in {t}")).map_err(Some);
    }
    if crate::window_key::allowed(app, headers) {
        return Ok("user".into());
    }
    Err(None)
}

async fn trees_post(State(app): Shared, headers: HeaderMap, peer: Peer, axum::Json(body): axum::Json<Value>) -> Response {
    let args: Args = body.as_object().cloned().unwrap_or_default();
    let who = match who_of(&app, &headers, &args, peer.map(|Extension(ConnectInfo(addr))| addr)).await {
        Ok(who) => who,
        Err(Some(error)) => return bad(StatusCode::FORBIDDEN, error),
        Err(None) => return crate::window_key::refused(),
    };
    let action = text(&args, "action").unwrap_or_else(|| "list".into());
    if action == "walk" {
        return match walk(&app, &args) {
            Ok(text) => ok(json!({"text": text})),
            Err(error) => bad(StatusCode::BAD_REQUEST, error),
        };
    }
    let result = act(&mut lock(&app.trees), &action, &args, &who);
    match result {
        Ok(found) => {
            if let Some(name) = &found.changed {
                let by = if who == "user" { "by the user".to_string() } else { format!("by the {who}") };
                changed(&app, name, &by);
            }
            ok(json!({"text": found.text, "data": found.data}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Args {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn agents_read_and_edit_trees() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TreeStore::open(dir.path());
        store.ensure_first_trees();
        let list = act(&mut store, "list", &args(json!({})), "agent in m1").unwrap();
        assert!(list.text.contains("agent-question: Agent asks a question."));
        assert!(list.changed.is_none());
        let read = act(&mut store, "read", &args(json!({"name": "agent-finished"})), "agent in m1").unwrap();
        assert_eq!(read.data["title"], "Agent finished its work");
        let first = act(&mut store, "box", &args(json!({"name": "agent-question"})), "agent in m1").unwrap();
        assert!(first.text.starts_with("agent-question r1: "));
        assert!(first.text.contains("Yes -> r2: "));
        let made = act(&mut store, "create", &args(json!({"tree": "{\"name\": \"tests\", \"when\": \"An agent runs tests\", \"boxes\": {\"r1\": {\"text\": \"Are they slow?\"}}}"})), "agent in m1").unwrap();
        assert_eq!(made.changed.as_deref(), Some("tests"));
        let added = act(&mut store, "add", &args(json!({"name": "tests", "text": "Run only the changed crate.", "from": "r1", "word": "Yes"})), "agent in m1").unwrap();
        assert_eq!(added.data["id"], "r2");
        act(&mut store, "change", &args(json!({"name": "tests", "id": "r1", "text": "Are the tests slow?"})), "agent in m1").unwrap();
        let tree = store.read("tests").unwrap();
        assert_eq!(tree.boxes["r1"].text, "Are the tests slow?");
        assert_eq!(tree.boxes["r1"].next.as_ref().unwrap()["Yes"], "r2");
        assert!(act(&mut store, "change", &args(json!({"name": "tests", "id": "r1"})), "x").unwrap_err().contains("change needs box"));
        act(&mut store, "meta", &args(json!({"name": "tests", "title": "Slow tests"})), "x").unwrap();
        assert_eq!(store.read("tests").unwrap().title, "Slow tests");
        act(&mut store, "undo", &args(json!({"name": "tests"})), "x").unwrap();
        assert_eq!(store.read("tests").unwrap().title, "tests");
        act(&mut store, "remove", &args(json!({"name": "tests", "id": "r2"})), "x").unwrap();
        assert_eq!(store.read("tests").unwrap().boxes.len(), 1);
        act(&mut store, "delete", &args(json!({"name": "tests"})), "x").unwrap();
        assert!(store.read("tests").is_err());
        assert!(act(&mut store, "read", &args(json!({})), "x").unwrap_err().contains("Give name"));
        assert!(act(&mut store, "grow", &args(json!({})), "x").unwrap_err().starts_with("Choose one of"));
        assert!(act(&mut store, "walk", &args(json!({})), "x").is_err());
        let log = std::fs::read_to_string(dir.path().join("edits.jsonl")).unwrap();
        assert!(log.contains("agent in m1"));
    }

    #[test]
    fn json_args_take_text_or_objects() {
        assert_eq!(json_arg(None).unwrap(), None);
        assert_eq!(json_arg(Some(&json!("{\"a\": 1}"))).unwrap(), Some(json!({"a": 1})));
        assert_eq!(json_arg(Some(&json!({"a": 1}))).unwrap(), Some(json!({"a": 1})));
        assert!(json_arg(Some(&json!("{"))).is_err());
        assert!(READS.iter().all(|r| ACTIONS.contains(r)));
    }
}
