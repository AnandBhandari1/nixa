use super::answer::short;
use super::{AGENT, lookup, when as remind_at};
use crate::app::App;
use crate::chat::{Extra, Who};
use crate::schedules::{jobs as sched, runner, when};
use crate::tools::{Args, flag, need, number, strings, text};
use nixa_store::schedules::{LastRun, RunResult, Schedule, valid_time};
use nixa_store::tasks::{self as store, Actor, Archived, AuthorKind, CommentInput, CommentKind, TaskFilter, TaskInput, TaskStatus};
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::Arc;

pub const BRIEFING_NAME: &str = "Morning briefing";
pub const REMINDER_NAME: &str = "Reminder #";
pub const REMINDER_HEAD: &str = "Personal reminder";
pub const PLACE_LINE: &str = "Weather place: ";
pub const TOPICS_LINE: &str = "News topics: ";
pub const TASK_LINE: &str = "Task number: ";
pub const DUE_LINE: &str = "Due: ";
pub const DUE_MS_LINE: &str = "Due ms: ";
pub const DEFAULT_TIME: &str = "08:00";
pub const TOPICS_MAX: usize = 5;
pub const SPACES_MAX: usize = 5;
pub const TITLE_MAX: usize = 200;
pub const REPORTS: &str = "assistant/reports";
pub const DELIVERED_KEY: &str = "assistant_reminder";
pub const ACTOR: &str = "Nixa assistant";
pub const CHAT_KIND: &str = "assistant";
pub const BAD_JOB: &str = "Assistant instructions need \"Morning briefing\" or \"Personal reminder\" on the first line. A reminder also needs its task number and its Due ms line.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Briefing { place: String, topics: Vec<String> },
    Reminder { task: i64, at: i64 },
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn clean_topics(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for topic in list.iter().flat_map(|t| t.split(',')).map(one_line).filter(|t| !t.is_empty()) {
        let topic = short(&topic, 60);
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&topic)) {
            out.push(topic);
        }
    }
    out.truncate(TOPICS_MAX);
    out
}

pub fn prompt_of(job: &Job, due: &str) -> String {
    match job {
        Job::Briefing { place, topics } => {
            let mut lines = vec![BRIEFING_NAME.to_string()];
            if !place.trim().is_empty() {
                lines.push(format!("{PLACE_LINE}{}", one_line(place)));
            }
            if !topics.is_empty() {
                lines.push(format!("{TOPICS_LINE}{}", topics.iter().map(|t| one_line(t)).collect::<Vec<_>>().join(", ")));
            }
            lines.join("\n")
        }
        Job::Reminder { task, at } => format!("{REMINDER_HEAD}\n{TASK_LINE}{task}\n{DUE_LINE}{due}\n{DUE_MS_LINE}{at}"),
    }
}

pub fn read_job(prompt: &str) -> Result<Job, String> {
    let mut lines = prompt.lines().map(str::trim).filter(|l| !l.is_empty());
    let head = lines.next().unwrap_or_default();
    let rest: Vec<&str> = lines.collect();
    let value = |prefix: &str| rest.iter().find_map(|l| l.strip_prefix(prefix.trim_end())).map(|v| v.trim().to_string());
    if head.eq_ignore_ascii_case(BRIEFING_NAME) {
        let place = short(&value(PLACE_LINE).unwrap_or_default(), 100);
        let topics = clean_topics(&[value(TOPICS_LINE).unwrap_or_default()]);
        return Ok(Job::Briefing { place, topics });
    }
    if head.eq_ignore_ascii_case(REMINDER_HEAD) {
        let task = value(TASK_LINE).and_then(|v| v.trim_start_matches('#').parse::<i64>().ok()).filter(|n| *n > 0);
        let at = value(DUE_MS_LINE).and_then(|v| v.parse::<i64>().ok()).filter(|n| *n > 0);
        if let (Some(task), Some(at)) = (task, at) {
            return Ok(Job::Reminder { task, at });
        }
    }
    Err(BAD_JOB.into())
}

fn actor() -> Actor {
    Actor::new(AuthorKind::Nixa, ACTOR)
}

fn fail(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

pub fn home_folder() -> String {
    let home = crate::privacy::home();
    std::fs::canonicalize(&home)
        .ok()
        .filter(|p| p.is_dir())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "/".into())
}

