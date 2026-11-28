use super::caller::{self, Caller};
use super::{open, print, route_locked, task_id};
use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use anyhow::{Result, bail};
use nixa_protocol::paths;
use nixa_store::Connection;
use nixa_store::claims::{self, ClaimInput, FileHolder, MAX_HOLD_FILES};
use nixa_store::spaces;
use nixa_store::task_rules;
use nixa_store::tasks::{self, Task};
use nixa_store::time::now_ms;
use std::collections::BTreeSet;
use std::io::Write;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    match argv.first().map(String::as_str) {
        Some("claim") => claim(ctx, &argv[1..], out),
        Some("release") => release(ctx, &argv[1..], out),
        _ => list(ctx, argv, out),
    }
}

fn locked_tasks(conn: &Connection, holders: Vec<FileHolder>) -> Result<Vec<FileHolder>> {
    let locked: BTreeSet<String> = spaces::list_spaces(conn)?.into_iter().filter(|s| s.locked).map(|s| s.space_id).collect();
    if locked.is_empty() {
        return Ok(holders);
    }
    let mut shown = Vec::new();
    for holder in holders {
        let space = tasks::get_task(conn, holder.task)?.map(|t| t.space_id).unwrap_or_default();
        if !locked.contains(&space) {
            shown.push(holder);
        }
    }
    Ok(shown)
}

fn list(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--file"], false)?;
    let path = p.value(&["--file"]).or_else(|| p.pos.first().cloned()).filter(|f| !f.trim().is_empty());
    p.finish(1, false)?;
    let conn = open()?;
    let now = now_ms();
    let mut holders = match path.as_deref() {
        Some(file) => claims::who_holds(&conn, file, now)?,
        None => claims::file_holders(&conn, now)?,
    };
    if std::env::var_os(paths::ENV_TERM_ID).is_some() {
        holders = locked_tasks(&conn, holders)?;
    }
    let text = if !holders.is_empty() {
        holders.iter().map(claims::holder_text).collect::<Vec<_>>().join("\n")
    } else if let Some(file) = &path {
        format!("No one holds {file}.")
    } else {
        "No files held right now.".to_string()
    };
    print(&ctx, out, &holders, &text)
}

fn holder_terminal(task: &Task, who: &Caller, given: Option<String>) -> Result<String> {
    let terminal = match (who.terminal(), given) {
        (Some(own), Some(other)) if other.trim() != own => {
            bail!("An agent holds files only for its own terminal {own}, not {other}. Nothing was changed.")
        }
        (Some(own), _) => {
            task_rules::check_held_in(task, own)?;
            own.to_string()
        }
        (None, Some(given)) => given.trim().to_string(),
        (None, None) => task.terminal_id.clone(),
    };
    if task.terminal_id.is_empty() {
        bail!("Task #{} has no claim. Claim it first. Nothing was changed.", task.id);
    }
    if task.terminal_id != terminal {
        bail!("Task #{} is held by terminal {}, not {terminal}. Nothing was changed.", task.id, task.terminal_id);
    }
    Ok(terminal)
}

fn claim(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--terminal", "--in"], false)?;
    let id = task_id(&p, "files claim")?;
    let given = p.value(&["--terminal", "--in"]);
    let files: Vec<String> = p.pos.iter().skip(1).chain(p.rest.iter()).filter(|f| !f.trim().is_empty()).cloned().collect();
    p.finish(usize::MAX, true)?;
    if files.is_empty() {
        return Err(usage("nixa task files claim needs a task id and at least one file or folder"));
    }
    if files.len() > MAX_HOLD_FILES {
        return Err(usage(format!("nixa task files claim takes at most {MAX_HOLD_FILES} files or folders")));
    }
    let who = caller::who(&ctx)?;
    let conn = open()?;
    let task = tasks::must_task(&conn, id)?;
    let terminal = holder_terminal(&task, &who, given)?;
    if who.terminal().is_some() {
        route_locked(&conn, &task)?;
    }
    let name = if task.claimed_by.is_empty() { terminal.clone() } else { task.claimed_by.clone() };
    let input = ClaimInput { task: id, terminal, name, files };
    let outcome = claims::claim_files(&conn, &input, now_ms())?;
    let mut lines = vec![format!(
        "Held for task #{id} in terminal {}: {}.",
        outcome.holder.terminal,
        outcome.holder.files.join(", ")
    )];
    lines.extend(outcome.clashes.iter().map(claims::clash_text));
    print(&ctx, out, &outcome, &lines.join("\n"))
}

fn release(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--terminal", "--in"], false)?;
    let given = p.value(&["--terminal", "--in"]);
    let id = match p.pos.first() {
        Some(word) => Some(args::number::<i64>(word.trim_start_matches('#'), "task id")?),
        None => None,
    };
    p.finish(1, false)?;
    let who = caller::who(&ctx)?;
    let conn = open()?;
    let task = match id {
        Some(id) => Some(tasks::must_task(&conn, id)?),
        None => None,
    };
    let terminal = match (who.terminal(), given) {
        (Some(own), Some(other)) if other.trim() != own => {
            bail!("An agent frees only the files of its own terminal {own}, not {other}. Nothing was changed.")
        }
        (Some(own), _) => own.to_string(),
        (None, Some(given)) => given.trim().to_string(),
        (None, None) => match task.as_ref().map(|t| t.terminal_id.clone()).filter(|t| !t.is_empty()) {
            Some(terminal) => terminal,
            None => return Err(usage("nixa task files release needs a task id with a claim, or --terminal ID")),
        },
    };
    let freed = claims::release_files(&conn, &terminal, id, now_ms())?;
    let what = match id {
        Some(id) => format!(" for task #{id}"),
        None => String::new(),
    };
    let lists = if freed == 1 { "list" } else { "lists" };
    let line = format!("Freed {freed} held file {lists}{what} in terminal {terminal}.");
    print(&ctx, out, &serde_json::json!({ "released": freed, "terminal": terminal, "task": id }), &line)
}
