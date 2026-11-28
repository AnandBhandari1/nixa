use super::checks::{MAX_PACKAGES, RunOptions, clean_path, inside, manifest_kind, packages_for, run_checks, scripts_of};
use crate::spaces::Space;
use crate::tools::{Args, ToolResult, need, schema, spec, strings};
use crate::turns::ToolSpec;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const NAME: &str = "nixa_run_checks";
pub const TOOL_CHECK_MS: u64 = 2 * 60 * 1000;
pub const TOOL_CHECK_MAX_MS: u64 = 10 * 60 * 1000;
const GIT_MS: u64 = 15_000;

pub fn spec_of() -> ToolSpec {
    spec(
        NAME,
        "Run a project's own checks now and get each result with its last lines: the check, typecheck, lint and test npm scripts, or cargo check, clippy and test for a Rust package. Name the project (a space name or id, or its folder); give paths (files or package folders inside it) to choose the packages, or leave them out to check the packages with changed files. seconds is the whole time limit (default 120, at most 600). It changes no files and runs no other command.",
        schema(json!({"project": {"type": "string"}, "paths": {"type": "array", "items": {"type": "string"}}, "seconds": {"type": "number"}}), &["project"]),
    )
}

pub fn seconds_of(value: Option<&Value>) -> u64 {
    match value.and_then(Value::as_f64) {
        Some(s) if s.is_finite() && s > 0.0 => ((s * 1000.0).round() as u64).min(TOOL_CHECK_MAX_MS),
        _ => TOOL_CHECK_MS,
    }
}

pub fn expand(path: &str) -> Option<PathBuf> {
    let text = path.trim();
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
    Some(std::fs::canonicalize(&full).unwrap_or_else(|_| clean_path(&full)))
}

pub fn find_project(spaces: &[Space], project: &str) -> Result<PathBuf, String> {
    let space = crate::spaces::find(spaces, project).ok_or_else(|| format!("No space or space folder matches {project}. Name a space from nixa_task spaces, or a folder inside one."))?;
    let folder = expand(&space.folder).ok_or_else(|| format!("The space {} has no usable folder.", space.name))?;
    if project.starts_with('/') || project.starts_with('~') {
        let given = expand(project).ok_or_else(|| format!("{project} is not a folder."))?;
        if given.is_dir() && given.starts_with(&folder) {
            return Ok(given);
        }
    }
    if !folder.is_dir() {
        return Err(format!("The folder {} of space {} does not exist.", folder.display(), space.name));
    }
    Ok(folder)
}

pub async fn git(folder: &Path, args: &[&str]) -> Option<String> {
    let run = tokio::process::Command::new("git").arg("-C").arg(folder).args(args).stdin(std::process::Stdio::null()).kill_on_drop(true).output();
    let out = tokio::time::timeout(Duration::from_millis(GIT_MS), run).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn parse_status(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut parts = raw.split('\0');
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let code = &entry[..2];
        out.push(entry[3..].to_string());
        if code.contains('R') || code.contains('C') {
            let source = parts.next();
            if code.contains('R')
                && let Some(source) = source.filter(|s| !s.is_empty())
            {
                out.push(source.to_string());
            }
        }
    }
    out
}

pub fn tool_cache(path: &str) -> bool {
    path.contains(".trail/cache/") || path.split('/').any(|part| part == "target" || part == "node_modules")
}

fn nested_repos(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.join(".git").exists()).collect();
    found.sort();
    found
}

