use crate::app::App;
use crate::schedules::jobs;
use crate::schedules::runner::{self, Choice};
use crate::tools::{Args, ToolResult, flag, need, schema, spec};
use crate::turns::ToolSpec;
use crate::util::clip;
use nixa_store::schedules::{Repeat, Schedule, ScheduleInput};
use serde_json::{Map, Value, json};
use std::sync::Arc;

mod work;

pub const NAME: &str = "nixa_schedules";
pub const ACTIONS: [&str; 10] = ["list", "add", "change", "pause", "resume", "skip", "remove", "run", "work", "all"];
pub const FIELDS: [&str; 12] = ["name", "agent", "folder", "prompt", "repeat", "time", "date", "days", "enabled", "follow_agent", "follow_prompt", "skip_unchanged"];
pub const PROMPT_CHARS: usize = 200;
pub const PREVIEW_CHARS: usize = 400;
pub const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const WHY: &str = "A schedule change asked by the chat brain";

pub fn spec_of() -> ToolSpec {
    let schedule = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": {"type": "string"},
            "agent": {"type": "string", "description": "An agent, member or team id from list. Never invent ids."},
            "folder": {"type": "string", "description": "Absolute project folder, ~/folder, or a space name."},
            "prompt": {"type": "string", "description": "The instructions the agent gets as its first message."},
            "repeat": {"type": "string", "enum": ["once", "hourly", "daily", "weekdays", "weekly"]},
            "time": {"type": "string", "description": "HH:MM, 24 hours, in this Mac's time zone. For hourly only the minutes count."},
            "date": {"type": "string", "description": "YYYY-MM-DD, needed for once."},
            "days": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 6}, "description": "Needed for weekly. Sunday is 0."},
            "enabled": {"type": "boolean", "description": "Default true for add."},
            "follow_agent": {"type": "string", "description": "Optional agent id that starts on the open tasks of the report when a run is done."},
            "follow_prompt": {"type": "string"},
            "skip_unchanged": {"type": "boolean", "description": "true: a timed run is skipped when nothing changed in the folder since the last run."}
        }
    });
    spec(
        NAME,
        "The user's saved schedules. Each one starts an agent in a project folder with instructions, once or on repeat. list shows every schedule with its id, agent, folder, when, next and last run, and the agent, member and team ids you may use. add saves a new one (schedule: name, agent, folder, prompt, repeat, time, and date for once or days for weekly). change changes the fields given in schedule for the schedule id. pause and resume turn one schedule off or on, skip skips its next run, remove deletes it, run starts it now. work queues the configured follow-up agent (or the schedule's agent) for a single open checkbox by displayed order, or all open checkboxes if task is omitted. Only use work when asked to start work, not when asked to read a report. all with enabled turns every schedule on or off. Automatic applies valid actions without waiting for approval: all report work, risky or not, starts and goes into the decision journal, and work blocked by a locked space or Hold starts by itself once that clears. Only Manual uses a yes card.",
        schema(
            json!({
                "action": {"type": "string", "enum": ACTIONS},
                "id": {"type": "string", "description": "Schedule id from list, for change, pause, resume, skip, remove, run and work. Never invent ids."},
                "task": {"type": "integer", "minimum": 1, "description": "For work: count only open boxes in the last report's Tasks section, starting at 1. Omit for all open tasks."},
                "enabled": {"type": "boolean", "description": "For all: true turns every schedule on, false pauses every schedule."},
                "schedule": schedule
            }),
            &["action"],
        ),
    )
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let action = need(args, "action")?;
    match action.as_str() {
        "list" => Ok(list(app)),
        "add" => add(app, args).await,
        "change" => change(app, args).await,
        "pause" | "resume" => toggle(app, args, action == "resume").await,
        "skip" | "remove" | "run" => one(app, args, &action).await,
        "work" => work::call(app, args).await,
        "all" => all(app, args).await,
        other => Err(format!("action {other} is not known. Use one of: {}.", ACTIONS.join(", "))),
    }
}

fn fail(error: anyhow::Error) -> String {
    format!("{error:#}")
}

pub fn when_words(repeat: Repeat, time: &str, date: &str, days: &[i64]) -> String {
    match repeat {
        Repeat::Once => format!("once on {date} at {time}"),
        Repeat::Hourly => format!("every hour at :{}", time.get(3..5).unwrap_or("00")),
        Repeat::Daily => format!("every day at {time}"),
        Repeat::Weekdays => format!("on weekdays at {time}"),
        Repeat::Weekly => {
            let names: Vec<&str> = days.iter().filter_map(|d| usize::try_from(*d).ok().and_then(|i| DAY_NAMES.get(i).copied())).collect();
            format!("every week on {} at {time}", names.join(", "))
        }
    }
}

