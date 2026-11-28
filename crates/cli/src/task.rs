mod agents;
mod caller;
mod files;
mod link;
mod plan;
pub(crate) mod trees;
mod waits;
mod walk;
mod workflow;
mod workflows;

use crate::args::{self, Parsed, usage};
use crate::ctx::{self, Ctx};
use crate::help;
use anyhow::{Result, anyhow};
use caller::Caller;
use nixa_protocol::{Reply, Request, Status, TermId};
use nixa_store::agents as custom;
use nixa_store::spaces::{self, Space};
use nixa_store::task_rules;
use nixa_store::tasks::{
    self, Actor, Archived, BlockedOn, CommentInput, CommentKind, Task, TaskFilter, TaskInput,
    TaskSource, TaskStatus,
};
use nixa_store::Connection;
use serde::Serialize;
use std::io::Write;

pub const LIST_MAX: usize = 50;
const TELL_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
pub const KIND_TERMINAL: &str = "agent";
pub const KIND_CUSTOM: &str = "member";

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::TASK.as_bytes())?;
        return Ok(2);
    };
    let asks_help = matches!(sub.as_str(), "help" | "-h" | "--help")
        || rest.first().is_some_and(|a| a == "-h" || a == "--help");
    if asks_help {
        out.write_all(help::TASK.as_bytes())?;
        return Ok(0);
    }
    match sub.as_str() {
        "ls" | "list" => ls(ctx, rest, out),
        "show" | "get" => show(ctx, rest, out),
        "add" | "new" => add(ctx, rest, out),
        "status" => status(ctx, rest, out),
        "comment" => comment(ctx, rest, out),
        "assign" => assign(ctx, rest, out),
        "edit" => edit(ctx, rest, out),
        "done" => done(ctx, rest, out),
        "claim" => claim(ctx, rest, out),
        "release" => release(ctx, rest, out),
        "next" => next(ctx, rest, out),
        "link" => link::run(ctx, rest, out),
        "files" => files::run(ctx, rest, out),
        "plan" => plan::run(ctx, rest, out),
        "workflow" => workflow::run(ctx, rest, out),
        "workflows" => workflows::run(ctx, rest, out),
        "trees" => trees::run(ctx, rest, out),
        "wait" | "waits" => waits::run(ctx, rest, out),
        "agents" => agents::run(ctx, rest, out),
        "spaces" => list_spaces(ctx, rest, out),
        other => Err(usage(format!("unknown command nixa task {other}"))),
    }
}

fn open() -> Result<Connection> {
    nixa_store::open_default().map_err(|e| anyhow!("the task store did not open: {e:#}"))
}

fn task_id(p: &Parsed, what: &str) -> Result<i64> {
    let word = p.pos.first().ok_or_else(|| usage(format!("nixa task {what} needs a task id")))?;
    args::number::<i64>(word.trim_start_matches('#'), "task id")
}

fn words(p: &Parsed, skip: usize) -> String {
    p.pos.iter().skip(skip).chain(p.rest.iter()).cloned().collect::<Vec<_>>().join(" ")
}

fn status_of(word: &str) -> Result<TaskStatus> {
    TaskStatus::parse(&word.trim().to_lowercase()).ok_or_else(|| {
        usage(format!("status {word} is not known. Use inbox, todo, doing, blocked, review, done or cancelled"))
    })
}

pub fn find_space(conn: &Connection, key: &str) -> Result<Space> {
    let want = key.trim().to_lowercase();
    let all = spaces::list_spaces(conn)?;
    all.iter()
        .find(|s| s.space_id.to_lowercase() == want)
        .or_else(|| all.iter().find(|s| s.name.trim().to_lowercase() == want))
        .cloned()
        .ok_or_else(|| {
            let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
            anyhow!("No space named {key}. The known spaces are: {}.", if names.is_empty() { "none".to_string() } else { names.join(", ") })
        })
}

