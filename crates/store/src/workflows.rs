use crate::time::now_ms;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

pub const SQL: &str = r#"
CREATE TABLE IF NOT EXISTS workflows (
    name TEXT PRIMARY KEY,
    body TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS workflow_backups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    body TEXT NOT NULL,
    at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS workflow_backups_by_name ON workflow_backups (name, id);
CREATE TABLE IF NOT EXISTS workflow_runs (
    task_id INTEGER PRIMARY KEY,
    workflow TEXT NOT NULL,
    space_id TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL,
    flow TEXT NOT NULL,
    run TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS workflow_runs_by_state ON workflow_runs (state, updated_at);
CREATE TABLE IF NOT EXISTS workflow_reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    number INTEGER NOT NULL,
    box TEXT NOT NULL,
    text TEXT NOT NULL,
    at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS workflow_reports_by_task ON workflow_reports (task_id, number);
"#;

pub const KEEP_BACKUPS: usize = 20;
pub const KEEP_RUNS: usize = 200;
pub const BODY_MAX: usize = 2_000_000;
pub const REPORT_MAX: usize = 200_000;
pub const ACTIVE_STATES: [&str; 3] = ["running", "paused", "waiting"];
pub const RUN_STATES: [&str; 6] = ["running", "paused", "waiting", "done", "failed", "cancelled"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWorkflow {
    pub name: String,
    pub body: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub task_id: i64,
    pub workflow: String,
    pub space_id: String,
    pub state: String,
    pub flow: String,
    pub run: String,
    pub started_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    pub id: i64,
    pub task_id: i64,
    pub number: i64,
    #[serde(rename = "box")]
    pub box_id: String,
    pub text: String,
    pub at: i64,
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    first_ok && name.len() <= 60 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn need_name(name: &str) -> Result<()> {
    if !valid_name(name) {
        bail!("Workflow name: small letters, numbers and dashes, up to 60.");
    }
    Ok(())
}

fn need_state(state: &str) -> Result<()> {
    if !RUN_STATES.contains(&state) {
        bail!("A workflow run state is one of: {}.", RUN_STATES.join(", "));
    }
    Ok(())
}

fn to_saved(row: &Row) -> rusqlite::Result<SavedWorkflow> {
    Ok(SavedWorkflow { name: row.get("name")?, body: row.get("body")?, updated_at: row.get("updated_at")? })
}

fn to_run(row: &Row) -> rusqlite::Result<WorkflowRun> {
    Ok(WorkflowRun {
        task_id: row.get("task_id")?,
        workflow: row.get("workflow")?,
        space_id: row.get("space_id")?,
        state: row.get("state")?,
        flow: row.get("flow")?,
        run: row.get("run")?,
        started_at: row.get("started_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn to_report(row: &Row) -> rusqlite::Result<RunReport> {
    Ok(RunReport {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        number: row.get("number")?,
        box_id: row.get("box")?,
        text: row.get("text")?,
        at: row.get("at")?,
    })
}

pub fn get_workflow(conn: &Connection, name: &str) -> Result<Option<SavedWorkflow>> {
    Ok(conn.query_row("SELECT * FROM workflows WHERE name = ?1", params![name], to_saved).optional()?)
}

pub fn list_workflows(conn: &Connection) -> Result<Vec<SavedWorkflow>> {
    let mut stmt = conn.prepare("SELECT * FROM workflows ORDER BY name")?;
    let rows = stmt.query_map([], to_saved)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn backup(conn: &Connection, name: &str, at: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO workflow_backups (name, body, at) SELECT name, body, ?2 FROM workflows WHERE name = ?1",
        params![name, at],
    )?;
    conn.execute(
        "DELETE FROM workflow_backups WHERE name = ?1 AND id NOT IN (SELECT id FROM workflow_backups WHERE name = ?1 ORDER BY id DESC LIMIT ?2)",
        params![name, KEEP_BACKUPS as i64],
    )?;
    Ok(())
}

pub fn put_workflow(conn: &Connection, name: &str, body: &str) -> Result<SavedWorkflow> {
    need_name(name)?;
    if body.len() > BODY_MAX {
        bail!("A workflow is at most {BODY_MAX} bytes.");
    }
    let at = now_ms();
    let tx = conn.unchecked_transaction()?;
    backup(&tx, name, at)?;
    tx.execute(
        "INSERT INTO workflows (name, body, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(name) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at",
        params![name, body, at],
    )?;
    tx.commit()?;
    Ok(SavedWorkflow { name: name.to_string(), body: body.to_string(), updated_at: at })
}

pub fn remove_workflow(conn: &Connection, name: &str) -> Result<bool> {
    need_name(name)?;
    let tx = conn.unchecked_transaction()?;
    backup(&tx, name, now_ms())?;
    let gone = tx.execute("DELETE FROM workflows WHERE name = ?1", params![name])? == 1;
    tx.commit()?;
    Ok(gone)
}

pub fn can_undo(conn: &Connection, name: &str) -> Result<bool> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM workflow_backups WHERE name = ?1", params![name], |row| row.get(0))?;
    Ok(count > 0)
}

pub fn undo_workflow(conn: &Connection, name: &str) -> Result<Option<SavedWorkflow>> {
    need_name(name)?;
    let tx = conn.unchecked_transaction()?;
    let last: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, body FROM workflow_backups WHERE name = ?1 ORDER BY id DESC LIMIT 1",
            params![name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((id, body)) = last else {
        return Ok(None);
    };
    let at = now_ms();
    tx.execute(
        "INSERT INTO workflows (name, body, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(name) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at",
        params![name, body, at],
    )?;
    tx.execute("DELETE FROM workflow_backups WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok(Some(SavedWorkflow { name: name.to_string(), body, updated_at: at }))
}

pub fn get_run(conn: &Connection, task_id: i64) -> Result<Option<WorkflowRun>> {
    Ok(conn.query_row("SELECT * FROM workflow_runs WHERE task_id = ?1", params![task_id], to_run).optional()?)
}

pub fn list_runs(conn: &Connection, active_only: bool) -> Result<Vec<WorkflowRun>> {
    let sql = if active_only {
        "SELECT * FROM workflow_runs WHERE state IN ('running', 'paused', 'waiting') ORDER BY task_id"
    } else {
        "SELECT * FROM workflow_runs ORDER BY task_id"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], to_run)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn start_run(conn: &Connection, run: &WorkflowRun) -> Result<WorkflowRun> {
    need_name(&run.workflow)?;
    need_state(&run.state)?;
    let at = now_ms();
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM workflow_reports WHERE task_id = ?1", params![run.task_id])?;
    tx.execute(
        "INSERT INTO workflow_runs (task_id, workflow, space_id, state, flow, run, started_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(task_id) DO UPDATE SET workflow = excluded.workflow, space_id = excluded.space_id, state = excluded.state,
           flow = excluded.flow, run = excluded.run, started_at = excluded.started_at, updated_at = excluded.updated_at",
        params![run.task_id, run.workflow, run.space_id, run.state, run.flow, run.run, run.started_at, at],
    )?;
    tx.commit()?;
    match get_run(conn, run.task_id)? {
        Some(saved) => Ok(saved),
        None => bail!("The workflow run of task #{} was not saved.", run.task_id),
    }
}

pub fn set_run(conn: &Connection, task_id: i64, state: &str, run: &str) -> Result<()> {
    need_state(state)?;
    let changed = conn.execute(
        "UPDATE workflow_runs SET state = ?2, run = ?3, updated_at = ?4 WHERE task_id = ?1",
        params![task_id, state, run, now_ms()],
    )?;
    if changed != 1 {
        bail!("Task #{task_id} has no workflow run.");
    }
    Ok(())
}

pub fn remove_run(conn: &Connection, task_id: i64) -> Result<bool> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM workflow_reports WHERE task_id = ?1", params![task_id])?;
    let gone = tx.execute("DELETE FROM workflow_runs WHERE task_id = ?1", params![task_id])? == 1;
    tx.commit()?;
    Ok(gone)
}

pub fn prune_runs(conn: &Connection, keep: usize) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT task_id FROM workflow_runs WHERE state NOT IN ('running', 'paused', 'waiting') ORDER BY updated_at DESC, task_id DESC",
    )?;
    let finished = stmt.query_map([], |row| row.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let old: Vec<i64> = finished.into_iter().skip(keep).collect();
    for task_id in &old {
        remove_run(conn, *task_id)?;
    }
    Ok(old)
}

pub fn add_report(conn: &Connection, task_id: i64, box_id: &str, text: &str) -> Result<RunReport> {
    let text: String = if text.len() > REPORT_MAX {
        let mut cut = REPORT_MAX;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text[..cut].to_string()
    } else {
        text.to_string()
    };
    let number: i64 = conn.query_row(
        "SELECT COALESCE(MAX(number), 0) + 1 FROM workflow_reports WHERE task_id = ?1",
        params![task_id],
        |row| row.get(0),
    )?;
    let at = now_ms();
    conn.execute(
        "INSERT INTO workflow_reports (task_id, number, box, text, at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![task_id, number, box_id, text, at],
    )?;
    Ok(RunReport { id: conn.last_insert_rowid(), task_id, number, box_id: box_id.to_string(), text, at })
}

pub fn get_report(conn: &Connection, id: i64) -> Result<Option<RunReport>> {
    Ok(conn.query_row("SELECT * FROM workflow_reports WHERE id = ?1", params![id], to_report).optional()?)
}

pub fn run_reports(conn: &Connection, task_id: i64) -> Result<Vec<RunReport>> {
    let mut stmt = conn.prepare("SELECT * FROM workflow_reports WHERE task_id = ?1 ORDER BY number")?;
    let rows = stmt.query_map(params![task_id], to_report)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;

    fn db(name: &str) -> Connection {
        let conn = temp_db(name);
        conn.execute_batch(SQL).unwrap();
        conn
    }

    fn run(task_id: i64, state: &str) -> WorkflowRun {
        WorkflowRun {
            task_id,
            workflow: "build-review".into(),
            space_id: "nixa".into(),
            state: state.into(),
            flow: "{}".into(),
            run: "{}".into(),
            started_at: 1,
            updated_at: 0,
        }
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_name("build-review"));
        assert!(valid_name("a1"));
        assert!(!valid_name("-a"));
        assert!(!valid_name("Big"));
        assert!(!valid_name(""));
        assert!(!valid_name(&"a".repeat(61)));
        let conn = db("wf-names");
        assert!(put_workflow(&conn, "Bad Name", "{}").is_err());
    }

    #[test]
    fn put_backup_undo_and_remove() {
        let conn = db("wf-crud");
        assert!(!can_undo(&conn, "flow").unwrap());
        put_workflow(&conn, "flow", "one").unwrap();
        assert!(!can_undo(&conn, "flow").unwrap());
        put_workflow(&conn, "flow", "two").unwrap();
        assert!(can_undo(&conn, "flow").unwrap());
        assert_eq!(get_workflow(&conn, "flow").unwrap().unwrap().body, "two");
        let back = undo_workflow(&conn, "flow").unwrap().unwrap();
        assert_eq!(back.body, "one");
        assert!(!can_undo(&conn, "flow").unwrap());
        assert!(undo_workflow(&conn, "flow").unwrap().is_none());
        assert!(remove_workflow(&conn, "flow").unwrap());
        assert!(get_workflow(&conn, "flow").unwrap().is_none());
        assert_eq!(undo_workflow(&conn, "flow").unwrap().unwrap().body, "one");
        put_workflow(&conn, "other", "x").unwrap();
        let names: Vec<String> = list_workflows(&conn).unwrap().into_iter().map(|w| w.name).collect();
        assert_eq!(names, vec!["flow".to_string(), "other".to_string()]);
    }

    #[test]
    fn keeps_twenty_backups() {
        let conn = db("wf-keep");
        for n in 0..30 {
            put_workflow(&conn, "flow", &format!("v{n}")).unwrap();
        }
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM workflow_backups", [], |r| r.get(0)).unwrap();
        assert_eq!(count, KEEP_BACKUPS as i64);
        assert_eq!(undo_workflow(&conn, "flow").unwrap().unwrap().body, "v28");
    }

    #[test]
    fn runs_and_reports() {
        let conn = db("wf-runs");
        assert!(start_run(&conn, &run(1, "nope")).is_err());
        start_run(&conn, &run(1, "running")).unwrap();
        start_run(&conn, &run(2, "running")).unwrap();
        assert!(set_run(&conn, 9, "done", "{}").is_err());
        set_run(&conn, 2, "done", "{\"x\":1}").unwrap();
        assert_eq!(get_run(&conn, 2).unwrap().unwrap().run, "{\"x\":1}");
        assert_eq!(list_runs(&conn, true).unwrap().len(), 1);
        assert_eq!(list_runs(&conn, false).unwrap().len(), 2);
        let first = add_report(&conn, 1, "n2", "report one").unwrap();
        let second = add_report(&conn, 1, "n3", &"é".repeat(REPORT_MAX)).unwrap();
        assert_eq!((first.number, second.number), (1, 2));
        assert!(second.text.len() <= REPORT_MAX);
        assert_eq!(get_report(&conn, first.id).unwrap().unwrap().text, "report one");
        assert_eq!(run_reports(&conn, 1).unwrap().len(), 2);
        start_run(&conn, &run(1, "running")).unwrap();
        assert!(run_reports(&conn, 1).unwrap().is_empty());
    }

    #[test]
    fn prune_keeps_active_runs() {
        let conn = db("wf-prune");
        for id in 1..=5 {
            start_run(&conn, &run(id, "running")).unwrap();
            if id != 3 {
                set_run(&conn, id, "done", "{}").unwrap();
            }
        }
        add_report(&conn, 1, "n1", "old").unwrap();
        let gone = prune_runs(&conn, 2).unwrap();
        assert_eq!(gone.len(), 2);
        assert!(get_run(&conn, 3).unwrap().is_some());
        assert_eq!(list_runs(&conn, false).unwrap().len(), 3);
    }
}
