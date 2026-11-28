use crate::app::{App, lock};
use crate::extras::usage::{DayUsage, KEEP_DAYS, day_key, format_tokens};
use crate::tools::{Args, ToolResult, number, schema, spec, text};
use crate::turns::ToolSpec;
use crate::util::now_ms;
use nixa_store::models::{ModelRow, PlanUsage};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const NAME: &str = "nixa_usage";
pub const WHAT: [&str; 4] = ["all", "brain", "agents", "goals"];
pub const DEFAULT_DAYS: u64 = 7;
pub const TOP_MODELS: usize = 5;
const DAY_MS: u64 = 86_400_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentSum {
    pub agent: String,
    pub jobs: u64,
    pub good: u64,
    pub bad: u64,
    pub last_at: i64,
    pub models: Vec<(String, u64, u64, u64)>,
}

pub fn spec_of() -> ToolSpec {
    spec(
        NAME,
        &format!("Read chat brain tokens, coding agent activity, and Nixa's local goals. brain: last turn, today and recent days in input, output and cached tokens (kept {KEEP_DAYS} days, not split by brain or model). agents: starts, good and bad results in saved agent, model and plan records. goals: the existing goal report. all returns every section. Agent counts are not tokens or estimates of plan quota. Read only."),
        schema(
            json!({
                "what": {"type": "string", "enum": WHAT, "description": "all (default), brain, agents or goals"},
                "days": {"type": "integer", "minimum": 1, "maximum": KEEP_DAYS, "description": "How many days of brain tokens to list, default 7"}
            }),
            &[],
        ),
    )
}

pub fn call(app: &App, args: &Args) -> ToolResult {
    let what = text(args, "what").unwrap_or_else(|| "all".into());
    if !WHAT.contains(&what.as_str()) {
        return Err(format!("what must be one of: {}.", WHAT.join(", ")));
    }
    let days = if args.contains_key("days") { number(args, "days").unwrap_or(0) } else { DEFAULT_DAYS };
    if !(1..=KEEP_DAYS).contains(&days) {
        return Err(format!("days must be a whole number from 1 to {KEEP_DAYS}."));
    }
    let mut parts = Vec::new();
    let now = now_ms();
    if matches!(what.as_str(), "all" | "brain") {
        parts.push(brain_part(app, days, now));
    }
    if matches!(what.as_str(), "all" | "agents") {
        parts.push(agents_part(app));
    }
    if matches!(what.as_str(), "all" | "goals") {
        parts.push(crate::extras::goals_text(app, now));
    }
    Ok(parts.join("\n\n"))
}

fn brain_part(app: &App, days: u64, now: u64) -> String {
    let (rows, today, snap) = {
        let usage = lock(&app.extras.usage);
        (usage.by_day(now), usage.today(now), usage.snapshot(now))
    };
    let oldest = day_key(now.saturating_sub((days - 1) * DAY_MS));
    brain_text(crate::agent::brain_info(app).as_ref(), &rows, today, &snap, days, &oldest)
}

fn agents_part(app: &App) -> String {
    let read = nixa_store::open(&app.db_file).and_then(|conn| nixa_store::models::read(&conn));
    match read {
        Ok(cache) => {
            let rows = nixa_store::models::model_rows(&cache);
            agents_text(&rows, &nixa_store::models::plan_usage(&rows))
        }
        Err(error) => format!("Coding agents: the model list could not be read ({error:#})."),
    }
}

pub fn brain_text(info: Option<&Value>, rows: &[(String, DayUsage)], today: DayUsage, snap: &Value, days: u64, oldest: &str) -> String {
    let word = |key: &str| info.and_then(|i| i.get(key)).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let now_line = match (word("name"), word("kind"), word("model")) {
        (Some(name), kind, model) => format!(
            "Chat brain now: {name}{}{}.",
            kind.map(|k| format!(" ({k})")).unwrap_or_default(),
            model.map(|m| format!(", model {m}")).unwrap_or_default()
        ),
        _ => "No chat brain is picked.".to_string(),
    };
    let kept: Vec<&(String, DayUsage)> = rows.iter().filter(|(day, _)| day.as_str() >= oldest).collect();
    let turns: u64 = kept.iter().map(|(_, u)| u.turns).sum();
    let tokens: u64 = kept.iter().map(|(_, u)| u.tokens).sum();
    let mut lines = vec![
        now_line,
        "The token counts are for every chat brain used on that day. They are not split by brain or model.".to_string(),
        match snap.get("turnText").and_then(Value::as_str) {
            Some(turn) => format!("Last turn: {turn} tokens."),
            None => "Last turn: none since the brain started.".to_string(),
        },
        format!("Today ({}): {} turns, {} tokens.", snap.get("day").and_then(Value::as_str).unwrap_or_default(), today.turns, format_tokens(today.tokens)),
        format!("Last {days} days: {turns} turns, {} tokens.", format_tokens(tokens)),
    ];
    if kept.is_empty() {
        lines.push("- No saved days in this range.".into());
    }
    for (day, u) in kept {
        lines.push(format!("- {day}: {} turns, {} tokens (input {}, output {}, cached {})", u.turns, format_tokens(u.tokens), format_tokens(u.input), format_tokens(u.output), format_tokens(u.cached)));
    }
    lines.join("\n")
}

