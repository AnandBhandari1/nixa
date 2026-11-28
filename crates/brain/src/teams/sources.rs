use super::crew::{good_name, name_key};
use crate::schedules::runner::Repo;
use crate::spaces::Space;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

pub fn skill_dir() -> PathBuf {
    home().unwrap_or_else(|| PathBuf::from("/")).join(".claude").join("skills")
}

pub fn skills_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .filter(|e| e.path().join("SKILL.md").is_file())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| good_name(n))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn json_of(text: Option<String>) -> Value {
    text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

fn keys_of(value: Option<&Value>, out: &mut BTreeSet<String>) {
    if let Some(map) = value.and_then(Value::as_object) {
        out.extend(map.keys().filter(|k| good_name(k)).cloned());
    }
}

pub struct McpFiles {
    pub claude: Option<String>,
    pub project: Option<String>,
    pub codex: Option<String>,
    pub cursor: Option<String>,
}

pub fn mcp_names_in(files: &McpFiles, folder: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    let claude = json_of(files.claude.clone());
    keys_of(claude.get("mcpServers"), &mut out);
    let folder = folder.trim_end_matches('/');
    if let Some(projects) = claude.get("projects").and_then(Value::as_object) {
        for (path, entry) in projects {
            if !folder.is_empty() && (folder == path || folder.starts_with(&format!("{path}/"))) {
                keys_of(entry.get("mcpServers"), &mut out);
            }
        }
    }
    keys_of(json_of(files.project.clone()).get("mcpServers"), &mut out);
    for line in files.codex.as_deref().unwrap_or_default().lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("[mcp_servers.").and_then(|r| r.strip_suffix(']'))
            && !rest.contains('.')
            && good_name(rest)
        {
            out.insert(rest.to_string());
        }
    }
    keys_of(json_of(files.cursor.clone()).get("mcpServers"), &mut out);
    out.into_iter().collect()
}

pub fn mcp_names(folder: &str) -> Vec<String> {
    let read = |p: PathBuf| std::fs::read_to_string(p).ok();
    let home = home();
    let files = McpFiles {
        claude: None,
        project: if folder.is_empty() { None } else { read(Path::new(folder).join(".mcp.json")) },
        codex: None,
        cursor: home.as_ref().and_then(|h| read(h.join(".cursor").join("mcp.json"))),
    };
    mcp_names_in(&files, folder)
}

pub fn find_space<'a>(spaces: &'a [Space], wanted: &str) -> Option<&'a Space> {
    let value = wanted.trim();
    let key = name_key(value);
    crate::spaces::find(spaces, value)
        .or_else(|| spaces.iter().find(|s| !key.is_empty() && name_key(&s.name) == key))
        .or_else(|| spaces.iter().find(|s| !key.is_empty() && name_key(s.folder.rsplit('/').find(|p| !p.is_empty()).unwrap_or_default()) == key))
        .or_else(|| spaces.iter().find(|s| s.folder == value))
}

pub fn pick_repo<'a>(repos: &'a [Repo], wanted: &str, folder: &str) -> Option<&'a Repo> {
    let value = wanted.trim();
    if value.is_empty() {
        return repos.iter().find(|r| r.folder == folder).or(repos.first());
    }
    let key = name_key(value);
    repos.iter().find(|r| r.folder == value).or_else(|| repos.iter().find(|r| name_key(&r.name) == key))
}
