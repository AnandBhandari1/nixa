use super::public::refresh_from;
use super::tick::{inventory_with, list_agents, only_picked};
use super::*;
use crate::app::App;
use nixa_store::models::{By, Program, load, model_rows};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const SCORES: &str = r#"{"leaderboards":[{"name":"Verified","results":[{"model_display":"Claude Opus 5.5","resolved":81.5}]}]}"#;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn seeded() -> (tempfile::TempDir, Arc<App>) {
    let (dir, app) = app();
    let conn = open(&app).unwrap();
    nixa_store::models::update(&conn, |cache| {
        cache.ranking = None;
        nixa_store::models::pick::ensure_map(cache, crate::util::now_ms() as i64, &nixa_store::models::lists::fixed_models())
    })
    .unwrap();
    (dir, app)
}

fn args(value: Value) -> crate::tools::Args {
    value.as_object().cloned().unwrap()
}

fn fake_cursor(_: &[&str]) -> anyhow::Result<String> {
    Ok("composer-2 - Composer\ngpt-5 - GPT\n".into())
}

async fn counting(body: &'static str) -> (&'static str, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = axum::Router::new().route(
        "/scores",
        axum::routing::get(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            async move { body }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    (Box::leak(format!("http://{addr}/scores").into_boxed_str()), hits)
}

fn cursor_installed(_: &App) -> Vec<Program> {
    vec![Program { key: "cursor".into(), run: vec!["cursor-agent".into()], installed: true }]
}

fn attempted(app: &App) -> i64 {
    load(&open(app).unwrap()).unwrap().public_refresh.attempt_at
}

async fn serve(body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = axum::Router::new().route("/scores", axum::routing::get(move || async move { body }));
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    format!("http://{addr}/scores")
}

#[tokio::test]
async fn tool_lists_edits_and_guards_pins() {
    let (_dir, app) = seeded();
    let mut events = app.out.subscribe();
    let out = call(&app, MODELS, &args(json!({"action": "list"}))).await.unwrap();
    let value: Value = serde_json::from_str(&out).unwrap();
    assert!(value["rows"].as_array().unwrap().iter().any(|r| r["id"] == "claude::claude-opus-5-5"));
    let auto = call(&app, MODELS, &args(json!({"action": "edit", "id": "claude::claude-sonnet-5-5", "pins": ["ui"]}))).await.unwrap();
    assert!(auto.contains("\"ui\""), "{auto}");
    assert_eq!(events.recv().await.unwrap()["type"], CHANGED);
    call(&app, MODELS, &args(json!({"action": "edit", "id": "claude::claude-opus-5-5", "cost": "cheap"}))).await.unwrap();
    assert_eq!(events.recv().await.unwrap()["type"], CHANGED);
    let pinned = action(&app, &json!({"action": "edit", "id": "claude::claude-opus-5-5", "pinned": true}), By::User).await.unwrap();
    assert!(pinned["rows"].as_array().unwrap().iter().any(|r| r["pinned"] == true));
    assert!(call(&app, MODELS, &args(json!({"action": "view", "task": "x"}))).await.is_err());
    let chosen = call(&app, MODELS, &args(json!({"action": "choose", "capabilities": ["cheap"]}))).await.unwrap();
    assert!(chosen.contains("claude-haiku-5-5"));
    assert_eq!(specs().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), NAMES.to_vec());
}

#[tokio::test]
async fn agent_model_tool_reads_fixed_list() {
    let (_dir, app) = app();
    let out = call(&app, AGENT_MODELS, &args(json!({"agent": "claude"}))).await.unwrap();
    let value: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["models"][0], "claude-opus-5-5");
    assert_eq!(value["leaderboard"].as_array().unwrap().len(), 3);
    assert!(call(&app, AGENT_MODELS, &args(json!({"agent": "claude", "x": 1}))).await.is_err());
    assert!(call(&app, AGENT_MODELS, &args(json!({"agent": "nobody"}))).await.is_err());
    assert!(call(&app, AGENT_MODELS, &args(json!({}))).await.is_err());
}