pub fn assistant_schedules(app: &App) -> Vec<Schedule> {
    sched::list(app).unwrap_or_default().into_iter().filter(|s| s.agent == AGENT).collect()
}

pub fn find(app: &App, name: &str) -> Option<Schedule> {
    assistant_schedules(app).into_iter().find(|s| s.name == name)
}

pub fn publish(app: &Arc<App>, text: &str) {
    app.add_chat(Who::Nixa, text, Extra { kind: Some(CHAT_KIND.into()), ..Extra::default() });
    crate::talk::speech::speak_later(app, text, crate::talk::speech::SayOptions::nixa("chat"));
}

pub fn briefing_text(zone: &str, time: &str, place: &str, topics: &[String], on: bool, id: &str) -> String {
    let weather = if place.is_empty() { "No weather place set.".to_string() } else { format!("Weather for {place}.") };
    let news = if topics.is_empty() { "No news topics set.".to_string() } else { format!("News about {}.", topics.join(", ")) };
    let state = if on { "On" } else { "Paused" };
    format!(
        "Morning briefing saved in Schedules, daily at {time} ({zone}). {weather} {news} It also reads your open tasks and uncommitted work in your spaces. {state}. Change or pause it in Settings, Schedules. Schedule: {id}."
    )
}

pub fn save_briefing(app: &Arc<App>, args: &Args) -> Result<String, String> {
    let existing = find(app, BRIEFING_NAME);
    let (old_place, old_topics) = match existing.as_ref().and_then(|s| read_job(&s.prompt).ok()) {
        Some(Job::Briefing { place, topics }) => (place, topics),
        _ => (String::new(), Vec::new()),
    };
    let time = text(args, "time")
        .or_else(|| existing.as_ref().map(|s| s.time.clone()))
        .unwrap_or_else(|| DEFAULT_TIME.into());
    if !valid_time(&time) {
        return Err("Use 24-hour time like 08:00.".into());
    }
    let place = if args.contains_key("place") || args.contains_key("location") {
        short(&text(args, "place").or_else(|| text(args, "location")).unwrap_or_default(), 100)
    } else {
        old_place
    };
    let topics = if args.contains_key("topics") { clean_topics(&strings(args, "topics")) } else { old_topics };
    let on = flag(args, "enabled").unwrap_or(true);
    let job = Job::Briefing { place: place.clone(), topics: topics.clone() };
    let mut schedule = json!({
        "name": BRIEFING_NAME,
        "agent": AGENT,
        "folder": home_folder(),
        "prompt": prompt_of(&job, ""),
        "repeat": "daily",
        "time": time,
        "enabled": on,
    });
    if let Some(s) = &existing {
        schedule["id"] = json!(s.id);
    }
    sched::command(app, "schedule.save", &json!({"schedule": schedule})).map_err(fail)?;
    let saved = find(app, BRIEFING_NAME).ok_or("The briefing was not saved.")?;
    Ok(briefing_text(&sched::zone_of(app).name, &time, &place, &topics, on, &saved.id))
}

pub fn briefing_on(app: &Arc<App>, on: bool) -> Result<String, String> {
    let s = find(app, BRIEFING_NAME).ok_or("There is no morning briefing yet. Save one first.")?;
    sched::command(app, "schedule.toggle", &json!({"id": s.id, "enabled": on})).map_err(fail)?;
    Ok(if on { "Morning briefing is on.".into() } else { "Morning briefing is paused.".into() })
}

fn comment(conn: &nixa_store::Connection, task: i64, kind: CommentKind, text: &str, extra: Map<String, Value>) {
    let input = CommentInput { kind: Some(kind), text: text.to_string(), extra, ..CommentInput::default() };
    if let Err(error) = store::add_comment(conn, task, &input, &actor()) {
        crate::util::log("assistant", &format!("no comment on task #{task}: {error:#}"));
    }
}

