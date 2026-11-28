pub mod ask;
pub mod check;
pub mod order;
pub mod redact;
pub mod report;
pub mod taint;

#[cfg(test)]
mod tests;

use crate::app::{App, lock};
use crate::brains::{choice_of, id_of_choice};
use crate::deep::AskInput;
use crate::provider::StepInput;
use crate::reply::AgentReply;
use crate::tools::{Args, ToolResult, schema, spec};
use crate::trees::RuleTree;
use crate::turns::{ToolSpec, Turn};
use crate::util::{append_line, log, now_ms};
use axum::Router;
use axum::extract::State as AxState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use check::{Record, Setting, Status};
use nixa_store::settings::{get_setting, set_setting, set_setting_bool, setting_bool};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const NAME: &str = "nixa_privacy_check";
pub const REPORT_KIND: &str = "privacy";
pub const CHECK_KIND: &str = "privacy_check";
pub const CHECK_SET_KIND: &str = "privacy_check_set";

#[derive(Default)]
pub struct State {
    pub guard: Mutex<ask::Guard>,
    pub taint: Mutex<taint::Taint>,
    pub order: Arc<order::Queue>,
    pub limits: Mutex<std::collections::HashMap<String, Option<u32>>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Screened {
    pub text: String,
    pub notes: Vec<String>,
}

pub const OWNER_KEY: &str = "privacy.owner";
pub const ALIASES_KEY: &str = "privacy.aliases";
pub const OWNER_KIND: &str = "privacy_owner";
pub const OWNER_SET_KIND: &str = "privacy_owner_set";

fn login_name() -> String {
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default();
    let user = user.trim();
    let mut chars = user.chars();
    match chars.next() {
        Some(first) if user.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_') => first.to_uppercase().chain(chars).collect(),
        _ => String::new(),
    }
}

pub fn owner_of(app: &App) -> (String, Vec<String>) {
    let Ok(conn) = store(app) else { return (login_name(), Vec::new()) };
    let owner = text_setting(&conn, OWNER_KEY).unwrap_or_else(login_name);
    let aliases = get_setting(&conn, ALIASES_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .map(|l| l.iter().filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)).collect())
        .unwrap_or_default();
    (owner, aliases)
}

pub fn names_of(app: &App) -> ask::Names {
    let (owner, aliases) = owner_of(app);
    ask::names(&owner, &app.name(), &aliases)
}

pub fn owner_view(app: &App) -> Value {
    let names = names_of(app);
    let saved = store(app).ok().and_then(|conn| text_setting(&conn, OWNER_KEY));
    json!({"owner": names.owner, "aliases": names.aliases, "saved": saved.is_some()})
}

pub fn save_owner(app: &App, body: &Value) -> Result<Value, String> {
    let conn = store(app)?;
    if let Some(owner) = body.get("owner") {
        let owner = owner.as_str().ok_or("owner is a name.")?.trim();
        if owner.chars().count() > 60 {
            return Err("Keep the name under 60 letters.".into());
        }
        set_setting(&conn, OWNER_KEY, &json!(owner)).map_err(|e| format!("Could not save {OWNER_KEY}: {e:#}"))?;
    }
    if let Some(aliases) = body.get("aliases") {
        let list = aliases.as_array().ok_or("aliases is a list of names.")?;
        let names: Vec<String> = list.iter().filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)).take(10).collect();
        set_setting(&conn, ALIASES_KEY, &json!(names)).map_err(|e| format!("Could not save {ALIASES_KEY}: {e:#}"))?;
    }
    Ok(owner_view(app))
}

pub fn privacy_on(app: &App) -> bool {
    crate::settings_api::privacy_on(app)
}

pub fn screen_user(app: &App, text: &str) -> Option<Screened> {
    let names = names_of(app);
    let guarded = lock(&app.privacy.guard).guard(text, privacy_on(app), now_ms(), &names);
    let Some(reply) = guarded.reply else {
        return Some(Screened { text: guarded.brain_text, notes: Vec::new() });
    };
    let kind = guarded.kind.map(|k| if k == ask::Kind::Activity { "activity" } else { "private" }).unwrap_or("private");
    let what = if guarded.brain_text.is_empty() { "answered privately" } else { "answered the private part" };
    log("privacy", &format!("{what}: {kind}, {} parts{}", guarded.refused, if guarded.follow_up { ", follow-up" } else { "" }));
    crate::agent::deliver(app, AgentReply::said(reply.clone()));
    if guarded.brain_text.is_empty() {
        return None;
    }
    Some(Screened { text: guarded.brain_text, notes: vec![ask::rest_note(&names, &reply)] })
}