pub fn task_line(task: &Task) -> String {
    let mut parts = vec![format!("#{} [{}] {}", task.id, task.status.as_str(), task.title.replace('\n', " "))];
    if !task.space_name.is_empty() {
        parts.push(format!("space {}", task.space_name));
    }
    if !task.assigned.is_empty() {
        parts.push(format!("for {}", task.assigned));
    }
    if !task.terminal_id.is_empty() {
        parts.push(format!("in {}", task.terminal_id));
    }
    if let Some(on) = task.blocked_on {
        parts.push(format!("blocked on {}", on.as_str()));
    }
    if task.needs_user {
        parts.push("needs the owner".into());
    }
    parts.join(", ")
}

fn print<T: Serialize>(ctx: &Ctx, out: &mut dyn Write, value: &T, line: &str) -> Result<i32> {
    if ctx.json {
        writeln!(out, "{}", serde_json::to_string(value)?)?;
    } else {
        writeln!(out, "{line}")?;
    }
    Ok(0)
}

fn ls(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--status", "--space", "-q", "--q", "--limit", "--for"], false)?;
    let conn = open()?;
    let all = p.flag(&["--all", "-a"]);
    let limit = match p.value(&["--limit"]) {
        Some(n) => args::number(&n, "--limit")?,
        None => LIST_MAX,
    };
    let mut filter = TaskFilter {
        archived: if all { Archived::Any } else { Archived::Hide },
        q: p.value(&["-q", "--q"]).unwrap_or_default(),
        assigned: p.value(&["--for"]).unwrap_or_default(),
        limit: Some(usize::MAX),
        ..TaskFilter::default()
    };
    for word in p.values(&["--status"]) {
        for one in word.split(',').filter(|w| !w.trim().is_empty()) {
            filter.status.push(status_of(one)?);
        }
    }
    if let Some(space) = p.value(&["--space"]) {
        filter.space = Some(find_space(&conn, &space)?.space_id);
    }
    p.finish(0, false)?;
    let list = open_first(tasks::list_tasks(&conn, &filter)?, limit);
    let text = if list.is_empty() { "No tasks.".to_string() } else { list.iter().map(task_line).collect::<Vec<_>>().join("\n") };
    print(&ctx, out, &list, &text)
}

fn open_first(mut list: Vec<Task>, limit: usize) -> Vec<Task> {
    list.sort_by_key(|t| (matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled), std::cmp::Reverse(t.id)));
    if limit > 0 {
        list.truncate(limit);
    }
    list
}

fn show(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    let id = task_id(&p, "show")?;
    p.finish(1, false)?;
    let conn = open()?;
    let detail = tasks::show_task(&conn, id)?.ok_or_else(|| anyhow!("No task #{id}."))?;
    let mut lines = vec![task_line(&detail.task)];
    if !detail.task.description.is_empty() {
        lines.push(detail.task.description.clone());
    }
    for c in &detail.comments {
        lines.push(format!("- {} ({}): {}", c.author, c.kind.as_str(), c.text.replace('\n', " ")));
    }
    print(&ctx, out, &detail, &lines.join("\n"))
}

fn add(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--desc", "--description", "-d", "--space", "--status"], false)?;
    let title = words(&p, 0);
    if title.trim().is_empty() {
        return Err(usage("nixa task add needs a title"));
    }
    let who = caller::who(&ctx)?;
    let conn = open()?;
    let mut input = TaskInput {
        title,
        description: p.value(&["--desc", "--description", "-d"]).unwrap_or_default(),
        source: Some(if who.terminal().is_some() { TaskSource::Agent } else { TaskSource::Manual }),
        ..TaskInput::default()
    };
    if let Some(word) = p.value(&["--status"]) {
        input.status = Some(status_of(&word)?);
    }
    if let Some(key) = p.value(&["--space"]) {
        let space = find_space(&conn, &key)?;
        input.space_id = space.space_id;
        input.space_name = space.name;
    }
    p.finish(usize::MAX, true)?;
    let mut moved = None;
    if who.terminal().is_some() {
        task_rules::check_agent_add(input.status)?;
        if is_locked(&conn, &input.space_id)? {
            moved = Some(std::mem::take(&mut input.space_name));
            input.space_id.clear();
        }
    }
    let task = tasks::add_task(&conn, &input, &caller::actor_of(&who))?;
    print(&ctx, out, &task, &added_line(&task, moved.as_deref()))
}

