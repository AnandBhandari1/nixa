use super::model::{self, WorkflowInfo};
use crate::app::App;
use crate::deep::{Ask, AskState};
use crate::util::clip;
use nixa_store::Connection;
use nixa_store::tasks::{self, Actor, AuthorKind, CommentInput, CommentKind, TaskPatch};
use nixa_store::workflows as store;
use regex::Regex;
use std::sync::LazyLock;

pub const LINE_MAX: usize = 480;
pub const WHEN_MAX: usize = 90;
pub const HEAD_LINES: usize = 4;

static NAMED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bworkflow[:\s]+([a-z0-9][a-z0-9-]{0,59})\b").expect("workflow pick"));

pub fn flows_line(flows: &[WorkflowInfo]) -> String {
    let usable: Vec<String> = flows.iter().filter(|f| f.broken.is_none()).map(|f| format!("{}: {}", f.name, clip(&f.when, WHEN_MAX))).collect();
    if usable.is_empty() {
        return String::new();
    }
    clip(&format!("Workflows (add \"workflow <name>\" to the pick when one fits, for example \"row 3 workflow <name>\"; leave it out for the normal flow): {}.", usable.join("; ")), LINE_MAX)
}

pub fn with_flows(notes: &str, line: &str) -> String {
    if line.is_empty() {
        return notes.to_string();
    }
    let mut lines: Vec<&str> = notes.lines().collect();
    let at = HEAD_LINES.min(lines.len());
    lines.insert(at, line);
    lines.join("\n")
}

pub fn flow_of_pick(text: &str, names: &[String]) -> Option<String> {
    let named = NAMED.captures(text)?.get(1)?.as_str().to_lowercase();
    names.contains(&named).then_some(named)
}

pub fn choose(text: &str, names: &[String], task_flow: &str, run_active: bool) -> Option<String> {
    if !task_flow.is_empty() || run_active {
        return None;
    }
    flow_of_pick(text, names)
}

pub fn infos(conn: &Connection) -> Vec<WorkflowInfo> {
    store::list_workflows(conn).unwrap_or_default().iter().map(|w| model::info(&w.name, &w.body)).collect()
}

pub fn usable_names(conn: &Connection) -> Vec<String> {
    infos(conn).into_iter().filter(|f| f.broken.is_none()).map(|f| f.name).collect()
}

pub fn line_for(app: &App) -> String {
    super::open_db(app).map(|conn| flows_line(&infos(&conn))).unwrap_or_default()
}

pub fn apply_pick(app: &App, task_id: i64, ask: &Ask, held: bool) -> Option<String> {
    if held || ask.state != AskState::Answered {
        return None;
    }
    let conn = super::open_db(app).ok()?;
    let task = tasks::get_task(&conn, task_id).ok().flatten()?;
    let active = store::get_run(&conn, task_id).ok().flatten().is_some_and(|run| store::ACTIVE_STATES.contains(&run.state.as_str()));
    let picked = choose(ask.pick.as_deref().unwrap_or_default(), &usable_names(&conn), &task.workflow, active)?;
    let patch = TaskPatch { workflow: Some(picked.clone()), ..TaskPatch::default() };
    tasks::update_task(&conn, task_id, &patch).ok()?;
    crate::queue::changed(app, "workflow", Some(task_id));
    let note = format!("The deep brain picked the workflow {picked} for this task ({}). The run starts when the task starts.", ask.id);
    if let Err(error) = tasks::add_comment(&conn, task_id, &CommentInput::new(CommentKind::Note, &note), &Actor::new(AuthorKind::System, "Nixa")) {
        crate::util::log("workflows", &format!("comment on #{task_id} failed: {error}"));
    }
    Some(picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, when: &str, broken: bool) -> WorkflowInfo {
        WorkflowInfo { name: name.into(), title: name.into(), when: when.into(), nodes: 3, broken: broken.then(|| "bad".to_string()) }
    }

    #[test]
    fn flows_line_lists_usable_workflows() {
        assert_eq!(flows_line(&[]), "");
        assert_eq!(flows_line(&[info("old", "x", true)]), "");
        let line = flows_line(&[info("build-review-fix", "Code changes that need a review", false), info("old", "x", true)]);
        assert!(line.starts_with("Workflows (add \"workflow <name>\""));
        assert!(line.contains("build-review-fix: Code changes that need a review"));
        assert!(!line.contains("old"));
        let long: Vec<WorkflowInfo> = (0..40).map(|i| info(&format!("flow-{i}"), "a long when line that says when to use this flow", false)).collect();
        assert!(flows_line(&long).chars().count() <= LINE_MAX + 1);
    }

    #[test]
    fn picks_read_a_named_workflow() {
        let names = vec!["build-review-fix".to_string(), "docs".to_string()];
        assert_eq!(flow_of_pick("row 3 workflow build-review-fix", &names).as_deref(), Some("build-review-fix"));
        assert_eq!(flow_of_pick("a9, Workflow: DOCS", &names).as_deref(), Some("docs"));
        assert_eq!(flow_of_pick("row 3 workflow other", &names), None);
        assert_eq!(flow_of_pick("row 3", &names), None);
        assert_eq!(choose("row 1 workflow docs", &names, "", false).as_deref(), Some("docs"));
        assert_eq!(choose("row 1 workflow docs", &names, "build-review-fix", false), None);
        assert_eq!(choose("row 1 workflow docs", &names, "", true), None);
    }

    #[test]
    fn flows_go_after_the_head_lines() {
        let notes = "Task #1: x\nNo one is assigned yet.\nThe agent map has 2 rows.\nOr answer a custom agent id.\nCustom agents in s: none.\nCriteria: 1) fast";
        let out = with_flows(notes, "Workflows: docs.");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[4], "Workflows: docs.");
        assert_eq!(lines[5], "Custom agents in s: none.");
        assert_eq!(with_flows(notes, ""), notes);
        assert_eq!(with_flows("one", "two"), "one\ntwo");
    }
}
