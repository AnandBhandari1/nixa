use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub folder: String,
}

pub fn from_store(conn: &nixa_store::Connection) -> Vec<Space> {
    from_store_of(&nixa_store::spaces::list_spaces(conn).unwrap_or_default())
}

/// The brain's short view of the stored spaces: id, shown name and folder.
pub fn from_store_of(rows: &[nixa_store::spaces::Space]) -> Vec<Space> {
    rows.iter()
        .map(|s| Space { name: if s.name.trim().is_empty() { s.space_id.clone() } else { s.name.clone() }, id: s.space_id.clone(), folder: s.folder.clone() })
        .collect()
}

pub fn load(db: &Path) -> Vec<Space> {
    nixa_store::open(db).map(|conn| from_store(&conn)).unwrap_or_default()
}

pub fn import_old(db: &Path, file: &Path) -> anyhow::Result<usize> {
    let conn = nixa_store::open(db)?;
    nixa_store::spaces::import_spaces_json(&conn, file)
}

fn expand(path: &str) -> Option<PathBuf> {
    let text = path.trim();
    if text.is_empty() {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let full = if text == "~" {
        home?
    } else if let Some(rest) = text.strip_prefix("~/") {
        home?.join(rest)
    } else {
        PathBuf::from(text)
    };
    if !full.is_absolute() {
        return None;
    }
    Some(std::fs::canonicalize(&full).unwrap_or(full))
}

pub fn folder_in(path: &str, folder: &str) -> bool {
    match (expand(path), expand(folder)) {
        (Some(a), Some(b)) => a.starts_with(&b),
        _ => false,
    }
}

pub fn same_folder(a: &str, b: &str) -> bool {
    match (expand(a), expand(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

pub fn for_folder<'a>(spaces: &'a [Space], folder: &str) -> Option<&'a Space> {
    spaces
        .iter()
        .filter(|s| folder_in(folder, &s.folder))
        .max_by_key(|s| expand(&s.folder).map(|p| p.as_os_str().len()).unwrap_or(0))
}

pub fn find<'a>(spaces: &'a [Space], key: &str) -> Option<&'a Space> {
    let key = key.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    spaces
        .iter()
        .find(|s| s.id.to_lowercase() == key)
        .or_else(|| spaces.iter().find(|s| s.name.trim().to_lowercase() == key))
        .or_else(|| if key.starts_with('/') || key.starts_with('~') { for_folder(spaces, &key) } else { None })
}

pub fn names_it(text: &str, space: &str) -> bool {
    let text = text.trim().to_lowercase();
    let space = space.trim().to_lowercase();
    if space.is_empty() {
        return true;
    }
    if text.starts_with(&format!("{space}:")) {
        return true;
    }
    let ends_word = |at: usize| text[at..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '-' && c != '_');
    ["in ", "space ", "from "].iter().any(|lead| {
        let wanted = format!("{lead}{space}");
        text.match_indices(&wanted).any(|(at, _)| {
            let starts_word = text[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
            starts_word && ends_word(at + wanted.len())
        })
    }) || text.match_indices(&format!("{space} deep brain")).any(|(at, _)| text[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric()))
}

pub fn named(space: &str, text: &str) -> String {
    let space = space.trim();
    if space.is_empty() || text.trim().is_empty() || names_it(text, space) {
        return text.to_string();
    }
    format!("{space}: {}", text.trim_start())
}

pub fn name_of(spaces: &[Space], key: &str) -> Option<String> {
    find(spaces, key).map(|s| s.name.clone()).filter(|n| !n.trim().is_empty())
}

pub fn home(spaces: &[Space]) -> Option<&Space> {
    spaces.iter().find(|s| s.id == "nixa" || s.name == "nixa").or(spaces.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_gets_its_space_name_once() {
        assert_eq!(named("shop", "Terminal 5 stays open."), "shop: Terminal 5 stays open.");
        assert_eq!(named("shop", "shop: Terminal 5 stays open."), "shop: Terminal 5 stays open.");
        assert_eq!(named("shop", "Agent 3 is done: fix the bar in shop"), "Agent 3 is done: fix the bar in shop");
        assert_eq!(named("shop", "Asked the deep brain of space Shop as d1."), "Asked the deep brain of space Shop as d1.");
        assert_eq!(named("shop", "shop deep brain: done."), "shop deep brain: done.");
        assert_eq!(named("nixa", "Nixa asked the deep brain d2 first."), "nixa: Nixa asked the deep brain d2 first.");
        assert_eq!(named("shop", "Work in shopping is done."), "shop: Work in shopping is done.");
        assert_eq!(named("shop", "Agent 2 done in shop-api"), "shop: Agent 2 done in shop-api");
        assert_eq!(named("", "Plain line."), "Plain line.");
        assert_eq!(named("shop", "  "), "  ");
    }

    #[test]
    fn finds_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().display().to_string();
        let inner = dir.path().join("inner");
        std::fs::create_dir_all(inner.join("deep")).unwrap();
        let spaces = vec![
            Space { id: "s1".into(), name: "Outer".into(), folder: root.clone() },
            Space { id: "s2".into(), name: "inner".into(), folder: inner.display().to_string() },
        ];
        assert_eq!(find(&spaces, "outer").unwrap().id, "s1");
        assert_eq!(find(&spaces, "S2").unwrap().id, "s2");
        assert_eq!(for_folder(&spaces, &inner.join("deep").display().to_string()).unwrap().id, "s2");
        assert!(for_folder(&spaces, "/nowhere/else").is_none());
        let db = dir.path().join("nixa.db");
        assert!(load(&db).is_empty());
        let old = dir.path().join("spaces.json");
        std::fs::write(&old, serde_json::to_string(&spaces).unwrap()).unwrap();
        assert_eq!(import_old(&db, &old).unwrap(), 2);
        let loaded = load(&db);
        assert_eq!(loaded.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["s1", "s2"]);
        assert_eq!(loaded[1].folder, inner.display().to_string());
        assert_eq!(import_old(&db, &old).unwrap(), 0);
    }
}
