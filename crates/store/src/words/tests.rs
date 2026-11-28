use super::*;
use crate::db;

const LEGACY_FLAG: &str = "needs_owner";
const LEGACY_WHO: &str = "owner";

const OLD_TABLES: &str = r#"
CREATE TABLE task_comments (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL, author_kind TEXT NOT NULL);
CREATE TABLE task_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE workflows (name TEXT PRIMARY KEY, body TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE workflow_backups (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, body TEXT NOT NULL, at INTEGER NOT NULL);
CREATE TABLE workflow_runs (task_id INTEGER PRIMARY KEY, flow TEXT NOT NULL, run TEXT NOT NULL);
CREATE TABLE model_lists (agent TEXT PRIMARY KEY, models TEXT NOT NULL DEFAULT '[]', details TEXT NOT NULL DEFAULT '{}');
CREATE TABLE model_launches (terminal TEXT PRIMARY KEY, data TEXT NOT NULL);
"#;

fn old_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!("CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, {LEGACY_FLAG} INTEGER NOT NULL DEFAULT 0, blocked_on TEXT NOT NULL DEFAULT '');")).unwrap();
    conn.execute_batch(OLD_TABLES).unwrap();
    let who = LEGACY_WHO;
    conn.execute("INSERT INTO tasks VALUES (1, 't', 1, ?1), (2, 'u', 0, 'task'), (3, 'v', 0, 'decision'), (4, 'w', 0, '')", params![who]).unwrap();
    conn.execute("INSERT INTO task_comments VALUES (1, 1, ?1), (2, 1, 'agent'), (3, 1, 'nixa'), (4, 1, 'system')", params![who]).unwrap();
    let handoff = format!(r#"{{"steps":1,"last":{{"kind":"{who}","why":"pick"}},"keep":{{"kind":"pass"}}}}"#);
    conn.execute("INSERT INTO task_meta VALUES ('handoff:1', ?1), ('note:1', ?2)", params![handoff, format!(r#"{{"kind":"{who}"}}"#)]).unwrap();
    let flow = format!(r#"{{"nodes":{{"n1":{{"gate":"{who}","who":{{"kind":"{who}"}}}},"n2":{{"gate":"checks","who":{{"kind":"agent","id":"{who}"}}}},"n3":{{"gate":"deep-brain","who":{{"kind":"custom-agent","id":"a1"}}}}}}}}"#);
    conn.execute("INSERT INTO workflows VALUES ('f', ?1, 0)", params![flow]).unwrap();
    conn.execute("INSERT INTO workflow_backups (name, body, at) VALUES ('f', ?1, 0)", params![flow]).unwrap();
    conn.execute("INSERT INTO workflow_runs VALUES (1, ?1, '{}')", params![flow]).unwrap();
    conn.execute("INSERT INTO model_lists VALUES ('codex', ?1, '{}')", params![format!(r#"[{{"capabilities":[{OLD_CAP},"speed"]}}]"#)]).unwrap();
    conn.execute("INSERT INTO model_launches VALUES ('m1', ?1)", params![format!(r#"{{"needs":[{OLD_CAP}]}}"#)]).unwrap();
    conn.execute("INSERT INTO settings VALUES ('models.ranking', ?1, 0)", params![format!(r#"{{"cap":[{OLD_CAP}]}}"#)]).unwrap();
    conn
}

#[test]
fn old_words_move_to_the_neutral_words_and_rows_stay() {
    let conn = old_db();
    migrate(&conn).unwrap();
    migrate(&conn).unwrap();
    assert!(has_column(&conn, "tasks", NEW_FLAG).unwrap());
    assert!(!has_column(&conn, "tasks", LEGACY_FLAG).unwrap());
    let rows: Vec<(i64, i64, String)> = conn
        .prepare("SELECT id, needs_user, blocked_on FROM tasks ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(rows, vec![(1, 1, "user".to_string()), (2, 0, "task".to_string()), (3, 0, "decision".to_string()), (4, 0, "".to_string())]);
    let kinds: Vec<String> = conn.prepare("SELECT author_kind FROM task_comments ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    assert_eq!(kinds, ["user", "agent", "nixa", "system"]);
    let meta = |key: &str| -> Value { serde_json::from_str(&conn.query_row("SELECT value FROM task_meta WHERE key = ?1", params![key], |r| r.get::<_, String>(0)).unwrap()).unwrap() };
    assert_eq!(meta("handoff:1")["last"]["kind"], "user");
    assert_eq!(meta("handoff:1")["keep"]["kind"], "pass");
    assert_eq!(meta("note:1")["kind"], LEGACY_WHO);
    for sql in ["SELECT body FROM workflows", "SELECT body FROM workflow_backups", "SELECT flow FROM workflow_runs"] {
        let flow: Value = serde_json::from_str(&conn.query_row(sql, [], |r| r.get::<_, String>(0)).unwrap()).unwrap();
        assert_eq!(flow["nodes"]["n1"]["gate"], "user", "{sql}");
        assert_eq!(flow["nodes"]["n1"]["who"]["kind"], "user", "{sql}");
        assert_eq!(flow["nodes"]["n2"]["gate"], "checks", "{sql}");
        assert_eq!(flow["nodes"]["n2"]["who"]["id"], LEGACY_WHO, "{sql}");
        assert_eq!(flow["nodes"]["n3"]["gate"], "deep-brain", "{sql}");
        assert_eq!(flow["nodes"]["n3"]["who"]["kind"], "custom-agent", "{sql}");
    }
    for sql in ["SELECT models FROM model_lists", "SELECT data FROM model_launches", "SELECT value FROM settings"] {
        let text: String = conn.query_row(sql, [], |r| r.get(0)).unwrap();
        assert!(text.contains(NEW_CAP) && !text.contains(OLD_CAP), "{sql}: {text}");
    }
}

#[test]
fn fresh_data_has_only_the_neutral_words() {
    let conn = db::open_memory().unwrap();
    assert!(has_column(&conn, "tasks", NEW_FLAG).unwrap());
    assert!(!has_column(&conn, "tasks", LEGACY_FLAG).unwrap());
    assert_eq!(legacy_needs_column(&conn).unwrap(), None);
}
