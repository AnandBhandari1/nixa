use super::dispatch::{self, AUTO, Fit, HELD, fit, mark_of};
use super::*;
use nixa_store::agents::{AddAgent, CustomAgent, add_agent};
use nixa_store::spaces::SpacePatch;
use nixa_store::task_plans::{self as plans, StepInput};
use nixa_store::tasks::{TaskInput, TaskStatus};
use std::sync::Mutex;

const DEEP: &str = "nixa-claude-dp0001";

static STARTED: Mutex<Vec<(String, i64)>> = Mutex::new(Vec::new());

fn fake_start(app: Arc<App>, task: Task, _: Space) -> BoxFuture<'static, ()> {
    let key = app.dir.display().to_string();
    Box::pin(async move { STARTED.lock().unwrap().push((key, task.id)) })
}

fn started(app: &App) -> Vec<i64> {
    let key = app.dir.display().to_string();
    STARTED.lock().unwrap().iter().filter(|(k, _)| *k == key).map(|(_, id)| *id).collect()
}

fn setup() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let folder = dir.path().join("work");
    std::fs::create_dir_all(&folder).unwrap();
    let conn = db(&app).unwrap();
    let patch = SpacePatch { name: Some("ace".into()), folder: Some(folder.display().to_string()), ..SpacePatch::default() };
    nixa_store::spaces::set_space(&conn, "s3", &patch).unwrap();
    let seat: crate::deep::Seat = serde_json::from_value(json!({"space": "s3", "id": "d1", "name": "Deep brain", "terminal": DEEP})).unwrap();
    lock(&app.deep).seats.push(seat);
    (dir, app)
}

fn agent(app: &App, title: &str, purpose: &str) -> CustomAgent {
    let input = AddAgent { title: title.into(), purpose: purpose.into(), ..AddAgent::default() };
    add_agent(&db(app).unwrap(), &input, "test").unwrap()
}

fn plain(id: &str, title: &str, purpose: &str) -> CustomAgent {
    let text = |s: &str| s.to_string();
    CustomAgent {
        id: text(id),
        title: text(title),
        purpose: text(purpose),
        instructions: String::new(),
        role: String::new(),
        job: String::new(),
        workflow: String::new(),
        start: None,
        off: false,
        migrated_from: String::new(),
        provider: String::new(),
        session: String::new(),
        space: String::new(),
        space_name: String::new(),
        folder: String::new(),
        terminal: String::new(),
        created_at: 0,
        last_used: 0,
        by: String::new(),
        skills: Vec::new(),
    }
}

fn add(app: &App, title: &str, status: TaskStatus, assigned: &str, terminal: &str) -> Task {
    let input = TaskInput {
        title: title.into(),
        status: Some(status),
        space_id: "s3".into(),
        space_name: "ace".into(),
        assigned_kind: if assigned.is_empty() { String::new() } else { crate::tasks::KIND_CUSTOM.into() },
        assigned: assigned.into(),
        ..TaskInput::default()
    };
    let by = nixa_store::tasks::Actor::in_terminal(AuthorKind::Agent, terminal, terminal);
    tasks::add_task(&db(app).unwrap(), &input, &by).unwrap()
}

fn fresh(app: &App, id: i64) -> Task {
    tasks::get_task(&db(app).unwrap(), id).unwrap().unwrap()
}

fn said(app: &App, id: i64, text: &str) -> usize {
    tasks::task_comments(&db(app).unwrap(), id).unwrap().iter().filter(|c| c.text.contains(text)).count()
}

#[test]
fn a_pick_needs_a_clear_lead_over_the_next_agent() {
    let qa = plain("a9", "QA Tester", "Writes and runs tests, fixes failing tests");
    let dev = plain("a7", "Developer", "Normal fixes and features, refactors, mobile");
    let writer = plain("a13", "Writer", "Docs, text users read, translation");
    let team = vec![qa.clone(), dev.clone(), writer.clone()];
    assert_eq!(fit(&team, "Fix failing tests"), Fit::One(Box::new(qa.clone())));
    assert_eq!(fit(&team, "Update the docs"), Fit::One(Box::new(writer)));
    assert_eq!(fit(&team, "Zebra quokka"), Fit::Nobody);
    let twin = plain("a20", "Mobile Developer", "Normal fixes and features, refactors, mobile");
    match fit(&[dev.clone(), twin.clone()], "Mobile fixes") {
        Fit::Several(list) => assert_eq!(list.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["a20", "a7"]),
        other => panic!("a tie must be ambiguous, got {other:?}"),
    }
    let off = CustomAgent { off: true, ..qa };
    assert_eq!(fit(&[off], "Fix failing tests"), Fit::Nobody);
}