fn status(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--on", "--note"], false)?;
    let id = task_id(&p, "status")?;
    let state = status_of(p.pos.get(1).ok_or_else(|| usage("nixa task status needs a status"))?)?;
    let needs = p.flag(&["--needs-user", "--needs-owner"]);
    let on = match p.value(&["--on"]) {
        Some(word) => Some(blocked_on(&word)?),
        None => None,
    };
    if on.is_some() && state != TaskStatus::Blocked {
        return Err(usage("--on works only with status blocked"));
    }
    let note = p.value(&["--note"]).filter(|n| !n.trim().is_empty());
    p.finish(2, false)?;
    let who = caller::who(&ctx)?;
    let conn = open()?;
    let before = tasks::must_task(&conn, id)?;
    let (mut state, mut note, mut to_review) = (state, note, false);
    if let Some(terminal) = who.terminal() {
        task_rules::check_agent_status_in(&before, terminal, state, note.as_deref().unwrap_or_default())?;
        let (after, text) = task_rules::agent_done(state, note.as_deref().unwrap_or_default());
        if after != state {
            (state, note, to_review) = (after, Some(text), true);
        }
        if state == TaskStatus::Doing {
            route_locked(&conn, &before)?;
        }
    }
    let handed = needs || on == Some(BlockedOn::User);
    let (saved_on, saved_needs) = task_rules::auto_blocked(on, needs);
    let tx = conn.unchecked_transaction()?;
    let task = tasks::set_status(&tx, id, state, saved_on, saved_needs)?;
    if let Some(text) = note {
        tasks::add_comment(&tx, id, &CommentInput::new(note_kind(state, saved_on, saved_needs), &text), &caller::actor_of(&who))?;
    }
    tx.commit()?;
    tell_brain(&who, id, task.status);
    let mut line = format!("Now {}.", task_line(&task));
    if handed {
        line.push_str(" This does not wait for the user. Nixa or the deep brain answers it. For a quick answer use nixa deep ask with --wait 60s.");
    }
    if to_review {
        line.push(' ');
        line.push_str(task_rules::DONE_TO_REVIEW);
    }
    print(&ctx, out, &task, &line)
}

pub fn added_line(task: &Task, moved_from: Option<&str>) -> String {
    let mut line = format!("Added {}.", task_line(task));
    if let Some(name) = moved_from {
        line.push_str(&format!(" Space {name} is locked, so the task was added with no space. Nixa places it."));
    }
    line
}

fn tell_brain(who: &Caller, id: i64, status: TaskStatus) {
    if !nixa_protocol::paths::data_dir().join(nixa_protocol::paths::WINDOW_TOKEN_FILE).exists() {
        return;
    }
    let mut body = serde_json::json!({"id": id, "why": format!("status:{}", status.as_str()), "db": nixa_store::db_path()});
    let sent = match who.terminal() {
        Some(terminal) => {
            body["terminal"] = serde_json::json!(terminal);
            crate::http::post_within("/tasks/changed", &body, TELL_WAIT)
        }
        None => crate::http::window_call("POST", "/tasks/changed", Some(&body)),
    };
    if let Err(error) = sent {
        eprintln!("nixa: Nixa was not told that task #{id} is now {}: {error}", status.as_str());
    }
}

fn note_kind(state: TaskStatus, on: Option<BlockedOn>, needs_user: bool) -> CommentKind {
    match state {
        TaskStatus::Review => CommentKind::Result,
        TaskStatus::Blocked if needs_user || on.is_some_and(BlockedOn::is_question) => CommentKind::Question,
        TaskStatus::Doing => CommentKind::Progress,
        _ => CommentKind::Note,
    }
}

pub fn blocked_on(word: &str) -> Result<BlockedOn> {
    let low = word.trim().to_lowercase();
    let key = if matches!(low.as_str(), "user" | "owner" | "me") { BlockedOn::User.as_str().to_string() } else { low };
    BlockedOn::parse(&key).ok_or_else(|| {
        let others: Vec<&str> = BlockedOn::ALL.iter().filter(|b| **b != BlockedOn::User).map(|b| b.as_str()).collect();
        usage(format!("--on {word} is not known. Use user, {}", others.join(", ")))
    })
}

