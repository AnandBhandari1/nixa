use crate::db::flag;
use crate::settings::{get_setting, set_setting};
use crate::text::{clean_line, clip};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FALLOVER_ORDER: [&str; 6] = ["claude", "codex", "omp", "agy", "opencode", "cursor"];
pub const PLACES: [&str; 6] = ["Tab", "Tab, same folder", "Tab, worktree", "Tab, worktree if big", "Split pane", "Split pane, read-only"];
pub const DEFAULT_CRITERIA: [&str; 4] = [
    "Capabilities only rank models. They never block a start: first a model that meets all needs, then the one that meets most needs, then any listed model.",
    "When the asked agent has no free model, the next installed agent in the fall over order takes the job. Nixa says which one took it and why.",
    "Code review goes to a different company than the one whose model wrote the code.",
    "Failed twice: use the row's backup. Backup failed too: the master does the job.",
];
const CRITERIA_KEY: &str = "agent_map.criteria";
const FALLOVER_KEY: &str = "agent_map.fallover";
const AT_KEY: &str = "agent_map.at";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentMapRow {
    pub id: String,
    pub section: String,
    pub job: String,
    pub member: String,
    pub backup: bool,
    pub agent: String,
    pub model: String,
    pub effort: String,
    pub pick: String,
    pub fallback: String,
    pub place: String,
    pub notes: String,
    pub off: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMap {
    pub version: u32,
    pub rows: Vec<AgentMapRow>,
    pub criteria: Vec<String>,
    pub fallover: Vec<String>,
    pub at: String,
}

fn word(text: &str, max: usize) -> String {
    clip(text.trim(), max)
}

fn clean_id(text: &str) -> String {
    word(text, 40).chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect()
}

pub fn clean_row(row: &AgentMapRow) -> Option<AgentMapRow> {
    let job = clean_line(&row.job, 400);
    if job.is_empty() {
        return None;
    }
    Some(AgentMapRow {
        id: clean_id(&row.id),
        section: clean_line(&row.section, 80),
        job,
        member: clean_line(&row.member, 80),
        backup: row.backup,
        agent: word(&row.agent, 40),
        model: word(&row.model, 160),
        effort: word(&row.effort, 40),
        pick: clean_line(&row.pick, 400),
        fallback: clean_line(&row.fallback, 400),
        place: clean_line(&row.place, 80),
        notes: clean_line(&row.notes, 800),
        off: row.off,
    })
}

fn to_row(row: &Row) -> rusqlite::Result<AgentMapRow> {
    Ok(AgentMapRow {
        id: row.get("id")?,
        section: row.get("section")?,
        job: row.get("job")?,
        member: row.get("member")?,
        backup: row.get::<_, i64>("backup")? == 1,
        agent: row.get("agent")?,
        model: row.get("model")?,
        effort: row.get("effort")?,
        pick: row.get("pick")?,
        fallback: row.get("fallback")?,
        place: row.get("place")?,
        notes: row.get("notes")?,
        off: row.get::<_, i64>("off")? == 1,
    })
}

fn write_row(conn: &Connection, row: &AgentMapRow, pos: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO agent_map_rows (id, pos, section, job, member, backup, agent, model, effort, pick, fallback, place, notes, off)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(id) DO UPDATE SET pos = excluded.pos, section = excluded.section, job = excluded.job, member = excluded.member,
           backup = excluded.backup, agent = excluded.agent, model = excluded.model, effort = excluded.effort, pick = excluded.pick,
           fallback = excluded.fallback, place = excluded.place, notes = excluded.notes, off = excluded.off",
        params![
            row.id,
            pos,
            row.section,
            row.job,
            row.member,
            flag(row.backup),
            row.agent,
            row.model,
            row.effort,
            row.pick,
            row.fallback,
            row.place,
            row.notes,
            flag(row.off),
        ],
    )?;
    Ok(())
}

fn renumber(conn: &Connection, rows: &[AgentMapRow]) -> Result<()> {
    for (pos, row) in rows.iter().enumerate() {
        conn.execute("UPDATE agent_map_rows SET pos = ?1 WHERE id = ?2", params![pos as i64, row.id])?;
    }
    Ok(())
}

