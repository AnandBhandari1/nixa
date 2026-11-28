use super::*;
use crate::brains::Checked;
use crate::chat::Who;
use crate::provider::Kind;
use axum::routing::post;
use std::sync::Arc;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    save_owner(&app, &json!({"owner": "Sam", "aliases": ["Lee"]})).unwrap();
    (dir, app)
}

fn args(value: Value) -> Args {
    value.as_object().cloned().unwrap_or_default()
}

fn said(app: &App, who: Who) -> Vec<String> {
    lock(&app.chat).list().iter().filter(|i| i.who == who).map(|i| i.text.clone()).collect()
}

async fn fake_brain(reply: String) -> String {
    let body = format!("data: {}\n\ndata: [DONE]\n\n", json!({"choices": [{"delta": {"content": reply}}]}));
    let router = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let body = body.clone();
            async move { ([("content-type", "text/event-stream")], body) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    format!("http://{addr}/v1")
}

fn add_brain(app: &App, url: &str) -> String {
    let checked = Checked { name: "Local".into(), kind: Kind::Openai, base_url: url.into(), key_env: None, model: "small".into() };
    app.brains.add(checked).unwrap().id
}

fn check_reply(task: bool) -> String {
    let mut body = serde_json::Map::new();
    for id in ["task", "deep", "answer", "wait", "correction", "own-step", "setting", "space"] {
        body.insert(id.into(), json!({"yes": false, "items": []}));
    }
    if task {
        body.insert("task".into(), json!({"yes": true, "items": [{"text": "Fix the login page", "quote": "fix the login page"}]}));
    }
    Value::Object(body).to_string()
}

#[test]
fn the_owner_name_is_saved_and_has_a_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    assert_eq!(owner_view(&app)["saved"], false);
    save_owner(&app, &json!({"owner": "Kai", "aliases": ["K", " ", 4]})).unwrap();
    assert_eq!(owner_view(&app), json!({"owner": "Kai", "aliases": ["K"], "saved": true}));
    assert!(save_owner(&app, &json!({"owner": 5})).is_err());
    save_owner(&app, &json!({"owner": ""})).unwrap();
    assert_ne!(names_of(&app).owner, "Kai");
}

#[test]
fn private_asks_get_the_privacy_reply_and_the_rest_goes_on() {
    let (_dir, app) = app();
    assert_eq!(screen_user(&app, "Where is Sam right now?"), None);
    assert_eq!(said(&app, Who::Nixa), vec!["I cannot answer that. Only Sam can answer that question."]);
    let screened = screen_user(&app, "Start the build in m2, and which apps are on Lee's laptop?").unwrap();
    assert_eq!(screened.text, "Start the build in m2");
    assert_eq!(screened.notes.len(), 1);
    assert!(screened.notes[0].contains("asked about Sam's private things"));
    let line = brain_line(&screened.notes, "Typed: \"x\"".into());
    assert!(line.starts_with("Typed: \"x\"\n\n") && crate::agent::from_user(&line), "a note never comes before the user line");
    assert_eq!(screen_user(&app, "Run the tests.").unwrap(), Screened { text: "Run the tests.".into(), notes: Vec::new() });
    crate::settings_api::set_privacy(&app, false).unwrap();
    assert_eq!(screen_user(&app, "Where is Sam right now?").unwrap().text, "Where is Sam right now?");
}

#[tokio::test]
async fn tool_errors_hide_secrets_from_cloud_brains() {
    let (_dir, app) = app();
    let key = format!("ghp_{}", "q".repeat(24));
    let call = crate::turns::ToolCall { id: "1".into(), name: key.clone(), args: args(json!({})) };
    let (out, failed) = crate::tools::call(&app, &call).await;
    assert!(failed);
    assert!(out.starts_with("Tool error:"), "{out}");
    assert!(!out.contains(&key), "{out}");
}

