use super::run::{self, lines, read, read_ok, split_z};
use super::words::{Outcome, is_full_sha, refusal};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Op {
    pub fn word(self) -> &'static str {
        match self {
            Op::Merge => "Merge",
            Op::Rebase => "Rebase",
            Op::CherryPick => "Cherry-pick",
            Op::Revert => "Revert",
        }
    }

    pub fn lower(self) -> String {
        self.word().to_lowercase()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoState {
    pub op: Option<Op>,
    pub step: Option<(usize, usize)>,
    pub conflicts: Vec<String>,
    pub branch: Option<String>,
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default().trim().to_string()
}

pub fn git_paths(root: &Path, names: &[&str]) -> Result<Vec<PathBuf>, String> {
    let mut args = vec!["rev-parse"];
    for n in names {
        args.push("--git-path");
        args.push(n);
    }
    let text = read_ok(root, &args)?;
    let found: Vec<PathBuf> = text
        .lines()
        .map(|l| {
            let p = PathBuf::from(l);
            if p.is_absolute() { p } else { root.join(p) }
        })
        .collect();
    if found.len() == names.len() {
        Ok(found)
    } else {
        Err("cannot read the repo state".into())
    }
}

pub fn first_todo(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or_default()
        .to_string()
}

pub fn read_state(root: &Path) -> Result<RepoState, String> {
    let names = [
        "MERGE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "sequencer/todo",
    ];
    let p = git_paths(root, &names)?;
    let has = |i: usize| p[i].exists();
    let dir = if p[1].is_dir() {
        Some(&p[1])
    } else if p[2].is_dir() {
        Some(&p[2])
    } else {
        None
    };
    let am = dir.is_some_and(|d| d.join("applying").exists());
    let op = if (has(1) || has(2)) && !am {
        Some(Op::Rebase)
    } else if has(1) || has(2) {
        None
    } else if has(0) {
        Some(Op::Merge)
    } else if has(3) {
        Some(Op::CherryPick)
    } else if has(4) {
        Some(Op::Revert)
    } else if has(5) {
        match first_todo(&read_text(&p[5])).as_str() {
            "revert" => Some(Op::Revert),
            "pick" | "p" => Some(Op::CherryPick),
            _ => None,
        }
    } else {
        None
    };
    let step = match (op, dir) {
        (Some(Op::Rebase), Some(d)) => {
            let (n, total) = if d == &p[1] {
                (read_text(&d.join("msgnum")), read_text(&d.join("end")))
            } else {
                (read_text(&d.join("next")), read_text(&d.join("last")))
            };
            match (n.parse::<usize>(), total.parse::<usize>()) {
                (Ok(n), Ok(t)) if t > 0 => Some((n, t)),
                _ => None,
            }
        }
        _ => None,
    };
    let from_rebase = match (op, dir) {
        (Some(Op::Rebase), Some(d)) => read_text(&d.join("head-name"))
            .strip_prefix("refs/heads/")
            .map(str::to_string),
        _ => None,
    };
    let conflicts = split_z(&read(root, &["diff", "--name-only", "--diff-filter=U", "-z"]).stdout);
    Ok(RepoState {
        op,
        step,
        conflicts,
        branch: from_rebase.or_else(|| current_branch(root)),
    })
}

pub fn busy_refusal(op: Op) -> Outcome {
    refusal(
        "busy",
        format!("Finish the {} first", op.lower()),
        "Continue or cancel it in the Changes panel. Nothing was changed.",
    )
}

pub fn op_gate(root: &Path) -> Result<Option<Outcome>, String> {
    Ok(read_state(root)?.op.map(busy_refusal))
}

pub fn current_branch(root: &Path) -> Option<String> {
    let out = run::git(root, &["symbolic-ref", "-q", "--short", "HEAD"]);
    let name = out.line();
    (out.ok() && !name.is_empty()).then_some(name)
}

pub fn upstream_of(root: &Path) -> Option<String> {
    let out = run::git(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]);
    let name = out.line();
    (out.ok() && !name.is_empty()).then_some(name)
}

