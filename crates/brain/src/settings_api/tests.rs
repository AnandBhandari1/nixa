use super::*;
use crate::chat::Who;
use std::os::unix::fs::PermissionsExt;
use tokio::sync::broadcast::Receiver;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn drain(rx: &mut Receiver<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    while let Ok(v) = rx.try_recv() {
        out.push(v);
    }
    out
}

fn last(list: &[Value], kind: &str) -> Value {
    list.iter().rev().find(|v| v["type"] == kind).cloned().unwrap_or(Value::Null)
}

fn infos(app: &App) -> Vec<String> {
    lock(&app.chat).list().iter().filter(|i| i.who == Who::Info).map(|i| i.text.clone()).collect()
}

#[test]
fn default_keywords_pass_the_check() {
    let rows = keywords::defaults();
    let value = serde_json::to_value(&rows).unwrap();
    assert_eq!(keywords::check(&value).unwrap(), rows);
    assert_eq!(rows.len(), 18);
    assert_eq!(value[0]["mustNot"].as_str().map(|s| s.starts_with("Never starts")), Some(true));
    assert_eq!(value[0]["builtIn"], true);
    assert_eq!(keywords::phrase("  Let’s  GO-on! "), "lets go on");
}

#[test]
fn bad_keyword_rows_are_refused() {
    let one = |row: Value| keywords::check(&json!([row])).unwrap_err();
    let good = json!({"id": "x", "word": "zap", "aliases": [], "effect": "status"});
    assert!(keywords::check(&json!([good])).is_ok());
    assert!(keywords::check(&json!({})).unwrap_err().contains("list"));
    assert!(one(json!({"id": "x", "aliases": [], "effect": "status"})).contains("id and a word"));
    assert!(one(json!({"id": "x", "word": "zap", "aliases": [], "effect": "fly"})).contains("fixed list"));
    assert!(one(json!({"id": "x", "word": "zap", "aliases": "a", "effect": "status"})).contains("aliases"));
    assert!(one(json!({"id": "x", "word": "zap", "aliases": ["push it now"], "effect": "status"})).contains("risk word"));
    assert!(one(json!({"id": "x", "word": "zap", "aliases": ["one two three four five"], "effect": "status"})).contains("longer than 4"));
    assert!(one(json!({"id": "x", "word": "zap", "aliases": ["okay"], "effect": "status"})).contains("filler"));
    assert!(keywords::check(&json!([{"id": "x", "word": "zap", "aliases": [], "effect": "mode-plan"}])).is_ok());
    let twice = keywords::check(&json!([
        {"id": "a", "word": "zap", "aliases": [], "effect": "status"},
        {"id": "b", "word": "zip", "aliases": ["zap"], "effect": "status"}
    ]));
    assert!(twice.unwrap_err().contains("already used by a"));
    let ids = keywords::check(&json!([
        {"id": "a", "word": "zap", "aliases": [], "effect": "status"},
        {"id": "a", "word": "zip", "aliases": [], "effect": "status"}
    ]));
    assert!(ids.unwrap_err().contains("used twice"));
    let kept = keywords::check(&json!([{"id": "x", "word": "zap", "aliases": [" a1 ", "a1", ""], "effect": "status", "on": false}])).unwrap();
    assert_eq!((kept[0].aliases.clone(), kept[0].on), (vec!["a1".to_string()], false));
}

#[test]
fn the_keyword_file_is_written_once_and_a_bad_file_keeps_the_last_list() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(keywords::FILE);
    let first = keywords::load(&file, &[]);
    assert!(first.wrote && first.error.is_none());
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    let mine = json!([{"id": "zap", "word": "zap", "aliases": ["zap it"], "effect": "status", "does": "Zaps.", "on": true}]);
    std::fs::write(&file, mine.to_string()).unwrap();
    let second = keywords::load(&file, &first.rows);
    assert_eq!(second.rows[0].word, "zap");
    assert_eq!(second.rows.len(), keywords::defaults().len() + 1);
    std::fs::write(&file, "{not json").unwrap();
    let third = keywords::load(&file, &second.rows);
    assert!(third.error.as_deref().unwrap().contains("not valid JSON"));
    assert_eq!(third.rows, second.rows);
    let view = keywords::view(&third.rows, third.error.as_deref());
    assert_eq!(view["keywords"][0]["word"], "zap");
    assert_eq!(view["keywords"][0]["live"], false);
    assert_eq!(view["keywords"][1]["live"], true);
    assert_eq!(view["risk"].as_array().unwrap().len(), 6);
    assert!(view["error"].as_str().is_some());
}