pub fn pass_on(app: &Arc<App>, head: &'static str, screened: Screened, chat_id: Option<u64>) {
    pass_on_with_handoff(app, head, screened, chat_id, crate::agent::tell);
}

pub(crate) fn pass_on_with_handoff(
    app: &Arc<App>,
    head: &'static str,
    screened: Screened,
    chat_id: Option<u64>,
    handoff: impl FnOnce(&Arc<App>, String) + Send + 'static,
) {
    let check = check_on(app);
    if !check && !order::busy(&app.privacy.order) {
        handoff(app, user_line(app, head, &screened.notes, &screened.text));
        return;
    }
    let turn = order::take(&app.privacy.order);
    let app = app.clone();
    tokio::spawn(async move {
        let mut notes = screened.notes;
        if check && let Some(note) = check_message(&app, &screened.text, chat_id).await {
            notes.push(note);
        }
        order::wait(&turn).await;
        handoff(&app, user_line(&app, head, &notes, &screened.text));
    });
}

fn user_line(app: &App, head: &str, notes: &[String], text: &str) -> String {
    let line = crate::prompt::context_line(app);
    brain_line(notes, format!("{head}: {}\n\n{line}", serde_json::to_string(text).unwrap_or_default()))
}

pub fn brain_line(notes: &[String], line: String) -> String {
    if notes.is_empty() {
        return line;
    }
    format!("{line}\n\n{}", notes.join("\n\n"))
}

pub fn cloud_brain(app: &App) -> bool {
    !crate::agent::current_brain(app).is_some_and(|b| crate::brains::is_local_url(&b.base_url))
}

pub fn before_tool(app: &App, name: &str, args: &Args) -> Option<String> {
    let owner = names_of(app).owner;
    let blocked = lock(&app.privacy.taint).check(name, args, &owner)?;
    let labels: Vec<&str> = blocked.labels.iter().map(|l| l.word()).collect();
    log("taint", &format!("block {name}: sink {}, labels {}, from {}", blocked.sink, labels.join(","), if blocked.from.is_empty() { "-".to_string() } else { blocked.from.join(",") }));
    Some(blocked.text)
}

pub fn after_tool(app: &App, name: &str, args: &Args, output: String) -> String {
    let labels = lock(&app.privacy.taint).mark(name, args, &output, now_ms());
    if labels.is_empty() {
        return output;
    }
    let words: Vec<&str> = labels.iter().map(|l| l.word()).collect();
    log("taint", &format!("mark {name}: {}", words.join(",")));
    if !labels.contains(&taint::Label::Secret) || !cloud_brain(app) {
        return output;
    }
    log("taint", &format!("redact {name}"));
    taint::hide_secret_shapes(&output)
}

pub fn clear(app: &App) {
    lock(&app.privacy.taint).clear();
    lock(&app.privacy.guard).last = None;
}

pub fn keep_seen(app: &App, context: &str, before: Option<u64>) {
    lock(&app.privacy.taint).keep_seen(context, before);
}

pub fn taint_view(app: &App) -> Value {
    let taint = lock(&app.privacy.taint);
    json!({"labels": taint.labels(), "marks": taint.marks.len()})
}

pub fn inputs(app: &App, home: &Path) -> report::Inputs {
    let setting = load_setting(app);
    report::Inputs {
        privacy_on: privacy_on(app),
        choice: app.settings().choice.unwrap_or_default(),
        opencode_model: app.cli.saved().models.get("opencode").cloned().filter(|m| !m.is_empty()),
        brains: app.brains.list(),
        check_brain: setting.brain.filter(|_| setting.on).map(|id| choice_of(&id)),
        mcp_files: report::mcp_files(home),
        phone: Some(report::PhoneDoor { on: crate::phone_door::saved(app).on, port: nixa_phone::PHONE_PORT }),
        web: Some(web_input(app)),
        browser: Some(crate::browser::enabled(app)),
        connectors: crate::connectors::call::working(&app.connectors.root)
            .into_iter()
            .map(|m| {
                let builtin = if m.kind == crate::connectors::manifest::Kind::Mcp { builtin_connector(app, &m.connector) } else { None };
                report::Connector { name: m.connector, hosts: m.hosts, command: m.kind == crate::connectors::manifest::Kind::Command, builtin }
            })
            .collect(),
        office_soap: Some(report::OfficeSoap { url: crate::soap::url(app), model: crate::soap::model(app), used: crate::soap::used(app) }),
        camera: Some(report::CameraLooks { allow: crate::camera::looks_on(app) && crate::phone_door::saved(app).on && crate::camera::caps(app).values().any(|on| *on), cloud: true }),
        decision: Some(decision_input(app)),
        model_scores: Some(crate::models::online::on(app)),
    }
}

