use anyhow::Result;
use rusqlite::{Connection, params};
use serde_json::Value;

const NEW_FLAG: &str = "needs_user";
const NEW_WHO: &str = "user";
const OLD_CAP: &str = "\"cheap-drone\"";
const NEW_CAP: &str = "\"cheap\"";

const VALID_BLOCKED_ON: [&str; 4] = ["user", "decision", "task", "other"];
const VALID_AUTHOR_KINDS: [&str; 4] = ["user", "nixa", "agent", "system"];
const VALID_HANDOFF_KINDS: [&str; 3] = ["done", "pass", "user"];
const VALID_GATES: [&str; 4] = ["none", "checks", "deep-brain", "user"];
const VALID_WHO_KINDS: [&str; 8] = ["", "agent", "custom-agent", "map-row", "model", "deep-brain", "nixa", "user"];

pub fn migrate(conn: &Connection) -> Result<()> {
    if !has_column(conn, "tasks", NEW_FLAG)?
        && let Some(old_flag) = legacy_needs_column(conn)?
    {
        conn.execute_batch(&format!("ALTER TABLE tasks RENAME COLUMN {old_flag} TO {NEW_FLAG}"))?;
    }
    conn.execute(
        "UPDATE tasks SET blocked_on = ?1 WHERE blocked_on != '' AND blocked_on NOT IN (?2, ?3, ?4, ?5)",
        params![NEW_WHO, VALID_BLOCKED_ON[0], VALID_BLOCKED_ON[1], VALID_BLOCKED_ON[2], VALID_BLOCKED_ON[3]],
    )?;
    conn.execute(
        "UPDATE task_comments SET author_kind = ?1 WHERE author_kind != '' AND author_kind NOT IN (?2, ?3, ?4, ?5)",
        params![NEW_WHO, VALID_AUTHOR_KINDS[0], VALID_AUTHOR_KINDS[1], VALID_AUTHOR_KINDS[2], VALID_AUTHOR_KINDS[3]],
    )?;
    rewrite_json(conn, "task_meta", "key", "value", "key LIKE 'handoff:%'", handoff_words)?;
    if has_table(conn, "workflows")? {
        rewrite_json(conn, "workflows", "name", "body", "1", flow_words)?;
        rewrite_json(conn, "workflow_backups", "id", "body", "1", flow_words)?;
        rewrite_json(conn, "workflow_runs", "task_id", "flow", "1", flow_words)?;
        rewrite_json(conn, "workflow_runs", "task_id", "run", "1", flow_words)?;
    }
    if has_table(conn, "model_lists")? {
        for column in ["models", "details"] {
            conn.execute(&format!("UPDATE model_lists SET {column} = REPLACE({column}, ?1, ?2) WHERE instr({column}, ?1) > 0"), params![OLD_CAP, NEW_CAP])?;
        }
        conn.execute("UPDATE model_launches SET data = REPLACE(data, ?1, ?2) WHERE instr(data, ?1) > 0", params![OLD_CAP, NEW_CAP])?;
    }
    conn.execute("UPDATE settings SET value = REPLACE(value, ?1, ?2) WHERE key LIKE 'models.%' AND instr(value, ?1) > 0", params![OLD_CAP, NEW_CAP])?;
    Ok(())
}

fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    let found: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", params![table], |row| row.get(0))?;
    Ok(found == 1)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(1))?.collect::<rusqlite::Result<_>>()?;
    Ok(names)
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(table_columns(conn, table)?.iter().any(|name| name == column))
}

fn legacy_needs_column(conn: &Connection) -> Result<Option<String>> {
    Ok(table_columns(conn, "tasks")?
        .into_iter()
        .find(|name| {
            name.starts_with("needs_")
                && name != NEW_FLAG
                && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        }))
}

fn rewrite_json(conn: &Connection, table: &str, key: &str, column: &str, filter: &str, change: fn(&mut Value) -> bool) -> Result<()> {
    let rows: Vec<(rusqlite::types::Value, String)> = {
        let mut stmt = conn.prepare(&format!("SELECT {key}, {column} FROM {table} WHERE {filter}"))?;
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<_>>()?
    };
    for (id, text) in rows {
        let Ok(mut value) = serde_json::from_str::<Value>(&text) else { continue };
        if change(&mut value) {
            conn.execute(&format!("UPDATE {table} SET {column} = ?1 WHERE {key} = ?2"), params![value.to_string(), id])?;
        }
    }
    Ok(())
}

fn normalize_word(value: Option<&mut Value>, allowed: &[&str]) -> bool {
    match value {
        Some(word) if word.as_str().is_some_and(|s| !s.is_empty() && !allowed.contains(&s)) => {
            *word = Value::String(NEW_WHO.into());
            true
        }
        _ => false,
    }
}

fn handoff_words(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = normalize_word(map.get_mut("kind"), &VALID_HANDOFF_KINDS);
            for (_, inner) in map.iter_mut() {
                changed |= handoff_words(inner);
            }
            changed
        }
        Value::Array(items) => items.iter_mut().fold(false, |changed, item| handoff_words(item) | changed),
        _ => false,
    }
}

fn flow_words(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = normalize_word(map.get_mut("gate"), &VALID_GATES);
            if let Some(Value::Object(who)) = map.get_mut("who") {
                changed |= normalize_word(who.get_mut("kind"), &VALID_WHO_KINDS);
            }
            for (_, inner) in map.iter_mut() {
                changed |= flow_words(inner);
            }
            changed
        }
        Value::Array(items) => items.iter_mut().fold(false, |changed, item| flow_words(item) | changed),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