pub fn rev_problem(rev: &str) -> Option<&'static str> {
    if rev.trim().is_empty() {
        return Some("No commit was given.");
    }
    if rev.starts_with('-') || rev.chars().any(|c| c.is_whitespace() || c == '\0') || rev.contains("..") {
        return Some("That is not a commit id.");
    }
    None
}

pub fn resolve_commit(root: &Path, rev: &str) -> Option<String> {
    if rev_problem(rev).is_some() {
        return None;
    }
    let spec = format!("{rev}^{{commit}}");
    let out = read(root, &["rev-parse", "-q", "--verify", "--end-of-options", &spec]);
    let sha = out.line();
    (out.ok() && is_full_sha(&sha)).then_some(sha)
}

pub fn commit_of(root: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    let out = read(root, &["rev-parse", "--verify", "-q", "--end-of-options", &spec]);
    let sha = out.line();
    (out.ok() && !sha.is_empty()).then_some(sha)
}

pub fn head_sha(root: &Path) -> Option<String> {
    let out = read(root, &["rev-parse", "-q", "--verify", "HEAD"]);
    let sha = out.line();
    (out.ok() && is_full_sha(&sha)).then_some(sha)
}

pub fn is_ancestor(root: &Path, older: &str, newer: &str) -> Result<bool, String> {
    let out = read(root, &["merge-base", "--is-ancestor", older, newer]);
    match out.code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(run::non_empty(out.said(), "cannot compare the commits")),
    }
}

pub fn on_line(root: &Path, sha: &str, head: Option<&str>) -> bool {
    match head {
        Some(h) => sha == h || is_ancestor(root, sha, h).unwrap_or(false),
        None => false,
    }
}

