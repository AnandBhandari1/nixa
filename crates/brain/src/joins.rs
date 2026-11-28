use crate::app::App;
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

pub fn install() {
    crate::queue::set_review(review);
}

fn active_run(app: &App, id: i64) -> bool {
    crate::workflows::open_db(app).ok().and_then(|conn| crate::workflows::stage_info(&conn, id)).is_some()
}

fn task_terminal(app: &App, id: i64) -> Option<String> {
    let conn = nixa_store::open(&app.db_file).ok()?;
    nixa_store::tasks::must_task(&conn, id).ok().map(|t| t.terminal_id).filter(|t| !t.is_empty())
}

pub fn check_held(app: &Arc<App>, id: i64) -> Option<String> {
    let terminal = task_terminal(app, id);
    let held = crate::modes::hold_work(app, "check", &format!("check task #{id}"), terminal.as_deref())?;
    Some(held)
}

pub fn review(app: Arc<App>, id: i64, _why: String) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        if check_held(&app, id).is_some() {
            crate::util::log("joins", &format!("task #{id} review waits for the hold"));
            return;
        }
        let result = if active_run(&app, id) {
            crate::workflows::settle(&app, id, crate::workflows::Settle::default()).await.map(|done| done.text)
        } else {
            crate::review_flow::settle_task(&app, id, true, Vec::new()).await.map(|settled| format!("{:?}", settled.outcome))
        };
        match result {
            Ok(text) => crate::util::log("joins", &format!("task #{id} review: {}", crate::util::clip_flat(&text, 200))),
            Err(error) => crate::util::log("joins", &format!("task #{id} review did not run: {error}")),
        }
    })
}

pub fn stage_text(app: &App, id: i64, prompt: String) -> String {
    let Some(info) = crate::workflows::open_db(app).ok().and_then(|conn| crate::workflows::stage_info(&conn, id)) else {
        return prompt;
    };
    let mut out = if info.checks { prompt } else { prompt.replace(&format!("\n{}", crate::queue::prompt::CHECKS_LINE), "") };
    if !info.lines.is_empty() {
        out.push_str("\nWorkflow stage:\n");
        out.push_str(&info.lines.join("\n"));
    }
    out
}

pub fn start_of(value: &Value) -> Option<i64> {
    (value.get("type").and_then(Value::as_str) == Some(crate::workflows::effects::START_EVENT)).then(|| value.get("task").and_then(Value::as_i64)).flatten()
}

pub async fn run(app: Arc<App>) {
    let mut events = app.out.subscribe();
    loop {
        match events.recv().await {
            Ok(value) => {
                if let Some(id) = start_of(&value) {
                    let app = app.clone();
                    tokio::spawn(async move {
                        if let Err(error) = crate::queue::start_now(&app, id, false).await {
                            crate::util::log("joins", &format!("workflow stage of task #{id} did not start: {error}"));
                        }
                    });
                }
            }
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workflow_start_events_name_their_task() {
        assert_eq!(start_of(&json!({"type": "workflow_start", "task": 7})), Some(7));
        assert_eq!(start_of(&json!({"type": "tasks_changed", "task": 7})), None);
        assert_eq!(start_of(&json!({"type": "workflow_start"})), None);
    }
}
