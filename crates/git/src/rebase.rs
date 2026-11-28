use crate::errors::describe;
use crate::ops::{self, Identity, Message, State, compose_message, identity_gate, split_message_text};
use crate::outcome::{Outcome, described, failure, plural, refusal, success};
use crate::remote;
use crate::run::{self, Job, Out};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

pub const ACTIONS: [&str; 5] = ["pick", "reword", "squash", "fixup", "drop"];

pub const JOINS: [&str; 2] = ["squash", "fixup"];

pub const MAX_COMMITS: usize = 200;

pub const MAX_TODO: usize = 400000;

pub const REBASE_FLAGS: [&str; 5] = ["--autostash", "--no-autosquash", "--no-update-refs", "--no-rebase-merges", "--empty=keep"];

pub const AMEND_COMMAND: &str = "git commit --amend --only --no-verify --allow-empty --cleanup=whitespace -F -";

pub const CHANGED_MARK: &str = "nixa-git: the branch changed while the rebase was starting";

pub const MAX_LISTED: usize = 10;

pub const REBASE_SECS: u64 = 600;

const MAX_DEEPER: usize = 50;

const IGNORED_LIST: [&str; 6] = ["ls-files", "-z", "-o", "-i", "--exclude-standard", "--full-name"];

const CHECK_AWK: &str = r#"BEGIN { n = split(want, w, " ") } $1 == "pick" || $1 == "p" { i++; s = tolower($2); if (i > n || length(s) < 4 || s !~ /^[0-9a-f]+$/ || index(w[i], s) != 1) bad = 1 } END { if (bad || i != n) exit 1; exit 0 }"#;

const LOG_FORMAT: &str = "%H%x00%h%x00%P%x00%T%x00%an%x00%ae%x00%at%x00%s%x00%b%x1e";

const NO_CHANGE: [char; 3] = [' ', '?', '!'];

static SHA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[0-9a-f]{7,64}$").expect("sha"));

static TODO_BROKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)invalid line \d+|could not parse|unknown command|cannot squash without a previous commit|please fix this using 'git rebase --edit-todo'")
        .expect("todo broken")
});

static CONTROL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\x00-\x1f\x7f]+").expect("control"));

