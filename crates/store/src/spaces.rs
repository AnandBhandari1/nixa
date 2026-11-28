use crate::db::{flag, list_text, text_list};
use crate::text::clean_text;
use crate::time::now_ms;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

pub const MAX_DOING: i64 = 8;
pub const MAX_DOING_TOP: i64 = 10;
pub const REPOS_MAX: usize = 12;
const REPO_CHARS: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    pub space_id: String,
    pub name: String,
    pub folder: String,
    pub auto_start: bool,
    pub max_doing: i64,
    pub locked: bool,
    pub repos: Vec<String>,
    pub color: i64,
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpacePatch {
    pub name: Option<String>,
    pub folder: Option<String>,
    pub auto_start: Option<bool>,
    pub max_doing: Option<i64>,
    pub locked: Option<bool>,
    pub repos: Option<Vec<String>>,
    pub color: Option<i64>,
    pub position: Option<i64>,
}

pub fn clean_repos(items: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let repo = crate::text::clip(item.trim().trim_end_matches('/'), REPO_CHARS);
        if repo.is_empty() || repo.split(['/', '\\']).any(|part| part == "..") || out.contains(&repo) {
            continue;
        }
        out.push(repo);
        if out.len() >= REPOS_MAX {
            break;
        }
    }
    out
}