#[tokio::test]
async fn privacy_is_saved_told_once_and_noted_for_the_brain() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    assert!(privacy_on(&app));
    assert_eq!(ws(&app, "set_privacy", &json!({"on": false})), Some(Ok(())));
    assert!(!privacy_on(&app));
    let out = drain(&mut rx);
    assert_eq!(last(&out, "state")["privacy"], false);
    assert!(lock(&app.brain).carry.contains("switch off"));
    assert_eq!(ws(&app, "set_privacy", &json!({"on": false})), Some(Ok(())));
    assert_eq!(infos(&app).iter().filter(|t| t.starts_with("Private and confidential is off")).count(), 1);
    assert!(start_lines(&app).contains("Private and confidential switch: off"));
    set_privacy(&app, true).unwrap();
    assert!(start_lines(&app).is_empty());
    assert!(matches!(ws(&app, "set_privacy", &json!({})), Some(Err(_))));
    let out = drain(&mut rx);
    assert_eq!(last(&out, "error")["about"], "set_privacy");
}

#[tokio::test]
async fn work_is_sent_with_the_keyword_list_and_no_mode() {
    let (_dir, app) = app();
    reload_keywords(&app);
    let mut rx = app.out.subscribe();
    assert_eq!(ws(&app, "set_work_mode", &json!({"mode": "review"})), None);
    assert_eq!(ws(&app, "keyword_clear", &json!({"what": "hold"})), Some(Ok(())));
    let out = drain(&mut rx);
    let event = last(&out, "work");
    assert!(event.get("mode").is_none());
    assert!(event["work"].get("mode").is_none());
    assert_eq!(event["work"]["keywords"].as_array().unwrap().len(), keywords::defaults().len());
    assert_eq!(event["work"]["riskDoes"], keywords::RISK_DOES);
}
#[tokio::test]
async fn hold_and_topic_are_cleared_from_the_app() {
    let (_dir, app) = app();
    lock(&app.switches.work).hold = Some(work::Hold { at: 1, alias: "pause".into() });
    lock(&app.switches.work).topic = Some(work::Topic { kind: work::TopicKind::Plan, text: "login".into(), at: 2, alias: "just plan".into() });
    let view = work_view(&app);
    assert_eq!(view["hold"]["at"], 1);
    assert_eq!(view["topic"]["kind"], "plan");
    assert!(work::note(&lock(&app.switches.work)).contains("Keyword plan is on for \"login\""));
    let mut rx = app.out.subscribe();
    assert_eq!(ws(&app, "keyword_clear", &json!({"what": "hold"})), Some(Ok(())));
    assert_eq!(ws(&app, "keyword_clear", &json!({"what": "topic"})), Some(Ok(())));
    assert_eq!(ws(&app, "keyword_clear", &json!({"what": "hold"})), Some(Ok(())));
    let lines = infos(&app);
    assert!(lines.contains(&"Hold is off. Work runs again.".to_string()));
    assert!(lines.contains(&"Plan is off.".to_string()));
    assert_eq!(lines.iter().filter(|l| l.starts_with("Hold is off")).count(), 1);
    let view = work_view(&app);
    assert!(view.get("hold").is_none() && view.get("topic").is_none());
    assert!(clear_keyword(&app, "both").is_err());
    assert_eq!(drain(&mut rx).iter().filter(|v| v["type"] == "work").count(), 3);
}