pub fn map_rows(conn: &Connection) -> Result<Vec<AgentMapRow>> {
    let mut stmt = conn.prepare("SELECT * FROM agent_map_rows ORDER BY pos, id")?;
    let rows = stmt.query_map([], to_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get_row(conn: &Connection, id: &str) -> Result<Option<AgentMapRow>> {
    Ok(conn.query_row("SELECT * FROM agent_map_rows WHERE id = ?1", params![id], to_row).optional()?)
}

fn string_list(value: Option<Value>) -> Option<Vec<String>> {
    value.and_then(|v| v.as_array().map(|items| items.iter().filter_map(|i| i.as_str().map(str::to_string)).collect()))
}

pub fn criteria(conn: &Connection) -> Result<Vec<String>> {
    Ok(string_list(get_setting(conn, CRITERIA_KEY)?).unwrap_or_else(|| DEFAULT_CRITERIA.iter().map(|s| s.to_string()).collect()))
}

pub fn after(order: &[String], old: &str) -> Vec<String> {
    match order.iter().position(|o| o == old) {
        Some(at) => order[at + 1..].iter().chain(order[..at].iter()).cloned().collect(),
        None => order.to_vec(),
    }
}

pub fn fallover(conn: &Connection) -> Result<Vec<String>> {
    Ok(string_list(get_setting(conn, FALLOVER_KEY)?).unwrap_or_else(|| FALLOVER_ORDER.iter().map(|s| s.to_string()).collect()))
}

pub fn set_criteria(conn: &Connection, items: &[String]) -> Result<Vec<String>> {
    let list: Vec<String> = items.iter().map(|i| clean_line(i, 600)).filter(|i| !i.is_empty()).collect();
    set_setting(conn, CRITERIA_KEY, &serde_json::to_value(&list)?)?;
    Ok(list)
}

pub fn set_fallover(conn: &Connection, items: &[String]) -> Result<Vec<String>> {
    let mut list: Vec<String> = Vec::new();
    for item in items.iter().map(|i| word(i, 40)) {
        if !item.is_empty() && !list.contains(&item) {
            list.push(item);
        }
    }
    set_setting(conn, FALLOVER_KEY, &serde_json::to_value(&list)?)?;
    Ok(list)
}

pub fn agent_map(conn: &Connection) -> Result<AgentMap> {
    let at = get_setting(conn, AT_KEY)?.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    Ok(AgentMap { version: 1, rows: map_rows(conn)?, criteria: criteria(conn)?, fallover: fallover(conn)?, at })
}

pub fn agent_map_json(conn: &Connection) -> Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(&agent_map(conn)?)?))
}

fn new_row_id(rows: &[AgentMapRow]) -> String {
    let mut n = rows.len() + 1;
    while rows.iter().any(|r| r.id == format!("r{n}")) {
        n += 1;
    }
    format!("r{n}")
}

pub fn add_row(conn: &Connection, row: &AgentMapRow) -> Result<AgentMapRow> {
    let Some(mut clean) = clean_row(row) else {
        bail!("An agent map row needs a job.");
    };
    let rows = map_rows(conn)?;
    if clean.id.is_empty() || rows.iter().any(|r| r.id == clean.id) {
        clean.id = new_row_id(&rows);
    }
    write_row(conn, &clean, rows.len() as i64)?;
    Ok(clean)
}

pub fn update_row(conn: &Connection, id: &str, row: &AgentMapRow) -> Result<AgentMapRow> {
    let Some(before) = get_row(conn, id)? else {
        bail!("No row {id} in the agent map.");
    };
    let Some(mut clean) = clean_row(row) else {
        bail!("An agent map row needs a job.");
    };
    clean.id = before.id;
    let pos = map_rows(conn)?.iter().position(|r| r.id == clean.id).unwrap_or(0);
    write_row(conn, &clean, pos as i64)?;
    Ok(clean)
}

pub fn remove_row(conn: &Connection, id: &str) -> Result<bool> {
    let gone = conn.execute("DELETE FROM agent_map_rows WHERE id = ?1", params![id])? == 1;
    renumber(conn, &map_rows(conn)?)?;
    Ok(gone)
}

pub fn move_row(conn: &Connection, id: &str, to: usize) -> Result<Vec<AgentMapRow>> {
    let mut rows = map_rows(conn)?;
    let Some(from) = rows.iter().position(|r| r.id == id) else {
        bail!("No row {id} in the agent map.");
    };
    let row = rows.remove(from);
    rows.insert(to.min(rows.len()), row);
    let tx = conn.unchecked_transaction()?;
    renumber(&tx, &rows)?;
    tx.commit()?;
    Ok(rows)
}

