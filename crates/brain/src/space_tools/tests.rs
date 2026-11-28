use super::*;
use nixa_protocol::TermInfo;
use std::collections::BTreeMap;

fn args(value: Value) -> Args {
    serde_json::from_value(value).unwrap()
}

async fn run(app: &Arc<App>, value: Value) -> Result<Value, String> {
    call(app, &args(value)).await.map(|text| serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn fixture() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn terminal_in(app: &App, id: &str, space: &str, running: bool) {
    let mut labels = BTreeMap::new();
    labels.insert(crate::deep::SPACE_LABEL.to_string(), space.to_string());
    let info: TermInfo = serde_json::from_value(json!({
        "id": id, "name": "builder", "cwd": "/tmp", "running": running, "labels": labels,
        "rows": 24, "cols": 80, "created_ms": 0
    }))
    .unwrap();
    crate::app::lock(&app.watch).world.insert(id.to_string(), crate::app::Seen { info, since: 0 });
}

#[tokio::test]
async fn add_list_rename_lock_and_remove_a_space() {
    let (dir, app) = fixture();
    let project = dir.path().join("shop");
    std::fs::create_dir_all(&project).unwrap();
    let folder = project.display().to_string();

    let added = run(&app, json!({"action": "add", "folder": folder})).await.unwrap();
    assert_eq!(added["existing"], false);
    assert_eq!(added["space"]["id"], "s1");
    assert_eq!(added["space"]["name"], "shop");
    assert_eq!(added["space"]["folder"], folder);
    assert!(added["note"].as_str().unwrap().contains("nixa_agent_start"));

    let again = run(&app, json!({"action": "add", "folder": folder, "name": "Other"})).await.unwrap();
    assert_eq!(again["existing"], true, "the same folder is one space");
    assert_eq!(again["space"]["id"], "s1");

    let list = run(&app, json!({"action": "list"})).await.unwrap();
    assert_eq!(list["spaces"].as_array().unwrap().len(), 1);
    assert_eq!(list["spaces"][0]["name"], "shop");

    let renamed = run(&app, json!({"action": "rename", "space": "shop", "name": "Web shop"})).await.unwrap();
    assert_eq!(renamed["space"]["name"], "Web shop");
    assert_eq!(app.spaces()[0].name, "Web shop", "the brain reads spaces from the store at once");

    let locked = run(&app, json!({"action": "lock", "space": "Web shop", "on": true})).await.unwrap();
    assert_eq!(locked["space"]["locked"], true);
    let refused = run(&app, json!({"action": "lock", "space": folder, "on": false})).await.unwrap_err();
    assert!(refused.contains("Only the user opens it") && refused.contains("Nothing was changed"), "{refused}");
    let conn = nixa_store::open(&app.db_file).unwrap();
    assert!(nixa_store::spaces::get_space(&conn, "s1").unwrap().unwrap().locked, "a folder path finds the space and it stays locked");
    nixa_store::spaces::set_space(&conn, "s1", &nixa_store::spaces::SpacePatch { locked: Some(false), ..Default::default() }).unwrap();

    let removed = run(&app, json!({"action": "remove", "space": "s1"})).await.unwrap();
    assert_eq!(removed["removed"], "s1");
    assert_eq!(removed["folder"], folder);
    let list = run(&app, json!({"action": "list"})).await.unwrap();
    assert!(list["spaces"].as_array().unwrap().is_empty());
    assert!(run(&app, json!({"action": "remove", "space": "s1"})).await.unwrap_err().contains("no space"));
}

#[tokio::test]
async fn add_refuses_folders_that_are_not_there_and_bad_actions() {
    let (dir, app) = fixture();
    let missing = run(&app, json!({"action": "add", "folder": dir.path().join("nope").display().to_string()})).await.unwrap_err();
    assert!(missing.contains("is not a folder") && missing.contains("Nothing was added"));
    let relative = run(&app, json!({"action": "add", "folder": "shop"})).await.unwrap_err();
    assert!(relative.contains("not absolute"));
    let file = dir.path().join("file.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(run(&app, json!({"action": "add", "folder": file.display().to_string()})).await.unwrap_err().contains("is not a folder"));
    assert!(run(&app, json!({"action": "close", "space": "s1"})).await.unwrap_err().contains("no action"));
    assert!(run(&app, json!({})).await.unwrap_err().contains("action is needed"));
    assert!(run(&app, json!({"action": "lock", "space": "s1"})).await.unwrap_err().contains("on is needed"));
    let _ = next_id(&[]);
}

#[tokio::test]
async fn remove_waits_for_terminals_tasks_and_team_runs_of_the_space() {
    let (dir, app) = fixture();
    let project = dir.path().join("app");
    std::fs::create_dir_all(&project).unwrap();
    let folder = project.display().to_string();
    run(&app, json!({"action": "add", "folder": folder})).await.unwrap();

    terminal_in(&app, "demo-codex-3", "s1", true);
    let busy = run(&app, json!({"action": "remove", "space": "s1"})).await.unwrap_err();
    assert!(busy.contains("demo-codex-3") && busy.contains("nixa_term_remove"), "{busy}");

    terminal_in(&app, "demo-codex-3", "s1", false);
    let conn = nixa_store::open(&app.db_file).unwrap();
    let task = nixa_store::tasks::add_task(
        &conn,
        &nixa_store::tasks::TaskInput { title: "Ship it".into(), space_id: "s1".into(), ..nixa_store::tasks::TaskInput::default() },
        &nixa_store::tasks::Actor::new(nixa_store::tasks::AuthorKind::Nixa, "Nixa"),
    )
    .unwrap();
    let tasks = run(&app, json!({"action": "remove", "space": "s1"})).await.unwrap_err();
    assert!(tasks.contains(&format!("#{}", task.id)), "{tasks}");
    nixa_store::tasks::set_status(&conn, task.id, TaskStatus::Done, None, false).unwrap();

    let removed = run(&app, json!({"action": "remove", "space": "s1"})).await.unwrap();
    assert_eq!(removed["removed"], "s1");
    assert_eq!(removed["note"], "The space is closed in the window. Its files stay.");
    assert!(project.is_dir(), "the files stay");
}

#[test]
fn the_window_gets_a_spaces_changed_message_type() {
    assert_eq!(CHANGED, "spaces_changed");
    assert_eq!(NAME, "nixa_spaces");
    assert!(specs()[0].schema["properties"]["action"]["enum"].as_array().unwrap().len() == 5);
}

#[test]
fn next_id_follows_the_window_and_colors_are_shared_out() {
    let row = |id: &str, color: i64| Row {
        space_id: id.to_string(),
        name: String::new(),
        folder: "/tmp".into(),
        auto_start: false,
        max_doing: 1,
        locked: false,
        repos: Vec::new(),
        color,
        position: 0,
        created_at: 0,
        updated_at: 0,
    };
    let rows = vec![row("s1", 0), row("s2", 1), row("demo", 2)];
    assert_eq!(next_id(&rows), "s3");
    assert_eq!(least_used_color(&rows), 3, "0, 1 and 2 are used once each, so the first unused color wins");
    assert_eq!(least_used_color(&[row("s1", 0), row("s2", 0), row("s3", 1), row("s4", 2), row("s5", 3), row("s6", 4), row("s7", 5)]), 1, "all are used, so the least used one wins");
}