#[tokio::test]
async fn window_routes_need_the_key_and_ws_replies() {
    let (_dir, app) = seeded();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = routes().with_state(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    let http = reqwest::Client::new();
    assert_eq!(http.get(format!("{base}/models")).send().await.unwrap().status(), 401);
    let key = crate::window_key::HEADER;
    let list: Value = http.get(format!("{base}/models")).header(key, &app.window_token).send().await.unwrap().json().await.unwrap();
    assert_eq!(list["ok"], true);
    let view: Value = http.get(format!("{base}/models/view")).header(key, &app.window_token).send().await.unwrap().json().await.unwrap();
    assert_eq!(view["data"]["choices"].as_array().unwrap().len(), 5);
    let id = list["data"]["rows"][1]["id"].as_str().unwrap().to_string();
    let moved: Value = http
        .post(format!("{base}/models/move"))
        .header(key, &app.window_token)
        .json(&json!({"id": id, "rank": 1}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(moved["data"]["rows"][0]["id"], id.as_str());
    assert_eq!(moved["data"]["rows"][0]["pinned"], true);
    let bad = http.post(format!("{base}/models/fly")).header(key, &app.window_token).json(&json!({})).send().await.unwrap();
    assert_eq!(bad.status(), 400);
    let agents: Value = http
        .post(format!("{base}/models/agent_models"))
        .header(key, &app.window_token)
        .json(&json!({"agent": "claude"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(agents["data"]["agent"], "claude");
    let reply = ws_request(&app, &json!({"type": WS_KIND, "request": "r1", "action": "view"})).await.unwrap();
    assert_eq!((reply["type"].as_str(), reply["request"].as_str(), reply["ok"].as_bool()), (Some("models_reply"), Some("r1"), Some(true)));
    let failed = ws_request(&app, &json!({"type": WS_KIND, "request": "r2", "args": {"action": "edit"}})).await.unwrap();
    assert_eq!(failed["ok"], false);
    assert!(ws_request(&app, &json!({"type": "other", "request": "r3"})).await.is_none());
}

#[tokio::test]
async fn public_scores_refresh_from_a_server() {
    let (_dir, app) = seeded();
    let url = serve(SCORES).await;
    assert!(!refresh_from(&app, &url, true).await);
    online::set(&app, true).unwrap();
    crate::settings_api::set_privacy(&app, false).unwrap();
    assert!(refresh_from(&app, &url, false).await);
    assert!(!refresh_from(&app, &url, false).await);
    let conn = open(&app).unwrap();
    let rows = model_rows(&load(&conn).unwrap());
    let opus = rows.iter().find(|r| r.id == "claude::claude-opus-5-5").unwrap();
    assert_eq!(opus.details.public_score, Some(81.5));
    assert_eq!(rows[0].id, opus.id);
    let empty = serve(r#"{"leaderboards":[]}"#).await;
    assert!(!refresh_from(&app, &empty, true).await);
    assert!(!refresh_from(&app, "http://127.0.0.1:9/none", true).await);
}

#[tokio::test]
async fn inventory_reads_installed_lists_once() {
    let (_dir, app) = app();
    let programs = vec![
        Program { key: "cursor".into(), run: vec!["cursor-agent".into()], installed: true },
        Program { key: "codex".into(), run: vec!["codex".into()], installed: false },
        Program { key: "claude".into(), run: vec!["claude".into()], installed: true },
    ];
    assert_eq!(list_agents(&programs), vec!["cursor".to_string()]);
    let none = "http://127.0.0.1:9/none";
    assert_eq!(inventory_with(&app, programs.clone(), fake_cursor, none).await, 2);
    assert_eq!(inventory_with(&app, programs, fake_cursor, none).await, 0);
    let conn = open(&app).unwrap();
    let rows = model_rows(&load(&conn).unwrap());
    assert!(rows.iter().any(|r| r.id == "cursor::composer-2"));
}

#[tokio::test]
async fn launch_results_count() {
    let (_dir, app) = seeded();
    let conn = open(&app).unwrap();
    let launch = nixa_store::models::ModelLaunch { model: "claude::claude-opus-5-5".into(), task: "review".into(), ..Default::default() };
    nixa_store::models::record(&conn, "m3", &launch).unwrap();
    assert!(launch_result(&app, "m3", true).unwrap());
    assert!(!launch_result(&app, "m3", true).unwrap());
    assert!(!launch_result(&app, "m4", true).unwrap());
}

#[tokio::test]
async fn agent_starts_pick_a_model_only_with_a_model_list() {
    let (_dir, app) = seeded();
    let claude = vec!["claude".to_string()];
    let fresh = pick_start(&app, &claude, "fix the menu").unwrap().unwrap();
    assert_eq!(fresh.argv[..2], ["claude".to_string(), "--model".to_string()]);
    assert!(fresh.row.id.starts_with("claude::"));
    let other = vec!["zz-agent".to_string()];
    assert!(pick_start(&app, &other, "fix the menu").unwrap().is_none());
    assert!(pick_start(&app, &claude, " ").unwrap().is_none());
    assert!(pick_start(&app, &[], "fix the menu").unwrap().is_none());
    let programs = vec![Program { key: "cursor".into(), run: vec!["cursor-agent".into()], installed: true }];
    inventory_with(&app, programs, fake_cursor, "http://127.0.0.1:9/none").await;
    let conn = open(&app).unwrap();
    let cursor = vec!["cursor-agent".to_string()];
    assert!(super::start::known_agent(&conn, &cursor));
    assert!(super::start::known_agent(&conn, &claude));
    assert!(!super::start::known_agent(&conn, &other));
    match pick_start(&app, &cursor, "fix the menu") {
        Ok(Some(prepared)) => {
            assert!(prepared.argv.iter().any(|a| a.contains("composer-2") || a.contains("gpt-5")));
            record_start(&app, &prepared, "m9", "fix the menu", "/tmp", "", "Cursor").unwrap();
        }
        Ok(None) => panic!("a known agent must get a model"),
        Err(error) => assert!(!error.is_empty()),
    }
}

static FRESH_RUNS: AtomicUsize = AtomicUsize::new(0);

fn fresh_cursor(argv: &[&str]) -> anyhow::Result<String> {
    FRESH_RUNS.fetch_add(1, Ordering::SeqCst);
    fake_cursor(argv)
}

static PICKED_RUNS: AtomicUsize = AtomicUsize::new(0);

fn picked_cursor(argv: &[&str]) -> anyhow::Result<String> {
    PICKED_RUNS.fetch_add(1, Ordering::SeqCst);
    fake_cursor(argv)
}

fn quick() -> super::tick::Timing {
    super::tick::Timing { check: Duration::from_millis(10), first: Duration::from_millis(30), every: Duration::from_millis(60) }
}

#[tokio::test]
async fn a_fresh_brain_makes_no_outside_call() {
    let (_dir, app) = app();
    let (url, hits) = counting(SCORES).await;
    nixa_store::settings::set_setting(&open(&app).unwrap(), nixa_store::models::WANTED_KEY, &json!(true)).unwrap();
    let ticking = tokio::spawn(super::tick::run_with(app.clone(), cursor_installed, fresh_cursor, url, quick()));
    tokio::time::sleep(Duration::from_millis(600)).await;
    ticking.abort();
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert_eq!(attempted(&app), 0);
    assert_eq!(FRESH_RUNS.load(Ordering::SeqCst), 0, "a fresh brain ran an agent's model command");
    let rows = model_rows(&load(&open(&app).unwrap()).unwrap());
    assert!(!rows.iter().any(|r| r.id.starts_with("cursor::")));
    assert!(!online::on(&app));
}

#[tokio::test]
async fn only_the_picked_cli_brain_gets_its_models_listed() {
    let (_dir, app) = app();
    app.save_settings(|s| s.choice = Some("cursor".into())).unwrap();
    crate::settings_api::set_privacy(&app, false).unwrap();
    assert_eq!(super::tick::picked(&app), vec!["cursor".to_string()]);
    let ticking = tokio::spawn(super::tick::run_with(app.clone(), cursor_installed, picked_cursor, "http://127.0.0.1:9/none", quick()));
    tokio::time::sleep(Duration::from_millis(600)).await;
    ticking.abort();
    assert!(PICKED_RUNS.load(Ordering::SeqCst) >= 1);
    let rows = model_rows(&load(&open(&app).unwrap()).unwrap());
    assert!(rows.iter().any(|r| r.id == "cursor::composer-2"));
}

#[test]
fn only_picked_keeps_the_picked_agents() {
    let all = vec![
        Program { key: "cursor".into(), run: vec!["cursor-agent".into()], installed: true },
        Program { key: "opencode".into(), run: vec!["opencode".into()], installed: true },
        Program { key: "agy".into(), run: vec!["agy".into()], installed: true },
    ];
    assert!(only_picked(all.clone(), &[]).is_empty());
    let kept = only_picked(all, &["opencode".to_string()]);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].key, "opencode");
}

#[tokio::test]
async fn new_models_do_not_fetch_scores_while_the_switch_is_off() {
    let (_dir, app) = app();
    let (url, hits) = counting(SCORES).await;
    assert_eq!(inventory_with(&app, cursor_installed(&app), fake_cursor, url).await, 2);
    assert!(!refresh_from(&app, url, true).await);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert_eq!(attempted(&app), 0);
}

#[tokio::test]
async fn the_switch_turns_public_scores_on_and_off() {
    let (_dir, app) = seeded();
    let (url, hits) = counting(SCORES).await;
    let list = action(&app, &json!({"action": "list"}), By::User).await.unwrap();
    assert_eq!(list["scoresOnline"], false);
    assert_eq!(list["scoresNote"], online::OFF_NOTE);
    let refreshed = action(&app, &json!({"action": "refresh"}), By::User).await.unwrap();
    assert_eq!((refreshed["refreshed"].as_bool(), refreshed["scoresOnline"].as_bool()), (Some(false), Some(false)));
    let read = super::routes::handle(&app, online::ACTION, json!({})).await.unwrap();
    assert_eq!((read["on"].as_bool(), read["label"].as_str()), (Some(false), Some(online::LABEL)));
    assert!(super::routes::handle(&app, online::ACTION, json!({"on": "yes"})).await.is_err());
    assert!(super::routes::handle(&app, online::ACTION, json!({"on": true, "url": "x"})).await.is_err());
    assert!(!online::on(&app));
    crate::settings_api::set_privacy(&app, false).unwrap();
    let mut events = app.out.subscribe();
    let on = super::routes::handle(&app, online::ACTION, json!({"on": true})).await.unwrap();
    assert_eq!((on["on"].as_bool(), on["note"].as_str()), (Some(true), Some("")));
    assert_eq!(events.recv().await.unwrap()["type"], CHANGED);
    assert!(refresh_from(&app, url, false).await);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let view = action(&app, &json!({"action": "view"}), By::User).await.unwrap();
    assert_eq!((view["scoresOnline"].as_bool(), view["scoresNote"].as_str()), (Some(true), Some("")));
    let reply = ws_request(&app, &json!({"type": WS_KIND, "request": "r1", "action": online::ACTION, "args": {"on": false}})).await.unwrap();
    assert_eq!(reply["data"]["on"], false);
    assert!(!refresh_from(&app, url, true).await);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn privacy_mode_stops_the_public_score_refresh() {
    let (_dir, app) = seeded();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/scores", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    tokio::spawn(async move {
        while listener.accept().await.is_ok() {
            seen.fetch_add(1, Ordering::SeqCst);
        }
    });
    online::set(&app, true).unwrap();
    crate::settings_api::set_privacy(&app, true).unwrap();
    assert!(!refresh_from(&app, &url, true).await);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}
