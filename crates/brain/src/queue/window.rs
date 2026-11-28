use super::{changed, db, on_change};
use crate::app::App;
use crate::tools::{Args, flag, text};
use nixa_store::Connection;
use nixa_store::agents::list_agents;
use nixa_store::tasks::{
    self, Actor, Archived, AuthorKind, BlockedOn, CommentInput, CommentKind, Task, TaskFilter, TaskGroup, TaskInput, TaskPatch, TaskSource,
    TaskStatus,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

pub const ACTIONS: [&str; 14] =
    ["options", "list", "show", "update", "comment", "add", "release", "archive", "unarchive", "delete", "speak", "speak_wait", "speak_stop", "workflow"];

pub const LIST_MAX: usize = 400;

fn fail(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn you(app: &App) -> Actor {
    Actor::new(AuthorKind::User, &crate::privacy::names_of(app).owner)
}

fn id_of(args: &Args) -> Result<i64, String> {
    args.get("id")
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().trim_start_matches('#').parse().ok())))
        .filter(|n| *n > 0)
        .ok_or_else(|| "id is needed: a task number.".to_string())
}

fn words(args: &Args, key: &str) -> Vec<String> {
    text(args, key)
        .map(|s| s.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect())
        .unwrap_or_default()
}

fn runs(conn: &Connection) -> HashMap<i64, String> {
    crate::workflows::driver::run_list(conn)
        .into_iter()
        .filter_map(|r| Some((r["task"].as_i64()?, r["line"].as_str().unwrap_or_default().to_string())))
        .collect()
}

fn task_json(task: &Task, comments: Option<i64>, run_box: Option<&String>) -> Value {
    let mut value = serde_json::to_value(task).unwrap_or(Value::Null);
    if let Some(n) = comments {
        value["comments"] = json!(n);
    }
    if let Some(line) = run_box {
        value["run_box"] = json!(line);
    }
    value
}

pub fn options(app: &App, conn: &Connection) -> Result<Value, String> {
    let places = super::places(conn);
    let spaces: Vec<Value> = places.spaces.iter().map(|s| json!({"id": s.id, "name": s.name, "folder": s.folder})).collect();
    let mut people: Vec<Value> = crate::schedules::runner::agent_list(&app.dir, &crate::schedules::runner::default_dirs())
        .into_iter()
        .filter(|(_, _, _, ok)| *ok)
        .map(|(key, label, _, _)| json!({"value": key, "label": label, "kind": "agent"}))
        .collect();
    for agent in list_agents(conn).map_err(fail)?.into_iter().filter(|a| !a.off) {
        let detail = if agent.job.is_empty() { agent.purpose.clone() } else { agent.job.clone() };
        people.push(json!({"value": format!("custom:{}", agent.id), "label": agent.title, "kind": "custom", "detail": crate::util::clip(&detail, 80)}));
    }
    let workflows = crate::workflows::routes::list_json(conn).map(|v| v["workflows"].clone()).unwrap_or_else(|_| json!([]));
    Ok(json!({
        "spaces": spaces,
        "assignees": people,
        "statuses": TaskStatus::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "sources": TaskSource::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "groups": TaskGroup::ALL.iter().map(|g| g.as_str()).collect::<Vec<_>>(),
        "workflows": workflows,
    }))
}

pub fn filter_of(args: &Args) -> Result<TaskFilter, String> {
    let mut filter = TaskFilter {
        archived: if flag(args, "archived") == Some(true) { Archived::Only } else { Archived::Hide },
        q: text(args, "q").unwrap_or_default(),
        old: flag(args, "old").unwrap_or(false),
        limit: Some(LIST_MAX),
        ..TaskFilter::default()
    };
    for word in words(args, "status") {
        filter.status.push(TaskStatus::parse(&word).ok_or_else(|| format!("status {word} is not known."))?);
    }
    for word in words(args, "source") {
        filter.source.push(TaskSource::parse(&word).ok_or_else(|| format!("source {word} is not known."))?);
    }
    for word in words(args, "group") {
        filter.group.push(TaskGroup::parse(&word).ok_or_else(|| format!("group {word} is not known."))?);
    }
    filter.space = text(args, "space");
    filter.assigned = text(args, "assigned").unwrap_or_default();
    Ok(filter)
}

fn list(conn: &Connection, args: &Args) -> Result<Value, String> {
    let found = tasks::list_tasks(conn, &filter_of(args)?).map_err(fail)?;
    let ids: Vec<i64> = found.iter().map(|t| t.id).collect();
    let counts = tasks::comment_counts(conn, &ids).map_err(fail)?;
    let lines = runs(conn);
    let list: Vec<Value> = found.iter().map(|t| task_json(t, Some(counts.get(&t.id).copied().unwrap_or(0)), lines.get(&t.id))).collect();
    Ok(json!({"tasks": list}))
}

