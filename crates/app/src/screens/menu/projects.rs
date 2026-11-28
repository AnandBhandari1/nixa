use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const SAVED_FILE: &str = "projects.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: String,
}

fn last_part(path: &str) -> String {
    path.split('/')
        .rfind(|p| !p.is_empty())
        .unwrap_or(path)
        .to_string()
}

pub fn saved(text: &str) -> Vec<Project> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut rows: Vec<&Value> = Vec::new();
    if let Some(list) = value.get("projects").and_then(Value::as_array) {
        rows.extend(list);
    }
    if let Some(list) = value.get("commands").and_then(Value::as_array) {
        rows.extend(
            list.iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("project")),
        );
    }
    rows.into_iter()
        .filter_map(|p| {
            let path = p.get("path").and_then(Value::as_str)?.trim();
            if path.is_empty() {
                return None;
            }
            let name = p
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| last_part(path));
            Some(Project {
                name,
                path: path.to_string(),
            })
        })
        .collect()
}

pub fn folders(dir: &Path) -> Vec<Project> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut list: Vec<Project> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (!name.starts_with('.')).then(|| Project {
                path: dir.join(&name).display().to_string(),
                name,
            })
        })
        .collect();
    list.sort_by_key(|p| p.name.to_lowercase());
    list
}

pub fn merge(first: Vec<Project>, then: Vec<Project>) -> Vec<Project> {
    let mut seen = HashSet::new();
    first
        .into_iter()
        .chain(then)
        .filter(|p| seen.insert(p.path.clone()))
        .collect()
}

pub fn projects_dir(home: &Path) -> PathBuf {
    home.join("Projects")
}

pub fn load(home: &Path, data: &Path) -> Vec<Project> {
    let text = std::fs::read_to_string(data.join(SAVED_FILE)).unwrap_or_default();
    merge(saved(&text), folders(&projects_dir(home)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mm-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saved_list_reads_projects_and_project_commands() {
        let text = r#"{"projects":[{"name":"Nixa","path":"/p/nixa"},{"path":"/p/oss/"},{"path":""}],
            "commands":[{"type":"project","path":"/p/web"},{"type":"script","path":"/p/x"}]}"#;
        let list = saved(text);
        assert_eq!(
            list,
            vec![
                Project {
                    name: "Nixa".into(),
                    path: "/p/nixa".into()
                },
                Project {
                    name: "oss".into(),
                    path: "/p/oss/".into()
                },
                Project {
                    name: "web".into(),
                    path: "/p/web".into()
                },
            ]
        );
        assert!(saved("not json").is_empty());
        assert!(saved("{}").is_empty());
    }

    #[test]
    fn folders_are_sorted_and_hidden_ones_skipped() {
        let home = temp("home");
        let dir = projects_dir(&home);
        for name in ["zeta", "Alpha", ".hidden", "beta"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        std::fs::write(dir.join("file.txt"), "x").unwrap();
        let names: Vec<String> = folders(&dir).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Alpha", "beta", "zeta"]);
        let data = temp("data");
        let beta = dir.join("beta").display().to_string();
        std::fs::write(
            data.join(SAVED_FILE),
            format!(r#"{{"projects":[{{"name":"Beta saved","path":"{beta}"}}]}}"#),
        )
        .unwrap();
        let all = load(&home, &data);
        assert_eq!(all[0].name, "Beta saved");
        assert_eq!(all.len(), 3);
        assert!(folders(&home.join("missing")).is_empty());
        std::fs::remove_dir_all(&home).ok();
        std::fs::remove_dir_all(&data).ok();
    }
}
