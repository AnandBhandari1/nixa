use super::actions;
use super::busy::{self, Claim, Evidence, Why};
use super::live::{self, Live};
use crate::app::{App, Seen, lock};
use nixa_protocol::{Status, TermId, TermInfo};
use nixa_store::agents::{AddAgent, AgentState, AgentUse, CustomAgent, add_agent, use_agent};
use nixa_store::tasks::{self, Actor, AuthorKind, BlockedOn, TaskInput, TaskStatus};
use std::collections::BTreeMap;

fn term(id: u64, status: Option<Status>, running: bool) -> TermInfo {
    TermInfo {
        id: TermId::from_number(id),
        name: String::new(),
        labels: BTreeMap::new(),
        parent: None,
        children: Vec::new(),
        cmd: vec!["claude".into()],
        cwd: "/tmp".into(),
        pid: None,
        running,
        exit_code: None,
        status,
        status_note: None,
        keep: false,
        draft: false,
        title: None,
        rows: 24,
        cols: 80,
        created_ms: 0,
        seq: 0,
    }
}

fn agent(id: &str, terminal: &str) -> CustomAgent {
    CustomAgent {
        id: id.into(),
        title: "Developer".into(),
        purpose: "code developer".into(),
        instructions: String::new(),
        role: String::new(),
        job: String::new(),
        workflow: String::new(),
        start: None,
        off: false,
        migrated_from: String::new(),
        provider: "claude".into(),
        session: String::new(),
        space: "s1".into(),
        space_name: String::new(),
        folder: String::new(),
        terminal: terminal.into(),
        created_at: 1,
        last_used: 1,
        by: "test".into(),
        skills: Vec::new(),
    }
}

fn live_with(status: Option<Status>, running: bool, evidence: Evidence) -> Live {
    let mut live = Live { evidence, ..Live::default() };
    live.terms.insert("m4".into(), term(4, status, running));
    live
}

fn claim(task: i64, status: TaskStatus, blocked_on: Option<BlockedOn>) -> Evidence {
    let mut ev = Evidence::default();
    ev.claims.insert("m4".into(), vec![Claim { task, status, blocked_on, space: String::new() }]);
    ev
}

fn view(live: &Live) -> live::AgentView {
    live::view_of(&nixa_protocol::paths::data_dir(), &agent("a7", "m4"), live, 1_000_000).0
}

#[test]
fn a_task_claim_with_no_space_makes_a_quiet_agent_busy() {
    let live = live_with(None, true, claim(73, TaskStatus::Doing, None));
    let v = view(&live);
    assert_eq!(v.state, AgentState::Working);
    assert_eq!(v.busy.len(), 1);
    assert_eq!(v.busy[0].why, Why::Task);
    assert_eq!(v.busy[0].task, Some(73));
    assert!(!busy::free_now(v.state, &v.busy, false, v.sure));
    assert!(v.unsure.is_empty(), "{}", v.unsure);
    let line = live::view_line(&v);
    assert!(line.ends_with("working in terminal m4: holds task #73 (doing, no space)"), "{line}");
    let json = serde_json::to_value(&v).unwrap();
    assert_eq!(json["busy"][0]["task"], 73);
    assert_eq!(json["busy"][0]["why"], "task");
}

#[test]
fn a_blocked_claim_or_a_question_means_waiting() {
    let v = view(&live_with(None, true, claim(74, TaskStatus::Blocked, Some(BlockedOn::User))));
    assert_eq!(v.state, AgentState::Waiting);
    assert!(live::view_line(&v).contains("holds task #74 (blocked on user, no space)"));
    let mut ev = Evidence::default();
    ev.asked.insert("m4".into());
    let v = view(&live_with(None, true, ev));
    assert_eq!(v.state, AgentState::Waiting);
    assert_eq!(v.busy[0].why, Why::Question);
}

#[test]
fn jobs_and_deep_seats_count_as_working() {
    let mut ev = Evidence::default();
    ev.jobs.insert("m4".into(), 1_000_000 - 5 * 60_000);
    let v = view(&live_with(None, true, ev));
    assert_eq!(v.state, AgentState::Working);
    assert!(v.busy[0].text.contains("5 minutes ago"), "{}", v.busy[0].text);
    let mut ev = Evidence::default();
    ev.deep.insert("m4".into());
    assert_eq!(view(&live_with(None, true, ev)).state, AgentState::Working);
}

#[test]
fn a_quiet_free_agent_says_it_is_not_sure() {
    let v = view(&live_with(None, true, Evidence::default()));
    assert_eq!(v.state, AgentState::Free);
    assert!(v.busy.is_empty());
    assert!(v.unsure.contains("reports no status"), "{}", v.unsure);
    assert!(live::view_line(&v).contains("(not sure: its terminal reports no status"));
    let v = view(&live_with(Some(Status::Done), true, Evidence::default()));
    assert_eq!(v.state, AgentState::Free);
    assert!(v.unsure.is_empty());
    let mut live = live_with(Some(Status::Done), true, Evidence::default());
    live.gaps.push("the engine did not answer".into());
    assert!(view(&live).unsure.contains("the engine did not answer"));
}

