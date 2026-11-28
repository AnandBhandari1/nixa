use crate::app::{App, lock};
use crate::queue::JobInput;
use nixa_store::tasks::{self, Actor, AuthorKind, TaskFilter, TaskStatus};
use serde_json::json;
use std::sync::Arc;

pub fn actor_name(title: &str, terminal: &str) -> String {
    let title = title.trim();
    if title.is_empty() { terminal.to_string() } else { title.to_string() }
}

pub async fn watch_use(app: &Arc<App>, terminal: &str, message: &str, title: &str) -> Option<i64> {
    let message = message.trim();
    if terminal.is_empty() || message.is_empty() || crate::queue::deep_seat(app, terminal) {
        return None;
    }
    if let Ok(id) = crate::engine::term_id(terminal)
        && let Ok(screen) = crate::engine::read(&id, Some(crate::followup::flow::REPORT_LINES), false).await
    {
        crate::followup::flow::before_send(app, terminal, &screen);
    }
    crate::followup::flow::job_started(app, terminal, message);
    lock(&app.watch).jobs.insert(terminal.to_string(), crate::util::now_ms());
    link(app, terminal, message, title).await
}

fn holds_work(app: &App, terminal: &str) -> bool {
    let Ok(conn) = crate::queue::db(app) else { return false };
    let filter = TaskFilter {
        terminal: terminal.to_string(),
        status: vec![TaskStatus::Doing, TaskStatus::Blocked],
        ..TaskFilter::default()
    };
    tasks::list_tasks(&conn, &filter).is_ok_and(|list| !list.is_empty())
}

async fn link(app: &Arc<App>, terminal: &str, message: &str, title: &str) -> Option<i64> {
    let space = app.space_of_terminal(terminal);
    let folder = lock(&app.watch).world.get(terminal).map(|s| s.info.cwd.clone()).unwrap_or_default();
    let place = space.as_ref().map(|s| s.folder.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| folder.clone());
    let base = if place.is_empty() || holds_work(app, terminal) { String::new() } else { crate::queue::base_of(&place).await };
    let mut actor = Actor::in_terminal(AuthorKind::Agent, &actor_name(title, terminal), terminal);
    actor.space_id = space.as_ref().map(|s| s.id.clone()).unwrap_or_default();
    let input = JobInput {
        terminal: terminal.to_string(),
        actor,
        request: message.to_string(),
        folder,
        space,
        base,
    };
    let id = crate::queue::job_started(app, &input)?;
    crate::queue::log("custom agent job", json!({"task": id, "terminal": terminal}));
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_actor_is_the_agent_title() {
        assert_eq!(actor_name(" Builder ", "m5"), "Builder");
        assert_eq!(actor_name("", "m5"), "m5");
    }

    #[tokio::test]
    async fn an_empty_message_starts_no_job() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        assert_eq!(watch_use(&app, "m5", "  ", "Builder").await, None);
        assert_eq!(watch_use(&app, "", "Fix it", "Builder").await, None);
        assert!(!lock(&app.watch).jobs.contains_key("m5"));
    }

    #[tokio::test]
    async fn a_used_agent_job_gets_a_task() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let id = link(&app, "m5", "Fix the parser and add its tests", "Builder").await.unwrap();
        let conn = crate::queue::db(&app).unwrap();
        let task = tasks::get_task(&conn, id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Doing);
        assert_eq!(task.terminal_id, "m5");
        assert_eq!(task.claimed_by, "Builder");
        assert_eq!(link(&app, "m5", "And the docs", "Builder").await, Some(id));
    }
}
