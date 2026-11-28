use super::api::Role;
use super::{asked, changed, db, locked_text, log, note, start_now, task_locked};
use crate::app::App;
use crate::extras::model_pick::{PICK_KEY, Pick};
use crate::tools::{Args, flag, schema, spec};
use crate::turns::ToolSpec;
use nixa_store::Connection;
use nixa_store::task_plans::{self as plans, Owner, Plan, StepInput, StepState};
use nixa_store::tasks::{self, CommentKind, Task, TaskStatus};
use nixa_store::text::clip_dots;
use nixa_store::time::now_ms;
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub const TOOL: &str = "nixa_task_plan";
pub const PICK_ASK: &str = "step plan";
pub const STALL_MS: i64 = 30 * 60 * 1000;
pub const NUDGE_KEY: &str = "plan_nudge:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Begun {
    Nothing,
    Handled,
    Start,
}

pub fn spec_of() -> ToolSpec {
    spec(
        TOOL,
        "The user's own step plan for a task. When the user gives steps, or names who does each part, add the task first, then save the steps here in the user's order with the owners the user named, and give run true. Owner of a step: user for a step the user does; nixa for a step you do yourself with your own tools; agent:<agent> [model] for an agent type (for example agent:claude or agent:codex gpt-5); custom:<id> for a saved custom agent. output is the full path of the one file a step writes, if any. done lists step numbers that are finished: with steps they are marked done in the new plan, without steps they are marked done in the saved plan. Nixa's code starts each step when the one before it is done, and the owner the user named wins over the model pick. Never start a terminal or send work yourself for a planned task. Without steps and done it shows the plan and where it is.",
        schema(
            json!({
                "id": {"type": "integer", "description": "The task number."},
                "steps": {"type": "array", "maxItems": plans::MAX_STEPS, "items": {"type": "object", "properties": {
                    "owner": {"type": "string", "description": "user, nixa, agent:<agent> [model] or custom:<id>"},
                    "work": {"type": "string", "description": "What this step does."},
                    "output": {"type": "string", "description": "Full path of the file this step writes, if any."}
                }, "required": ["owner", "work"], "additionalProperties": false}},
                "done": {"type": "array", "items": {"type": "integer", "minimum": 1}},
                "run": {"type": "boolean", "description": "Start the plan now: the first step that is not done."}
            }),
            &["id"],
        ),
    )
}

fn fail(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn id_of(args: &Args) -> Result<i64, String> {
    args.get("id")
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().trim_start_matches('#').parse().ok())))
        .filter(|n| *n > 0)
        .ok_or_else(|| "id is needed: a task number.".to_string())
}

