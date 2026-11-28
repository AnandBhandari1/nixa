use crate::agents::AgentState;
use crate::tasks::{BlockedOn, Task, TaskStatus};

pub const UNREAD: &str = "Task claims could not be read, so an agent that holds a task may show as free.";

pub fn claim_text(task: i64, status: TaskStatus, blocked_on: Option<BlockedOn>, space: &str) -> String {
    let space = if space.is_empty() { ", no space" } else { "" };
    match (status, blocked_on) {
        (TaskStatus::Blocked, Some(on)) => format!("holds task #{task} (blocked on {}{space})", on.as_str()),
        (TaskStatus::Blocked, None) => format!("holds task #{task} (blocked{space})"),
        (status, _) => format!("holds task #{task} ({}{space})", status.as_str()),
    }
}

pub fn held_in<'a>(held: &'a [Task], terminal: &str) -> Vec<&'a Task> {
    if terminal.is_empty() {
        return Vec::new();
    }
    held.iter()
        .filter(|t| t.terminal_id == terminal && matches!(t.status, TaskStatus::Doing | TaskStatus::Blocked))
        .collect()
}

pub fn with_claims(state: AgentState, held: &[&Task]) -> AgentState {
    if matches!(state, AgentState::Closed | AgentState::Working | AgentState::Waiting) || held.is_empty() {
        return state;
    }
    if held.iter().any(|t| t.status == TaskStatus::Blocked) {
        AgentState::Waiting
    } else {
        AgentState::Working
    }
}

pub fn claims_note(state: AgentState, held: &[&Task]) -> String {
    if state == AgentState::Closed || held.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = held.iter().map(|t| claim_text(t.id, t.status, t.blocked_on, &t.space_id)).collect();
    format!(": {}", parts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;
    use crate::tasks::{Actor, AuthorKind, TaskInput, add_task, claim_task, held_tasks, set_status};

    #[test]
    fn claims_in_any_space_make_an_agent_busy() {
        let conn = temp_db("agent-busy");
        let user = Actor::new(AuthorKind::User, "user");
        let one = add_task(&conn, &TaskInput { title: "No space".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() }, &user).unwrap();
        let two = add_task(&conn, &TaskInput { title: "Other".into(), space_id: "s2".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() }, &user).unwrap();
        assert!(claim_task(&conn, one.id, &Actor::in_terminal(AuthorKind::Agent, "Dev", "m4"), "").unwrap().ok);
        assert!(claim_task(&conn, two.id, &Actor::in_terminal(AuthorKind::Agent, "Dev", "m5"), "").unwrap().ok);
        set_status(&conn, two.id, TaskStatus::Blocked, Some(BlockedOn::User), true).unwrap();
        let held = held_tasks(&conn).unwrap();
        let m4 = held_in(&held, "m4");
        assert_eq!(with_claims(AgentState::Free, &m4), AgentState::Working);
        assert_eq!(claims_note(AgentState::Working, &m4), format!(": holds task #{} (doing, no space)", one.id));
        let m5 = held_in(&held, "m5");
        assert_eq!(with_claims(AgentState::Free, &m5), AgentState::Waiting);
        assert!(claims_note(AgentState::Waiting, &m5).contains("blocked on user"));
        assert_eq!(with_claims(AgentState::Closed, &m4), AgentState::Closed);
        assert_eq!(with_claims(AgentState::Waiting, &m4), AgentState::Waiting);
        assert!(held_in(&held, "").is_empty());
        assert_eq!(with_claims(AgentState::Free, &held_in(&held, "m9")), AgentState::Free);
    }
}
