use super::journal::{self, Decision, result_moves};
use crate::prompts::ask_match::same_ask;
use nixa_store::Connection;
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::LazyLock;

pub const HIGH_MIN: u64 = 5;
pub const HIGH_RATE: f64 = 0.9;
pub const MIDDLE_MIN: u64 = 2;
pub const MIDDLE_RATE: f64 = 0.6;
pub const HALVE_OVER: u64 = 40;
pub const QUIET_MS: u64 = 30 * 60 * 1000;
pub const ERROR_MS: u64 = 5 * 60 * 1000;
pub const COMPLAINT_MS: u64 = 2 * 60 * 1000;
pub const SAME_JOB_MS: u64 = 2 * 60 * 60 * 1000;
pub const HOLD_YES_MS: u64 = 2 * 60 * 60 * 1000;
pub const SETTLE_WINDOW_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub const NO_PROJECT: &str = "none";
pub const SCORES_SHOWN: usize = 12;
pub const ERROR_REASON: &str = "error on screen:";
const ACTION_CHARS: usize = 40;

pub const SQL: &str = "
CREATE TABLE IF NOT EXISTS scores (
    key TEXT PRIMARY KEY,
    good INTEGER NOT NULL DEFAULT 0,
    bad INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
";

fn fail(e: impl std::fmt::Display) -> String {
    format!("The scores failed: {e}")
}

fn part(text: Option<&str>) -> String {
    text.unwrap_or_default().split(|c: char| c == '|' || c.is_whitespace()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn project_part(project: Option<&str>) -> String {
    let clean = part(project);
    let clean = clean.trim_end_matches('/');
    if clean.is_empty() || clean == "(no folder)" { NO_PROJECT.into() } else { clean.to_string() }
}

pub fn kind_key(kind: &str, action: Option<&str>, project: Option<&str>) -> String {
    let action: String = part(action).to_lowercase().replace(' ', "-").chars().take(ACTION_CHARS).collect();
    let action = if action.is_empty() { "any".to_string() } else { action };
    format!("{kind}|{action}|{}", project_part(project))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyParts {
    pub kind: String,
    pub action: String,
    pub project: String,
}

pub fn key_parts(key: &str) -> KeyParts {
    let mut split = key.splitn(3, '|');
    let kind = split.next().unwrap_or_default().to_string();
    let action = split.next().unwrap_or_default().to_string();
    let project = split.next().filter(|p| !p.is_empty()).unwrap_or(NO_PROJECT).to_string();
    KeyParts { kind, action, project }
}

static YES_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^Needs your yes:\s*").expect("pattern"));
static SEND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^send\b").expect("pattern"));
static APPLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^apply\s+([a-z0-9_.-]+)").expect("pattern"));
static FIRST_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\p{L}\p{N}_.-]+").expect("pattern"));
static RULE_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^rule (\S+?):").expect("pattern"));

pub fn message_action(text: &str) -> String {
    let clean = YES_HEAD.replace(text, "");
    let clean = clean.trim();
    if SEND.is_match(clean) {
        return "message".into();
    }
    if let Some(found) = APPLY.captures(clean) {
        return found[1].to_lowercase();
    }
    FIRST_WORD.find(clean).map(|m| m.as_str().to_lowercase()).unwrap_or_default()
}

pub fn prompt_action(kind: &str, tool: Option<&str>) -> String {
    match kind {
        "permission" => {
            let tool = part(tool);
            format!("permission:{}", if tool.is_empty() { "tool" } else { &tool })
        }
        "plan" => "plan".into(),
        _ => "question".into(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActionInput<'a> {
    pub kind: &'a str,
    pub choice: &'a str,
    pub reason: Option<&'a str>,
    pub member: Option<&'a str>,
    pub agent: Option<&'a str>,
    pub verbs: &'a [String],
}

pub fn action_of(input: &ActionInput) -> String {
    let verb = input.verbs.first();
    match input.kind {
        "rule" => format!("rule:{}", RULE_ID.captures(input.reason.unwrap_or_default()).map(|c| c[1].to_string()).unwrap_or_else(|| "any".into())),
        "job" | "fix" | "close" => input.member.or(input.agent).filter(|m| !m.is_empty()).unwrap_or("agent").to_string(),
        "hold" => verb.map(|v| format!("git:{v}")).unwrap_or_else(|| "step".into()),
        "prompt" => "question".into(),
        _ => verb.map(|v| format!("git:{v}")).unwrap_or_else(|| message_action(input.choice)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    High,
    Middle,
    Low,
}

impl Level {
    pub fn word(self) -> &'static str {
        match self {
            Level::High => "high",
            Level::Middle => "middle",
            Level::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ScoreRow {
    pub key: String,
    pub good: u64,
    pub bad: u64,
}

pub fn level_of(good: u64, bad: u64) -> (Level, f64, u64) {
    let count = good + bad;
    let rate = if count == 0 { 0.0 } else { good as f64 / count as f64 };
    let level = if count >= HIGH_MIN && rate >= HIGH_RATE {
        Level::High
    } else if count >= MIDDLE_MIN && rate >= MIDDLE_RATE {
        Level::Middle
    } else {
        Level::Low
    };
    (level, rate, count)
}

pub fn score_line(row: &ScoreRow) -> String {
    let parts = key_parts(&row.key);
    let (level, _, count) = level_of(row.good, row.bad);
    let place = if parts.project == NO_PROJECT { "with no project".to_string() } else { format!("in {}", journal::project_name(&parts.project)) };
    format!("{} {} {place}: {} of {count} good, {}", parts.kind, parts.action, row.good, level.word())
}

pub fn ensure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(SQL).map_err(fail)
}

pub fn get(conn: &Connection, key: &str) -> Result<ScoreRow, String> {
    let mut stmt = conn.prepare("SELECT good, bad FROM scores WHERE key = ?1").map_err(fail)?;
    let mut rows = stmt.query_map((key,), |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))).map_err(fail)?;
    let (good, bad) = rows.next().transpose().map_err(fail)?.unwrap_or((0, 0));
    Ok(ScoreRow { key: key.to_string(), good: good.max(0) as u64, bad: bad.max(0) as u64 })
}

pub fn next_counts(row: &ScoreRow, result: &str, before: Option<&str>) -> (u64, u64) {
    let moved = u64::from(before == Some("good") && result == "bad");
    let good = (row.good + u64::from(result == "good")).saturating_sub(moved);
    let bad = row.bad + u64::from(result == "bad");
    if good + bad > HALVE_OVER {
        ((good as f64 / 2.0).round() as u64, (bad as f64 / 2.0).round() as u64)
    } else {
        (good, bad)
    }
}

pub fn add(conn: &Connection, key: &str, result: &str, before: Option<&str>, now: u64) -> Result<ScoreRow, String> {
    let row = get(conn, key)?;
    if result == "neutral" {
        return Ok(row);
    }
    let (good, bad) = next_counts(&row, result, before);
    conn.execute(
        "INSERT INTO scores (key, good, bad, updated_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(key) DO UPDATE SET good = excluded.good, bad = excluded.bad, updated_at = excluded.updated_at",
        (key, good as i64, bad as i64, now as i64),
    )
    .map_err(fail)?;
    Ok(ScoreRow { key: key.to_string(), good, bad })
}

pub fn level(conn: &Connection, key: &str) -> Result<Level, String> {
    let row = get(conn, key)?;
    Ok(level_of(row.good, row.bad).0)
}

pub fn top(conn: &Connection, project: Option<&str>, limit: usize) -> Result<Vec<ScoreRow>, String> {
    let mut stmt = conn.prepare("SELECT key, good, bad FROM scores ORDER BY good + bad DESC, updated_at DESC").map_err(fail)?;
    let rows = stmt
        .query_map((), |row| Ok(ScoreRow { key: row.get(0)?, good: row.get::<_, i64>(1)?.max(0) as u64, bad: row.get::<_, i64>(2)?.max(0) as u64 }))
        .map_err(fail)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(fail)?;
    let wanted = project.map(|p| p.trim().trim_end_matches('/')).filter(|p| !p.is_empty());
    Ok(rows
        .into_iter()
        .filter(|row| match wanted {
            Some(w) => {
                let own = key_parts(&row.key).project;
                own == w || journal::project_name(&own) == w
            }
            None => true,
        })
        .take(limit)
        .collect())
}

pub fn lines(conn: &Connection, project: Option<&str>, limit: usize) -> Result<Vec<String>, String> {
    Ok(top(conn, project, limit)?.iter().map(score_line).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settle {
    pub id: i64,
    pub outcome: String,
    pub proof: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checks {
    Passed,
    Failed,
    None,
}

pub fn check_state(ran: Option<(bool, bool)>) -> Checks {
    match ran {
        None => Checks::None,
        Some((_, true)) => Checks::None,
        Some((true, false)) => Checks::Failed,
        Some((false, false)) => Checks::Passed,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobClose {
    pub outcome: String,
    pub checks: Option<Checks>,
    pub spent: bool,
    pub short: Option<String>,
}

pub fn job_result(input: &JobClose) -> (String, String) {
    let short = input.short.as_deref().map(|s| format!(": {s}")).unwrap_or_default();
    let checks = input.checks.unwrap_or(Checks::None);
    if input.outcome == "done" {
        return match checks {
            Checks::Passed => ("good".into(), format!("closed done, checks passed{short}")),
            Checks::Failed => ("neutral".into(), format!("closed done, but the checks failed{short}")),
            Checks::None => ("neutral".into(), "closed done with no checks".into()),
        };
    }
    if input.spent || input.outcome == "used all rounds" {
        return ("bad".into(), "closed after all fix rounds".into());
    }
    if input.outcome == "same fix twice" {
        return ("bad".into(), "closed after the same fix twice".into());
    }
    if input.outcome == "checks failed" {
        return ("bad".into(), format!("closed with failed checks{short}"));
    }
    ("neutral".into(), format!("closed: {}", input.outcome))
}

pub fn fix_result(checks: Checks, short: Option<&str>) -> Option<(String, String)> {
    let tail = short.map(|s| format!(": {s}")).unwrap_or_default();
    match checks {
        Checks::Passed => Some(("good".into(), format!("the next check passed{tail}"))),
        Checks::Failed => Some(("bad".into(), format!("the check failed again{tail}"))),
        Checks::None => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub terminal: String,
    pub at: u64,
}

#[derive(Debug, Clone, Default)]
pub struct SettleInput<'a> {
    pub decisions: &'a [Decision],
    pub now: u64,
    pub approvals: &'a [Mark],
    pub complaints: &'a [u64],
    pub errors: &'a [Mark],
}

const TIMED: [&str; 3] = ["prompt", "auto-yes", "rule"];

fn same_job(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim()) || same_ask(a, b)
}

pub fn settle(input: &SettleInput) -> Vec<Settle> {
    let mut rows: Vec<&Decision> = input.decisions.iter().collect();
    rows.sort_by(|a, b| a.at.cmp(&b.at).then(a.id.cmp(&b.id)));
    let mut out: HashMap<i64, Settle> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();
    let current = |out: &HashMap<i64, Settle>, row: &Decision| -> Option<String> { out.get(&row.id).map(|s| s.outcome.clone()).or_else(|| row.result.clone()) };
    let mut set = |out: &mut HashMap<i64, Settle>, row: &Decision, outcome: &str, proof: &str| {
        if row.key.is_none() {
            return;
        }
        let before = out.get(&row.id).map(|s| s.outcome.clone()).or_else(|| row.result.clone());
        if !result_moves(before.as_deref(), outcome) {
            return;
        }
        if !out.contains_key(&row.id) {
            order.push(row.id);
        }
        out.insert(row.id, Settle { id: row.id, outcome: outcome.into(), proof: proof.into() });
    };

    for row in &rows {
        if row.corrected && row.kind != "confirm" {
            set(&mut out, row, "bad", "the user corrected it");
        }
    }

    for at in input.complaints {
        let target = rows.iter().rfind(|r| r.kind != "confirm" && r.at <= *at && at - r.at <= COMPLAINT_MS);
        if let Some(target) = target {
            set(&mut out, target, "bad", "the user complained within 2 minutes");
        }
    }

    let jobs: Vec<&Decision> = rows.iter().copied().filter(|r| r.kind == "job" && r.terminal.is_some()).collect();
    for close in rows.iter().filter(|r| r.kind == "close" && r.terminal.is_some() && r.key.is_some()) {
        let Some(first) = jobs.iter().rfind(|j| j.terminal == close.terminal && j.at <= close.at) else { continue };
        let project = key_parts(close.key.as_deref().unwrap_or_default()).project;
        let back = jobs.iter().find(|j| {
            j.id != first.id
                && j.at > close.at
                && j.at - close.at <= SAME_JOB_MS
                && j.key.as_deref().is_some_and(|k| key_parts(k).project == project)
                && same_job(&first.choice, &j.choice)
        });
        if back.is_none() {
            continue;
        }
        set(&mut out, close, "bad", "the same job came back within 2 hours");
        set(&mut out, first, "bad", "the same job came back within 2 hours");
    }

    for row in &rows {
        if current(&out, row).is_some() {
            continue;
        }
        let age = input.now.saturating_sub(row.at);
        let kind = row.kind.as_str();
        if kind == "confirm" {
            match row.outcome.as_deref() {
                Some("yes") => set(&mut out, row, "good", "the user said yes"),
                Some("no") => set(&mut out, row, "bad", "the user said no"),
                _ => set(&mut out, row, "neutral", "no answer"),
            }
            continue;
        }
        if TIMED.contains(&kind) {
            if age < QUIET_MS {
                continue;
            }
            let stopped = input.errors.iter().any(|e| Some(&e.terminal) == row.terminal.as_ref() && e.at > row.at && e.at - row.at <= ERROR_MS);
            if stopped {
                set(&mut out, row, "neutral", "an error stop came within 5 minutes");
            } else {
                set(&mut out, row, "good", "30 minutes with no correction, complaint or error stop");
            }
            continue;
        }
        if kind == "hold" {
            let yes = input.approvals.iter().any(|a| Some(&a.terminal) == row.terminal.as_ref() && a.at >= row.at && a.at - row.at <= HOLD_YES_MS);
            if yes {
                set(&mut out, row, "good", "the user said yes to the held step within 2 hours");
            } else if age > HOLD_YES_MS {
                set(&mut out, row, "neutral", "no yes within 2 hours");
            }
            continue;
        }
        if kind == "close" {
            if age >= SAME_JOB_MS {
                set(&mut out, row, "good", "the same job did not come back in 2 hours");
            }
            continue;
        }
        if (kind == "job" || kind == "fix") && row.terminal.is_some() {
            let next = jobs.iter().any(|j| j.terminal == row.terminal && j.id != row.id && (j.at > row.at || (j.at == row.at && j.id > row.id)));
            if next {
                set(&mut out, row, "neutral", "a new job started before a result");
            }
        }
    }
    order.into_iter().filter_map(|id| out.remove(&id)).collect()
}

pub fn error_stops(decisions: &[Decision]) -> Vec<Mark> {
    decisions
        .iter()
        .filter(|d| d.reason.as_deref().is_some_and(|r| r.starts_with(ERROR_REASON)))
        .filter_map(|d| d.terminal.clone().map(|terminal| Mark { terminal, at: d.at }))
        .collect()
}

pub fn apply(conn: &Connection, results: &[Settle], now: u64) -> Result<usize, String> {
    let mut changed = 0;
    for result in results {
        let Some(done) = journal::settle(conn, result.id, &result.outcome, Some(&result.proof), now)? else { continue };
        add(conn, &done.key, &result.outcome, done.before.as_deref(), now)?;
        changed += 1;
    }
    Ok(changed)
}

pub fn job_closed(conn: &Connection, terminal: &str, input: &JobClose, now: u64) -> Result<usize, String> {
    let job = journal::open_rows(conn, terminal, "job")?.pop();
    let fixes = journal::open_rows(conn, terminal, "fix")?;
    let (outcome, proof) = job_result(input);
    let mut results: Vec<Settle> = job.map(|j| Settle { id: j.id, outcome: outcome.clone(), proof: proof.clone() }).into_iter().collect();
    results.extend(fixes.iter().map(|f| Settle { id: f.id, outcome: "neutral".into(), proof: "the job closed before the next check".into() }));
    apply(conn, &results, now)
}

pub fn checked(conn: &Connection, terminal: &str, checks: Checks, short: Option<&str>, now: u64) -> Result<usize, String> {
    let Some((outcome, proof)) = fix_result(checks, short) else { return Ok(0) };
    let results: Vec<Settle> = journal::open_rows(conn, terminal, "fix")?.iter().map(|f| Settle { id: f.id, outcome: outcome.clone(), proof: proof.clone() }).collect();
    apply(conn, &results, now)
}

pub fn settle_now(conn: &Connection, approvals: &[Mark], complaints: &[u64], now: u64) -> Result<usize, String> {
    let decisions = journal::since(conn, now.saturating_sub(SETTLE_WINDOW_MS))?;
    let errors = error_stops(&decisions);
    let results = settle(&SettleInput { decisions: &decisions, now, approvals, complaints, errors: &errors });
    apply(conn, &results, now)
}