fn field(map: &Map<String, Value>, key: &str) -> String {
    map.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn steps_of(args: &Args) -> Vec<StepInput> {
    match args.get("steps") {
        Some(Value::Array(list)) => list
            .iter()
            .map(|item| match item {
                Value::String(line) => plans::parse_step(line),
                Value::Object(map) => StepInput { owner: field(map, "owner"), work: field(map, "work"), output: field(map, "output") },
                _ => StepInput::default(),
            })
            .collect(),
        Some(Value::String(line)) if !line.trim().is_empty() => vec![plans::parse_step(line)],
        _ => Vec::new(),
    }
}

pub fn done_of(args: &Args) -> Vec<usize> {
    let words: Vec<String> = match args.get("done") {
        Some(Value::Array(list)) => list.iter().map(|v| v.as_u64().map(|n| n.to_string()).unwrap_or_else(|| v.as_str().unwrap_or_default().to_string())).collect(),
        Some(Value::Number(n)) => vec![n.to_string()],
        Some(Value::String(s)) => s.split([',', ' ']).map(str::to_string).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<usize> = words.iter().filter_map(|w| w.trim().parse::<usize>().ok()).filter(|n| *n > 0).collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn by_of(app: &App, role: &Role) -> String {
    match role {
        Role::User => "the user".into(),
        Role::Brain => app.name(),
        Role::Terminal(terminal) => format!("the agent in terminal {terminal}"),
    }
}

fn steps_body(plan: &Plan) -> String {
    plan.steps.iter().map(plans::step_line).collect::<Vec<_>>().join("\n")
}

fn output_line(step: &plans::PlanStep) -> String {
    if step.output.is_empty() { String::new() } else { format!(" Write {}.", step.output) }
}

fn tell(app: &Arc<App>, text: &str) {
    crate::watch::tell_event(app, text);
}

fn save(conn: &Connection, plan: &Plan) {
    if let Err(error) = plans::save_plan(conn, plan) {
        log("plan save failed", json!({"task": plan.task, "error": error.to_string()}));
    }
}

fn set_doing(conn: &Connection, plan: &mut Plan, index: usize) {
    let step = &mut plan.steps[index];
    step.state = StepState::Doing;
    step.started_at = now_ms();
    step.told = false;
    step.result.clear();
    let n = step.n;
    save(conn, plan);
    let _ = tasks::set_task_meta(conn, &nudge_key(plan.task, n), "");
}

fn nudge_key(task: i64, n: usize) -> String {
    format!("{NUDGE_KEY}{task}:{n}")
}

pub fn nudge_due(conn: &Connection, task: i64, n: usize, now: i64) -> bool {
    let key = nudge_key(task, n);
    let last = tasks::task_meta(conn, &key).ok().flatten().and_then(|v| v.trim().parse::<i64>().ok());
    if last.is_some_and(|at| now.saturating_sub(at) < STALL_MS) {
        return false;
    }
    let _ = tasks::set_task_meta(conn, &key, &now.to_string());
    last.is_some()
}

pub fn stalled_event(task: i64, n: usize, name: &str, minutes: i64) -> String {
    format!("Event: Task #{task} step {n} ({name}) has not ended for {minutes} more minutes. Do not wait for the user: do the step now with your own tools and call {TOOL} with id {task} and done [{n}], give it to another agent with {TOOL} (new steps), or ask the deep brain with nixa_deep_ask. Record what you chose in one short line.")
}

fn stall_step(app: &Arc<App>, plan: &Plan, index: usize) -> bool {
    let Ok(conn) = db(app) else { return false };
    let Some(task) = tasks::get_task(&conn, plan.task).ok().flatten() else { return false };
    if task.archived_at > 0 {
        return false;
    }
    let step = &plan.steps[index];
    if !nudge_due(&conn, plan.task, step.n, now_ms()) {
        return false;
    }
    let name = plans::step_name(step);
    if step.state == StepState::Blocked {
        crate::modes::note(app, "plan", &format!("task #{} step {}: Nixa runs the blocked step again", plan.task, step.n), None, Some(&step.result));
        log("plan blocked retry", json!({"task": plan.task, "step": step.n}));
        return reopen_blocked(&conn, plan.task).is_ok();
    }
    let minutes = STALL_MS / 60_000;
    crate::modes::note(app, "plan", &format!("task #{} step {}: Nixa did not wait, it asked again after {minutes} minutes", plan.task, step.n), None, Some(&step.work));
    tell(app, &stalled_event(plan.task, step.n, &name, minutes));
    log("plan step stalled", json!({"task": plan.task, "step": step.n}));
    false
}

fn block(app: &Arc<App>, conn: &Connection, plan: &mut Plan, index: usize, why: &str) {
    let step = &mut plan.steps[index];
    step.state = StepState::Blocked;
    step.result = clip_dots(why, plans::RESULT_MAX);
    let (n, result, name) = (step.n, step.result.clone(), plans::step_name(step));
    save(conn, plan);
    note(conn, plan.task, &format!("Step {n} is blocked: {result}"), CommentKind::Note);
    crate::modes::note(app, "plan", &format!("task #{} step {n}: Nixa decides how to unblock it", plan.task), None, Some(&result));
    tell(app, &blocked_event(plan.task, n, &name, &result));
    log("plan blocked", json!({"task": plan.task, "step": n}));
}

pub fn blocked_event(task: i64, n: usize, name: &str, result: &str) -> String {
    format!("Event: Task #{task} step {n} ({name}) is blocked: {result} Do not wait for the user. Decide how to unblock it yourself, or ask the deep brain with nixa_deep_ask: fix the cause, give the step to another agent with {TOOL} (new steps), or mark it done when it is not needed. Then run the plan again with {TOOL} id {task} run true. Tell the user in one short line what you picked.")
}

pub fn nixa_step_event(id: i64, n: usize, total: usize, work: &str, before: &str) -> String {
    let head = format!("Event: Task #{id} step {n} of {total} is yours to do: {work} {before} Do it with your own tools. When it is done, call {TOOL} with id {id} and done [{n}].");
    format!("{head} If you cannot do it, do not wait for the user: ask the deep brain with nixa_deep_ask, or give the step to an agent with {TOOL} (new steps). Then go on with the plan.")
}

pub fn user_step_event(id: i64, n: usize, total: usize, work: &str, before: &str) -> String {
    format!("Event: Task #{id} step {n} of {total} was planned for the user: {work} {before} Do not wait for the user: do it yourself with your own tools, or give it to an agent. When it is done, call {TOOL} with id {id} and done [{n}]. When it physically needs the user (a sign-in, a key, a test on the user's own device), tell the user in one short line as a note, find another way (a saved key, a mock, or skip that step) and go on with the other steps.")
}

fn finished(app: &Arc<App>, conn: &Connection, plan: &mut Plan) {
    if plan.finished {
        return;
    }
    plan.finished = true;
    save(conn, plan);
    note(conn, plan.task, "Every plan step is done.", CommentKind::Note);
    let steps: Vec<String> = plan
        .steps
        .iter()
        .map(|s| if s.output.is_empty() { format!("Step {} ({}) is done.", s.n, plans::step_name(s)) } else { format!("Step {} ({}) wrote {}.", s.n, plans::step_name(s), s.output) })
        .collect();
    tell(app, &format!("Event: Task #{}: every plan step is done. {} Tell the user in one or two plain sentences.", plan.task, steps.join(" ")));
    log("plan finished", json!({"task": plan.task}));
}

pub fn model_of(agent: &str, model: &str) -> String {
    match model.trim() {
        "" => String::new(),
        m if m.contains("::") => m.to_string(),
        m => format!("{agent}::{m}"),
    }
}

fn assign(conn: &Connection, task: &Task, kind: &str, who: &str, model: &str) -> Result<(), String> {
    if !task.terminal_id.is_empty() && matches!(task.status, TaskStatus::Doing | TaskStatus::Blocked) {
        return Err(format!("Task #{} is still held by terminal {}. Release it first, then run the plan again.", task.id, task.terminal_id));
    }
    if task.status != TaskStatus::Todo {
        tasks::set_status(conn, task.id, TaskStatus::Todo, None, false).map_err(fail)?;
    }
    tasks::assign_task(conn, task.id, kind, who).map_err(fail)?;
    let model = if kind == crate::tasks::KIND_TERMINAL { model_of(who, model) } else { String::new() };
    let label = if model.is_empty() { who.to_string() } else { model.clone() };
    let pick = Pick {
        assigned_kind: kind.to_string(),
        assigned: who.to_string(),
        model,
        label,
        ask: PICK_ASK.into(),
        why: "the user named this owner in the task's step plan".into(),
        ..Pick::default()
    };
    let raw = serde_json::to_string(&pick).map_err(|e| e.to_string())?;
    tasks::set_task_meta(conn, &format!("{PICK_KEY}{}", task.id), &raw).map_err(fail)?;
    Ok(())
}

fn begin(app: &Arc<App>, id: i64) -> Begun {
    let Ok(conn) = db(app) else { return Begun::Nothing };
    let Ok(Some(mut plan)) = plans::get_plan(&conn, id) else { return Begun::Nothing };
    let Ok(Some(task)) = tasks::get_task(&conn, id) else { return Begun::Nothing };
    let Some(index) = plans::current(&plan) else {
        finished(app, &conn, &mut plan);
        return Begun::Handled;
    };
    let step = plan.steps[index].clone();
    if step.state == StepState::Blocked {
        return Begun::Handled;
    }
    if task.status == TaskStatus::Cancelled {
        block(app, &conn, &mut plan, index, "the task was cancelled");
        return Begun::Handled;
    }
    let owner = match plans::owner_of(&step.owner) {
        Ok(owner) => owner,
        Err(error) => {
            block(app, &conn, &mut plan, index, &error.to_string());
            return Begun::Handled;
        }
    };
    if step.state == StepState::Doing {
        let waits = matches!(owner, Owner::Agent { .. } | Owner::Custom(_))
            && task.terminal_id.is_empty()
            && matches!(task.status, TaskStatus::Todo | TaskStatus::Inbox);
        return if waits { Begun::Start } else { Begun::Handled };
    }
    let total = plan.steps.len();
    let before = plans::earlier(&plan, index).join(" ");
    let n = step.n;
    match &owner {
        Owner::Nixa => {
            set_doing(&conn, &mut plan, index);
            note(&conn, id, &format!("Step {n} is for {}: {}", app.name(), step.work), CommentKind::Note);
            let work = format!("{}{}", step.work, output_line(&step));
            tell(app, &nixa_step_event(id, n, total, &work, &before));
            log("plan step", json!({"task": id, "step": n, "owner": "nixa"}));
            Begun::Handled
        }
        Owner::User => {
            set_doing(&conn, &mut plan, index);
            note(&conn, id, &format!("Step {n} was for the user. {} does it: {}", app.name(), step.work), CommentKind::Note);
            crate::modes::note(app, "plan", &format!("task #{id} step {n}: Nixa takes the user's step"), None, Some(&step.work));
            tell(app, &user_step_event(id, n, total, &format!("{}{}", step.work, output_line(&step)), &before));
            log("plan step", json!({"task": id, "step": n, "owner": "user", "by": "nixa"}));
            changed(app, "plan", Some(id));
            Begun::Handled
        }
        Owner::Agent { agent, model } => {
            if let Err(error) = assign(&conn, &task, crate::tasks::KIND_TERMINAL, agent, model) {
                block(app, &conn, &mut plan, index, &error);
                return Begun::Handled;
            }
            set_doing(&conn, &mut plan, index);
            note(&conn, id, &format!("Step {n} goes to {}, as the plan names it. No model pick.", plans::owner_name(&owner)), CommentKind::Note);
            log("plan step", json!({"task": id, "step": n, "owner": step.owner}));
            Begun::Start
        }
        Owner::Custom(agent) => {
            match nixa_store::agents::get_agent(&conn, agent) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    block(app, &conn, &mut plan, index, &format!("there is no custom agent {agent}"));
                    return Begun::Handled;
                }
                Err(error) => {
                    block(app, &conn, &mut plan, index, &error.to_string());
                    return Begun::Handled;
                }
            }
            if let Err(error) = assign(&conn, &task, crate::tasks::KIND_CUSTOM, agent, "") {
                block(app, &conn, &mut plan, index, &error);
                return Begun::Handled;
            }
            set_doing(&conn, &mut plan, index);
            note(&conn, id, &format!("Step {n} goes to {}, as the plan names it.", plans::owner_name(&owner)), CommentKind::Note);
            log("plan step", json!({"task": id, "step": n, "owner": step.owner}));
            Begun::Start
        }
    }
}

pub async fn advance(app: &Arc<App>, id: i64, by_user: bool) {
    if !by_user
        && let Ok(conn) = db(app)
        && let Err(why) = super::waits::refuse_and_hold(&conn, id, super::waits::PLAN)
    {
        log("plan step waits", json!({"task": id, "why": why}));
        return;
    }
    if begin(app, id) != Begun::Start {
        return;
    }
    let Err(error) = start_now(app, id, by_user).await else { return };
    if !by_user && db(app).is_ok_and(|conn| super::waits::waits_on_open(&conn, id)) {
        if let Ok(conn) = db(app) {
            super::waits::hold(&conn, id, super::waits::PLAN);
        }
        log("plan step waits", json!({"task": id, "why": error}));
        return;
    }
    if let Ok(conn) = db(app)
        && let Ok(Some(mut plan)) = plans::get_plan(&conn, id)
        && let Some(index) = plans::current(&plan)
    {
        block(app, &conn, &mut plan, index, &format!("the start failed: {error}"));
    }
}

fn step_done(conn: &Connection, id: i64, index: usize) -> bool {
    let Ok(Some(mut plan)) = plans::get_plan(conn, id) else { return false };
    let Some(step) = plan.steps.get_mut(index).filter(|s| s.state == StepState::Doing) else { return false };
    step.state = StepState::Done;
    step.done_at = now_ms();
    let (n, name) = (step.n, plans::step_name(step));
    save(conn, &plan);
    note(conn, id, &format!("Step {n} is done by {name}."), CommentKind::Note);
    log("plan step done", json!({"task": id, "step": n}));
    true
}

async fn tell_agent(app: &Arc<App>, plan: &Plan, index: usize, terminal: &str) {
    if asked(app, terminal) {
        return;
    }
    let step = &plan.steps[index];
    let text = format!(
        "This task has a plan. You do step {} of {}: {}{} {} Do only this step. When it is done and tested, set the task to review as usual.",
        step.n,
        plan.steps.len(),
        step.work,
        output_line(step),
        plans::earlier(plan, index).join(" ")
    );
    let Ok(id) = crate::engine::term_id(terminal) else { return };
    if crate::engine::send(&id, &text, true, false).await.is_err() {
        return;
    }
    if let Ok(conn) = db(app)
        && let Ok(Some(mut fresh)) = plans::get_plan(&conn, plan.task)
        && let Some(saved) = fresh.steps.get_mut(index)
    {
        saved.told = true;
        save(&conn, &fresh);
    }
}

pub async fn tick(app: &Arc<App>) {
    let list = db(app).ok().and_then(|conn| plans::list_plans(&conn).ok()).unwrap_or_default();
    for plan in list {
        let Some(index) = plans::current(&plan) else {
            if !plan.finished
                && let Ok(conn) = db(app)
            {
                let mut plan = plan;
                finished(app, &conn, &mut plan);
            }
            continue;
        };
        let step = &plan.steps[index];
        if step.state == StepState::Blocked {
            if stall_step(app, &plan, index) {
                advance(app, plan.task, false).await;
            }
            continue;
        }
        if step.state != StepState::Doing {
            continue;
        }
        let Ok(owner) = plans::owner_of(&step.owner) else { continue };
        if owner == Owner::Nixa {
            stall_step(app, &plan, index);
            continue;
        }
        let Some(task) = db(app).ok().and_then(|conn| tasks::get_task(&conn, plan.task).ok().flatten()) else { continue };
        if task.archived_at > 0 {
            continue;
        }
        match task.status {
            TaskStatus::Done => {
                let moved = db(app).map(|conn| step_done(&conn, plan.task, index)).unwrap_or(false);
                if moved {
                    advance(app, plan.task, false).await;
                }
                continue;
            }
            TaskStatus::Cancelled => {
                if let Ok(conn) = db(app) {
                    let mut plan = plan.clone();
                    block(app, &conn, &mut plan, index, "the task was cancelled");
                }
                continue;
            }
            _ => {}
        }
        if owner == Owner::User {
            stall_step(app, &plan, index);
            continue;
        }
        if task.terminal_id.is_empty() || step.told {
            continue;
        }
        tell_agent(app, &plan, index, &task.terminal_id).await;
    }
}

pub fn soon(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move { tick(&app).await });
}