fn show(conn: &Connection, id: i64) -> Result<Value, String> {
    let detail = tasks::show_task(conn, id).map_err(fail)?.ok_or_else(|| format!("No task #{id}."))?;
    let run = crate::workflows::driver::run_show(conn, id).map(|v| v["run"].clone()).unwrap_or(Value::Null);
    let count = detail.comments.len() as i64;
    Ok(json!({"task": task_json(&detail.task, Some(count), None), "comments": detail.comments, "run": run}))
}

pub fn assign_of(conn: &Connection, who: &str) -> Result<(String, String), String> {
    let who = who.trim();
    if who.is_empty() || who == "none" {
        return Ok((String::new(), String::new()));
    }
    if let Some(id) = who.strip_prefix("custom:").or_else(|| who.strip_prefix("member:")) {
        return Ok((crate::tasks::KIND_CUSTOM.into(), id.to_string()));
    }
    match crate::tasks::assign_kind(conn, who) {
        Ok(kind) => Ok((kind.to_string(), who.to_string())),
        Err(_) => Ok((crate::tasks::KIND_TERMINAL.into(), who.to_string())),
    }
}

fn space_of(conn: &Connection, key: &str) -> Result<(String, String), String> {
    let key = key.trim();
    if key.is_empty() || key == "none" {
        return Ok((String::new(), String::new()));
    }
    let places = super::places(conn);
    let space = crate::spaces::find(&places.spaces, key).ok_or_else(|| format!("No space named {key}."))?;
    Ok((space.id.clone(), space.name.clone()))
}

pub struct Changed {
    pub task: Task,
    pub why: Option<String>,
}

fn then_of(value: &str) -> String {
    let value = value.trim();
    if value == "none" {
        return String::new();
    }
    value.strip_prefix("member:").map(nixa_store::review::custom_ref).unwrap_or_else(|| value.to_string())
}

pub fn update(conn: &Connection, args: &Args) -> Result<Changed, String> {
    let id = id_of(args)?;
    let mut patch = TaskPatch { title: text(args, "title"), description: args.get("description").and_then(Value::as_str).map(str::to_string), ..TaskPatch::default() };
    if let Some(word) = text(args, "group") {
        patch.group = Some(TaskGroup::parse(&word).ok_or_else(|| format!("group {word} is not known."))?);
    }
    if let Some(key) = args.get("space").and_then(Value::as_str) {
        let (space, name) = space_of(conn, key)?;
        patch.space_id = Some(space);
        patch.space_name = Some(name);
    }
    if let Some(who) = args.get("assign").and_then(Value::as_str) {
        let (kind, who) = assign_of(conn, who)?;
        patch.assigned_kind = Some(kind);
        patch.assigned = Some(who);
    }
    if let Some(then) = args.get("then").and_then(Value::as_str) {
        patch.then_member = Some(then_of(then));
    }
    if let Some(flow) = args.get("workflow").and_then(Value::as_str) {
        patch.workflow = Some(if flow == "none" { String::new() } else { flow.trim().to_string() });
    }
    let mut task = if patch == TaskPatch::default() { tasks::must_task(conn, id).map_err(fail)? } else { tasks::update_task(conn, id, &patch).map_err(fail)? };
    let mut why = None;
    if let Some(word) = text(args, "status") {
        let status = TaskStatus::parse(&word).ok_or_else(|| format!("status {word} is not known."))?;
        let on = match text(args, "on") {
            Some(w) => Some(BlockedOn::parse(&w).ok_or_else(|| format!("on {w} is not known."))?),
            None => None,
        };
        let needs = flag(args, "needsUser").or_else(|| flag(args, "needs_user")).unwrap_or(false);
        task = tasks::set_status(conn, id, status, on, needs).map_err(fail)?;
        why = Some(format!("status:{}", status.as_str()));
    } else if patch.assigned.is_some() {
        why = Some("assigned".into());
    }
    Ok(Changed { task, why })
}

pub fn add(conn: &Connection, args: &Args, you: &Actor) -> Result<Task, String> {
    let title = text(args, "title").ok_or("A task needs a title.")?;
    let mut input = TaskInput { title, description: text(args, "description").unwrap_or_default(), status: Some(TaskStatus::Todo), ..TaskInput::default() };
    if let Some(word) = text(args, "group") {
        input.group = Some(TaskGroup::parse(&word).ok_or_else(|| format!("group {word} is not known."))?);
    }
    if let Some(key) = text(args, "space") {
        let (space, name) = space_of(conn, &key)?;
        input.space_id = space;
        input.space_name = name;
    }
    if let Some(who) = text(args, "assign") {
        let (kind, who) = assign_of(conn, &who)?;
        input.assigned_kind = kind;
        input.assigned = who;
    }
    let task = tasks::add_task(conn, &input, you).map_err(fail)?;
    match text(args, "then").map(|t| then_of(&t)).filter(|t| !t.is_empty()) {
        Some(then) => tasks::update_task(conn, task.id, &TaskPatch { then_member: Some(then), ..TaskPatch::default() }).map_err(fail),
        None => Ok(task),
    }
}

