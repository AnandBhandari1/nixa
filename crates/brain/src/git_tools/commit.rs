use super::identity::{DEFAULT_HOST, prep_for, prepare, read_repo_config};
use super::policy::{clean_message, clip, lines, never_commit, plural, tail};
use super::repo::{base_name, count_pair, current_branch, not_on_remote, relative_to, status_of, upstream_of, busy_with};
use super::run::{git_args, read};
use crate::app::App;
use crate::tools::{Args, strings, text};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

pub const COMMIT_FILES: usize = 400;
pub const COMMIT_MS: u64 = 120_000;

pub fn locked_text(root: &str, spaces: &[crate::project_tools::find::ProjectSpace]) -> Option<String> {
    spaces
        .iter()
        .filter(|s| s.locked && !s.folder.is_empty())
        .find(|s| crate::spaces::folder_in(root, &s.folder) || std::fs::canonicalize(&s.folder).is_ok_and(|f| std::path::Path::new(root).starts_with(f)))
        .map(|s| format!("Space \"{}\" is locked. Nixa does not change it until the user unlocks it.", s.name))
}

pub fn shown_files(files: &[String]) -> String {
    if files.len() <= 3 { files.join(", ") } else { format!("{} and {} more", files[..3].join(", "), files.len() - 3) }
}

pub fn commit_ask(count: usize, repo: &str, branch: &str, files: &[String], message: &str) -> String {
    format!("Commit {} in {repo} on branch {branch} ({}) with the message \"{}\"", plural(count, "file"), shown_files(files), clip(message, 120))
}

pub fn commit_mark(branch: &str, message: &str, sums: &[(String, String)]) -> String {
    let mut parts = vec![branch.to_string(), message.to_string()];
    parts.extend(sums.iter().map(|(p, h)| format!("{p}={h}")));
    crate::util::short_hash(&parts.join("\n"))
}

pub async fn commit(app: &App, root: &str, args: &Args) -> Result<Value, String> {
    let message = clean_message(text(args, "message").as_deref())?;
    let repo = base_name(root);
    if let Some(busy) = busy_with(root).await {
        return Err(format!("{repo} is in the middle of {busy}. Nixa does not commit then. Nothing was committed."));
    }
    let branch = current_branch(root).await;
    if branch.is_empty() {
        return Err(format!("{repo} is not on a branch (detached HEAD). Nothing was committed."));
    }
    let entries = status_of(root).await?;
    let dirty: HashMap<String, super::repo::Entry> = entries.iter().map(|e| (e.path.clone(), e.clone())).collect();
    let mut asked: Vec<String> = Vec::new();
    for item in strings(args, "files").iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let rel = relative_to(root, item)?;
        if !asked.contains(&rel) {
            asked.push(rel);
        }
    }
    let wanted: Vec<String> = if asked.is_empty() { entries.iter().map(|e| e.path.clone()).collect() } else { asked.clone() };
    let not_changed: Vec<String> = asked.iter().filter(|p| !dirty.contains_key(*p)).cloned().collect();
    if !not_changed.is_empty() {
        return Err(format!("These files have no change in {repo}: {}. Nothing was committed.", clip(&not_changed.join(", "), 300)));
    }
    let left_out: Vec<Value> = wanted.iter().filter_map(|p| never_commit(p).map(|why| json!({"file": p, "why": why}))).collect();
    let files: Vec<String> = wanted.iter().filter(|p| never_commit(p).is_none()).cloned().collect();
    if files.is_empty() {
        let note = if dirty.is_empty() { "Nothing to commit: the repo has no changes." } else { "Nothing to commit: every changed file is one Nixa never commits." };
        return Ok(json!({"repo": repo, "path": root, "branch": branch, "committed": false, "note": note, "left_out": left_out}));
    }
    if files.len() > COMMIT_FILES {
        return Err(format!("{} is too many for one commit by Nixa (the limit is {COMMIT_FILES}). Name the files.", plural(files.len(), "file")));
    }
    let ready = prepare(&prep_for(&app.dir, root, false)).await;
    let config = read_repo_config(root, DEFAULT_HOST).await;
    if (config.name.is_none() || config.email.is_none()) && ready.profile.is_none() {
        return Err(format!("{repo} has no git name and email. Pick a profile for it in the Git panel first. Nothing was committed."));
    }
    let fixed = prepare(&prep_for(&app.dir, root, true)).await;
    let to_add: Vec<String> = files.iter().filter(|p| dirty.get(*p).is_some_and(|e| !e.code.ends_with(' '))).cloned().collect();
    let origins: Vec<String> = files.iter().filter_map(|p| dirty.get(p).and_then(|e| e.from.clone())).collect();
    if !to_add.is_empty() {
        let mut add = vec!["--literal-pathspecs".to_string(), "add".into(), "-A".into(), "--".into()];
        add.extend(to_add);
        let added = git_args(root, &add, COMMIT_MS).await;
        if !added.ok() {
            return Err(format!("git add failed in {repo}: {}. Nothing was committed.", tail(&added.stderr, 400)));
        }
    }
    let mut paths: BTreeSet<String> = files.iter().cloned().collect();
    paths.extend(origins);
    let mut made_args = vec!["--literal-pathspecs".to_string(), "commit".into(), "--no-edit".into(), "-m".into(), message.clone(), "--".into()];
    made_args.extend(paths);
    let made = git_args(root, &made_args, COMMIT_MS).await;
    if !made.ok() {
        let why = if made.stderr.trim().is_empty() { &made.stdout } else { &made.stderr };
        return Err(format!("git commit failed in {repo}: {}. Nothing was committed; the files are staged now.", tail(why, 400)));
    }
    let sha = read(root, &["rev-parse", "--short", "HEAD"]).await.map(|s| s.trim().to_string()).unwrap_or_default();
    let in_commit = lines(&read(root, &["show", "--name-only", "--format=", "HEAD"]).await.unwrap_or_default());
    let remaining = status_of(root).await.map(|e| e.len()).unwrap_or(0);
    let upstream = upstream_of(root, &branch).await;
    let ahead = if upstream.is_empty() { not_on_remote(root, "HEAD", None).await } else { count_pair(root, &format!("{upstream}...HEAD")).await.1 };
    let first = message.lines().next().unwrap_or_default().to_string();
    app.info(&format!("Nixa committed {} in {repo} on {branch}: {sha} \"{}\". Not pushed.", plural(in_commit.len(), "file"), clip(&first, 80)));
    crate::util::log("git", &format!("commit in {root} on {branch}: {sha}, {} files", in_commit.len()));
    let mut out = json!({"repo": repo, "path": root, "branch": branch, "committed": true, "commit": sha, "message": first, "files": in_commit, "left_out": left_out, "still_changed": remaining, "ahead": ahead, "pushed": false});
    if !upstream.is_empty() {
        out["upstream"] = json!(upstream);
    }
    if !fixed.notes.is_empty() {
        out["identity"] = json!(fixed.notes);
    }
    Ok(out)
}