pub fn auto_ok(conn: &Connection, id: i64) -> bool {
    let Ok(Some(plan)) = plans::get_plan(conn, id) else { return true };
    let Some(index) = plans::current(&plan) else { return true };
    let step = &plan.steps[index];
    step.state == StepState::Doing && matches!(plans::owner_of(&step.owner), Ok(Owner::Agent { .. } | Owner::Custom(_)))
}

fn mark(conn: &Connection, id: i64, done: &[usize]) -> Result<bool, String> {
    let mut plan = plans::get_plan(conn, id)
        .map_err(fail)?
        .ok_or_else(|| format!("Task #{id} has no plan. Give steps to set one."))?;
    let marked = plans::mark_done(&mut plan, done, now_ms());
    if marked.is_empty() {
        return Ok(false);
    }
    plans::save_plan(conn, &plan).map_err(fail)?;
    let list: Vec<String> = marked.iter().map(|n| n.to_string()).collect();
    let s = if marked.len() == 1 { "" } else { "s" };
    note(conn, id, &format!("Plan step{s} {} marked done.", list.join(", ")), CommentKind::Note);
    Ok(true)
}

fn reopen_blocked(conn: &Connection, id: i64) -> Result<(), String> {
    let Some(mut plan) = plans::get_plan(conn, id).map_err(fail)? else {
        return Err(format!("Task #{id} has no plan. Give steps to set one."));
    };
    let Some(index) = plans::current(&plan) else { return Ok(()) };
    let step = &mut plan.steps[index];
    if step.state != StepState::Blocked {
        return Ok(());
    }
    step.state = StepState::Todo;
    step.result.clear();
    plan.finished = false;
    plans::save_plan(conn, &plan).map_err(fail)
}