pub fn remind(app: &Arc<App>, args: &Args) -> Result<String, String> {
    let title = short(&need(args, "text")?, TITLE_MAX);
    let when_text = need(args, "when")?;
    let zone = sched::zone_of(app);
    let at = remind_at::to_minute(remind_at::reminder_at(&when_text, sched::now(), &zone)?);
    let due = when::date_time(&zone, at);
    let (day, time) = remind_at::local_day_time(&zone, at);
    let follow = number(args, "task").map(|t| format!(" Follow up on task #{t}.")).unwrap_or_default();
    let conn = sched::conn(app).map_err(fail)?;
    let input = TaskInput {
        title,
        description: format!("Personal reminder due {due}.{follow}"),
        status: Some(TaskStatus::Todo),
        due_date: day.clone(),
        ..TaskInput::default()
    };
    let task = store::add_task(&conn, &input, &actor()).map_err(fail)?;
    let name = format!("{REMINDER_NAME}{}", task.id);
    let job = Job::Reminder { task: task.id, at };
    let payload = json!({"schedule": {
        "name": name,
        "agent": AGENT,
        "folder": home_folder(),
        "prompt": prompt_of(&job, &due),
        "repeat": "once",
        "date": day,
        "time": time,
        "enabled": true,
    }});
    match sched::command(app, "schedule.save", &payload) {
        Ok(()) => {
            let id = find(app, &name).map(|s| s.id).unwrap_or_default();
            comment(&conn, task.id, CommentKind::Note, &format!("Reminder schedule {id}, due {due}. Pause it in Settings, Schedules, or finish or cancel this task to stop it."), Map::new());
            Ok(format!("Reminder saved as task #{}, for {due}. It shows in Tasks; its schedule is in Settings, Schedules.", task.id))
        }
        Err(error) => {
            let why = fail(error);
            comment(&conn, task.id, CommentKind::Error, &format!("Reminder delivery was not scheduled: {why}. Set the time again."), Map::new());
            Ok(format!("Task #{} was saved, but the reminder was not scheduled: {why} Set the time again.", task.id))
        }
    }
}

pub fn cancel_reminder(app: &Arc<App>, id: &str) -> Result<String, String> {
    if !sched::claim(app, id) {
        return Err("The reminder is being delivered. Try again in a moment.".into());
    }
    let result = cancel_schedule_and_task(app, id);
    sched::release(app, id);
    let task = result?;
    sched::changed(app);
    crate::queue::changed(app, "reminder cancelled", Some(task));
    Ok(format!("Cancelled reminder #{task}."))
}

fn cancel_schedule_and_task(app: &App, id: &str) -> Result<i64, String> {
    let conn = sched::conn(app).map_err(fail)?;
    let tx = conn.unchecked_transaction().map_err(fail)?;
    let schedule = nixa_store::schedules::get_schedule(&tx, id).map_err(fail)?.ok_or("Reminder not found.")?;
    let task = match read_job(&schedule.prompt) {
        Ok(Job::Reminder { task, .. }) if schedule.agent == AGENT && schedule.name == format!("{REMINDER_NAME}{task}") => task,
        _ => return Err("This schedule is not a personal reminder.".into()),
    };
    if let Some(found) = store::get_task(&tx, task).map_err(fail)?
        && found.status.is_open()
    {
        store::set_status(&tx, task, TaskStatus::Cancelled, None, false).map_err(fail)?;
    }
    nixa_store::schedules::remove_schedule(&tx, id).map_err(fail)?;
    tx.commit().map_err(fail)?;
    Ok(task)
}

fn open_filter() -> TaskFilter {
    TaskFilter {
        archived: Archived::Hide,
        status: TaskStatus::ALL.into_iter().filter(|s| s.is_open()).collect(),
        limit: Some(500),
        ..TaskFilter::default()
    }
}

pub fn task_text(app: &App) -> String {
    let list = sched::conn(app).map_err(fail).and_then(|c| store::list_tasks(&c, &open_filter()).map_err(fail));
    match list {
        Err(_) => "The task list could not be read.".into(),
        Ok(list) if list.is_empty() => "0 open tasks. Source: task list, checked now.".into(),
        Ok(list) => {
            let first: Vec<String> = list.iter().take(3).map(|t| format!("#{} {}", t.id, short(&t.title, 80))).collect();
            let word = if list.len() == 1 { "task" } else { "tasks" };
            format!("{} open {word}: {}. Source: task list, checked now.", list.len(), first.join("; "))
        }
    }
}

pub fn work_line(found: &[(String, Option<usize>)]) -> String {
    if found.is_empty() {
        return "Uncommitted work: no spaces yet.".into();
    }
    let parts: Vec<String> = found
        .iter()
        .filter_map(|(name, count)| count.map(|n| if n == 0 { format!("{name} none") } else { format!("{name} {n} changed files") }))
        .collect();
    if parts.is_empty() {
        return "Uncommitted work: no git folders in your spaces.".into();
    }
    format!("Uncommitted work: {}. Source: git status, checked now.", parts.join(", "))
}

