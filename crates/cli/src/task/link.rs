use super::caller;
use super::{open, print, route_locked, task_id, task_line};
use crate::args::usage;
use crate::ctx::{self, Ctx};
use anyhow::Result;
use nixa_store::task_rules;
use nixa_store::tasks::{self, Task, TaskPatch};
use std::io::Write;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--repo", "--branch", "--commit", "--pr"], false)?;
    let id = task_id(&p, "link")?;
    let patch = TaskPatch {
        repo: p.value(&["--repo"]),
        branch: p.value(&["--branch"]),
        commit_sha: p.value(&["--commit"]),
        pr_url: p.value(&["--pr"]),
        ..TaskPatch::default()
    };
    p.finish(1, false)?;
    if patch == TaskPatch::default() {
        return Err(usage("nixa task link needs --repo, --branch, --commit or --pr"));
    }
    let who = caller::who(&ctx)?;
    let conn = open()?;
    let before = tasks::must_task(&conn, id)?;
    if let Some(terminal) = who.terminal() {
        task_rules::check_held_in(&before, terminal)?;
        route_locked(&conn, &before)?;
    }
    let task = tasks::update_task(&conn, id, &patch)?;
    print(&ctx, out, &task, &format!("Linked {}.", link_line(&task)))
}

pub fn link_line(task: &Task) -> String {
    let parts: Vec<String> = [("repo", &task.repo), ("branch", &task.branch), ("commit", &task.commit_sha), ("pr", &task.pr_url)]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(label, value)| format!("{label} {value}"))
        .collect();
    if parts.is_empty() { task_line(task) } else { format!("{}, {}", task_line(task), parts.join(", ")) }
}
