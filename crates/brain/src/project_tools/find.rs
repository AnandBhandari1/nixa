use super::policy::{home, project_path};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectSpace {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub repos: Vec<String>,
    pub locked: bool,
}

pub fn spaces_of(db: &Path) -> Vec<ProjectSpace> {
    let saved = nixa_store::open(db).and_then(|conn| nixa_store::spaces::list_spaces(&conn)).unwrap_or_default();
    if !saved.is_empty() {
        return saved.into_iter().map(|s| ProjectSpace { id: s.space_id, name: s.name, folder: s.folder, repos: s.repos, locked: s.locked }).collect();
    }
    crate::spaces::load(db).into_iter().map(|s| ProjectSpace { id: s.id, name: s.name, folder: s.folder, ..ProjectSpace::default() }).collect()
}

pub fn projects_dir() -> PathBuf {
    home().unwrap_or_default().join("Projects")
}

pub fn expand_home(path: &str) -> String {
    match home() {
        Some(h) if path == "~" => h.display().to_string(),
        Some(h) if path.starts_with("~/") => h.join(&path[2..]).display().to_string(),
        _ => path.to_string(),
    }
}

pub fn subdirs(folder: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false) && !e.file_name().to_string_lossy().starts_with('.'))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn last_name(folder: &str) -> String {
    folder.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_lowercase()
}

pub fn find_project_folder(name: &str, spaces: &[ProjectSpace], projects: &Path) -> Result<String, String> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return Err("Name the project: a space name, a repo name, or an absolute folder.".into());
    }
    if wanted.starts_with('/') || wanted.starts_with('~') {
        let folder = project_path(&expand_home(wanted), ".")?;
        let holds = |dir: &Path| std::fs::canonicalize(dir).is_ok_and(|real| real.parent().is_some() && folder.starts_with(&real));
        let known = spaces.iter().any(|s| holds(Path::new(&s.folder)) || s.repos.iter().any(|r| holds(Path::new(r))));
        if known || holds(projects) || folder.join(".git").exists() {
            return Ok(folder.display().to_string());
        }
        return Err(format!("{wanted} is not in an open space or the Projects folder and is not a git repo. Choose a project folder."));
    }
    if wanted.split(['/', '\\']).any(|p| p == "..") {
        return Err("Give a project name or an absolute folder, without ..".into());
    }
    let lower = wanted.to_lowercase();
    let space = spaces.iter().find(|s| s.name.to_lowercase() == lower).or_else(|| spaces.iter().find(|s| last_name(&s.folder) == lower));
    if let Some(space) = space {
        return project_path(&space.folder, ".").map(|p| p.display().to_string());
    }
    let mut tries: Vec<PathBuf> = spaces.iter().map(|s| Path::new(&s.folder).join(wanted)).collect();
    tries.push(projects.join(wanted));
    let level = subdirs(projects);
    tries.extend(level.iter().map(|d| d.join(wanted)));
    tries.extend(level.iter().flat_map(|d| subdirs(d)).map(|d| d.join(wanted)));
    if let Some(found) = tries.iter().find(|p| p.join(".git").exists()) {
        return project_path(&found.display().to_string(), ".").map(|p| p.display().to_string());
    }
    let open: Vec<&str> = spaces.iter().map(|s| s.name.as_str()).collect();
    let named = if open.is_empty() { String::new() } else { format!(" ({})", open.join(", ")) };
    Err(format!("No project \"{wanted}\". Name an open space{named}, a repo folder name, or give the absolute folder."))
}

pub fn find(app: &crate::app::App, name: &str) -> Result<String, String> {
    find_project_folder(name, &spaces_of(&app.db_file), &projects_dir())
}
