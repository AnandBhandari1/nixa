use anyhow::{bail, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

pub const OWNER_TTL_MS: i64 = 30 * 60 * 1000;
pub const MAX_HOLD_FILES: usize = 50;
pub const BRIEF_HOLDERS: usize = 5;
pub const BRIEF_FILES: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHolder {
    pub task: i64,
    pub terminal: String,
    pub name: String,
    pub files: Vec<String>,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileClash {
    pub holder: FileHolder,
    pub file: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaimInput {
    pub task: i64,
    pub terminal: String,
    pub name: String,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimOutcome {
    pub holder: FileHolder,
    pub clashes: Vec<FileClash>,
}

pub fn clean_claim_path(path: &str) -> String {
    let mut text = path.replace('\\', "/");
    if let Some(rest) = text.strip_prefix("./") {
        text = rest.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    out.trim_end_matches('/').trim().to_string()
}

pub fn paths_overlap(a: &str, b: &str) -> bool {
    let one = clean_claim_path(a);
    let two = clean_claim_path(b);
    one == two || two.starts_with(&format!("{one}/")) || one.starts_with(&format!("{two}/"))
}

pub fn clash_text(clash: &FileClash) -> String {
    format!("clash: task #{} in terminal {} holds {}", clash.holder.task, clash.holder.terminal, clash.file)
}

pub fn holder_text(holder: &FileHolder) -> String {
    let name = if !holder.name.is_empty() && holder.name != holder.terminal { format!(" ({})", holder.name) } else { String::new() };
    format!("#{} in {}{}: {}", holder.task, holder.terminal, name, holder.files.join(", "))
}

pub fn clashes_of(holders: &[FileHolder], files: &[String]) -> Vec<FileClash> {
    let mut out = Vec::new();
    for holder in holders {
        for held in &holder.files {
            if files.iter().any(|file| paths_overlap(held, file)) {
                out.push(FileClash { holder: holder.clone(), file: held.clone() });
            }
        }
    }
    out
}

fn clip_files(files: &[String]) -> String {
    if files.len() <= BRIEF_FILES {
        files.join(", ")
    } else {
        format!("{} and {} more", files[..BRIEF_FILES].join(", "), files.len() - BRIEF_FILES)
    }
}

pub fn others_block(holders: &[FileHolder], exclude_task: Option<i64>, exclude_terminal: Option<&str>) -> Vec<String> {
    let shown: Vec<&FileHolder> = holders
        .iter()
        .filter(|h| Some(h.task) != exclude_task && Some(h.terminal.as_str()) != exclude_terminal)
        .take(BRIEF_HOLDERS)
        .collect();
    if shown.is_empty() {
        return vec!["others working now: none".to_string()];
    }
    let mut lines = vec!["others working now:".to_string()];
    lines.extend(shown.iter().map(|h| format!("- #{} in {}: {}", h.task, h.terminal, clip_files(&h.files))));
    lines
}

fn sweep(conn: &Connection, now: i64) -> Result<usize> {
    Ok(conn.execute("DELETE FROM file_claims WHERE at < ?1", params![now - OWNER_TTL_MS])?)
}

pub fn file_holders(conn: &Connection, now: i64) -> Result<Vec<FileHolder>> {
    let mut stmt = conn.prepare(
        "SELECT task, terminal, name, path, at FROM file_claims WHERE at >= ?1 ORDER BY at, task, terminal, rowid",
    )?;
    let rows = stmt.query_map(params![now - OWNER_TTL_MS], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?))
    })?;
    let mut holders: Vec<FileHolder> = Vec::new();
    for row in rows {
        let (task, terminal, name, path, at) = row?;
        match holders.iter_mut().find(|h| h.task == task && h.terminal == terminal) {
            Some(holder) => holder.files.push(path),
            None => holders.push(FileHolder { task, terminal, name, files: vec![path], at }),
        }
    }
    Ok(holders)
}

pub fn who_holds(conn: &Connection, path: &str, now: i64) -> Result<Vec<FileHolder>> {
    let wanted = clean_claim_path(path);
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    Ok(file_holders(conn, now)?
        .into_iter()
        .filter(|h| h.files.iter().any(|held| paths_overlap(held, &wanted)))
        .collect())
}

pub fn check_files(conn: &Connection, files: &[String], task: Option<i64>, terminal: Option<&str>, now: i64) -> Result<Vec<FileClash>> {
    let others: Vec<FileHolder> = file_holders(conn, now)?
        .into_iter()
        .filter(|h| !(Some(h.task) == task && Some(h.terminal.as_str()) == terminal))
        .collect();
    Ok(clashes_of(&others, files))
}

pub fn claim_files(conn: &Connection, input: &ClaimInput, now: i64) -> Result<ClaimOutcome> {
    let mut files: Vec<String> = Vec::new();
    for file in input.files.iter().map(|f| clean_claim_path(f)) {
        if !file.is_empty() && !files.contains(&file) {
            files.push(file);
        }
    }
    files.truncate(MAX_HOLD_FILES);
    if files.is_empty() {
        bail!("Name at least one file or folder to claim.");
    }
    let terminal = input.terminal.trim().to_string();
    if terminal.is_empty() {
        bail!("A file claim needs a terminal id.");
    }
    if input.task <= 0 {
        bail!("A file claim needs a task number.");
    }
    let name = if input.name.trim().is_empty() { terminal.clone() } else { input.name.trim().to_string() };
    let tx = conn.unchecked_transaction()?;
    sweep(&tx, now)?;
    let clashes = check_files(&tx, &files, Some(input.task), Some(&terminal), now)?;
    tx.execute("DELETE FROM file_claims WHERE task = ?1 AND terminal = ?2", params![input.task, terminal])?;
    for file in &files {
        tx.execute(
            "INSERT INTO file_claims (task, terminal, name, path, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![input.task, terminal, name, file, now],
        )?;
    }
    tx.commit()?;
    Ok(ClaimOutcome { holder: FileHolder { task: input.task, terminal, name, files, at: now }, clashes })
}

pub fn release_files(conn: &Connection, terminal: &str, task: Option<i64>, now: i64) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    sweep(&tx, now)?;
    let before = file_holders(&tx, now)?.len();
    match task {
        Some(task) => tx.execute("DELETE FROM file_claims WHERE terminal = ?1 AND task = ?2", params![terminal, task])?,
        None => tx.execute("DELETE FROM file_claims WHERE terminal = ?1", params![terminal])?,
    };
    let after = file_holders(&tx, now)?.len();
    tx.commit()?;
    Ok(before - after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;

    fn input(task: i64, terminal: &str, files: &[&str]) -> ClaimInput {
        ClaimInput { task, terminal: terminal.into(), name: String::new(), files: files.iter().map(|f| f.to_string()).collect() }
    }

    #[test]
    fn paths_clean_and_overlap() {
        assert_eq!(clean_claim_path("./src//a/"), "src/a");
        assert!(paths_overlap("src", "src/a.rs"));
        assert!(paths_overlap("src/a.rs", "./src/a.rs"));
        assert!(!paths_overlap("src/a", "src/ab"));
    }

    #[test]
    fn claim_check_release() {
        let conn = temp_db("claims");
        let now = 1_000_000_000;
        let first = claim_files(&conn, &input(1, "m1", &["crates/store", "crates/store/src/lib.rs"]), now).unwrap();
        assert!(first.clashes.is_empty());
        assert_eq!(first.holder.files.len(), 2);
        let second = claim_files(&conn, &input(2, "m2", &["crates/store/src/db.rs"]), now + 1).unwrap();
        assert_eq!(second.clashes.len(), 1);
        assert_eq!(clash_text(&second.clashes[0]), "clash: task #1 in terminal m1 holds crates/store");
        let again = claim_files(&conn, &input(1, "m1", &["docs"]), now + 2).unwrap();
        assert!(again.clashes.is_empty());
        assert_eq!(who_holds(&conn, "crates/store/src/db.rs", now + 3).unwrap().len(), 1);
        assert_eq!(check_files(&conn, &["docs/x.md".to_string()], None, None, now + 3).unwrap().len(), 1);
        assert_eq!(file_holders(&conn, now + 3).unwrap().len(), 2);
        let block = others_block(&file_holders(&conn, now + 3).unwrap(), Some(1), None);
        assert_eq!(block, vec!["others working now:".to_string(), "- #2 in m2: crates/store/src/db.rs".to_string()]);
        assert_eq!(release_files(&conn, "m1", None, now + 4).unwrap(), 1);
        assert_eq!(release_files(&conn, "m2", Some(9), now + 4).unwrap(), 0);
        assert!(claim_files(&conn, &input(1, "m1", &[" "]), now).is_err());
    }

    #[test]
    fn old_claims_expire() {
        let conn = temp_db("claims-ttl");
        claim_files(&conn, &input(1, "m1", &["a"]), 0).unwrap();
        assert_eq!(file_holders(&conn, OWNER_TTL_MS).unwrap().len(), 1);
        assert!(file_holders(&conn, OWNER_TTL_MS + 1).unwrap().is_empty());
        let late = claim_files(&conn, &input(2, "m2", &["a"]), OWNER_TTL_MS + 1).unwrap();
        assert!(late.clashes.is_empty());
    }
}
