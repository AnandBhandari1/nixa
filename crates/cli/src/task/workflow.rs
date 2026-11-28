use super::caller::{self, Caller};
use super::{open, print, task_id, task_line};
use crate::args::usage;
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::{Result, bail};
use nixa_store::Connection;
use nixa_store::task_rules;
use nixa_store::tasks::{self, CommentInput, CommentKind, Task, TaskPatch};
use nixa_store::workflows::{self, WorkflowRun};
use serde_json::{Value, json};
use std::io::Write;

pub const ACTIVE: [&str; 3] = ["running", "paused", "waiting"];
pub const RUN_ACTS: [&str; 5] = ["pause", "resume", "cancel", "retry", "answer"];
pub const WORDS: &str = "show, set NAME, clear, pause, resume, cancel, retry or answer TEXT";

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    let id = task_id(&p, "workflow")?;
    let what = p.pos.get(1).map(|w| w.trim().to_lowercase()).unwrap_or_else(|| "show".into());
    let text = p.pos.iter().skip(2).chain(p.rest.iter()).cloned().collect::<Vec<_>>().join(" ").trim().to_string();
    p.finish(usize::MAX, true)?;
    let conn = open()?;
    let task = tasks::must_task(&conn, id)?;
    let run = workflows::get_run(&conn, id)?;
    match what.as_str() {
        "show" => {
            if !text.is_empty() {
                return Err(usage("nixa task workflow ID show takes no more words"));
            }
            let lines = show_lines(&task, run.as_ref());
            print(&ctx, out, &json!({ "task": task, "run": run }), &lines.join("\n"))
        }
        "set" | "clear" => {
            if what == "clear" && !text.is_empty() {
                return Err(usage("nixa task workflow ID clear takes no more words"));
            }
            if what == "set" && text.is_empty() {
                return Err(usage("nixa task workflow ID set needs a workflow name"));
            }
            let who = caller::who(&ctx)?;
            let name = if what == "clear" { String::new() } else { text };
            let task = set_workflow(&conn, &task, run.as_ref(), &name, &who)?;
            let line = if name.is_empty() { format!("Task #{id} has no workflow now.") } else { format!("Task #{id} follows workflow {name} when it starts.") };
            print(&ctx, out, &task, &line)
        }
        act if RUN_ACTS.contains(&act) => {
            if act == "answer" && text.is_empty() {
                return Err(usage("nixa task workflow ID answer needs the choice, for example: Go on from this box"));
            }
            if act != "answer" && !text.is_empty() {
                return Err(usage(format!("nixa task workflow ID {act} takes no more words")));
            }
            if run.is_none() {
                bail!("Task #{id} has no workflow run. Nothing was changed.");
            }
            let reply = http::window_call("POST", "/workflows", Some(&json!({ "action": act, "task": id, "text": text })))?;
            let line = reply.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).map(str::to_string).unwrap_or_else(|| format!("Done: {act} for task #{id}."));
            print(&ctx, out, &reply, &line)
        }
        other => Err(usage(format!("nixa task workflow ID {other} is not known. Use {WORDS}"))),
    }
}

fn state_of(run: &WorkflowRun) -> Value {
    serde_json::from_str(&run.run).unwrap_or(Value::Null)
}

pub fn is_active(run: Option<&WorkflowRun>) -> bool {
    run.is_some_and(|r| ACTIVE.contains(&r.state.as_str()))
}

pub fn run_line(run: &WorkflowRun) -> String {
    let state = state_of(run);
    let mut line = format!("Run of {}: {}", run.workflow, run.state);
    if let Some(node) = state.get("node").and_then(Value::as_str).filter(|n| !n.is_empty()) {
        line.push_str(&format!(", at box {node}"));
    }
    let steps = state.get("steps").and_then(Value::as_u64).unwrap_or(0);
    line.push_str(&format!(", {steps} steps"));
    if let Some(why) = state.pointer("/ask/why").and_then(Value::as_str).filter(|w| !w.trim().is_empty()) {
        line.push_str(&format!(". It waits: {why}"));
    }
    line.push('.');
    line
}

pub fn show_lines(task: &Task, run: Option<&WorkflowRun>) -> Vec<String> {
    let mut lines = vec![task_line(task)];
    lines.push(if task.workflow.is_empty() { "No workflow on this task.".to_string() } else { format!("Workflow: {}", task.workflow) });
    lines.push(match run {
        Some(run) => run_line(run),
        None => "No run yet.".to_string(),
    });
    lines
}

fn set_workflow(conn: &Connection, task: &Task, run: Option<&WorkflowRun>, name: &str, who: &Caller) -> Result<Task> {
    if who.terminal().is_some() {
        task_rules::check_open_in(task)?;
    }
    if is_active(run) {
        bail!("Task #{} has an active workflow run. Cancel it first. Nothing was changed.", task.id);
    }
    if !name.is_empty() {
        if !workflows::valid_name(name) {
            bail!("Workflow name: small letters, numbers and dashes, up to 60. Nothing was changed.");
        }
        if workflows::get_workflow(conn, name)?.is_none() {
            let names: Vec<String> = workflows::list_workflows(conn)?.into_iter().map(|w| w.name).collect();
            let known = if names.is_empty() { "none".to_string() } else { names.join(", ") };
            bail!("There is no workflow named {name}. Saved workflows: {known}. Nothing was changed.");
        }
    }
    let tx = conn.unchecked_transaction()?;
    let changed = tasks::update_task(&tx, task.id, &TaskPatch { workflow: Some(name.to_string()), ..TaskPatch::default() })?;
    let note = if name.is_empty() { "Workflow cleared.".to_string() } else { format!("Workflow set to {name}.") };
    tasks::add_comment(&tx, task.id, &CommentInput::new(CommentKind::Note, &note), &caller::actor_of(who))?;
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_of(state: &str, run: Value) -> WorkflowRun {
        WorkflowRun {
            task_id: 1,
            workflow: "build-review".into(),
            space_id: String::new(),
            state: state.into(),
            flow: String::new(),
            run: run.to_string(),
            started_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn run_lines_read_the_saved_state() {
        let run = run_of("waiting", json!({ "node": "b2", "steps": 3, "ask": { "why": "a gate needs you" } }));
        assert_eq!(run_line(&run), "Run of build-review: waiting, at box b2, 3 steps. It waits: a gate needs you.");
        assert!(is_active(Some(&run)));
        let broken = WorkflowRun { run: "not json".into(), ..run_of("done", Value::Null) };
        assert_eq!(run_line(&broken), "Run of build-review: done, 0 steps.");
        assert!(!is_active(Some(&broken)));
        assert!(!is_active(None));
    }
}