pub async fn work_text(app: &App) -> String {
    let mut found = Vec::new();
    for space in app.spaces().into_iter().take(SPACES_MAX) {
        let count = runner::git(&space.folder, &["status", "--porcelain"]).await.map(|out| out.lines().filter(|l| !l.trim().is_empty()).count());
        found.push((space.name, count));
    }
    work_line(&found)
}

async fn briefing(app: &Arc<App>, place: &str, topics: &[String]) -> String {
    let mut lines = vec![format!("Morning briefing, {}.", lookup::stamp(app))];
    if !place.is_empty() {
        lines.push(format!("Weather: {}", lookup::live(app, &format!("Weather today in {place}")).await));
    }
    if !topics.is_empty() {
        lines.push(format!("News: {}", lookup::live(app, &format!("{} news today", topics.join(", "))).await));
    }
    lines.push(task_text(app));
    lines.push(work_text(app).await);
    lines.join("\n")
}

fn reminder(app: &Arc<App>, task: i64, at: i64) -> Result<String, String> {
    let conn = sched::conn(app).map_err(fail)?;
    let Some(detail) = store::show_task(&conn, task).map_err(fail)? else {
        return Ok(format!("Reminder #{task} skipped: the task is gone."));
    };
    let t = &detail.task;
    if t.status.is_closed() || t.archived_at > 0 {
        return Ok(format!("Reminder #{task} skipped: the task is {} or archived.", t.status.as_str()));
    }
    if detail.comments.iter().any(|c| c.extra.get(DELIVERED_KEY).and_then(Value::as_i64) == Some(at)) {
        return Ok(format!("Reminder #{task} was already given."));
    }
    let zone = sched::zone_of(app);
    let text = format!("Reminder: {}. Task #{task} stays in your task list until you finish it. Due {}.", short(&t.title, TITLE_MAX), when::date_time(&zone, at));
    publish(app, &text);
    let mut extra = Map::new();
    extra.insert(DELIVERED_KEY.into(), json!(at));
    comment(&conn, task, CommentKind::Note, &format!("Reminder given at {}.", when::date_time(&zone, sched::now())), extra);
    Ok(text)
}

pub async fn execute(app: &Arc<App>, job: &Job) -> Result<String, String> {
    match job {
        Job::Briefing { place, topics } => {
            let text = briefing(app, place, topics).await;
            publish(app, &text);
            Ok(text)
        }
        Job::Reminder { task, at } => reminder(app, *task, *at),
    }
}

fn report_path(app: &App, s: &Schedule, at: i64) -> PathBuf {
    let id: String = s.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    app.dir.join(REPORTS).join(format!("{}-{id}.md", when::stamp(&sched::zone_of(app), at)))
}

pub fn report_text(name: &str, text: &str) -> String {
    format!("# {name}\n\n## Summary\n\n{text}\n\n## Findings\n\nSources and how fresh they are are named above.\n\n## Tasks\n\nNone\n\n## Output\n\nDone by Nixa's personal assistant. No agent was started.\n")
}

fn write_report(app: &App, s: &Schedule, at: i64, text: &str) -> String {
    let path = report_path(app, s, at);
    let written = path
        .parent()
        .map(nixa_protocol::paths::ensure_private_dir)
        .unwrap_or(Ok(()))
        .and_then(|_| crate::util::atomic_write(&path, &report_text(&s.name, text)));
    match written {
        Ok(()) => path.display().to_string(),
        Err(error) => {
            crate::util::log("assistant", &format!("the report {} was not written: {error}", path.display()));
            String::new()
        }
    }
}

pub async fn run_schedule(app: &Arc<App>, s: &Schedule) -> LastRun {
    let at = sched::now();
    let outcome = match read_job(&s.prompt) {
        Ok(job) => execute(app, &job).await,
        Err(error) => Err(error),
    };
    match outcome {
        Ok(text) => LastRun {
            report: write_report(app, s, at, &text),
            done_at: Some(sched::now()),
            open_tasks: Some(0),
            ..sched::new_run(at, RunResult::Started)
        },
        Err(error) => LastRun { error, ..sched::new_run(at, RunResult::Failed) },
    }
}
