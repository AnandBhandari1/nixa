use super::run::read;
use std::path::Path;

pub const LOG_FORMAT: &str = "%H%x1f%h%x1f%P%x1f%s%x1f%an%x1f%at%x1f%D";
pub const PAGE: usize = 200;
pub const CAP: usize = 5000;
pub const FILE_CAP: usize = 2000;
pub const SEARCH_CAP: usize = 400;
pub const LANE_MAX: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Head,
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    pub name: String,
    pub kind: RefKind,
    pub head: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub lane: usize,
    pub lanes: usize,
    pub segments: Vec<Segment>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub from: usize,
    pub from_y: f32,
    pub to: usize,
    pub to_y: f32,
    pub color: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Commit {
    pub sha: String,
    pub short: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author: String,
    pub at: i64,
    pub refs: Vec<Ref>,
    pub row: Row,
}

fn rank(r: &Ref) -> usize {
    if r.head {
        return 0;
    }
    match r.kind {
        RefKind::Local => 1,
        RefKind::Remote => 2,
        RefKind::Tag => 3,
        RefKind::Head => 4,
    }
}

pub fn parse_refs(text: &str) -> Vec<Ref> {
    let mut refs = Vec::new();
    for raw in text.split(", ") {
        let mut part = raw.trim();
        if part.is_empty() {
            continue;
        }
        if part == "HEAD" {
            refs.push(Ref {
                name: "HEAD".into(),
                kind: RefKind::Head,
                head: true,
            });
            continue;
        }
        let head = part.starts_with("HEAD -> ");
        if head {
            part = &part["HEAD -> ".len()..];
        }
        if let Some(rest) = part.strip_prefix("tag: ") {
            part = rest;
        }
        let found = [
            ("refs/heads/", RefKind::Local),
            ("refs/remotes/", RefKind::Remote),
            ("refs/tags/", RefKind::Tag),
        ]
        .into_iter()
        .find(|(prefix, _)| part.starts_with(prefix));
        let Some((prefix, kind)) = found else {
            continue;
        };
        let name = &part[prefix.len()..];
        if name.is_empty() || (kind == RefKind::Remote && name.ends_with("/HEAD")) {
            continue;
        }
        refs.push(Ref {
            name: name.to_string(),
            kind,
            head,
        });
    }
    refs.sort_by_key(rank);
    refs
}

pub fn parse_log(text: &str) -> Vec<Commit> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\u{1f}').collect();
            let get = |i: usize| f.get(i).copied().unwrap_or_default();
            Commit {
                sha: get(0).to_string(),
                short: get(1).to_string(),
                parents: get(2).split(' ').filter(|p| !p.is_empty()).map(str::to_string).collect(),
                subject: get(3).to_string(),
                author: get(4).to_string(),
                at: get(5).parse().unwrap_or(0),
                refs: parse_refs(get(6)),
                row: Row::default(),
            }
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogQuery {
    pub limit: usize,
    pub skip: usize,
    pub grep: Option<String>,
    pub author: Option<String>,
    pub path: Option<String>,
    pub all: bool,
    pub upstream: Option<String>,
}

pub fn log_args(q: &LogQuery) -> Vec<String> {
    let mut a: Vec<String> = vec!["log".into(), "-n".into(), q.limit.max(1).to_string()];
    if q.skip > 0 {
        a.push(format!("--skip={}", q.skip));
    }
    if let Some(g) = &q.grep {
        a.extend(["-i".into(), "-F".into(), format!("--grep={g}")]);
    }
    if let Some(w) = &q.author {
        a.extend(["-i".into(), "-F".into(), format!("--author={w}")]);
    }
    if q.path.is_none() {
        a.push("--date-order".into());
        if q.all {
            a.extend(["--branches".into(), "--tags".into(), "HEAD".into()]);
        } else if q.upstream.is_some() {
            a.push("HEAD".into());
        }
        if let Some(up) = &q.upstream {
            a.push(format!("--remotes={up}"));
        }
    }
    a.push("--decorate=full".into());
    a.push(format!("--format={LOG_FORMAT}"));
    if let Some(p) = &q.path {
        a.extend(["--follow".into(), "--".into(), p.clone()]);
    }
    a
}