#[test]
fn a_closed_terminal_with_a_claim_is_closed_and_unsure() {
    let v = view(&live_with(None, false, claim(73, TaskStatus::Doing, None)));
    assert_eq!(v.state, AgentState::Closed);
    assert!(v.busy.is_empty());
    assert!(v.unsure.contains("closed terminal still holds task #73"), "{}", v.unsure);
}

#[test]
fn mixed_signals_prefer_waiting_and_finished_jobs_clear() {
    let mut ev = claim(73, TaskStatus::Doing, None);
    ev.asked.insert("m4".into());
    assert_eq!(view(&live_with(None, true, ev)).state, AgentState::Waiting);
    let mut ev = Evidence::default();
    ev.jobs.insert("m4".into(), 1_000_000 - 60_000);
    ev.since.insert("m4".into(), (Some(Status::Done), 1_000_000 - 30_000));
    let v = view(&live_with(Some(Status::Done), true, ev));
    assert_eq!(v.state, AgentState::Free);
    assert!(v.busy.is_empty());
    let mut ev = Evidence::default();
    ev.jobs.insert("m4".into(), 1_000);
    let v = live::view_of(&nixa_protocol::paths::data_dir(), &agent("a7", "m4"), &live_with(None, true, ev), 1_000 + busy::JOB_QUIET_MS + 1).0;
    assert_eq!(v.state, AgentState::Free);
    assert!(v.unsure.contains("never heard back"), "{}", v.unsure);
}

#[test]
fn a_job_after_an_old_done_stays_busy_until_the_quiet_limit_and_handoffs_count() {
    let mut ev = Evidence::default();
    ev.jobs.insert("m4".into(), 500_000);
    ev.since.insert("m4".into(), (Some(Status::Done), 100_000));
    let a = agent("a7", "m4");
    let dir = nixa_protocol::paths::data_dir();
    let v = live::view_of(&dir, &a, &live_with(Some(Status::Done), true, ev.clone()), 600_000).0;
    assert_eq!(v.state, AgentState::Working);
    let v = live::view_of(&dir, &a, &live_with(Some(Status::Done), true, ev), 500_000 + busy::JOB_QUIET_MS + 1).0;
    assert_eq!(v.state, AgentState::Free);
    assert!(v.unsure.contains("never heard back"));
    let mut ev = Evidence::default();
    ev.handed.insert("m4".into(), 990_000);
    let v = view(&live_with(Some(Status::Done), true, ev.clone()));
    assert_eq!(v.state, AgentState::Working);
    assert!(v.busy[0].text.starts_with("Nixa handed it work"));
    let v = live::view_of(&dir, &a, &live_with(Some(Status::Done), true, ev), 990_000 + busy::HANDED_MS + 1).0;
    assert_eq!(v.state, AgentState::Free);
}

#[test]
fn an_engine_gap_is_not_sure() {
    let mut live = live_with(Some(Status::Done), true, Evidence::default());
    live.gaps.push("the engine did not answer".into());
    let v = view(&live);
    assert!(!v.sure);
    assert!(!busy::free_now(v.state, &v.busy, false, v.sure));
}

#[test]
fn unreadable_claims_keep_an_agent_out_of_the_free_list() {
    let ev = Evidence { claims_ok: false, gaps: vec!["task claims could not be read".into()], ..Evidence::default() };
    let v = view(&live_with(Some(Status::Done), true, ev));
    assert_eq!(v.state, AgentState::Free);
    assert!(!v.sure);
    assert!(!busy::free_now(v.state, &v.busy, false, v.sure));
    assert!(v.unsure.contains("task claims could not be read"));
}

#[test]
fn the_shared_view_reads_claims_from_any_space() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let id = TermId::from_number(5).to_string();
    lock(&app.watch).world.insert(id.clone(), Seen { info: term(5, None, true), since: 0 });
    let conn = crate::queue::db(&app).unwrap();
    let a = add_agent(&conn, &AddAgent { title: "Developer".into(), purpose: "code developer".into(), provider: "claude".into(), ..AddAgent::default() }, "test").unwrap();
    let a = use_agent(&conn, &a.id, &AgentUse { terminal: Some(id.clone()), ..AgentUse::default() }).unwrap();
    let task = tasks::add_task(&conn, &TaskInput { title: "Wrap the composer".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() }, &Actor::new(AuthorKind::User, "user")).unwrap();
    assert!(tasks::claim_task(&conn, task.id, &Actor::in_terminal(AuthorKind::Agent, "Developer", &id), "").unwrap().ok);
    let views = actions::views_now(&app, std::slice::from_ref(&a));
    assert_eq!(views[0].state, AgentState::Working);
    assert_eq!(views[0].busy[0].task, Some(task.id));
    assert!(views[0].sure);
    assert!(actions::free_ids(&views).is_empty());
    assert_eq!(super::start::step(&views[0], &super::start::UseInput::default(), false), super::start::Step::Busy);
    let line = live::view_line(&views[0]);
    assert!(line.contains(&format!("working in terminal {id}: holds task #{} (doing, no space)", task.id)), "{line}");
    tasks::set_status(&conn, task.id, TaskStatus::Review, None, false).unwrap();
    let views = actions::views_now(&app, std::slice::from_ref(&a));
    assert_eq!(views[0].state, AgentState::Free);
    assert!(views[0].unsure.contains("reports no status"));
    assert_eq!(actions::free_ids(&views), vec![a.id.clone()]);
}