#[tokio::test]
async fn nixa_headphones_and_git_voice_switches() {
    let (dir, app) = app();
    let mut rx = app.out.subscribe();
    assert_eq!(ws(&app, "set_nixa", &json!({"on": false})), Some(Ok(())));
    assert!(dir.path().join(OFF_FILE).exists());
    assert_eq!(state_word(&app), "off");
    assert_eq!(ws(&app, "set_headphones", &json!({"on": true})), Some(Ok(())));
    assert!(dir.path().join(HEADPHONES_FILE).exists());
    assert_eq!(ws(&app, "set_git_voice", &json!({"on": false})), Some(Ok(())));
    assert!(!git_voice(&app));
    let out = drain(&mut rx);
    let state = last(&out, "state");
    assert_eq!((state["state"].as_str(), state["nixa"].as_bool(), state["headphones"].as_bool(), state["gitVoice"].as_bool()), (Some("off"), Some(false), Some(true), Some(false)));
    assert_eq!(last(&out, "git_voice")["on"], false);
    assert!(infos(&app).iter().any(|t| t.starts_with("Nixa is off.")));
    assert_eq!(ws(&app, "set_nixa", &json!({"on": true})), Some(Ok(())));
    assert_eq!(ws(&app, "set_headphones", &json!({"on": false})), Some(Ok(())));
    assert!(!dir.path().join(OFF_FILE).exists() && !dir.path().join(HEADPHONES_FILE).exists());
    assert_eq!(state_word(&app), "idle");
    assert!(infos(&app).iter().any(|t| t == "Nixa is on."));
    assert_eq!(ws(&app, "set_nothing", &json!({})), None);
}

#[tokio::test]
async fn resume_turns_nixa_on_and_ends_quiet_time() {
    let (dir, app) = app();
    assert_eq!(ws(&app, "set_nixa", &json!({"on": false})), Some(Ok(())));
    crate::talk::flow::set_quiet_panel(&app, 5.0);
    assert!(lock(&app.talk.presence).quiet(crate::util::now_ms()));
    assert_eq!(ws(&app, "resume", &json!({})), Some(Ok(())));
    assert!(!dir.path().join(OFF_FILE).exists());
    assert!(!lock(&app.talk.presence).quiet(crate::util::now_ms()));
    assert_eq!(state_word(&app), "idle");
    assert_eq!(ws(&app, "resume", &json!({})), Some(Ok(())));
}

#[tokio::test]
async fn the_watch_sees_flag_files_changed_by_others() {
    let (dir, app) = app();
    tick(&app);
    let mut rx = app.out.subscribe();
    tick(&app);
    assert!(drain(&mut rx).is_empty());
    std::fs::write(dir.path().join(OFF_FILE), "").unwrap();
    tick(&app);
    let out = drain(&mut rx);
    assert_eq!(last(&out, "state")["state"], "off");
    assert!(infos(&app).iter().any(|t| t.starts_with("Nixa is off.")));
    std::fs::write(&app.switches.keywords_file, "[1]").unwrap();
    let stamp = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options().write(true).open(&app.switches.keywords_file).unwrap().set_modified(stamp).unwrap();
    tick(&app);
    let out = drain(&mut rx);
    assert!(last(&out, "work")["work"]["error"].as_str().unwrap().contains("Row 1"));
    assert!(infos(&app).iter().any(|t| t.starts_with("The keyword list was not loaded")));
}

#[tokio::test]
async fn switches_ask_sends_state_and_work() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    assert_eq!(ws(&app, "switches", &json!({})), Some(Ok(())));
    let kinds: Vec<String> = drain(&mut rx).iter().map(|v| v["type"].as_str().unwrap_or_default().to_string()).collect();
    assert_eq!(kinds, ["state", "work"]);
    let all = view(&app);
    assert_eq!(all["agentVoice"], true);
    assert!(all["work"].get("mode").is_none());
    assert!(all.get("type").is_none());
}

#[tokio::test]
async fn the_post_route_needs_the_window_token() {
    let (_dir, app) = app();
    let router = routes().with_state(app.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    let http = reqwest::Client::new();
    let url = format!("http://{addr}/switches");
    let got: Value = http.get(&url).send().await.unwrap().json().await.unwrap();
    assert_eq!(got["switches"]["privacy"], true);
    let refused = http.post(&url).json(&json!({"type": "set_privacy", "on": false})).send().await.unwrap();
    assert_eq!(refused.status(), 401);
    assert!(privacy_on(&app));
    let sent = http
        .post(&url)
        .header(crate::window_key::HEADER, app.window_token.clone())
        .json(&json!({"type": "set_privacy", "on": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(sent.status(), 200);
    let body: Value = sent.json().await.unwrap();
    assert_eq!(body["switches"]["privacy"], false);
    let odd = http
        .post(&url)
        .header(crate::window_key::HEADER, app.window_token.clone())
        .json(&json!({"type": "set_work_mode", "mode": "fly"}))
        .send()
        .await
        .unwrap();
    assert_eq!(odd.status(), 400);
}