fn is_locked(conn: &Connection, space_id: &str) -> Result<bool> {
    Ok(!space_id.is_empty() && spaces::list_spaces(conn)?.iter().any(|s| s.space_id == space_id && s.locked))
}

fn refuse_locked(conn: &Connection, space_id: &str) -> Result<()> {
    if is_locked(conn, space_id)? {
        return Err(anyhow!("This space is locked and takes no new work. Only the user unlocks it. Put the work in an open space or leave the space out instead. Nothing was changed."));
    }
    Ok(())
}

fn route_locked(conn: &Connection, task: &Task) -> Result<()> {
    if task_rules::move_off_locked(conn, task)? {
        eprintln!("nixa: {}", task_rules::LOCKED_MOVED);
        return Ok(());
    }
    refuse_locked(conn, &task.space_id)
}

fn claim_for(conn: &Connection, id: i64, terminal: &str, name: Option<String>, who: &Caller) -> Result<Task> {
    let task = tasks::get_task(conn, id)?.ok_or_else(|| anyhow!("No task #{id}."))?;
    route_locked(conn, &task)?;
    let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| terminal.to_string());
    let by = Actor::in_terminal(caller::actor_of(who).kind, &name, terminal);
    let result = tasks::claim_task(conn, id, &by, "")?;
    if !result.ok {
        return Err(anyhow!("{} Nothing was claimed.", result.reason.trim()));
    }
    match result.task {
        Some(task) => Ok(task),
        None => tasks::get_task(conn, id)?.ok_or_else(|| anyhow!("No task #{id}.")),
    }
}

fn terminal_of(p: &mut Parsed, who: &Caller) -> Result<String> {
    let given = p.value(&["--terminal", "--in"]);
    let terminal = match (given, who.terminal()) {
        (Some(other), Some(own)) if other.trim() != own => {
            return Err(anyhow!("An agent claims work only for its own terminal {own}, not {other}. Nothing was claimed."));
        }
        (_, Some(own)) => own.to_string(),
        (Some(given), None) => given,
        (None, None) => return Err(usage("a claim needs a terminal: run it inside a Nixa terminal or give --terminal ID")),
    };
    terminal.parse::<TermId>().map_err(|_| usage(format!("{terminal} is not a terminal id")))?;
    Ok(terminal)
}

fn claim(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--terminal", "--in", "--name"], false)?;
    let id = task_id(&p, "claim")?;
    let who = caller::who(&ctx)?;
    let terminal = terminal_of(&mut p, &who)?;
    let name = p.value(&["--name"]);
    p.finish(1, false)?;
    let conn = open()?;
    let task = claim_for(&conn, id, &terminal, name, &who)?;
    print(&ctx, out, &task, &format!("Claimed {}.", task_line(&task)))
}

fn release(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--note"], false)?;
    let id = task_id(&p, "release")?;
    let note = p.value(&["--note"]);
    p.finish(1, false)?;
    let who = caller::who(&ctx)?;
    let conn = open()?;
    let before = tasks::get_task(&conn, id)?.ok_or_else(|| anyhow!("No task #{id}."))?;
    if let Some(terminal) = who.terminal() {
        task_rules::check_agent_release(&before, terminal)?;
    }
    let tx = conn.unchecked_transaction()?;
    let task = tasks::release_task(&tx, id)?;
    if let Some(text) = note.filter(|n| !n.trim().is_empty()) {
        tasks::add_comment(&tx, id, &CommentInput::new(CommentKind::Note, &text), &caller::actor_of(&who))?;
    }
    tx.commit()?;
    print(&ctx, out, &task, &format!("Released {}.", task_line(&task)))
}

pub fn space_here(conn: &Connection, cwd: &std::path::Path) -> Result<Option<Space>> {
    let mut best: Option<Space> = None;
    for s in spaces::list_spaces(conn)? {
        let folder = s.folder.trim_end_matches('/');
        if folder.is_empty() || !cwd.starts_with(folder) {
            continue;
        }
        if best.as_ref().is_none_or(|b| b.folder.len() < s.folder.len()) {
            best = Some(s);
        }
    }
    Ok(best)
}

