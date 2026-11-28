use crate::ops::{State, read_state, split_z};
use crate::outcome::{Outcome, failure, plural, success};
use crate::run::{Out, git};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const READ: [&str; 6] = ["--literal-pathspecs", "--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "diff.autoRefreshIndex=false"];

pub const MAX_FILES: usize = 1000;

pub const MAX_REPOS: usize = 24;

const SKIP_DIRS: [&str; 5] = ["node_modules", "vendor", "target", "dist", "build"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub folder: String,
    pub status: char,
    pub fresh: bool,
    pub conflict: bool,
    pub x: char,
    pub y: char,
    pub staged: bool,
    pub add: u64,
    pub del: u64,
    pub binary: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stat {
    pub add: u64,
    pub del: u64,
    pub binary: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub branch: Option<String>,
    pub detached: bool,
    pub unborn: bool,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub root: PathBuf,
    pub header: Header,
    pub files: Vec<FileChange>,
    pub limited: bool,
    pub has_head: bool,
    pub head: Option<String>,
    pub has_remote: bool,
    pub state: Option<State>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    pub path: PathBuf,
    pub count: usize,
}

pub fn read_git(root: &Path, args: &[&str]) -> Out {
    let mut all = READ.to_vec();
    all.extend_from_slice(args);
    git(root, &all)
}

pub fn folder_of(path: &str) -> String {
    path.rfind('/').map(|i| path[..i].to_string()).unwrap_or_default()
}

pub fn status_file(entry: &str, stats: &HashMap<String, Stat>) -> FileChange {
    let mut chars = entry.chars();
    let x = chars.next().unwrap_or(' ');
    let y = chars.next().unwrap_or(' ');
    let path = entry.get(3..).unwrap_or("").to_string();
    let xy = format!("{x}{y}");
    let fresh = xy == "??";
    let conflict = xy.contains('U') || xy == "AA" || xy == "DD";
    let status = if conflict {
        'U'
    } else if xy.contains('D') {
        'D'
    } else if fresh || xy.contains('A') {
        'A'
    } else {
        'M'
    };
    let stat = stats.get(&path).copied().unwrap_or_default();
    FileChange {
        folder: folder_of(&path),
        status,
        fresh,
        conflict,
        x,
        y,
        staged: !conflict && x != ' ' && x != '?',
        add: stat.add,
        del: stat.del,
        binary: stat.binary,
        path,
    }
}

pub fn parse_header(header: Option<&str>) -> Header {
    let mut out = Header::default();
    let Some(header) = header else { return out };
    if let Some(name) = header.strip_prefix("No commits yet on ") {
        out.unborn = true;
        out.branch = Some(name.trim().to_string());
        return out;
    }
    if header.starts_with("HEAD (no branch)") {
        out.detached = true;
        return out;
    }
    if let (Some(open), Some(close)) = (header.find('['), header.rfind(']'))
        && open < close
    {
        let track = &header[open + 1..close];
        let count = |word: &str| {
            track.split(", ").find_map(|part| part.strip_prefix(word).and_then(|n| n.trim().parse().ok())).unwrap_or(0)
        };
        out.ahead = Some(count("ahead "));
        out.behind = Some(count("behind "));
    }
    match header.find("...") {
        Some(dots) => {
            out.branch = Some(header[..dots].trim().to_string());
            out.upstream = header[dots + 3..].split(' ').next().map(String::from);
        }
        None => out.branch = Some(header.trim().to_string()),
    }
    out
}

pub fn parse_numstat(text: &str, stats: &mut HashMap<String, Stat>) {
    for entry in text.split('\0') {
        let mut parts = entry.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let old = stats.get(path).copied().unwrap_or_default();
        stats.insert(
            path.to_string(),
            Stat { add: old.add + a.parse::<u64>().unwrap_or(0), del: old.del + d.parse::<u64>().unwrap_or(0), binary: a == "-" },
        );
    }
}

fn need(out: Out) -> Result<Out, String> {
    if out.ok() { Ok(out) } else { Err(out.stderr.trim().to_string()) }
}

pub fn repo_root(cwd: &Path) -> Option<PathBuf> {
    let out = git(cwd, &["rev-parse", "--show-toplevel"]);
    let text = out.stdout.trim();
    if out.ok() && !text.is_empty() { Some(PathBuf::from(text)) } else { None }
}

pub fn status(root: &Path) -> Result<Status, String> {
    let out = need(read_git(root, &["status", "--porcelain=v1", "-z", "-b", "--no-renames", "--untracked-files=all"]))?;
    let head = read_git(root, &["rev-parse", "--verify", "HEAD"]);
    let remote = read_git(root, &["remote"]);
    let state = read_state(root).ok();
    let mut stats = HashMap::new();
    if head.ok() {
        parse_numstat(&need(read_git(root, &["diff", "HEAD", "--numstat", "-z", "--no-renames"]))?.stdout, &mut stats);
    } else {
        parse_numstat(&need(read_git(root, &["diff", "--cached", "--numstat", "-z", "--no-renames"]))?.stdout, &mut stats);
        parse_numstat(&need(read_git(root, &["diff", "--numstat", "-z", "--no-renames"]))?.stdout, &mut stats);
    }
    let mut entries = split_z(&out.stdout);
    let header = if entries.first().is_some_and(|e| e.starts_with("## ")) { Some(entries.remove(0)[3..].to_string()) } else { None };
    let files = entries.iter().take(MAX_FILES).map(|e| status_file(e, &stats)).collect();
    Ok(Status {
        root: root.to_path_buf(),
        header: parse_header(header.as_deref()),
        files,
        limited: entries.len() > MAX_FILES,
        has_head: head.ok(),
        head: if head.ok() { Some(head.stdout.trim().to_string()) } else { None },
        has_remote: !remote.stdout.trim().is_empty(),
        state,
    })
}

pub fn file_status(root: &Path, path: &str) -> Result<Option<FileChange>, String> {
    let out = need(read_git(root, &["status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all", "--", path]))?;
    let Some(entry) = split_z(&out.stdout).into_iter().find(|e| e.get(3..) == Some(path)) else { return Ok(None) };
    let stat = read_git(root, &["diff", "HEAD", "--numstat", "--no-renames", "--", path]);
    let mut stats = HashMap::new();
    let line = stat.stdout.trim_end_matches('\n').to_string();
    let mut parts = line.split('\t');
    let a = parts.next().unwrap_or("");
    let d = parts.next().unwrap_or("");
    stats.insert(path.to_string(), Stat { add: a.parse().unwrap_or(0), del: d.parse().unwrap_or(0), binary: a == "-" });
    Ok(Some(status_file(&entry, &stats)))
}

fn repo_entry(dir: &Path, name: &str) -> Option<RepoEntry> {
    let top = repo_root(dir)?;
    let same = top.canonicalize().ok()? == dir.canonicalize().ok()?;
    if !same {
        return None;
    }
    let out = git(dir, &["--no-optional-locks", "-c", "core.fsmonitor=false", "status", "--porcelain", "-u", "no"]);
    let count = out.stdout.lines().filter(|l| !l.is_empty()).count();
    Some(RepoEntry { name: name.to_string(), path: dir.to_path_buf(), count })
}

pub fn discover_repos(folder: &Path) -> Vec<RepoEntry> {
    let mut repos = Vec::new();
    let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(entry) = repo_entry(folder, &name) {
        repos.push(entry);
    }
    let Ok(read) = std::fs::read_dir(folder) else { return repos };
    let mut children: Vec<(String, PathBuf)> = read
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| !n.starts_with('.') && !SKIP_DIRS.contains(&n.as_str()))
        .collect();
    children.sort();
    for (child, path) in children.into_iter().take(MAX_REPOS) {
        if let Some(entry) = repo_entry(&path, &child) {
            repos.push(entry);
        }
    }
    repos
}

fn act(root: &Path, commands: &[Vec<String>], ok: &str, fail: &str) -> Outcome {
    for args in commands {
        let mut all = vec!["--literal-pathspecs"];
        all.extend(args.iter().map(String::as_str));
        let out = git(root, &all);
        if !out.ok() {
            let text = out.said();
            let short: Vec<&str> = text.trim().split('\n').collect();
            let line = short[short.len().saturating_sub(2)..].join(" ");
            return failure(&text).with("title", fail.into()).detail(&line);
        }
    }
    success(ok)
}

fn with_paths(verb: &[&str], paths: &[String]) -> Vec<String> {
    verb.iter().map(|v| v.to_string()).chain(std::iter::once("--".to_string())).chain(paths.iter().cloned()).collect()
}

pub fn stage_files(root: &Path, paths: &[String], label: &str) -> Outcome {
    act(root, &[with_paths(&["add", "-A"], paths)], &format!("Staged {label}"), "Stage failed")
}

pub fn unstage_files(root: &Path, paths: &[String], label: &str) -> Outcome {
    act(root, &[with_paths(&["reset", "-q"], paths)], &format!("Unstaged {label}"), "Unstage failed")
}

pub fn discard_commands(files: &[FileChange], paths: &[String]) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    for path in paths {
        let Some(file) = files.iter().find(|f| &f.path == path) else { continue };
        let one = std::slice::from_ref(path);
        if file.staged || file.conflict {
            commands.push(with_paths(&["reset", "-q"], one));
            commands.push(if file.x == 'A' || file.fresh { with_paths(&["clean", "-fd"], one) } else { with_paths(&["checkout"], one) });
        } else if file.fresh {
            commands.push(with_paths(&["clean", "-fd"], one));
        } else {
            commands.push(with_paths(&["checkout"], one));
        }
    }
    commands
}

