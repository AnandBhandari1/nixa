use super::identity::{DEFAULT_HOST, git_repos, read_repo_config};
use super::policy::{lines, plural, tail};
use super::run::{READ_MS, git, read};
use crate::project_tools::find::ProjectSpace;
use regex::Regex;
use serde::Serialize;
use std::path::Path;
use std::sync::LazyLock;

pub const STATE_FILES: usize = 40;
pub const LAST_COMMITS: usize = 5;
pub const OTHER_BRANCHES: usize = 20;
pub const BUSY_FILES: [(&str, &str); 6] = [("MERGE_HEAD", "a merge"), ("rebase-merge", "a rebase"), ("rebase-apply", "a rebase or am"), ("CHERRY_PICK_HEAD", "a cherry-pick"), ("REVERT_HEAD", "a revert"), ("BISECT_LOG", "a bisect")];

static AHEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"ahead (\d+)").expect("ahead regex"));

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub code: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RepoState {
    pub repo: String,
    pub path: String,
    pub branch: String,
    pub head: String,
    pub upstream: String,
    pub ahead: u64,
    pub behind: u64,
    pub changed: usize,
    pub files: Vec<String>,
    pub staged: usize,
    pub untracked: usize,
    pub last_commits: Vec<String>,
    pub unpushed_branches: Vec<String>,
    pub account: String,
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy: Option<String>,
}

pub fn parse_status(stdout: &str) -> Vec<Entry> {
    let parts: Vec<&str> = stdout.split('\0').collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < parts.len() {
        let item = parts[index];
        index += 1;
        if item.len() < 4 || !item.is_char_boundary(2) || !item.is_char_boundary(3) {
            continue;
        }
        let code = item[..2].to_string();
        let moved = code.contains('R') || code.contains('C');
        let from = if moved { parts.get(index).filter(|p| !p.is_empty()).map(|p| p.to_string()) } else { None };
        if moved {
            index += 1;
        }
        out.push(Entry { code, path: item[3..].to_string(), from });
    }
    out
}

pub fn base_name(root: &str) -> String {
    root.trim_end_matches('/').rsplit('/').next().unwrap_or(root).to_string()
}

pub fn named_roots(folder: &str, repos: &[String]) -> Vec<String> {
    repos
        .iter()
        .map(|r| if r.starts_with('/') { r.clone() } else { Path::new(folder).join(r).display().to_string() })
        .filter(|r| Path::new(r).join(".git").exists())
        .collect()
}

fn real(path: &str) -> String {
    std::fs::canonicalize(path).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string())
}

pub async fn repos_in(folder: &str, spaces: &[ProjectSpace]) -> Vec<String> {
    let here = real(folder);
    if let Some(space) = spaces.iter().find(|s| !s.repos.is_empty() && real(&s.folder) == here) {
        let roots = named_roots(folder, &space.repos);
        if !roots.is_empty() {
            return roots;
        }
    }
    let top = git(folder, &["rev-parse", "--show-toplevel"], READ_MS).await;
    if top.ok() && !top.stdout.trim().is_empty() {
        return vec![real(top.stdout.trim())];
    }
    git_repos(folder).await
}

pub fn pick(roots: Vec<String>, folder: &str, repo: Option<&str>, one: bool) -> Result<Vec<String>, String> {
    if roots.is_empty() {
        return Err(format!("{folder} has no git repo, and none of its folders is one."));
    }
    let names = || roots.iter().map(|r| base_name(r)).collect::<Vec<_>>().join(", ");
    if let Some(name) = repo.map(|r| r.trim().to_lowercase()).filter(|r| !r.is_empty()) {
        let found: Vec<String> = roots.iter().filter(|r| base_name(r).to_lowercase() == name).cloned().collect();
        if found.is_empty() {
            return Err(format!("No repo \"{}\" in {folder}. Its repos: {}.", repo.unwrap_or_default().trim(), names()));
        }
        return Ok(found);
    }
    if one && roots.len() > 1 {
        return Err(format!("{folder} holds {} repos: {}. Name one with repo.", roots.len(), names()));
    }
    Ok(roots)
}

pub async fn busy_with(root: &str) -> Option<&'static str> {
    let dir = read(root, &["rev-parse", "--absolute-git-dir"]).await?;
    let dir = dir.trim();
    if dir.is_empty() {
        return None;
    }
    BUSY_FILES.iter().find(|(file, _)| Path::new(dir).join(file).exists()).map(|(_, what)| *what)
}

pub async fn status_of(root: &str) -> Result<Vec<Entry>, String> {
    let out = git(root, &["--no-optional-locks", "status", "--porcelain=v1", "-z", "--untracked-files=all"], READ_MS).await;
    if !out.ok() {
        let why = tail(&out.stderr, 400);
        return Err(format!("git status failed in {}: {}", base_name(root), if why.is_empty() { "no output".into() } else { why }));
    }
    Ok(parse_status(&out.stdout))
}

pub async fn current_branch(root: &str) -> String {
    read(root, &["symbolic-ref", "--short", "-q", "HEAD"]).await.map(|s| s.trim().to_string()).unwrap_or_default()
}