pub fn read_log(root: &Path, args: &[String]) -> Result<Vec<Commit>, String> {
    let mut all: Vec<&str> = vec!["--literal-pathspecs"];
    all.extend(args.iter().map(String::as_str));
    let out = read(root, &all);
    if out.ok() {
        Ok(parse_log(&out.stdout))
    } else {
        Err(out.said())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchPlan {
    pub query: String,
    pub id: Option<String>,
}

pub fn search_plan(query: &str) -> Option<SearchPlan> {
    let text = query.trim();
    if text.is_empty() {
        return None;
    }
    let id = super::words::is_sha_like(text).then(|| text.to_lowercase());
    Some(SearchPlan {
        query: text.to_string(),
        id,
    })
}

pub fn merge_search(by_id: Vec<Commit>, lists: Vec<Vec<Commit>>, cap: usize) -> Vec<Commit> {
    let mut seen = std::collections::HashSet::new();
    let mut first = Vec::new();
    let mut rest = Vec::new();
    for c in by_id {
        if !c.sha.is_empty() && seen.insert(c.sha.clone()) {
            first.push(c);
        }
    }
    for list in lists {
        for c in list {
            if !c.sha.is_empty() && seen.insert(c.sha.clone()) {
                rest.push(c);
            }
        }
    }
    rest.sort_by_key(|c| std::cmp::Reverse(c.at));
    first.extend(rest);
    first.truncate(cap);
    first
}

pub fn linear_rows(n: usize) -> Vec<Row> {
    (0..n)
        .map(|i| {
            let mut segments = Vec::new();
            if i > 0 {
                segments.push(Segment { from: 0, from_y: 0.0, to: 0, to_y: 0.5, color: 0 });
            }
            if i + 1 < n {
                segments.push(Segment { from: 0, from_y: 0.5, to: 0, to_y: 1.0, color: 0 });
            }
            Row { lane: 0, lanes: 1, segments }
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

pub fn lane_rows(commits: &[Commit]) -> Vec<Row> {
    let mut lanes: Vec<Option<String>> = Vec::new();
    commits
        .iter()
        .map(|commit| {
            let before = lanes.clone();
            let lane = match before.iter().position(|s| s.as_deref() == Some(commit.sha.as_str())) {
                Some(i) => i,
                None => {
                    let i = free(&lanes);
                    put(&mut lanes, i, Some(commit.sha.clone()));
                    i
                }
            };
            let mut segments = Vec::new();
            for (j, sha) in before.iter().enumerate() {
                let Some(sha) = sha else { continue };
                if *sha != commit.sha {
                    segments.push(Segment { from: j, from_y: 0.0, to: j, to_y: 1.0, color: j });
                } else {
                    segments.push(Segment { from: j, from_y: 0.0, to: lane, to_y: 0.5, color: j });
                    if j != lane {
                        lanes[j] = None;
                    }
                }
            }
            let first = commit.parents.first().cloned();
            let waiting = first.as_ref().and_then(|f| {
                lanes
                    .iter()
                    .enumerate()
                    .position(|(j, s)| s.as_deref() == Some(f.as_str()) && j != lane)
            });
            put(&mut lanes, lane, if waiting.is_none() { first.clone() } else { None });
            if first.is_some() {
                let to = waiting.unwrap_or(lane);
                segments.push(Segment { from: lane, from_y: 0.5, to, to_y: 1.0, color: to });
            }
            for parent in commit.parents.iter().skip(1) {
                let k = match lanes.iter().position(|s| s.as_deref() == Some(parent.as_str())) {
                    Some(k) => k,
                    None => {
                        let k = free(&lanes);
                        put(&mut lanes, k, Some(parent.clone()));
                        k
                    }
                };
                segments.push(Segment { from: lane, from_y: 0.5, to: k, to_y: 1.0, color: k });
            }
            let width = before.len().max(lanes.len()).max(lane + 1);
            while lanes.last().is_some_and(Option::is_none) {
                lanes.pop();
            }
            Row { lane, segments, lanes: width }
        })
        .collect()
}

pub fn set_rows(commits: &mut [Commit], linear: bool) -> usize {
    let rows = if linear { linear_rows(commits.len()) } else { lane_rows(commits) };
    let mut most = 1;
    for (c, r) in commits.iter_mut().zip(rows) {
        most = most.max(r.lanes);
        c.row = r;
    }
    if commits.is_empty() { 1 } else { most.min(LANE_MAX) }
}

pub fn file_history(root: &Path, path: &str, limit: usize) -> Result<Vec<Commit>, String> {
    let q = LogQuery {
        limit,
        path: Some(path.to_string()),
        ..LogQuery::default()
    };
    read_log(root, &log_args(&q))
}

pub fn find_by_id(root: &Path, id: &str) -> Vec<Commit> {
    let spec = format!("{id}^{{commit}}");
    let out = read(root, &["rev-parse", "--verify", "--quiet", &spec]);
    let sha = out.line();
    if !out.ok() || !super::words::is_full_sha(&sha) {
        return Vec::new();
    }
    let fmt = format!("--format={LOG_FORMAT}");
    read_log(
        root,
        &["log".into(), "-1".into(), "--decorate=full".into(), fmt, sha],
    )
    .unwrap_or_default()
}

pub fn visible<'a>(commits: &'a [Commit], filter: &str, searching: bool) -> Vec<&'a Commit> {
    let q = filter.trim().to_lowercase();
    if searching || q.is_empty() {
        return commits.iter().collect();
    }
    commits
        .iter()
        .filter(|c| {
            c.subject.to_lowercase().contains(&q) || c.author.to_lowercase().contains(&q) || c.sha.starts_with(&q)
        })
        .collect()
}
