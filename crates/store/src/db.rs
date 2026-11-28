use anyhow::{Context, Result};
use rusqlite::{Connection, ErrorCode, Transaction, TransactionBehavior, params};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DB_FILE: &str = "nixa.db";
const BUSY_MS: u64 = 2000;

const V1: &str = r#"
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL,
    space_id TEXT NOT NULL DEFAULT '',
    space_name TEXT NOT NULL DEFAULT '',
    assigned_kind TEXT NOT NULL DEFAULT '',
    assigned TEXT NOT NULL DEFAULT '',
    claimed_by TEXT NOT NULL DEFAULT '',
    terminal_id TEXT NOT NULL DEFAULT '',
    claimed_at INTEGER NOT NULL DEFAULT 0,
    active_at INTEGER NOT NULL DEFAULT 0,
    needs_user INTEGER NOT NULL DEFAULT 0,
    blocked_on TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL,
    task_group TEXT NOT NULL DEFAULT 'project',
    created_by TEXT NOT NULL DEFAULT '',
    created_terminal TEXT NOT NULL DEFAULT '',
    priority TEXT NOT NULL DEFAULT '',
    due_date TEXT NOT NULL DEFAULT '',
    repo TEXT NOT NULL DEFAULT '',
    branch TEXT NOT NULL DEFAULT '',
    commit_sha TEXT NOT NULL DEFAULT '',
    pr_url TEXT NOT NULL DEFAULT '',
    base TEXT NOT NULL DEFAULT '',
    uncommitted TEXT NOT NULL DEFAULT '',
    then_member TEXT NOT NULL DEFAULT '',
    workflow TEXT NOT NULL DEFAULT '',
    archived_at INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    closed_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX tasks_status ON tasks (status);
CREATE INDEX tasks_space ON tasks (space_id);
CREATE INDEX tasks_terminal ON tasks (terminal_id);
CREATE INDEX tasks_group ON tasks (task_group);

CREATE TABLE task_comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    at INTEGER NOT NULL,
    author_kind TEXT NOT NULL,
    author TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    terminal_id TEXT NOT NULL DEFAULT '',
    space_id TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    links TEXT NOT NULL DEFAULT '{}',
    extra TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX task_comments_task ON task_comments (task_id, at);

