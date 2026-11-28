use super::index::{Root, Source};
use crate::project_tools::find::ProjectSpace;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const FOLDERS_KEY: &str = "knowledge.folders";
pub const MAX_FOLDERS: usize = 20;
const DOCS: &str = "docs";

fn label_of(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| dir.display().to_string())
}

fn push(out: &mut Vec<Root>, seen: &mut HashSet<PathBuf>, root: Root) {
    if seen.insert(root.dir.clone()) {
        out.push(root);
    }
}

fn docs_of(folder: &Path, label: &str, out: &mut Vec<Root>, seen: &mut HashSet<PathBuf>) {
    let Ok(docs) = std::fs::canonicalize(folder.join(DOCS)) else { return };
    if docs.is_dir() && !super::index::blocked_in(&docs, &super::index::private_roots()) {
        push(out, seen, Root { source: Source::Docs, dir: docs, label: label.to_string() });
    }
}

pub fn saved_spaces(db: &Path) -> Vec<ProjectSpace> {
    nixa_store::open(db)
        .and_then(|conn| nixa_store::spaces::list_spaces(&conn))
        .unwrap_or_default()
        .into_iter()
        .map(|s| ProjectSpace { id: s.space_id, name: s.name, folder: s.folder, repos: s.repos, locked: s.locked })
        .collect()
}

fn homes_of(home: Option<&Path>) -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = home.map(Path::to_path_buf).into_iter().collect();
    homes.extend(home.and_then(|h| std::fs::canonicalize(h).ok()));
    homes
}

fn wide_folder(path: &Path, homes: &[PathBuf]) -> bool {
    too_wide(path, homes) || std::fs::canonicalize(path).ok().is_some_and(|real| too_wide(&real, homes))
}

pub fn space_roots(spaces: &[ProjectSpace], home: Option<&Path>) -> Vec<Root> {
    let homes = homes_of(home);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for space in spaces {
        let folder = PathBuf::from(space.folder.trim());
        if !folder.is_absolute() || wide_folder(&folder, &homes) {
            continue;
        }
        let name = if space.name.trim().is_empty() { label_of(&folder) } else { space.name.trim().to_string() };
        docs_of(&folder, &name, &mut out, &mut seen);
        for repo in space.repos.iter().map(PathBuf::from).filter(|r| r.is_absolute() && !wide_folder(r, &homes)) {
            docs_of(&repo, &label_of(&repo), &mut out, &mut seen);
        }
        for sub in crate::project_tools::find::subdirs(&folder) {
            docs_of(&sub, &label_of(&sub), &mut out, &mut seen);
        }
    }
    out
}

fn too_wide(path: &Path, homes: &[PathBuf]) -> bool {
    path.parent().is_none() || homes.iter().any(|h| h == path || h.starts_with(path))
}

pub fn check_folder(raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    check_folder_in(raw, home, &super::index::private_roots())
}

pub fn check_folder_in(raw: &str, home: Option<&Path>, private: &[PathBuf]) -> Result<PathBuf, String> {
    let expanded = crate::project_tools::find::expand_home(raw.trim());
    let path = PathBuf::from(&expanded);
    if !path.is_absolute() {
        return Err(format!("{raw}: use a full folder path."));
    }
    let homes = homes_of(home);
    let real = std::fs::canonicalize(&path).ok();
    if too_wide(&path, &homes) || real.as_deref().is_some_and(|r| too_wide(r, &homes)) {
        return Err(format!("{raw}: choose a notes folder, not the home folder or the disk top."));
    }
    let private_here = |p: &Path| crate::project_tools::policy::protected_in(p, private) || crate::project_tools::policy::sensitive_name(p);
    if private_here(&path) || real.as_deref().is_some_and(private_here) {
        return Err(format!("{raw}: this folder holds private settings or secrets."));
    }
    Ok(path)
}

pub fn clean_folders(raw: &[String], home: Option<&Path>) -> Result<Vec<String>, String> {
    clean_folders_in(raw, home, &super::index::private_roots())
}

pub fn clean_folders_in(raw: &[String], home: Option<&Path>, private: &[PathBuf]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for item in raw.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let path = check_folder_in(item, home, private)?.display().to_string();
        if !out.contains(&path) {
            out.push(path);
        }
    }
    if out.len() > MAX_FOLDERS {
        return Err(format!("At most {MAX_FOLDERS} folders."));
    }
    Ok(out)
}

pub fn folders(db: &Path) -> Vec<String> {
    let value = nixa_store::open(db).ok().and_then(|conn| nixa_store::settings::get_setting(&conn, FOLDERS_KEY).ok().flatten());
    match value {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

pub fn save_folders(db: &Path, list: &[String]) -> Result<(), String> {
    let conn = nixa_store::open(db).map_err(|e| format!("{e:#}"))?;
    nixa_store::settings::set_setting(&conn, FOLDERS_KEY, &json!(list)).map_err(|e| format!("{e:#}"))
}

pub fn folder_roots(list: &[String], home: Option<&Path>) -> (Vec<Root>, Vec<String>) {
    folder_roots_in(list, home, &super::index::private_roots())
}

pub fn folder_roots_in(list: &[String], home: Option<&Path>, private: &[PathBuf]) -> (Vec<Root>, Vec<String>) {
    let mut roots = Vec::new();
    let mut missing = Vec::new();
    for item in list {
        match check_folder_in(item, home, private) {
            Ok(dir) if dir.is_dir() => {
                let real = std::fs::canonicalize(&dir).unwrap_or(dir);
                roots.push(Root { source: Source::Notes, label: label_of(&real), dir: real });
            }
            Ok(_) => missing.push(item.clone()),
            Err(error) => crate::util::log("knowledge", &error),
        }
    }
    (roots, missing)
}
