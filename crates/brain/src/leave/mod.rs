pub mod book;
pub mod limits;
pub mod lines;
pub mod routes;

#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod tests;

use crate::app::{App, lock};
use crate::schedules::zone::{self, Zone};
use crate::util::{log, now_ms};
use book::{Book, Leave};
use lines::Moved;
use nixa_protocol::{Status, TermInfo};
use nixa_store::tasks::{self, CommentKind, Task, TaskFilter, TaskStatus};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const LEAVE_EVENT: &str = "agent_leave";
pub const BACK_EVENT: &str = "agent_back";
pub const READ_LINES: usize = 60;
pub const BY_SCREEN: &str = "screen";
pub const BY_AGENT: &str = "agent";
pub const EXTEND_CAP_MS: u64 = 5 * 60 * 60 * 1000;

pub struct State {
    pub book: Mutex<Book>,
    pub seen: Mutex<HashMap<String, u64>>,
    pub moving: Mutex<HashSet<i64>>,
    pub file: PathBuf,
}

impl State {
    pub fn open(dir: &Path) -> State {
        let file = dir.join(book::FILE);
        State { book: Mutex::new(book::load(&file)), seen: Mutex::default(), moving: Mutex::default(), file }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub terminal: String,
    pub name: String,
    pub engine: String,
    pub model: String,
    pub line: String,
    pub before: String,
    pub until: Option<u64>,
    pub by: String,
    pub note: String,
}

fn save(app: &App) {
    let book = lock(&app.leave.book).clone();
    if let Err(e) = book::save(&app.leave.file, &book) {
        log("leave", &format!("the leave list was not saved: {e}"));
    }
}

fn here() -> Zone {
    zone::local()
}

pub fn known_engine(engine: &str) -> bool {
    let key = book::engine_key(engine);
    !key.is_empty()
        && (crate::schedules::runner::BUILT_IN.iter().any(|(k, _, run)| book::same_engine(k, &key) || run.iter().any(|r| book::same_engine(r, &key)))
            || nixa_store::agent_map::FALLOVER_ORDER.iter().any(|k| book::same_engine(k, &key)))
}

pub fn title_of(engine: &str) -> String {
    crate::schedules::runner::BUILT_IN
        .iter()
        .find(|(k, _, _)| book::same_engine(k, engine))
        .map(|(_, name, _)| name.to_string())
        .unwrap_or_else(|| engine.to_string())
}

pub fn persona(info: &TermInfo) -> String {
    info.labels.get(crate::persona::NAME_LABEL).map(|n| n.trim().to_string()).unwrap_or_default()
}

fn model_of(info: &TermInfo) -> String {
    let mut it = info.cmd.iter();
    while let Some(arg) = it.next() {
        if arg == "--model" || arg == "-m" {
            return it.next().cloned().unwrap_or_default();
        }
        if let Some(m) = arg.strip_prefix("--model=") {
            return m.to_string();
        }
    }
    String::new()
}

pub fn away_text(app: &App, engine: &str) -> Option<String> {
    let now = now_ms();
    let book = lock(&app.leave.book);
    let leave = book::engine_away(&book, engine, now)?;
    let back = lines::back_part(&here(), leave.until, leave.known, now);
    Some(format!("{} is on leave: it hit its usage limit ({back})", title_of(&leave.engine)))
}

pub fn away_in(dir: &Path) -> Vec<String> {
    book::away_engines(&book::load(&dir.join(book::FILE)), now_ms())
}

pub fn view(app: &App) -> Value {
    let now = now_ms();
    let place = here();
    let book = lock(&app.leave.book);
    let items: Vec<Value> = book
        .items
        .iter()
        .map(|l| {
            json!({
                "terminal": l.terminal, "name": l.name, "engine": l.engine, "model": l.model,
                "until": l.until, "known": l.known, "back": lines::back_part(&place, l.until, l.known, now),
                "line": l.line, "by": l.by, "task": l.task, "title": l.title, "moved_to": l.moved_to,
                "to_engine": l.to_engine, "waiting": l.waiting,
            })
        })
        .collect();
    let text = if items.is_empty() {
        "No agent is on leave. Every engine can take work.".to_string()
    } else {
        book.items
            .iter()
            .map(|l| {
                let who = if l.name.is_empty() { l.terminal.clone() } else { format!("{} ({})", l.name, l.terminal) };
                let task = if l.task > 0 {
                    if l.moved_to.is_empty() { format!(", task #{} waits", l.task) } else { format!(", task #{} moved to {}", l.task, l.moved_to) }
                } else {
                    String::new()
                };
                format!("{who}: {} on leave, {}{task}", title_of(&l.engine), lines::back_part(&place, l.until, l.known, now))
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    json!({"items": items, "away": book::away_engines(&book, now), "text": text})
}

pub fn leave_event(id: &str, name: &str, until: u64, line: &str, to: &str, to_name: &str) -> Value {
    json!({"type": LEAVE_EVENT, "id": id, "name": name, "until": until, "line": line, "to": to, "to_name": to_name})
}

pub fn back_event(id: &str, name: &str, line: &str) -> Value {
    json!({"type": BACK_EVENT, "id": id, "name": name, "line": line})
}

pub fn open_task(conn: &nixa_store::Connection, terminal: &str) -> Option<Task> {
    let filter = TaskFilter { terminal: terminal.to_string(), status: vec![TaskStatus::Doing, TaskStatus::Blocked], old: true, ..TaskFilter::default() };
    tasks::list_tasks(conn, &filter).ok()?.into_iter().filter(|t| t.terminal_id == terminal && t.archived_at == 0).min_by_key(|t| t.id)
}

pub fn add(app: &App, report: &Report, now: u64) -> Option<Leave> {
    let (until, known) = book::until_of(report.until, now);
    let leave = Leave {
        terminal: report.terminal.clone(),
        name: report.name.clone(),
        engine: book::engine_key(&report.engine),
        model: report.model.clone(),
        line: report.line.clone(),
        before: report.before.clone(),
        note: report.note.clone(),
        by: report.by.clone(),
        since: now,
        until,
        known,
        ..Leave::default()
    };
    let added = book::add(&mut lock(&app.leave.book), leave.clone());
    if !added {
        return None;
    }
    save(app);
    log("leave", &format!("{} ({}) is on leave until {until}{}", leave.terminal, leave.engine, if known { "" } else { " (time not known)" }));
    Some(leave)
}

async fn name_of(terminal: &str) -> String {
    let Ok(id) = crate::engine::term_id(terminal) else { return String::new() };
    crate::engine::get(&id).await.map(|info| persona(&info)).unwrap_or_default()
}

pub async fn hand(app: &Arc<App>, leave: &Leave, to: Option<&str>) -> Moved {
    let Ok(conn) = crate::queue::db(app) else { return Moved::Nothing };
    let Some(task) = open_task(&conn, &leave.terminal) else { return Moved::Nothing };
    let title = format!("#{} {}", task.id, task.title);
    if !lock(&app.leave.moving).insert(task.id) {
        return Moved::Nothing;
    }
    let installed = crate::workflows::installed_programs(&app.dir);
    let order = nixa_store::agent_map::fallover(&conn).unwrap_or_default();
    let away = book::away_engines(&lock(&app.leave.book), now_ms());
    let to = to.map(str::trim).filter(|t| !t.is_empty());
    if let Some(t) = to
        && let Some(why) = book::why_not(&installed, &away, &leave.engine, t)
    {
        lock(&app.leave.moving).remove(&task.id);
        return Moved::Failed { title, error: format!("{t} cannot take it: {why}") };
    }
    let target = match to {
        Some(t) => book::pick_engine(&installed, &[t.to_string()], &away, &leave.engine).filter(|k| book::same_engine(k, t)),
        None => book::pick_engine(&installed, &order, &away, &leave.engine),
    };
    book::update(&mut lock(&app.leave.book), &leave.terminal, |l| {
        l.task = task.id;
        l.title = title.clone();
    });
    let Some(key) = target else {
        lock(&app.leave.moving).remove(&task.id);
        return Moved::Waiting { title };
    };
    let back = lines::back_part(&here(), leave.until, leave.known, now_ms());
    let said = if leave.note.trim().is_empty() { leave.before.clone() } else { leave.note.clone() };
    let who = if leave.name.is_empty() { title_of(&leave.engine) } else { leave.name.clone() };
    let note = lines::handoff_note(&who, &title_of(&leave.engine), &leave.terminal, &back, &said, &task.branch, &task.repo);
    crate::queue::note(&conn, task.id, &note, CommentKind::Handoff);
    drop(conn);
    let result = crate::queue::continue_task(app, task.id, Some((crate::tasks::KIND_TERMINAL.to_string(), key.clone()))).await;
    lock(&app.leave.moving).remove(&task.id);
    match result {
        Ok(done) => {
            book::update(&mut lock(&app.leave.book), &leave.terminal, |l| {
                l.moved_to = done.terminal.clone();
                l.to_engine = key.clone();
                l.waiting = false;
            });
            Moved::Moved { title, to_title: title_of(&key) }
        }
        Err(error) => Moved::Failed { title, error: crate::util::clip_flat(error.trim_end_matches('.'), 200) },
    }
}

async fn fun_line(app: &App, ask: String, names: &[&str], fallback: String) -> String {
    let live = crate::soap::fun::live(&lock(&app.soap.fun), now_ms());
    if !live || cfg!(test) {
        return fallback;
    }
    let body = lines::ask_body(&crate::soap::model(app), &ask);
    match crate::soap::fun::call(app, &body, Duration::from_millis(crate::soap::fun::LIMIT_MS)).await {
        Ok(reply) => reply.get("line").and_then(Value::as_str).and_then(|l| lines::good_line(l, names)).unwrap_or(fallback),
        Err(_) => fallback,
    }
}

pub async fn follow(app: &Arc<App>, leave: Leave, to: Option<String>) -> String {
    let moved = hand(app, &leave, to.as_deref()).await;
    let now = now_ms();
    let place = here();
    let back = lines::back_part(&place, leave.until, leave.known, now);
    let engine = title_of(&leave.engine);
    let (moved_to, to_title) = {
        let book = lock(&app.leave.book);
        let l = book::on_leave(&book, &leave.terminal);
        (l.map(|l| l.moved_to.clone()).unwrap_or_default(), l.map(|l| l.to_engine.clone()).unwrap_or_default())
    };
    let to_name = if moved_to.is_empty() { String::new() } else { name_of(&moved_to).await };
    if let Moved::Waiting { .. } = moved {
        book::update(&mut lock(&app.leave.book), &leave.terminal, |l| l.waiting = true);
    }
    save(app);
    let text = lines::chat_line(&engine, &back, &moved);
    app.info(&text);
    let who = if leave.name.is_empty() { engine.clone() } else { leave.name.clone() };
    let to_who = if !to_name.is_empty() { to_name.clone() } else if !to_title.is_empty() { title_of(&to_title) } else { String::new() };
    let when = lines::when_word(&place, leave.until, leave.known, now);
    let seed = lines::seed(&format!("{}{}", leave.terminal, leave.since));
    let fallback = lines::fallback_leave(&who, &to_who, &when, seed);
    let line = fun_line(app, lines::leave_ask(&who, &to_who, &when), &[&who, &to_who], fallback).await;
    let until = if leave.known { leave.until } else { 0 };
    app.send_out(leave_event(&leave.terminal, &leave.name, until, &line, &moved_to, &to_name));
    text
}

pub fn taken(leave: &Leave, now: u64) -> String {
    let back = lines::back_part(&here(), leave.until, leave.known, now);
    format!("{} is on leave ({back}). Nixa moves your open task to another agent now and says the result in chat.", title_of(&leave.engine))
}

pub fn report_now(app: &Arc<App>, report: Report, to: Option<String>) -> Result<String, String> {
    let now = now_ms();
    let leave = take(app, &report, now)?;
    let text = taken(&leave, now);
    let app = app.clone();
    tokio::spawn(async move {
        follow(&app, leave, to).await;
    });
    Ok(text)
}

fn take(app: &App, report: &Report, now: u64) -> Result<Leave, String> {
    add(app, report, now).ok_or_else(|| {
        let book = lock(&app.leave.book);
        match book::on_leave(&book, &report.terminal) {
            Some(l) => format!("{} is already on leave ({}).", report.terminal, lines::back_part(&here(), l.until, l.known, now)),
            None => "This limit was already handled.".into(),
        }
    })
}

pub async fn report(app: &Arc<App>, report: Report, to: Option<String>) -> Result<String, String> {
    let leave = take(app, &report, now_ms())?;
    Ok(follow(app, leave, to).await)
}

pub async fn handover(app: &Arc<App>, terminal: &str, engine: &str, to: Option<&str>, note: &str) -> Result<String, String> {
    let leave = Leave { terminal: terminal.to_string(), engine: book::engine_key(engine), note: note.to_string(), name: name_of(terminal).await, until: now_ms(), known: false, ..Leave::default() };
    let moved = hand(app, &leave, to).await;
    match moved {
        Moved::Nothing => Err(format!("Terminal {terminal} holds no open task to hand over.")),
        Moved::Waiting { title } => Err(format!("No other free agent for task {title}. It stays with you.")),
        Moved::Failed { title, error } => Err(format!("Task {title} was not handed over: {error}.")),
        Moved::Moved { title, to_title } => {
            let text = format!("Task {title} handed over from {} to {to_title}.", title_of(engine));
            app.info(&text);
            Ok(text)
        }
    }
}

async fn read_hit(terminal: &str, now: u64, place: &Zone) -> Option<limits::Hit> {
    let id = crate::engine::term_id(terminal).ok()?;
    let text = crate::engine::read(&id, Some(READ_LINES), false).await.ok()?;
    limits::find(&text, now, place)
}

async fn running(terminal: &str) -> bool {
    let Ok(id) = crate::engine::term_id(terminal) else { return false };
    crate::engine::get(&id).await.is_ok_and(|info| info.running)
}

pub fn keep_away(leave: &Leave, hit: Option<&limits::Hit>, now: u64) -> Option<String> {
    let hit = hit?;
    if leave.known || leave.line.is_empty() {
        return None;
    }
    if hit.line != leave.line {
        return Some(hit.line.clone());
    }
    (now.saturating_sub(leave.since) < EXTEND_CAP_MS).then(|| hit.line.clone())
}

pub async fn come_back(app: &Arc<App>, leave: &Leave, alive: bool) {
    book::remove(&mut lock(&app.leave.book), &leave.terminal);
    if !alive {
        log("leave", &format!("{} left the leave list quietly: its terminal is gone", leave.terminal));
        return;
    }
    let engine = title_of(&leave.engine);
    if book::engine_away(&lock(&app.leave.book), &leave.engine, now_ms()).is_none() {
        app.info(&format!("{engine} is back: its usage limit time passed."));
    }
    let who = if leave.name.is_empty() { engine } else { leave.name.clone() };
    let fallback = lines::fallback_back(&who, lines::seed(&leave.terminal));
    let line = fun_line(app, lines::back_ask(&who), &[&who], fallback).await;
    app.send_out(back_event(&leave.terminal, &leave.name, &line));
}

async fn returns(app: &Arc<App>, now: u64, place: &Zone) {
    let due = book::due(&lock(&app.leave.book), now);
    let mut changed = false;
    for leave in due {
        changed = true;
        let alive = running(&leave.terminal).await;
        let hit = if alive && !leave.known && !leave.line.is_empty() { read_hit(&leave.terminal, now, place).await } else { None };
        if let Some(line) = keep_away(&leave, hit.as_ref(), now) {
            let fresh = line != leave.line;
            book::update(&mut lock(&app.leave.book), &leave.terminal, |l| {
                l.until = now + book::UNKNOWN_MS;
                if fresh {
                    l.line = line;
                    l.since = now;
                }
            });
            continue;
        }
        come_back(app, &leave, alive).await;
    }
    if changed {
        save(app);
    }
}

async fn retry_waiting(app: &Arc<App>) {
    let waiting: Vec<Leave> = lock(&app.leave.book).items.iter().filter(|l| l.waiting && l.task > 0 && l.moved_to.is_empty()).cloned().collect();
    for leave in waiting {
        let installed = crate::workflows::installed_programs(&app.dir);
        let away = book::away_engines(&lock(&app.leave.book), now_ms());
        if book::pick_engine(&installed, &installed, &away, &leave.engine).is_none() {
            continue;
        }
        let moved = hand(app, &leave, None).await;
        if !matches!(moved, Moved::Waiting { .. }) {
            book::update(&mut lock(&app.leave.book), &leave.terminal, |l| l.waiting = false);
            save(app);
        }
        match &moved {
            Moved::Moved { title, to_title } => app.info(&format!("Task {title} moved to {to_title}, because {} is still on leave.", title_of(&leave.engine))),
            Moved::Failed { .. } => {
                let back = lines::back_part(&here(), leave.until, leave.known, now_ms());
                app.info(&lines::chat_line(&title_of(&leave.engine), &back, &moved));
            }
            _ => {}
        }
    }
}

pub fn watch_now(info: &TermInfo, last_seq: Option<u64>) -> bool {
    info.running && info.status != Some(Status::Working) && last_seq != Some(info.seq) && known_engine(&crate::prompts::scan::agent_of(info))
}

pub async fn tick(app: &Arc<App>, now: u64) {
    let place = here();
    returns(app, now, &place).await;
    retry_waiting(app).await;
    let terms: Vec<TermInfo> = app.terminals().into_iter().map(|s| s.info).collect();
    let live: Vec<String> = terms.iter().filter(|t| t.running).map(|t| t.id.to_string()).collect();
    if book::forget(&mut lock(&app.leave.book), &live) {
        save(app);
    }
    lock(&app.leave.seen).retain(|t, _| live.contains(t));
    if !crate::settings_api::nixa_on(&app.dir) {
        return;
    }
    for info in terms {
        let terminal = info.id.to_string();
        if info.status == Some(Status::Working) && lock(&app.leave.book).handled.remove(&terminal).is_some() {
            save(app);
        }
        if book::on_leave(&lock(&app.leave.book), &terminal).is_some() {
            continue;
        }
        let last = lock(&app.leave.seen).get(&terminal).copied();
        if !watch_now(&info, last) {
            continue;
        }
        lock(&app.leave.seen).insert(terminal.clone(), info.seq);
        let Some(hit) = read_hit(&terminal, now, &place).await else { continue };
        let report = Report {
            terminal,
            name: persona(&info),
            engine: crate::prompts::scan::agent_of(&info),
            model: model_of(&info),
            line: hit.line,
            before: hit.before,
            until: hit.until,
            by: BY_SCREEN.into(),
            note: String::new(),
        };
        if let Some(leave) = add(app, &report, now) {
            let app = app.clone();
            tokio::spawn(async move {
                follow(&app, leave, None).await;
            });
        }
    }
}