static AMEND_DROP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]").expect("amend drop"));

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RebaseStep {
    pub sha: String,
    pub action: String,
    pub message: Option<Message>,
    pub subject: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RebaseCommit {
    pub sha: String,
    pub short: String,
    pub parents: Vec<String>,
    pub tree: String,
    pub author: String,
    pub email: String,
    pub at: i64,
    pub subject: String,
    pub body: String,
    pub is_merge: bool,
    pub pushed: bool,
    pub empty: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    pub index: i64,
    pub sha: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StepCheck {
    pub ok: bool,
    pub problems: Vec<Problem>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepSummary {
    pub total: usize,
    pub kept: usize,
    pub dropped: usize,
    pub squashed: usize,
    pub reworded: usize,
    pub moved: bool,
    pub first_change: i64,
    pub pushed_changed: usize,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatusEntry {
    pub x: char,
    pub y: char,
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalChanges {
    pub dirty: bool,
    pub staged: bool,
    pub mixed: Vec<String>,
    pub unmerged: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AtRisk {
    pub risky: Vec<String>,
    pub deeper: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub base: String,
    pub base_short: String,
    pub base_subject: String,
    pub base_tree: String,
    pub onto: String,
    pub upstream: Option<String>,
    pub head: String,
    pub head_ref: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub count: usize,
    pub commits: Vec<RebaseCommit>,
    pub merges: bool,
    pub pushed: usize,
    pub dirty: bool,
    pub staged: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StepRef {
    pub index: usize,
    pub sha: String,
    pub short: String,
    pub subject: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StepConflict {
    pub index: usize,
    pub sha: String,
    pub short: String,
    pub subject: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RebaseOpts {
    pub base: Option<String>,
    pub expect_head: Option<String>,
    pub steps: Vec<RebaseStep>,
    pub identity: Identity,
    pub job: Job,
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn one_line(text: &str) -> String {
    CONTROL.replace_all(text, " ").trim().to_string()
}

fn thrown(text: &str) -> Outcome {
    failure(text)
}

pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

pub fn message_of(message: &Message) -> String {
    compose_message(&message.summary, &message.body)
}

pub fn amend_line(message: &Message) -> String {
    let text = message_of(message).replace("\r\n", "\n").replace('\r', "\n");
    let text = AMEND_DROP.replace_all(&text, "");
    let text = text.trim_end_matches('\n');
    let quoted: Vec<String> = text.split('\n').map(shell_quote).collect();
    format!("exec printf '%s\\n' {} | {AMEND_COMMAND}", quoted.join(" "))
}

fn own_message(commit: Option<&RebaseCommit>) -> Message {
    commit.map(|c| Message { summary: c.subject.clone(), body: c.body.clone() }).unwrap_or_default()
}

fn same_message(a: &Message, b: &Message) -> bool {
    message_of(a).trim() == message_of(b).trim()
}

fn by_sha(commits: &[RebaseCommit]) -> HashMap<&str, &RebaseCommit> {
    commits.iter().map(|c| (c.sha.as_str(), c)).collect()
}

pub fn start_steps(commits: &[RebaseCommit]) -> Vec<RebaseStep> {
    commits.iter().map(|c| RebaseStep { sha: c.sha.clone(), action: "pick".into(), ..RebaseStep::default() }).collect()
}

pub fn move_step(steps: &[RebaseStep], index: usize, delta: isize) -> Vec<RebaseStep> {
    let to = index as isize + delta;
    if index >= steps.len() || to < 0 || to as usize >= steps.len() || to as usize == index {
        return steps.to_vec();
    }
    let mut next = steps.to_vec();
    let moved = next.remove(index);
    next.insert(to as usize, moved);
    next
}

pub fn join_target(steps: &[RebaseStep], index: usize) -> Option<usize> {
    (0..index.min(steps.len())).rev().find(|&i| steps[i].action == "pick" || steps[i].action == "reword")
}

pub fn joined_message(steps: &[RebaseStep], commits: &[RebaseCommit], index: usize) -> Message {
    let map = by_sha(commits);
    let own = |step: &RebaseStep| own_message(map.get(step.sha.as_str()).copied());
    let Some(step) = steps.get(index) else { return Message::default() };
    let Some(head) = join_target(steps, index) else { return own(step) };
    let first = &steps[head];
    let start = match (&first.message, first.action.as_str()) {
        (Some(m), "reword") => m.clone(),
        _ => own(first),
    };
    let mut text = message_of(&start).trim().to_string();
    for (j, s) in steps.iter().enumerate().take(index + 1).skip(head + 1) {
        if s.action != "squash" {
            continue;
        }
        match &s.message {
            Some(m) if j != index => text = message_of(m).trim().to_string(),
            _ => {
                let more = message_of(&own(s)).trim().to_string();
                text = [text, more].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n\n");
            }
        }
    }
    split_message_text(&text)
}

pub fn validate_steps(steps: &[RebaseStep], commits: Option<&[RebaseCommit]>) -> StepCheck {
    let mut problems: Vec<Problem> = Vec::new();
    let mut add = |index: i64, text: String| {
        let sha = if index >= 0 { steps.get(index as usize).map(|s| s.sha.clone()) } else { None };
        problems.push(Problem { index, sha, text });
    };
    if steps.is_empty() {
        let text = "There are no commits to change.".to_string();
        return StepCheck { ok: false, problems: vec![Problem { index: -1, sha: None, text: text.clone() }], problem: Some(text) };
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for (i, step) in steps.iter().enumerate() {
        let sha = step.sha.as_str();
        if !SHA.is_match(sha) {
            add(i as i64, "A commit id is missing or wrong.".into());
        } else if seen.contains(sha) {
            add(i as i64, format!("Commit {} is in the list twice.", short_sha(sha)));
        } else {
            seen.insert(sha);
        }
        if !ACTIONS.contains(&step.action.as_str()) {
            add(i as i64, format!("\"{}\" is not a known action.", one_line(&step.action)));
        }
        let needs = step.action == "reword" || (step.action == "squash" && step.message.is_some());
        let summary = step.message.as_ref().map(|m| m.summary.trim().to_string()).unwrap_or_default();
        if needs && summary.is_empty() {
            add(i as i64, format!("Write a summary for {}.", short_sha(sha)));
        }
    }
    if let Some(commits) = commits {
        let want: HashSet<&str> = commits.iter().map(|c| c.sha.as_str()).collect();
        if want.len() != seen.len() || seen.iter().any(|sha| !want.contains(sha)) {
            add(-1, "The list does not match the branch any more. Load it again.".into());
        }
    }
    match steps.iter().position(|s| s.action != "drop") {
        None => add(-1, "Keep at least one commit.".into()),
        Some(first) if JOINS.contains(&steps[first].action.as_str()) => add(
            first as i64,
            "The first kept commit cannot be joined. There is no commit above it to join into.".into(),
        ),
        Some(_) => {}
    }
    let problem = problems.first().map(|p| p.text.clone());
    StepCheck { ok: problems.is_empty(), problems, problem }
}

pub fn build_todo(steps: &[RebaseStep], commits: &[RebaseCommit]) -> Result<String, String> {
    let check = validate_steps(steps, None);
    if !check.ok {
        return Err(check.problem.unwrap_or_default());
    }
    let known = !commits.is_empty();
    let map = by_sha(commits);
    let mut out: Vec<String> = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let subject = step.subject.clone().or_else(|| map.get(step.sha.as_str()).map(|c| c.subject.clone())).unwrap_or_default();
        let subject = one_line(&subject);
        let tail = if subject.is_empty() { String::new() } else { format!(" {subject}") };
        if step.action == "reword" {
            out.push(format!("pick {}{tail}", step.sha));
            out.push(amend_line(&step.message.clone().unwrap_or_default()));
            continue;
        }
        out.push(format!("{} {}{tail}", step.action, step.sha));
        if step.action != "squash" {
            continue;
        }
        if let Some(message) = &step.message {
            out.push(amend_line(message));
            continue;
        }
        if known {
            let joined = joined_message(steps, commits, i);
            if !joined.summary.trim().is_empty() {
                out.push(amend_line(&joined));
            }
        }
    }
    Ok(format!("{}\n", out.join("\n")))
}

fn same_step(step: Option<&RebaseStep>, commit: Option<&RebaseCommit>) -> bool {
    let (Some(step), Some(commit)) = (step, commit) else { return false };
    if step.sha != commit.sha {
        return false;
    }
    if step.action == "pick" {
        return true;
    }
    step.action == "reword" && step.message.as_ref().is_some_and(|m| same_message(m, &own_message(Some(commit))))
}

pub fn first_change(steps: &[RebaseStep], commits: &[RebaseCommit]) -> Option<usize> {
    let n = steps.len().max(commits.len());
    (0..n).find(|&i| !same_step(steps.get(i), commits.get(i)))
}

pub fn is_unchanged(steps: &[RebaseStep], commits: &[RebaseCommit]) -> bool {
    steps.len() == commits.len() && first_change(steps, commits).is_none()
}

pub fn summarize_steps(steps: &[RebaseStep], commits: &[RebaseCommit]) -> StepSummary {
    let count = |action: &str| steps.iter().filter(|s| s.action == action).count();
    let map = by_sha(commits);
    let reworded = steps
        .iter()
        .filter(|s| match (&s.message, s.action.as_str()) {
            (Some(m), "reword") => !same_message(m, &own_message(map.get(s.sha.as_str()).copied())),
            (Some(_), "squash") => true,
            _ => false,
        })
        .count();
    let moved = commits.len() == steps.len() && commits.iter().map(|c| c.sha.as_str()).ne(steps.iter().map(|s| s.sha.as_str()));
    let first = first_change(steps, commits);
    let pushed_changed = first.map(|f| commits.iter().skip(f).filter(|c| c.pushed).count()).unwrap_or(0);
    let kept = count("pick") + count("reword");
    let dropped = count("drop");
    let squashed = count("squash") + count("fixup");
    let mut parts = vec![format!("{kept} kept"), format!("{dropped} dropped"), format!("{squashed} squashed")];
    if reworded > 0 {
        parts.push(plural(reworded, "new message", "new messages"));
    }
    let text = format!("{}{}.", parts.join(", "), if moved { ". New order" } else { "" });
    StepSummary {
        total: steps.len(),
        kept,
        dropped,
        squashed,
        reworded,
        moved,
        first_change: first.map(|f| f as i64).unwrap_or(-1),
        pushed_changed,
        text,
    }
}

pub fn parse_log(text: &str) -> Vec<RebaseCommit> {
    text.split('\x1e')
        .filter_map(|record| {
            let body = record.strip_prefix('\n').unwrap_or(record);
            if body.trim().is_empty() {
                return None;
            }
            let parts: Vec<&str> = body.split('\0').collect();
            let field = |i: usize| parts.get(i).copied().unwrap_or("").to_string();
            let sha = field(0);
            if !SHA.is_match(&sha) {
                return None;
            }
            let parents: Vec<String> = field(2).split(' ').filter(|p| !p.is_empty()).map(String::from).collect();
            let rest = if parts.len() > 8 { parts[8..].join("\0") } else { String::new() };
            Some(RebaseCommit {
                sha,
                short: field(1),
                is_merge: parents.len() > 1,
                parents,
                tree: field(3),
                author: field(4),
                email: field(5),
                at: field(6).parse().unwrap_or(0),
                subject: field(7),
                body: rest.trim_end().to_string(),
                ..RebaseCommit::default()
            })
        })
        .collect()
}

fn nul_list(text: &str) -> Vec<String> {
    ops::split_z(text)
}

pub fn parse_status_z(text: &str) -> Vec<StatusEntry> {
    let parts: Vec<&str> = text.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let chars: Vec<char> = parts[i].chars().collect();
        i += 1;
        if chars.len() < 4 {
            continue;
        }
        out.push(StatusEntry { x: chars[0], y: chars[1], path: chars[3..].iter().collect() });
        if chars[0] == 'R' || chars[0] == 'C' {
            i += 1;
        }
    }
    out
}

fn is_unmerged(e: &StatusEntry) -> bool {
    e.x == 'U' || e.y == 'U' || (e.x == 'A' && e.y == 'A') || (e.x == 'D' && e.y == 'D')
}

pub fn local_changes(entries: &[StatusEntry]) -> LocalChanges {
    let unmerged = entries.iter().filter(|e| is_unmerged(e)).map(|e| e.path.clone()).collect();
    let mixed = entries
        .iter()
        .filter(|e| !is_unmerged(e) && !NO_CHANGE.contains(&e.x) && !NO_CHANGE.contains(&e.y))
        .map(|e| e.path.clone())
        .collect();
    let staged = entries.iter().any(|e| !is_unmerged(e) && !NO_CHANGE.contains(&e.x));
    LocalChanges { dirty: !entries.is_empty(), staged, mixed, unmerged }
}

fn parents_of(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (1..parts.len()).map(|i| parts[..i].join("/")).collect()
}

pub fn ignored_at_risk(ignored: &[String], touched: &[String]) -> AtRisk {
    let files: HashSet<&str> = touched.iter().map(String::as_str).collect();
    let dirs: HashSet<String> = files.iter().flat_map(|f| parents_of(f)).collect();
    let mut out = AtRisk::default();
    for entry in ignored {
        let name = entry.trim_end_matches('/');
        if name.is_empty() {
            continue;
        }
        if files.contains(name) || parents_of(name).iter().any(|p| files.contains(p.as_str())) {
            out.risky.push(entry.clone());
        } else if dirs.contains(name) {
            if entry.ends_with('/') { out.deeper.push(entry.clone()) } else { out.risky.push(entry.clone()) }
        }
    }
    out
}

fn read_list(root: &Path, args: &[&str]) -> Result<Vec<String>, Box<Outcome>> {
    let r = run::read(root, args);
    if r.ok() { Ok(nul_list(&r.stdout)) } else { Err(Box::new(failure(&r.said()))) }
}

fn ignored_in_danger(root: &Path, base: &str, head: &str) -> Result<Vec<String>, Box<Outcome>> {
    let mut args = IGNORED_LIST.to_vec();
    args.extend(["--directory", "--no-empty-directory", "--", ":(top)"]);
    let entries = read_list(root, &args)?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let range = format!("{base}..{head}");
    let mut touched = read_list(root, &["log", "-z", "--format=", "--name-only", "--no-renames", "--no-relative", &range])?;
    touched.extend(read_list(root, &["ls-tree", "-r", "-z", "--name-only", "--full-tree", base])?);
    let first = ignored_at_risk(&entries, &touched);
    let mut files: Vec<String> = first.risky.clone();
    files.extend(first.deeper.iter().skip(MAX_DEEPER).cloned());
    let deeper: Vec<String> = first.deeper.iter().take(MAX_DEEPER).cloned().collect();
    if !deeper.is_empty() {
        let specs: Vec<String> = deeper.iter().map(|dir| format!(":(top,literal){dir}")).collect();
        let mut inner = IGNORED_LIST.to_vec();
        inner.push("--");
        inner.extend(specs.iter().map(String::as_str));
        let second = ignored_at_risk(&read_list(root, &inner)?, &touched);
        files.extend(second.risky);
        files.extend(second.deeper);
    }
    Ok(files.into_iter().collect::<BTreeSet<_>>().into_iter().collect())
}

fn list_text(paths: &[String]) -> String {
    let shown = paths.iter().take(MAX_LISTED).cloned().collect::<Vec<_>>().join(", ");
    if paths.len() > MAX_LISTED { format!("{shown} and {} more", paths.len() - MAX_LISTED) } else { shown }
}

fn check_script(plan: &Plan) -> String {
    let want = plan.commits.iter().map(|c| c.sha.as_str()).collect::<Vec<_>>().join(" ");
    [
        format!("want={}", shell_quote(&want)),
        format!("head={}", shell_quote(&plan.head)),
        format!("name={}", shell_quote(plan.head_ref.as_deref().unwrap_or("detached HEAD"))),
        "todo=$1".into(),
        "shift".into(),
        "d=$(git rev-parse --git-path rebase-merge) || exit 1".into(),
        "ok=1".into(),
        r#"[ "$(cat "$d/orig-head" 2>/dev/null)" = "$head" ] || ok=0"#.into(),
        r#"[ "$(cat "$d/head-name" 2>/dev/null)" = "$name" ] || ok=0"#.into(),
        format!(r#"[ "$ok" = 1 ] && awk -v want="$want" {} "$1" || ok=0"#, shell_quote(CHECK_AWK)),
        format!(r#"if [ "$ok" != 1 ]; then echo {} >&2; exit 1; fi"#, shell_quote(CHANGED_MARK)),
        r#"cp "$todo" "$1""#.into(),
        String::new(),
    ]
    .join("\n")
}

fn trimmed(out: &Out) -> Option<String> {
    let text = out.stdout.trim();
    if out.ok() && !text.is_empty() { Some(text.to_string()) } else { None }
}

fn resolve_base(root: &Path, base: Option<&str>, head: &str) -> Result<(String, Option<String>), Box<Outcome>> {
    if let Some(name) = base {
        if name.starts_with('-') || name.chars().any(char::is_whitespace) {
            return Err(Box::new(refusal("invalid", "That is not a commit id", "")));
        }
        let spec = format!("{name}^{{commit}}");
        let Some(found) = trimmed(&run::read(root, &["rev-parse", "-q", "--verify", "--end-of-options", &spec])) else {
            return Err(Box::new(refusal("unknownBase", "That commit is not in this repo", "Load the history again and pick a commit from it.")));
        };
        let in_line = run::read(root, &["merge-base", "--is-ancestor", &found, head]);
        if in_line.code == Some(1) {
            return Err(Box::new(refusal("notOnBranch", "That commit is not on this branch", "Pick a commit from the current branch.")));
        }
        if !in_line.ok() {
            return Err(Box::new(failure(&in_line.said())));
        }
        return Ok((found, None));
    }
    let Some(upstream) = trimmed(&run::read(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"])) else {
        return Err(Box::new(refusal(
            "noUpstream",
            "This branch has no remote branch to compare with",
            "Pick a commit in History and use Change commits from here.",
        )));
    };
    let Some(mb) = trimmed(&run::read(root, &["merge-base", "HEAD", "@{upstream}"])) else {
        return Err(Box::new(refusal(
            "noBase",
            &format!("This branch and {upstream} share no commits"),
            "Pick a commit in History and use Change commits from here.",
        )));
    };
    Ok((mb.lines().next().unwrap_or("").to_string(), Some(upstream)))
}

fn with_plan(mut out: Outcome, plan: &Plan) -> Outcome {
    if let Value::Object(map) = json!(plan) {
        out.extra.extend(map);
    }
    out
}

pub fn plan_of(out: &Outcome) -> Option<Plan> {
    serde_json::from_value(Value::Object(out.extra.clone())).ok()
}

fn load_plan(root: &Path, base: Option<&str>) -> Result<(Outcome, Plan), Box<Outcome>> {
    let state = ops::read_state(root).map_err(|e| Box::new(thrown(&e)))?;
    if let Some(op) = &state.op {
        return Err(Box::new(refusal(
            "busy",
            &format!("A {} is in progress", ops::op_word(op).to_lowercase()),
            "Finish it or cancel it in the Changes panel first.",
        )
        .with("state", json!(state))));
    }
    let Some(head) = trimmed(&run::read(root, &["rev-parse", "-q", "--verify", "HEAD^{commit}"])) else {
        return Err(Box::new(refusal("noCommits", "This repo has no commits yet", "")));
    };
    let (base, upstream) = resolve_base(root, base, &head)?;
    let base_r = run::read(root, &["log", "-1", "--format=%h%x00%T%x00%s", &base]);
    if !base_r.ok() {
        return Err(Box::new(failure(&base_r.said())));
    }
    let base_text = base_r.stdout.strip_suffix('\n').unwrap_or(&base_r.stdout).to_string();
    let base_parts: Vec<&str> = base_text.split('\0').collect();
    let part = |i: usize| base_parts.get(i).copied().unwrap_or("").to_string();
    let range = format!("{base}..{head}");
    let count_r = run::read(root, &["rev-list", "--count", &range]);
    if !count_r.ok() {
        return Err(Box::new(failure(&count_r.said())));
    }
    let count: usize = count_r.stdout.trim().parse().unwrap_or(0);
    let head_ref = trimmed(&run::read(root, &["symbolic-ref", "-q", "HEAD"])).filter(|r| r.starts_with("refs/heads/"));
    let mut plan = Plan {
        base_short: part(0),
        base_tree: part(1),
        base_subject: part(2),
        onto: upstream.clone().unwrap_or_else(|| part(0)),
        base: base.clone(),
        upstream,
        head: head.clone(),
        detached: head_ref.is_none(),
        head_ref,
        branch: state.branch.clone(),
        count,
        ..Plan::default()
    };
    if count > MAX_COMMITS {
        return Err(Box::new(with_plan(
            refusal(
                "tooMany",
                &format!("There are {count} commits to change"),
                &format!("This window can change up to {MAX_COMMITS}. Pick a newer commit in History."),
            ),
            &plan,
        )));
    }
    let format = format!("--format={LOG_FORMAT}");
    let log_r = run::read(root, &["-c", "core.quotepath=off", "log", "--topo-order", "--reverse", "--no-color", &format, &range]);
    if !log_r.ok() {
        return Err(Box::new(failure(&log_r.said())));
    }
    let raw = parse_log(&log_r.stdout);
    let mut trees: HashMap<String, String> = raw.iter().map(|c| (c.sha.clone(), c.tree.clone())).collect();
    trees.insert(base.clone(), plan.base_tree.clone());
    let unpushed_r = run::read(root, &["rev-list", &range, "--not", "--remotes"]);
    let unpushed: HashSet<String> =
        if unpushed_r.ok() { ops::lines(&unpushed_r.stdout).into_iter().collect() } else { raw.iter().map(|c| c.sha.clone()).collect() };
    plan.commits = raw
        .iter()
        .map(|c| {
            let parent_tree = c.parents.first().and_then(|p| trees.get(p));
            RebaseCommit { pushed: !unpushed.contains(&c.sha), empty: !c.is_merge && parent_tree == Some(&c.tree), ..c.clone() }
        })
        .collect();
    let status_r = run::read(root, &["status", "--porcelain=v1", "-z", "--untracked-files=no", "--ignore-submodules=all"]);
    if !status_r.ok() {
        return Err(Box::new(failure(&status_r.said())));
    }
    let local = local_changes(&parse_status_z(&status_r.stdout));
    plan.merges = plan.commits.iter().any(|c| c.is_merge);
    plan.pushed = plan.commits.iter().filter(|c| c.pushed).count();
    plan.dirty = local.dirty;
    plan.staged = local.staged;
    if plan.merges {
        return Err(Box::new(with_plan(
            refusal("merges", "There is a merge in these commits", "This window cannot change commits across a merge. Pick a commit after the merge."),
            &plan,
        )));
    }
    if plan.commits.is_empty() {
        return Ok((with_plan(success("No commits after this one"), &plan), plan));
    }
    if !local.unmerged.is_empty() {
        let out = refusal("unmerged", "Some files still have a conflict", "Fix them in the Changes panel first.").detail(&list_text(&local.unmerged));
        return Err(Box::new(with_plan(out, &plan).with("files", json!(local.unmerged))));
    }
    if !local.mixed.is_empty() {
        let out = refusal("mixedChanges", "Some files have staged and unstaged parts", "Commit or stash them first.").detail(&list_text(&local.mixed));
        return Err(Box::new(with_plan(out, &plan).with("files", json!(local.mixed))));
    }
    let danger = ignored_in_danger(root, &base, &head)?;
    if !danger.is_empty() {
        let out = refusal(
            "ignored",
            "These files are ignored by git and would be lost",
            "Git would write over them while it changes the commits. Move them out of the repo folder first, then press Load again.",
        )
        .detail(&list_text(&danger));
        return Err(Box::new(with_plan(out, &plan).with("files", json!(danger))));
    }
    let title = format!("{} to change", plural(plan.commits.len(), "commit", "commits"));
    Ok((with_plan(success(&title), &plan), plan))
}

pub fn rebase_plan(root: &Path, base: Option<&str>) -> Outcome {
    match load_plan(root, base) {
        Ok((out, _)) => out,
        Err(out) => *out,
    }
}

fn head_changed() -> Outcome {
    refusal("headChanged", "The branch changed since this window loaded", "Press Load again, then make your changes again.")
}

fn checked_plan(root: &Path, opts: &RebaseOpts) -> Result<Plan, Box<Outcome>> {
    let (_, plan) = load_plan(root, opts.base.as_deref())?;
    if opts.expect_head.as_ref().is_some_and(|h| !h.is_empty() && *h != plan.head) {
        return Err(Box::new(head_changed()));
    }
    let check = validate_steps(&opts.steps, Some(&plan.commits));
    if !check.ok {
        return Err(Box::new(refusal("invalid", check.problem.as_deref().unwrap_or(""), "").with("problems", json!(check.problems))));
    }
    Ok(plan)
}

fn conflict_files(text: &str) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for line in text.split('\n').skip(1) {
        if line.trim().is_empty() {
            break;
        }
        if !files.iter().any(|f| f == line) {
            files.push(line.to_string());
        }
    }
    files
}

fn step_ref(index: usize, commit: &RebaseCommit) -> StepRef {
    StepRef { index, sha: commit.sha.clone(), short: commit.short.clone(), subject: commit.subject.clone() }
}

fn preview_stop(title: &str, kind: &str, hint: &str, empty: &[StepRef], checked: usize) -> Outcome {
    let mut out = success(title).hint(hint);
    out.kind = kind.into();
    out.with("clean", false.into()).with("sameFiles", Value::Null).with("empty", json!(empty)).with("checked", checked.into())
}

fn empty_note(empty: &[StepRef]) -> String {
    let one = empty.len() == 1;
    let shorts = empty.iter().map(|e| e.short.as_str()).collect::<Vec<_>>().join(", ");
    format!(
        "{} will have no changes left, because {} already there. {}: {shorts}. Drop {} if you do not want {}.",
        plural(empty.len(), "commit", "commits"),
        if one { "its changes are" } else { "their changes are" },
        if one { "It stays as an empty commit" } else { "They stay as empty commits" },
        if one { "it" } else { "them" },
        if one { "it" } else { "them" },
    )
}

pub fn preview_rebase(root: &Path, opts: &RebaseOpts) -> Outcome {
    let plan = match checked_plan(root, opts) {
        Ok(plan) => plan,
        Err(out) => return *out,
    };
    let map = by_sha(&plan.commits);
    let mut tree = plan.base_tree.clone();
    let mut before = plan.base_tree.clone();
    let mut empty: Vec<StepRef> = Vec::new();
    let mut checked = 0;
    for (i, step) in opts.steps.iter().enumerate() {
        if step.action == "drop" {
            continue;
        }
        let Some(commit) = map.get(step.sha.as_str()) else { continue };
        let merge_base = format!("--merge-base={}", commit.parents.first().map(String::as_str).unwrap_or(ops::EMPTY_TREE));
        let r = run::git(
            root,
            &["-c", "core.quotepath=off", "merge-tree", "--write-tree", "--name-only", "--no-messages", &merge_base, &tree, &commit.sha],
        );
        if r.code == Some(1) {
            let files = conflict_files(&r.stdout);
            let detail = if files.is_empty() {
                String::new()
            } else {
                let more = if files.len() > 8 { format!(" and {} more", files.len() - 8) } else { String::new() };
                format!("Files: {}{more}", files.iter().take(8).cloned().collect::<Vec<_>>().join(", "))
            };
            let conflict = StepConflict { index: i, sha: commit.sha.clone(), short: commit.short.clone(), subject: commit.subject.clone(), files };
            return preview_stop(
                &format!("{} will have a conflict", commit.short),
                "conflict",
                "Move it to another place, drop it, or fix the conflict when git stops there.",
                &empty,
                checked,
            )
            .detail(&detail)
            .with("conflict", json!(conflict));
        }
        if !r.ok() {
            return failure(&r.said());
        }
        let next = r.stdout.split('\n').next().unwrap_or("").trim().to_string();
        if !SHA.is_match(&next) {
            let said = r.said();
            return failure(if said.is_empty() { "git merge-tree gave no result" } else { &said });
        }
        let joins = JOINS.contains(&step.action.as_str());
        if joins && next == before {
            return preview_stop(
                &format!("{} will stop the rebase", commit.short),
                "emptyJoin",
                "Joining it into the commit above leaves no changes at all, so git stops there. Drop both commits instead.",
                &empty,
                checked,
            )
            .with("conflict", Value::Null)
            .with("stop", json!(step_ref(i, commit)));
        }
        if !joins {
            before = tree.clone();
        }
        if next == tree && !commit.empty && !joins {
            empty.push(step_ref(i, commit));
        }
        tree = next;
        checked += 1;
    }
    let head_tree = run::read(root, &["rev-parse", &format!("{}^{{tree}}", plan.head)]).stdout.trim().to_string();
    let same_files = tree == head_tree;
    let mut notes = vec![if same_files { "The files at the end stay the same as now." } else { "The files at the end will be different from now." }.to_string()];
    if !empty.is_empty() {
        notes.push(empty_note(&empty));
    }
    success("No conflicts expected")
        .hint(&notes.join(" "))
        .with("clean", true.into())
        .with("conflict", Value::Null)
        .with("sameFiles", same_files.into())
        .with("empty", json!(empty))
        .with("checked", checked.into())
}

fn force_hint(count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    let lead = if count == 1 { "1 changed commit was".to_string() } else { format!("{count} changed commits were") };
    format!("{lead} pushed before. Push with force (safe, with lease) to update the remote.")
}

fn identity_refusal(mut out: Outcome) -> Outcome {
    if out.kind == "identity" {
        out.hint = "Then press Apply again.".into();
        let warnings: Vec<String> = out.detail.split('\n').filter(|w| !w.is_empty()).map(String::from).collect();
        return out.with("needsProfile", true.into()).with("warnings", json!(warnings));
    }
    out
}

fn sequence_editor(check: &Path, todo: &Path) -> (String, String) {
    let value = format!("/bin/sh {} {}", shell_quote(&check.to_string_lossy()), shell_quote(&todo.to_string_lossy()));
    ("GIT_SEQUENCE_EDITOR".into(), value)
}

fn stopped_part_way(kind: &str, title: &str, hint: &str, text: &str, r: &Out, state: &Option<State>, summary: &StepSummary) -> Outcome {
    Outcome {
        ok: false,
        kind: kind.into(),
        title: title.into(),
        hint: hint.into(),
        detail: describe(text).detail,
        stderr: r.stderr.clone(),
        ..Outcome::default()
    }
    .with("state", json!(state))
    .with("summary", json!(summary))
}

pub fn run_rebase(root: &Path, opts: &RebaseOpts) -> Outcome {
    let plan = match checked_plan(root, opts) {
        Ok(plan) => plan,
        Err(out) => return *out,
    };
    if is_unchanged(&opts.steps, &plan.commits) {
        return refusal("nothing", "Nothing to change", "Pick another action or move a commit first.");
    }
    let identity = match identity_gate(&opts.identity) {
        Ok(value) => value,
        Err(out) => return identity_refusal(*out),
    };
    let todo = match build_todo(&opts.steps, &plan.commits) {
        Ok(todo) => todo,
        Err(error) => return thrown(&error),
    };
    if todo.len() > MAX_TODO {
        return refusal("tooBig", "The new messages are too long", "Make the messages shorter, or change fewer commits at once.");
    }
    let todo_file = match ops::temp_text("nixa-git-rebase-todo.", &todo) {
        Ok(file) => file,
        Err(error) => return thrown(&error),
    };
    let check_file = match ops::temp_text("nixa-git-rebase-check.", &check_script(&plan)) {
        Ok(file) => file,
        Err(error) => return thrown(&error),
    };
    let mut env = opts.job.env.clone();
    env.push(sequence_editor(check_file.path(), todo_file.path()));
    let job = Job { cancel: opts.job.cancel.clone(), timeout: opts.job.timeout.or(Some(Duration::from_secs(REBASE_SECS))), env };
    let mut args = vec!["rebase", "-i"];
    args.extend(REBASE_FLAGS);
    args.push(&plan.base);
    let r = run::job(root, &args, &job);
    drop(todo_file);
    drop(check_file);
    let summary = summarize_steps(&opts.steps, &plan.commits);
    let state = ops::read_state(root).ok();
    let head = ops::head_sha(root);
    let moved = head.as_deref() != Some(plan.head.as_str());
    let rebasing = state.as_ref().is_some_and(|s| s.op.as_deref() == Some("rebase"));
    if r.cancelled || r.timed_out {
        let stop = if r.cancelled {
            refusal("cancelled", "Stopped", "").with("cancelled", true.into())
        } else {
            described("timed out waiting").with("timedOut", true.into())
        };
        let hint = if rebasing {
            "The rebase stopped part way. Continue or cancel it in the Changes panel."
        } else if moved {
            "The commits may have changed. Check the history."
        } else {
            "Nothing was changed."
        };
        return stop.hint(hint).with("state", json!(state));
    }
    let text = r.said();
    if rebasing {
        if state.as_ref().is_some_and(|s| !s.conflicts.is_empty()) {
            return stopped_part_way("conflict", "Stopped on a conflict", "Fix it in the Changes panel, then press Continue.", &text, &r, &state, &summary);
        }
        if !r.ok() && TODO_BROKEN.is_match(&text) && run::git(root, &["rebase", "--abort"]).ok() {
            return refusal("invalid", "Git did not take the step list", "Nothing was changed.").detail(&describe(&text).detail).stderr(&r.stderr);
        }
        return stopped_part_way("stopped", "The rebase stopped part way", "Continue or cancel it in the Changes panel.", &text, &r, &state, &summary);
    }
    if !r.ok() && text.contains(CHANGED_MARK) {
        let kept = if remote::autostash_clash(&text) { " Your own changes are kept in the stash." } else { "" };
        return refusal("headChanged", "The branch changed while starting", &format!("Nothing was changed.{kept} Load again."))
            .stderr(&r.stderr)
            .with("state", json!(state));
    }
    if !r.ok() {
        return failure(&text).with("state", json!(state));
    }
    let short = head.as_deref().map(short_sha);
    let force = summary.pushed_changed;
    let done = |title: &str, hint: String| {
        success(title)
            .hint(&hint)
            .stderr(&r.stderr)
            .with("head", json!(short))
            .with("summary", json!(summary))
            .with("pushed", force.into())
            .with("needsForce", (force > 0).into())
            .with("identity", identity.clone())
    };
    if remote::autostash_clash(&text) {
        let hint = format!(
            "Your own changes did not fit with the new commits. They are kept in the stash. Open the stash window to get them back. {}",
            force_hint(force)
        );
        return done("Commits changed, but your changes clashed", hint.trim().to_string())
            .with("warning", true.into())
            .with("stashConflict", true.into());
    }
    done("Commits changed", force_hint(force))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrepo::{self, Repo};

    fn step(sha: &str, action: &str) -> RebaseStep {
        RebaseStep { sha: sha.into(), action: action.into(), ..RebaseStep::default() }
    }

    fn msg(summary: &str, body: &str) -> Message {
        Message { summary: summary.into(), body: body.into() }
    }

    fn commit(sha: &str, subject: &str, body: &str) -> RebaseCommit {
        RebaseCommit { sha: sha.into(), short: sha[..7].into(), subject: subject.into(), body: body.into(), ..RebaseCommit::default() }
    }

    fn sha(c: char) -> String {
        c.to_string().repeat(40)
    }

    fn opts(base: &str, steps: Vec<RebaseStep>) -> RebaseOpts {
        RebaseOpts { base: Some(base.into()), steps, ..RebaseOpts::default() }
    }

    fn subjects(repo: &Repo, range: &str) -> Vec<String> {
        repo.git(&["log", "--reverse", "--format=%s", range]).lines().map(String::from).collect()
    }

    #[test]
    fn validates_steps() {
        let (a, b) = (sha('a'), sha('b'));
        assert_eq!(validate_steps(&[], None).problem.as_deref(), Some("There are no commits to change."));
        assert!(validate_steps(&[step(&a, "pick"), step(&b, "squash")], None).ok);
        let bad = validate_steps(&[step("xyz", "pick"), step(&a, "jump"), step(&a, "reword")], None);
        let texts: Vec<&str> = bad.problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["A commit id is missing or wrong.", "\"jump\" is not a known action.", "Commit aaaaaaa is in the list twice.", "Write a summary for aaaaaaa."]
        );
        assert_eq!(bad.problems[1].sha.as_deref(), Some(a.as_str()));
        let joined = validate_steps(&[step(&a, "drop"), step(&b, "fixup")], None);
        assert_eq!(joined.problems[0].index, 1);
        assert!(joined.problems[0].text.starts_with("The first kept commit cannot be joined."));
        let dropped = validate_steps(&[step(&a, "drop")], None);
        assert_eq!(dropped.problem.as_deref(), Some("Keep at least one commit."));
        assert_eq!(dropped.problems[0].sha, None);
        let commits = vec![commit(&a, "A", ""), commit(&b, "B", "")];
        let stale = validate_steps(&[step(&a, "pick")], Some(&commits));
        assert_eq!(stale.problem.as_deref(), Some("The list does not match the branch any more. Load it again."));
        let mut squash = step(&b, "squash");
        squash.message = Some(msg(" ", ""));
        assert_eq!(validate_steps(&[step(&a, "pick"), squash], None).problem.as_deref(), Some("Write a summary for bbbbbbb."));
    }

    #[test]
    fn builds_the_todo() {
        let (a, b, c, d) = (sha('a'), sha('b'), sha('c'), sha('d'));
        let commits = vec![commit(&a, "A one", "a body"), commit(&b, "B\ttwo", ""), commit(&c, "C", ""), commit(&d, "D", "")];
        let mut first = step(&a, "reword");
        first.message = Some(msg("New A", "it's here"));
        let steps = vec![first, step(&b, "squash"), step(&c, "fixup"), step(&d, "drop")];
        let todo = build_todo(&steps, &commits).unwrap();
        let lines: Vec<&str> = todo.lines().collect();
        assert_eq!(lines[0], format!("pick {a} A one"));
        assert_eq!(lines[1], format!("exec printf '%s\\n' 'New A' '' 'it'\\''s here' | {AMEND_COMMAND}"));
        assert_eq!(lines[2], format!("squash {b} B two"));
        assert_eq!(lines[3], format!("exec printf '%s\\n' 'New A' '' 'it'\\''s here' '' 'B\ttwo' | {AMEND_COMMAND}"));
        assert_eq!(lines[4], format!("fixup {c} C"));
        assert_eq!(lines[5], format!("drop {d} D"));
        assert!(todo.ends_with('\n'));
        let bare = build_todo(&[step(&a, "pick"), step(&b, "squash")], &[]).unwrap();
        assert_eq!(bare, format!("pick {a}\nsquash {b}\n"));
        assert!(build_todo(&[step(&a, "fixup")], &[]).is_err());
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(amend_line(&msg("x\u{1}y", "")), format!("exec printf '%s\\n' 'xy' | {AMEND_COMMAND}"));
    }

    #[test]
    fn step_helpers() {
        let (a, b, c) = (sha('a'), sha('b'), sha('c'));
        let commits = vec![commit(&a, "A", "body a"), commit(&b, "B", ""), commit(&c, "C", "")];
        let steps = start_steps(&commits);
        assert!(is_unchanged(&steps, &commits));
        assert_eq!(first_change(&steps, &commits), None);
        let moved = move_step(&steps, 2, -2);
        assert_eq!(moved[0].sha, c);
        assert_eq!(move_step(&steps, 0, -1), steps);
        assert_eq!(first_change(&moved, &commits), Some(0));
        let mut same = steps.clone();
        same[1] = RebaseStep { action: "reword".into(), message: Some(msg("B", "")), ..same[1].clone() };
        assert!(is_unchanged(&same, &commits));
        let joins = vec![step(&a, "pick"), step(&b, "squash"), step(&c, "squash")];
        assert_eq!(join_target(&joins, 2), Some(0));
        assert_eq!(join_target(&joins, 0), None);
        assert_eq!(joined_message(&joins, &commits, 2), msg("A", "body a\n\nB\n\nC"));
        let mut told = joins.clone();
        told[1].message = Some(msg("Mid", ""));
        assert_eq!(joined_message(&told, &commits, 2), msg("Mid", "C"));
        let mut summary_steps = vec![step(&c, "pick"), step(&a, "reword"), step(&b, "fixup")];
        summary_steps[1].message = Some(msg("New", ""));
        let mut pushed = commits.clone();
        pushed[0].pushed = true;
        pushed[1].pushed = true;
        let s = summarize_steps(&summary_steps, &pushed);
        assert_eq!(s.text, "2 kept, 0 dropped, 1 squashed, 1 new message. New order.");
        assert_eq!(s.first_change, 0);
        assert_eq!(s.pushed_changed, 2);
        assert!(s.moved);
        let none = summarize_steps(&steps, &commits);
        assert_eq!(none.text, "3 kept, 0 dropped, 0 squashed.");
        assert_eq!(none.first_change, -1);
    }

    #[test]
    fn parses_status_log_and_ignored() {
        let entries = parse_status_z("M  a.txt\0MM b.txt\0R  new.txt\0old.txt\0UU c.txt\0 M d.txt\0");
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[2].path, "new.txt");
        let local = local_changes(&entries);
        assert!(local.dirty && local.staged);
        assert_eq!(local.mixed, vec!["b.txt"]);
        assert_eq!(local.unmerged, vec!["c.txt"]);
        let clean = local_changes(&parse_status_z(" M d.txt\0"));
        assert!(clean.dirty && !clean.staged);
        let a = sha('a');
        let log = format!("{a}\0aaaaaaa\0{} {}\0tree\0Ann\0ann@x\012\0Subject\0Body\n\n\x1e\n{}\0b\0\0t\0B\0b@x\0x\0S\0\x1e\n", sha('b'), sha('c'), "zz");
        let commits = parse_log(&log);
        assert_eq!(commits.len(), 1);
        assert!(commits[0].is_merge);
        assert_eq!(commits[0].at, 12);
        assert_eq!(commits[0].body, "Body");
        let risk = ignored_at_risk(
            &["build/".into(), "gen/".into(), "x.log".into(), "keep/".into(), "src/out".into()],
            &["build/a.o".into(), "x.log".into(), "src/out/y".into(), "gen".into()],
        );
        assert_eq!(risk.risky, vec!["gen/", "x.log", "src/out"]);
        assert_eq!(risk.deeper, vec!["build/"]);
        assert_eq!(list_text(&(0..12).map(|i| i.to_string()).collect::<Vec<_>>()), "0, 1, 2, 3, 4, 5, 6, 7, 8, 9 and 2 more");
    }

    #[test]
    fn rewords_squashes_fixups_and_drops_for_real() {
        let repo = testrepo::new();
        let base = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
        let c1 = repo.commit_file("b.txt", "b\n", "add b");
        let c2 = repo.commit_file("c.txt", "c\n", "add c");
        let c3 = repo.commit_file("c.txt", "c\nc2\n", "fix c");
        let c4 = repo.commit_file("d.txt", "d\n", "add d");
        let c5 = repo.commit_file("e.txt", "e\n", "add e");
        repo.write("a.txt", "one\ntwo\nthree\nlocal\n");
        let plan = rebase_plan(repo.root(), Some(&base));
        assert!(plan.ok, "{plan:?}");
        assert_eq!(plan.title, "5 commits to change");
        let data = plan_of(&plan).unwrap();
        assert_eq!(data.count, 5);
        assert_eq!(data.head, c5);
        assert_eq!(data.head_ref.as_deref(), Some("refs/heads/main"));
        assert_eq!(data.branch.as_deref(), Some("main"));
        assert!(data.dirty && !data.staged);
        assert_eq!(data.commits.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(), vec![c1.clone(), c2.clone(), c3.clone(), c4.clone(), c5.clone()]);
        let mut first = step(&c1, "reword");
        first.message = Some(msg("Add the b file", "Body line"));
        let steps = vec![first, step(&c2, "squash"), step(&c3, "fixup"), step(&c4, "drop"), step(&c5, "pick")];
        let preview = preview_rebase(repo.root(), &RebaseOpts { expect_head: Some(c5.clone()), ..opts(&base, steps.clone()) });
        assert!(preview.ok, "{preview:?}");
        assert_eq!(preview.title, "No conflicts expected");
        assert!(preview.flag("clean"));
        assert_eq!(preview.get("sameFiles"), Some(&Value::Bool(false)));
        let out = run_rebase(repo.root(), &RebaseOpts { expect_head: Some(c5.clone()), ..opts(&base, steps) });
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Commits changed");
        assert!(!out.flag("needsForce"));
        assert_eq!(out.get("summary").unwrap()["text"], "2 kept, 1 dropped, 2 squashed, 1 new message.");
        let range = format!("{base}..HEAD");
        assert_eq!(subjects(&repo, &range), vec!["Add the b file", "add e"]);
        let full = repo.git(&["log", "-1", "--format=%B", "HEAD~1"]);
        assert_eq!(full.trim_end(), "Add the b file\n\nBody line\n\nadd c");
        assert_eq!(repo.read("c.txt"), "c\nc2\n");
        assert!(!repo.path("d.txt").exists());
        assert_eq!(repo.read("e.txt"), "e\n");
        assert_eq!(repo.read("a.txt"), "one\ntwo\nthree\nlocal\n");
        assert_eq!(ops::read_state(repo.root()).unwrap().op, None);
        let now = plan_of(&rebase_plan(repo.root(), Some(&base))).unwrap();
        let mut told = step(&now.commits[1].sha, "squash");
        told.message = Some(msg("All in one", ""));
        let again = run_rebase(repo.root(), &opts(&base, vec![step(&now.commits[0].sha, "pick"), told]));
        assert!(again.ok, "{again:?}");
        assert_eq!(subjects(&repo, &range), vec!["All in one"]);
        assert_eq!(repo.git(&["log", "-1", "--format=%B"]).trim_end(), "All in one");
        assert_eq!(repo.read("e.txt"), "e\n");
    }

    #[test]
    fn preview_finds_conflicts_empty_joins_and_empty_commits() {
        let repo = testrepo::new();
        let base = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
        let c1 = repo.commit_file("a.txt", "one\nX\nthree\n", "to X");
        let c2 = repo.commit_file("a.txt", "one\nY\nthree\n", "to Y");
        let swapped = preview_rebase(repo.root(), &opts(&base, vec![step(&c2, "pick"), step(&c1, "pick")]));
        assert!(swapped.ok);
        assert_eq!(swapped.kind, "conflict");
        assert!(!swapped.flag("clean"));
        assert_eq!(swapped.title, format!("{} will have a conflict", &c2[..7]));
        assert_eq!(swapped.detail, "Files: a.txt");
        assert_eq!(swapped.get("conflict").unwrap()["index"], 0);
        assert_eq!(swapped.get("conflict").unwrap()["files"][0], "a.txt");
        let same = preview_rebase(repo.root(), &opts(&base, vec![step(&c1, "pick"), step(&c2, "pick")]));
        assert_eq!(same.get("sameFiles"), Some(&Value::Bool(true)));
        assert_eq!(same.hint, "The files at the end stay the same as now.");
        let other = testrepo::new();
        let root = other.git(&["rev-parse", "HEAD"]).trim().to_string();
        let add = other.commit_file("x.txt", "x\n", "add x");
        other.git(&["rm", "-q", "x.txt"]);
        other.git(&["commit", "-q", "-m", "remove x"]);
        let remove = other.git(&["rev-parse", "HEAD"]).trim().to_string();
        let again = other.commit_file("x.txt", "x\n", "add x again");
        let join = preview_rebase(other.root(), &opts(&root, vec![step(&add, "pick"), step(&remove, "fixup"), step(&again, "drop")]));
        assert_eq!(join.kind, "emptyJoin", "{join:?}");
        assert_eq!(join.get("stop").unwrap()["index"], 1);
        let empty = preview_rebase(other.root(), &opts(&root, vec![step(&add, "pick"), step(&remove, "drop"), step(&again, "pick")]));
        assert!(empty.flag("clean"), "{empty:?}");
        assert_eq!(empty.get("empty").unwrap()[0]["index"], 2);
        assert!(empty.hint.contains("1 commit will have no changes left, because its changes are already there. It stays as an empty commit"));
    }

    #[test]
    fn plan_refusals() {
        let repo = testrepo::new();
        let base = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(rebase_plan(repo.root(), None).kind, "noUpstream");
        assert_eq!(rebase_plan(repo.root(), Some("-x")).kind, "invalid");
        assert_eq!(rebase_plan(repo.root(), Some("a b")).kind, "invalid");
        assert_eq!(rebase_plan(repo.root(), Some("deadbeef")).kind, "unknownBase");
        let none = rebase_plan(repo.root(), Some(&base));
        assert!(none.ok);
        assert_eq!(none.title, "No commits after this one");
        assert_eq!(rebase_plan(testrepo::empty().root(), None).kind, "noCommits");
        repo.git(&["checkout", "-q", "-b", "side"]);
        let side = repo.commit_file("a.txt", "side\n", "side change");
        repo.git(&["checkout", "-q", "main"]);
        assert_eq!(rebase_plan(repo.root(), Some(&side)).kind, "notOnBranch");
        let c1 = repo.commit_file("a.txt", "main\n", "main change");
        repo.write("a.txt", "staged\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "staged and more\n");
        let mixed = rebase_plan(repo.root(), Some(&base));
        assert_eq!(mixed.kind, "mixedChanges");
        assert_eq!(mixed.detail, "a.txt");
        assert!(plan_of(&mixed).unwrap().staged);
        repo.git(&["checkout", "-q", "--", "."]);
        repo.git(&["reset", "-q", "--hard"]);
        let merge = run::git(repo.root(), &["merge", "side"]);
        assert!(!merge.ok());
        let busy = rebase_plan(repo.root(), Some(&base));
        assert_eq!(busy.kind, "busy");
        assert_eq!(busy.title, "A merge is in progress");
        repo.git(&["merge", "--abort"]);
        let steps = vec![step(&c1, "pick")];
        assert_eq!(run_rebase(repo.root(), &opts(&base, steps.clone())).kind, "nothing");
        let wrong = run_rebase(repo.root(), &RebaseOpts { expect_head: Some(base.clone()), ..opts(&base, vec![step(&c1, "drop")]) });
        assert_eq!(wrong.kind, "headChanged");
        assert_eq!(preview_rebase(repo.root(), &opts(&base, vec![])).title, "There are no commits to change.");
        let mut reword = step(&c1, "reword");
        reword.message = Some(msg("New", ""));
        let ask = run_rebase(repo.root(), &RebaseOpts { identity: Identity::Ask { warnings: vec!["w".into()] }, ..opts(&base, vec![reword]) });
        assert_eq!(ask.kind, "identity");
        assert_eq!(ask.hint, "Then press Apply again.");
        assert!(ask.flag("needsProfile"));
        assert_eq!(repo.git(&["rev-parse", "HEAD"]).trim(), c1);
        repo.git(&["merge", "-q", "--no-ff", "-m", "merge side", "-X", "ours", "side"]);
        let merges = rebase_plan(repo.root(), Some(&base));
        assert_eq!(merges.kind, "merges");
        assert!(plan_of(&merges).unwrap().merges);
    }

    #[test]
    fn refuses_when_ignored_files_would_be_lost() {
        let repo = testrepo::new();
        let base = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
        repo.commit_file("gen/x.txt", "tracked\n", "add gen");
        repo.git(&["rm", "-q", "gen/x.txt"]);
        repo.git(&["commit", "-q", "-m", "remove gen"]);
        repo.commit_file("b.txt", "b\n", "add b");
        std::fs::write(repo.path(".git/info/exclude"), "gen/\n").unwrap();
        repo.write("gen/x.txt", "mine\n");
        let out = rebase_plan(repo.root(), Some(&base));
        assert_eq!(out.kind, "ignored", "{out:?}");
        assert_eq!(out.detail, "gen/x.txt");
        assert_eq!(out.get("files").unwrap()[0], "gen/x.txt");
    }

    #[test]
    fn upstream_plan_and_pushed_commits() {
        let (repo, _remote) = testrepo::with_remote();
        let base = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
        let c1 = repo.commit_file("b.txt", "b\n", "add b");
        let c2 = repo.commit_file("c.txt", "c\n", "add c");
        let plan = plan_of(&rebase_plan(repo.root(), None)).unwrap();
        assert_eq!(plan.base, base);
        assert_eq!(plan.onto, "origin/main");
        assert_eq!(plan.upstream.as_deref(), Some("origin/main"));
        assert_eq!(plan.pushed, 0);
        repo.git(&["push", "-q"]);
        let pushed = plan_of(&rebase_plan(repo.root(), Some(&base))).unwrap();
        assert_eq!(pushed.pushed, 2);
        let mut reword = step(&c2, "reword");
        reword.message = Some(msg("add the c file", ""));
        let out = run_rebase(repo.root(), &opts(&base, vec![step(&c1, "pick"), reword]));
        assert!(out.ok, "{out:?}");
        assert!(out.flag("needsForce"));
        assert_eq!(out.get("pushed"), Some(&Value::from(1)));
        assert_eq!(out.hint, "1 changed commit was pushed before. Push with force (safe, with lease) to update the remote.");
        assert_eq!(repo.git(&["rev-parse", "HEAD~1"]).trim(), c1);
    }

    #[test]
    fn check_script_stops_a_changed_branch() {
        let repo = testrepo::new();
        let base = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
        let c1 = repo.commit_file("b.txt", "b\n", "add b");
        let mut plan = plan_of(&rebase_plan(repo.root(), Some(&base))).unwrap();
        plan.head = sha('0');
        let todo = ops::temp_text("nixa-git-rebase-todo.", &format!("drop {c1}\n")).unwrap();
        let check = ops::temp_text("nixa-git-rebase-check.", &check_script(&plan)).unwrap();
        let env = vec![sequence_editor(check.path(), todo.path())];
        let mut args = vec!["rebase", "-i"];
        args.extend(REBASE_FLAGS);
        args.push(&base);
        let r = run::job(repo.root(), &args, &Job { env, ..Job::default() });
        assert!(!r.ok());
        assert!(r.said().contains(CHANGED_MARK), "{r:?}");
        assert_eq!(repo.git(&["rev-parse", "HEAD"]).trim(), c1);
        assert_eq!(ops::read_state(repo.root()).unwrap().op, None);
    }
}