fn next(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--space", "--terminal", "--in", "--name"], false)?;
    let take = p.flag(&["--claim"]);
    let key = p.value(&["--space"]);
    let who = if take { caller::who(&ctx)? } else { Caller::User };
    let terminal = if take { Some(terminal_of(&mut p, &who)?) } else { None };
    let name = p.value(&["--name"]);
    p.finish(0, false)?;
    let conn = open()?;
    let space = match key {
        Some(key) => find_space(&conn, &key)?,
        None => {
            let cwd = std::env::current_dir()?;
            space_here(&conn, &cwd)?.ok_or_else(|| usage("this folder is not in a space: give --space SPACE"))?
        }
    };
    let Some(task) = tasks::next_task(&conn, &space.space_id)? else {
        return print(&ctx, out, &Option::<Task>::None, &format!("No free todo task in {}.", space.name));
    };
    let task = match terminal {
        Some(terminal) => claim_for(&conn, task.id, &terminal, name, &who)?,
        None => task,
    };
    let word = if take { "Claimed" } else { "Next" };
    print(&ctx, out, &task, &format!("{word} {}.", task_line(&task)))
}

fn comment(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--kind"], false)?;
    let id = task_id(&p, "comment")?;
    let text = words(&p, 1);
    if text.trim().is_empty() {
        return Err(usage("nixa task comment needs text"));
    }
    let kind = match p.value(&["--kind"]) {
        Some(k) => CommentKind::parse(&k).ok_or_else(|| usage(format!("--kind {k} is not known")))?,
        None => CommentKind::Note,
    };
    p.finish(usize::MAX, true)?;
    let who = caller::who(&ctx)?;
    let conn = open()?;
    let made = tasks::add_comment(&conn, id, &CommentInput::new(kind, &text), &caller::actor_of(&who))?;
    let line = if made.is_some() { format!("Comment added to #{id}.") } else { format!("The same comment is already on #{id}.") };
    print(&ctx, out, &made, &line)
}

pub fn assign_kind(conn: &Connection, who: &str) -> Result<&'static str> {
    if who.parse::<TermId>().is_ok() {
        return Ok(KIND_TERMINAL);
    }
    match custom::get_agent(conn, who)? {
        Some(_) => Ok(KIND_CUSTOM),
        None => Err(anyhow!("{who} is not a terminal id or a custom agent id. Run nixa task agents to find one.")),
    }
}

fn assign(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    let id = task_id(&p, "assign")?;
    let who = p.pos.get(1).cloned().ok_or_else(|| usage("nixa task assign needs who: a terminal id or a custom agent id"))?;
    p.finish(2, false)?;
    let by = caller::who(&ctx)?;
    let conn = open()?;
    if by.terminal().is_some() {
        let task = tasks::must_task(&conn, id)?;
        task_rules::check_open_in(&task)?;
    }
    let kind = assign_kind(&conn, &who)?;
    let task = tasks::assign_task(&conn, id, kind, &who)?;
    print(&ctx, out, &task, &format!("Assigned: {}.", task_line(&task)))
}

fn edit(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--title", "--desc", "--description", "-d", "--space"], false)?;
    let id = task_id(&p, "edit")?;
    let title = p.value(&["--title"]).map(|t| t.trim().to_string());
    let description = p.value(&["--desc", "--description", "-d"]);
    let space = p.value(&["--space"]);
    p.finish(1, false)?;
    if title.is_none() && description.is_none() && space.is_none() {
        return Err(usage("nixa task edit needs --title, --desc or --space"));
    }
    if title.as_deref() == Some("") {
        return Err(usage("the title cannot be empty"));
    }
    let who = caller::who(&ctx)?;
    let conn = open()?;
    let tx = conn.unchecked_transaction()?;
    let before = tasks::must_task(&tx, id)?;
    if who.terminal().is_some() {
        task_rules::check_open_in(&before)?;
    }
    let mut patch = tasks::TaskPatch { title, description, ..tasks::TaskPatch::default() };
    let mut kept = None;
    if let Some(key) = space {
        let space = find_space(&tx, &key)?;
        if space.space_id != before.space_id && is_locked(&tx, &space.space_id)? {
            kept = Some(space.name);
        } else {
            patch.space_id = Some(space.space_id);
            patch.space_name = Some(space.name);
        }
    }
    let task = tasks::update_task(&tx, id, &patch)?;
    tx.commit()?;
    let mut line = format!("Changed: {}.", task_line(&task));
    if let Some(name) = kept {
        line.push_str(&format!(" Space {name} is locked, so the task stayed in its old space. The rest is saved."));
    }
    print(&ctx, out, &task, &line)
}