pub fn comment(conn: &Connection, args: &Args, you: &Actor) -> Result<Value, String> {
    let id = id_of(args)?;
    let kind = match text(args, "kind") {
        Some(k) => CommentKind::parse(&k).ok_or_else(|| format!("kind {k} is not known."))?,
        None => CommentKind::Note,
    };
    let body = text(args, "text").ok_or("A comment needs text.")?;
    let added = tasks::add_comment(conn, id, &CommentInput::new(kind, &body), you).map_err(fail)?;
    Ok(json!({"comment": added, "task": tasks::get_task(conn, id).map_err(fail)?}))
}

fn speak_text(task: &Task) -> String {
    let body = crate::util::clip_flat(&task.description, 1500);
    if body.is_empty() { format!("Task {}: {}.", task.id, task.title) } else { format!("Task {}: {}. {body}", task.id, task.title) }
}

pub async fn action(app: &Arc<App>, name: &str, args: &Args) -> Result<Value, String> {
    let conn = db(app)?;
    match name {
        "options" => options(app, &conn),
        "list" => list(&conn, args),
        "show" => show(&conn, id_of(args)?),
        "update" => {
            let done = update(&conn, args)?;
            drop(conn);
            let id = done.task.id;
            match &done.why {
                Some(why) => {
                    super::status_event(app, id, why, "user").await;
                }
                None => changed(app, "update", Some(id)),
            }
            let fresh = tasks::get_task(&db(app)?, id).map_err(fail)?.unwrap_or(done.task);
            Ok(json!({"task": fresh}))
        }
        "comment" => {
            let out = comment(&conn, args, &you(app))?;
            drop(conn);
            changed(app, "comment", out["task"]["id"].as_i64());
            if let Some(id) = out["task"]["id"].as_i64() {
                super::notes::soon(app, id);
            }
            Ok(out)
        }
        "add" => {
            let task = add(&conn, args, &you(app))?;
            drop(conn);
            on_change(app, task.id, "added", "user");
            changed(app, "added", Some(task.id));
            Ok(json!({"task": task}))
        }
        "release" => {
            let id = id_of(args)?;
            let task = tasks::release_task(&conn, id).map_err(fail)?;
            drop(conn);
            on_change(app, id, "released", "user");
            changed(app, "released", Some(id));
            Ok(json!({"task": task}))
        }
        "archive" | "unarchive" => {
            let id = id_of(args)?;
            let task = if name == "archive" { tasks::archive_task(&conn, id) } else { tasks::unarchive_task(&conn, id) }.map_err(fail)?;
            drop(conn);
            if name == "archive"
                && let Err(error) = crate::workflows::on_change(app, id, "archive", "user").await
            {
                app.info(&format!("Task #{id}: {error}"));
            }
            changed(app, name, Some(id));
            Ok(json!({"task": task}))
        }
        "delete" => {
            let id = id_of(args)?;
            if flag(args, "confirm") != Some(true) {
                return Err("Deleting a task needs confirm: true.".into());
            }
            if !tasks::remove_task(&conn, id).map_err(fail)? {
                return Err(format!("No task #{id}."));
            }
            changed(app, "deleted", Some(id));
            Ok(json!({"id": id}))
        }
        "speak" => {
            let task = tasks::must_task(&conn, id_of(args)?).map_err(fail)?;
            drop(conn);
            Ok(crate::talk::hold::read_window::begin(app, &speak_text(&task), task.id))
        }
        "speak_wait" => {
            drop(conn);
            crate::talk::hold::read_window::wait(app, args).await
        }
        "speak_stop" => crate::talk::hold::read_window::stop(app, args),
        "workflow" => {
            let id = id_of(args)?;
            let what = text(args, "do").ok_or("do is needed: pause, resume or cancel.")?;
            if !matches!(what.as_str(), "pause" | "resume" | "cancel" | "retry") {
                return Err(format!("do {what} is not known. Use pause, resume or cancel."));
            }
            drop(conn);
            crate::workflows::act(app, id, &what, crate::workflows::WINDOW_WHO, "").await?;
            changed(app, "workflow", Some(id));
            let task = tasks::get_task(&db(app)?, id).map_err(fail)?;
            Ok(json!({"task": task}))
        }
        other => Err(format!("The tasks message does not know the action {other}.")),
    }
}