fn decision_input(app: &App) -> report::DecisionModel {
    let set = crate::decide::settings::load(app);
    report::DecisionModel { on: set.on, provider: crate::decide::PROVIDER.to_string(), model: set.model }
}

fn builtin_connector(app: &App, name: &str) -> Option<report::Builtin> {
    use crate::connectors::builtin::{BROWSER_MINE, BROWSER_OWN, CAMERA, WEB_SEARCH, WIFI};
    use report::Reach;
    let web = crate::web::setup::load(app).on && !privacy_on(app);
    let browser = crate::browser::enabled(app) && !privacy_on(app);
    let (reach, in_use, detail) = match name {
        WEB_SEARCH => (Reach::Internet, web, "Uses Nixa web search and page reading. Off while web search is off or Privacy mode is on."),
        BROWSER_OWN | BROWSER_MINE => (Reach::Internet, browser, "Uses Nixa browser actions. Off while browser actions are off or Privacy mode is on."),
        WIFI => (Reach::ThisMac, true, "Reads the Wi-Fi state of this Mac. Nothing is sent out."),
        CAMERA => (Reach::Unknown, true, "Reads the camera snapshot URL the user saved. It can be on the local network or on the internet."),
        _ => return None,
    };
    Some(report::Builtin { reach, in_use, detail: detail.to_string() })
}

fn web_input(app: &App) -> report::WebSearch {
    let setup = crate::web::setup::load(app);
    report::WebSearch { on: setup.on, provider: setup.provider.word().to_string() }
}

pub fn home() -> std::path::PathBuf {
    std::env::var("HOME").map(std::path::PathBuf::from).unwrap_or_default()
}

pub fn report_now(app: &App) -> report::Report {
    report::report(&inputs(app, &home()))
}

fn store(app: &App) -> Result<nixa_store::Connection, String> {
    nixa_store::open(&app.db_file).map_err(|e| format!("The store did not open: {e:#}"))
}

fn text_setting(conn: &nixa_store::Connection, key: &str) -> Option<String> {
    get_setting(conn, key).ok().flatten().and_then(|v| v.as_str().map(str::trim).map(str::to_string)).filter(|s| !s.is_empty())
}

pub fn load_setting(app: &App) -> Setting {
    let Ok(conn) = store(app) else { return Setting::default() };
    Setting { on: setting_bool(&conn, check::ON_KEY, false).unwrap_or(false), brain: text_setting(&conn, check::BRAIN_KEY), model: text_setting(&conn, check::MODEL_KEY) }
}

fn brain_id(app: &App, given: &str) -> Result<String, String> {
    let given = given.trim();
    let id = id_of_choice(given).unwrap_or(given);
    app.brains.get(id).map(|b| b.id).ok_or_else(|| "That brain is not saved. Pick one of the saved API brains.".to_string())
}

pub fn save_setting(app: &App, body: &Value) -> Result<Setting, String> {
    let before = load_setting(app);
    let brain = match body.get("brain") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(brain_id(app, s)?),
        Some(Value::String(_)) | Some(Value::Null) => None,
        Some(_) => return Err("brain is the id of a saved API brain.".into()),
        None => before.brain.clone(),
    };
    let model = match body.get("model") {
        Some(Value::String(s)) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Some(Value::Null) => None,
        Some(_) => return Err("model is a model name.".into()),
        None => before.model.clone(),
    };
    let on = match body.get("on") {
        Some(Value::Bool(on)) => *on,
        Some(_) => return Err("on is true or false.".into()),
        None => before.on,
    };
    if on && brain.is_none() {
        return Err("Pick a saved API brain for the message check first.".into());
    }
    let conn = store(app)?;
    let save = |key: &str, value: &Option<String>| set_setting(&conn, key, &value.as_ref().map(|v| json!(v)).unwrap_or(json!(""))).map_err(|e| format!("Could not save {key}: {e:#}"));
    save(check::BRAIN_KEY, &brain)?;
    save(check::MODEL_KEY, &model)?;
    set_setting_bool(&conn, check::ON_KEY, on).map_err(|e| format!("Could not save {}: {e:#}", check::ON_KEY))?;
    log("privacy", if on { "message check on" } else { "message check off" });
    Ok(Setting { on, brain, model })
}

