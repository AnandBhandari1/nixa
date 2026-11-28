use std::path::Path;
use std::process::{Command, Output};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInfo {
    pub name: String,
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub repo: String,
    pub top: String,
    pub git_dir: String,
    pub linked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Look {
    Repo(GitInfo),
    NoRepo,
    Unsure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub bare: bool,
    pub prunable: bool,
}

pub type Stamp = Vec<Option<SystemTime>>;

fn git(cwd: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

fn base(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

pub fn repo_name(common: &str) -> String {
    match common.strip_suffix("/.git") {
        Some(parent) => base(parent),
        None => base(common).trim_end_matches(".git").to_string(),
    }
}

pub fn main_top(info: &GitInfo) -> String {
    match info.repo.strip_suffix("/.git") {
        Some(parent) => parent.to_string(),
        None => info.top.clone(),
    }
}

pub fn parse_status(out: &str) -> (Option<String>, u32, u32) {
    let mut branch = None;
    let (mut ahead, mut behind) = (0, 0);
    for line in out.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            branch = Some(head.trim().to_string());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            let mut parts = ab.split_whitespace();
            let num = |p: Option<&str>| {
                p.map(|s| s.trim_start_matches(['+', '-']))
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0)
            };
            ahead = num(parts.next());
            behind = num(parts.next());
        }
    }
    (branch, ahead, behind)
}

pub fn look(cwd: &str) -> Look {
    if cwd.is_empty() || !Path::new(cwd).is_dir() {
        return Look::NoRepo;
    }
    let status = match git(cwd, &["status", "--porcelain=v2", "--branch", "--untracked-files=no"]) {
        Ok(out) => out,
        Err(_) => return Look::Unsure,
    };
    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr).to_lowercase();
        return if err.contains("not a git repository") {
            Look::NoRepo
        } else {
            Look::Unsure
        };
    }
    let paths = git(
        cwd,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--git-common-dir",
            "--absolute-git-dir",
        ],
    );
    let Ok(paths) = paths.map(|o| text(&o.stdout)) else {
        return Look::Unsure;
    };
    let lines: Vec<&str> = paths.lines().map(str::trim).collect();
    let [top, repo, git_dir] = lines[..] else {
        return Look::Unsure;
    };
    let (branch, ahead, behind) = parse_status(&String::from_utf8_lossy(&status.stdout));
    Look::Repo(GitInfo {
        name: repo_name(repo),
        branch,
        ahead,
        behind,
        repo: repo.to_string(),
        top: top.to_string(),
        git_dir: git_dir.to_string(),
        linked: repo != format!("{top}/.git"),
    })
}

pub fn stamp(info: &GitInfo) -> Stamp {
    let at = |path: String| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    vec![
        at(format!("{}/HEAD", info.git_dir)),
        at(format!("{}/logs/HEAD", info.git_dir)),
        at(format!("{}/refs/heads", info.repo)),
        at(format!("{}/refs/remotes", info.repo)),
        at(format!("{}/packed-refs", info.repo)),
        at(format!("{}/FETCH_HEAD", info.repo)),
    ]
}

pub fn parse_worktrees(out: &str) -> Vec<Worktree> {
    let mut list = Vec::new();
    let mut now: Option<Worktree> = None;
    for line in out.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            list.extend(now.take());
            now = Some(Worktree {
                path: path.to_string(),
                branch: None,
                bare: false,
                prunable: false,
            });
        } else if let Some(item) = now.as_mut() {
            if let Some(branch) = line.strip_prefix("branch ") {
                item.branch = Some(branch.trim_start_matches("refs/heads/").to_string());
            } else if line == "bare" {
                item.bare = true;
            } else if line.starts_with("prunable") {
                item.prunable = true;
            }
        }
    }
    list.extend(now);
    list
}

fn run(cwd: &str, args: &[&str]) -> Result<String, String> {
    let out = git(cwd, args).map_err(|e| format!("git could not start: {e}"))?;
    if out.status.success() {
        Ok(text(&out.stdout))
    } else {
        let err = text(&out.stderr);
        Err(if err.is_empty() { format!("git {} failed", args.join(" ")) } else { err })
    }
}

pub fn worktrees(cwd: &str) -> Result<Vec<Worktree>, String> {
    run(cwd, &["worktree", "list", "--porcelain"]).map(|out| parse_worktrees(&out))
}

pub fn safe_branch(branch: &str) -> String {
    branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' })
        .collect::<String>()
        .trim_matches(['-', '.'])
        .to_string()
}