#[test]
fn private_tool_results_cannot_be_written_and_secrets_are_hidden_from_cloud_brains() {
    let (_dir, app) = app();
    let out = after_tool(&app, "nixa_chat", &args(json!({})), "Sam said the new clinic opens on Elm Street next month".into());
    assert!(out.contains("Elm Street"));
    let blocked = before_tool(&app, "remember", &args(json!({"text": "the new clinic opens on Elm Street"}))).unwrap();
    assert!(blocked.starts_with("Blocked by the private data guard: this remember call carries private data read earlier from nixa_chat."));
    assert!(blocked.contains("tell Sam what was blocked"));
    assert_eq!(taint_view(&app), json!({"labels": ["private"], "marks": 1}));
    let key = format!("ghp_{}", "q".repeat(24));
    let hidden = after_tool(&app, "nixa_term_read", &args(json!({})), format!("token {key} here"));
    assert_eq!(hidden, "token <secret> here");
    let id = add_brain(&app, "http://127.0.0.1:9/v1");
    app.save_settings(|s| s.choice = Some(choice_of(&id))).unwrap();
    assert!(!cloud_brain(&app));
    assert_eq!(after_tool(&app, "nixa_term_read", &args(json!({})), format!("token {key} here")), format!("token {key} here"));
    clear(&app);
    assert!(before_tool(&app, "remember", &args(json!({"text": "the new clinic opens on Elm Street"}))).is_none());
    assert_eq!(taint_view(&app)["marks"], 0);
}

#[test]
fn the_report_reads_settings_and_the_tool_gives_text() {
    let (_dir, app) = app();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join(".claude.json"), r#"{"mcpServers": {"notes": {"url": "https://notes.example.net/mcp"}}}"#).unwrap();
    let id = add_brain(&app, "http://localhost:11434/v1");
    save_setting(&app, &json!({"on": true, "brain": id})).unwrap();
    let report = report::report(&inputs(&app, home.path()));
    assert_eq!(report.paths[0].name, "none");
    assert_eq!(report.paths[1].area, "Message check brain");
    assert_eq!(report.paths[1].reach, report::Reach::ThisMac);
    assert!(!report.paths.iter().any(|p| p.name.contains("Claude Code")), "other agents' config is not read");
    assert!(report.paths.iter().any(|p| p.area == "Decisions" && !p.in_use));
    assert!(report.privacy_mode);
    assert_eq!(specs()[0].name, NAME);
    assert!(call(&app, NAME, &Args::new()).unwrap().unwrap().starts_with("Privacy check."));
    assert!(call(&app, "other", &Args::new()).is_none());
    let mut rx = app.out.subscribe();
    assert!(ws(&app, REPORT_KIND, &json!({})));
    let sent = rx.try_recv().unwrap();
    assert_eq!(sent["type"], REPORT_KIND);
    assert!(sent["text"].as_str().unwrap().contains("Chat brain"));
    assert!(!ws(&app, "nothing", &json!({})));
}

#[test]
fn message_check_settings_are_checked() {
    let (_dir, app) = app();
    assert_eq!(load_setting(&app), Setting::default());
    assert_eq!(save_setting(&app, &json!({"on": true})), Err("Pick a saved API brain for the message check first.".into()));
    assert!(save_setting(&app, &json!({"brain": "api:00000000"})).is_err());
    let id = add_brain(&app, "http://127.0.0.1:9/v1");
    let saved = save_setting(&app, &json!({"on": true, "brain": choice_of(&id), "model": " tiny "})).unwrap();
    assert_eq!(saved, Setting { on: true, brain: Some(id.clone()), model: Some("tiny".into()) });
    assert_eq!(load_setting(&app), saved);
    let off = save_setting(&app, &json!({"on": false, "model": null})).unwrap();
    assert_eq!(off, Setting { on: false, brain: Some(id), model: None });
    let mut rx = app.out.subscribe();
    assert!(ws(&app, CHECK_SET_KIND, &json!({"on": "yes"})));
    assert!(rx.try_recv().unwrap()["error"].is_string());
}

