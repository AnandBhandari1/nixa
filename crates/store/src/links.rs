use anyhow::Result;
use rusqlite::{params, Connection};

pub const OLD_DROPPED: &str = "task_dropped_v1";

const LINK_SQL: &str = r#"
ALTER TABLE tasks ADD COLUMN link_source TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN link_id TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN link_number TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN link_url TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN link_status TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN link_seen_at INTEGER NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX tasks_link ON tasks (link_source, link_id, source) WHERE link_id <> '';
ALTER TABLE task_comments ADD COLUMN link_comment_id TEXT NOT NULL DEFAULT '';
CREATE UNIQUE INDEX task_comments_link ON task_comments (task_id, link_comment_id) WHERE link_comment_id <> '';
CREATE TABLE IF NOT EXISTS task_dropped (
    source TEXT NOT NULL,
    link_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    at INTEGER NOT NULL,
    PRIMARY KEY (source, link_id)
);
"#;

pub fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    let found: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", params![name], |row| row.get(0))?;
    Ok(found > 0)
}

pub fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let found: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2", params![table, column], |row| row.get(0))?;
    Ok(found > 0)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    if has_table(conn, "task_dropped")? && !has_column(conn, "task_dropped", "link_id")? {
        conn.execute_batch(&format!("ALTER TABLE task_dropped RENAME TO {OLD_DROPPED}"))?;
    }
    conn.execute_batch(LINK_SQL)?;
    Ok(())
}

#[cfg(test)]
mod tests;