pub fn check_on(app: &App) -> bool {
    load_setting(app).on
}

pub fn message_tree(app: &App) -> Option<RuleTree> {
    let mut trees = lock(&app.trees);
    if let Ok(tree) = trees.read(check::TREE) {
        return Some(tree);
    }
    if !trees.names().iter().any(|n| n == check::TREE)
        && let Ok(tree) = trees.create(&check::default_tree(), "nixa")
    {
        return Some(tree);
    }
    crate::trees::check_tree(&check::default_tree(), None).ok()
}

pub const CHECK_TOKENS_KEY: &str = "privacy.check.max_tokens";

pub async fn check_tokens(app: &App, endpoint: &crate::provider::Endpoint, model: &str) -> Option<u32> {
    let limit = crate::agent::endpoint_limit(app, endpoint, model).await;
    crate::agent::pick_limit(crate::agent::saved_limit(app, CHECK_TOKENS_KEY), limit)
}

async fn complete(app: &App, brain: &crate::brains::SavedBrain, model: &str, system: &str, user: &str) -> Result<String, (Status, String)> {
    let endpoint = crate::agent::endpoint_of(brain).await;
    let turns = vec![Turn::User { text: user.to_string() }];
    let input = StepInput { system, turns: &turns, tools: &[], model, effort: None, max_tokens: check_tokens(app, &endpoint, model).await };
    let cancel = CancellationToken::new();
    let run = crate::provider::step(&app.http, &endpoint, &input, &cancel, |_| {});
    match tokio::time::timeout(Duration::from_millis(check::TIMEOUT_MS), run).await {
        Err(_) => {
            cancel.cancel();
            Err((Status::TimedOut, format!("the check took longer than {} s", check::TIMEOUT_MS / 1000)))
        }
        Ok(Err(error)) => Err((Status::Failed, error.to_string())),
        Ok(Ok(result)) => Ok(result.text),
    }
}

fn deep_ask(app: &App, item: &check::Item) -> String {
    let spaces = app.spaces();
    let Some(space) = crate::spaces::home(&spaces).cloned() else { return "not asked: no space is known".into() };
    if app.decision.nixa_decides() {
        return format!("not asked: Nixa decides, so decide it yourself: {}", item.text);
    }
    let input = AskInput { question: item.text.clone(), options: Vec::new(), notes: item.quote.clone(), kind: None, wait: None, brain: None };
    match crate::deep_service::ask(app, input, &space) {
        Ok((ask, _)) => format!("asked {}", ask.id),
        Err(error) => format!("not asked: {error}"),
    }
}

fn keep_record(app: &App, record: &Record) {
    let line = serde_json::to_string(record).unwrap_or_default();
    if let Err(e) = append_line(&app.dir.join(check::LOG_FILE), &line) {
        log("privacy", &format!("the message check record was not written: {e}"));
    }
}

pub async fn run_check(app: &Arc<App>, text: &str, chat_id: Option<u64>) -> Option<Record> {
    let setting = load_setting(app);
    if !setting.on {
        return None;
    }
    let started = now_ms();
    let message = check::message_of(text);
    let finish = |status: Status, reason: String| {
        let mut record = Record::new(started, now_ms().saturating_sub(started), status, &message, chat_id);
        record.reason = Some(reason);
        keep_record(app, &record);
        record
    };
    let Some(tree) = message_tree(app) else { return Some(finish(Status::Failed, "the message check tree is broken".into())) };
    let questions = check::questions(&tree);
    let owner = names_of(app).owner;
    let saved = setting.brain.as_deref().and_then(|id| app.brains.get(id));
    let model = setting.model.clone().or_else(|| saved.as_ref().map(|b| b.model.clone())).filter(|m| !m.trim().is_empty());
    let (Some(saved), Some(model)) = (saved, model) else {
        return Some(finish(Status::NoModel, "no check model is set in Nixa settings".into()));
    };
    let reply = match complete(app, &saved, &model, &check::prompt(&questions, &owner), &message).await {
        Ok(reply) => reply,
        Err((status, reason)) => return Some(finish(status, reason)),
    };
    let answers = match check::parse_reply(&reply, &questions, &message) {
        Ok(answers) => answers,
        Err(reason) => return Some(finish(Status::BadReply, reason)),
    };
    let mut asked = 0;
    let (steps, note) = check::act(&answers, &questions, chat_id, &owner, &mut |item| {
        let result = deep_ask(app, item);
        if result.starts_with("asked ") {
            asked += 1;
        }
        result
    });
    if asked > 0 {
        crate::watch::deep_tick(app).await;
    }
    let mut record = Record::new(started, now_ms().saturating_sub(started), Status::Done, &message, chat_id);
    record.answers = answers;
    record.note = note;
    app.info(&format!("Checked: {}", check::summary(&steps)));
    record.steps = steps;
    keep_record(app, &record);
    Some(record)
}

