pub mod debate;
pub mod gap_tasks;
pub mod gaps;
pub mod goals;
pub mod model_pick;
pub mod opinions;
pub mod peer;
pub mod shadow;
pub mod usage;
pub mod world;

#[cfg(test)]
mod tests;

use crate::app::{App, lock};
use crate::tools::{Args, ToolResult, schema, spec, text};
use crate::turns::ToolSpec;
use crate::util::now_ms;
use axum::Router;
use axum::extract::{Path as AxPath, Query, State as AxState};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use nixa_store::gaps::GapKind;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const TICK_MS: u64 = 1000;
pub const SLOW_TICK_MS: u64 = 5000;
pub const GAPS_TOOL: &str = "nixa_tool_gaps";
pub const GAP_TASK_TOOL: &str = "nixa_tool_gap_task";
pub const GOALS_TOOL: &str = "nixa_goals";
pub const NAMES: [&str; 5] = [debate::NAME, model_pick::MAP_TOOL, GAPS_TOOL, GAP_TASK_TOOL, GOALS_TOOL];
pub const EVENT_KIND: &str = "extras";

pub struct State {
    pub usage: Mutex<usage::Usage>,
    pub goals: Mutex<goals::Goals>,
    pub gap_turn: Mutex<gaps::Turn>,
    pub opinions: Mutex<opinions::Opinions>,
    pub debates: Mutex<debate::Debates>,
    pub deep_seen: Mutex<HashSet<String>>,
    pub downs: Mutex<HashMap<String, u64>>,
    pub slow_at: Mutex<u64>,
}

impl State {
    pub fn open(dir: &Path) -> State {
        let goals = goals::Goals::open(dir, now_ms());
        let seen = seen_ids(&goals);
        State::with(usage::Usage::open(dir), goals, seen)
    }

    pub fn memory() -> State {
        State::with(usage::Usage::memory(), goals::Goals::memory(), HashSet::new())
    }

    fn with(usage: usage::Usage, goals: goals::Goals, seen: HashSet<String>) -> State {
        State {
            usage: Mutex::new(usage),
            goals: Mutex::new(goals),
            gap_turn: Mutex::new(gaps::Turn::default()),
            opinions: Mutex::new(opinions::Opinions::default()),
            debates: Mutex::new(debate::Debates::default()),
            deep_seen: Mutex::new(seen),
            downs: Mutex::new(HashMap::new()),
            slow_at: Mutex::new(0),
        }
    }
}

fn seen_ids(goals: &goals::Goals) -> HashSet<String> {
    goals
        .since(0)
        .into_iter()
        .filter_map(|r| match r.entry {
            goals::Entry::Deep { id, .. } => Some(id),
            _ => None,
        })
        .collect()
}

pub fn specs() -> Vec<ToolSpec> {
    let mut out = debate::specs();
    out.extend(model_pick::specs());
    out.push(spec(
        GAPS_TOOL,
        "List the tool gaps and refusals Nixa noted: tools that failed or were missing, and steps a rule held. Each gap shows its cause (capability, setup, passing or unknown) and its task when one is filed. Read only. Use it when the user asks what Nixa could not do, or before you say a tool is missing.",
        schema(json!({"kind": {"type": "string", "enum": ["gap", "refusal"], "description": "Optional: only gaps or only refusals"}}), &[]),
    ));
    out.push(spec(
        GAP_TASK_TOOL,
        "Turn one confirmed tool gap (its id from nixa_tool_gaps) into one todo task in the space the gap came from (the nixa space when it has none), with redacted evidence and the expected behavior, and link the task to the gap. Calling it again for the same gap returns the same task. It refuses approval holds, setup problems and passing failures, and files a gap with an unknown cause only with confirm true. It never starts the task.",
        schema(json!({"id": {"type": "integer", "description": "The tool gap id"}, "confirm": {"type": "boolean", "description": "Only for a gap whose cause is unknown: true after you checked that a tool or action really is missing"}}), &["id"]),
    ));
    out.push(spec(
        GOALS_TOOL,
        "Show Nixa's goal stats for the last 7 days (deep brain answers, answer time, deep brain back after a stop, spoken reply length, held duplicate work) and the brain usage by day for the last 30 days. Read only.",
        schema(json!({"part": {"type": "string", "enum": ["goals", "usage", "both"], "description": "Which part, default both"}}), &[]),
    ));
    out
}