fn done(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    let id = task_id(&p, "done")?;
    let note = words(&p, 1);
    p.finish(usize::MAX, true)?;
    let who = caller::who(&ctx)?;
    let conn = open()?;
    if let Some(terminal) = who.terminal() {
        let before = tasks::must_task(&conn, id)?;
        task_rules::check_agent_status_in(&before, terminal, TaskStatus::Done, &note)?;
        let (state, text) = task_rules::agent_done(TaskStatus::Done, &note);
        let tx = conn.unchecked_transaction()?;
        tasks::set_status(&tx, id, state, None, false)?;
        tasks::add_comment(&tx, id, &CommentInput::new(CommentKind::Result, &text), &caller::actor_of(&who))?;
        tx.commit()?;
        let task = tasks::must_task(&conn, id)?;
        tell_brain(&who, id, task.status);
        return print(&ctx, out, &task, &format!("Now {}. {}", task_line(&task), task_rules::DONE_TO_REVIEW));
    }
    let task = tasks::done_task(&conn, id, &note, &caller::actor_of(&who))?;
    tell_brain(&who, id, task.status);
    print(&ctx, out, &task, &format!("Done: {}.", task_line(&task)))
}

fn live_terms(ctx: &Ctx) -> Vec<(String, Option<Status>)> {
    match ctx::call(ctx, &Request::List { parent: None, label: None }) {
        Ok(Reply::Terms { terms }) => terms.into_iter().filter(|t| t.running).map(|t| (t.id.to_string(), t.status)).collect(),
        _ => Vec::new(),
    }
}

fn list_agents(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--space"], false)?;
    let q = words(&p, 0);
    let conn = open()?;
    let place = match p.value(&["--space"]) {
        Some(key) => find_space(&conn, &key)?.space_id,
        None => String::new(),
    };
    p.finish(usize::MAX, true)?;
    let list = if q.trim().is_empty() { custom::list_agents(&conn)? } else { custom::find_agents(&conn, &q, &place)? };
    let live = live_terms(&ctx);
    let (held, unread) = match nixa_store::tasks::held_tasks(&conn) {
        Ok(held) => (held, false),
        Err(_) => (Vec::new(), true),
    };
    let lines: Vec<String> = list
        .iter()
        .take(LIST_MAX)
        .map(|a| {
            let seen = live.iter().find(|(t, _)| !a.terminal.is_empty() && t == &a.terminal);
            let mine = nixa_store::agent_busy::held_in(&held, &a.terminal);
            let state = nixa_store::agent_busy::with_claims(custom::agent_state(seen.is_some(), seen.and_then(|s| s.1)), &mine);
            format!("{}{}", custom::agent_line(a, state), nixa_store::agent_busy::claims_note(state, &mine))
        })
        .collect();
    let text = if !lines.is_empty() {
        lines.join("\n")
    } else if q.trim().is_empty() {
        "No custom agents yet.".to_string()
    } else {
        format!("No custom agent fits \"{q}\".")
    };
    let text = if unread { format!("{text}\n{}", nixa_store::agent_busy::UNREAD) } else { text };
    print(&ctx, out, &list, &text)
}

fn list_spaces(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let conn = open()?;
    let list = spaces::list_spaces(&conn)?;
    let text = if list.is_empty() {
        "No spaces yet.".to_string()
    } else {
        list.iter().map(|s| format!("{}  {}  {}", s.space_id, s.name, s.folder)).collect::<Vec<_>>().join("\n")
    };
    print(&ctx, out, &list, &text)
}