pub async fn changed_in(folder: &Path) -> Option<Vec<String>> {
    let roots = match git(folder, &["rev-parse", "--show-toplevel"]).await {
        Some(top) if !top.trim().is_empty() => vec![PathBuf::from(top.trim())],
        _ => nested_repos(folder),
    };
    let base = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    let mut out = Vec::new();
    let mut read_any = false;
    for root in roots {
        let Some(status) = git(&root, &["--no-optional-locks", "status", "--porcelain=v1", "-z", "--untracked-files=all"]).await else {
            continue;
        };
        read_any = true;
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        for path in parse_status(&status).into_iter().filter(|p| !tool_cache(p)) {
            let full = clean_path(&root.join(&path));
            if let Ok(rel) = full.strip_prefix(&base)
                && !rel.as_os_str().is_empty()
            {
                out.push(rel.to_string_lossy().to_string());
            }
        }
    }
    read_any.then_some(out)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckPlan {
    pub packages: Vec<PathBuf>,
    pub not_checked: Vec<String>,
    pub no_scripts: Vec<String>,
}

fn shown(folder: &Path, dir: &Path) -> String {
    match dir.strip_prefix(folder) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.to_string_lossy().to_string(),
        _ => dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
    }
}

pub fn plan_checks(folder: &Path, paths: &[String]) -> CheckPlan {
    let folder = clean_path(folder);
    let all = packages_for(&folder, paths, usize::MAX);
    let (with, without): (Vec<PathBuf>, Vec<PathBuf>) = all.into_iter().partition(|dir| !scripts_of(dir).is_empty());
    CheckPlan {
        not_checked: with.iter().skip(MAX_PACKAGES).map(|d| shown(&folder, d)).collect(),
        packages: with.into_iter().take(MAX_PACKAGES).collect(),
        no_scripts: without.iter().map(|d| shown(&folder, d)).collect(),
    }
}

pub fn given_paths(folder: &Path, given: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for path in given.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if !inside(folder, Path::new(path)) {
            return Err(format!("{path} is outside the project folder {}.", folder.display()));
        }
        let full = clean_path(&folder.join(path));
        if full.exists() {
            crate::project_tools::policy::project_path(&folder.to_string_lossy(), path)?;
        }
        match manifest_kind(&full) {
            Some(super::checks::Kind::Npm) => out.push(format!("{}/package.json", path.trim_end_matches('/'))),
            Some(super::checks::Kind::Cargo) => out.push(format!("{}/Cargo.toml", path.trim_end_matches('/'))),
            None => out.push(path.to_string()),
        }
    }
    Ok(out)
}

pub async fn call(spaces: &[Space], args: &Args) -> ToolResult {
    let folder = find_project(spaces, &need(args, "project")?)?;
    crate::project_tools::policy::project_path(&folder.to_string_lossy(), "")?;
    let given = strings(args, "paths");
    let paths = if given.iter().any(|p| !p.trim().is_empty()) {
        given_paths(&folder, &given)?
    } else {
        changed_in(&folder).await.unwrap_or_default()
    };
    let plan = plan_checks(&folder, &paths);
    let shown_folder = folder.to_string_lossy().to_string();
    if plan.packages.is_empty() {
        let none = if plan.no_scripts.is_empty() {
            "no package.json or Cargo.toml found for these paths".to_string()
        } else {
            format!("no check, typecheck, lint or test script in {}", plan.no_scripts.join(", "))
        };
        return Ok(json!({"folder": shown_folder, "ran": false, "none": none}).to_string());
    }
    let options = RunOptions { packages: Some(plan.packages.clone()), total_ms: Some(seconds_of(args.get("seconds"))), ..RunOptions::default() };
    let outcome = run_checks(&folder, &paths, &options).await;
    crate::util::log("checks", &format!("tool checks in {shown_folder}: failed {}, packages {}", outcome.failed, plan.packages.len()));
    let mut out = json!({"folder": shown_folder, "ran": true, "failed": outcome.failed, "results": outcome.results});
    if !outcome.none.is_empty() {
        out["none"] = json!(outcome.none);
    }
    if !plan.not_checked.is_empty() {
        out["not_checked"] = json!(plan.not_checked);
        out["note"] = json!(format!("Only {MAX_PACKAGES} packages run at once; run again with paths for the rest."));
    }
    if !plan.no_scripts.is_empty() {
        out["no_scripts"] = json!(plan.no_scripts);
    }
    Ok(out.to_string())
}