pub async fn call(app: &Arc<App>, name: &str, args: &Args) -> Option<ToolResult> {
    let result = match name {
        debate::NAME => debate::call(app, args).await,
        model_pick::MAP_TOOL => model_pick::call(app, args),
        GAPS_TOOL => {
            let kind = text(args, "kind").and_then(|k| GapKind::parse(&k));
            gaps::list(app, kind).map(|rows| gaps::list_text(&rows))
        }
        GAP_TASK_TOOL => match args.get("id").and_then(Value::as_i64) {
            Some(id) => gaps::file_task(app, id, args.get("confirm").and_then(Value::as_bool).unwrap_or(false)),
            None => Err("id (the tool gap id) is needed.".into()),
        },
        GOALS_TOOL => {
            let now = now_ms();
            Ok(match text(args, "part").as_deref() {
                Some("goals") => goals_text(app, now),
                Some("usage") => lock(&app.extras.usage).text(now),
                _ => format!("{}\n\n{}", goals_text(app, now), lock(&app.extras.usage).text(now)),
            })
        }
        _ => return None,
    };
    Some(result)
}

pub fn after_tool(app: &App, name: &str, args: &Args, result: &str, failed: bool) {
    gaps::after_tool(app, name, args, result, failed);
}

pub fn after_reply(app: &App, text: &str, asked: &str, tools: &[String]) {
    gaps::note_reply(app, text, asked, tools);
}

pub fn end_turn(app: &App) {
    gaps::end_turn(app);
}

pub fn note_goal(app: &App, entry: goals::Entry) {
    lock(&app.extras.goals).note(entry, now_ms());
}

pub fn held(app: &App) {
    note_goal(app, goals::Entry::Held);
}

pub fn routed(app: &App, kind: &str, to_deep: bool, id: Option<&str>) {
    note_goal(app, goals::Entry::Route { kind: kind.into(), to: if to_deep { "deep" } else { "quick" }.into(), id: id.map(str::to_string) });
}

pub fn said(app: &App, text: &str) {
    let words = goals::word_count(text);
    if words > 0 {
        note_goal(app, goals::Entry::Said { words });
    }
}

pub fn busy(app: &App, terminal: &str) -> bool {
    let opinions = lock(&app.extras.opinions);
    opinions.owns(terminal) || opinions.pending(terminal) || lock(&app.extras.debates).owns(terminal)
}

pub fn prompt_note(app: &App, terminal: &str) -> &'static str {
    if lock(&app.extras.debates).owns(terminal) { debate::PROMPT_NOTE } else { "" }
}

fn corrected(app: &App) -> impl Fn(i64) -> bool {
    let conn = crate::modes::store(app).ok();
    move |row: i64| conn.as_ref().and_then(|c| crate::modes::journal::get(c, row).ok().flatten()).is_some_and(|d| d.corrected)
}

pub fn goal_lines(app: &App, now: u64) -> Vec<goals::GoalLine> {
    goal_lines_for(app, now, goals::GOAL_DAYS).0
}

pub fn goal_lines_for(app: &App, now: u64, days: u64) -> (Vec<goals::GoalLine>, usize) {
    let rows = lock(&app.extras.goals).since(now.saturating_sub(days * 86_400_000));
    let check = corrected(app);
    (goals::report(&rows, &check, now), rows.len())
}

pub fn goals_text(app: &App, now: u64) -> String {
    goals::text(&goal_lines(app, now), goals::GOAL_DAYS)
}

fn sample_usage(app: &App, now: u64) {
    crate::usage_events::sample(app, None, now);
}

fn deep_goals(app: &App, now: u64) {
    let (asks, seats) = {
        let deep = lock(&app.deep);
        let asks: Vec<crate::deep::Ask> = deep.asks.iter().filter(|a| matches!(a.state, crate::deep::AskState::Answered | crate::deep::AskState::Timeout)).cloned().collect();
        let seats: Vec<(String, Option<u64>, bool)> = deep.seats.iter().map(|s| (s.id.clone(), s.down_at, s.terminal.is_some())).collect();
        (asks, seats)
    };
    for ask in asks {
        if !lock(&app.extras.deep_seen).insert(ask.id.clone()) {
            continue;
        }
        if let Some(entry) = goals::deep_entry(&ask, None) {
            note_goal(app, entry);
        }
    }
    let mut downs = lock(&app.extras.downs);
    for (brain, down_at, up) in seats {
        match (down_at, downs.get(&brain).copied()) {
            (Some(at), None) => {
                downs.insert(brain.clone(), at);
                lock(&app.extras.goals).note(goals::Entry::Down { brain }, now);
            }
            (None, Some(at)) if up => {
                downs.remove(&brain);
                lock(&app.extras.goals).note(goals::Entry::Back { brain, ms: now.saturating_sub(at) }, now);
            }
            _ => {}
        }
    }
}

pub async fn tick(app: &Arc<App>) {
    let now = now_ms();
    sample_usage(app, now);
    debate::tick(app).await;
    opinions::tick(app).await;
    let slow = {
        let mut at = lock(&app.extras.slow_at);
        let due = now.saturating_sub(*at) >= SLOW_TICK_MS;
        if due {
            *at = now;
        }
        due
    };
    if slow {
        deep_goals(app, now);
        model_pick::tick(app);
    }
}

