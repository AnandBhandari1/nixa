use super::*;
use crate::db::{migrate, migrate_to, open_memory, temp_db};

const OLD_SHAPE: &str = r#"
ALTER TABLE tasks ADD COLUMN boss_id TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN boss_number TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN boss_url TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN boss_project TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN boss_status TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN boss_assignee TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN boss_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN boss_seen_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN boss_detail_at INTEGER NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX tasks_boss ON tasks (source, boss_id) WHERE boss_id <> '';
ALTER TABLE task_comments ADD COLUMN boss_comment_id TEXT NOT NULL DEFAULT '';
CREATE UNIQUE INDEX task_comments_boss ON task_comments (task_id, boss_comment_id) WHERE boss_comment_id <> '';
CREATE TABLE task_dropped (
    source TEXT NOT NULL,
    boss_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    at INTEGER NOT NULL,
    PRIMARY KEY (source, boss_id)
);
ALTER TABLE spaces ADD COLUMN boss_project TEXT NOT NULL DEFAULT '';
INSERT INTO tasks (id, title, status, source, created_at, updated_at, boss_id, boss_number, boss_url, boss_project, boss_status, boss_assignee, boss_hash, boss_seen_at, boss_detail_at)
    VALUES (1, 'Pulled', 'todo', 'boss_task', 1, 1, 'b1', 'T-1', 'https://example.com/b1', 'Shop', 'open', 'sam', 'h1', 5, 6);
INSERT INTO tasks (id, title, status, source, created_at, updated_at) VALUES (2, 'Plain', 'todo', 'manual', 1, 1);
INSERT INTO task_comments (task_id, at, author_kind, kind, text, boss_comment_id) VALUES (1, 1, 'boss', 'boss', 'from outside', 'c1');
INSERT INTO task_comments (task_id, at, author_kind, kind, text) VALUES (2, 1, 'nixa', 'note', 'plain note');
INSERT INTO task_dropped (source, boss_id, title, at) VALUES ('boss_task', 'b9', 'Gone', 3);
INSERT INTO spaces (space_id, name, created_at, updated_at, boss_project) VALUES ('s1', 'shop', 1, 1, 'Shop');
INSERT INTO spaces (space_id, name, created_at, updated_at) VALUES ('s2', 'site', 1, 1);
"#;

fn old_data(name: &str) -> Connection {
    let conn = Connection::open(std::env::temp_dir().join("nixa-store-tests").join(format!("{name}-{}-{}.db", std::process::id(), crate::time::now_ms()))).unwrap();
    migrate_to(&conn, 4).unwrap();
    conn.execute_batch(OLD_SHAPE).unwrap();
    conn
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn rows(conn: &Connection) -> (i64, i64, i64) {
    (count(conn, "SELECT COUNT(*) FROM tasks"), count(conn, "SELECT COUNT(*) FROM task_comments"), count(conn, "SELECT COUNT(*) FROM spaces"))
}

#[test]
fn fresh_data_gets_link_fields_once() {
    let conn = open_memory().unwrap();
    for column in ["link_source", "link_id", "link_number", "link_url", "link_status", "link_seen_at"] {
        assert!(has_column(&conn, "tasks", column).unwrap(), "{column}");
    }
    assert!(has_column(&conn, "task_comments", "link_comment_id").unwrap());
    assert!(has_column(&conn, "task_dropped", "link_id").unwrap());
    assert!(!has_table(&conn, OLD_DROPPED).unwrap());
    migrate(&conn).unwrap();
}

#[test]
fn old_data_keeps_its_columns_and_rows() {
    let conn = temp_db("links-dir");
    drop(conn);
    let conn = old_data("links-old");
    let before = rows(&conn);
    migrate(&conn).unwrap();
    assert_eq!(rows(&conn), before);
    assert_eq!(before, (2, 2, 2));
    for column in ["boss_id", "boss_number", "boss_url", "boss_project", "boss_status", "boss_assignee", "boss_hash", "boss_seen_at", "boss_detail_at"] {
        assert!(has_column(&conn, "tasks", column).unwrap(), "{column}");
    }
    assert!(has_column(&conn, "task_comments", "boss_comment_id").unwrap());
    assert!(has_column(&conn, "spaces", "boss_project").unwrap());
    assert!(has_column(&conn, OLD_DROPPED, "boss_id").unwrap());
    assert_eq!(count(&conn, &format!("SELECT COUNT(*) FROM {OLD_DROPPED}")), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks WHERE boss_id = 'b1' AND boss_project = 'Shop'"), 1);
    let task = crate::tasks::get_task(&conn, 2).unwrap().unwrap();
    assert!(task.link_id.is_empty());
    let changed = crate::tasks::update_task(&conn, 2, &crate::tasks::TaskPatch { link_status: Some("open".into()), ..Default::default() }).unwrap();
    assert_eq!(changed.link_status, "open");
    assert!(crate::tasks::remove_task(&conn, 2).unwrap());
    migrate(&conn).unwrap();
}

#[test]
fn legacy_tasks_comments_and_spaces_still_load() {
    let conn = old_data("links-public");
    migrate(&conn).unwrap();
    let shown = crate::tasks::show_task(&conn, 1).unwrap().unwrap();
    assert_eq!(shown.task.source, crate::tasks::TaskSource::Manual);
    assert_eq!(shown.task.group, crate::tasks::TaskGroup::Project);
    assert!(shown.task.link_source.is_empty() && shown.task.link_id.is_empty());
    assert_eq!(shown.comments[0].text, "from outside");
    assert_eq!(shown.comments[0].kind, crate::tasks::CommentKind::Note);
    assert_eq!(crate::spaces::get_space(&conn, "s1").unwrap().unwrap().name, "shop");
    assert!(!has_table(&conn, "task_boss").unwrap());
    assert!(!has_table(&conn, "space_boss").unwrap());
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM task_dropped"), 0);
    assert!(!crate::tasks::was_dropped(&conn, crate::tasks::TaskSource::Manual, "b9").unwrap());
}