#[tokio::test]
async fn the_message_check_runs_against_the_check_brain() {
    let (dir, app) = app();
    assert!(run_check(&app, "anything", None).await.is_none());
    let url = fake_brain(check_reply(true)).await;
    let id = add_brain(&app, &url);
    save_setting(&app, &json!({"on": true, "brain": id})).unwrap();
    let record = run_check(&app, "Please fix the login page soon", Some(3)).await.unwrap();
    assert_eq!(record.status, Status::Done, "{:?}", record.reason);
    assert!(record.answers["task"].yes);
    assert!(record.note.contains("nixa_task (action add), and name chat item 3"));
    assert_eq!(said(&app, Who::Info).last().map(String::as_str), Some("Checked: nothing to do"));
    let log_text = std::fs::read_to_string(dir.path().join(check::LOG_FILE)).unwrap();
    assert_eq!(log_text.lines().count(), 1);
    assert!(dir.path().join("trees").join(format!("{}.json", check::TREE)).exists());
    let note = check_message(&app, "Please fix the login page soon", None).await.unwrap();
    assert!(note.starts_with("Note: Nixa's message check ran"));

    let bad = fake_brain("not json".into()).await;
    let bad_id = add_brain(&app, &bad);
    save_setting(&app, &json!({"brain": bad_id})).unwrap();
    assert_eq!(run_check(&app, "hello", None).await.unwrap().status, Status::BadReply);

    let quiet = fake_brain(check_reply(false)).await;
    let quiet_id = add_brain(&app, &quiet);
    save_setting(&app, &json!({"brain": quiet_id})).unwrap();
    assert_eq!(check_message(&app, "hello", None).await, None);

    let silent_id = app.brains.add(Checked { name: "Bare".into(), kind: Kind::Openai, base_url: "http://127.0.0.1:9/v1".into(), key_env: None, model: String::new() }).unwrap().id;
    save_setting(&app, &json!({"brain": silent_id})).unwrap();
    assert_eq!(run_check(&app, "hello", None).await.unwrap().status, Status::NoModel);
}

#[tokio::test]
async fn a_line_waits_for_an_earlier_line_that_is_still_checked() {
    let (_dir, app) = app();
    lock(&app.brain).busy = true;
    let earlier = order::take(&app.privacy.order);
    pass_on(&app, "Typed", Screened { text: "second".into(), notes: Vec::new() }, None);
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(lock(&app.brain).queue.is_empty());
    crate::agent::tell(&app, "first".into());
    drop(earlier);
    for _ in 0..50 {
        if lock(&app.brain).queue.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let queue = lock(&app.brain).queue.clone();
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0], "first");
    assert!(queue[1].starts_with("Typed: \"second\""));
    assert!(!order::busy(&app.privacy.order));
    pass_on(&app, "Heard", Screened { text: "third".into(), notes: Vec::new() }, None);
    assert_eq!(lock(&app.brain).queue.len(), 3);
}

#[test]
fn nixa_decides_keeps_check_choices_from_the_deep_brain() {
    let (_dir, app) = app();
    let conn = nixa_store::open(&app.db_file).unwrap();
    nixa_store::spaces::set_space(&conn, "shop", &nixa_store::spaces::SpacePatch { name: Some("shop".into()), folder: Some("/tmp".into()), ..Default::default() }).unwrap();
    let item = check::Item { text: "Pick a name".into(), quote: "name".into() };
    app.decision.set_choice("nixa").unwrap();
    let result = deep_ask(&app, &item);
    assert!(result.starts_with("not asked: Nixa decides, so decide it yourself"), "{result}");
    assert!(result.ends_with("Pick a name"));
    assert!(lock(&app.deep).asks.is_empty());
    app.decision.set_choice("deep").unwrap();
    assert!(deep_ask(&app, &item).starts_with("asked "));
    assert_eq!(lock(&app.deep).asks.len(), 1);
}
