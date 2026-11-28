use super::handoff::{
    CUSTOM_ROLE, HANDOFF_PREFIX, Person, REPORT_LIMIT, after_step, custom_person, escalation, fix_after, handoff_start_key, is_review_role, member_for,
};
use crate::util::clip;
use anyhow::Result;
use nixa_store::Connection;
use nixa_store::agents::{agent_by_terminal, get_agent, list_agents};
use nixa_store::review::{
    HandoffMove, HandoffState, PICKUP_ROLE, ReviewSeat, custom_id_of, custom_ref, findings_of, handoff_key, handoff_state, in_chain, pick_reviewer,
    pickup_move, review_files, set_handoff_state,
};
use nixa_store::tasks::{
    Actor, AuthorKind, BlockedOn, CommentInput, CommentKind, Task, TaskComment, TaskFilter, TaskPatch, TaskStatus, add_comment, get_task,
    is_run_holder, is_starting, list_tasks, must_task, remove_task_meta, set_status, set_task_meta, set_uncommitted, task_comments, update_task,
};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

pub const LAST_REPORT: &str = "Last report from";
pub const CHECK_FAIL_LIMIT: u32 = 3;
const SHOWN_FILES: usize = 8;
const SHARING_MAX: usize = 8;

static CHECKED_BY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bchecked by:").expect("pattern"));
static FAILING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i:\b[1-9]\d*\s+(?:tests?\s+)?failed\b|\bfail(?:ed|ures?)\s*:\s*[1-9])|test result: FAILED").expect("pattern"));

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CheckLine {
    pub failed: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Facts {
    pub changed: Vec<String>,
    pub unread: bool,
    pub checks: Option<CheckLine>,
    pub no_base: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Team {
    pub people: Vec<Person>,
    pub seats: Vec<ReviewSeat>,
    pub worker: Option<String>,
    pub deep_first: bool,
    pub folders: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    Send { terminal: String, text: String },
    Stop { terminal: String },
    Info { text: String },
    Speak { text: String },
    AskDeep { task: i64, question: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Back,
    Moved,
    User,
    Done,
    Held,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Settled {
    pub outcome: Outcome,
    pub effects: Vec<Effect>,
}

pub fn nixa() -> Actor {
    Actor::new(AuthorKind::System, "Nixa")
}

fn note(conn: &Connection, id: i64, kind: CommentKind, text: &str) -> Result<()> {
    add_comment(conn, id, &CommentInput::new(kind, text), &nixa())?;
    Ok(())
}

pub fn place(task: &Task) -> String {
    if !task.space_name.trim().is_empty() {
        task.space_name.clone()
    } else if !task.space_id.trim().is_empty() {
        task.space_id.clone()
    } else {
        "no space".to_string()
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn file_list(files: &[String], max: usize) -> String {
    if files.len() <= max {
        files.join(", ")
    } else {
        format!("{} and {} more", files[..max].join(", "), files.len() - max)
    }
}

pub fn same_file(folder: &str, item: &str, file: &str) -> bool {
    let strip = |text: &str| -> String {
        let text = text.trim();
        let rel = text.strip_prefix(folder.trim_end_matches('/')).map(|r| r.trim_start_matches('/')).unwrap_or(text);
        rel.trim_start_matches("./").to_string()
    };
    let a = strip(item);
    !a.is_empty() && a == strip(file)
}

pub fn step_report(task: &Task, comments: &[TaskComment]) -> String {
    comments
        .iter()
        .rev()
        .find(|c| c.kind == CommentKind::Result && c.at >= task.claimed_at && (c.author_kind != AuthorKind::System || c.text.starts_with(LAST_REPORT)))
        .map(|c| c.text.clone())
        .unwrap_or_default()
}

fn saved_custom(conn: &Connection, task: &Task, team: &Team) -> Option<String> {
    if task.assigned_kind != "member" || task.assigned.is_empty() || member_for(&team.people, &task.assigned).is_some() {
        return None;
    }
    let id = task.assigned.trim().trim_start_matches("custom:").trim_start_matches("member:").to_string();
    get_agent(conn, &id).ok().flatten().map(|a| a.id)
}

fn custom_of(task: &Task, team: &Team) -> Option<String> {
    if task.assigned_kind == "agent"
        && let Some(id) = custom_id_of(&task.assigned)
    {
        return Some(id);
    }
    team.worker.clone()
}

pub fn member_of(conn: &Connection, task: &Task, team: &Team, state: &HandoffState) -> Option<Person> {
    if task.assigned_kind == "member"
        && !task.assigned.is_empty()
        && let Some(person) = member_for(&team.people, &task.assigned)
    {
        return Some(person.clone());
    }
    let id = saved_custom(conn, task, team).or_else(|| custom_of(task, team))?;
    let reviewer = state.reviewer == custom_ref(&id);
    Some(match get_agent(conn, &id).ok().flatten() {
        Some(agent) => custom_person(&agent.id, &agent.title, &agent.role, reviewer),
        None => custom_person(&id, &format!("custom agent {id}"), "", reviewer),
    })
}

fn target_of(conn: &Connection, task: &Task, to: &str, team: &Team) -> Option<(Person, String, String)> {
    if let Some(id) = custom_id_of(to) {
        let agent = get_agent(conn, &id).ok().flatten().filter(|a| !a.off)?;
        return Some((custom_person(&agent.id, &agent.title, &agent.role, false), "agent".into(), custom_ref(&agent.id)));
    }
    if let Some(member) = member_for(&team.people, to) {
        return Some((member.clone(), "member".into(), member.id.clone()));
    }
    let spaces = crate::spaces::from_store(conn);
    let agent = list_agents(conn).ok()?.into_iter().find(|a| !a.off && a.job.eq_ignore_ascii_case(to) && same_space(a, task, &spaces))?;
    Some((custom_person(&agent.id, &agent.title, &agent.role, false), "agent".into(), custom_ref(&agent.id)))
}

fn same_space(agent: &nixa_store::agents::CustomAgent, task: &Task, spaces: &[crate::spaces::Space]) -> bool {
    let want = task.space_id.trim();
    if want.is_empty() {
        return false;
    }
    let own = crate::custom_agents::own_space(agent, spaces).map(|s| s.id.as_str()).unwrap_or(agent.space.as_str());
    own == want
}

fn job_member(conn: &Connection, member: Person) -> Person {
    if member.role != CUSTOM_ROLE {
        return member;
    }
    match custom_id_of(&member.id).and_then(|id| get_agent(conn, &id).ok().flatten()) {
        Some(agent) if !agent.job.trim().is_empty() => Person { role: agent.job.trim().to_lowercase(), ..member },
        _ => member,
    }
}

fn holds_other(conn: &Connection, id: i64, terminal: &str) -> Result<bool> {
    let filter = TaskFilter { status: vec![TaskStatus::Todo, TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review], ..TaskFilter::default() };
    Ok(list_tasks(conn, &filter)?.iter().any(|o| o.id != id && o.terminal_id == terminal))
}

fn blocked_on_user(task: &Task, comments: &[TaskComment]) -> Vec<Effect> {
    let question = comments.iter().rev().find(|c| c.kind == CommentKind::Question);
    let line = match question {
        Some(q) => format!("Task #{} in {} waits for you: {}", task.id, place(task), clip(&q.text, 160)),
        None => format!("Task #{} in {} waits for you.", task.id, place(task)),
    };
    vec![Effect::Info { text: line.clone() }, Effect::Speak { text: line }]
}

fn kept_files(task: &Task, files: &[String]) -> Vec<String> {
    let mut kept = task.uncommitted.clone();
    for f in files {
        if !kept.contains(f) {
            kept.push(f.clone());
        }
    }
    kept
}

#[derive(Clone, Copy, Debug)]
pub struct Moving<'a> {
    pub member: Option<&'a Person>,
    pub summary: &'a str,
    pub files: &'a [String],
    pub stop_old: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Work<'a> {
    pub changed: &'a [String],
    pub mine: &'a [String],
    pub shared: bool,
}

pub fn move_task(conn: &Connection, task: &Task, step: HandoffMove, moving: Moving, team: &Team) -> Result<Settled> {
    let Moving { member, summary, files, stop_old } = moving;
    let kept = kept_files(task, files);
    match step {
        HandoffMove::Done => Ok(Settled { outcome: Outcome::Done, effects: Vec::new() }),
        HandoffMove::User { why } if team.deep_first => {
            set_status(conn, task.id, TaskStatus::Review, None, false)?;
            if !kept.is_empty() {
                set_uncommitted(conn, task.id, &kept)?;
            }
            let text = format!("{summary} {why}").trim().to_string();
            note(conn, task.id, CommentKind::Note, &format!("{text} Nixa asked for a decision instead of waiting for the user."))?;
            crate::util::log("review_flow", &format!("handoff decided by Nixa or the deep brain, task {}", task.id));
            let question = format!("Task #{} in {}: {text} Check the work, then pick: done, send one more fix to an agent, or give it to another agent.", task.id, place(task));
            Ok(Settled { outcome: Outcome::Held, effects: vec![Effect::AskDeep { task: task.id, question }] })
        }
        HandoffMove::User { why } => {
            let held = set_status(conn, task.id, TaskStatus::Blocked, Some(BlockedOn::User), true)?;
            if !kept.is_empty() {
                set_uncommitted(conn, task.id, &kept)?;
            }
            note(conn, task.id, CommentKind::Question, format!("{summary} {why} Waiting for the user.").trim())?;
            crate::util::log("review_flow", &format!("handoff to the user, task {}", task.id));
            Ok(Settled { outcome: Outcome::User, effects: blocked_on_user(&held, &task_comments(conn, task.id)?) })
        }
        HandoffMove::Pass { to, why, state } => {
            let Some((next, kind, assigned)) = target_of(conn, task, &to, team) else {
                let gone = match custom_id_of(&to) {
                    Some(id) => format!("saved custom agent \"{id}\""),
                    None => format!("active team member \"{to}\""),
                };
                let step = HandoffMove::User { why: format!("{why} Nixa has no {gone} to pass it to.") };
                return move_task(conn, task, step, moving, team);
            };
            let from = member.map(|m| m.name.clone()).filter(|n| !n.is_empty()).or_else(|| Some(task.claimed_by.clone()).filter(|n| !n.is_empty())).unwrap_or_else(|| "the last step".into());
            let comments = task_comments(conn, task.id)?;
            let report = step_report(task, &comments);
            let old = task.terminal_id.clone();
            set_status(conn, task.id, TaskStatus::Todo, None, false)?;
            update_task(conn, task.id, &TaskPatch { assigned_kind: Some(kind), assigned: Some(assigned.clone()), then_member: Some(String::new()), ..TaskPatch::default() })?;
            if !kept.is_empty() {
                set_uncommitted(conn, task.id, &kept)?;
            }
            set_handoff_state(conn, task.id, &state)?;
            set_task_meta(conn, &handoff_start_key(task.id), "1")?;
            let report = if report.is_empty() { String::new() } else { format!("\nReport from {from}: \"\"\"{}\"\"\"", clip(&report, REPORT_LIMIT)) };
            note(conn, task.id, CommentKind::Handoff, &format!("{HANDOFF_PREFIX}{from} to {}: {summary} {why}{report}", next.name))?;
            let mut effects = Vec::new();
            if stop_old && !old.is_empty() && !is_starting(&old) && !is_run_holder(&old) && agent_by_terminal(conn, &old).ok().flatten().is_none() && !holds_other(conn, task.id, &old)? {
                effects.push(Effect::Stop { terminal: old });
            }
            effects.push(Effect::Info { text: format!("Task #{} in {}: {why} Nixa passed it to {}.", task.id, place(task), next.name) });
            crate::util::log("review_flow", &format!("handoff task {} to {assigned}, step {}", task.id, state.steps));
            Ok(Settled { outcome: Outcome::Moved, effects })
        }
    }
}

pub fn sharing(conn: &Connection, task: &Task, folder: &str, team: &Team) -> Result<Vec<i64>> {
    let since = if task.claimed_at > 0 { task.claimed_at } else { task.created_at };
    let filter = TaskFilter {
        status: vec![TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review, TaskStatus::Done, TaskStatus::Cancelled],
        old: true,
        ..TaskFilter::default()
    };
    let mut ids: Vec<i64> = list_tasks(conn, &filter)?
        .into_iter()
        .filter(|o| o.id != task.id && !o.terminal_id.is_empty() && o.terminal_id != task.terminal_id)
        .filter(|o| o.closed_at == 0 || o.closed_at >= since)
        .filter(|o| team.folders.get(&o.space_id).filter(|f| !f.is_empty()).unwrap_or(&o.repo) == folder)
        .map(|o| o.id)
        .collect();
    ids.sort();
    ids.truncate(SHARING_MAX);
    Ok(ids)
}

pub fn pickup(task: &Task, comments: &[TaskComment], member: Option<&Person>, state: &HandoffState, work: Work, team: &Team) -> Option<HandoffMove> {
    let Work { changed, mine, shared } = work;
    if task.space_id.is_empty() || !state.reviewer.is_empty() {
        return None;
    }
    if task.terminal_id.is_empty() || is_starting(&task.terminal_id) || is_run_holder(&task.terminal_id) {
        return None;
    }
    if member.is_some_and(|m| is_review_role(&m.role)) || changed.is_empty() {
        return None;
    }
    if !team.deep_first {
        crate::util::log("review_flow", &format!("review pickup skipped, task {}: Nixa decides, not the deep brain", task.id));
        return None;
    }
    let said: Vec<&str> = comments
        .iter()
        .filter(|c| c.at >= task.claimed_at && (c.author_kind == AuthorKind::Agent || (c.author_kind == AuthorKind::System && c.text.starts_with(LAST_REPORT))))
        .map(|c| c.text.as_str())
        .collect();
    let said = said.join("\n");
    if CHECKED_BY.is_match(&said) {
        crate::util::log("review_flow", &format!("review pickup skipped, task {}: checked by non-Claude helpers", task.id));
        return None;
    }
    let strict = shared || task.base.is_empty();
    let files = review_files(changed, mine, &said, strict);
    if files.is_empty() {
        return None;
    }
    let worker = team.worker.clone().unwrap_or_default();
    let reviewer = pick_reviewer(&team.seats, &task.space_id, &task.terminal_id, &worker, &[])?;
    let back = member.map(|m| m.id.clone()).or_else(|| team.worker.as_ref().map(|w| custom_ref(w))).unwrap_or_default();
    let found = pickup_move(reviewer, &files, state, &back);
    if found.is_some() {
        crate::util::log("review_flow", &format!("review pickup, task {} by {}, {} files", task.id, reviewer.id, files.len()));
    }
    found
}

fn checks_text(facts: &Facts) -> String {
    facts.checks.as_ref().map(|c| format!(" Checks: {}.", clip(&c.text, 400))).unwrap_or_default()
}

fn held_review(conn: &Connection, task: &Task, text: &str, info: &str) -> Result<Settled> {
    let held = set_status(conn, task.id, TaskStatus::Review, None, true)?;
    note(conn, task.id, CommentKind::Result, text)?;
    Ok(Settled { outcome: Outcome::Held, effects: vec![Effect::Info { text: format!("Task #{} in {} waits for you: {info}", held.id, place(&held)) }] })
}

pub fn settle(conn: &Connection, id: i64, folder: &str, facts: &Facts, team: &Team) -> Result<Settled> {
    let task = must_task(conn, id)?;
    let checks = checks_text(facts);
    let state = handoff_state(conn, id)?;
    let member = member_of(conn, &task, team, &state);
    if facts.no_base && !facts.changed.is_empty() {
        let why = if task.base.is_empty() { "Nixa has no start point for this claim" } else { "Nixa could not read the start point of this claim" };
        let n = facts.changed.len();
        let text = format!("Checked by Nixa: {n} changed file{} in the folder ({}). {why}, so it cannot tell which of these files the task changed.", plural(n), file_list(&facts.changed, SHOWN_FILES));
        return hold_for_check(conn, &task, &text, team);
    }
    if facts.checks.as_ref().is_some_and(|c| c.failed) {
        return checks_failed(conn, &task, member.as_ref(), &state, facts, &checks, team);
    }
    if facts.unread {
        let text = format!("Nixa could not read the git changes in {folder} (git failed or too many changed files), so it cannot tell what changed.{checks}");
        if team.deep_first {
            let question = format!("Task #{id} in {}: {text} Check the work, then pick: done, send a fix, or give it to another agent.", place(&task));
            note(conn, id, CommentKind::Note, &format!("{text} Nixa asked the deep brain."))?;
            return Ok(Settled { outcome: Outcome::Held, effects: vec![Effect::AskDeep { task: id, question }] });
        }
        return held_review(conn, &task, &format!("{text} Waiting for the user: check the work."), "Nixa could not read its git changes.");
    }
    let comments = task_comments(conn, id)?;
    let mut summary = format!("Checked by Nixa: no code change.{}", if checks.is_empty() { " No checks to run." } else { &checks });
    let mut mine: Vec<String> = Vec::new();
    let mut shared = false;
    if !facts.changed.is_empty() {
        let count = format!("{} changed file{}", facts.changed.len(), plural(facts.changed.len()));
        let named: Vec<&String> = comments.iter().filter(|c| c.author_kind != AuthorKind::System && c.at >= task.claimed_at).flat_map(|c| c.links.files.iter()).collect();
        mine = facts.changed.iter().filter(|f| named.iter().any(|n| same_file(folder, n, f))).cloned().collect();
        let others = sharing(conn, &task, folder, team)?;
        shared = !others.is_empty();
        let mut whose = String::new();
        if !mine.is_empty() {
            if mine.len() == facts.changed.len() {
                whose.push_str(" The agent named all of them.");
            } else {
                whose.push_str(&format!(" The agent named {} of them: {}.", mine.len(), clip(&mine.join(", "), 300)));
            }
        }
        if shared {
            let list: Vec<String> = others.iter().map(|o| format!("#{o}")).collect();
            whose.push_str(&format!(" Other agents also worked in this folder during this task ({}), so some of these files may be theirs.", list.join(", ")));
        }
        summary = format!("Checked by Nixa: {count} ({}).{whose}{checks}", file_list(&facts.changed, SHOWN_FILES));
    }
    let step = match &member {
        Some(m) => after_step(m, &task.then_member, findings_of(&step_report(&task, &comments)), &state),
        None => HandoffMove::Done,
    };
    let step = match step {
        HandoffMove::Done => pickup(&task, &comments, member.as_ref(), &state, Work { changed: &facts.changed, mine: &mine, shared }, team).unwrap_or(HandoffMove::Done),
        other => other,
    };
    if step != HandoffMove::Done {
        return move_task(conn, &task, step, Moving { member: member.as_ref(), summary: &summary, files: &facts.changed, stop_old: false }, team);
    }
    if let Some(why) = no_proof(&task, &comments, member.as_ref(), facts, &mine, shared) {
        return hold_for_check(conn, &task, &format!("{summary} {why}"), team);
    }
    finish(conn, &task, facts, &summary, team.deep_first)
}

fn no_proof(task: &Task, comments: &[TaskComment], member: Option<&Person>, facts: &Facts, mine: &[String], shared: bool) -> Option<String> {
    let report = step_report(task, comments);
    if let Some(line) = report.lines().find(|line| FAILING.is_match(line)) {
        return Some(format!("The agent's own report names a failing check: \"{}\".", clip(line.trim(), 200)));
    }
    if facts.changed.is_empty() && task.uncommitted.is_empty() {
        return None;
    }
    let said: String = comments.iter().filter(|c| c.author_kind == AuthorKind::Agent && c.at >= task.claimed_at).map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n");
    let told = facts.changed.iter().filter(|f| mine.contains(f) || names(&said, f)).count();
    let passed = facts.checks.as_ref().is_some_and(|c| !c.failed);
    let reviewed = member.is_some_and(|m| is_review_role(&m.role));
    let named = told > 0 && (told == facts.changed.len() || shared);
    if passed || reviewed || named {
        return None;
    }
    Some("No checks ran and the agent did not name the changed files, so Nixa has no proof that the work is done.".into())
}

fn names(text: &str, file: &str) -> bool {
    let part = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '.');
    text.match_indices(file).any(|(at, _)| {
        let before = text[..at].chars().next_back().is_none_or(|c| !part(c) || c == '/');
        let mut after = text[at + file.len()..].chars();
        let ends = match after.next() {
            None => true,
            Some('.') => after.next().is_none_or(|c| !c.is_alphanumeric()),
            Some(c) => !part(c),
        };
        before && ends
    })
}

fn hold_for_check(conn: &Connection, task: &Task, text: &str, team: &Team) -> Result<Settled> {
    if team.deep_first {
        let question = format!("Task #{} in {}: {text} Check the work, then pick: done, send a fix, or give it to another agent.", task.id, place(task));
        note(conn, task.id, CommentKind::Note, &format!("{text} Nixa asked the deep brain."))?;
        return Ok(Settled { outcome: Outcome::Held, effects: vec![Effect::AskDeep { task: task.id, question }] });
    }
    held_review(conn, task, &format!("{text} Waiting for the user: check the work."), "Nixa has no proof that the work is done.")
}

pub fn commit_tail(auto: bool) -> &'static str {
    if auto {
        " Not committed yet: Nixa decides the git step itself after the check, or asks the deep brain with nixa_deep_ask. It does not wait for the user."
    } else {
        " Not committed: the user commits when they want. Nixa does not commit."
    }
}

pub fn finish_checked(conn: &Connection, task: &Task, facts: &Facts, summary: &str, auto: bool) -> Result<Settled> {
    finish(conn, task, facts, summary, auto)
}

fn finish(conn: &Connection, task: &Task, facts: &Facts, summary: &str, auto: bool) -> Result<Settled> {
    let earlier: Vec<String> = task.uncommitted.iter().filter(|f| !facts.changed.contains(f)).cloned().collect();
    let mut kept = facts.changed.clone();
    kept.extend(earlier.iter().cloned());
    remove_task_meta(conn, &handoff_key(task.id))?;
    remove_task_meta(conn, &handoff_start_key(task.id))?;
    set_status(conn, task.id, TaskStatus::Done, None, false)?;
    if !kept.is_empty() {
        set_uncommitted(conn, task.id, &kept)?;
    }
    let done = must_task(conn, task.id)?;
    let before = if earlier.is_empty() {
        String::new()
    } else {
        format!(" Earlier steps of this task changed {} more file{} ({}).", earlier.len(), plural(earlier.len()), clip(&earlier.join(", "), 300))
    };
    let tail = if kept.is_empty() { "" } else { commit_tail(auto) };
    let line = format!("{summary}{before} Done.{tail}");
    note(conn, task.id, CommentKind::Result, &line)?;
    let mut effects = Vec::new();
    if !kept.is_empty() {
        effects.push(Effect::Info {
            text: format!("Task #{} in {} is done and checked: {} changed file{}, not committed.", task.id, place(&done), kept.len(), plural(kept.len())),
        });
    }
    Ok(Settled { outcome: Outcome::Done, effects })
}

fn checks_failed(conn: &Connection, task: &Task, member: Option<&Person>, state: &HandoffState, facts: &Facts, checks: &str, team: &Team) -> Result<Settled> {
    let fails = state.fails + 1;
    if let Some(m) = member
        && fails >= 2
        && let Some(up) = escalation(m, state, "could not make Nixa's checks pass two times")
    {
        let summary = format!("Nixa's checks failed again.{checks}");
        return move_task(conn, task, up, Moving { member, summary: &summary, files: &facts.changed, stop_old: true }, team);
    }
    if fails >= CHECK_FAIL_LIMIT {
        if !in_chain(state) {
            remove_task_meta(conn, &handoff_key(task.id))?;
        }
        let why = format!("Nixa's checks failed {fails} times.");
        return move_task(conn, task, HandoffMove::User { why }, Moving { member, summary: checks.trim(), files: &facts.changed, stop_old: false }, team);
    }
    let mut next = state.clone();
    next.fails = fails;
    set_handoff_state(conn, task.id, &next)?;
    if task.terminal_id.is_empty() || is_starting(&task.terminal_id) || is_run_holder(&task.terminal_id) {
        if team.deep_first {
            set_status(conn, task.id, TaskStatus::Review, None, false)?;
            return hold_for_check(conn, task, &format!("Nixa's checks failed.{checks} No agent holds the task."), team);
        }
        return held_review(conn, task, &format!("Nixa's checks failed.{checks} No agent holds the task, so it waits for the user."), "Nixa's checks failed.");
    }
    set_status(conn, task.id, TaskStatus::Doing, None, false)?;
    note(conn, task.id, CommentKind::Error, &format!("Nixa's checks failed.{checks} Back to doing."))?;
    let text = format!(
        "Nixa's checks failed on task #{}.{checks} Fix it, then run: $NIXA_BIN task status {} review --note \"the result and the proof\"",
        task.id, task.id
    );
    crate::util::log("review_flow", &format!("review failed, task {}, fails {fails}", task.id));
    Ok(Settled { outcome: Outcome::Back, effects: vec![Effect::Send { terminal: task.terminal_id.clone(), text }] })
}

pub fn escalate_blocked(conn: &Connection, id: i64, changed: &[String], team: &Team) -> Result<Option<Settled>> {
    let Some(task) = get_task(conn, id)? else {
        return Ok(None);
    };
    if task.status != TaskStatus::Blocked || task.blocked_on.is_some_and(BlockedOn::is_question) || task.terminal_id.is_empty() {
        return Ok(None);
    }
    let state = handoff_state(conn, id)?;
    let Some(member) = member_of(conn, &task, team, &state).map(|m| job_member(conn, m)) else {
        return Ok(None);
    };
    let Some(up) = escalation(&member, &state, "is stuck") else {
        return Ok(None);
    };
    let comments = task_comments(conn, id)?;
    let said = comments.iter().rev().find(|c| c.author_kind != AuthorKind::System && c.at >= task.claimed_at);
    let on = task.blocked_on.map(|b| b.as_str()).unwrap_or("other");
    let summary = match said {
        Some(c) => format!("Blocked on {on}: \"{}\".", clip(&c.text, 300)),
        None => format!("Blocked on {on}."),
    };
    move_task(conn, &task, up, Moving { member: Some(&member), summary: &summary, files: changed, stop_old: true }, team).map(Some)
}

pub fn seat_gone(conn: &Connection, id: i64, why: &str, team: &Team) -> Result<Option<Settled>> {
    let Some(task) = get_task(conn, id)? else {
        return Ok(None);
    };
    let state = handoff_state(conn, id)?;
    let gone = match task.assigned_kind.as_str() {
        "agent" => custom_id_of(&task.assigned),
        "member" if !task.assigned.is_empty() && member_for(&team.people, &task.assigned).is_none() => {
            Some(task.assigned.trim().trim_start_matches("custom:").trim_start_matches("member:").to_string())
        }
        _ => None,
    };
    let Some(gone) = gone else {
        return Ok(None);
    };
    if task.status != TaskStatus::Todo || !task.terminal_id.is_empty() || !state.auto {
        return Ok(None);
    }
    let why = clip(why, 300);
    let gone_ref = custom_ref(&gone);
    if state.reviewer != gone_ref {
        if state.fixer != gone_ref {
            return Ok(None);
        }
        let Some(fixer) = fix_after(PICKUP_ROLE).and_then(|role| member_for(&team.people, role)).cloned() else {
            remove_task_meta(conn, &handoff_start_key(id))?;
            let step = HandoffMove::User { why: format!("{why} The review found problems, and Nixa has no agent to fix them.") };
            return move_task(conn, &task, step, Moving { member: None, summary: "The fix step could not start.", files: &[], stop_old: false }, team).map(Some);
        };
        update_task(conn, id, &TaskPatch { assigned_kind: Some("member".into()), assigned: Some(fixer.id.clone()), ..TaskPatch::default() })?;
        let mut next = state.clone();
        next.fixer = fixer.id.clone();
        set_handoff_state(conn, id, &next)?;
        note(conn, id, CommentKind::Note, &format!("{why} Nixa gave the fix to {}.", fixer.name))?;
        return Ok(Some(Settled { outcome: Outcome::Moved, effects: Vec::new() }));
    }
    let worker = custom_id_of(&state.back).unwrap_or_default();
    if let Some(next) = pick_reviewer(&team.seats, &task.space_id, "", &worker, std::slice::from_ref(&gone)) {
        let reference = custom_ref(&next.id);
        update_task(conn, id, &TaskPatch { assigned_kind: Some("agent".into()), assigned: Some(reference.clone()), ..TaskPatch::default() })?;
        let mut moved = state.clone();
        moved.reviewer = reference;
        set_handoff_state(conn, id, &moved)?;
        note(conn, id, CommentKind::Note, &format!("{why} Nixa gave the review to {} ({}), another review agent of this space.", next.title, next.id))?;
        return Ok(Some(Settled { outcome: Outcome::Moved, effects: Vec::new() }));
    }
    remove_task_meta(conn, &handoff_key(id))?;
    remove_task_meta(conn, &handoff_start_key(id))?;
    let what = if state.recheck { "the fix was not checked again" } else { "no agent reviewed the code" };
    let text = format!("{why} No other review agent is saved for this space, so {what}. The task stays in review until it is checked.");
    if team.deep_first {
        set_status(conn, id, TaskStatus::Review, None, false)?;
    }
    hold_for_check(conn, &task, &text, team).map(Some)
}

pub fn folder_of(task: &Task, team: &Team) -> Option<String> {
    team.folders
        .get(&task.space_id)
        .filter(|f| !f.trim().is_empty())
        .cloned()
        .or_else(|| Some(task.repo.clone()).filter(|r| !r.trim().is_empty() && Path::new(r).is_dir()))
}
