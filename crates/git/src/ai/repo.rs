use super::prompts::title_from_branch;
use super::text::first_useful_line;
use std::path::Path;

pub const REVIEW_FILES: &[&str] = &[".nixa/review.md", ".angkorgit/review.md"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitOut {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub fn git(root: &Path, args: &[&str]) -> GitOut {
    let out = crate::run::git(root, args);
    GitOut {
        code: out.code,
        stdout: out.stdout,
        stderr: out.stderr,
    }
}

pub fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = git(root, args);
    if out.code == Some(0) {
        return Ok(out.stdout);
    }
    let line = first_useful_line(&out.stderr);
    Err(if line.is_empty() {
        format!("git {} failed", args.first().copied().unwrap_or(""))
    } else {
        line
    })
}

pub fn staged_diff(root: &Path) -> Result<String, String> {
    git_text(root, &["diff", "--cached", "--no-color", "--no-ext-diff"])
}

pub fn current_branch(root: &Path) -> Option<String> {
    let out = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let name = out.stdout.trim();
    (out.code == Some(0) && !name.is_empty()).then(|| name.to_string())
}

pub fn review_rules(root: &Path) -> String {
    REVIEW_FILES
        .iter()
        .filter_map(|name| std::fs::read_to_string(root.join(name)).ok())
        .find(|text| !text.trim().is_empty())
        .unwrap_or_default()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitContext {
    pub oid: String,
    pub summary: String,
    pub files: Vec<String>,
    pub patch: String,
}

pub fn commit_context(root: &Path, sha: &str) -> Result<CommitContext, String> {
    let head = git_text(root, &["show", "-s", "--no-color", "--format=%H%x1f%s", sha, "--"])?;
    let mut parts = head.trim().splitn(2, '\u{1f}');
    let oid = parts.next().unwrap_or("").to_string();
    let summary = parts.next().unwrap_or("").to_string();
    let patch = git_text(root, &["show", "--no-color", "--no-ext-diff", "-m", "--first-parent", "--format=", sha, "--"])?;
    let names = git_text(root, &["show", "--no-color", "-m", "--first-parent", "--name-only", "--format=", sha, "--"])?;
    let files = names.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    Ok(CommitContext { oid, summary, files, patch })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogEntry {
    pub oid: String,
    pub summary: String,
    pub body: String,
}

pub fn log_entries(root: &Path, args: &[&str]) -> Vec<LogEntry> {
    let mut all = vec!["log", "--no-color", "--format=%h%x1f%s%x1f%b%x1e"];
    all.extend_from_slice(args);
    let out = git(root, &all);
    if out.code != Some(0) {
        return vec![];
    }
    out.stdout
        .split('\u{1e}')
        .map(|chunk| chunk.trim_start_matches('\n'))
        .filter(|chunk| !chunk.trim().is_empty())
        .map(|chunk| {
            let mut p = chunk.split('\u{1f}');
            LogEntry {
                oid: p.next().unwrap_or("").trim().to_string(),
                summary: p.next().unwrap_or("").trim().to_string(),
                body: p.next().unwrap_or("").trim().to_string(),
            }
        })
        .collect()
}

fn ref_exists(root: &Path, name: &str) -> bool {
    let spec = format!("{name}^{{commit}}");
    git(root, &["rev-parse", "--verify", "--quiet", &spec]).code == Some(0)
}

pub fn resolve_base(root: &Path, base: Option<&str>) -> Option<String> {
    let name = match base.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => b.to_string(),
        None => {
            let out = git(root, &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]);
            if out.code != Some(0) {
                return None;
            }
            out.stdout.trim().to_string()
        }
    };
    if name.is_empty() || name.starts_with('-') {
        return None;
    }
    let candidates = if name.contains('/') {
        vec![name.clone(), format!("origin/{name}")]
    } else {
        vec![format!("origin/{name}"), name.clone()]
    };
    candidates.into_iter().find(|r| ref_exists(root, r))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrContext {
    pub branch: Option<String>,
    pub base_ref: Option<String>,
    pub commits: Vec<LogEntry>,
    pub commits_text: String,
    pub diff_stat: String,
    pub title: String,
}

pub fn pr_context(root: &Path, base: Option<&str>, branch: Option<&str>) -> PrContext {
    let branch = branch.map(str::to_string).or_else(|| current_branch(root));
    let base_ref = resolve_base(root, base);
    let ahead = match &base_ref {
        Some(b) => log_entries(root, &["--reverse", "-n", "100", &format!("{b}..HEAD")]),
        None => vec![],
    };
    let commits: Vec<LogEntry> = if ahead.is_empty() {
        log_entries(root, &["-n", "15", "HEAD"])
            .into_iter()
            .map(|c| LogEntry { body: String::new(), ..c })
            .collect()
    } else {
        ahead.clone()
    };
    let commits_text = commits
        .iter()
        .map(|c| {
            if c.body.is_empty() {
                format!("{} {}", c.oid, c.summary)
            } else {
                format!("{} {}\n{}", c.oid, c.summary, c.body)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let diff_stat = match (&base_ref, ahead.is_empty()) {
        (Some(b), false) => git_text(root, &["diff", "--stat", "--no-color", &format!("{b}...HEAD")]).unwrap_or_default(),
        _ => String::new(),
    };
    let title = if ahead.len() == 1 {
        ahead[0].summary.clone()
    } else {
        let from_branch = title_from_branch(branch.as_deref().unwrap_or(""));
        if from_branch.is_empty() {
            commits.first().map(|c| c.summary.clone()).unwrap_or_default()
        } else {
            from_branch
        }
    };
    PrContext {
        branch,
        base_ref,
        commits,
        commits_text,
        diff_stat: diff_stat.trim_end().to_string(),
        title,
    }
}

#[cfg(test)]
pub mod testrepo {
    use super::git;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    pub struct Repo(pub PathBuf);

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub fn run(root: &Path, args: &[&str]) {
        let out = git(root, args);
        assert_eq!(out.code, Some(0), "git {args:?}: {}", out.stderr);
    }

    pub fn new() -> Repo {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("nixa-git-ai-repo-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        run(&dir, &["init", "-q", "-b", "main"]);
        run(&dir, &["config", "user.name", "Test"]);
        run(&dir, &["config", "user.email", "test@example.invalid"]);
        run(&dir, &["config", "commit.gpgsign", "false"]);
        Repo(dir)
    }

    pub fn commit(root: &Path, file: &str, text: &str, message: &str) {
        fs::write(root.join(file), text).unwrap();
        run(root, &["add", file]);
        run(root, &["commit", "-q", "-m", message]);
    }
}

#[cfg(test)]
mod tests {
    use super::testrepo::{self, commit, run};
    use super::*;
    use std::fs;

    #[test]
    fn staged_diff_and_branch() {
        let repo = testrepo::new();
        let root = repo.0.as_path();
        commit(root, "a.txt", "one\n", "first");
        assert_eq!(staged_diff(root).unwrap(), "");
        fs::write(root.join("a.txt"), "two\n").unwrap();
        run(root, &["add", "a.txt"]);
        assert!(staged_diff(root).unwrap().contains("+two"));
        assert_eq!(current_branch(root).as_deref(), Some("main"));
        run(root, &["checkout", "-q", "--detach"]);
        assert_eq!(current_branch(root), None);
    }

    #[test]
    fn commit_context_lists_files() {
        let repo = testrepo::new();
        let root = repo.0.as_path();
        commit(root, "a.txt", "one\n", "first");
        fs::write(root.join("b.txt"), "b\n").unwrap();
        commit(root, "a.txt", "two\n", "second change");
        run(root, &["add", "b.txt"]);
        run(root, &["commit", "-q", "--amend", "--no-edit"]);
        let c = commit_context(root, "HEAD").unwrap();
        assert_eq!(c.summary, "second change");
        assert_eq!(c.oid.len(), 40);
        assert_eq!(c.files, vec!["a.txt", "b.txt"]);
        assert!(c.patch.contains("+two"));
        assert!(commit_context(root, "nope-ref").is_err());
    }

    #[test]
    fn review_rules_from_the_repo() {
        let repo = testrepo::new();
        let root = repo.0.as_path();
        assert_eq!(review_rules(root), "");
        fs::create_dir_all(root.join(".angkorgit")).unwrap();
        fs::write(root.join(".angkorgit/review.md"), "second").unwrap();
        assert_eq!(review_rules(root), "second");
        fs::create_dir_all(root.join(".nixa")).unwrap();
        fs::write(root.join(".nixa/review.md"), " \n").unwrap();
        assert_eq!(review_rules(root), "second");
        fs::write(root.join(".nixa/review.md"), "first").unwrap();
        assert_eq!(review_rules(root), "first");
    }

    #[test]
    fn pr_context_uses_the_base() {
        let repo = testrepo::new();
        let root = repo.0.as_path();
        commit(root, "a.txt", "one\n", "base work");
        run(root, &["checkout", "-q", "-b", "feature/add-login"]);
        commit(root, "b.txt", "b\n", "Add login form");
        let one = pr_context(root, Some("main"), None);
        assert_eq!(one.base_ref.as_deref(), Some("main"));
        assert_eq!(one.title, "Add login form");
        assert!(one.diff_stat.contains("b.txt"));
        commit(root, "c.txt", "c\n", "Add login check");
        let two = pr_context(root, Some("main"), None);
        assert_eq!(two.title, "Add login");
        assert_eq!(two.commits.len(), 2);
        assert!(two.commits_text.contains("Add login form"));
        let none = pr_context(root, Some("nope"), None);
        assert_eq!(none.base_ref, None);
        assert_eq!(none.commits.len(), 3);
        assert_eq!(none.diff_stat, "");
        assert_eq!(resolve_base(root, Some("--evil")), None);
    }
}
