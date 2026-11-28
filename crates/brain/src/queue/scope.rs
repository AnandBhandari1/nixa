use super::prompt::AgentPlan;
use nixa_store::claims::{FileHolder, paths_overlap};
use nixa_store::tasks::{AuthorKind, Task, TaskComment};
use regex::Regex;
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

pub const CHECKED_PREFIX: &str = "Checked by Nixa:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub repo: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub space: String,
    pub scope: Scope,
}

static READ_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[.!?]\s+|\n\s*)(?:(?:read[- ]only|only read)\s*[:.,]|(?:no (?:file )?(?:edits?|changes)|(?:do not|don't|never) (?:edit|change|write) (?:any )?(?:files?|code)\s*[.,;]))").unwrap()
});
static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\b[a-z][a-z\d+.-]*://[^\s<>"'`]+"#).unwrap());
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:^|[\s`"'<>(\[{},;=])((?:\.{1,2}/|/)?[\w@.+-]+(?:/[\w@.+-]+)*/?(?::\d+(?:[-:]\d+)*)?)"#).unwrap()
});
static EXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.[a-z][a-z\d]*(?::\d.*)?$").unwrap());
static LINE_TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":\d+(?:[-:]\d+)*$").unwrap());
static BACKUPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|/)backups/").unwrap());

pub fn read_only_task(title: &str, description: &str) -> bool {
    READ_ONLY.is_match(&format!("{title}\n{description}"))
}

pub fn normal(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

pub fn local_of(repo: &str, path: &str) -> Option<String> {
    let full = if path.starts_with('/') {
        normal(path)
    } else {
        normal(&format!("{repo}/{path}"))
    };
    let root = normal(repo);
    if full == root {
        return Some(String::new());
    }
    let inner = full.strip_prefix(&root)?;
    if root == "/" {
        return Some(full.trim_start_matches('/').to_string());
    }
    inner.strip_prefix('/').map(str::to_string)
}

pub fn found_paths(text: &str) -> Vec<String> {
    let plain = LINK.replace_all(text, "");
    PATH.captures_iter(&plain)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .filter(|path| path.contains('/') || EXT.is_match(path))
        .collect()
}

fn add(files: &mut BTreeSet<String>, repo: &str, path: &str, root: bool) {
    let clean = LINE_TAIL.replace(path, "");
    let clean = clean.trim_end_matches(['.', ':']);
    let Some(local) = local_of(repo, clean) else {
        return;
    };
    if BACKUPS.is_match(&local) {
        return;
    }
    if local.is_empty() {
        if root {
            files.insert(".".into());
        }
        return;
    }
    files.insert(local.trim_end_matches('/').to_string());
}

pub fn work_folder(task: &Task, space_folder: &str) -> String {
    let repo = task.repo.trim().trim_end_matches('/');
    let space = space_folder.trim().trim_end_matches('/');
    if repo.is_empty() || repo.split('/').any(|part| part == "..") {
        return space.to_string();
    }
    if repo.starts_with('/') || space.is_empty() {
        return repo.to_string();
    }
    format!("{space}/{repo}")
}

pub fn repo_of(task: &Task, space_folder: &str) -> String {
    let repo = if task.repo.trim().is_empty() {
        space_folder
    } else {
        task.repo.trim()
    };
    normal(repo)
}

pub fn scope_of(
    task: &Task,
    space_folder: &str,
    comments: &[TaskComment],
    holders: &[FileHolder],
    plan: Option<&AgentPlan>,
) -> Scope {
    let repo = repo_of(task, space_folder);
    let mut files = BTreeSet::new();
    let claimed: Vec<&String> = holders
        .iter()
        .filter(|h| h.task == task.id)
        .flat_map(|h| h.files.iter())
        .collect();
    if !claimed.is_empty() {
        for file in claimed {
            add(&mut files, &repo, file, true);
        }
        return Scope {
            repo,
            files: files.into_iter().collect(),
        };
    }
    if read_only_task(&task.title, &task.description) {
        return Scope {
            repo,
            files: Vec::new(),
        };
    }
    let mut texts: Vec<String> = vec![task.title.clone(), task.description.clone()];
    for comment in comments {
        if matches!(comment.author_kind, AuthorKind::Nixa | AuthorKind::System)
            && comment.text.starts_with(CHECKED_PREFIX)
        {
            continue;
        }
        texts.push(comment.text.clone());
        for file in &comment.links.files {
            add(&mut files, &repo, file, false);
        }
    }
    if let Some(plan) = plan {
        for file in &plan.allowed_files {
            add(&mut files, &repo, file, false);
        }
        texts.extend(plan.steps.iter().cloned());
    }
    for text in &texts {
        for path in found_paths(text) {
            add(&mut files, &repo, &path, false);
        }
    }
    Scope {
        repo,
        files: files.into_iter().collect(),
    }
}

pub fn overlap(a: &Scope, b: &Scope) -> bool {
    a.repo == b.repo
        && a.files.iter().any(|x| {
            b.files
                .iter()
                .any(|y| x == "." || y == "." || paths_overlap(x, y))
        })
}

pub struct Wait<'a> {
    pub task: i64,
    pub space: &'a str,
    pub scope: &'a Scope,
    pub active: &'a [(i64, Scope)],
    pub launches: &'a HashMap<i64, Launch>,
    pub doing: &'a [i64],
    pub max_doing: i64,
}

pub fn start_wait(wait: &Wait) -> Option<String> {
    for (id, held) in wait.active {
        if *id != wait.task && overlap(held, wait.scope) {
            return Some(format!("files overlap with task #{id}"));
        }
    }
    let mut ids: Vec<&i64> = wait.launches.keys().collect();
    ids.sort();
    for id in ids {
        let held = &wait.launches[id];
        if *id != wait.task && overlap(&held.scope, wait.scope) {
            return Some(format!("files overlap with task #{id}"));
        }
    }
    let doing = wait.doing.iter().filter(|id| **id != wait.task).count();
    let pending = wait
        .launches
        .iter()
        .filter(|(id, l)| **id != wait.task && l.space == wait.space && !wait.doing.contains(id))
        .count();
    if (doing + pending) as i64 >= wait.max_doing.max(1) {
        return Some("the space has reached its parallel task limit".into());
    }
    None
}