pub fn count_revs(root: &Path, args: &[&str]) -> usize {
    let mut all = vec!["rev-list", "--count"];
    all.extend_from_slice(args);
    read_ok(root, &all)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

pub fn count_strict(root: &Path, args: &[&str]) -> Result<usize, String> {
    let mut all = vec!["rev-list", "--count"];
    all.extend_from_slice(args);
    let text = read_ok(root, &all)?;
    Ok(text.trim().parse().unwrap_or(0))
}

pub fn count_both(root: &Path, a: &str, b: &str) -> Result<(usize, usize), String> {
    let range = format!("{a}...{b}");
    let text = read_ok(root, &["rev-list", "--left-right", "--count", &range])?;
    let mut parts = text.split_whitespace().map(|v| v.parse::<usize>().unwrap_or(0));
    Ok((parts.next().unwrap_or(0), parts.next().unwrap_or(0)))
}

pub fn parents_of(root: &Path, sha: &str) -> Vec<String> {
    read_ok(root, &["rev-list", "--parents", "-n", "1", sha])
        .ok()
        .and_then(|t| lines(&t).into_iter().next())
        .map(|l| l.split(' ').skip(1).map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn remote_branches_with(root: &Path, sha: &str) -> Vec<String> {
    let text = read_ok(
        root,
        &["for-each-ref", "--contains", sha, "--format=%(refname)", "refs/remotes"],
    )
    .unwrap_or_default();
    lines(&text)
        .into_iter()
        .filter(|r| !r.ends_with("/HEAD"))
        .filter_map(|r| r.strip_prefix("refs/remotes/").map(str::to_string))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefsAt {
    pub branches: Vec<String>,
    pub tags: Vec<String>,
}

pub fn refs_at(root: &Path, sha: &str) -> RefsAt {
    let text = read_ok(
        root,
        &["for-each-ref", "--points-at", sha, "--format=%(refname)", "refs/heads", "refs/tags"],
    )
    .unwrap_or_default();
    let all = lines(&text);
    RefsAt {
        branches: all.iter().filter_map(|r| r.strip_prefix("refs/heads/").map(str::to_string)).collect(),
        tags: all.iter().filter_map(|r| r.strip_prefix("refs/tags/").map(str::to_string)).collect(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub fetch_url: Option<String>,
    pub push_url: Option<String>,
    pub web_url: Option<String>,
}

pub fn list_remotes(root: &Path) -> Vec<Remote> {
    let text = read_ok(root, &["remote", "-v"]).unwrap_or_default();
    let mut out: Vec<Remote> = Vec::new();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once('\t') else {
            continue;
        };
        let (url, kind) = if let Some(u) = rest.strip_suffix(" (fetch)") {
            (u, "fetch")
        } else if let Some(u) = rest.strip_suffix(" (push)") {
            (u, "push")
        } else {
            continue;
        };
        let at = match out.iter().position(|r| r.name == name) {
            Some(i) => i,
            None => {
                out.push(Remote {
                    name: name.to_string(),
                    ..Remote::default()
                });
                out.len() - 1
            }
        };
        if kind == "fetch" {
            out[at].fetch_url = Some(url.to_string());
        } else {
            out[at].push_url = Some(url.to_string());
        }
    }
    for r in &mut out {
        r.web_url = web_url(r.fetch_url.as_deref().or(r.push_url.as_deref()).unwrap_or_default());
    }
    out
}

fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn known_host(host: &str) -> String {
    let low = host.to_lowercase();
    for base in ["github.com", "gitlab.com", "bitbucket.org"] {
        if let Some(rest) = low.strip_prefix(base)
            && (rest.starts_with('-') || rest.starts_with('_'))
            && rest.len() > 1
        {
            return base.to_string();
        }
    }
    low
}

pub fn web_url(url: &str) -> Option<String> {
    let u = url.trim();
    if u.is_empty() || u.chars().any(|c| c.is_whitespace() || (c as u32) < 0x20 || c == '\u{7f}') {
        return None;
    }
    if let Some(cut) = u.find("::")
        && u[..cut].chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    {
        return None;
    }
    let mut scheme = "https";
    let (host, path) = if let Some(cut) = u.find("://") {
        let proto = u[..cut].to_lowercase();
        if !["ssh", "git", "https", "http", "git+ssh", "ssh+git"].contains(&proto.as_str()) {
            return None;
        }
        if proto == "http" {
            scheme = "http";
        }
        let rest = &u[cut + 3..];
        let slash = rest.find('/')?;
        let auth = &rest[..slash];
        let hostport = auth.rsplit('@').next().unwrap_or(auth);
        let host = hostport.split(':').next().unwrap_or(hostport);
        let path = rest[slash + 1..].split(['?', '#']).next().unwrap_or_default();
        (host.to_string(), path.to_string())
    } else {
        let after = u.split_once('@').map_or(u, |(user, rest)| {
            if user.contains('/') || user.contains(':') { u } else { rest }
        });
        let (host, path) = after.split_once(':')?;
        if host.contains('/') || host.contains('@') || path.starts_with('/') {
            return None;
        }
        (host.to_string(), path.split(['?', '#']).next().unwrap_or_default().to_string())
    };
    if !host_ok(&host) || path.is_empty() {
        return None;
    }
    let host = known_host(&host);
    let mut path = path.trim_matches('/').to_string();
    if path.to_lowercase().ends_with(".git") {
        path.truncate(path.len() - 4);
    }
    let path = path.trim_end_matches('/').to_string();
    if path.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}/{path}"))
}

fn host_of(url: &str) -> String {
    url.split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default().to_lowercase())
        .unwrap_or_default()
}

pub fn commit_url(base: &str, sha: &str) -> String {
    let host = host_of(base);
    if host.contains("gitlab") {
        format!("{base}/-/commit/{sha}")
    } else if host == "bitbucket.org" {
        format!("{base}/commits/{sha}")
    } else {
        format!("{base}/commit/{sha}")
    }
}

pub fn forge_name(host: &str) -> String {
    let h = host.to_lowercase();
    if h == "github.com" || h.starts_with("github.") {
        "GitHub".into()
    } else if h.contains("gitlab") {
        "GitLab".into()
    } else if h == "bitbucket.org" {
        "Bitbucket".into()
    } else if h == "codeberg.org" {
        "Codeberg".into()
    } else if h.is_empty() {
        "the web".into()
    } else {
        h
    }
}

pub fn pick_web_remote<'a>(remotes: &'a [Remote], held: &[String], upstream: Option<&str>) -> Option<&'a Remote> {
    let usable: Vec<&Remote> = remotes.iter().filter(|r| r.web_url.is_some()).collect();
    let holds = |r: &Remote| held.iter().any(|b| b.starts_with(&format!("{}/", r.name)));
    let up_name = usable
        .iter()
        .map(|r| r.name.as_str())
        .filter(|n| upstream.is_some_and(|u| u.starts_with(&format!("{n}/"))))
        .max_by_key(|n| n.len());
    let tiers: [&dyn Fn(&Remote) -> bool; 6] = [
        &|r| holds(r) && Some(r.name.as_str()) == up_name,
        &|r| holds(r) && r.name == "origin",
        &|r| holds(r),
        &|r| Some(r.name.as_str()) == up_name,
        &|r| r.name == "origin",
        &|_| true,
    ];
    tiers
        .iter()
        .find_map(|test| usable.iter().find(|r| test(r)).copied())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Web {
    pub remote: String,
    pub name: String,
    pub url: String,
}

pub fn web_for(remotes: &[Remote], held: &[String], upstream: Option<&str>, sha: &str) -> Option<Web> {
    let r = pick_web_remote(remotes, held, upstream)?;
    let base = r.web_url.clone()?;
    Some(Web {
        remote: r.name.clone(),
        name: forge_name(&host_of(&base)),
        url: commit_url(&base, sha),
    })
}

pub const HOOK_NAMES: [&str; 4] = ["pre-commit", "prepare-commit-msg", "commit-msg", "post-commit"];

pub fn commit_hooks(root: &Path) -> Vec<String> {
    let Ok(dirs) = git_paths(root, &["hooks"]) else {
        return Vec::new();
    };
    HOOK_NAMES
        .iter()
        .filter(|name| {
            fs::metadata(dirs[0].join(name))
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
        .map(|n| n.to_string())
        .collect()
}

pub struct TempDir(pub PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn temp_dir() -> Result<TempDir, String> {
    let base = std::env::temp_dir();
    for n in 0..50u32 {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = base.join(format!("nixa-git-hist.{}.{stamp}.{n}", std::process::id()));
        if fs::create_dir(&dir).is_ok() {
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
            return Ok(TempDir(dir));
        }
    }
    Err("cannot make a folder for the message".into())
}

pub fn status_paths(text: &str) -> Vec<String> {
    let parts: Vec<&str> = text.split('\0').collect();
    let mut files = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let entry = parts[i];
        if entry.len() >= 4 {
            files.push(entry[3..].to_string());
            let b = entry.as_bytes();
            if b[0] == b'R' || b[0] == b'C' || b[1] == b'R' || b[1] == b'C' {
                i += 1;
            }
        }
        i += 1;
    }
    files
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusRow {
    pub x: char,
    pub y: char,
    pub path: String,
}

pub fn status_rows(text: &str) -> Vec<StatusRow> {
    let parts: Vec<&str> = text.split('\0').collect();
    let mut rows = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let entry = parts[i];
        if entry.len() >= 4 {
            let mut c = entry.chars();
            let x = c.next().unwrap_or(' ');
            let y = c.next().unwrap_or(' ');
            rows.push(StatusRow {
                x,
                y,
                path: entry[3..].to_string(),
            });
            if x == 'R' || x == 'C' || y == 'R' || y == 'C' {
                i += 1;
            }
        }
        i += 1;
    }
    rows
}

pub fn tracked_rows(root: &Path) -> Vec<StatusRow> {
    let out = read(root, &["status", "--porcelain=v1", "-z", "--untracked-files=no"]);
    if out.ok() { status_rows(&out.stdout) } else { Vec::new() }
}