#[tokio::test]
async fn nixa_assigns_and_starts_a_deep_brain_task() {
    let (_dir, app) = setup();
    let qa = agent(&app, "QA Tester", "Writes and runs tests, fixes failing tests");
    agent(&app, "Writer", "Docs, text users read, translation");
    let task = add(&app, "Fix failing tests", TaskStatus::Inbox, "", DEEP);
    let other = add(&app, "Fix failing tests too", TaskStatus::Todo, &qa.id, "nixa-claude-other1");
    dispatch::tick(&app, fake_start).await;
    let now = fresh(&app, task.id);
    assert_eq!(now.status, TaskStatus::Todo);
    assert_eq!((now.assigned_kind.as_str(), now.assigned.as_str()), (crate::tasks::KIND_CUSTOM, qa.id.as_str()));
    assert_eq!(mark_of(&db(&app).unwrap(), task.id).as_deref(), Some(AUTO));
    assert_eq!(said(&app, task.id, "In ace, Nixa starts this deep brain task"), 1);
    assert_eq!(said(&app, task.id, "Nixa assigned this deep brain task to QA Tester"), 1);
    assert_eq!(started(&app), vec![task.id]);
    assert!(mark_of(&db(&app).unwrap(), other.id).is_none(), "a task from a normal agent is left alone");
}

#[tokio::test]
async fn nixa_picks_an_agent_for_unclear_tasks() {
    let (_dir, app) = setup();
    agent(&app, "Developer", "Normal fixes and features, refactors, mobile");
    agent(&app, "Mobile Developer", "Normal fixes and features, refactors, mobile");
    let nobody = add(&app, "Zebra quokka", TaskStatus::Todo, "", DEEP);
    let several = add(&app, "Mobile fixes", TaskStatus::Inbox, "", DEEP);
    dispatch::tick(&app, fake_start).await;
    let first = nixa_store::agent_map::fallover(&db(&app).unwrap()).unwrap().into_iter().next().unwrap();
    let now = fresh(&app, nobody.id);
    assert_eq!((now.assigned_kind.as_str(), now.assigned.as_str()), (crate::tasks::KIND_TERMINAL, first.as_str()));
    let now = fresh(&app, several.id);
    assert_eq!(now.assigned_kind, crate::tasks::KIND_CUSTOM);
    assert!(["a1", "a2"].contains(&now.assigned.as_str()));
    assert_eq!(said(&app, several.id, "Nixa picked"), 1);
    for id in [nobody.id, several.id] {
        assert_ne!(mark_of(&db(&app).unwrap(), id).as_deref(), Some(HELD));
    }
}

#[tokio::test]
async fn existing_gates_still_hold_the_start() {
    let (_dir, app) = setup();
    let qa = agent(&app, "QA Tester", "Writes and runs tests, fixes failing tests");
    let planned = add(&app, "Fix failing tests first", TaskStatus::Todo, &qa.id, DEEP);
    plans::set_plan(&db(&app).unwrap(), planned.id, &[StepInput { owner: "user".into(), work: "check the screen".into(), output: String::new() }], "test", &[]).unwrap();
    dispatch::tick(&app, fake_start).await;
    assert!(started(&app).is_empty(), "a plan step owned by the user holds the start");
    assert_eq!(mark_of(&db(&app).unwrap(), planned.id).as_deref(), Some(AUTO));

    crate::app::lock(&app.switches.work).hold = Some(crate::settings_api::work::Hold { at: crate::util::now_ms(), alias: "hold".into() });
    let held = add(&app, "Fix failing tests now", TaskStatus::Todo, &qa.id, DEEP);
    assert!(crate::modes::gate(&app, "auto-start", None).is_some(), "the real start below must stop at the Hold keyword");
    dispatch::tick(&app, dispatch::queue_start).await;
    let now = fresh(&app, held.id);
    assert_eq!(now.status, TaskStatus::Todo);
    assert!(now.terminal_id.is_empty(), "the Hold keyword holds the real queue start");

    let conn = db(&app).unwrap();
    nixa_store::spaces::set_space(&conn, "s3", &SpacePatch { locked: Some(true), ..SpacePatch::default() }).unwrap();
    let locked = add(&app, "Fix failing tests later", TaskStatus::Todo, &qa.id, DEEP);
    dispatch::tick(&app, fake_start).await;
    assert!(mark_of(&conn, locked.id).is_none(), "a locked space is not touched");
    nixa_store::spaces::set_space(&conn, "s3", &SpacePatch { locked: Some(false), ..SpacePatch::default() }).unwrap();

    crate::app::lock(&app.switches.work).hold = None;
    dispatch::tick(&app, fake_start).await;
}