pub async fn act(app: &Arc<App>, args: &Args, role: &Role) -> Result<Value, String> {
    let id = id_of(args)?;
    let steps = steps_of(args);
    let done = done_of(args);
    let run = flag(args, "run") == Some(true);
    let conn = db(app)?;
    let task = tasks::get_task(&conn, id).map_err(fail)?.ok_or_else(|| format!("No task #{id}."))?;
    if *role != Role::User {
        let rows = nixa_store::spaces::list_spaces(&conn).unwrap_or_default();
        if let Some(locked) = task_locked(&task, &rows) {
            return Err(locked_text(&locked.name));
        }
    }
    let mut moved = false;
    if !steps.is_empty() {
        let plan = plans::set_plan(&conn, id, &steps, &by_of(app, role), &done).map_err(fail)?;
        note(
            &conn,
            id,
            &format!("Plan set by {}. Nixa runs the steps in this order and starts each one when the one before it is done.\n{}", plan.by, steps_body(&plan)),
            CommentKind::Note,
        );
        log("plan set", json!({"task": id, "steps": plan.steps.len()}));
    } else if !done.is_empty() {
        moved = mark(&conn, id, &done)?;
    }
    if run {
        reopen_blocked(&conn, id)?;
    }
    drop(conn);
    if run || moved {
        advance(app, id, *role == Role::User).await;
    }
    let conn = db(app)?;
    let plan = plans::get_plan(&conn, id).map_err(fail)?;
    let task = tasks::get_task(&conn, id).map_err(fail)?;
    drop(conn);
    let text = plan.as_ref().map(plans::plan_text).unwrap_or_else(|| format!("Task #{id} has no plan."));
    if !steps.is_empty() || moved || run {
        changed(app, "plan", Some(id));
    }
    Ok(json!({"task": task, "plan": plan, "text": text}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Args {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn steps_come_as_lines_or_objects() {
        let list = steps_of(&args(json!({"steps": ["agent:claude | Build it | /tmp/x.md", {"owner": "user", "work": "Check it"}, 7]})));
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].output, "/tmp/x.md");
        assert_eq!(list[1].owner, "user");
        assert_eq!(list[2], StepInput::default());
        assert_eq!(steps_of(&args(json!({"steps": "nixa | Write the summary"})))[0].owner, "nixa");
        assert!(steps_of(&args(json!({}))).is_empty());
    }

    #[test]
    fn done_numbers_are_clean() {
        assert_eq!(done_of(&args(json!({"done": [2, 1, 2, 0]}))), vec![1, 2]);
        assert_eq!(done_of(&args(json!({"done": "1, 3"}))), vec![1, 3]);
        assert_eq!(done_of(&args(json!({"done": 4}))), vec![4]);
        assert!(done_of(&args(json!({}))).is_empty());
    }

    #[test]
    fn automatic_plan_events_do_not_wait_for_the_user() {
        let auto = blocked_event(4, 2, "Build", "no agent");
        assert!(auto.to_lowercase().contains("do not wait for the user") && auto.contains("run true"), "{auto}");
        assert!(user_step_event(4, 2, 3, "Check it", "").contains("do it yourself"));
        let mine = nixa_step_event(4, 2, 3, "Write it", "");
        assert!(mine.contains("nixa_deep_ask") && !mine.contains("tell the user why"));
        let stalled = stalled_event(4, 2, "Build", 30);
        assert!(stalled.to_lowercase().contains("do not wait for the user") && stalled.contains("nixa_deep_ask"), "{stalled}");
    }

    #[test]
    fn a_stalled_step_is_raised_once_per_time_limit() {
        let conn = nixa_store::open_memory().unwrap();
        assert!(!nudge_due(&conn, 4, 2, 1_000), "the first look only starts the clock");
        assert!(!nudge_due(&conn, 4, 2, 1_000 + STALL_MS - 1));
        assert!(nudge_due(&conn, 4, 2, 1_000 + STALL_MS));
        assert!(!nudge_due(&conn, 4, 2, 1_000 + STALL_MS + 1));
    }

    #[test]
    fn models_carry_their_agent() {
        assert_eq!(model_of("codex", "gpt-5"), "codex::gpt-5");
        assert_eq!(model_of("codex", "codex::gpt-5"), "codex::gpt-5");
        assert_eq!(model_of("claude", " "), "");
    }

    #[tokio::test]
    async fn a_plan_is_set_shown_and_its_steps_marked() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let id = {
            let conn = db(&app).unwrap();
            let input = tasks::TaskInput { title: "Write the guide".into(), status: Some(TaskStatus::Todo), ..tasks::TaskInput::default() };
            tasks::add_task(&conn, &input, &super::super::nixa()).unwrap().id
        };
        let set = act(&app, &args(json!({"id": id, "steps": ["nixa | Read the code", "user | Look at it"]})), &Role::User).await.unwrap();
        assert!(set["text"].as_str().unwrap().contains("1) [todo] Nixa: Read the code"), "{set}");
        let shown = act(&app, &args(json!({"id": id})), &Role::Brain).await.unwrap();
        assert_eq!(shown["plan"]["steps"].as_array().unwrap().len(), 2);
        let marked = act(&app, &args(json!({"id": id, "done": [1]})), &Role::User).await.unwrap();
        assert_eq!(marked["plan"]["steps"][0]["state"], "done");
        let handed = act(&app, &args(json!({"id": id, "done": [2]})), &Role::Terminal("m3".into())).await.unwrap();
        assert_eq!(handed["plan"]["steps"][1]["state"], "done", "in Decision automatic an agent may close the user's step");
        assert!(act(&app, &args(json!({"id": id + 50})), &Role::User).await.is_err());
        assert!(act(&app, &args(json!({"id": id, "steps": ["somebody | x"]})), &Role::User).await.is_err());
    }
}
