use super::lock::task_locked;
use super::{changed, db, log, note};
use crate::app::App;
use nixa_store::Connection;
use nixa_store::task_waits::{KEY, all_waits, open_waits, set_waits, waits_of, waits_text};
use nixa_store::tasks::{self, Actor, AuthorKind, BlockedOn, CommentInput, CommentKind, Task, TaskStatus};
use serde_json::json;
use std::sync::Arc;

pub fn waits_on_open(conn: &Connection, id: i64) -> bool {
    !open_waits(conn, id).is_empty()
}

pub fn refuse_open(conn: &Connection, id: i64) -> Result<(), String> {
    let open = open_waits(conn, id);
    if open.is_empty() {
        return Ok(());
    }
    Err(format!("Task #{id} waits on {}, still open. Nixa starts it after they close, or the user can start it now.", waits_text(&open)))
}

pub const START: &str = "wait_start:";
pub const PLAN: &str = "wait_plan:";

pub fn hold(conn: &Connection, id: i64, kind: &str) {
    if let Err(error) = tasks::set_task_meta(conn, &format!("{kind}{id}"), "1") {
        log("wait hold not saved", json!({"task": id, "error": error.to_string()}));
    }
}

pub fn refuse_and_hold(conn: &Connection, id: i64, kind: &str) -> Result<(), String> {
    let refused = refuse_open(conn, id);
    if refused.is_err() {
        hold(conn, id, kind);
    }
    refused
}

fn take(conn: &Connection, id: i64, kind: &str) -> bool {
    tasks::remove_task_meta(conn, &format!("{kind}{id}")).unwrap_or(false)
}

pub fn keep_blocked(conn: &Connection, task: &Task) -> bool {
    task.status == TaskStatus::Blocked && waits_on_open(conn, task.id)
}

fn tell_holder(conn: &Connection, id: i64, text: &str) -> anyhow::Result<()> {
    let by = Actor::new(AuthorKind::Nixa, "Nixa");
    tasks::add_comment(conn, id, &CommentInput::new(CommentKind::Note, text), &by)?;
    Ok(())
}

fn resume(conn: &Connection, task: &Task, names: &str) -> anyhow::Result<&'static str> {
    let on_task = task.status == TaskStatus::Blocked && task.blocked_on == Some(BlockedOn::Task);
    let held = !task.terminal_id.is_empty();
    if on_task && !held {
        tasks::set_status(conn, task.id, TaskStatus::Todo, None, false)?;
        note(conn, task.id, &format!("All tasks it waited on are closed ({names}). Back to todo."), CommentKind::Note);
        return Ok("todo");
    }
    if on_task {
        tasks::set_status(conn, task.id, TaskStatus::Doing, None, false)?;
        tasks::touch_terminal(conn, &task.terminal_id)?;
        tell_holder(conn, task.id, &format!("All tasks this task waited on are closed ({names}). It is back in doing. Go on with it."))?;
        return Ok("doing");
    }
    if held && task.status == TaskStatus::Doing {
        tell_holder(conn, task.id, &format!("All tasks this task waited on are closed ({names}). Go on with it."))?;
        return Ok("told holder");
    }
    let text = if task.status == TaskStatus::Blocked {
        format!("All tasks it waited on are closed ({names}). It is still blocked on {}.", task.blocked_on.unwrap_or(BlockedOn::Other).as_str())
    } else {
        format!("All tasks it waited on are closed ({names}). It can start now.")
    };
    note(conn, task.id, &text, CommentKind::Note);
    Ok("noted")
}

fn settle(conn: &Connection, id: i64) -> anyhow::Result<Option<(Vec<i64>, &'static str)>> {
    let tx = conn.unchecked_transaction()?;
    let on = waits_of(&tx, id);
    let Some(task) = tasks::get_task(&tx, id)? else {
        tasks::remove_task_meta(&tx, &format!("{KEY}{id}"))?;
        tx.commit()?;
        return Ok(None);
    };
    if task.status.is_closed() {
        tasks::remove_task_meta(&tx, &format!("{KEY}{id}"))?;
        tx.commit()?;
        return Ok(None);
    }
    if on.is_empty() || waits_on_open(&tx, id) {
        return Ok(None);
    }
    set_waits(&tx, id, &[])?;
    let how = resume(&tx, &task, &waits_text(&on))?;
    tx.commit()?;
    Ok(Some((on, how)))
}


pub async fn sweep(app: &Arc<App>) {
    let Ok(conn) = db(app) else { return };
    let rows = nixa_store::spaces::list_spaces(&conn).unwrap_or_default();
    let mut due = Vec::new();
    for (id, _) in all_waits(&conn) {
        if let Some(task) = tasks::get_task(&conn, id).ok().flatten()
            && task_locked(&task, &rows).is_some()
        {
            continue;
        }
        match settle(&conn, id) {
            Ok(Some((on, how))) => {
                log("waits closed", json!({"task": id, "on": on, "how": how}));
                if matches!(how, "todo" | "noted") {
                    due.push((id, take(&conn, id, PLAN), take(&conn, id, START)));
                }
                changed(app, "queue", Some(id));
            }
            Ok(None) => {}
            Err(error) => log("waits not settled", json!({"task": id, "error": error.to_string()})),
        }
    }
    drop(conn);
    for (id, plan, start) in due {
        if plan {
            super::plan::advance(app, id, false).await;
        } else if start
            && let Err(error) = super::start_now(app, id, false).await
        {
            log("start after waits failed", json!({"task": id, "error": crate::util::clip(&error, 200)}));
        }
    }
}