pub fn describe(title: &str, input: &ScheduleInput, agent: &str, follow: &str) -> String {
    let mut lines = vec![
        format!("{title}: {}?", input.name),
        format!("Agent: {agent}"),
        format!("Folder: {}", input.folder),
        format!("When: {}", when_words(input.repeat, &input.time, &input.date, &input.days)),
    ];
    if !input.prompt.is_empty() {
        lines.push(format!("Instructions: {}", clip(&input.prompt, PREVIEW_CHARS)));
    }
    if !input.follow_agent.is_empty() {
        lines.push(format!("Follow-up agent on the report tasks: {follow}"));
    }
    if input.skip_unchanged {
        lines.push("Skips a timed run when nothing changed since the last run".into());
    }
    if !input.enabled {
        lines.push("Saved as paused".into());
    }
    lines.join("\n")
}

pub fn changes(old: &Value, new: &Value) -> Vec<String> {
    FIELDS
        .iter()
        .filter(|k| old.get(**k) != new.get(**k))
        .map(|k| {
            let show = |v: Option<&Value>| match v {
                Some(Value::String(s)) => clip(s, PREVIEW_CHARS),
                Some(Value::Null) | None => "(none)".into(),
                Some(other) => other.to_string(),
            };
            format!("{k}: {} to {}", show(old.get(*k)), show(new.get(*k)))
        })
        .collect()
}

pub fn merge(old: &Value, given: &Map<String, Value>) -> Result<Value, String> {
    if let Some(bad) = given.keys().find(|k| !FIELDS.contains(&k.as_str())) {
        return Err(format!("{bad} is not a schedule field. Fields: {}.", FIELDS.join(", ")));
    }
    let mut out = old.clone();
    for (key, value) in given {
        out[key.as_str()] = value.clone();
    }
    Ok(out)
}

struct Lists {
    agents: Vec<Choice>,
    members: Vec<Choice>,
    teams: Vec<Choice>,
}

fn lists(app: &App) -> Lists {
    Lists { agents: runner::agents(app), members: runner::members(app), teams: runner::teams(app) }
}

impl Lists {
    fn known(&self, id: &str) -> bool {
        id == crate::assistant::AGENT || [&self.agents, &self.members, &self.teams].iter().any(|l| l.iter().any(|c| c.id == id))
    }
}

fn check_input(app: &App, v: &Value, lists: &Lists) -> Result<ScheduleInput, String> {
    let folder_text = v.get("folder").and_then(Value::as_str).unwrap_or_default();
    if folder_text.trim().is_empty() {
        return Err("A schedule needs folder.".into());
    }
    let folder = jobs::folder_of(app, folder_text).map_err(fail)?;
    let input = jobs::read_input(v, folder).map_err(fail)?;
    if !lists.known(&input.agent) {
        return Err(format!("{} is not a known agent, member or team id. Use an id from list.", input.agent));
    }
    if !input.follow_agent.is_empty() && !lists.known(&input.follow_agent) {
        return Err(format!("{} is not a known follow-up agent id. Use an id from list.", input.follow_agent));
    }
    if input.enabled && jobs::input_next(&jobs::zone_of(app), &input, jobs::now()).is_none() {
        return Err("That date and time has already passed.".into());
    }
    Ok(input)
}

fn brief(item: &Value) -> Value {
    let view = &item["view"];
    json!({
        "id": item["id"],
        "name": item["name"],
        "on": item["enabled"],
        "agent": item["agent"],
        "agent_name": view["agent_name"],
        "folder": item["folder"],
        "when": view["when"],
        "next": view["next"],
        "last": view["last"],
        "last_report": item["last_run"]["report"],
        "open_tasks": item["last_run"]["open_tasks"],
        "failed": view["failed"],
        "follow_agent": item["follow_agent"],
        "skip_unchanged": item["skip_unchanged"],
        "busy": view["busy"],
        "prompt": clip(item["prompt"].as_str().unwrap_or_default(), PROMPT_CHARS),
    })
}

fn ids(list: &Value) -> Value {
    json!(list.as_array().map(|l| l.iter().map(|c| json!({"id": c["id"], "name": c["name"]})).collect::<Vec<_>>()).unwrap_or_default())
}

pub fn list(app: &App) -> String {
    let snap = crate::schedules::snapshot(app);
    let items: Vec<Value> = snap["schedules"].as_array().map(|l| l.iter().map(brief).collect()).unwrap_or_default();
    let mut out = json!({
        "all_on": snap["enabled"],
        "zone": snap["zone"],
        "schedules": items,
        "agents": ids(&snap["agents"]),
        "members": ids(&snap["members"]),
        "teams": ids(&snap["teams"]),
    });
    if let Some(error) = snap.get("error") {
        out["error"] = error.clone();
    }
    out.to_string()
}

