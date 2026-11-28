use super::driver::{self, ASK_KIND, ASK_WAIT_MS, Effect};
use super::{RUNS_CHANGED, open_db};
use crate::app::App;
use crate::deep::AskInput;
use crate::engine;
use nixa_store::tasks::{self, Actor, AuthorKind, CommentInput, CommentKind, TaskStatus};
use serde_json::json;
use std::sync::Arc;

pub const START_EVENT: &str = "workflow_start";

async fn blocking(app: &Arc<App>, work: impl FnOnce(&nixa_store::Connection) + Send + 'static) {
    let worker = app.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let _guard = crate::app::lock(&worker.workflow_writes);
        if let Ok(conn) = open_db(&worker) {
            work(&conn);
        }
    })
    .await;
}

async fn saved_seat(app: &Arc<App>, terminal: &str) -> bool {
    let worker = app.clone();
    let terminal = terminal.to_string();
    tokio::task::spawn_blocking(move || {
        open_db(&worker).ok().and_then(|conn| nixa_store::agents::agent_by_terminal(&conn, &terminal).ok().flatten()).is_some()
    })
    .await
    .unwrap_or(true)
}

async fn send(terminal: &str, text: &str) -> bool {
    let Ok(id) = engine::term_id(terminal) else {
        return false;
    };
    engine::send(&id, text, true, false).await.is_ok()
}

async fn close(terminal: &str) {
    if let Ok(id) = engine::term_id(terminal) {
        let _ = engine::remove(&id).await;
    }
}

async fn quiet(app: Arc<App>, terminal: String, line: String) {
    let seat = saved_seat(&app, &terminal).await;
    let told = send(&terminal, &line).await;
    if !seat {
        close(&terminal).await;
    }
    crate::util::log("workflow", &format!("stop line to {terminal}: {}", if told { "sent" } else { "not sent" }));
}

async fn retry(app: &Arc<App>, task: i64, terminal: &str, text: &str) {
    if send(terminal, text).await {
        return;
    }
    let line = format!("Nixa could not send the retry to {terminal}, so the stage starts again on a fresh agent.");
    blocking(app, move |conn| {
        let _ = tasks::set_status(conn, task, TaskStatus::Todo, None, false);
        let _ = tasks::add_comment(conn, task, &CommentInput::new(CommentKind::Error, &line), &Actor::new(AuthorKind::System, "Nixa"));
    })
    .await;
    app.send_out(json!({"type": START_EVENT, "task": task}));
}

async fn nixa_answer(app: &Arc<App>, task: i64, pick: &str) {
    if crate::queue::paused(app) {
        return;
    }
    if let Err(error) = Box::pin(super::act(app, task, "answer", driver::NIXA_BY, pick)).await {
        app.info(&format!("Task #{task}: Nixa could not answer the workflow question: {error}"));
    }
}

async fn failed(app: &Arc<App>, task: i64, error: String) {
    if let Err(error) = Box::pin(super::with_ctx(app, move |ctx| driver::deep_failed(ctx, task, &error))).await {
        app.info(&format!("Task #{task}: Nixa could not recover the workflow decision: {error}"));
    }
}

async fn ask_deep(app: &Arc<App>, task: i64, space: &str, question: String, options: Vec<String>, notes: String) {
    let space = match crate::deep_service::resolve(app, Some(space), None) {
        Ok(space) => space,
        Err(error) => {
            failed(app, task, error).await;
            return;
        }
    };
    let input = AskInput { question, options, notes, kind: Some(ASK_KIND.into()), wait: Some(ASK_WAIT_MS), brain: None };
    let result = crate::deep_service::ask(app, input, &space);
    match result {
        Ok((ask, _)) => {
            let id = ask.id.clone();
            blocking(app, move |conn| {
                let _ = driver::set_deep(conn, task, &id);
            })
            .await;
            crate::watch::deep_tick(app).await;
        }
        Err(error) => failed(app, task, error).await,
    }
}

pub async fn apply(app: &Arc<App>, effects: Vec<Effect>) {
    let mut changed: Vec<i64> = Vec::new();
    for effect in effects {
        match effect {
            Effect::Quiet { terminal, line } => {
                tokio::spawn(quiet(app.clone(), terminal, line));
            }
            Effect::Retry { task, terminal, text } => retry(app, task, &terminal, &text).await,
            Effect::Close { terminal } => {
                if !saved_seat(app, &terminal).await {
                    close(&terminal).await;
                }
            }
            Effect::AskDeep { task, space, question, options, notes } => ask_deep(app, task, &space, question, options, notes).await,
            Effect::Pick { task, pick } => nixa_answer(app, task, &pick).await,
            Effect::Start { task } => app.send_out(json!({"type": START_EVENT, "task": task})),
            Effect::Info { text } => app.info(&text),
            Effect::Changed { task } => {
                if !changed.contains(&task) {
                    changed.push(task);
                }
            }
        }
    }
    for task in changed {
        app.send_out(json!({"type": RUNS_CHANGED, "task": task}));
    }
}