pub fn starts(count: u64) -> String {
    if count == 1 { "1 start".into() } else { format!("{count} starts") }
}

pub fn when(at: i64) -> String {
    if at <= 0 { "never".into() } else { crate::modes::clock::stamp(at as u64) }
}

pub fn sum_agents(rows: &[ModelRow]) -> Vec<AgentSum> {
    let mut by: BTreeMap<String, AgentSum> = BTreeMap::new();
    for row in rows {
        let sum = by.entry(row.agent.clone()).or_insert_with(|| AgentSum { agent: row.agent.clone(), ..AgentSum::default() });
        let usage = row.details.usage.clone().unwrap_or_default();
        sum.jobs += usage.jobs;
        sum.good += row.details.good;
        sum.bad += row.details.bad;
        sum.last_at = sum.last_at.max(usage.at);
        if usage.jobs > 0 || row.details.good > 0 || row.details.bad > 0 {
            sum.models.push((row.model.clone(), usage.jobs, row.details.good, row.details.bad));
        }
    }
    let mut out: Vec<AgentSum> = by.into_values().collect();
    for sum in &mut out {
        sum.models.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        sum.models.truncate(TOP_MODELS);
    }
    out.sort_by(|a, b| b.jobs.cmp(&a.jobs).then_with(|| a.agent.cmp(&b.agent)));
    out
}

pub fn agents_text(rows: &[ModelRow], plans: &[PlanUsage]) -> String {
    let sums = sum_agents(rows);
    let mut lines = vec!["Coding agents: starts and good and bad results from saved model records, not limited to the chat token date range. Nixa does not get token counts or plan quota from coding agents.".to_string()];
    let used = sums.iter().filter(|s| s.jobs > 0 || s.good > 0 || s.bad > 0);
    if sums.iter().all(|s| s.jobs == 0) {
        lines.push("- No job start is counted yet.".into());
    }
    for sum in used {
        lines.push(format!("- {}: {}, {} good, {} bad, last start {}", sum.agent, starts(sum.jobs), sum.good, sum.bad, when(sum.last_at)));
        for (model, jobs, good, bad) in &sum.models {
            lines.push(format!("  - {model}: {}, {good} good, {bad} bad", starts(*jobs)));
        }
    }
    let idle: Vec<&str> = sums.iter().filter(|s| s.jobs == 0 && s.good == 0 && s.bad == 0).map(|s| s.agent.as_str()).collect();
    if !idle.is_empty() {
        lines.push(format!("Agents with no counted start: {}.", idle.join(", ")));
    }
    lines.push("Plans:".into());
    if plans.is_empty() {
        lines.push("- No plan is known.".into());
    }
    let mut ordered: Vec<&PlanUsage> = plans.iter().collect();
    ordered.sort_by(|a, b| b.jobs.cmp(&a.jobs).then_with(|| a.plan.cmp(&b.plan)));
    for plan in ordered {
        let name = if plan.plan.trim().is_empty() { "Unknown plan" } else { plan.plan.as_str() };
        let companies = plan.companies.iter().filter(|c| !c.is_empty()).cloned().collect::<Vec<_>>().join(", ");
        lines.push(format!(
            "- {name} ({}): {}, {} good, {} bad, {} of {} models usable, {}, last start {}",
            if companies.is_empty() { "unknown company".to_string() } else { companies },
            starts(plan.jobs),
            plan.good,
            plan.bad,
            plan.available,
            plan.models,
            if plan.included { "paid by a plan" } else { "metered or unknown billing" },
            when(plan.last_at)
        ));
    }
    lines.join("\n")
}
