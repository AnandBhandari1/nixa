pub mod files;
pub mod find;
pub mod policy;
pub mod project_of;
pub mod search;

#[cfg(test)]
mod tests;

use crate::app::App;
use crate::tools::{Args, ToolResult, flag, need, schema, spec, text};
use crate::turns::ToolSpec;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const GIT: &str = "project_git";
pub const FILES: &str = "project_files";
pub const SEARCH: &str = "project_search";
pub const READ_LINES: &str = "project_read_lines";
pub const NAMES: [&str; 4] = [GIT, FILES, SEARCH, READ_LINES];

#[derive(Default)]
pub struct State {
    pub roots: Mutex<HashMap<String, String>>,
}

pub fn specs() -> Vec<ToolSpec> {
    let project = json!({"type": "string", "description": "Space name, repo folder name, or absolute project folder"});
    vec![
        spec(GIT, "Read git status, branches, diffs or history of one project. Read only: no writes, commits or pushes. Secret files are left out of diffs.", schema(json!({"project": project, "action": {"type": "string", "enum": files::GIT_ACTIONS}}), &["project", "action"])),
        spec(FILES, "List a folder or read a text file (up to 1 MiB) inside one project. Links cannot leave the project. Secret and env files and private config folders are refused.", schema(json!({"project": project, "path": {"type": "string", "description": "Path inside the project, . for the top"}, "action": {"type": "string", "enum": ["list", "read"]}}), &["project", "path", "action"])),
        spec(SEARCH, "Search the text files of a project for a word, a phrase or a regex, like grep. Name the project (space name, repo name or absolute folder); path narrows it to a folder or one file inside it. Returns each match with its file, line number and line. It skips node_modules, dist, build, target, hidden folders, secret files and files over 1 MiB, and stops at the match limit, 5000 files or 5 seconds, and says so.", schema(json!({"project": project, "query": {"type": "string"}, "path": {"type": "string"}, "regex": {"type": "boolean"}, "case_sensitive": {"type": "boolean"}, "limit": {"type": "integer", "minimum": 1, "maximum": search::SEARCH_MAX_MATCHES}}), &["project", "query"])),
        spec(READ_LINES, "Read some lines of a text file in a project, with line numbers, for files of any size. Name the project and the path inside it; from is the first line (default 1) and count how many lines (default 200, at most 400). Use project_search first to find the line. end true means the file has no more lines.", schema(json!({"project": project, "path": {"type": "string"}, "from": {"type": "integer", "minimum": 1}, "count": {"type": "integer", "minimum": 1, "maximum": search::RANGE_MAX_LINES}}), &["project", "path"])),
    ]
}

fn json_text(value: serde_json::Value) -> String {
    value.to_string()
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(work).await.map_err(|e| format!("The search stopped: {e}"))?
}

pub async fn search(folder: String, args: &Args) -> ToolResult {
    let query = need(args, "query")?;
    let path = text(args, "path").unwrap_or_else(|| ".".into());
    let pattern = search::matcher(&query, flag(args, "regex") == Some(true), flag(args, "case_sensitive") == Some(true))?;
    let limit = search::positive(args.get("limit"), search::SEARCH_MATCHES, search::SEARCH_MAX_MATCHES);
    let found = blocking(move || {
        let root = policy::project_path(&folder, ".")?;
        let start = policy::project_path(&folder, &path)?;
        Ok(search::search_folder(&root, &start, &pattern, limit, Duration::from_millis(search::SEARCH_MS)))
    })
    .await?;
    let mut out = json!({"query": query});
    if let (Some(map), Ok(serde_json::Value::Object(extra))) = (out.as_object_mut(), serde_json::to_value(&found)) {
        map.extend(extra);
    }
    if found.more {
        out["note"] = json!("More matches exist: narrow the path or the query.");
    }
    Ok(json_text(out))
}

pub async fn read_lines(folder: String, args: &Args) -> ToolResult {
    let path = need(args, "path")?;
    let from = search::positive(args.get("from"), 1, usize::MAX / 2);
    let count = search::positive(args.get("count"), search::RANGE_LINES, search::RANGE_MAX_LINES);
    let range = blocking(move || {
        let target = policy::project_path(&folder, &path)?;
        search::read_range(&target, from, count)
    })
    .await?;
    let mut out = serde_json::to_value(&range).map_err(|e| e.to_string())?;
    out["observed_at"] = json!(crate::util::now_ms());
    Ok(json_text(out))
}

pub async fn call(app: &Arc<App>, name: &str, args: &Args) -> Option<ToolResult> {
    if !NAMES.contains(&name) {
        return None;
    }
    let folder = match need(args, "project").and_then(|p| find::find(app, &p)) {
        Ok(folder) => folder,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        GIT => match need(args, "action") {
            Ok(action) => files::inspect_git(&folder, &action).await.map(json_text),
            Err(e) => Err(e),
        },
        FILES => {
            let path = text(args, "path").unwrap_or_else(|| ".".into());
            match text(args, "action").as_deref() {
                Some("list") => files::list_dir(&folder, &path).map(json_text),
                Some("read") => files::read_file(&folder, &path).map(json_text),
                _ => Err("Choose list or read.".into()),
            }
        }
        SEARCH => search(folder, args).await,
        _ => read_lines(folder, args).await,
    })
}
