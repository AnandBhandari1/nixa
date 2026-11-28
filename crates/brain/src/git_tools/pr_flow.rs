use super::github::{OpenPr, PrFacts, facts_for, open_prs, parse_pr_url, parse_remote};
use super::policy::plural;
use super::run::{LONG_MS, Out, git};
use crate::app::{App, lock};
use crate::spaces::Space;
use crate::teams::model::{By, Run, RunPr, RunState};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

pub const WORK_FILE: &str = "pr-work.json";
pub const RECENT_MS: u64 = 15 * 60 * 1000;
pub const FLOWS_KEEP: usize = 30;
pub const CHANGED: &str = "pr_flows";
pub const USER_STOP: &str = "stopped by the user";

static WANTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[\s#])#?(\d{1,6})\b").expect("pr number regex"));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Picked,
    Nothing,
    #[default]
    Failed,
}

impl Outcome {
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Picked => "picked",
            Outcome::Nothing => "nothing",
            Outcome::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentState {
    #[default]
    None,
    Waiting,
    Posted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowState {
    Running,
    Done,
    #[default]
    Stopped,
    Failed,
}

impl FlowState {
    pub fn word(self) -> &'static str {
        match self {
            FlowState::Running => "running",
            FlowState::Done => "done",
            FlowState::Stopped => "stopped",
            FlowState::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrRecord {
    pub key: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub head: String,
    pub outcome: Outcome,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub commit: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
    pub comment_state: CommentState,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub run: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub flow: String,
    pub at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Current {
    pub number: u64,
    pub run: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DoneItem {
    pub number: u64,
    pub outcome: Outcome,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub commit: String,
    pub comment: CommentState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrFlow {
    pub id: String,
    pub team: String,
    pub team_name: String,
    pub repo: String,
    pub slug: String,
    pub branch: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub space: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub space_name: String,
    pub by: By,
    pub state: FlowState,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub queue: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<Current>,
    pub done: Vec<DoneItem>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkFile {
    pub version: u32,
    pub prs: BTreeMap<String, PrRecord>,
    pub flows: Vec<PrFlow>,
}

impl Default for WorkFile {
    fn default() -> WorkFile {
        WorkFile { version: 1, prs: BTreeMap::new(), flows: Vec::new() }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Extra {
    pub candidates: BTreeMap<u64, OpenPr>,
    pub skills: Vec<String>,
    pub mcps: Vec<String>,
    pub task: String,
}

pub struct Flows {
    pub path: PathBuf,
    pub file: Mutex<WorkFile>,
    pub extra: Mutex<HashMap<String, Extra>>,
    pub noted: Mutex<HashSet<String>>,
    pub made: Mutex<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct FlowStart {
    pub team: String,
    pub project: String,
    pub branch: String,
    pub space: Option<Space>,
    pub skills: Vec<String>,
    pub mcps: Vec<String>,
    pub task: String,
    pub by: By,
}

pub fn parse_work(source: Option<&str>) -> WorkFile {
    let Some(text) = source.filter(|t| !t.trim().is_empty()) else { return WorkFile::default() };
    let Ok(mut file) = serde_json::from_str::<WorkFile>(text) else { return WorkFile::default() };
    file.version = 1;
    let skip = file.flows.len().saturating_sub(FLOWS_KEEP);
    file.flows.drain(..skip);
    file
}

pub fn cut_running(file: &mut WorkFile) -> bool {
    let mut cut = false;
    for flow in file.flows.iter_mut().filter(|f| f.state == FlowState::Running) {
        flow.state = FlowState::Stopped;
        flow.note = "Nixa stopped during this PR flow. Start it again; reviewed PRs are skipped.".into();
        flow.current = None;
        cut = true;
    }
    cut
}

fn write_file(path: &Path, file: &WorkFile) {
    let mut kept = file.clone();
    let skip = kept.flows.len().saturating_sub(FLOWS_KEEP);
    kept.flows.drain(..skip);
    if let Err(e) = crate::util::write_json(path, &kept) {
        crate::util::log("pr flow", &format!("could not save {}: {e}", path.display()));
    }
}

impl Flows {
    pub fn open(dir: &Path) -> Flows {
        let path = dir.join(WORK_FILE);
        let mut file = parse_work(std::fs::read_to_string(&path).ok().as_deref());
        if cut_running(&mut file) {
            write_file(&path, &file);
        }
        Flows { path, file: Mutex::new(file), extra: Mutex::new(HashMap::new()), noted: Mutex::new(HashSet::new()), made: Mutex::new(0) }
    }
}

pub fn pr_key(slug: &str, number: u64) -> String {
    format!("{slug}#{number}")
}

pub fn wanted_number(task: &str) -> Option<u64> {
    WANTED.captures(task).and_then(|c| c[1].parse().ok())
}

fn short_name(root: &str) -> String {
    root.trim_end_matches('/').rsplit('/').next().unwrap_or(root).to_string()
}

fn clip(text: &str, size: usize) -> String {
    crate::util::clip_flat(text, size)
}

pub struct BriefInput<'a> {
    pub flow: &'a PrFlow,
    pub pr: &'a RunPr,
    pub purpose: &'a str,
    pub task: &'a str,
    pub facts: Option<&'a PrFacts>,
    pub diff: &'a Path,
    pub stat: &'a str,
    pub comment: &'a Path,
    pub skills: &'a [String],
    pub mcps: &'a [String],
    pub skill_dir: &'a Path,
}

const COMMITTER: &str = "Nixa commits the picked parts with nixa_git_commit after the run";
const POSTER: &str = "No team step posts it. Nixa posts it to the PR after the run with nixa_pr_flow post.";

pub fn brief_text(input: &BriefInput) -> String {
    let flow = input.flow;
    let pr = input.pr;
    let task = if input.task.trim().is_empty() { "Review this PR, take only its good parts into our code, test them, and write the comment draft." } else { input.task.trim() };
    let mut lines = vec![format!("# PR #{}: {}", pr.number, pr.title), String::new(), format!("Task: {task}")];
    if !input.purpose.is_empty() {
        lines.push(format!("Team purpose: {}", input.purpose));
    }
    lines.push(String::new());
    lines.push(format!("Repo: {} on branch {}. Work there as uncommitted changes. Never commit, never push, never run a GitHub action: {}. Nixa resets the tree when the run fails.", flow.repo, flow.branch, COMMITTER));
    let by = if pr.author.is_empty() { String::new() } else { format!(" by {}", pr.author) };
    lines.push(format!("PR: #{} {}{by}. Link: {}.", pr.number, pr.title, pr.url));
    if let Some(facts) = input.facts {
        let from = if facts.head.repo.is_empty() { String::new() } else { format!(" in {}", facts.head.repo) };
        let draft = if facts.draft { " It is a draft." } else { "" };
        let body = if facts.body.is_empty() { String::new() } else { format!(" Description: {}", clip(&facts.body, 600)) };
        lines.push(format!("PR branch: {}{from} onto {}.{draft}{body}", facts.head.branch, facts.base));
    }
    let head = if pr.head.is_empty() { "unknown" } else { pr.head.as_str() };
    lines.push(format!("PR head: {head}, fetched as refs/nixa/pr/{}. The diff against our branch is in {}. Changed files:", pr.number, input.diff.display()));
    lines.push(if input.stat.trim().is_empty() { "(no stat)".into() } else { input.stat.trim_end().to_string() });
    lines.push(String::new());
    lines.extend(crate::teams::prompt::skill_line(input.skills, input.skill_dir));
    lines.extend(crate::teams::prompt::mcp_line(input.mcps));
    lines.push(format!("Comment draft: the PR Commenter writes it to {} with three parts: What is good, What we have taken, What we have not taken (facts only, no cause, no fix). {}", input.comment.display(), POSTER));
    lines.push("Reports: every step writes its report to the path given in its prompt, starting with one plain sentence and ending with RESULT: PASS or RESULT: FAIL.".into());
    lines.push(crate::teams::prompt::rule_line(true).into());
    lines.push(String::new());
    lines.join("\n")
}

pub fn flow_brief(flow: &PrFlow) -> Value {
    let done: Vec<String> = flow
        .done
        .iter()
        .map(|d| {
            let commit = if d.commit.is_empty() { String::new() } else { format!(" {}", d.commit.chars().take(7).collect::<String>()) };
            let comment = match d.comment {
                CommentState::Waiting => ", comment ready",
                CommentState::Posted => ", comment posted",
                CommentState::None => "",
            };
            format!("#{}: {}{commit}{comment}", d.number, d.outcome.word())
        })
        .collect();
    let mut out = json!({"id": flow.id, "team": flow.team_name, "repo": flow.repo, "branch": flow.branch, "state": flow.state.word(), "queue": flow.queue.iter().map(|n| format!("#{n}")).collect::<Vec<_>>(), "done": done});
    if !flow.space_name.is_empty() {
        out["space"] = json!(flow.space_name);
    }
    if let Some(current) = &flow.current {
        out["current"] = json!(format!("#{} (run {})", current.number, current.run));
    }
    if !flow.notes.is_empty() {
        out["notes"] = json!(flow.notes);
    }
    if !flow.note.is_empty() {
        out["note"] = json!(flow.note);
    }
    out
}

pub fn end_text(flow: &PrFlow, waiting: &[u64]) -> (String, String) {
    let where_ = if flow.space_name.is_empty() { short_name(&flow.repo) } else { flow.space_name.clone() };
    let parts: Vec<String> = flow
        .done
        .iter()
        .map(|d| match d.outcome {
            Outcome::Picked if !d.commit.is_empty() => format!("#{} picked (commit {})", d.number, d.commit.chars().take(7).collect::<String>()),
            Outcome::Picked => format!("#{} picked, Nixa commits it", d.number),
            Outcome::Nothing => format!("#{} nothing to take", d.number),
            Outcome::Failed => format!("#{} failed", d.number),
        })
        .collect();
    let head = if flow.state == FlowState::Done {
        let list = if parts.is_empty() { String::new() } else { format!(": {}", parts.join(", ")) };
        format!("The {} is done in {where_}: {}{list}.", flow.team_name, plural(flow.done.len(), "PR"))
    } else {
        let how = if flow.state == FlowState::Stopped { "is stopped" } else { "stopped" };
        let note = if flow.note.is_empty() { ".".to_string() } else { format!(": {}", flow.note) };
        let so_far = if parts.is_empty() { String::new() } else { format!(" Done so far: {}.", parts.join(", ")) };
        format!("The {} in {where_} {how}{note}{so_far}", flow.team_name)
    };
    let numbers = waiting.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ");
    let comments = if waiting.is_empty() { "No comment draft waits.".to_string() } else { format!("Nixa posts {}: {numbers}.", plural(waiting.len(), "comment draft")) };
    let skipped = flow.notes.join(" ");
    let state = "Nixa decides the commit and the GitHub steps next.";
    let chat = [head.clone(), skipped, comments.clone(), state.into()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
    let spoken = format!("{head} {comments}");
    (chat, spoken)
}

fn flows(app: &App) -> &Flows {
    &app.git_tools.flows
}

fn save(app: &App) {
    let state = flows(app);
    let file = lock(&state.file).clone();
    write_file(&state.path, &file);
    app.send_out(json!({"type": CHANGED}));
}

fn change<R>(app: &App, id: &str, edit: impl FnOnce(&mut PrFlow) -> R) -> Option<R> {
    let out = {
        let mut file = lock(&flows(app).file);
        file.flows.iter_mut().find(|f| f.id == id).map(edit)
    };
    if out.is_some() {
        save(app);
    }
    out
}

pub fn get(app: &App, id: &str) -> Option<PrFlow> {
    lock(&flows(app).file).flows.iter().find(|f| f.id == id).cloned()
}

pub fn list(app: &App) -> Vec<PrFlow> {
    lock(&flows(app).file).flows.clone()
}

pub fn prs(app: &App) -> Vec<PrRecord> {
    lock(&flows(app).file).prs.values().cloned().collect()
}

fn new_id(app: &App) -> String {
    let n = {
        let mut made = lock(&flows(app).made);
        *made += 1;
        *made
    };
    let at = crate::util::now_ms();
    let hash = crate::util::short_hash(&format!("{at}-{n}-{}", std::process::id()));
    format!("pr-{}-{}", crate::util::day_of(at), &hash[..6])
}

async fn must(root: &str, args: &[&str], what: &str) -> Result<String, String> {
    let out = git(root, args, LONG_MS).await;
    if !out.ok() {
        return Err(format!("{what} failed: {}", out.last_line()));
    }
    Ok(out.stdout)
}

pub async fn dirty_files(root: &str) -> Result<Vec<String>, String> {
    let text = must(root, &["status", "--porcelain=v1", "-uall"], "git status").await?;
    let mut out: Vec<String> = text.lines().filter_map(|l| l.get(3..)).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    out.sort();
    Ok(out)
}

pub async fn recent_change(root: &str, recent_ms: u64, now: u64) -> bool {
    let out = git(root, &["status", "--porcelain=v1", "-z", "-uall"], LONG_MS).await;
    if !out.ok() {
        return false;
    }
    out.stdout.split('\0').filter(|e| e.len() >= 4).filter_map(|e| e.get(3..)).any(|path| {
        std::fs::metadata(Path::new(root).join(path))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|t| now.saturating_sub(t.as_millis() as u64) < recent_ms)
    })
}

async fn account(app: &App, repo: &str) -> Option<String> {
    let found = super::identity::prepare(&super::identity::prep_for(&app.dir, repo, true)).await;
    let first = lock(&flows(app).noted).insert(repo.to_string());
    if first || !found.repaired.is_empty() {
        for note in &found.notes {
            crate::teams::line(app, &format!("PR team: {note}"));
        }
    }
    found.account
}

fn record(app: &App, flow: &PrFlow, pr: &RunPr, outcome: Outcome, run: &str, comment: Option<&Path>) {
    let key = pr_key(&flow.slug, pr.number);
    let mut file = lock(&flows(app).file);
    let before = file.prs.get(&key).cloned().unwrap_or_default();
    let posted = before.comment_state == CommentState::Posted;
    let entry = PrRecord {
        key: key.clone(),
        repo: flow.repo.clone(),
        number: pr.number,
        title: pr.title.clone(),
        url: pr.url.clone(),
        head: if pr.head.is_empty() { before.head.clone() } else { pr.head.clone() },
        outcome,
        commit: before.commit.clone(),
        comment: comment.map(|c| c.display().to_string()).unwrap_or(before.comment.clone()),
        comment_state: if posted { CommentState::Posted } else if comment.is_some() { CommentState::Waiting } else { CommentState::None },
        author: if pr.author.is_empty() { before.author.clone() } else { pr.author.clone() },
        run: if run.is_empty() { before.run.clone() } else { run.to_string() },
        flow: flow.id.clone(),
        at: crate::util::now_ms(),
    };
    file.prs.insert(key, entry);
}

fn end_flow(app: &Arc<App>, id: &str, state: FlowState, note: &str) {
    let Some(flow) = change(app, id, |flow| {
        flow.state = state;
        flow.ended_at = Some(crate::util::now_ms());
        flow.current = None;
        flow.note = note.to_string();
        flow.clone()
    }) else {
        return;
    };
    lock(&flows(app).extra).remove(id);
    let waiting: Vec<u64> = prs(app).into_iter().filter(|p| p.repo == flow.repo && p.comment_state == CommentState::Waiting).map(|p| p.number).collect();
    let (chat, spoken) = end_text(&flow, &waiting);
    crate::teams::line(app, &chat);
    crate::teams::say(app, &spoken);
    if note != USER_STOP {
        crate::watch::tell_event(app, &end_event(&flow, &waiting));
    }
}

pub fn end_event(flow: &PrFlow, waiting: &[u64]) -> String {
    let picked: Vec<String> = flow.done.iter().filter(|d| d.outcome == Outcome::Picked && d.commit.is_empty()).map(|d| format!("#{}", d.number)).collect();
    let mut steps = Vec::new();
    if !picked.is_empty() {
        steps.push(format!("commit the picked parts of {} in {} with nixa_git_commit", picked.join(", "), flow.repo));
    }
    if !waiting.is_empty() {
        steps.push(format!("post the comment drafts of {} with nixa_pr_flow post", waiting.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ")));
    }
    if flow.state != FlowState::Stopped {
        steps.push(format!("start the {} again with nixa_team_run start for the PRs left; reviewed PRs are skipped", flow.team_name));
    }
    if steps.is_empty() {
        steps.push("decide the next step yourself".into());
    }
    format!("Event: PR flow {} ended ({}). Do not wait for the user: {}. Record each step in one short line.", flow.id, flow.state.word(), steps.join("; then "))
}

pub fn mark_posted(app: &App, repo: &str, number: u64) {
    let changed = {
        let mut file = lock(&flows(app).file);
        let mut hit = false;
        for pr in file.prs.values_mut().filter(|p| p.repo == repo && p.number == number) {
            pr.comment_state = CommentState::Posted;
            hit = true;
        }
        for flow in file.flows.iter_mut().filter(|f| f.repo == repo) {
            for item in flow.done.iter_mut().filter(|d| d.number == number && d.comment == CommentState::Waiting) {
                item.comment = CommentState::Posted;
                hit = true;
            }
        }
        hit
    };
    if changed {
        save(app);
    }
}

fn fail_pr(app: &App, id: &str, pr: &RunPr, note: String) {
    if let Some(flow) = get(app, id) {
        record(app, &flow, pr, Outcome::Failed, "", None);
    }
    change(app, id, |flow| {
        flow.done.push(DoneItem { number: pr.number, outcome: Outcome::Failed, ..DoneItem::default() });
        flow.notes.push(note);
    });
}

async fn begin_pr(app: &Arc<App>, flow: &PrFlow, number: u64, found: &OpenPr, extra: &Extra) -> Result<(RunPr, String), String> {
    let mut pr = RunPr { number, title: found.title.clone(), url: found.url.clone(), author: found.author.clone(), ..RunPr::default() };
    let file = crate::teams::runs::teams_file(app)?;
    let crew = crate::teams::crew::find_team(&file, &flow.team).cloned().ok_or_else(|| format!("the team {} is gone.", flow.team))?;
    let local = format!("refs/nixa/pr/{number}");
    let fetch = format!("+refs/pull/{number}/head:{local}");
    must(&flow.repo, &["fetch", "--no-tags", "origin", &fetch], &format!("git fetch of PR #{number}")).await?;
    pr.head = must(&flow.repo, &["rev-parse", &local], "git rev-parse").await?.trim().to_string();
    let mut facts: Option<PrFacts> = None;
    if parse_pr_url(&pr.url).is_some() && account(app, &flow.repo).await.is_some() {
        match facts_for(app, &pr.url, Some(&flow.repo)).await {
            Ok(found) => {
                if pr.author.is_empty() {
                    pr.author = found.author.clone();
                }
                facts = Some(found);
            }
            Err(e) => {
                change(app, &flow.id, |f| f.notes.push(format!("PR #{number}: GitHub facts not read ({e}).")));
            }
        }
    }
    let dir = Path::new(&flow.repo).join("docs").join("team-runs").join(&flow.id).join(format!("pr-{number}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let base = must(&flow.repo, &["merge-base", &flow.branch, &local], "git merge-base").await?.trim().to_string();
    let diff = dir.join(format!("pr-{number}.diff"));
    let diff_text = must(&flow.repo, &["diff", "--no-color", &base, &local], "git diff").await?;
    std::fs::write(&diff, diff_text).map_err(|e| format!("cannot write {}: {e}", diff.display()))?;
    let stat = must(&flow.repo, &["diff", "--stat=120", &base, &local], "git diff --stat").await?;
    let comment = dir.join("comment.md");
    let brief = dir.join("brief.md");
    let skills = crate::teams::crew::names(&[crew.skills.clone(), extra.skills.clone()].concat());
    let mcps = crate::teams::crew::names(&[crew.mcps.clone(), extra.mcps.clone()].concat());
    let skill_dir = crate::teams::sources::skill_dir();
    let text = brief_text(&BriefInput { flow, pr: &pr, purpose: &crew.purpose, task: &extra.task, facts: facts.as_ref(), diff: &diff, stat: &stat, comment: &comment, skills: &skills, mcps: &mcps, skill_dir: &skill_dir });
    std::fs::write(&brief, text).map_err(|e| format!("cannot write {}: {e}", brief.display()))?;
    let space = (!flow.space.is_empty()).then(|| Space { id: flow.space.clone(), name: flow.space_name.clone(), folder: flow.repo.clone() });
    let input = crate::teams::RunStart {
        team: flow.team.clone(),
        task: format!("PR #{number}: {}", pr.title),
        project: flow.repo.clone(),
        by: flow.by,
        space,
        branch: flow.branch.clone(),
        skills: extra.skills.clone(),
        mcps: extra.mcps.clone(),
        brief: brief.display().to_string(),
        flow: flow.id.clone(),
        pr: Some(pr.clone()),
        ..crate::teams::RunStart::default()
    };
    let run = crate::teams::runs::start(app, input).map_err(|e| format!("the run did not start ({e})"))?;
    Ok((pr, run.id))
}

pub async fn next_pr(app: &Arc<App>, id: &str) {
    loop {
        let Some(flow) = get(app, id).filter(|f| f.state == FlowState::Running) else { return };
        let Some(number) = change(app, id, |f| (!f.queue.is_empty()).then(|| f.queue.remove(0))).flatten() else {
            return end_flow(app, id, FlowState::Done, "");
        };
        let extra = lock(&flows(app).extra).get(id).cloned().unwrap_or_default();
        let Some(found) = extra.candidates.get(&number).cloned() else {
            change(app, id, |f| f.notes.push(format!("PR #{number} was not in the list any more.")));
            continue;
        };
        match begin_pr(app, &flow, number, &found, &extra).await {
            Ok((pr, run)) => {
                record(app, &flow, &pr, Outcome::Failed, &run, None);
                change(app, id, |f| f.current = Some(Current { number, run: run.clone() }));
                crate::teams::line(app, &format!("{}: started on PR #{number} (run {run}).", flow.team_name));
                return;
            }
            Err(why) => {
                let pr = RunPr { number, title: found.title.clone(), url: found.url.clone(), author: found.author.clone(), ..RunPr::default() };
                crate::teams::line(app, &format!("{}: PR #{number} skipped: {why}", flow.team_name));
                fail_pr(app, id, &pr, format!("PR #{number}: {why}"));
            }
        }
    }
}

pub async fn reset_tree(repo: &str) -> Result<(), String> {
    must(repo, &["reset", "-q", "--hard"], "git reset").await?;
    must(repo, &["clean", "-fdq"], "git clean").await?;
    Ok(())
}

pub fn ended(app: &Arc<App>, run: &Run) {
    if run.flow.is_empty() || run.pr.is_none() {
        return;
    }
    let Some(flow) = get(app, &run.flow).filter(|f| f.state == FlowState::Running) else { return };
    if flow.current.as_ref().map(|c| c.run.as_str()) != Some(run.id.as_str()) {
        return;
    }
    let app = app.clone();
    let run = run.clone();
    tokio::spawn(async move { settle(&app, &flow, &run).await });
}

pub fn comment_ready(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| !t.trim().is_empty())
}

pub async fn settle(app: &Arc<App>, flow: &PrFlow, run: &Run) {
    let Some(pr) = run.pr.clone() else { return };
    let comment = Path::new(&flow.repo).join("docs").join("team-runs").join(&flow.id).join(format!("pr-{}", pr.number)).join("comment.md");
    let has_comment = comment_ready(&comment);
    let comment_state = if has_comment { CommentState::Waiting } else { CommentState::None };
    let drafted = has_comment.then_some(comment.as_path());
    change(app, &flow.id, |f| f.current = None);
    let ready = if has_comment { " Comment draft ready." } else { "" };
    let settled: Result<Option<String>, String> = async {
        match run.state {
            RunState::Done => {
                let files = dirty_files(&flow.repo).await?;
                if !files.is_empty() {
                    record(app, flow, &pr, Outcome::Picked, &run.id, drafted);
                    change(app, &flow.id, |f| f.done.push(DoneItem { number: pr.number, outcome: Outcome::Picked, comment: comment_state, ..DoneItem::default() }));
                    crate::teams::line(app, &format!("{}: PR #{} picked parts are left uncommitted in the folder ({}). Waiting for you: commit. Nixa does not commit.{ready}", flow.team_name, pr.number, plural(files.len(), "file")));
                    let left: Vec<u64> = get(app, &flow.id).map(|f| f.queue.clone()).unwrap_or_default();
                    if !left.is_empty() {
                        change(app, &flow.id, |f| f.queue.clear());
                        let list = left.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ");
                        return Ok(Some(format!("PR #{}'s picked parts wait for your commit, so {list} wait for the next run", pr.number)));
                    }
                } else {
                    record(app, flow, &pr, Outcome::Nothing, &run.id, drafted);
                    change(app, &flow.id, |f| f.done.push(DoneItem { number: pr.number, outcome: Outcome::Nothing, comment: comment_state, ..DoneItem::default() }));
                    crate::teams::line(app, &format!("{}: PR #{} left nothing to take.{ready}", flow.team_name, pr.number));
                }
                Ok(None)
            }
            RunState::Stopped | RunState::Cut => {
                reset_tree(&flow.repo).await?;
                record(app, flow, &pr, Outcome::Failed, &run.id, None);
                change(app, &flow.id, |f| f.done.push(DoneItem { number: pr.number, outcome: Outcome::Failed, ..DoneItem::default() }));
                Ok(Some(format!("the run of PR #{} was stopped", pr.number)))
            }
            _ => {
                reset_tree(&flow.repo).await?;
                record(app, flow, &pr, Outcome::Failed, &run.id, None);
                change(app, &flow.id, |f| f.done.push(DoneItem { number: pr.number, outcome: Outcome::Failed, ..DoneItem::default() }));
                let why = if run.note.is_empty() { run.state.word().to_string() } else { run.note.clone() };
                crate::teams::line(app, &format!("{}: PR #{} failed ({why}). The tree is reset; on to the next PR.", flow.team_name, pr.number));
                Ok(None)
            }
        }
    }
    .await;
    match settled {
        Ok(Some(note)) => end_flow(app, &flow.id, FlowState::Stopped, &note),
        Ok(None) => next_or_hold(app, flow).await,
        Err(e) => {
            change(app, &flow.id, |f| f.notes.push(format!("PR #{}: {e}", pr.number)));
            crate::teams::line(app, &format!("{}: PR #{}: {e}", flow.team_name, pr.number));
            next_or_hold(app, flow).await;
        }
    }
}

async fn next_or_hold(app: &Arc<App>, flow: &PrFlow) {
    let left = get(app, &flow.id).is_some_and(|f| !f.queue.is_empty());
    if left && let Some(held) = crate::modes::hold_work(app, "team-step", &format!("the next PR of the {}", flow.team_name), None) {
        return end_flow(app, &flow.id, FlowState::Stopped, &format!("The next PR waits. {held}"));
    }
    next_pr(app, &flow.id).await;
}

fn running_on<'a>(file: &'a WorkFile, repo: &str) -> Option<&'a PrFlow> {
    file.flows.iter().find(|f| f.state == FlowState::Running && f.repo == repo)
}

pub async fn start(app: &Arc<App>, input: FlowStart) -> Result<PrFlow, String> {
    let file = crate::teams::runs::teams_file(app)?;
    let wanted = input.team.trim();
    let wanted = wanted.get(..5).filter(|h| h.eq_ignore_ascii_case("team:")).map(|_| wanted[5..].trim()).unwrap_or(wanted);
    let crew = crate::teams::crew::find_team(&file, wanted).cloned().ok_or_else(|| format!("No team \"{wanted}\"."))?;
    if crew.flow != "pr" {
        return Err(format!("The {} is not a PR team. Set its flow to pr in Teams.", crew.name));
    }
    if crew.steps.is_empty() {
        return Err(format!("The {} has no steps.", crew.name));
    }
    let repo = input.project.trim().trim_end_matches('/').to_string();
    if !git(&repo, &["rev-parse", "--show-toplevel"], LONG_MS).await.ok() {
        return Err(format!("{repo} is not a git repo. Pick the repo folder."));
    }
    let remote = git(&repo, &["remote", "get-url", "origin"], LONG_MS).await;
    let origin = if remote.ok() { parse_remote(remote.stdout.trim()) } else { None };
    let Some(origin) = origin else {
        return Err(format!("{} has no origin remote, so its PRs cannot be found.", short_name(&repo)));
    };
    let slug = format!("{}/{}/{}", origin.host, origin.owner, origin.repo);
    let current = git(&repo, &["branch", "--show-current"], LONG_MS).await.stdout.trim().to_string();
    let branch = if input.branch.trim().is_empty() { current.clone() } else { input.branch.trim().to_string() };
    if branch.is_empty() {
        return Err(format!("{} is not on a branch.", short_name(&repo)));
    }
    if current != branch {
        return Err(format!("{} is on branch {}, not {branch}. Switch it, or pick {current} in the form.", short_name(&repo), if current.is_empty() { "(none)" } else { &current }));
    }
    if let Some(going) = running_on(&lock(&flows(app).file), &repo) {
        return Err(format!("The {} already works in {} (flow {}). Stop it first.", going.team_name, short_name(&repo), going.id));
    }
    let _ = crate::schedules::runner::exclude(&repo).await;
    if recent_change(&repo, RECENT_MS, crate::util::now_ms()).await {
        return Err(format!("{} has files changed in the last {} minutes. Someone may be working there. The PR team waits.", short_name(&repo), RECENT_MS / 60_000));
    }
    let mut flow = PrFlow {
        id: new_id(app),
        team: crew.id.clone(),
        team_name: crew.name.clone(),
        repo: repo.clone(),
        slug: slug.clone(),
        branch: branch.clone(),
        space: input.space.as_ref().map(|s| s.id.clone()).unwrap_or_default(),
        space_name: input.space.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
        by: input.by,
        state: FlowState::Running,
        started_at: crate::util::now_ms(),
        ..PrFlow::default()
    };
    let where_ = if flow.space_name.is_empty() { short_name(&repo) } else { flow.space_name.clone() };
    let user = account(app, &repo).await.ok_or_else(|| format!("{} has no git account in the git profiles, so its open PRs cannot be read. Pick a profile for it in Settings, Git accounts.", short_name(&repo)))?;
    let base = super::github::api_base(&origin.host, std::env::var(super::github::API_ENV).ok());
    let all = open_prs(&app.http, &origin, &user, &base).await.map_err(|e| format!("Could not read the open PRs: {e}"))?;
    let mut here: Vec<(u64, OpenPr)> = all.into_iter().map(|p| (p.number, p)).collect();
    let only = wanted_number(&input.task);
    if let Some(only) = only {
        here.retain(|(n, _)| *n == only);
        if here.is_empty() {
            return Err(format!("PR #{only} is not open in {}/{}.", origin.owner, origin.repo));
        }
    }
    here.sort_by(|a, b| a.1.created_at.cmp(&b.1.created_at).then(a.0.cmp(&b.0)));
    let mut candidates = BTreeMap::new();
    let mut queue = Vec::new();
    let mut skipped = 0;
    let known = lock(&flows(app).file).prs.clone();
    for (number, pr) in here {
        if let Some(seen) = known.get(&pr_key(&slug, number)).filter(|k| !k.head.is_empty() && (k.outcome != Outcome::Failed || only.is_none())) {
            let head = git(&repo, &["ls-remote", "origin", &format!("refs/pull/{number}/head")], LONG_MS).await;
            let sha = if head.ok() { head.stdout.split_whitespace().next().unwrap_or_default().to_string() } else { String::new() };
            if !sha.is_empty() && sha == seen.head {
                flow.notes.push(if seen.outcome == Outcome::Failed { format!("PR #{number} failed at its current commit last time; skipped. Run the PR team with only PR {number} to try again.") } else { format!("PR #{number} was already reviewed at its current commit.") });
                skipped += 1;
                continue;
            }
        }
        queue.push(number);
        candidates.insert(number, pr);
    }
    flow.queue = queue;
    if !flow.queue.is_empty() {
        let dirty = dirty_files(&repo).await?;
        if !dirty.is_empty() {
            return Err(format!("{} has {}. Nixa does not commit. Commit or stash them yourself, then start the {} again.", short_name(&repo), plural(dirty.len(), "uncommitted file"), crew.name));
        }
        let fetched = git(&repo, &["fetch", "--no-tags", "origin", &branch], LONG_MS).await;
        if !fetched.ok() {
            flow.notes.push(format!("git fetch of {branch} failed: {}.", fetched.last_line()));
        } else {
            let counts: Out = git(&repo, &["rev-list", "--left-right", "--count", &format!("{branch}...origin/{branch}")], LONG_MS).await;
            let (ahead, behind) = if counts.ok() { super::repo::pair_of(&counts.stdout) } else { (0, 0) };
            if behind > 0 && ahead == 0 {
                let ff = git(&repo, &["merge", "--ff-only", &format!("origin/{branch}")], LONG_MS).await;
                if ff.ok() {
                    crate::teams::line(app, &format!("{}: {where_} was {} behind origin and is now up to date.", crew.name, plural(behind as usize, "commit")));
                } else {
                    flow.notes.push(format!("{branch} could not fast-forward to origin."));
                }
            } else if behind > 0 && ahead > 0 {
                let text = format!("{branch} is {} ahead and {} behind origin; Nixa works on the local branch.", plural(ahead as usize, "commit"), plural(behind as usize, "commit"));
                crate::teams::line(app, &format!("{}: {text} Merge or rebase when you push.", crew.name));
                flow.notes.push(text);
            }
        }
    }
    lock(&flows(app).extra).insert(flow.id.clone(), Extra { candidates, skills: input.skills.clone(), mcps: input.mcps.clone(), task: input.task.trim().to_string() });
    lock(&flows(app).file).flows.push(flow.clone());
    save(app);
    let list = if flow.queue.is_empty() { String::new() } else { format!(": {}", flow.queue.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ")) };
    let skipped_text = if skipped > 0 { format!(" {} skipped.", plural(skipped, "PR")) } else { String::new() };
    crate::teams::line(app, &format!("{}: flow {} started in {where_} ({repo}, branch {branch}). {} to do{list}.{skipped_text}", crew.name, flow.id, plural(flow.queue.len(), "open PR")));
    crate::teams::say(app, &format!("Starting the {} in {where_} on {}.", crew.name, plural(flow.queue.len(), "open PR")));
    next_pr(app, &flow.id).await;
    Ok(get(app, &flow.id).unwrap_or(flow))
}

pub fn stop(app: &Arc<App>, id: &str) -> Result<PrFlow, String> {
    let flow = get(app, id.trim()).ok_or_else(|| format!("No PR flow \"{}\".", id.trim()))?;
    if flow.state != FlowState::Running {
        return Err(format!("The PR flow {} is not running. It is {}.", flow.id, flow.state.word()));
    }
    if let Some(current) = &flow.current
        && crate::teams::runs::stop(app, &current.run).is_ok()
    {
        return Ok(get(app, &flow.id).unwrap_or(flow));
    }
    end_flow(app, &flow.id, FlowState::Stopped, USER_STOP);
    Ok(get(app, &flow.id).unwrap_or(flow))
}