pub async fn run(app: Arc<App>) {
    let mut every = tokio::time::interval(Duration::from_millis(TICK_MS));
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        every.tick().await;
        tick(&app).await;
    }
}

pub async fn terminal_programs(terminal: &str) -> Result<Value, String> {
    let id = crate::engine::term_id(terminal).map_err(|e| e.to_string())?;
    let info = crate::engine::get(&id).await.map_err(|e| e.to_string())?;
    let Some(pid) = info.pid else { return Ok(json!({"terminal": terminal, "programs": []})) };
    let programs = peer::programs_in(pid).await;
    let shell = peer::program_of(pid).await;
    Ok(json!({"terminal": terminal, "pid": pid, "shell": shell, "programs": programs}))
}

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

fn answer(done: Result<Value, String>) -> Response {
    match done {
        Ok(body) => reply(StatusCode::OK, body),
        Err(error) => reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": error})),
    }
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/extras", get(overview_get))
        .route("/extras/usage", get(usage_get))
        .route("/extras/goals", get(goals_get))
        .route("/extras/gaps", get(gaps_get).post(gaps_post))
        .route("/extras/world", get(world_get))
        .route("/extras/peer/{terminal}", get(peer_get))
        .route("/extras/model-pick/{task}", get(pick_get).post(pick_post))
}

fn view(app: &App) -> Value {
    let now = now_ms();
    json!({
        "usage": lock(&app.extras.usage).snapshot(now),
        "goals": goal_lines(app, now),
        "debate": lock(&app.extras.debates).status(),
        "opinions": lock(&app.extras.opinions).count(),
    })
}

macro_rules! guard {
    ($app:expr, $headers:expr) => {
        if !crate::window_key::allowed(&$app, &$headers) {
            return crate::window_key::refused();
        }
    };
}

async fn overview_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    guard!(app, headers);
    let mut body = view(&app);
    body["ok"] = json!(true);
    reply(StatusCode::OK, body)
}

async fn usage_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    guard!(app, headers);
    let now = now_ms();
    let usage = lock(&app.extras.usage);
    reply(StatusCode::OK, json!({"ok": true, "usage": usage.view(now), "text": usage.text(now)}))
}

async fn goals_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap, Query(query): Query<HashMap<String, String>>) -> Response {
    guard!(app, headers);
    let days = match goals::days_of(query.get("days").map(String::as_str)) {
        Ok(days) => days,
        Err(error) => return reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": error})),
    };
    let now = now_ms();
    let (lines, rows) = goal_lines_for(&app, now, days);
    reply(StatusCode::OK, json!({"ok": true, "days": days, "rows": rows, "goals": lines, "text": goals::text(&lines, days)}))
}

async fn gaps_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap, Query(query): Query<HashMap<String, String>>) -> Response {
    guard!(app, headers);
    let kind = query.get("kind").and_then(|k| GapKind::parse(k));
    answer(gaps::list(&app, kind).map(|rows| json!({"ok": true, "gaps": rows, "text": gaps::list_text(&rows)})))
}

async fn gaps_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    guard!(app, headers);
    let kind = body.get("kind").and_then(Value::as_str).and_then(GapKind::parse);
    let done = match body.get("action").and_then(Value::as_str) {
        Some("remove") => match body.get("id").and_then(Value::as_i64) {
            Some(id) => gaps::remove(&app, id).map(|removed| json!({"ok": true, "removed": removed})),
            None => Err("id is needed.".into()),
        },
        Some("clear") => gaps::clear(&app, kind).map(|count| json!({"ok": true, "cleared": count})),
        Some("file") => match body.get("id").and_then(Value::as_i64) {
            Some(id) => gaps::file_task(&app, id, body.get("confirm").and_then(Value::as_bool).unwrap_or(false)).map(|text| json!({"ok": true, "text": text})),
            None => Err("id is needed.".into()),
        },
        _ => Err("Use action remove with id, file with id and an optional confirm, or clear with an optional kind.".into()),
    };
    answer(done)
}

async fn world_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    guard!(app, headers);
    reply(StatusCode::OK, json!({"ok": true, "text": world::context(&app)}))
}

async fn peer_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap, AxPath(terminal): AxPath<String>) -> Response {
    guard!(app, headers);
    answer(terminal_programs(&terminal).await.map(|mut v| {
        v["ok"] = json!(true);
        v
    }))
}

async fn pick_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap, AxPath(task): AxPath<i64>) -> Response {
    guard!(app, headers);
    let pick = model_pick::saved_pick(&app, task);
    reply(StatusCode::OK, json!({"ok": true, "task": task, "pick": pick, "text": pick.as_ref().map(model_pick::pick_line)}))
}

async fn pick_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, AxPath(task): AxPath<i64>) -> Response {
    guard!(app, headers);
    answer(model_pick::ask_for_task(&app, task).map(|ask| json!({"ok": true, "task": task, "ask": ask})))
}