pub fn discard_files(root: &Path, files: &[FileChange], paths: &[String], label: &str) -> Outcome {
    let commands = discard_commands(files, paths);
    if commands.is_empty() {
        return success("Nothing to discard");
    }
    act(root, &commands, &format!("Discarded {label}"), "Discard failed")
}

pub fn discard_all_commands(files: &[FileChange], size: usize) -> Vec<Vec<String>> {
    let mut resets = Vec::new();
    let mut cleans = Vec::new();
    let mut checkouts = Vec::new();
    for f in files {
        let indexed = f.staged || f.conflict;
        if indexed {
            resets.push(f.path.clone());
        }
        if f.fresh || (indexed && f.x == 'A') {
            cleans.push(f.path.clone());
        } else {
            checkouts.push(f.path.clone());
        }
    }
    let size = size.max(1);
    let chunks = |verb: &[&str], paths: &[String]| paths.chunks(size).map(|c| with_paths(verb, c)).collect::<Vec<_>>();
    let mut out = chunks(&["reset", "-q"], &resets);
    out.extend(chunks(&["clean", "-fd"], &cleans));
    out.extend(chunks(&["checkout"], &checkouts));
    out
}

pub fn leftover_note(paths: &[String], error: &str) -> Option<Outcome> {
    if paths.is_empty() {
        return None;
    }
    let names: Vec<String> = paths.iter().take(4).map(|p| p.rsplit('/').next().unwrap_or(p).to_string()).collect();
    let more = if paths.len() > 4 { ", …" } else { "" };
    let body = [
        format!("{} still changed: {}{more}.", plural(paths.len(), "file is", "files are"), names.join(", ")),
        "A file may be locked or open in another app, or be a separate git repo inside this folder.".to_string(),
        error.to_string(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    Some(
        Outcome { ok: false, kind: "leftover".into(), title: "Some changes are still there".into(), hint: body, ..Outcome::default() }
            .with("level", "warning".into())
            .with("files", json!(paths)),
    )
}

pub fn discard_all(root: &Path, files: &[FileChange]) -> Outcome {
    let commands = discard_all_commands(files, 100);
    if commands.is_empty() {
        return success("Nothing to discard");
    }
    let mut error = String::new();
    for args in &commands {
        let mut all = vec!["--literal-pathspecs"];
        all.extend(args.iter().map(String::as_str));
        let out = git(root, &all);
        if !out.ok() && error.is_empty() {
            let text = if out.stderr.trim().is_empty() { out.stdout.clone() } else { out.stderr.clone() };
            let lines: Vec<&str> = text.trim().split('\n').collect();
            error = lines[lines.len().saturating_sub(2)..].join(" ");
        }
    }
    let after = read_git(root, &["status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all"]);
    let left: Vec<String> = if after.ok() {
        split_z(&after.stdout).into_iter().filter(|e| !e.starts_with("## ")).map(|e| e.get(3..).unwrap_or("").to_string()).collect()
    } else {
        Vec::new()
    };
    if let Some(note) = leftover_note(&left, &error) {
        return note;
    }
    if !error.is_empty() {
        return failure(&error).with("title", "Discard failed".into());
    }
    success("Discarded all changes").hint(&plural(files.len(), "file", "files"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrepo;

    #[test]
    fn header_shapes() {
        let h = parse_header(Some("main...origin/main [ahead 2, behind 1]"));
        assert_eq!(h.branch.as_deref(), Some("main"));
        assert_eq!(h.upstream.as_deref(), Some("origin/main"));
        assert_eq!((h.ahead, h.behind), (Some(2), Some(1)));
        assert!(parse_header(Some("No commits yet on dev")).unborn);
        assert!(parse_header(Some("HEAD (no branch)")).detached);
        let plain = parse_header(Some("topic"));
        assert_eq!(plain.branch.as_deref(), Some("topic"));
        assert_eq!(plain.ahead, None);
    }

    #[test]
    fn status_lists_files_and_stats() {
        let repo = testrepo::new();
        let root = repo.root();
        repo.write("a.txt", "one\nTWO\nthree\nfour\n");
        repo.write("dir/new.txt", "n\n");
        repo.write("b.txt", "b\n");
        repo.git(&["add", "b.txt"]);
        let s = status(root).unwrap();
        assert_eq!(s.header.branch.as_deref(), Some("main"));
        assert!(s.has_head);
        assert!(!s.has_remote);
        let a = s.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!((a.status, a.add, a.del, a.staged), ('M', 2, 1, false));
        let n = s.files.iter().find(|f| f.path == "dir/new.txt").unwrap();
        assert!(n.fresh);
        assert_eq!(n.folder, "dir");
        let b = s.files.iter().find(|f| f.path == "b.txt").unwrap();
        assert!(b.staged);
        assert_eq!(b.status, 'A');
        assert_eq!(file_status(root, "a.txt").unwrap().unwrap().add, 2);
        assert!(file_status(root, "zzz").unwrap().is_none());
        assert!(stage_files(root, &["a.txt".into()], "a.txt").ok);
        assert!(status(root).unwrap().files.iter().find(|f| f.path == "a.txt").unwrap().staged);
        assert!(unstage_files(root, &["a.txt".into()], "a.txt").ok);
        let files = status(root).unwrap().files;
        assert!(discard_files(root, &files, &["a.txt".into(), "b.txt".into()], "2 files").ok);
        assert_eq!(repo.read("a.txt"), "one\ntwo\nthree\n");
        assert!(!repo.path("b.txt").exists());
        let files = status(root).unwrap().files;
        let all = discard_all(root, &files);
        assert!(all.ok, "{all:?}");
        assert!(status(root).unwrap().files.is_empty());
    }

    #[test]
    fn unborn_repo_and_discovery() {
        let repo = testrepo::empty();
        repo.write("x.txt", "x\n");
        repo.git(&["add", "x.txt"]);
        let s = status(repo.root()).unwrap();
        assert!(s.header.unborn);
        assert!(!s.has_head);
        assert_eq!(s.files[0].add, 1);
        let parent = tempfile::tempdir().unwrap();
        let child = parent.path().join("child");
        std::fs::create_dir(&child).unwrap();
        testrepo::sh_git(&child, &["init", "-q"]);
        std::fs::create_dir(parent.path().join("node_modules")).unwrap();
        let found = discover_repos(parent.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "child");
        assert!(leftover_note(&[], "").is_none());
        assert_eq!(discard_all_commands(&s.files, 100).len(), 2);
    }
}