CREATE TABLE task_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE spaces (
    space_id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    folder TEXT NOT NULL DEFAULT '',
    auto_start INTEGER NOT NULL DEFAULT 0,
    max_doing INTEGER NOT NULL DEFAULT 8,
    locked INTEGER NOT NULL DEFAULT 0,
    repos TEXT NOT NULL DEFAULT '[]',
    decision_mode TEXT NOT NULL DEFAULT 'global',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE file_claims (
    task INTEGER NOT NULL,
    terminal TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    path TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (task, terminal, path)
);
CREATE INDEX file_claims_at ON file_claims (at);

CREATE TABLE custom_agents (
    id TEXT PRIMARY KEY,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    purpose TEXT NOT NULL,
    instructions TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT '',
    job TEXT NOT NULL DEFAULT '',
    workflow TEXT NOT NULL DEFAULT '',
    start TEXT NOT NULL DEFAULT '',
    off INTEGER NOT NULL DEFAULT 0,
    migrated_from TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL DEFAULT '',
    session TEXT NOT NULL DEFAULT '',
    space TEXT NOT NULL DEFAULT '',
    space_name TEXT NOT NULL DEFAULT '',
    folder TEXT NOT NULL DEFAULT '',
    terminal TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    last_used INTEGER NOT NULL,
    by TEXT NOT NULL DEFAULT ''
);
CREATE INDEX custom_agents_terminal ON custom_agents (terminal);

CREATE TABLE agent_map_rows (
    id TEXT PRIMARY KEY,
    pos INTEGER NOT NULL,
    section TEXT NOT NULL DEFAULT '',
    job TEXT NOT NULL,
    member TEXT NOT NULL DEFAULT '',
    backup INTEGER NOT NULL DEFAULT 0,
    agent TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    effort TEXT NOT NULL DEFAULT '',
    pick TEXT NOT NULL DEFAULT '',
    fallback TEXT NOT NULL DEFAULT '',
    place TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    off INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE memory (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    day TEXT NOT NULL,
    text TEXT NOT NULL,
    held TEXT NOT NULL DEFAULT '',
    at INTEGER NOT NULL
);

CREATE TABLE schedules (
    id TEXT PRIMARY KEY,
    number INTEGER NOT NULL,
    name TEXT NOT NULL,
    agent TEXT NOT NULL,
    folder TEXT NOT NULL DEFAULT '',
    prompt TEXT NOT NULL,
    job TEXT NOT NULL DEFAULT '',
    repeat TEXT NOT NULL,
    time TEXT NOT NULL,
    date TEXT NOT NULL DEFAULT '',
    days TEXT NOT NULL DEFAULT '[]',
    enabled INTEGER NOT NULL DEFAULT 1,
    follow_agent TEXT NOT NULL DEFAULT '',
    follow_prompt TEXT NOT NULL DEFAULT '',
    next_run INTEGER NOT NULL DEFAULT 0,
    last_run TEXT NOT NULL DEFAULT '',
    skip_unchanged INTEGER NOT NULL DEFAULT 0,
    skipped TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE privacy_list (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    at INTEGER NOT NULL,
    UNIQUE (kind, value)
);

CREATE TABLE tool_gaps (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    tool TEXT NOT NULL DEFAULT '',
    asked TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 1,
    first_at INTEGER NOT NULL,
    last_at INTEGER NOT NULL,
    UNIQUE (kind, source, tool, reason)
);

CREATE TABLE counters (
    name TEXT PRIMARY KEY,
    value INTEGER NOT NULL
);
"#;

const V2: &str = r#"
ALTER TABLE spaces ADD COLUMN color INTEGER NOT NULL DEFAULT 0;
ALTER TABLE spaces ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
"#;

enum Step {
    Sql(&'static str),
    Run(fn(&Connection) -> Result<()>),
}

const MIGRATIONS: &[&[Step]] = &[
    &[Step::Sql(V1)],
    &[Step::Sql(V2)],
    &[Step::Sql(crate::models::SQL), Step::Sql(crate::workflows::SQL)],
    &[Step::Sql(crate::journal::JOURNAL_SQL), Step::Sql(crate::journal::SCORES_SQL)],
    &[Step::Run(crate::links::migrate)],
    &[Step::Run(crate::words::migrate)],
    &[Step::Sql(crate::gaps::TASK_SQL)],
    &[Step::Sql(crate::gaps::SPACE_SQL)],
    &[Step::Sql("ALTER TABLE custom_agents ADD COLUMN skills TEXT NOT NULL DEFAULT '';")],
];

pub fn db_path() -> PathBuf {
    nixa_protocol::paths::data_dir().join(DB_FILE)
}

pub fn open_default() -> Result<Connection> {
    open(&db_path())
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = private_parent(path) {
        nixa_protocol::paths::ensure_private_dir(dir)
            .with_context(|| format!("could not make the folder {}", dir.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    let conn = Connection::open(path).with_context(|| format!("could not open {}", path.display()))?;
    conn.busy_timeout(Duration::from_millis(BUSY_MS))?;
    retry_busy(|| Ok(conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?)).context("could not turn on WAL")?;
    conn.pragma_update(None, "secure_delete", "ON").context("could not turn on secure delete")?;
    retry_busy(|| migrate(&conn)).context("could not migrate the database")?;
    Ok(conn)
}

pub fn wipe_free(conn: &Connection) {
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
}

fn is_busy(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(e, _)) if matches!(e.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
        )
    })
}

fn retry_busy(mut work: impl FnMut() -> Result<()>) -> Result<()> {
    let until = Instant::now() + Duration::from_millis(BUSY_MS);
    loop {
        match work() {
            Err(error) if is_busy(&error) && Instant::now() < until => std::thread::sleep(Duration::from_millis(5)),
            other => return other,
        }
    }
}

fn private_parent(path: &Path) -> Option<&Path> {
    path.parent().filter(|dir| !dir.as_os_str().is_empty())
}

pub fn open_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.busy_timeout(Duration::from_millis(BUSY_MS))?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    migrate_to(conn, MIGRATIONS.len())
}

pub(crate) fn migrate_to(conn: &Connection, last: usize) -> Result<()> {
    if usize::try_from(schema_version(conn)?).unwrap_or(0) >= last {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let have = usize::try_from(schema_version(&tx)?).unwrap_or(0);
    for (index, step) in MIGRATIONS.iter().enumerate().take(last).skip(have) {
        for part in step.iter() {
            match part {
                Step::Sql(sql) => tx.execute_batch(sql).map_err(anyhow::Error::from),
                Step::Run(run) => run(&tx),
            }
            .with_context(|| format!("migration {} failed", index + 1))?;
        }
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn next_number(conn: &Connection, name: &str) -> Result<i64> {
    let value: i64 = conn.query_row(
        "INSERT INTO counters (name, value) VALUES (?1, 1) ON CONFLICT(name) DO UPDATE SET value = value + 1 RETURNING value",
        params![name],
        |row| row.get(0),
    )?;
    Ok(value)
}

pub(crate) fn bump_number(conn: &Connection, name: &str, at_least: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO counters (name, value) VALUES (?1, ?2) ON CONFLICT(name) DO UPDATE SET value = MAX(value, excluded.value)",
        params![name, at_least],
    )?;
    Ok(())
}

pub(crate) fn flag(on: bool) -> i64 {
    i64::from(on)
}

pub(crate) fn list_text(items: &[String]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string())
}

pub(crate) fn text_list(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Vec<serde_json::Value>>(text)
        .map(|items| {
            items
                .into_iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn temp_db(name: &str) -> Connection {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join("nixa-store-tests");
    let n = COUNT.fetch_add(1, Ordering::SeqCst);
    let path = dir.join(format!("{name}-{}-{}-{n}.db", std::process::id(), crate::time::now_ms()));
    open(&path).expect("temp db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_once_and_keeps_version() {
        let conn = temp_db("db");
        assert_eq!(schema_version(&conn).unwrap(), MIGRATIONS.len() as i64);
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), MIGRATIONS.len() as i64);
    }

    #[test]
    fn fresh_schema_has_only_public_names() {
        let conn = open_memory().unwrap();
        let mut stmt = conn.prepare("SELECT name || ' ' || COALESCE(sql, '') FROM sqlite_master").unwrap();
        let schema: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();
        let text = schema.join("\n").to_lowercase();
        assert!(!text.contains("boss"), "{text}");
        for word in ["link_source", "link_id", "link_number", "link_url", "link_status", "link_seen_at", "link_comment_id", "tasks_link", "task_dropped"] {
            assert!(text.contains(word), "{word}");
        }
    }

    #[test]
    fn journal_and_scores_tables_are_made() {
        let conn = temp_db("journal");
        for table in ["decisions", "journal_meta", "changes", "scores"] {
            let found: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", params![table], |row| row.get(0)).unwrap();
            assert_eq!(found, 1, "{table}");
        }
        conn.execute_batch(crate::journal::JOURNAL_SQL).unwrap();
        conn.execute_batch(crate::journal::SCORES_SQL).unwrap();
    }

    #[test]
    fn file_is_owner_only() {
        let dir = std::env::temp_dir().join("nixa-store-tests");
        let path = dir.join(format!("perm-{}-{}.db", std::process::id(), crate::time::now_ms()));
        let _conn = open(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
    }

    #[test]
    fn many_threads_open_a_new_db_at_once() {
        let dir = std::env::temp_dir().join("nixa-store-tests");
        for round in 0..5 {
            let path = dir.join(format!("race-{}-{}-{round}.db", std::process::id(), crate::time::now_ms()));
            let workers: Vec<_> = (0..16)
                .map(|_| {
                    let path = path.clone();
                    std::thread::spawn(move || {
                        let conn = open(&path)?;
                        conn.execute("INSERT INTO settings (key, value, updated_at) VALUES (?1, '1', 0)", params![format!("k{:?}", std::thread::current().id())])?;
                        schema_version(&conn)
                    })
                })
                .collect();
            for worker in workers {
                assert_eq!(worker.join().unwrap().unwrap(), MIGRATIONS.len() as i64);
            }
            let conn = open(&path).unwrap();
            let rows: i64 = conn.query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0)).unwrap();
            assert_eq!(rows, 16);
        }
    }

    #[test]
    fn bare_file_name_has_no_folder_to_make() {
        assert_eq!(private_parent(Path::new("nixa.db")), None);
        assert_eq!(private_parent(Path::new("data/nixa.db")), Some(Path::new("data")));
        assert_eq!(private_parent(Path::new("/tmp/nixa.db")), Some(Path::new("/tmp")));
    }

    #[test]
    fn counters_count_up() {
        let conn = open_memory().unwrap();
        assert_eq!(next_number(&conn, "x").unwrap(), 1);
        assert_eq!(next_number(&conn, "x").unwrap(), 2);
        bump_number(&conn, "x", 10).unwrap();
        assert_eq!(next_number(&conn, "x").unwrap(), 11);
    }
}
