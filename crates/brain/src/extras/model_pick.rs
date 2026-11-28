use crate::app::{App, lock};
use crate::deep::{Ask, AskInput, AskState};
use crate::tools::{Args, ToolResult, schema, spec, text};
use crate::turns::ToolSpec;
use crate::util::{clip, log};
use nixa_store::agent_map::AgentMapRow;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

pub const MAP_TOOL: &str = "nixa_agent_map";
pub const ASK_KIND: &str = "model";
pub const WAIT_MS: u64 = 5 * 60 * 1000;
pub const OWN: &str = "Nixa's own pick (the model chooser)";
pub const NOTES_MAX: usize = 1400;
pub const OPTIONS: usize = 5;
pub const FACT_MAX: usize = 240;
pub const PICK_KEY: &str = "model_pick:";
pub const ASK_KEY: &str = "model_pick.ask:";
pub const NIXA_DECIDES: &str = "Nixa decides, so the deep brain is not asked";

static HOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:do not|don't|never) start\b|\bstart (?:nobody|no one|none|nothing)\b|^\s*(?:own:\s*)?(?:cancel|close|wait)\b").expect("hold"));
static CANCEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bcancel").expect("cancel"));
static CLOSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:close|done)\b").expect("close"));
static CUSTOM_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\ba(\d{1,4})\b").expect("custom id"));
static CUSTOM_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)custom agent\s+["']?([^"'.;,()]+)"#).expect("custom name"));
static ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\brow\s*(\d{1,3})\b").expect("row"));
static MODEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b([a-z][a-z-]*)::([^\s,;()]+)").expect("model"));
static OWN_WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^own\W*$|own pick|model chooser").expect("own"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hold {
    Wait,
    Cancel,
    Done,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pick {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub none: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub assigned_kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub assigned: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub backup: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub row: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<usize>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    pub ask: String,
    pub why: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold: Option<Hold>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomRef {
    pub id: String,
    pub title: String,
    pub space: String,
    pub state: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskRef {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub assigned_kind: String,
    pub assigned: String,
}

fn none(ask: &str, why: impl Into<String>, hold: Option<Hold>) -> Pick {
    Pick { none: true, ask: ask.to_string(), why: why.into(), hold, ..Pick::default() }
}

fn words(text: &str) -> HashSet<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(str::to_string).collect()
}

pub fn pick_of_row(row: &AgentMapRow) -> String {
    if !row.member.trim().is_empty() {
        return row.member.clone();
    }
    if !row.agent.is_empty() && !row.model.is_empty() {
        return format!("{}::{}", row.agent, row.model);
    }
    row.pick.clone()
}

pub fn row_line(row: &AgentMapRow, number: usize) -> String {
    let pick = pick_of_row(row);
    let mut parts = vec![format!("row {number}: {}", if row.section.is_empty() { row.job.clone() } else { format!("{} / {}", row.section, row.job) })];
    parts.push(if pick.is_empty() { "no pick".into() } else { pick });
    if !row.effort.is_empty() {
        parts.push(format!("effort {}", row.effort));
    }
    if row.backup {
        parts.push("backup".into());
    }
    if !row.place.is_empty() {
        parts.push(row.place.clone());
    }
    parts.join(", ")
}

pub fn rows_for_task(rows: &[AgentMapRow], text: &str, max: usize) -> Vec<(AgentMapRow, usize)> {
    let wanted = words(text);
    let mut scored: Vec<(usize, usize, AgentMapRow)> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.off)
        .map(|(i, r)| {
            let have = words(&format!("{} {} {} {} {}", r.job, r.section, r.notes, r.pick, r.member));
            (have.intersection(&wanted).count(), i + 1, r.clone())
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(max).map(|(_, n, r)| (r, n)).collect()
}

pub fn customs_line(space: &str, customs: &[CustomRef]) -> String {
    let list: Vec<String> = customs.iter().map(|c| if c.state.is_empty() { format!("{} {}", c.id, c.title) } else { format!("{} {} ({})", c.id, c.title, c.state) }).collect();
    clip(&format!("Custom agents in {space}: {}.", if list.is_empty() { "none".into() } else { list.join(", ") }), FACT_MAX)
}

pub fn ask_for(task: &TaskRef, rows: &[AgentMapRow], criteria: &[String], facts: &[String]) -> AskInput {
    let found = rows_for_task(rows, &format!("{}\n{}", task.title, task.description), OPTIONS);
    let assigned = if task.assigned.is_empty() { "No one is assigned yet.".to_string() } else { format!("Assigned now: {} {}.", if task.assigned_kind == "member" { "member" } else { "agent" }, task.assigned) };
    let mut head = vec![
        format!("Task #{}: {}", task.id, clip(&task.title, 160)),
        assigned,
        format!("The agent map has {} rows, in the user's order. You may answer \"row <number>\" for any row, or agent::model for one model.", rows.len()),
        "Or answer a custom agent id of this space (for example a9): Nixa sends the task to it, and waits when it is busy. Or \"wait: <why>\" to hold the start, or \"do not start: <why>\" (add cancel or done to close the task).".to_string(),
    ];
    head.extend(facts.iter().map(|f| clip(f, FACT_MAX)));
    let head = head.join("\n");
    let room = NOTES_MAX.saturating_sub(head.chars().count() + 12);
    let criteria: Vec<String> = criteria.iter().enumerate().map(|(i, c)| format!("{}) {c}", i + 1)).collect();
    let notes = if room > 40 && !criteria.is_empty() { format!("{head}\nCriteria: {}", clip(&criteria.join(" "), room)) } else { head };
    let mut options: Vec<String> = found.iter().map(|(row, number)| clip(&row_line(row, *number), 200)).collect();
    options.push(OWN.into());
    AskInput {
        question: format!("Which row of the agent map fits task #{} ({})? Nixa starts the task on that row's agent and model.", task.id, clip(&task.title, 160)),
        options,
        notes,
        kind: Some(ASK_KIND.into()),
        wait: Some(WAIT_MS),
        brain: None,
    }
}

fn hold_of(text: &str) -> Option<Hold> {
    if !HOLD.is_match(text) {
        return None;
    }
    if CANCEL.is_match(text) {
        return Some(Hold::Cancel);
    }
    if CLOSE.is_match(text) {
        return Some(Hold::Done);
    }
    Some(Hold::Wait)
}

fn custom_in<'a>(text: &str, customs: &'a [CustomRef]) -> Option<&'a CustomRef> {
    for found in CUSTOM_ID.captures_iter(text) {
        let id = format!("a{}", &found[1]);
        if let Some(item) = customs.iter().find(|c| c.id == id) {
            return Some(item);
        }
    }
    let named = CUSTOM_NAME.captures(text)?.get(1)?.as_str().trim().to_lowercase();
    customs.iter().find(|c| c.title.to_lowercase() == named || named.starts_with(&c.title.to_lowercase()))
}

fn agent_pick(agent: &str, model: &str, installed: &dyn Fn(&str) -> bool, base: Pick) -> Pick {
    if !installed(agent) {
        return none(&base.ask, format!("{agent} is not installed, so {} cannot start", base.label), None);
    }
    Pick { assigned_kind: "agent".into(), assigned: agent.to_string(), model: format!("{agent}::{model}"), ..base }
}

pub fn row_pick(row: &AgentMapRow, number: usize, customs: &[CustomRef], installed: &dyn Fn(&str) -> bool, ask: &str, why: &str) -> Pick {
    let label = format!("row {number} ({}): {}", clip(&row.job, 80), { let p = pick_of_row(row); if p.is_empty() { "no pick".to_string() } else { p } });
    let base = Pick { row: row.id.clone(), number: Some(number), label: label.clone(), ask: ask.into(), why: why.into(), effort: row.effort.clone(), backup: row.backup, ..Pick::default() };
    if !row.member.trim().is_empty() {
        let wanted = row.member.trim().to_lowercase();
        if let Some(custom) = customs.iter().find(|c| c.title.to_lowercase() == wanted || c.id.to_lowercase() == wanted) {
            let model = if !row.agent.is_empty() && !row.model.is_empty() { format!("{}::{}", row.agent, row.model) } else { String::new() };
            return Pick { assigned_kind: "agent".into(), assigned: format!("custom:{}", custom.id), model, ..base };
        }
        if row.agent.is_empty() || row.model.is_empty() {
            return none(ask, format!("{label} names {}, who is not a custom agent here", row.member), None);
        }
    }
    if !row.agent.is_empty() && !row.model.is_empty() {
        return agent_pick(&row.agent, &row.model, installed, base);
    }
    none(ask, format!("{label} is a command or a tool, not an agent start"), None)
}

pub fn pick_from_answer(ask: &Ask, rows: &[AgentMapRow], customs: &[CustomRef], installed: &dyn Fn(&str) -> bool, space: &str) -> Pick {
    let id = ask.id.as_str();
    let why = clip(ask.why.as_deref().unwrap_or_default().trim(), 300);
    if ask.state != AskState::Answered {
        return none(id, "the deep brain gave no answer in time", None);
    }
    let text = ask.pick.as_deref().unwrap_or_default().trim().to_string();
    let custom = custom_in(&text, customs);
    let hold = hold_of(&text);
    if let Some(hold) = hold
        && !(hold == Hold::Wait && custom.is_some())
    {
        let joined = [text.as_str(), why.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(". Why: ");
        return none(id, clip(&joined, 300), Some(hold));
    }
    if let Some(custom) = custom {
        if !space.is_empty() && !custom.space.is_empty() && custom.space != space {
            return none(id, format!("the custom agent {} ({}) belongs to another space", custom.id, custom.title), None);
        }
        return Pick { assigned_kind: "agent".into(), assigned: format!("custom:{}", custom.id), label: format!("custom agent {} ({})", custom.id, clip(&custom.title, 80)), ask: id.into(), why, ..Pick::default() };
    }
    if text.is_empty() || text == OWN || crate::deep::says_ask_user(&text) || OWN_WORDS.is_match(&text) {
        return none(id, if why.is_empty() { "the deep brain left the pick to Nixa".to_string() } else { why }, None);
    }
    if let Some(found) = ROW.captures(&text) {
        let at: usize = found[1].parse().unwrap_or(0);
        return match at.checked_sub(1).and_then(|i| rows.get(i)) {
            Some(row) => row_pick(row, at, customs, installed, id, &why),
            None => none(id, format!("the table has no row {at}"), None),
        };
    }
    if let Some(found) = MODEL.captures(&text) {
        let model = found[2].trim_end_matches('.').to_string();
        let base = Pick { label: format!("{}::{}", &found[1], &found[2]), ask: id.into(), why, ..Pick::default() };
        return agent_pick(&found[1], &model, installed, base);
    }
    none(id, format!("Nixa could not read the pick \"{}\"", clip(&text, 80)), None)
}

pub fn pick_line(pick: &Pick) -> String {
    if pick.none {
        return match pick.hold {
            Some(Hold::Wait) => format!("Deep brain {} said do not start this task: {}. Nixa does not start it now; a later start asks the deep brain again.", pick.ask, pick.why),
            Some(Hold::Cancel) => format!("Deep brain {} said do not start this task: {}. Nixa cancelled the task and started nothing.", pick.ask, pick.why),
            Some(Hold::Done) => format!("Deep brain {} said do not start this task: {}. Nixa closed the task as done and started nothing.", pick.ask, pick.why),
            None => format!("Deep brain {} gave no model pick for this task: {}. Nixa starts it with its own pick.", pick.ask, pick.why),
        };
    }
    let why = if pick.why.is_empty() { String::new() } else { format!(". Why: {}", pick.why) };
    format!("Deep brain {} picked {} for this task{why}.", pick.ask, pick.label)
}

pub fn read_pick(raw: &str) -> Option<Pick> {
    let pick: Pick = serde_json::from_str(raw).ok()?;
    if pick.ask.is_empty() {
        return None;
    }
    if pick.none || (matches!(pick.assigned_kind.as_str(), "member" | "agent") && !pick.assigned.is_empty()) { Some(pick) } else { None }
}

pub fn map_text(rows: &[AgentMapRow], criteria: &[String], fallover: &[String], q: &str) -> String {
    let shown: Vec<(AgentMapRow, usize)> = if q.trim().is_empty() {
        rows.iter().enumerate().filter(|(_, r)| !r.off).map(|(i, r)| (r.clone(), i + 1)).collect()
    } else {
        rows_for_task(rows, q, 10)
    };
    let mut out = vec![format!("Agent map: {} rows.", rows.len())];
    out.extend(shown.iter().map(|(r, n)| row_line(r, *n)));
    out.push("Selection criteria:".into());
    out.extend(criteria.iter().enumerate().map(|(i, c)| format!("{}. {c}", i + 1)));
    out.push(format!("Fall over order: {}.", if fallover.is_empty() { "none".into() } else { fallover.join(", ") }));
    out.join("\n")
}

pub fn specs() -> Vec<ToolSpec> {
    vec![spec(
        MAP_TOOL,
        "Read the agent map: the ordered table of which custom agent or model does which job, the selection criteria and the fall over order. Read only. Only the user changes it, in the Agent map window. When a task arrives, Nixa can ask the deep brain to pick a row and then starts the task on that row's agent and model.",
        schema(json!({"q": {"type": "string", "description": "Optional words of a job; the closest rows come first"}}), &[]),
    )]
}

pub fn call(app: &App, args: &Args) -> ToolResult {
    let conn = nixa_store::open(&app.db_file).map_err(|e| format!("{e:#}"))?;
    let map = nixa_store::agent_map::agent_map(&conn).map_err(|e| format!("{e:#}"))?;
    Ok(map_text(&map.rows, &map.criteria, &map.fallover, &text(args, "q").unwrap_or_default()))
}

fn customs_of(conn: &nixa_store::Connection, space: &str) -> Vec<CustomRef> {
    nixa_store::agents::list_agents(conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| !a.off && (space.is_empty() || a.space.is_empty() || a.space == space))
        .map(|a| CustomRef { id: a.id, title: a.title, space: a.space, state: String::new() })
        .collect()
}

fn installed_fn(app: &App) -> impl Fn(&str) -> bool {
    let programs = crate::models::programs(app);
    move |agent: &str| {
        let key = match agent {
            "antigravity" => "agy",
            other => other,
        };
        programs.iter().any(|p| p.installed && (p.key == key || p.key == agent))
    }
}

pub fn ask_for_task(app: &Arc<App>, task_id: i64) -> Result<String, String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| format!("{e:#}"))?;
    let task = nixa_store::tasks::get_task(&conn, task_id).map_err(|e| format!("{e:#}"))?.ok_or_else(|| format!("There is no task #{task_id}."))?;
    let map = nixa_store::agent_map::agent_map(&conn).map_err(|e| format!("{e:#}"))?;
    if map.rows.iter().all(|r| r.off) {
        return Err("The agent map has no rows to pick from.".into());
    }
    let space = crate::deep_service::resolve(app, Some(&task.space_id), None)?;
    if app.decision.nixa_decides() {
        return Err(NIXA_DECIDES.into());
    }
    let customs = customs_of(&conn, &space.id);
    let facts = vec![customs_line(&space.name, &customs)];
    let reference = TaskRef { id: task.id, title: task.title.clone(), description: task.description.clone(), assigned_kind: task.assigned_kind.clone(), assigned: task.assigned.clone() };
    let mut input = ask_for(&reference, &map.rows, &map.criteria, &facts);
    if task.workflow.is_empty() {
        input.notes = crate::workflows::pick::with_flows(&input.notes, &crate::workflows::pick::line_for(app));
    }
    let (made, _) = crate::deep_service::ask(app, input, &space)?;
    nixa_store::tasks::set_task_meta(&conn, &format!("{ASK_KEY}{}", made.id), &task_id.to_string()).map_err(|e| format!("{e:#}"))?;
    let app = app.clone();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move { crate::watch::deep_tick(&app).await });
    }
    Ok(made.id)
}