pub fn import_agent_map(conn: &Connection, source: &str, at: &str) -> Result<AgentMap> {
    let raw: Value = serde_json::from_str(source)?;
    let Some(items) = raw.get("rows").and_then(Value::as_array) else {
        bail!("The agent map needs a rows list.");
    };
    let mut rows: Vec<AgentMapRow> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let Ok(parsed) = serde_json::from_value::<AgentMapRow>(item.clone()) else {
            continue;
        };
        let Some(mut row) = clean_row(&parsed) else {
            continue;
        };
        if row.id.is_empty() {
            row.id = format!("r{}", index + 1);
        }
        let base = row.id.clone();
        let mut n = 2;
        while rows.iter().any(|r| r.id == row.id) {
            row.id = format!("{base}-{n}");
            n += 1;
        }
        rows.push(row);
    }
    let text_items = |key: &str| raw.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|i| i.as_str().map(str::to_string)).collect::<Vec<_>>());
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM agent_map_rows", [])?;
    for (pos, row) in rows.iter().enumerate() {
        write_row(&tx, row, pos as i64)?;
    }
    if let Some(items) = text_items("criteria") {
        set_criteria(&tx, &items)?;
    }
    if let Some(items) = text_items("fallover") {
        set_fallover(&tx, &items)?;
    }
    let stamp = raw.get("at").and_then(Value::as_str).map(|s| word(s, 40)).filter(|s| !s.is_empty()).unwrap_or_else(|| at.to_string());
    set_setting(&tx, AT_KEY, &Value::String(stamp))?;
    tx.commit()?;
    agent_map(conn)
}

pub fn rows_for_job(conn: &Connection, job: &str) -> Result<Vec<AgentMapRow>> {
    let wanted = job.trim().to_lowercase();
    Ok(map_rows(conn)?
        .into_iter()
        .filter(|r| !r.off && (r.job.to_lowercase() == wanted || r.member.to_lowercase() == wanted || r.id.to_lowercase() == wanted))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;

    fn row(job: &str) -> AgentMapRow {
        AgentMapRow { job: job.into(), agent: "codex".into(), ..AgentMapRow::default() }
    }

    #[test]
    fn rows_crud_and_move() {
        let conn = temp_db("agent-map");
        let a = add_row(&conn, &row("Build code")).unwrap();
        let b = add_row(&conn, &row("Review code")).unwrap();
        let c = add_row(&conn, &AgentMapRow { id: "r1".into(), ..row("Test code") }).unwrap();
        assert_eq!((a.id.as_str(), b.id.as_str(), c.id.as_str()), ("r1", "r2", "r3"));
        assert!(add_row(&conn, &row(" ")).is_err());
        let moved = move_row(&conn, "r3", 0).unwrap();
        assert_eq!(moved.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["r3", "r1", "r2"]);
        let changed = update_row(&conn, "r1", &AgentMapRow { model: "big".into(), ..row("Build code fast") }).unwrap();
        assert_eq!(changed.id, "r1");
        assert_eq!(map_rows(&conn).unwrap()[1].model, "big");
        assert!(remove_row(&conn, "r3").unwrap());
        assert_eq!(map_rows(&conn).unwrap()[0].id, "r1");
        assert_eq!(rows_for_job(&conn, "review code").unwrap().len(), 1);
        assert_eq!(criteria(&conn).unwrap().len(), DEFAULT_CRITERIA.len());
        assert_eq!(set_fallover(&conn, &["codex".into(), "codex".into(), "omp".into()]).unwrap(), vec!["codex".to_string(), "omp".to_string()]);
        assert_eq!(agent_map(&conn).unwrap().fallover, vec!["codex".to_string(), "omp".to_string()]);
    }

    #[test]
    fn import_json() {
        let conn = temp_db("agent-map-import");
        let source = r#"{"rows":[{"id":"x","job":"Build"},{"id":"x","job":"Fix"},{"job":""},{"job":"Plan"}],"criteria":["one"],"at":"2026-11-22"}"#;
        let map = import_agent_map(&conn, source, "now").unwrap();
        assert_eq!(map.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["x", "x-2", "r4"]);
        assert_eq!(map.criteria, vec!["one".to_string()]);
        assert_eq!(map.at, "2026-11-22");
        assert!(import_agent_map(&conn, "{}", "now").is_err());
        assert!(agent_map_json(&conn).unwrap().contains("\"version\": 1"));
    }

    #[test]
    fn the_fall_over_order_starts_after_the_old_engine() {
        let order: Vec<String> = ["claude", "codex", "omp", "agy"].iter().map(|s| s.to_string()).collect();
        assert_eq!(after(&order, "omp"), vec!["agy", "claude", "codex"]);
        assert_eq!(after(&order, "agy"), vec!["claude", "codex", "omp"]);
        assert_eq!(after(&order, "kimi"), order);
    }
}