fn found(app: &App, id: &str) -> Value {
    let snap = crate::schedules::snapshot(app);
    snap["schedules"].as_array().and_then(|l| l.iter().find(|s| s["id"] == id).map(brief)).unwrap_or(Value::Null)
}

fn done(app: &App, action: &str, id: &str) -> String {
    json!({"status": "done", "action": action, "schedule": found(app, id)}).to_string()
}

fn schedule_of(app: &App, args: &Args) -> Result<Schedule, String> {
    let id = need(args, "id")?;
    jobs::get(app, &id).map_err(fail)
}

async fn add(app: &Arc<App>, args: &Args) -> ToolResult {
    let mut given = args.get("schedule").and_then(Value::as_object).cloned().ok_or("add needs schedule.")?;
    given.remove("id");
    if let Some(bad) = given.keys().find(|k| !FIELDS.contains(&k.as_str())) {
        return Err(format!("{bad} is not a schedule field. Fields: {}.", FIELDS.join(", ")));
    }
    given.entry("enabled").or_insert(json!(true));
    let mut value = Value::Object(given);
    let names = lists(app);
    let input = check_input(app, &value, &names)?;
    value["folder"] = json!(input.folder);
    let before: Vec<String> = jobs::list(app).map_err(fail)?.into_iter().map(|s| s.id).collect();
    jobs::command(app, "schedule.save", &json!({"schedule": value})).map_err(fail)?;
    let id = jobs::list(app).map_err(fail)?.into_iter().find(|s| !before.contains(&s.id)).map(|s| s.id).unwrap_or_default();
    Ok(done(app, "add", &id))
}

async fn change(app: &Arc<App>, args: &Args) -> ToolResult {
    let old = schedule_of(app, args)?;
    let mut given = args.get("schedule").and_then(Value::as_object).cloned().ok_or("change needs schedule with the fields to change.")?;
    given.remove("id");
    let before = serde_json::to_value(&old).map_err(|e| e.to_string())?;
    let mut merged = merge(&before, &given)?;
    let names = lists(app);
    let input = check_input(app, &merged, &names)?;
    merged["folder"] = json!(input.folder);
    let changed = changes(&before, &merged);
    if changed.is_empty() {
        return Ok(json!({"status": "unchanged", "reason": "Nothing in the schedule would change."}).to_string());
    }
    merged["id"] = json!(old.id);
    jobs::command(app, "schedule.save", &json!({"schedule": merged})).map_err(fail)?;
    Ok(done(app, "change", &old.id))
}

async fn toggle(app: &Arc<App>, args: &Args, on: bool) -> ToolResult {
    let s = schedule_of(app, args)?;
    if s.enabled == on {
        return Ok(json!({"status": "unchanged", "reason": format!("{} is already {}.", s.name, if on { "on" } else { "paused" })}).to_string());
    }
    jobs::command(app, "schedule.toggle", &json!({"id": s.id, "enabled": on})).map_err(fail)?;
    Ok(done(app, if on { "resume" } else { "pause" }, &s.id))
}

async fn one(app: &Arc<App>, args: &Args, action: &str) -> ToolResult {
    let s = schedule_of(app, args)?;
    let command = match action {
        "skip" => "schedule.skip",
        "remove" => "schedule.delete",
        _ => {
            let step = format!("run the schedule {} now", s.name);
            if let Some(reason) = work::blocked_now(app, &s) {
                let id = s.id.clone();
                crate::modes::note(app, "auto-yes", &step, None, Some("Nixa keeps the run until the lock or hold clears"));
                work::start_later(app, s, work::Later::Run, step);
                return Ok(json!({"status": "waiting", "action": "run", "id": id, "reason": work::waiting_text(&reason)}).to_string());
            }
            if let Some(held) = crate::term_tools::work_hold(app, "schedule-run", &step, None) {
                return Ok(json!({"status": "held", "reason": held, "changed": "Nothing started."}).to_string());
            }
            "schedule.run"
        }
    };
    jobs::command(app, command, &json!({"id": s.id})).map_err(fail)?;
    if action == "remove" {
        return Ok(json!({"status": "done", "action": "remove", "removed": s.name}).to_string());
    }
    Ok(done(app, action, &s.id))
}

async fn all(app: &Arc<App>, args: &Args) -> ToolResult {
    let on = flag(args, "enabled").ok_or("all needs enabled: true or false.")?;
    if jobs::all_on(app) == on {
        return Ok(json!({"status": "unchanged", "reason": format!("Schedules are already {}.", if on { "on" } else { "paused" })}).to_string());
    }
    jobs::command(app, "scheduler.set", &json!({"enabled": on})).map_err(fail)?;
    Ok(json!({"status": "done", "action": "all", "all_on": jobs::all_on(app)}).to_string())
}