pub async fn upstream_of(root: &str, branch: &str) -> String {
    if branch.is_empty() {
        return String::new();
    }
    let spec = format!("{branch}@{{upstream}}");
    read(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", &spec]).await.map(|s| s.trim().to_string()).unwrap_or_default()
}

pub fn pair_of(text: &str) -> (u64, u64) {
    let nums: Vec<u64> = text.split_whitespace().filter_map(|n| n.parse().ok()).collect();
    if nums.len() == 2 { (nums[0], nums[1]) } else { (0, 0) }
}

pub async fn count_pair(root: &str, range: &str) -> (u64, u64) {
    read(root, &["rev-list", "--left-right", "--count", range]).await.map(|t| pair_of(&t)).unwrap_or((0, 0))
}

pub async fn not_on_remote(root: &str, reference: &str, remote: Option<&str>) -> u64 {
    let flag = remote.map(|r| format!("--remotes={r}")).unwrap_or_else(|| "--remotes".into());
    read(root, &["rev-list", "--count", reference, "--not", &flag]).await.and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

pub async fn other_branches(root: &str, current: &str) -> Vec<String> {
    let refs = read(root, &["for-each-ref", "--format=%(refname:short)%09%(upstream:short)%09%(upstream:track,nobracket)", "refs/heads"]).await.unwrap_or_default();
    let mut out = Vec::new();
    for line in lines(&refs).into_iter().take(OTHER_BRANCHES) {
        let mut parts = line.split('\t');
        let name = parts.next().unwrap_or_default().to_string();
        let upstream = parts.next().unwrap_or_default().to_string();
        let track = parts.next().unwrap_or_default().to_string();
        if name.is_empty() || name == current {
            continue;
        }
        let ahead = AHEAD.captures(&track).and_then(|c| c[1].parse::<usize>().ok());
        if !upstream.is_empty() {
            if let Some(ahead) = ahead {
                out.push(format!("{name} ({} ahead of {upstream})", plural(ahead, "commit")));
            }
        } else {
            let count = not_on_remote(root, &format!("refs/heads/{name}"), None).await;
            if count > 0 {
                out.push(format!("{name} (no upstream, {} not on a remote)", plural(count as usize, "commit")));
            }
        }
    }
    out
}

pub async fn state_of(root: &str) -> Result<RepoState, String> {
    let entries = status_of(root).await?;
    let branch = current_branch(root).await;
    let head = read(root, &["rev-parse", "--short", "HEAD"]).await.map(|s| s.trim().to_string()).unwrap_or_default();
    let upstream = upstream_of(root, &branch).await;
    let (behind, ahead) = if !upstream.is_empty() {
        count_pair(root, &format!("{upstream}...HEAD")).await
    } else if !head.is_empty() {
        (0, not_on_remote(root, "HEAD", None).await)
    } else {
        (0, 0)
    };
    let config = read_repo_config(root, DEFAULT_HOST).await;
    let busy = busy_with(root).await.map(str::to_string);
    let last = read(root, &["log", &format!("-{LAST_COMMITS}"), "--format=%h %ad %s", "--date=short"]).await.unwrap_or_default();
    Ok(RepoState {
        repo: base_name(root),
        path: root.to_string(),
        branch: if !branch.is_empty() { branch.clone() } else if !head.is_empty() { format!("detached at {head}") } else { "no commits yet".into() },
        head,
        upstream,
        ahead,
        behind,
        changed: entries.len(),
        files: entries.iter().take(STATE_FILES).map(|e| format!("{} {}{}", e.code.trim(), e.path, e.from.as_ref().map(|f| format!(" (from {f})")).unwrap_or_default())).collect(),
        staged: entries.iter().filter(|e| !e.code.starts_with(' ') && !e.code.starts_with('?')).count(),
        untracked: entries.iter().filter(|e| e.code == "??").count(),
        last_commits: lines(&last),
        unpushed_branches: other_branches(root, &branch).await,
        account: config.username.unwrap_or_default(),
        profile: config.profile.unwrap_or_default(),
        busy,
    })
}

pub fn relative_to(root: &str, path: &str) -> Result<String, String> {
    let full = if path.starts_with('/') { std::path::PathBuf::from(path) } else { Path::new(root).join(path) };
    let mut parts: Vec<String> = Vec::new();
    for part in full.components() {
        match part {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(name) => parts.push(name.to_string_lossy().to_string()),
            _ => {}
        }
    }
    let clean = format!("/{}", parts.join("/"));
    let base = format!("{}/", root.trim_end_matches('/'));
    match clean.strip_prefix(&base) {
        Some(rel) if !rel.is_empty() => Ok(rel.to_string()),
        _ => Err(format!("{path} is not inside {}.", base_name(root))),
    }
}

pub async fn hashes(root: &str, paths: &[String]) -> Result<Vec<(String, String)>, String> {
    let present: Vec<String> = paths.iter().filter(|p| Path::new(root).join(p).is_file()).cloned().collect();
    let mut out: Vec<(String, String)> = paths.iter().map(|p| (p.clone(), "gone".to_string())).collect();
    if present.is_empty() {
        return Ok(out);
    }
    let mut args = vec!["hash-object".to_string(), "--".to_string()];
    args.extend(present.iter().cloned());
    let hashed = super::run::git_args(root, &args, READ_MS).await;
    if !hashed.ok() {
        return Err(format!("git could not read the files in {}: {}", base_name(root), tail(&hashed.stderr, 400)));
    }
    for (path, hash) in present.iter().zip(lines(&hashed.stdout)) {
        if let Some(slot) = out.iter_mut().find(|(p, _)| p == path) {
            slot.1 = hash;
        }
    }
    Ok(out)
}
