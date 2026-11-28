use crate::db::{flag, next_number};
use crate::text::{clean_line, clean_text};
use crate::time::{civil_from_days, days_from_civil, now_ms, parse_day, weekday_of_days, DAY_MS, HOUR_MS, MINUTE_MS};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

const COUNTER: &str = "schedules";
const PROMPT_MAX: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    Once,
    Hourly,
    Daily,
    Weekdays,
    Weekly,
}

impl Repeat {
    pub const ALL: [Repeat; 5] = [Repeat::Once, Repeat::Hourly, Repeat::Daily, Repeat::Weekdays, Repeat::Weekly];

    pub fn as_str(self) -> &'static str {
        match self {
            Repeat::Once => "once",
            Repeat::Hourly => "hourly",
            Repeat::Daily => "daily",
            Repeat::Weekdays => "weekdays",
            Repeat::Weekly => "weekly",
        }
    }

    pub fn parse(text: &str) -> Option<Repeat> {
        Self::ALL.into_iter().find(|r| r.as_str() == text.trim())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunResult {
    Started,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastRun {
    pub at: i64,
    pub result: RunResult,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub terminal: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub report: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_tasks: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub follow_terminal: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub team_run: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub marker: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub at: i64,
    pub count: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScheduleJob {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub space: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub repo: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mcps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    pub agent: String,
    pub folder: String,
    pub prompt: String,
    pub job: Option<ScheduleJob>,
    pub repeat: Repeat,
    pub time: String,
    pub date: String,
    pub days: Vec<i64>,
    pub enabled: bool,
    pub follow_agent: String,
    pub follow_prompt: String,
    pub next_run: Option<i64>,
    pub last_run: Option<LastRun>,
    pub skip_unchanged: bool,
    pub skipped: Option<Skipped>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleInput {
    pub name: String,
    pub agent: String,
    #[serde(default)]
    pub folder: String,
    pub prompt: String,
    #[serde(default)]
    pub job: Option<ScheduleJob>,
    pub repeat: Repeat,
    pub time: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub days: Vec<i64>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub follow_agent: String,
    #[serde(default)]
    pub follow_prompt: String,
    #[serde(default)]
    pub skip_unchanged: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SchedulePatch {
    pub name: Option<String>,
    pub agent: Option<String>,
    pub folder: Option<String>,
    pub prompt: Option<String>,
    pub job: Option<Option<ScheduleJob>>,
    pub repeat: Option<Repeat>,
    pub time: Option<String>,
    pub date: Option<String>,
    pub days: Option<Vec<i64>>,
    pub enabled: Option<bool>,
    pub follow_agent: Option<String>,
    pub follow_prompt: Option<String>,
    pub skip_unchanged: Option<bool>,
}

pub fn valid_time(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 5 || b[2] != b':' || !b.iter().enumerate().all(|(i, c)| i == 2 || c.is_ascii_digit()) {
        return false;
    }
    let hours = (b[0] - b'0') * 10 + (b[1] - b'0');
    let minutes = (b[3] - b'0') * 10 + (b[4] - b'0');
    hours < 24 && minutes < 60
}

pub fn valid_date(text: &str) -> bool {
    parse_day(text).is_some()
}

fn hour_minute(time: &str) -> Option<(i64, i64)> {
    if !valid_time(time) {
        return None;
    }
    Some((time[0..2].parse().ok()?, time[3..5].parse().ok()?))
}

pub fn next_occurrence(repeat: Repeat, time: &str, date: &str, days: &[i64], after: i64, utc_offset_minutes: i64) -> Option<i64> {
    let (hours, minutes) = hour_minute(time)?;
    let offset = utc_offset_minutes * MINUTE_MS;
    let local_after = after + offset;
    if repeat == Repeat::Hourly {
        let hour_start = local_after.div_euclid(HOUR_MS) * HOUR_MS;
        for step in 0..3 {
            let candidate = hour_start + step * HOUR_MS + minutes * MINUTE_MS - offset;
            if candidate > after {
                return Some(candidate);
            }
        }
        return None;
    }
    let today = local_after.div_euclid(DAY_MS);
    let first_days: Vec<i64> = if repeat == Repeat::Once {
        let (y, m, d) = parse_day(date)?;
        vec![days_from_civil(y, m, d)]
    } else {
        (0..15).map(|step| today + step).collect()
    };
    for day in first_days {
        let candidate = day * DAY_MS + hours * HOUR_MS + minutes * MINUTE_MS - offset;
        if candidate <= after {
            continue;
        }
        let weekday = weekday_of_days(day);
        if repeat == Repeat::Weekdays && (weekday == 0 || weekday == 6) {
            continue;
        }
        if repeat == Repeat::Weekly && !days.contains(&weekday) {
            continue;
        }
        return Some(candidate);
    }
    None
}

pub fn schedule_next(schedule: &Schedule, after: i64, utc_offset_minutes: i64) -> Option<i64> {
    next_occurrence(schedule.repeat, &schedule.time, &schedule.date, &schedule.days, after, utc_offset_minutes)
}

pub fn local_day(ms: i64, utc_offset_minutes: i64) -> String {
    let (y, m, d) = civil_from_days((ms + utc_offset_minutes * MINUTE_MS).div_euclid(DAY_MS));
    format!("{y:04}-{m:02}-{d:02}")
}

fn check(schedule: &Schedule) -> Result<()> {
    if schedule.name.is_empty() {
        bail!("A schedule needs a name.");
    }
    if schedule.agent.is_empty() {
        bail!("A schedule needs an agent.");
    }
    if schedule.prompt.is_empty() && !schedule.agent.starts_with("team:") {
        bail!("A schedule needs a prompt.");
    }
    if !valid_time(&schedule.time) {
        bail!("The time must be HH:MM, for example 09:30.");
    }
    if schedule.repeat == Repeat::Once && !valid_date(&schedule.date) {
        bail!("A one time schedule needs a real date as YYYY-MM-DD.");
    }
    if schedule.repeat == Repeat::Weekly && (schedule.days.is_empty() || schedule.days.iter().any(|d| !(0..=6).contains(d))) {
        bail!("A weekly schedule needs days from 0 (Sunday) to 6 (Saturday).");
    }
    Ok(())
}

fn clean_days(days: &[i64]) -> Vec<i64> {
    let mut out = days.to_vec();
    out.sort();
    out.dedup();
    out
}

fn json_or_empty<T: Serialize>(value: &Option<T>) -> Result<String> {
    Ok(match value {
        Some(v) => serde_json::to_string(v)?,
        None => String::new(),
    })
}

fn from_json<T: for<'de> Deserialize<'de>>(text: &str) -> Option<T> {
    if text.is_empty() { None } else { serde_json::from_str(text).ok() }
}

fn to_schedule(row: &Row) -> rusqlite::Result<Schedule> {
    let next: i64 = row.get("next_run")?;
    Ok(Schedule {
        id: row.get("id")?,
        name: row.get("name")?,
        agent: row.get("agent")?,
        folder: row.get("folder")?,
        prompt: row.get("prompt")?,
        job: from_json(&row.get::<_, String>("job")?),
        repeat: Repeat::parse(&row.get::<_, String>("repeat")?).unwrap_or(Repeat::Once),
        time: row.get("time")?,
        date: row.get("date")?,
        days: from_json(&row.get::<_, String>("days")?).unwrap_or_default(),
        enabled: row.get::<_, i64>("enabled")? == 1,
        follow_agent: row.get("follow_agent")?,
        follow_prompt: row.get("follow_prompt")?,
        next_run: (next > 0).then_some(next),
        last_run: from_json(&row.get::<_, String>("last_run")?),
        skip_unchanged: row.get::<_, i64>("skip_unchanged")? == 1,
        skipped: from_json(&row.get::<_, String>("skipped")?),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn save(conn: &Connection, s: &Schedule) -> Result<()> {
    conn.execute(
        "UPDATE schedules SET name = ?2, agent = ?3, folder = ?4, prompt = ?5, job = ?6, repeat = ?7, time = ?8, date = ?9, days = ?10,
           enabled = ?11, follow_agent = ?12, follow_prompt = ?13, next_run = ?14, last_run = ?15, skip_unchanged = ?16, skipped = ?17, updated_at = ?18
         WHERE id = ?1",
        params![
            s.id,
            s.name,
            s.agent,
            s.folder,
            s.prompt,
            json_or_empty(&s.job)?,
            s.repeat.as_str(),
            s.time,
            s.date,
            serde_json::to_string(&s.days)?,
            flag(s.enabled),
            s.follow_agent,
            s.follow_prompt,
            s.next_run.unwrap_or(0),
            json_or_empty(&s.last_run)?,
            flag(s.skip_unchanged),
            json_or_empty(&s.skipped)?,
            s.updated_at,
        ],
    )?;
    Ok(())
}

pub fn get_schedule(conn: &Connection, id: &str) -> Result<Option<Schedule>> {
    Ok(conn.query_row("SELECT * FROM schedules WHERE id = ?1", params![id], to_schedule).optional()?)
}

fn must_schedule(conn: &Connection, id: &str) -> Result<Schedule> {
    match get_schedule(conn, id)? {
        Some(s) => Ok(s),
        None => bail!("No schedule {id}."),
    }
}

pub fn list_schedules(conn: &Connection) -> Result<Vec<Schedule>> {
    let mut stmt = conn.prepare("SELECT * FROM schedules ORDER BY number")?;
    let rows = stmt.query_map([], to_schedule)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn add_schedule(conn: &Connection, input: &ScheduleInput, utc_offset_minutes: i64) -> Result<Schedule> {
    let at = now_ms();
    let mut schedule = Schedule {
        id: String::new(),
        name: clean_line(&input.name, 120),
        agent: clean_line(&input.agent, 80),
        folder: input.folder.trim().to_string(),
        prompt: clean_text(&input.prompt, PROMPT_MAX),
        job: input.job.clone(),
        repeat: input.repeat,
        time: input.time.trim().to_string(),
        date: input.date.trim().to_string(),
        days: clean_days(&input.days),
        enabled: input.enabled,
        follow_agent: clean_line(&input.follow_agent, 80),
        follow_prompt: clean_text(&input.follow_prompt, PROMPT_MAX),
        next_run: None,
        last_run: None,
        skip_unchanged: input.skip_unchanged,
        skipped: None,
        created_at: at,
        updated_at: at,
    };
    check(&schedule)?;
    schedule.next_run = if schedule.enabled { schedule_next(&schedule, at, utc_offset_minutes) } else { None };
    let number = next_number(conn, COUNTER)?;
    schedule.id = format!("s{number}");
    conn.execute(
        "INSERT INTO schedules (id, number, name, agent, prompt, repeat, time, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![schedule.id, number, schedule.name, schedule.agent, schedule.prompt, schedule.repeat.as_str(), schedule.time, at],
    )?;
    save(conn, &schedule)?;
    Ok(schedule)
}

pub fn update_schedule(conn: &Connection, id: &str, patch: &SchedulePatch, utc_offset_minutes: i64) -> Result<Schedule> {
    let mut s = must_schedule(conn, id)?;
    let when_changed = patch.repeat.is_some() || patch.time.is_some() || patch.date.is_some() || patch.days.is_some() || patch.enabled.is_some();
    if let Some(v) = &patch.name {
        s.name = clean_line(v, 120);
    }
    if let Some(v) = &patch.agent {
        s.agent = clean_line(v, 80);
    }
    if let Some(v) = &patch.folder {
        s.folder = v.trim().to_string();
    }
    if let Some(v) = &patch.prompt {
        s.prompt = clean_text(v, PROMPT_MAX);
    }
    if let Some(v) = &patch.job {
        s.job = v.clone();
    }
    if let Some(v) = patch.repeat {
        s.repeat = v;
    }
    if let Some(v) = &patch.time {
        s.time = v.trim().to_string();
    }
    if let Some(v) = &patch.date {
        s.date = v.trim().to_string();
    }
    if let Some(v) = &patch.days {
        s.days = clean_days(v);
    }
    if let Some(v) = patch.enabled {
        s.enabled = v;
    }
    if let Some(v) = &patch.follow_agent {
        s.follow_agent = clean_line(v, 80);
    }
    if let Some(v) = &patch.follow_prompt {
        s.follow_prompt = clean_text(v, PROMPT_MAX);
    }
    if let Some(v) = patch.skip_unchanged {
        s.skip_unchanged = v;
    }
    check(&s)?;
    s.updated_at = now_ms();
    if when_changed {
        s.next_run = if s.enabled { schedule_next(&s, s.updated_at, utc_offset_minutes) } else { None };
    }
    save(conn, &s)?;
    Ok(s)
}

pub fn set_schedule_enabled(conn: &Connection, id: &str, on: bool, utc_offset_minutes: i64) -> Result<Schedule> {
    update_schedule(conn, id, &SchedulePatch { enabled: Some(on), ..SchedulePatch::default() }, utc_offset_minutes)
}

pub fn remove_schedule(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM schedules WHERE id = ?1", params![id])? == 1)
}

pub fn due_schedules(conn: &Connection, now: i64) -> Result<Vec<Schedule>> {
    let mut stmt = conn.prepare("SELECT * FROM schedules WHERE enabled = 1 AND next_run > 0 AND next_run <= ?1 ORDER BY next_run, number")?;
    let rows = stmt.query_map(params![now], to_schedule)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn record_run(conn: &Connection, id: &str, run: &LastRun, utc_offset_minutes: i64) -> Result<Schedule> {
    let mut s = must_schedule(conn, id)?;
    s.last_run = Some(run.clone());
    s.updated_at = now_ms();
    s.next_run = if s.repeat == Repeat::Once {
        s.enabled = false;
        None
    } else if s.enabled {
        schedule_next(&s, run.at.max(s.next_run.unwrap_or(0)), utc_offset_minutes)
    } else {
        None
    };
    save(conn, &s)?;
    Ok(s)
}

pub fn update_last_run(conn: &Connection, id: &str, run: &LastRun) -> Result<Schedule> {
    let mut s = must_schedule(conn, id)?;
    s.last_run = Some(run.clone());
    s.updated_at = now_ms();
    save(conn, &s)?;
    Ok(s)
}

pub fn note_skipped(conn: &Connection, id: &str, at: i64, utc_offset_minutes: i64) -> Result<Schedule> {
    let mut s = must_schedule(conn, id)?;
    let count = s.skipped.map(|k| k.count).unwrap_or(0) + 1;
    s.skipped = Some(Skipped { at, count });
    s.updated_at = now_ms();
    s.next_run = if s.enabled && s.repeat != Repeat::Once { schedule_next(&s, at.max(s.next_run.unwrap_or(0)), utc_offset_minutes) } else { None };
    save(conn, &s)?;
    Ok(s)
}

pub fn set_next_run(conn: &Connection, id: &str, next: Option<i64>) -> Result<Schedule> {
    let mut s = must_schedule(conn, id)?;
    s.next_run = next;
    s.updated_at = now_ms();
    save(conn, &s)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;

    fn at(y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
        days_from_civil(y, m, d) * DAY_MS + h * HOUR_MS + min * MINUTE_MS
    }

    fn input(repeat: Repeat, time: &str) -> ScheduleInput {
        ScheduleInput {
            name: "Morning check".into(),
            agent: "codex".into(),
            folder: "/p".into(),
            prompt: "Check the build".into(),
            job: None,
            repeat,
            time: time.into(),
            date: String::new(),
            days: Vec::new(),
            enabled: true,
            follow_agent: String::new(),
            follow_prompt: String::new(),
            skip_unchanged: false,
        }
    }

    #[test]
    fn next_times() {
        let sat = at(2026, 11, 21, 10, 0);
        assert_eq!(next_occurrence(Repeat::Daily, "09:30", "", &[], sat, 0), Some(at(2026, 11, 22, 9, 30)));
        assert_eq!(next_occurrence(Repeat::Daily, "11:00", "", &[], sat, 0), Some(at(2026, 11, 21, 11, 0)));
        assert_eq!(next_occurrence(Repeat::Weekdays, "09:00", "", &[], sat, 0), Some(at(2026, 11, 23, 9, 0)));
        assert_eq!(next_occurrence(Repeat::Weekly, "09:00", "", &[3], sat, 0), Some(at(2026, 11, 25, 9, 0)));
        assert_eq!(next_occurrence(Repeat::Hourly, "00:15", "", &[], sat, 0), Some(at(2026, 11, 21, 10, 15)));
        assert_eq!(next_occurrence(Repeat::Once, "08:00", "2026-11-21", &[], sat, 0), None);
        assert_eq!(next_occurrence(Repeat::Once, "08:00", "2026-12-01", &[], sat, 0), Some(at(2026, 12, 1, 8, 0)));
        assert_eq!(next_occurrence(Repeat::Daily, "09:00", "", &[], sat, 480), Some(at(2026, 11, 22, 1, 0)));
        assert_eq!(next_occurrence(Repeat::Daily, "9:00", "", &[], sat, 0), None);
        assert_eq!(local_day(at(2026, 11, 21, 20, 0), 480), "2026-11-22");
    }

    #[test]
    fn team_schedules_may_have_no_prompt() {
        let conn = temp_db("schedules-team");
        let mut team = input(Repeat::Daily, "09:00");
        team.prompt = String::new();
        assert_eq!(add_schedule(&conn, &team, 0).unwrap_err().to_string(), "A schedule needs a prompt.");
        team.agent = "team:pr".into();
        let saved = add_schedule(&conn, &team, 0).unwrap();
        assert_eq!(saved.prompt, "");
        let patch = SchedulePatch { prompt: Some(String::new()), ..SchedulePatch::default() };
        assert_eq!(update_schedule(&conn, &saved.id, &patch, 0).unwrap().prompt, "");
    }

    #[test]
    fn old_schedule_job_keeps_its_public_fields() {
        let conn = temp_db("schedules-legacy");
        let saved = add_schedule(&conn, &input(Repeat::Daily, "09:00"), 0).unwrap();
        conn.execute(
            "UPDATE schedules SET job = ?1 WHERE id = ?2",
            (r#"{"space":"s1","repo":"shop","boss_project":"previous"}"#, &saved.id),
        ).unwrap();
        let job = get_schedule(&conn, &saved.id).unwrap().unwrap().job.unwrap();
        assert_eq!(job.space, "s1");
        assert_eq!(job.repo, "shop");
        assert_eq!(serde_json::to_value(job).unwrap(), serde_json::json!({"space":"s1","repo":"shop"}));
    }

    #[test]
    fn add_due_run() {
        let conn = temp_db("schedules");
        assert!(add_schedule(&conn, &input(Repeat::Daily, "25:00"), 0).is_err());
        assert!(add_schedule(&conn, &input(Repeat::Once, "09:00"), 0).is_err());
        assert!(add_schedule(&conn, &input(Repeat::Weekly, "09:00"), 0).is_err());
        let daily = add_schedule(&conn, &input(Repeat::Daily, "09:00"), 0).unwrap();
        assert_eq!(daily.id, "s1");
        let next = daily.next_run.unwrap();
        assert!(next > now_ms());
        assert!(due_schedules(&conn, now_ms()).unwrap().is_empty());
        assert_eq!(due_schedules(&conn, next).unwrap().len(), 1);
        let run = LastRun { at: next, result: RunResult::Started, terminal: "m7".into(), report: String::new(), done_at: None, open_tasks: None, follow_terminal: String::new(), team_run: String::new(), error: String::new(), marker: String::new() };
        let after = record_run(&conn, "s1", &run, 0).unwrap();
        assert_eq!(after.next_run, Some(next + DAY_MS));
        assert_eq!(after.last_run.unwrap().terminal, "m7");
        let skipped = note_skipped(&conn, "s1", next + DAY_MS, 0).unwrap();
        assert_eq!(skipped.skipped.unwrap().count, 1);
        assert_eq!(skipped.next_run, Some(next + 2 * DAY_MS));
        let off = set_schedule_enabled(&conn, "s1", false, 0).unwrap();
        assert_eq!(off.next_run, None);
        let mut once = input(Repeat::Once, "09:00");
        once.date = "2099-01-01".into();
        let one = add_schedule(&conn, &once, 0).unwrap();
        let done = record_run(&conn, &one.id, &run, 0).unwrap();
        assert!(!done.enabled);
        assert_eq!(list_schedules(&conn).unwrap().len(), 2);
        let renamed = update_schedule(&conn, "s1", &SchedulePatch { name: Some("Evening".into()), ..SchedulePatch::default() }, 0).unwrap();
        assert_eq!(renamed.name, "Evening");
        assert!(remove_schedule(&conn, "s1").unwrap());
        assert!(get_schedule(&conn, "s1").unwrap().is_none());
    }
}
