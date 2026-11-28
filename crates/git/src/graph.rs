use crate::ops::{State, clean_path};
use crate::outcome::plural;
use crate::run::read_ok;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

pub const LOG_FORMAT: &str = "%H%x1f%h%x1f%P%x1f%s%x1f%an%x1f%at%x1f%D";

pub const HISTORY_PAGE: usize = 200;

pub const HISTORY_CAP: usize = 5000;

pub const SEARCH_CAP: usize = 400;

pub const GRAPH_SCOPE: [&str; 3] = ["--branches", "--tags", "HEAD"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefName {
    pub name: String,
    pub kind: String,
    pub head: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub sha: String,
    pub short: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author: String,
    pub at: i64,
    pub refs: Vec<RefName>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCommit {
    pub sha: String,
    pub short: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author: String,
    pub at: i64,
    pub path: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub from_lane: usize,
    pub from_y: f32,
    pub to_lane: usize,
    pub to_y: f32,
    pub color: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LaneRow {
    pub lane: usize,
    pub lanes: usize,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogQuery {
    pub limit: usize,
    pub skip: usize,
    pub grep: Option<String>,
    pub author: Option<String>,
    pub path: Option<String>,
    pub all: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPlan {
    pub query: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateText {
    pub what: String,
    pub conflicts: usize,
    pub detail: String,
}

const REF_KINDS: [(&str, &str); 3] = [("refs/heads/", "local"), ("refs/remotes/", "remote"), ("refs/tags/", "tag")];

pub fn parse_refs(text: &str) -> Vec<RefName> {
    let mut refs: Vec<RefName> = Vec::new();
    for raw in text.split(", ") {
        let mut part = raw.trim();
        if part.is_empty() {
            continue;
        }
        if part == "HEAD" {
            refs.push(RefName { name: "HEAD".into(), kind: "head".into(), head: true });
            continue;
        }
        let head = part.starts_with("HEAD -> ");
        if head {
            part = &part["HEAD -> ".len()..];
        }
        part = part.strip_prefix("tag: ").unwrap_or(part);
        let Some((prefix, kind)) = REF_KINDS.iter().find(|(p, _)| part.starts_with(p)) else { continue };
        let name = &part[prefix.len()..];
        if name.is_empty() || (*kind == "remote" && name.ends_with("/HEAD")) {
            continue;
        }
        refs.push(RefName { name: name.into(), kind: kind.to_string(), head });
    }
    let rank = |r: &RefName| {
        if r.head {
            0
        } else {
            REF_KINDS.iter().position(|(_, k)| *k == r.kind).map(|i| i + 1).unwrap_or(0)
        }
    };
    refs.sort_by_key(rank);
    refs
}

pub fn parse_log(text: &str) -> Vec<Commit> {
    text.split('\n')
        .filter(|l| !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\x1f').collect();
            let get = |i: usize| f.get(i).copied().unwrap_or("");
            Commit {
                sha: get(0).into(),
                short: get(1).into(),
                parents: get(2).split(' ').filter(|p| !p.is_empty()).map(String::from).collect(),
                subject: get(3).into(),
                author: get(4).into(),
                at: get(5).parse().unwrap_or(0),
                refs: parse_refs(get(6)),
            }
        })
        .collect()
}

pub fn log_args(q: &LogQuery) -> Vec<String> {
    let limit = if q.limit == 0 { HISTORY_PAGE } else { q.limit };
    let mut args = vec!["log".to_string(), "-n".into(), limit.to_string()];
    if q.skip > 0 {
        args.push(format!("--skip={}", q.skip));
    }
    if let Some(g) = &q.grep {
        args.extend(["-i".into(), "-F".into(), format!("--grep={g}")]);
    }
    if let Some(a) = &q.author {
        args.extend(["-i".into(), "-F".into(), format!("--author={a}")]);
    }
    let path = q.path.as_deref().filter(|p| !p.is_empty());
    if path.is_none() {
        args.push("--date-order".into());
        if q.all {
            args.extend(GRAPH_SCOPE.iter().map(|s| s.to_string()));
        } else if q.upstream.is_some() {
            args.push("HEAD".into());
        }
        if let Some(u) = &q.upstream {
            args.push(format!("--remotes={u}"));
        }
    }
    args.push("--decorate=full".into());
    args.push(format!("--format={LOG_FORMAT}"));
    if let Some(p) = path {
        args.extend(["--follow".into(), "--".into(), p.to_string()]);
    }
    args
}

pub fn load_log(root: &Path, q: &LogQuery) -> Result<Vec<Commit>, String> {
    let mut args: Vec<String> = vec!["-c".into(), "core.quotepath=off".into(), "--literal-pathspecs".into()];
    args.extend(log_args(q));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = crate::run::read(root, &refs);
    if out.ok() {
        return Ok(parse_log(&out.stdout));
    }
    let text = out.said();
    if text.contains("does not have any commits") || text.contains("bad default revision") {
        return Ok(Vec::new());
    }
    Err(crate::run::failure_text(&out, &refs))
}

pub fn search_plan(query: &str) -> Option<SearchPlan> {
    let text = query.trim();
    if text.is_empty() {
        return None;
    }
    let is_id = (4..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit());
    Some(SearchPlan { query: text.into(), id: if is_id { Some(text.to_lowercase()) } else { None } })
}

pub fn merge_search(by_id: &[Commit], lists: &[Vec<Commit>], cap: usize) -> Vec<Commit> {
    let mut seen = HashSet::new();
    let mut first = Vec::new();
    let mut rest = Vec::new();
    for c in by_id {
        if !c.sha.is_empty() && seen.insert(c.sha.clone()) {
            first.push(c.clone());
        }
    }
    for list in lists {
        for c in list {
            if !c.sha.is_empty() && seen.insert(c.sha.clone()) {
                rest.push(c.clone());
            }
        }
    }
    rest.sort_by_key(|c| std::cmp::Reverse(c.at));
    first.into_iter().chain(rest).take(cap).collect()
}

pub fn search(root: &Path, query: &str, all: bool) -> Result<Vec<Commit>, String> {
    let Some(plan) = search_plan(query) else { return Ok(Vec::new()) };
    let mut by_id = Vec::new();
    if let Some(id) = &plan.id {
        let spec = format!("{id}^{{commit}}");
        let out = crate::run::read(root, &["rev-parse", "-q", "--verify", &spec]);
        if out.ok() {
            let sha = out.stdout.trim().to_string();
            let format = format!("--format={LOG_FORMAT}");
            let text = read_ok(root, &["log", "-1", "--decorate=full", &format, &sha])?;
            by_id = parse_log(&text);
        }
    }
    let grep = load_log(root, &LogQuery { limit: SEARCH_CAP, grep: Some(plan.query.clone()), all, ..LogQuery::default() })?;
    let author = load_log(root, &LogQuery { limit: SEARCH_CAP, author: Some(plan.query.clone()), all, ..LogQuery::default() })?;
    Ok(merge_search(&by_id, &[grep, author], SEARCH_CAP))
}

pub fn linear_rows(count: usize) -> Vec<LaneRow> {
    (0..count)
        .map(|i| {
            let mut segments = Vec::new();
            if i > 0 {
                segments.push(Segment { from_lane: 0, from_y: 0.0, to_lane: 0, to_y: 0.5, color: 0 });
            }
            if i + 1 < count {
                segments.push(Segment { from_lane: 0, from_y: 0.5, to_lane: 0, to_y: 1.0, color: 0 });
            }
            LaneRow { lane: 0, lanes: 1, segments }
        })
        .collect()
}

fn free(lanes: &[Option<String>]) -> usize {
    lanes.iter().position(Option::is_none).unwrap_or(lanes.len())
}

fn put(lanes: &mut Vec<Option<String>>, at: usize, value: Option<String>) {
    if at >= lanes.len() {
        lanes.resize(at + 1, None);
    }
    lanes[at] = value;
}

pub fn lane_rows(commits: &[Commit]) -> Vec<LaneRow> {
    let mut lanes: Vec<Option<String>> = Vec::new();
    commits
        .iter()
        .map(|commit| {
            let before = lanes.clone();
            let lane = match before.iter().position(|s| s.as_deref() == Some(commit.sha.as_str())) {
                Some(l) => l,
                None => {
                    let l = free(&lanes);
                    put(&mut lanes, l, Some(commit.sha.clone()));
                    l
                }
            };
            let mut segments = Vec::new();
            for (j, sha) in before.iter().enumerate() {
                let Some(sha) = sha else { continue };
                if sha != &commit.sha {
                    segments.push(Segment { from_lane: j, from_y: 0.0, to_lane: j, to_y: 1.0, color: j });
                } else {
                    segments.push(Segment { from_lane: j, from_y: 0.0, to_lane: lane, to_y: 0.5, color: j });
                    if j != lane {
                        lanes[j] = None;
                    }
                }
            }
            let first = commit.parents.first();
            let waiting = first.and_then(|f| lanes.iter().enumerate().position(|(j, s)| s.as_ref() == Some(f) && j != lane));
            put(&mut lanes, lane, if waiting.is_none() { first.cloned() } else { None });
            if first.is_some() {
                let to = waiting.unwrap_or(lane);
                segments.push(Segment { from_lane: lane, from_y: 0.5, to_lane: to, to_y: 1.0, color: to });
            }
            for parent in commit.parents.iter().skip(1) {
                let k = match lanes.iter().position(|s| s.as_ref() == Some(parent)) {
                    Some(k) => k,
                    None => {
                        let k = free(&lanes);
                        put(&mut lanes, k, Some(parent.clone()));
                        k
                    }
                };
                segments.push(Segment { from_lane: lane, from_y: 0.5, to_lane: k, to_y: 1.0, color: k });
            }
            let width = before.len().max(lanes.len()).max(lane + 1);
            while lanes.last().is_some_and(Option::is_none) {
                lanes.pop();
            }
            LaneRow { lane, segments, lanes: width }
        })
        .collect()
}

pub fn file_history(root: &Path, path: &str, limit: usize) -> Result<Vec<FileCommit>, String> {
    let p = clean_path(path)?;
    let limit = limit.clamp(1, 2000).to_string();
    let text = read_ok(
        root,
        &[
            "-c",
            "core.quotepath=off",
            "--literal-pathspecs",
            "log",
            "--follow",
            "-n",
            &limit,
            "--format=%x1e%H%x1f%h%x1f%P%x1f%s%x1f%an%x1f%at",
            "--name-only",
            "--",
            &p,
        ],
    )?;
    Ok(text
        .split('\x1e')
        .filter(|chunk| !chunk.trim().is_empty())
        .map(|chunk| {
            let mut rows = chunk.split('\n');
            let head = rows.next().unwrap_or("");
            let named: Vec<&str> = rows.map(str::trim).filter(|l| !l.is_empty()).collect();
            let f: Vec<&str> = head.split('\x1f').collect();
            let get = |i: usize| f.get(i).copied().unwrap_or("");
            FileCommit {
                sha: get(0).into(),
                short: get(1).into(),
                parents: get(2).split(' ').filter(|x| !x.is_empty()).map(String::from).collect(),
                subject: get(3).into(),
                author: get(4).into(),
                at: get(5).parse().unwrap_or(0),
                path: named.last().map(|s| s.to_string()).unwrap_or_else(|| p.clone()),
            }
        })
        .collect())
}

pub fn state_text(state: &State) -> StateText {
    let n = state.conflicts.len();
    let what = match (state.op.as_deref(), state.step) {
        (Some("rebase"), Some(step)) => format!("Rebasing: step {} of {}", step.n, step.total),
        (Some(op), _) => format!("{} in progress", op_text(op)),
        (None, _) => "Working in progress".into(),
    };
    let detail = if n > 0 { format!("{} with conflicts", plural(n, "file", "files")) } else { "No conflicts left".into() };
    StateText { what, conflicts: n, detail }
}

fn op_text(op: &str) -> &'static str {
    match op {
        "merge" => "Merging",
        "rebase" => "Rebasing",
        "cherry-pick" => "Cherry-picking",
        "revert" => "Reverting",
        _ => "Working",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrepo;

    fn c(sha: &str, parents: &[&str]) -> Commit {
        Commit { sha: sha.into(), parents: parents.iter().map(|p| p.to_string()).collect(), ..Commit::default() }
    }

    #[test]
    fn refs_sorted_head_first() {
        let refs = parse_refs("tag: refs/tags/v1, HEAD -> refs/heads/main, refs/remotes/origin/main, refs/remotes/origin/HEAD");
        let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["main", "origin/main", "v1"]);
        assert!(refs[0].head);
        assert_eq!(refs[2].kind, "tag");
    }

    #[test]
    fn lanes_for_a_merge() {
        let rows = lane_rows(&[c("m", &["a", "b"]), c("b", &["r"]), c("a", &["r"]), c("r", &[])]);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[3].lane, 1);
        assert_eq!(rows[0].lanes, 2);
        assert_eq!(linear_rows(3)[1].segments.len(), 2);
    }

    #[test]
    fn log_args_and_search_plan() {
        let args = log_args(&LogQuery { grep: Some("fix".into()), all: true, ..LogQuery::default() });
        assert!(args.contains(&"--grep=fix".to_string()));
        assert!(args.contains(&"--branches".to_string()));
        let path = log_args(&LogQuery { path: Some("a.txt".into()), ..LogQuery::default() });
        assert!(path.ends_with(&["--follow".to_string(), "--".to_string(), "a.txt".to_string()]));
        assert!(!path.contains(&"--date-order".to_string()));
        assert_eq!(search_plan(" ABCD ").unwrap().id.as_deref(), Some("abcd"));
        assert_eq!(search_plan("fix bug").unwrap().id, None);
        assert!(search_plan("  ").is_none());
    }

    #[test]
    fn real_log_search_and_file_history() {
        let repo = testrepo::new();
        let root = repo.root();
        let second = repo.commit_file("a.txt", "x\n", "fix the thing");
        repo.git(&["mv", "a.txt", "b.txt"]);
        repo.git(&["commit", "-q", "-m", "rename"]);
        let log = load_log(root, &LogQuery { all: true, ..LogQuery::default() }).unwrap();
        assert_eq!(log.len(), 3);
        assert!(log[0].refs.iter().any(|r| r.name == "main" && r.head));
        let found = search(root, "fix", true).unwrap();
        assert_eq!(found.len(), 1);
        let by_id = search(root, &second[..8], true).unwrap();
        assert_eq!(by_id[0].sha, second);
        let hist = file_history(root, "b.txt", 50).unwrap();
        assert_eq!(hist.len(), 3);
        assert_eq!(hist[2].path, "a.txt");
        let empty = testrepo::empty();
        assert!(load_log(empty.root(), &LogQuery::default()).unwrap().is_empty());
        let st = state_text(&State { op: Some("merge".into()), conflicts: vec!["a".into()], ..State::default() });
        assert_eq!(st.what, "Merging in progress");
        assert_eq!(st.detail, "1 file with conflicts");
    }
}
