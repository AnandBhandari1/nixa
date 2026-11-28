use crate::app::App;
use crate::engine;
use nixa_protocol::{ChildReport, Reply, Request, Status};
use regex::Regex;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

pub const BLOCKED_ALERT_MS: u64 = 60_000;
pub const QUIET_ALERT_MS: u64 = 90_000;
pub const READ_LINES: usize = 40;
pub const TAIL_LINES: usize = 8;
pub const TAIL_CHARS: usize = 800;
pub const MARK_NOTE: &str = "stopped without a result, parent told";

static INTERRUPTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)interrupted by user|conversation interrupted|^\W*interrupted\b").expect("interrupt pattern"));
static BUSY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)esc to (?:interrupt|cancel)|^\s*[⠀-⣿✻✶✳✢✽·∙●◐◓◑◒◴◵◶◷]\s+\S+…|\bworking \(\d").expect("busy pattern"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Idle,
    Prompt,
    Error,
    Quiet,
    Interrupted,
}

impl Why {
    pub fn text(self) -> &'static str {
        match self {
            Why::Idle => "went idle without a result",
            Why::Prompt => "is waiting at a prompt",
            Why::Error => "reported an error and stopped",
            Why::Quiet => "stopped working but still says working",
            Why::Interrupted => "was interrupted",
        }
    }

    pub fn marks(self) -> bool {
        !matches!(self, Why::Prompt | Why::Error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub parent: String,
    pub status: Option<Status>,
    pub status_at: u64,
    pub alerted: bool,
    pub screen: Option<(String, u64)>,
}

#[derive(Debug, Default)]
pub struct Kids {
    pub jobs: HashMap<String, Job>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Send { to: String, text: String },
    Mark { id: String },
}

fn open(parent: &str, now: u64) -> Job {
    Job { parent: parent.to_string(), status: Some(Status::Working), status_at: now, alerted: false, screen: None }
}

pub fn on_status(kids: &mut Kids, id: &str, parent: Option<&str>, status: Option<Status>, now: u64) -> Option<Why> {
    let Some(parent) = parent else {
        kids.jobs.remove(id);
        return None;
    };
    match status {
        Some(Status::Working) => {
            let fresh = kids.jobs.get(id).is_none_or(|j| j.alerted || j.parent != parent);
            if fresh {
                kids.jobs.insert(id.to_string(), open(parent, now));
            } else if let Some(job) = kids.jobs.get_mut(id)
                && job.status != status
            {
                job.status = status;
                job.status_at = now;
                job.screen = None;
            }
            None
        }
        Some(Status::Done) => {
            kids.jobs.remove(id);
            None
        }
        _ => {
            let job = kids.jobs.get_mut(id)?;
            let was = job.status;
            if was != status {
                job.status = status;
                job.status_at = now;
                job.screen = None;
            }
            (!job.alerted && was == Some(Status::Working) && status == Some(Status::Idle)).then_some(Why::Idle)
        }
    }
}

pub fn seed(kids: &mut Kids, id: &str, parent: Option<&str>, status: Option<Status>, now: u64) {
    if let Some(parent) = parent
        && !kids.jobs.contains_key(id)
        && matches!(status, Some(Status::Working | Status::Blocked | Status::Error))
    {
        let mut job = open(parent, now);
        job.status = status;
        kids.jobs.insert(id.to_string(), job);
    }
}

pub fn gone(kids: &mut Kids, id: &str) {
    kids.jobs.remove(id);
}

pub fn keep_alive(kids: &mut Kids, alive: &dyn Fn(&str) -> bool) {
    kids.jobs.retain(|id, _| alive(id));
}

pub fn due(kids: &Kids, now: u64) -> (Vec<(String, Why)>, Vec<String>) {
    let mut alerts = Vec::new();
    let mut looks = Vec::new();
    let mut ids: Vec<&String> = kids.jobs.keys().collect();
    ids.sort();
    for id in ids {
        let job = &kids.jobs[id];
        if job.alerted {
            continue;
        }
        let waited = now.saturating_sub(job.status_at);
        match job.status {
            Some(Status::Blocked) if waited >= BLOCKED_ALERT_MS => alerts.push((id.clone(), Why::Prompt)),
            Some(Status::Error) if waited >= BLOCKED_ALERT_MS => alerts.push((id.clone(), Why::Error)),
            Some(Status::Working) if waited >= QUIET_ALERT_MS => looks.push(id.clone()),
            _ => {}
        }
    }
    (alerts, looks)
}

pub fn judge_screen(kids: &mut Kids, id: &str, screen: &str, now: u64) -> Option<Why> {
    let job = kids.jobs.get_mut(id).filter(|j| !j.alerted && j.status == Some(Status::Working))?;
    let tail = tail_of(screen, READ_LINES);
    if INTERRUPTED.is_match(&tail_of(screen, TAIL_LINES)) {
        return Some(Why::Interrupted);
    }
    if BUSY.is_match(&tail) {
        job.screen = Some((tail, now));
        return None;
    }
    match &job.screen {
        Some((seen, at)) if *seen == tail && now.saturating_sub(*at) >= QUIET_ALERT_MS => Some(Why::Quiet),
        Some((seen, _)) if *seen == tail => None,
        _ => {
            job.screen = Some((tail, now));
            None
        }
    }
}

pub fn closable(status: Option<Status>, keep: bool, reports: &[ChildReport], child: &str) -> bool {
    status == Some(Status::Done)
        && !keep
        && reports.iter().rev().find(|r| r.child.as_str() == child).is_some_and(|r| r.current_job_result && r.read)
}

async fn close_read_children(app: &Arc<App>, terms: &[nixa_protocol::TermInfo]) {
    if !crate::settings_api::nixa_on(&app.dir) {
        return;
    }
    let mut seen: HashMap<String, Vec<ChildReport>> = HashMap::new();
    for t in terms {
        let Some(parent) = t.parent.as_ref().map(|p| p.to_string()) else { continue };
        if t.status != Some(Status::Done) || t.keep {
            continue;
        }
        if !seen.contains_key(&parent) {
            let reports = reports_of(&parent).await;
            seen.insert(parent.clone(), reports);
        }
        if closable(t.status, t.keep, &seen[&parent], t.id.as_str()) {
            crate::util::log("kids", &format!("closed done child {} after parent {parent} read its report", t.id));
            let _ = engine::quick(Request::Kill { id: t.id.clone() }).await;
        }
    }
}

pub fn has_result(reports: &[ChildReport], child: &str) -> bool {
    reports.iter().any(|r| r.child.as_str() == child && r.current_job_result)
}

fn tail_of(screen: &str, lines: usize) -> String {
    let kept: Vec<&str> = screen.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    kept[kept.len().saturating_sub(lines)..].join("\n")
}

pub fn last_lines(screen: &str) -> String {
    let clean = crate::prompts::screen::clean_screen(screen, READ_LINES);
    let tail = tail_of(&clean, TAIL_LINES);
    let hidden = nixa_store::text::hide_secrets(&tail, &[]);
    let mut out: String = hidden.chars().take(TAIL_CHARS).collect();
    if hidden.chars().count() > TAIL_CHARS {
        out.push('…');
    }
    out
}

pub fn alert_text(child: &str, name: &str, why: Why, lines: &str) -> String {
    let shown = if lines.trim().is_empty() { "(nothing on screen)".to_string() } else { lines.to_string() };
    format!(
        "Nixa: your child terminal {child} ({name}) {} for its job. Last lines:\n{shown}\nRead it with nixa term read {child}, then send it the next step or stop it.",
        why.text()
    )
}

pub fn plan(job: &Job, child: &str, name: &str, why: Why, parent_running: bool, lines: &str) -> Vec<Effect> {
    let mut out = Vec::new();
    if parent_running {
        out.push(Effect::Send { to: job.parent.clone(), text: alert_text(child, name, why, lines) });
    }
    if why.marks() {
        out.push(Effect::Mark { id: child.to_string() });
    }
    out
}

async fn reports_of(parent: &str) -> Vec<ChildReport> {
    let Ok(id) = engine::term_id(parent) else { return Vec::new() };
    match engine::quick(Request::ChildReports { id, reader: None }).await {
        Ok(Reply::ChildReports { reports }) => reports,
        _ => Vec::new(),
    }
}

async fn run_effect(effect: Effect) {
    match effect {
        Effect::Send { to, text } => {
            if let Ok(id) = engine::term_id(&to)
                && let Err(error) = engine::send(&id, &text, true, false).await
            {
                crate::util::log("kids", &format!("could not tell parent {to}: {error}"));
            }
        }
        Effect::Mark { id } => {
            if let Ok(id) = engine::term_id(&id) {
                let _ = engine::quick(Request::StatusSet { id, status: Some(Status::Blocked), note: Some(MARK_NOTE.into()) }).await;
            }
        }
    }
}

pub async fn alert(kids: &mut Kids, child: &str, why: Why) {
    let Some(job) = kids.jobs.get(child).cloned().filter(|j| !j.alerted) else { return };
    if why == Why::Idle && has_result(&reports_of(&job.parent).await, child) {
        kids.jobs.remove(child);
        return;
    }
    let Ok(id) = engine::term_id(child) else { return };
    let name = engine::get(&id).await.map(|t| crate::util::clip_flat(&t.name, 40)).unwrap_or_default();
    let screen = engine::read(&id, Some(READ_LINES), false).await.unwrap_or_default();
    let parent_running = match engine::term_id(&job.parent) {
        Ok(parent) => engine::get(&parent).await.is_ok_and(|t| t.running),
        Err(_) => false,
    };
    if let Some(job) = kids.jobs.get_mut(child) {
        job.alerted = true;
    }
    crate::util::log("kids", &format!("child {child} {}; parent {} told: {parent_running}", why.text(), job.parent));
    for effect in plan(&job, child, &name, why, parent_running, &last_lines(&screen)) {
        run_effect(effect).await;
    }
}

pub const WORKING: &str = "working";
pub const WORKING_TEXT: &str = "The agent is still busy on its job, for example a long build, test or download is running. It is not stuck, not finished and not waiting for an answer.";

pub fn working_input(child: &str, lines: &str) -> String {
    let shown = if lines.trim().is_empty() { "(nothing on screen)".to_string() } else { crate::agent::data_block(lines) };
    format!("Child agent terminal {child}. Its screen did not change for a while. Last lines:\n{shown}")
}

async fn still_working(app: &Arc<App>, child: &str, screen: &str) -> bool {
    if !crate::settings_api::nixa_on(&app.dir) {
        return false;
    }
    let set = crate::decide::settings::load(app);
    if !set.uses("screens") {
        return false;
    }
    let questions = vec![crate::decide::predicate(WORKING, WORKING_TEXT)];
    let answers = match crate::decide::ask(app, &set.model, vec![crate::decide::text(&working_input(child, &last_lines(screen)))], questions).await {
        Ok(answers) => answers,
        Err(_) => return false,
    };
    let busy = matches!(answers.get(WORKING), Some(crate::decide::Answer::Predicate { probability, .. }) if *probability >= set.level);
    if busy {
        crate::util::log("kids", &format!("child {child} looks quiet, but the decision model says it is still working"));
    }
    busy
}

pub async fn tick(app: &Arc<App>, kids: &mut Kids, now: u64) {
    let terms: Vec<_> = app.terminals().into_iter().filter(|s| s.info.running).map(|s| s.info).collect();
    keep_alive(kids, &|id| terms.iter().any(|t| t.id.as_str() == id));
    for t in &terms {
        seed(kids, t.id.as_str(), t.parent.as_ref().map(|p| p.to_string()).as_deref(), t.status, now);
    }
    let (alerts, looks) = due(kids, now);
    for id in looks {
        let Ok(term) = engine::term_id(&id) else { continue };
        let Ok(screen) = engine::read(&term, Some(READ_LINES), false).await else { continue };
        if let Some(why) = judge_screen(kids, &id, &screen, now) {
            if why == Why::Quiet && still_working(app, &id, &screen).await {
                if let Some(job) = kids.jobs.get_mut(&id) {
                    job.screen = None;
                }
                continue;
            }
            alert(kids, &id, why).await;
        }
    }
    for (id, why) in alerts {
        alert(kids, &id, why).await;
    }
    close_read_children(app, &terms).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_protocol::TermId;

    fn report(child: &str, result: bool) -> ChildReport {
        ChildReport { child: child.parse::<TermId>().unwrap(), status: Some(Status::Idle), exited: false, note: String::new(), current_job_result: result, read: false }
    }

    #[test]
    fn a_done_child_closes_only_after_the_parent_read_its_result() {
        let mut done = report("m3", true);
        assert!(!closable(Some(Status::Done), false, &[done.clone()], "m3"));
        done.read = true;
        assert!(closable(Some(Status::Done), false, &[done.clone()], "m3"));
        assert!(!closable(Some(Status::Done), true, &[done.clone()], "m3"));
        assert!(!closable(Some(Status::Idle), false, &[done.clone()], "m3"));
        assert!(!closable(Some(Status::Done), false, &[done.clone()], "m4"));
        assert!(!closable(Some(Status::Done), false, &[done, report("m3", false)], "m3"));
    }

    #[test]
    fn a_job_opens_only_for_a_child() {
        let mut kids = Kids::default();
        assert_eq!(on_status(&mut kids, "m2", None, Some(Status::Working), 1), None);
        assert!(kids.jobs.is_empty());
        assert_eq!(on_status(&mut kids, "m2", None, Some(Status::Idle), 2), None);
        on_status(&mut kids, "m3", Some("m1"), Some(Status::Working), 1);
        assert_eq!(kids.jobs["m3"].parent, "m1");
    }

    #[test]
    fn working_then_idle_alerts_once_and_done_does_not() {
        let mut kids = Kids::default();
        on_status(&mut kids, "m3", Some("m1"), Some(Status::Working), 1);
        assert_eq!(on_status(&mut kids, "m3", Some("m1"), Some(Status::Idle), 2), Some(Why::Idle));
        kids.jobs.get_mut("m3").unwrap().alerted = true;
        assert_eq!(on_status(&mut kids, "m3", Some("m1"), Some(Status::Idle), 3), None);
        on_status(&mut kids, "m4", Some("m1"), Some(Status::Working), 1);
        assert_eq!(on_status(&mut kids, "m4", Some("m1"), Some(Status::Done), 2), None);
        assert!(!kids.jobs.contains_key("m4"));
        assert_eq!(on_status(&mut kids, "m4", Some("m1"), Some(Status::Idle), 3), None);
    }

    #[test]
    fn a_new_working_turn_opens_a_new_job() {
        let mut kids = Kids::default();
        on_status(&mut kids, "m3", Some("m1"), Some(Status::Working), 1);
        on_status(&mut kids, "m3", Some("m1"), Some(Status::Idle), 2);
        kids.jobs.get_mut("m3").unwrap().alerted = true;
        on_status(&mut kids, "m3", Some("m1"), Some(Status::Blocked), 3);
        assert!(kids.jobs["m3"].alerted);
        on_status(&mut kids, "m3", Some("m1"), Some(Status::Working), 4);
        assert!(!kids.jobs["m3"].alerted);
        assert_eq!(on_status(&mut kids, "m3", Some("m1"), Some(Status::Idle), 5), Some(Why::Idle));
    }

    #[test]
    fn a_long_prompt_or_error_is_due_once() {
        let mut kids = Kids::default();
        on_status(&mut kids, "m3", Some("m1"), Some(Status::Working), 0);
        on_status(&mut kids, "m3", Some("m1"), Some(Status::Blocked), 1_000);
        assert!(due(&kids, 1_000 + BLOCKED_ALERT_MS - 1).0.is_empty());
        assert_eq!(due(&kids, 1_000 + BLOCKED_ALERT_MS).0, vec![("m3".to_string(), Why::Prompt)]);
        on_status(&mut kids, "m5", Some("m1"), Some(Status::Working), 0);
        on_status(&mut kids, "m5", Some("m1"), Some(Status::Error), 0);
        kids.jobs.get_mut("m3").unwrap().alerted = true;
        assert_eq!(due(&kids, BLOCKED_ALERT_MS * 5).0, vec![("m5".to_string(), Why::Error)]);
    }

    #[test]
    fn a_quiet_screen_alerts_and_a_busy_one_does_not() {
        let mut kids = Kids::default();
        on_status(&mut kids, "m3", Some("m1"), Some(Status::Working), 0);
        assert!(due(&kids, QUIET_ALERT_MS - 1).1.is_empty());
        assert_eq!(due(&kids, QUIET_ALERT_MS).1, vec!["m3".to_string()]);
        let quiet = "I changed the parser.\n\n❯ \n";
        assert_eq!(judge_screen(&mut kids, "m3", quiet, QUIET_ALERT_MS), None);
        assert_eq!(judge_screen(&mut kids, "m3", quiet, QUIET_ALERT_MS * 2 - 1), None);
        assert_eq!(judge_screen(&mut kids, "m3", quiet, QUIET_ALERT_MS * 2), Some(Why::Quiet));
        let busy = "Reading files\n✻ Thinking… (12s · esc to interrupt)\n";
        on_status(&mut kids, "m4", Some("m1"), Some(Status::Working), 0);
        for at in [1, 2, 3] {
            assert_eq!(judge_screen(&mut kids, "m4", busy, QUIET_ALERT_MS * at), None);
        }
        on_status(&mut kids, "m6", Some("m1"), Some(Status::Working), 0);
        assert_eq!(judge_screen(&mut kids, "m6", "Fixing it\n⎿  Interrupted by user\n❯ \n", QUIET_ALERT_MS), Some(Why::Interrupted));
        assert_eq!(judge_screen(&mut kids, "m7", quiet, QUIET_ALERT_MS), None);
    }

    #[test]
    fn seeds_only_open_children_and_forgets_gone_ones() {
        let mut kids = Kids::default();
        seed(&mut kids, "m3", Some("m1"), Some(Status::Working), 5);
        seed(&mut kids, "m4", Some("m1"), Some(Status::Idle), 5);
        seed(&mut kids, "m5", None, Some(Status::Working), 5);
        assert_eq!(kids.jobs.keys().collect::<Vec<_>>(), vec!["m3"]);
        on_status(&mut kids, "m6", Some("m1"), Some(Status::Working), 1);
        gone(&mut kids, "m6");
        keep_alive(&mut kids, &|id| id != "m3");
        assert!(kids.jobs.is_empty());
    }

    #[test]
    fn only_a_current_job_result_counts() {
        assert!(!has_result(&[], "m3"));
        assert!(!has_result(&[report("m3", false), report("m4", true)], "m3"));
        assert!(has_result(&[report("m3", false), report("m3", true)], "m3"));
        let plain = ChildReport { child: "m3".parse().unwrap(), status: Some(Status::Done), exited: false, note: "done".into(), current_job_result: false, read: false };
        assert!(!has_result(&[plain], "m3"));
    }

    #[test]
    fn the_alert_names_the_child_and_hides_secrets() {
        let screen = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nkey sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345\n";
        let lines = last_lines(screen);
        assert!(!lines.contains("line 2\n") && lines.contains("line 9"));
        assert!(!lines.contains("abcdefghijklmnopqrstuvwxyz012345"));
        let text = alert_text("m3", "Tester", Why::Interrupted, &lines);
        assert!(text.starts_with("Nixa: your child terminal m3 (Tester) was interrupted for its job. Last lines:"));
        assert!(text.contains("nixa term read m3"));
        assert!(alert_text("m3", "T", Why::Idle, " ").contains("(nothing on screen)"));
    }

    #[test]
    fn the_plan_tells_the_parent_once_and_marks_the_child() {
        let job = open("m1", 0);
        let effects = plan(&job, "m3", "Tester", Why::Idle, true, "last");
        assert_eq!(effects.len(), 2);
        assert!(matches!(&effects[0], Effect::Send { to, text } if to == "m1" && text.contains("m3 (Tester) went idle without a result")));
        assert_eq!(effects[1], Effect::Mark { id: "m3".into() });
        assert_eq!(plan(&job, "m3", "T", Why::Prompt, true, ""), vec![Effect::Send { to: "m1".into(), text: alert_text("m3", "T", Why::Prompt, "") }]);
        assert_eq!(plan(&job, "m3", "T", Why::Quiet, false, ""), vec![Effect::Mark { id: "m3".into() }]);
    }
}
