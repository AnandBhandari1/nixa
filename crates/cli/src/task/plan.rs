use super::caller;
use super::{open, print, task_id};
use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::Result;
use nixa_store::task_plans;
use nixa_store::tasks::{self, CommentInput, CommentKind};
use serde_json::{Map, Value, json};
use std::io::Write;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--step", "--done"], false)?;
    let id = task_id(&p, "plan")?;
    let steps = p.values(&["--step"]);
    let done = done_numbers(&p.values(&["--done"]))?;
    let start = p.flag(&["--run"]);
    let clear = p.flag(&["--clear"]);
    p.finish(1, false)?;
    let conn = open()?;
    tasks::must_task(&conn, id)?;
    if steps.is_empty() && done.is_empty() && !start && !clear {
        let plan = task_plans::get_plan(&conn, id)?;
        let text = plan.as_ref().map(task_plans::plan_text).unwrap_or_else(|| format!("Task #{id} has no step plan."));
        return print(&ctx, out, &plan, &text);
    }
    if steps.len() > task_plans::MAX_STEPS {
        return Err(usage(format!("a step plan has at most {} steps", task_plans::MAX_STEPS)));
    }
    let who = caller::who(&ctx)?;
    if clear {
        if !steps.is_empty() || !done.is_empty() || start {
            return Err(usage("--clear takes no --step, --done or --run"));
        }
        let gone = task_plans::remove_plan(&conn, id)?;
        if gone {
            tasks::add_comment(&conn, id, &CommentInput::new(CommentKind::Note, "Step plan removed."), &caller::actor_of(&who))?;
        }
        let line = if gone { format!("Removed the step plan of task #{id}.") } else { format!("Task #{id} had no step plan.") };
        return print(&ctx, out, &json!({ "task": id, "removed": gone }), &line);
    }
    drop(conn);
    let body = plan_body(&steps, &done, start, who.terminal());
    let path = format!("/tasks/{id}/plan");
    let reply = match who.terminal() {
        Some(_) => http::post(&path, &body)?,
        None => http::window_call("POST", &path, Some(&body))?,
    };
    let text = reply.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).map(str::to_string).unwrap_or_else(|| format!("Task #{id}: plan saved."));
    print(&ctx, out, &reply, &text)
}

pub fn plan_body(steps: &[String], done: &[usize], start: bool, terminal: Option<&str>) -> Value {
    let mut body = Map::new();
    if !steps.is_empty() {
        body.insert("steps".into(), json!(steps));
    }
    if !done.is_empty() {
        body.insert("done".into(), json!(done));
    }
    if start {
        body.insert("run".into(), json!(true));
    }
    if let Some(terminal) = terminal {
        body.insert("terminal".into(), json!(terminal));
    }
    Value::Object(body)
}

pub fn done_numbers(words: &[String]) -> Result<Vec<usize>> {
    let mut numbers = Vec::new();
    for word in words.iter().flat_map(|w| w.split(',')).map(str::trim).filter(|w| !w.is_empty()) {
        let n = args::number::<usize>(word, "--done")?;
        if n == 0 {
            return Err(usage("--done counts steps from 1"));
        }
        if !numbers.contains(&n) {
            numbers.push(n);
        }
    }
    Ok(numbers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn done_lists_are_read() {
        assert_eq!(done_numbers(&strs(&["1,3", "3", " 2 "])).unwrap(), vec![1, 3, 2]);
        assert!(done_numbers(&strs(&["0"])).is_err());
        assert!(done_numbers(&strs(&["two"])).is_err());
        assert!(done_numbers(&[]).unwrap().is_empty());
    }

    #[test]
    fn plan_bodies_carry_only_what_was_given() {
        let body = plan_body(&strs(&["user | Read the brief", "agent:codex gpt-5 | Build it"]), &[1], true, None);
        assert_eq!(body, json!({ "steps": ["user | Read the brief", "agent:codex gpt-5 | Build it"], "done": [1], "run": true }));
        assert_eq!(plan_body(&[], &[2], false, Some("m4")), json!({ "done": [2], "terminal": "m4" }));
        assert_eq!(plan_body(&[], &[], false, None), json!({}));
    }
}