pub async fn check_message(app: &Arc<App>, text: &str, chat_id: Option<u64>) -> Option<String> {
    run_check(app, text, chat_id).await.map(|r| r.note).filter(|n| !n.is_empty())
}

pub fn specs() -> Vec<ToolSpec> {
    vec![spec(
        NAME,
        "List every way data can leave this Mac: the chat brain, the message check brain, each saved brain and whether its URL is local, MCP servers of Claude Code, Codex and Cursor, the phone door and voice. Reads settings only, never chat, memory, keys or file content. Nothing is changed.",
        schema(json!({}), &[]),
    )]
}

pub fn call(app: &App, name: &str, _args: &Args) -> Option<ToolResult> {
    (name == NAME).then(|| Ok(report::text(&report_now(app))))
}

fn report_body(app: &App) -> Value {
    let report = report_now(app);
    json!({"report": report, "text": report::text(&report)})
}

pub fn ws(app: &Arc<App>, kind: &str, message: &Value) -> bool {
    match kind {
        REPORT_KIND => {
            let mut body = report_body(app);
            body["type"] = json!(REPORT_KIND);
            app.send_out(body);
            true
        }
        CHECK_KIND => {
            app.send_out(json!({"type": CHECK_KIND, "setting": load_setting(app)}));
            true
        }
        CHECK_SET_KIND => {
            let body = match save_setting(app, message) {
                Ok(setting) => json!({"type": CHECK_KIND, "setting": setting}),
                Err(error) => json!({"type": CHECK_KIND, "setting": load_setting(app), "error": error}),
            };
            app.send_out(body);
            true
        }
        OWNER_KIND => {
            app.send_out(json!({"type": OWNER_KIND, "owner": owner_view(app)}));
            true
        }
        OWNER_SET_KIND => {
            let body = match save_owner(app, message) {
                Ok(view) => json!({"type": OWNER_KIND, "owner": view}),
                Err(error) => json!({"type": OWNER_KIND, "owner": owner_view(app), "error": error}),
            };
            app.send_out(body);
            true
        }
        _ => false,
    }
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/privacy", get(report_get))
        .route("/privacy/check", get(check_get).post(check_post))
        .route("/privacy/ask", post(ask_post))
        .route("/privacy/owner", get(owner_get).post(owner_post))
        .route("/privacy/taint", get(taint_get))
        .route("/privacy/taint/clear", post(taint_clear))
}

fn answer(result: Result<Value, String>) -> Response {
    match result {
        Ok(Value::Object(mut map)) => {
            map.insert("ok".into(), json!(true));
            (StatusCode::OK, axum::Json(Value::Object(map))).into_response()
        }
        Ok(other) => (StatusCode::OK, axum::Json(json!({"ok": true, "data": other}))).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, axum::Json(json!({"ok": false, "error": error}))).into_response(),
    }
}

async fn report_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(Ok(report_body(&app)))
}

async fn check_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(Ok(json!({"setting": load_setting(&app)})))
}

async fn check_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let done = save_setting(&app, &body);
    if let Ok(setting) = &done {
        app.send_out(json!({"type": CHECK_KIND, "setting": setting}));
    }
    answer(done.map(|s| json!({"setting": s})))
}

async fn ask_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let Some(text) = body.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) else {
        return answer(Err("text is needed".into()));
    };
    let names = names_of(&app);
    let found = ask::check(text, &names);
    let reply = found.kind.map(|k| ask::reply_of(k, &names.owner));
    answer(Ok(json!({"privacy": privacy_on(&app), "check": found, "reply": reply})))
}

async fn taint_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(Ok(taint_view(&app)))
}

async fn taint_clear(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    clear(&app);
    answer(Ok(taint_view(&app)))
}

async fn owner_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(Ok(json!({"owner": owner_view(&app)})))
}

async fn owner_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(save_owner(&app, &body).map(|view| json!({"owner": view})))
}