fn to_space(row: &Row) -> rusqlite::Result<Space> {
    Ok(Space {
        space_id: row.get("space_id")?,
        name: row.get("name")?,
        folder: row.get("folder")?,
        auto_start: row.get::<_, i64>("auto_start")? == 1,
        max_doing: row.get::<_, i64>("max_doing")?.max(1),
        locked: row.get::<_, i64>("locked")? == 1,
        repos: clean_repos(&text_list(&row.get::<_, String>("repos")?)),
        color: row.get::<_, i64>("color")?.max(0),
        position: row.get("position")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn list_spaces(conn: &Connection) -> Result<Vec<Space>> {
    let mut stmt = conn.prepare("SELECT * FROM spaces ORDER BY position, name, space_id")?;
    let rows = stmt.query_map([], to_space)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get_space(conn: &Connection, space_id: &str) -> Result<Option<Space>> {
    Ok(conn.query_row("SELECT * FROM spaces WHERE space_id = ?1", params![space_id], to_space).optional()?)
}

pub fn set_space(conn: &Connection, space_id: &str, patch: &SpacePatch) -> Result<Space> {
    let id = clean_text(space_id, 120);
    if id.is_empty() {
        bail!("A space id is needed.");
    }
    let at = now_ms();
    let before = get_space(conn, &id)?.unwrap_or(Space {
        space_id: id.clone(),
        name: String::new(),
        folder: String::new(),
        auto_start: false,
        max_doing: MAX_DOING,
        locked: false,
        repos: Vec::new(),
        color: 0,
        position: 0,
        created_at: at,
        updated_at: at,
    });
    let text = |value: &Option<String>, old: &str| value.as_deref().map(|v| clean_text(v, 500)).unwrap_or_else(|| old.to_string());
    let after = Space {
        name: text(&patch.name, &before.name),
        folder: patch.folder.as_deref().map(|f| if f.trim().is_empty() { String::new() } else { f.to_string() }).unwrap_or(before.folder.clone()),
        auto_start: patch.auto_start.unwrap_or(before.auto_start),
        max_doing: patch.max_doing.map(|n| if n <= 0 { MAX_DOING } else { n.min(MAX_DOING_TOP) }).unwrap_or(before.max_doing),
        locked: patch.locked.unwrap_or(before.locked),
        repos: patch.repos.as_deref().map(clean_repos).unwrap_or(before.repos.clone()),
        color: patch.color.map(|n| n.max(0)).unwrap_or(before.color),
        position: patch.position.unwrap_or(before.position),
        updated_at: at,
        ..before
    };
    conn.execute(
        "INSERT INTO spaces (space_id, name, folder, auto_start, max_doing, locked, repos, created_at, updated_at, color, position)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(space_id) DO UPDATE SET name = excluded.name, folder = excluded.folder,
           auto_start = excluded.auto_start, max_doing = excluded.max_doing, locked = excluded.locked, repos = excluded.repos,
           updated_at = excluded.updated_at, color = excluded.color, position = excluded.position",
        params![
            after.space_id,
            after.name,
            after.folder,
            flag(after.auto_start),
            after.max_doing,
            flag(after.locked),
            list_text(&after.repos),
            after.created_at,
            after.updated_at,
            after.color,
            after.position,
        ],
    )?;
    Ok(after)
}

pub fn remove_space(conn: &Connection, space_id: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM spaces WHERE space_id = ?1", params![space_id])? == 1)
}

pub fn folder_key(folder: &str) -> String {
    if let Ok(real) = std::fs::canonicalize(folder) {
        return real.to_string_lossy().to_string();
    }
    let key = folder.trim_end_matches('/');
    if key.is_empty() && folder.starts_with('/') { "/".to_string() } else { key.to_string() }
}

pub fn space_for_folder(conn: &Connection, folder: &str) -> Result<Option<Space>> {
    if folder.trim().is_empty() {
        return Ok(None);
    }
    let wanted = folder_key(folder);
    Ok(list_spaces(conn)?.into_iter().find(|s| !s.folder.trim().is_empty() && folder_key(&s.folder) == wanted))
}

pub const ACTIVE_KEY: &str = "app.active_space";
pub const OLD_FILE: &str = "spaces.json";

pub fn active_space(conn: &Connection) -> Result<Option<String>> {
    Ok(crate::settings::get_setting(conn, ACTIVE_KEY)?.and_then(|v| v.as_str().map(str::to_string)).filter(|s| !s.is_empty()))
}

pub fn set_active_space(conn: &Connection, space_id: Option<&str>) -> Result<()> {
    match space_id {
        Some(id) => crate::settings::set_setting_text(conn, ACTIVE_KEY, id),
        None => crate::settings::remove_setting(conn, ACTIVE_KEY).map(|_| ()),
    }
}

#[derive(Deserialize)]
struct OldSpace {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    folder: String,
    #[serde(default)]
    color: Option<i64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OldFile {
    List(Vec<OldSpace>),
    Wrapped {
        #[serde(default)]
        spaces: Vec<OldSpace>,
        #[serde(default)]
        active: Option<String>,
    },
}

pub fn import_spaces_json(conn: &Connection, path: &std::path::Path) -> Result<usize> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let (list, active) = match serde_json::from_str::<OldFile>(&text) {
        Ok(OldFile::List(list)) => (list, None),
        Ok(OldFile::Wrapped { spaces, active }) => (spaces, active),
        Err(e) => bail!("{} could not be read: {e}", path.display()),
    };
    let start = list_spaces(conn)?.iter().map(|s| s.position + 1).max().unwrap_or(0);
    let tx = conn.unchecked_transaction()?;
    let mut added = 0;
    for old in &list {
        if old.id.trim().is_empty() || get_space(&tx, old.id.trim())?.is_some() {
            continue;
        }
        let patch = SpacePatch {
            name: Some(old.name.clone()),
            folder: Some(old.folder.clone()),
            color: old.color,
            position: Some(start + added as i64),
            ..SpacePatch::default()
        };
        set_space(&tx, old.id.trim(), &patch)?;
        added += 1;
    }
    if let Some(id) = active.filter(|a| !a.is_empty())
        && active_space(&tx)?.is_none()
    {
        set_active_space(&tx, Some(&id))?;
    }
    tx.commit()?;
    std::fs::rename(path, path.with_extension("json.imported"))?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;

    #[test]
    fn imports_both_old_files_once() {
        let conn = temp_db("spaces-import");
        let dir = std::env::temp_dir().join(format!("nixa-store-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(OLD_FILE);
        assert_eq!(import_spaces_json(&conn, &path).unwrap(), 0);
        std::fs::write(&path, r#"{"spaces":[{"id":"s1","name":"one","folder":"/a/one","color":2},{"id":"s2","name":"two","folder":"/a/two"}],"active":"s2"}"#).unwrap();
        assert_eq!(import_spaces_json(&conn, &path).unwrap(), 2);
        assert!(!path.exists());
        assert!(path.with_extension("json.imported").exists());
        let all = list_spaces(&conn).unwrap();
        assert_eq!(all.iter().map(|s| (s.space_id.as_str(), s.color, s.position)).collect::<Vec<_>>(), vec![("s1", 2, 0), ("s2", 0, 1)]);
        assert_eq!(active_space(&conn).unwrap().as_deref(), Some("s2"));
        std::fs::write(&path, r#"[{"id":"s2","name":"other","folder":"/x"},{"id":"nixa","name":"nixa","folder":"/p/nixa"}]"#).unwrap();
        assert_eq!(import_spaces_json(&conn, &path).unwrap(), 1);
        assert_eq!(get_space(&conn, "s2").unwrap().unwrap().name, "two");
        assert_eq!(get_space(&conn, "nixa").unwrap().unwrap().position, 2);
        std::fs::write(&path, "{\"spaces\": [").unwrap();
        assert!(import_spaces_json(&conn, &path).is_err());
        assert!(path.exists());
        set_active_space(&conn, None).unwrap();
        assert!(active_space(&conn).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn set_get_list_remove() {
        let conn = temp_db("spaces");
        assert!(set_space(&conn, " ", &SpacePatch::default()).is_err());
        let made = set_space(&conn, "s8", &SpacePatch { name: Some("nixa".into()), folder: Some("/p/nixa/".into()), ..SpacePatch::default() }).unwrap();
        assert_eq!(made.max_doing, MAX_DOING);
        let repos = vec!["a/".to_string(), "a".to_string(), "../x".to_string(), "b".to_string()];
        let patch = SpacePatch { max_doing: Some(50), repos: Some(repos), ..SpacePatch::default() };
        let changed = set_space(&conn, "s8", &patch).unwrap();
        assert_eq!(changed.name, "nixa");
        assert_eq!(changed.max_doing, MAX_DOING_TOP);
        assert_eq!(changed.repos, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(get_space(&conn, "s8").unwrap().unwrap(), changed);
        set_space(&conn, "s1", &SpacePatch { name: Some("alpha".into()), ..SpacePatch::default() }).unwrap();
        let all = list_spaces(&conn).unwrap();
        assert_eq!(all[0].space_id, "s1");
        assert_eq!(space_for_folder(&conn, "/p/nixa").unwrap().unwrap().space_id, "s8");
        assert!(remove_space(&conn, "s1").unwrap());
        assert!(get_space(&conn, "s1").unwrap().is_none());
    }

    #[test]
    fn folders_keep_their_spaces() {
        let conn = temp_db("spaces-edge");
        let made = set_space(&conn, "s1", &SpacePatch { folder: Some("/w/shop ".into()), ..SpacePatch::default() }).unwrap();
        assert_eq!(made.folder, "/w/shop ");
        assert_eq!(space_for_folder(&conn, "/w/shop ").unwrap().unwrap().space_id, "s1");
        assert!(space_for_folder(&conn, "/w/shop").unwrap().is_none());
        assert_eq!(set_space(&conn, "s2", &SpacePatch { folder: Some("  ".into()), ..SpacePatch::default() }).unwrap().folder, "");
        assert!(space_for_folder(&conn, " ").unwrap().is_none());
        assert!(space_for_folder(&conn, "/").unwrap().is_none());
        set_space(&conn, "s3", &SpacePatch { folder: Some("/".into()), ..SpacePatch::default() }).unwrap();
        assert_eq!(space_for_folder(&conn, "/").unwrap().unwrap().space_id, "s3");
        assert_eq!(space_for_folder(&conn, "//").unwrap().unwrap().space_id, "s3");
        assert_eq!(folder_key("/w/shop/"), "/w/shop");
        assert_eq!(folder_key("/"), "/");
    }

    #[test]
    fn a_folder_and_its_link_are_one_space() {
        let conn = temp_db("spaces-link");
        let dir = std::env::temp_dir().join(format!("nixa-store-link-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("shop")).unwrap();
        let link = dir.join("shop-link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(dir.join("shop"), &link).unwrap();
        let real = std::fs::canonicalize(dir.join("shop")).unwrap().to_string_lossy().to_string();
        let typed = link.to_string_lossy().to_string();
        assert_eq!(folder_key(&typed), real);
        assert_eq!(folder_key(&format!("{typed}/")), real);
        set_space(&conn, "s1", &SpacePatch { folder: Some(typed.clone()), ..SpacePatch::default() }).unwrap();
        assert_eq!(space_for_folder(&conn, &real).unwrap().unwrap().space_id, "s1");
        assert_eq!(get_space(&conn, "s1").unwrap().unwrap().folder, typed);
        set_space(&conn, "s1", &SpacePatch { folder: Some(real.clone()), ..SpacePatch::default() }).unwrap();
        assert_eq!(space_for_folder(&conn, &typed).unwrap().unwrap().space_id, "s1");
        std::fs::remove_dir_all(&dir).ok();
    }
}