pub fn saved_pick(app: &App, task_id: i64) -> Option<Pick> {
    let conn = nixa_store::open(&app.db_file).ok()?;
    nixa_store::tasks::task_meta(&conn, &format!("{PICK_KEY}{task_id}")).ok().flatten().and_then(|raw| read_pick(&raw))
}

pub fn settle(app: &App, ask: &Ask) -> Option<(i64, Pick)> {
    if ask.kind.as_deref() != Some(ASK_KIND) || !matches!(ask.state, AskState::Answered | AskState::Timeout) {
        return None;
    }
    let conn = nixa_store::open(&app.db_file).ok()?;
    let key = format!("{ASK_KEY}{}", ask.id);
    let task_id: i64 = nixa_store::tasks::task_meta(&conn, &key).ok().flatten()?.parse().ok()?;
    let task = nixa_store::tasks::get_task(&conn, task_id).ok().flatten()?;
    let map = nixa_store::agent_map::agent_map(&conn).ok()?;
    let customs = customs_of(&conn, &task.space_id);
    let installed = installed_fn(app);
    let pick = pick_from_answer(ask, &map.rows, &customs, &installed, &task.space_id);
    let raw = serde_json::to_string(&pick).ok()?;
    if let Err(e) = nixa_store::tasks::set_task_meta(&conn, &format!("{PICK_KEY}{task_id}"), &raw) {
        log("extras", &format!("could not save the model pick for task #{task_id}: {e:#}"));
        return None;
    }
    let _ = nixa_store::tasks::remove_task_meta(&conn, &key);
    app.info(&pick_line(&pick));
    if let Some(flow) = crate::workflows::pick::apply_pick(app, task_id, ask, pick.hold.is_some()) {
        log("extras", &format!("task #{task_id} gets workflow {flow} from {}", ask.id));
    }
    Some((task_id, pick))
}

pub fn tick(app: &Arc<App>) {
    let asks: Vec<Ask> = lock(&app.deep).asks.iter().filter(|a| a.kind.as_deref() == Some(ASK_KIND) && matches!(a.state, AskState::Answered | AskState::Timeout)).cloned().collect();
    for ask in asks {
        if let Some((task, pick)) = settle(app, &ask) {
            log("extras", &format!("model pick for task #{task} from {}", ask.id));
            crate::queue::pick::picked(app, task, &pick);
        }
    }
}
