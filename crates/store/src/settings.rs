use crate::time::now_ms;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<Value>> {
    let text: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| row.get(0))
        .optional()?;
    Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
}

pub fn set_setting(conn: &Connection, key: &str, value: &Value) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, serde_json::to_string(value)?, now_ms()],
    )?;
    Ok(())
}

pub fn remove_setting(conn: &Connection, key: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM settings WHERE key = ?1", params![key])? == 1)
}

pub fn all_settings(conn: &Connection) -> Result<BTreeMap<String, Value>> {
    settings_with_prefix(conn, "")
}

pub fn settings_with_prefix(conn: &Connection, prefix: &str) -> Result<BTreeMap<String, Value>> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings ORDER BY key")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (key, text) = row?;
        if key.starts_with(prefix)
            && let Ok(value) = serde_json::from_str(&text)
        {
            out.insert(key, value);
        }
    }
    Ok(out)
}

pub fn setting_text(conn: &Connection, key: &str, default: &str) -> Result<String> {
    Ok(get_setting(conn, key)?
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| default.to_string()))
}

pub fn setting_bool(conn: &Connection, key: &str, default: bool) -> Result<bool> {
    Ok(get_setting(conn, key)?.and_then(|v| v.as_bool()).unwrap_or(default))
}

pub fn setting_i64(conn: &Connection, key: &str, default: i64) -> Result<i64> {
    Ok(get_setting(conn, key)?.and_then(|v| v.as_i64()).unwrap_or(default))
}

pub fn set_setting_text(conn: &Connection, key: &str, value: &str) -> Result<()> {
    set_setting(conn, key, &Value::String(value.to_string()))
}

pub fn set_setting_bool(conn: &Connection, key: &str, value: bool) -> Result<()> {
    set_setting(conn, key, &Value::Bool(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;
    use serde_json::json;

    #[test]
    fn key_values() {
        let conn = temp_db("settings");
        assert!(get_setting(&conn, "voice.name").unwrap().is_none());
        set_setting(&conn, "voice.name", &json!("Toby")).unwrap();
        set_setting(&conn, "voice.rate", &json!(180)).unwrap();
        set_setting_bool(&conn, "mic.on", true).unwrap();
        set_setting(&conn, "voice.name", &json!("Harper")).unwrap();
        assert_eq!(setting_text(&conn, "voice.name", "x").unwrap(), "Harper");
        assert_eq!(setting_i64(&conn, "voice.rate", 0).unwrap(), 180);
        assert!(setting_bool(&conn, "mic.on", false).unwrap());
        assert!(setting_bool(&conn, "missing", true).unwrap());
        assert_eq!(settings_with_prefix(&conn, "voice.").unwrap().len(), 2);
        assert_eq!(all_settings(&conn).unwrap().len(), 3);
        assert!(remove_setting(&conn, "mic.on").unwrap());
        assert!(!remove_setting(&conn, "mic.on").unwrap());
    }
}