#[tokio::test]
async fn twins_and_agents_of_locked_spaces_are_left_to_the_inbox_check() {
    let (_dir, app) = setup();
    let conn = db(&app).unwrap();
    let qa = agent(&app, "QA Tester", "Writes and runs tests, fixes failing tests");
    let first = add(&app, "Fix failing tests", TaskStatus::Todo, &qa.id, "m1");
    assert!(tasks::claim_task(&conn, first.id, &nixa_store::tasks::Actor::in_terminal(AuthorKind::Agent, "QA", "m1"), "").unwrap().ok);
    let twin = add(&app, "Fix failing tests!", TaskStatus::Inbox, "", DEEP);
    dispatch::tick(&app, fake_start).await;
    assert!(mark_of(&conn, twin.id).is_none(), "a twin of #{} waits for the inbox check", first.id);
    assert_eq!(fresh(&app, twin.id).status, TaskStatus::Inbox);
    nixa_store::spaces::set_space(&conn, "s9", &SpacePatch { name: Some("other".into()), folder: Some(app.dir.display().to_string()), locked: Some(true), ..SpacePatch::default() }).unwrap();
    nixa_store::agents::use_agent(&conn, &qa.id, &nixa_store::agents::AgentUse { space: Some("s9".into()), ..Default::default() }).unwrap();
    let task = add(&app, "Make the failing tests pass", TaskStatus::Todo, "", DEEP);
    dispatch::tick(&app, fake_start).await;
    assert_eq!(said(&app, task.id, "No custom agent fits its title"), 1, "an agent of a locked space is not picked");
    let fell = add(&app, "Make the failing tests green", TaskStatus::Todo, "", DEEP);
    dispatch::tick(&app, fake_start).await;
    assert_eq!(said(&app, fell.id, "No custom agent fits its title. Nixa gave it to the agent"), 1, "an agent of a locked space is not picked");
    assert_ne!(fresh(&app, fell.id).assigned, qa.id);
    let given = add(&app, "Run the failing tests", TaskStatus::Todo, &qa.id, DEEP);
    dispatch::tick(&app, fake_start).await;
    assert_eq!(mark_of(&conn, given.id).as_deref(), Some(AUTO));
    assert!(!started(&app).contains(&given.id), "an assigned agent of a locked space is not started");
}

#[test]
fn a_retry_mark_is_due_after_ten_minutes() {
    let mark = format!("{}{}", dispatch::RETRY, 1_000);
    assert!(!dispatch::retry_due(&mark, 1_000 + dispatch::RETRY_MS - 1));
    assert!(dispatch::retry_due(&mark, 1_000 + dispatch::RETRY_MS));
    assert!(!dispatch::retry_due(AUTO, u64::MAX));
}

#[tokio::test]
async fn nixa_puts_a_task_with_no_space_in_an_open_space() {
    let (_dir, app) = setup();
    let qa = agent(&app, "QA Tester", "Writes and runs tests, fixes failing tests");
    let input = TaskInput { title: "Fix failing tests".into(), status: Some(TaskStatus::Inbox), ..TaskInput::default() };
    let by = nixa_store::tasks::Actor::in_terminal(AuthorKind::Agent, DEEP, DEEP);
    let task = tasks::add_task(&db(&app).unwrap(), &input, &by).unwrap();
    dispatch::tick(&app, fake_start).await;
    let now = fresh(&app, task.id);
    assert_eq!(now.space_id, "s3");
    assert_eq!(now.assigned, qa.id);
    assert_eq!(said(&app, task.id, "Nixa put it in ace"), 1);
    assert_eq!(started(&app), vec![task.id]);
}
