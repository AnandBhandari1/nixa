use super::policy::{project_path, protected_path, refused, secret_part, sensitive_name};
use crate::git_tools::run::{READ_MS, git_args};
use serde_json::{Value, json};
use std::path::Path;

pub const READ_BYTES: u64 = 1024 * 1024;
pub const GIT_ACTIONS: [&str; 5] = ["status", "branches", "diff", "staged", "history"];

pub fn git_words(action: &str) -> Option<Vec<&'static str>> {
    Some(match action {
        "status" => vec!["status", "--porcelain=v1", "--branch", "--", "."],
        "branches" => vec!["branch", "--list", "--verbose"],
        "diff" => vec!["diff", "--relative", "--no-ext-diff", "--no-textconv"],
        "staged" => vec!["diff", "--cached", "--relative", "--no-ext-diff", "--no-textconv"],
        "history" => vec!["log", "-20", "--format=%h %ad %s", "--date=iso-strict", "--", "."],
        _ => return None,
    })
}

pub fn safe_diff_path(path: &str) -> bool {
    !path.split('/').any(secret_part) && !protected_path(Path::new(path)) && !sensitive_name(Path::new(path))
}

pub async fn inspect_git(project: &str, action: &str) -> Result<Value, String> {
    let cwd = project_path(project, ".")?.display().to_string();
    let words = git_words(action).ok_or("Git actions here: status, branches, diff, staged, history. No git writes.")?;
    let mut args: Vec<String> = vec!["--no-optional-locks".into()];
    args.extend(words.iter().map(|w| w.to_string()));
    let mut omitted = 0;
    if action == "diff" || action == "staged" {
        let mut names_args = args.clone();
        names_args.extend(["--name-only".to_string(), "-z".to_string()]);
        let paths = git_args(&cwd, &names_args, READ_MS).await;
        if !paths.ok() {
            return Err(if paths.stderr.trim().is_empty() { "Git inspection failed.".into() } else { paths.stderr.trim().to_string() });
        }
        let names: Vec<&str> = paths.stdout.split('\0').filter(|n| !n.is_empty()).collect();
        let safe: Vec<&str> = names.iter().copied().filter(|n| safe_diff_path(n)).collect();
        omitted = names.len() - safe.len();
        if safe.is_empty() {
            return Ok(json!({"project": cwd, "action": action, "output": "", "omitted_paths": omitted, "observed_at": crate::util::now_ms()}));
        }
        args.push("--".into());
        args.extend(safe.iter().map(|p| format!(":(literal){p}")));
    }
    let out = git_args(&cwd, &args, READ_MS).await;
    if !out.ok() {
        return Err(if out.stderr.trim().is_empty() { "Git inspection failed.".into() } else { out.stderr.trim().to_string() });
    }
    Ok(json!({"project": cwd, "action": action, "output": crate::util::clip(&out.stdout, 200_000), "omitted_paths": omitted, "observed_at": crate::util::now_ms()}))
}

pub fn list_dir(project: &str, path: &str) -> Result<Value, String> {
    let target = project_path(project, path)?;
    let mut entries: Vec<(String, bool)> = std::fs::read_dir(&target)
        .map_err(|e| format!("{} cannot be listed: {e}.", target.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| !refused(&e.path()) && !secret_part(&e.file_name().to_string_lossy()))
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.file_type().map(|t| t.is_dir()).unwrap_or(false)))
        .collect();
    entries.sort();
    let list: Vec<Value> = entries.into_iter().map(|(name, directory)| json!({"name": name, "directory": directory})).collect();
    Ok(json!({"path": target.display().to_string(), "entries": list}))
}

pub fn read_file(project: &str, path: &str) -> Result<Value, String> {
    let target = project_path(project, path)?;
    let info = std::fs::metadata(&target).map_err(|e| format!("{} cannot be read: {e}.", target.display()))?;
    if !info.is_file() {
        return Err("Not a file. Use list for a folder.".into());
    }
    if info.len() > READ_BYTES {
        return Err("The file is over the 1 MiB read limit. Use project_read_lines.".into());
    }
    let bytes = std::fs::read(&target).map_err(|e| format!("{} cannot be read: {e}.", target.display()))?;
    if bytes.contains(&0) {
        return Err("Not a text file.".into());
    }
    Ok(json!({"path": target.display().to_string(), "text": String::from_utf8_lossy(&bytes), "observed_at": crate::util::now_ms()}))
}
