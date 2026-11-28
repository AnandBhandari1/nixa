use crate::app::App;
use anyhow::{Result, bail};
use nixa_store::tasks::{CommentInput, CommentKind, Task, TaskStatus, add_comment, get_task, set_status, unclaim_task};
use std::sync::Arc;

const RESEND_MS: u64 = 3000;

fn still_waiting(actual: Option<Task>, checked: &Task) -> bool {
    actual.is_some_and(|task| {
        task.status == TaskStatus::Doing
            && task.terminal_id == checked.terminal_id
            && task.claimed_at == checked.claimed_at
            && task.updated_at == checked.updated_at
            && task.archived_at == 0
    })
}

async fn deliver(terminal: &str, text: &str) -> Result<()> {
    let id = crate::engine::term_id(terminal)?;
    crate::engine::send(&id, text, true, false).await.map(|_| ())
}

pub(super) async fn send(app: &Arc<App>, task: &Task, terminal: &str, text: &str) -> Result<Option<String>> {
    {
        let conn = super::open(app)?;
        if !still_waiting(get_task(&conn, task.id)?, task) {
            bail!("Task #{} changed before Nixa could send the fix request.", task.id);
        }
    }

    crate::modes::gate(app, "fix", Some(terminal));
    let mut result = deliver(terminal, text).await;
    if result.is_err() {
        tokio::time::sleep(std::time::Duration::from_millis(RESEND_MS)).await;
        result = deliver(terminal, text).await;
    }
    match result {
        Ok(()) => Ok(None),
        Err(error) => {
            crate::util::log("review_flow", &format!("could not send to {terminal}: {error:#}"));
            let reason = format!("Nixa could not deliver the fix request to {terminal}.");
            restart(app, task, terminal, &reason).map(Some)
        }
    }
}

pub(super) fn restart(app: &App, task: &Task, terminal: &str, reason: &str) -> Result<String> {
    let conn = super::open(app)?;
    let tx = conn.unchecked_transaction()?;
    if !still_waiting(get_task(&tx, task.id)?, task) {
        bail!("Task #{} changed while Nixa tried to send the fix request.", task.id);
    }
    set_status(&tx, task.id, TaskStatus::Todo, None, false)?;
    unclaim_task(&tx, task.id)?;
    let line = format!("{reason} The task goes back to todo and starts again on a fresh agent.");
    add_comment(&tx, task.id, &CommentInput::new(CommentKind::Error, &line), &super::settle::nixa())?;
    tx.commit()?;
    crate::modes::note(app, "fix", &format!("task #{}: start again on a fresh agent", task.id), Some(terminal), Some(reason));
    crate::queue::changed(app, "status:todo", Some(task.id));
    Ok(format!("Task #{}: {line}", task.id))
}
