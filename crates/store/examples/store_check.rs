use nixa_store::{db_path, open_default, schema_version};

const TABLES: [&str; 14] = [
    "tasks",
    "task_comments",
    "task_meta",
    "spaces",
    "file_claims",
    "custom_agents",
    "agent_map_rows",
    "settings",
    "memory",
    "schedules",
    "privacy_list",
    "tool_gaps",
    "counters",
    "task_dropped",
];

fn main() -> anyhow::Result<()> {
    let conn = open_default()?;
    println!("db {}", db_path().display());
    println!("schema version {}", schema_version(&conn)?);
    for table in TABLES {
        let count: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))?;
        println!("{table} {count}");
    }
    Ok(())
}