pub fn worktree_path(info: &GitInfo, branch: &str) -> String {
    let main = main_top(info);
    let parent = Path::new(&main)
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "/".to_string());
    format!("{}/{}-{}", parent.trim_end_matches('/'), base(&main), safe_branch(branch))
}

pub fn add_worktree(info: &GitInfo, branch: &str) -> Result<String, String> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Type a branch name.".to_string());
    }
    let main = main_top(info);
    run(&main, &["check-ref-format", "--branch", branch])
        .map_err(|_| format!("{branch} is not a good branch name."))?;
    let path = worktree_path(info, branch);
    if Path::new(&path).exists() {
        return Err(format!("{path} is already there."));
    }
    let known = run(&main, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
    if known {
        run(&main, &["worktree", "add", &path, branch])?;
    } else {
        run(&main, &["worktree", "add", "-b", branch, &path])?;
    }
    Ok(path)
}

pub fn remove_worktree(info: &GitInfo) -> Result<(), String> {
    if !info.linked {
        return Err("This is the main checkout, not a worktree.".to_string());
    }
    let main = main_top(info);
    run(&main, &["worktree", "remove", &info.top]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nixa-git-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn sh(cwd: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn commit(cwd: &Path, name: &str) {
        fs::write(cwd.join(name), name).unwrap();
        sh(cwd, &["add", name]);
        sh(cwd, &["commit", "-qm", name]);
    }

    #[test]
    fn status_lines() {
        let (b, a, c) = parse_status("# branch.oid abc\n# branch.head spike\n# branch.upstream origin/spike\n# branch.ab +2 -1\n");
        assert_eq!((b.as_deref(), a, c), (Some("spike"), 2, 1));
        assert_eq!(parse_status("# branch.head (detached)\n"), (Some("(detached)".into()), 0, 0));
        assert_eq!(repo_name("/p/nixa/.git"), "nixa");
        assert_eq!(repo_name("/p/nixa.git"), "nixa");
        assert_eq!(safe_branch("feat/new ui"), "feat-new-ui");
    }

    #[test]
    fn worktree_lines() {
        let list = parse_worktrees("worktree /p/a\nHEAD 1\nbranch refs/heads/main\n\nworktree /p/a-x\nHEAD 2\nbranch refs/heads/feat/x\n\nworktree /p/b\nHEAD 3\ndetached\nprunable gitdir file points to non-existent location\n");
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].branch.as_deref(), Some("feat/x"));
        assert!(list[2].prunable);
        assert!(list[2].branch.is_none());
    }

    #[test]
    fn a_real_repo_with_a_worktree() {
        let root = temp("repo");
        let origin = root.join("origin.git");
        let main = root.join("proj");
        fs::create_dir_all(&main).unwrap();
        sh(&root, &["init", "-q", "--bare", origin.to_str().unwrap()]);
        sh(&main, &["init", "-q"]);
        commit(&main, "a");
        sh(&main, &["remote", "add", "origin", origin.to_str().unwrap()]);
        sh(&main, &["push", "-q", "-u", "origin", "main"]);
        commit(&main, "b");
        commit(&main, "c");
        let Look::Repo(info) = look(main.to_str().unwrap()) else {
            panic!("not a repo");
        };
        assert_eq!(info.name, "proj");
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!((info.ahead, info.behind), (2, 0));
        assert!(!info.linked);
        let before = stamp(&info);
        assert!(before[0].is_some());
        let plain = temp("plain");
        assert_eq!(look(plain.to_str().unwrap()), Look::NoRepo);
        assert_eq!(look("/no/such/folder"), Look::NoRepo);
        let path = add_worktree(&info, "spike").unwrap();
        assert_eq!(path, root.join("proj-spike").display().to_string());
        assert!(add_worktree(&info, "spike").is_err());
        assert!(add_worktree(&info, "bad..name").is_err());
        let Look::Repo(wt) = look(&path) else {
            panic!("worktree not seen");
        };
        assert!(wt.linked);
        assert_eq!(wt.repo, info.repo);
        assert_eq!(wt.name, "proj");
        assert_eq!(wt.branch.as_deref(), Some("spike"));
        assert_eq!(main_top(&wt), info.top);
        let list = worktrees(&path).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|w| w.path == path && w.branch.as_deref() == Some("spike")));
        assert!(remove_worktree(&info).is_err());
        remove_worktree(&wt).unwrap();
        assert!(!Path::new(&path).exists());
        assert_eq!(worktrees(main.to_str().unwrap()).unwrap().len(), 1);
        sh(&main, &["checkout", "-q", "-b", "other"]);
        assert_ne!(stamp(&info), before);
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&plain).ok();
    }
}
