use crate::tasks;
use crate::text::{clip, clip_dots, one_line};
use anyhow::{Result, bail};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const MAX_STEPS: usize = 8;
pub const META_PREFIX: &str = "plansteps:";
pub const WORK_MAX: usize = 1200;
pub const OUTPUT_MAX: usize = 500;
pub const RESULT_MAX: usize = 1500;
pub const BY_MAX: usize = 80;
pub const OWNER_WORDS: &str = "user, nixa, agent:<agent> [model], or custom:<custom agent id>";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    User,
    Nixa,
    Agent { agent: String, model: String },
    Custom(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Todo,
    Doing,
    Done,
    Blocked,
}

impl StepState {
    pub fn as_str(self) -> &'static str {
        match self {
            StepState::Todo => "todo",
            StepState::Doing => "doing",
            StepState::Done => "done",
            StepState::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub n: usize,
    pub owner: String,
    pub work: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    pub state: StepState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result: String,
    #[serde(default)]
    pub told: bool,
    #[serde(default)]
    pub started_at: i64,
    #[serde(default)]
    pub done_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub task: i64,
    pub steps: Vec<PlanStep>,
    pub at: i64,
    pub by: String,
    #[serde(default)]
    pub finished: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepInput {
    pub owner: String,
    pub work: String,
    pub output: String,
}

fn after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

fn plain_word(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn owner_of(raw: &str) -> Result<Owner> {
    let text = raw.trim();
    let lower = text.to_lowercase();
    if lower == "user" || lower == "me" {
        return Ok(Owner::User);
    }
    if lower == "nixa" {
        return Ok(Owner::Nixa);
    }
    if let Some(rest) = after(text, "agent:") {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let (first, model) = match words.as_slice() {
            [one] => one.split_once("::").map(|(a, m)| (a, m.to_string())).unwrap_or((*one, String::new())),
            [one, two] => (*one, two.to_string()),
            _ => ("", String::new()),
        };
        let agent = first.to_lowercase();
        if !plain_word(&agent) || (words.len() == 1 && rest.contains("::") && model.is_empty()) {
            bail!("Step owner \"{text}\" is not right. Write agent:<agent> [model], for example agent:claude or agent:codex gpt-5.");
        }
        return Ok(Owner::Agent { agent, model });
    }
    if let Some(rest) = after(text, "custom:").or_else(|| after(text, "member:")) {
        let id = rest.trim();
        if id.is_empty() || id.contains(char::is_whitespace) {
            bail!("Step owner \"{text}\" needs a custom agent id, for example custom:a3.");
        }
        return Ok(Owner::Custom(id.to_string()));
    }
    bail!("Unknown step owner \"{text}\". Use {OWNER_WORDS}.")
}

pub fn owner_name(owner: &Owner) -> String {
    match owner {
        Owner::User => "the user".into(),
        Owner::Nixa => "Nixa".into(),
        Owner::Agent { agent, model } if model.is_empty() => agent.clone(),
        Owner::Agent { agent, model } => format!("{agent} on {model}"),
        Owner::Custom(id) => format!("custom agent {id}"),
    }
}

pub fn step_name(step: &PlanStep) -> String {
    owner_of(&step.owner).map(|o| owner_name(&o)).unwrap_or_else(|_| step.owner.clone())
}

pub fn parse_step(text: &str) -> StepInput {
    let mut parts = text.split('|').map(str::trim);
    StepInput {
        owner: parts.next().unwrap_or_default().to_string(),
        work: parts.next().unwrap_or_default().to_string(),
        output: parts.next().unwrap_or_default().to_string(),
    }
}

pub fn build_plan(task: i64, inputs: &[StepInput], by: &str, done: &[usize], now: i64) -> Result<Plan> {
    if inputs.is_empty() {
        bail!("A plan needs at least one step.");
    }
    if inputs.len() > MAX_STEPS {
        bail!("A plan has at most {MAX_STEPS} steps.");
    }
    let mut steps = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        let n = index + 1;
        let work = clip_dots(&one_line(&input.work), WORK_MAX);
        if work.is_empty() {
            bail!("Step {n} needs its work, for example: agent:claude | Write the parser and its tests | /full/path/docs/parser.md");
        }
        if let Err(error) = owner_of(&input.owner) {
            bail!("Step {n}: {error}");
        }
        let finished = done.contains(&n);
        steps.push(PlanStep {
            n,
            owner: one_line(&input.owner),
            work,
            output: clip(&one_line(&input.output), OUTPUT_MAX),
            state: if finished { StepState::Done } else { StepState::Todo },
            result: String::new(),
            told: false,
            started_at: 0,
            done_at: if finished { now } else { 0 },
        });
    }
    Ok(Plan { task, steps, at: now, by: clip(&one_line(by), BY_MAX), finished: false })
}

pub fn current(plan: &Plan) -> Option<usize> {
    plan.steps.iter().position(|s| s.state != StepState::Done)
}

pub fn is_open(plan: &Plan) -> bool {
    current(plan).is_some()
}

pub fn running(plan: &Plan) -> Option<&PlanStep> {
    plan.steps.iter().find(|s| s.state == StepState::Doing)
}

pub fn mark_done(plan: &mut Plan, numbers: &[usize], now: i64) -> Vec<usize> {
    let mut changed = Vec::new();
    for step in plan.steps.iter_mut().filter(|s| numbers.contains(&s.n) && s.state != StepState::Done) {
        step.state = StepState::Done;
        step.done_at = now;
        changed.push(step.n);
    }
    changed
}

pub fn earlier(plan: &Plan, index: usize) -> Vec<String> {
    plan.steps
        .iter()
        .take(index)
        .filter(|s| s.state == StepState::Done)
        .map(|s| {
            let file = if s.output.is_empty() { String::new() } else { format!(", file {}", s.output) };
            let result = if s.result.is_empty() { String::new() } else { format!(". Its result: {}", clip_dots(&s.result, 500)) };
            format!("Step {} ({}) is done{file}{result}.", s.n, step_name(s))
        })
        .collect()
}

pub fn step_line(step: &PlanStep) -> String {
    let output = if step.output.is_empty() { String::new() } else { format!(" Writes {}.", step.output) };
    let result = if step.result.is_empty() { String::new() } else { format!(" Result: {}", clip_dots(&step.result, 300)) };
    format!("{}) [{}] {}: {}{output}{result}", step.n, step.state.as_str(), step_name(step), step.work)
}

pub fn plan_text(plan: &Plan) -> String {
    let mut lines = vec![format!("Plan for task #{}, set by {}:", plan.task, plan.by)];
    lines.extend(plan.steps.iter().map(step_line));
    lines.join("\n")
}

pub fn plan_key(task: i64) -> String {
    format!("{META_PREFIX}{task}")
}

pub fn read_plan(raw: &str) -> Option<Plan> {
    serde_json::from_str(raw).ok()
}

pub fn get_plan(conn: &Connection, task: i64) -> Result<Option<Plan>> {
    Ok(tasks::task_meta(conn, &plan_key(task))?.and_then(|raw| read_plan(&raw)))
}

pub fn save_plan(conn: &Connection, plan: &Plan) -> Result<()> {
    tasks::set_task_meta(conn, &plan_key(plan.task), &serde_json::to_string(plan)?)
}

pub fn remove_plan(conn: &Connection, task: i64) -> Result<bool> {
    tasks::remove_task_meta(conn, &plan_key(task))
}

pub fn list_plans(conn: &Connection) -> Result<Vec<Plan>> {
    Ok(tasks::task_meta_list(conn, META_PREFIX)?
        .into_iter()
        .filter(|(key, _)| key.starts_with(META_PREFIX))
        .filter_map(|(_, raw)| read_plan(&raw))
        .collect())
}

pub fn has_open_plan(conn: &Connection, task: i64) -> bool {
    get_plan(conn, task).ok().flatten().is_some_and(|p| is_open(&p))
}

pub fn set_plan(conn: &Connection, task: i64, inputs: &[StepInput], by: &str, done: &[usize]) -> Result<Plan> {
    if !tasks::has_task(conn, task)? {
        bail!("No task #{task}.");
    }
    if let Some(old) = get_plan(conn, task)?
        && let Some(step) = running(&old)
    {
        bail!("Task #{task} step {} is running now. Change the plan when it ends, or mark it done first.", step.n);
    }
    let plan = build_plan(task, inputs, by, done, crate::time::now_ms())?;
    save_plan(conn, &plan)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{Actor, AuthorKind, TaskInput};

    fn input(owner: &str, work: &str) -> StepInput {
        StepInput { owner: owner.into(), work: work.into(), output: String::new() }
    }

    #[test]
    fn owners_parse() {
        assert_eq!(owner_of("user").unwrap(), Owner::User);
        assert_eq!(owner_of(" Me ").unwrap(), Owner::User);
        assert_eq!(owner_of("nixa").unwrap(), Owner::Nixa);
        assert_eq!(owner_of("agent:claude").unwrap(), Owner::Agent { agent: "claude".into(), model: String::new() });
        assert_eq!(owner_of("Agent: codex gpt-5").unwrap(), Owner::Agent { agent: "codex".into(), model: "gpt-5".into() });
        assert_eq!(owner_of("custom:a3").unwrap(), Owner::Custom("a3".into()));
        assert_eq!(owner_of("member:a7").unwrap(), Owner::Custom("a7".into()));
        assert_eq!(owner_of("agent:claude::opus").unwrap(), Owner::Agent { agent: "claude".into(), model: "opus".into() });
        assert!(owner_of("agent:claude::").is_err());
        assert!(owner_of("agent:::opus").is_err());
        assert!(owner_of("agent:").is_err());
        assert!(owner_of("agent:claude opus extra").is_err());
        assert!(owner_of("custom:").is_err());
        assert!(owner_of("somebody").is_err());
        assert_eq!(owner_name(&Owner::Agent { agent: "codex".into(), model: "gpt-5".into() }), "codex on gpt-5");
    }

    #[test]
    fn steps_parse_from_one_line() {
        let step = parse_step("agent:claude | Write the parser | /tmp/docs/parser.md");
        assert_eq!(step, StepInput { owner: "agent:claude".into(), work: "Write the parser".into(), output: "/tmp/docs/parser.md".into() });
        assert_eq!(parse_step("user").work, "");
    }

    #[test]
    fn plans_check_their_steps() {
        assert!(build_plan(1, &[], "u", &[], 5).is_err());
        let many: Vec<StepInput> = (0..MAX_STEPS + 1).map(|_| input("user", "x")).collect();
        assert!(build_plan(1, &many, "u", &[], 5).is_err());
        assert!(build_plan(1, &[input("user", "  ")], "u", &[], 5).is_err());
        let error = build_plan(1, &[input("user", "a"), input("nobody", "b")], "u", &[], 5).unwrap_err().to_string();
        assert!(error.starts_with("Step 2:"), "{error}");
        let plan = build_plan(4, &[input("agent:claude", "Build it"), input("user", "Check it")], "the user", &[1], 5).unwrap();
        assert_eq!(plan.steps[0].state, StepState::Done);
        assert_eq!(plan.steps[0].done_at, 5);
        assert_eq!(current(&plan), Some(1));
        let text = plan_text(&plan);
        assert!(text.starts_with("Plan for task #4, set by the user:"), "{text}");
        assert!(text.contains("2) [todo] the user: Check it"), "{text}");
        assert_eq!(earlier(&plan, 1), vec!["Step 1 (claude) is done.".to_string()]);
    }

    #[test]
    fn done_marks_only_open_steps() {
        let mut plan = build_plan(1, &[input("user", "a"), input("nixa", "b")], "u", &[1], 5).unwrap();
        assert_eq!(mark_done(&mut plan, &[1, 2, 9], 7), vec![2]);
        assert!(!is_open(&plan));
    }

    #[test]
    fn plans_are_saved_with_the_task() {
        let conn = crate::open_memory().unwrap();
        assert!(set_plan(&conn, 1, &[input("user", "a")], "u", &[]).is_err());
        let task = tasks::add_task(&conn, &TaskInput { title: "Plan me".into(), ..TaskInput::default() }, &Actor::new(AuthorKind::User, "u")).unwrap();
        let plan = set_plan(&conn, task.id, &[input("agent:codex", "Build"), input("user", "Look")], "u", &[]).unwrap();
        assert_eq!(get_plan(&conn, task.id).unwrap(), Some(plan.clone()));
        assert!(has_open_plan(&conn, task.id));
        let mut doing = plan.clone();
        doing.steps[0].state = StepState::Doing;
        save_plan(&conn, &doing).unwrap();
        assert!(set_plan(&conn, task.id, &[input("user", "x")], "u", &[]).is_err());
        assert_eq!(list_plans(&conn).unwrap().len(), 1);
        assert!(remove_plan(&conn, task.id).unwrap());
        assert_eq!(get_plan(&conn, task.id).unwrap(), None);
    }
}
