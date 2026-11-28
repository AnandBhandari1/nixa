use super::*;
use nixa_protocol::{CreateSpec, TermId, TermInfo, wire};
use serde_json::Value;
use std::io::{BufReader, ErrorKind};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::Path;

fn save_agents(dir: &Path, agents: Value) {
    std::fs::write(dir.join(agent_catalog::AGENTS_FILE), json!({"agents": agents}).to_string()).unwrap();
}

fn mock_engine(app: &App) -> UnixListener {
    let path = app.dir.join("engine.sock");
    let listener = UnixListener::bind(&path).unwrap();
    app.announce.set_socket(Some(path));
    listener
}

fn mock_terminal(id: &str, name: &str, cwd: &str, parent: Option<&str>) -> TermInfo {
    serde_json::from_value(json!({
        "id": id, "name": name, "cwd": cwd, "parent": parent, "running": true,
        "rows": 24, "cols": 80, "created_ms": 0
    })).unwrap()
}

fn create_request(listener: UnixListener) -> std::thread::JoinHandle<CreateSpec> {
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let Some(Request::Create { spec }) = wire::read_request(&mut reader).unwrap() else {
            panic!("Expected a Create request");
        };
        let term = mock_terminal(
            "m2",
            spec.name.as_deref().unwrap_or_default(),
            spec.cwd.as_deref().unwrap_or("/tmp"),
            spec.parent.as_ref().map(TermId::as_str),
        );
        wire::write_reply(&mut stream, &Reply::Term { term }).unwrap();
        spec
    })
}

fn assert_no_create(listener: &UnixListener) {
    listener.set_nonblocking(true).unwrap();
    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
}

fn fixture() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    save_agents(dir.path(), json!({
        "fixture": {"name": "Fixture Agent", "run": ["/bin/sh"]},
        "gone": {"name": "Missing Agent", "run": ["/missing/nixa-agent-fixture"]},
    }));
    (dir, app)
}

#[tokio::test]
async fn catalog_is_listed_and_called_through_the_chat_registry() {
    let (_dir, app) = fixture();
    let tools = crate::cli_brains::door::tool_list(&app);
    for name in NAMES {
        assert!(tools.as_array().unwrap().iter().any(|tool| tool["name"] == name));
    }
    let (reply, error) = crate::cli_brains::door::run_tool(&app, LIST, &json!({})).await;
    assert!(!error, "{reply}");
    let listed: Value = serde_json::from_str(&reply).unwrap();
    let agents = listed["agents"].as_array().unwrap();
    assert!(agents.contains(&json!({
        "key": "fixture", "name": "Fixture Agent", "program": "/bin/sh", "installed": true,
    })));
    assert!(agents.contains(&json!({
        "key": "gone", "name": "Missing Agent", "program": "/missing/nixa-agent-fixture", "installed": false,
    })));
}

#[tokio::test]
async fn chat_start_picks_the_named_model_and_sends_one_create_request() {
    let (dir, app) = fixture();
    let program = dir.path().join("claude");
    std::fs::write(&program, "").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    save_agents(dir.path(), json!({"claude": {"run": [program]}}));
    let conn = nixa_store::open(&app.db_file).unwrap();
    let current = nixa_store::models::update(&conn, |cache| {
        nixa_store::models::pick::ensure_map(cache, 1, &nixa_store::models::lists::fixed_models())
    }).unwrap();
    let row = nixa_store::models::model_rows(&current).into_iter().find(|row| row.agent == "claude").unwrap();
    let folder = dir.path().display().to_string();
    nixa_store::spaces::set_space(&conn, "demo", &nixa_store::spaces::SpacePatch {
        folder: Some(folder.clone()), ..Default::default()
    }).unwrap();

    let listener = mock_engine(&app);
    let engine = create_request(listener);
    let task = "Read the project README";
    let args = json!({"agent": "claude", "space": "demo", "name": "Reader", "model": row.id, "task": task});
    let (reply, error) = crate::cli_brains::door::run_tool(&app, START, &args).await;
    assert!(!error, "{reply}");
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["model"], row.id);
    assert_eq!(reply["first_message"], true);
    assert_eq!(reply["terminal"], "m2");
    let spec = engine.join().unwrap();
    assert_eq!(spec.cmd[0], program.display().to_string());
    assert_eq!(&spec.cmd[1..4], ["--dangerously-skip-permissions", "--model", &row.model]);
    let sent = spec.cmd.last().cloned().unwrap_or_default();
    assert!(sent.starts_with(task) && sent.ends_with(&crate::status_rule::rule()), "the first message carries the status rule: {sent}");
    assert_eq!(spec.cwd.as_deref(), Some(folder.as_str()));
    assert_eq!(spec.name.as_deref(), Some("Reader"));
    assert_eq!(spec.labels["agent"], "claude");
    assert_eq!(spec.labels[crate::deep::SPACE_LABEL], "demo");
    let launch = nixa_store::models::launch_of(&conn, "m2").unwrap().unwrap();
    assert_eq!(launch.selection.named, row.id);
    assert!(launch.task.starts_with(task) && launch.task.ends_with(&crate::status_rule::rule()));
}

#[tokio::test]
async fn unavailable_agent_falls_back_and_named_model_refuses() {
    let (dir, app) = fixture();
    let conn = nixa_store::open(&app.db_file).unwrap();
    nixa_store::agent_map::set_fallover(&conn, &["fixture".into()]).unwrap();
    let listener = mock_engine(&app);
    let args = json!({"agent": "gone", "model": "exact-model"});
    let err = call(&app, START, args.as_object().unwrap()).await.unwrap_err();
    assert!(err.contains("A named model is never swapped"), "{err}");
    assert_no_create(&listener);
    listener.set_nonblocking(false).unwrap();
    let engine = create_request(listener);
    let args = json!({"agent": "gone", "cwd": dir.path()});
    let reply = call(&app, START, args.as_object().unwrap()).await.unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert!(reply["note"].as_str().unwrap().contains("Fixture Agent started instead"));
    assert_eq!(reply["first_message"], false);
    let spec = engine.join().unwrap();
    assert_eq!(spec.cmd, ["/bin/sh"]);
    assert_eq!(spec.labels["agent"], "fixture");
}

#[tokio::test]
async fn start_refuses_a_parent_that_already_has_a_parent() {
    let (_dir, app) = fixture();
    let listener = mock_engine(&app);
    let engine = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let id = TermId::from_number(2);
        assert_eq!(wire::read_request(&mut reader).unwrap(), Some(Request::Get { id }));
        let term = mock_terminal("m2", "Child", "/tmp", Some("m1"));
        wire::write_reply(&mut stream, &Reply::Term { term }).unwrap();
        listener
    });
    let args = json!({"agent": "fixture", "parent": "m2"});
    let err = call(&app, START, args.as_object().unwrap()).await.unwrap_err();
    assert!(err.contains("m2 as a parent: it already has a parent"), "{err}");
    assert_no_create(&engine.join().unwrap());
}
