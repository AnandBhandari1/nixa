use super::prompt::{PlanInput, build_plan};
use super::waits::{keep_blocked, sweep, waits_on_open};
use super::*;
use nixa_store::spaces::SpacePatch;
use nixa_store::task_waits::{set_waits, waits_of};
use nixa_store::tasks::{BlockedOn, TaskInput, TaskStatus};

fn setup() -> (tempfile::TempDir, Arc<App>, Space) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let folder = dir.path().join("work");
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.display().to_string();
    let patch = SpacePatch { name: Some("nixa".into()), folder: Some(folder.clone()), ..SpacePatch::default() };
    nixa_store::spaces::set_space(&db(&app).unwrap(), "nixa", &patch).unwrap();
    (dir, app, Space { id: "nixa".into(), name: "nixa".into(), folder })
}

fn add(app: &App, title: &str, assigned: &str) -> Task {
    let input = TaskInput {
        title: title.into(),
        status: Some(TaskStatus::Todo),
        space_id: "nixa".into(),
        space_name: "nixa".into(),
        assigned_kind: crate::tasks::KIND_TERMINAL.into(),
        assigned: assigned.into(),
        ..TaskInput::default()
    };
    tasks::add_task(&db(app).unwrap(), &input, &nixa()).unwrap()
}

fn fresh(app: &App, id: i64) -> Task {
    tasks::get_task(&db(app).unwrap(), id).unwrap().unwrap()
}

fn said(app: &App, id: i64, text: &str) -> Vec<nixa_store::tasks::TaskComment> {
    tasks::task_comments(&db(app).unwrap(), id).unwrap().into_iter().filter(|c| c.text.contains(text)).collect()
}

#[tokio::test]
async fn auto_start_skips_a_task_while_a_task_it_waits_on_is_open() {
    let (_dir, app, space) = setup();
    let conn = db(&app).unwrap();
    let first = add(&app, "Build the store", "claude");
    let then = add(&app, "Use the store", "claude");
    set_waits(&conn, then.id, &[first.id]).unwrap();
    assert!(waits_on_open(&conn, then.id), "the real start below must stop at the wait check");
    super::start::start_one(&app, then.clone(), space.clone(), How::Auto).await;
    let now = fresh(&app, then.id);
    assert_eq!(now.status, TaskStatus::Todo);
    assert!(now.terminal_id.is_empty());
    assert!(!super::dispatch::ready(&app, &conn, &now, &places(&conn)));
    let refused = super::start::start_now(&app, then.id, false).await.unwrap_err();
    assert!(refused.contains(&format!("waits on task #{}, still open", first.id)), "{refused}");
    let key = format!("{}{}", super::waits::START, then.id);
    assert!(tasks::task_meta(&conn, &key).unwrap().is_some(), "the refused start is kept for the wait sweep");
    assert_eq!(fresh(&app, then.id).status, TaskStatus::Todo);
    tasks::set_status(&conn, first.id, TaskStatus::Done, None, false).unwrap();
    assert!(!waits_on_open(&conn, then.id));
}

#[tokio::test]
async fn a_task_blocked_on_tasks_goes_back_to_todo_when_they_all_close() {
    let (_dir, app, _) = setup();
    let conn = db(&app).unwrap();
    let a = add(&app, "First part", "claude");
    let b = add(&app, "Second part", "claude");
    let waiting = add(&app, "Join the parts", "claude");
    set_waits(&conn, waiting.id, &[a.id, b.id]).unwrap();
    tasks::set_status(&conn, waiting.id, TaskStatus::Blocked, Some(BlockedOn::Task), false).unwrap();
    tasks::set_status(&conn, a.id, TaskStatus::Done, None, false).unwrap();
    sweep(&app).await;
    assert_eq!(fresh(&app, waiting.id).status, TaskStatus::Blocked, "one open task still holds it");
    tasks::set_status(&conn, b.id, TaskStatus::Cancelled, None, false).unwrap();
    sweep(&app).await;
    let now = fresh(&app, waiting.id);
    assert_eq!(now.status, TaskStatus::Todo);
    assert!(now.blocked_on.is_none());
    assert!(waits_of(&conn, waiting.id).is_empty());
    assert_eq!(said(&app, waiting.id, &format!("All tasks it waited on are closed (task #{}, task #{}). Back to todo.", a.id, b.id)).len(), 1);
    sweep(&app).await;
    assert_eq!(said(&app, waiting.id, "All tasks it waited on are closed").len(), 1, "a resume happens once");
}

#[tokio::test]
async fn a_held_task_goes_back_to_doing_and_its_holder_is_told() {
    let (_dir, app, _) = setup();
    let conn = db(&app).unwrap();
    let first = add(&app, "First part", "claude");
    let held = add(&app, "Second part", "claude");
    let by = nixa_store::tasks::Actor::in_terminal(AuthorKind::Agent, "worker", "m5");
    assert!(tasks::claim_task(&conn, held.id, &by, "").unwrap().ok);
    set_waits(&conn, held.id, &[first.id]).unwrap();
    tasks::set_status(&conn, held.id, TaskStatus::Blocked, Some(BlockedOn::Task), false).unwrap();
    assert!(keep_blocked(&conn, &fresh(&app, held.id)), "the quiet sweep keeps a task that waits on an open task");
    tasks::set_status(&conn, first.id, TaskStatus::Done, None, false).unwrap();
    assert!(!keep_blocked(&conn, &fresh(&app, held.id)));
    sweep(&app).await;
    let now = fresh(&app, held.id);
    assert_eq!((now.status, now.terminal_id.as_str()), (TaskStatus::Doing, "m5"));
    assert!(now.active_at >= now.claimed_at, "the resume counts as activity, so the quiet sweep does not free it at once");
    let told = said(&app, held.id, "It is back in doing. Go on with it");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].author_kind, AuthorKind::Nixa, "a Nixa note is delivered to the holder");
    let comments = tasks::task_comments(&conn, held.id).unwrap();
    assert_eq!(super::notes::since_claim(&now, &comments, 0).len(), 1, "the note is due for delivery");
}

#[tokio::test]
async fn a_task_blocked_on_the_user_stays_blocked_when_its_waits_close() {
    let (_dir, app, _) = setup();
    let conn = db(&app).unwrap();
    let first = add(&app, "First part", "claude");
    let asks = add(&app, "Ask the user", "claude");
    let by = nixa_store::tasks::Actor::in_terminal(AuthorKind::Agent, "worker", "m6");
    assert!(tasks::claim_task(&conn, asks.id, &by, "").unwrap().ok);
    set_waits(&conn, asks.id, &[first.id]).unwrap();
    tasks::set_status(&conn, asks.id, TaskStatus::Blocked, Some(BlockedOn::User), true).unwrap();
    tasks::set_status(&conn, first.id, TaskStatus::Done, None, false).unwrap();
    sweep(&app).await;
    let now = fresh(&app, asks.id);
    assert_eq!((now.status, now.blocked_on, now.needs_user), (TaskStatus::Blocked, Some(BlockedOn::User), true));
    assert_eq!(said(&app, asks.id, "It is still blocked on user").len(), 1);
    assert!(said(&app, asks.id, "Go on").is_empty());
}

#[tokio::test]
async fn a_plan_step_waits_instead_of_blocking() {
    let (_dir, app, _) = setup();
    let conn = db(&app).unwrap();
    let first = add(&app, "First part", "claude");
    let planned = add(&app, "Planned work", "");
    let step = nixa_store::task_plans::StepInput { owner: "agent:claude".into(), work: "build it".into(), output: String::new() };
    nixa_store::task_plans::set_plan(&conn, planned.id, &[step], "test", &[]).unwrap();
    set_waits(&conn, planned.id, &[first.id]).unwrap();
    super::plan::advance(&app, planned.id, false).await;
    let plan = nixa_store::task_plans::get_plan(&conn, planned.id).unwrap().unwrap();
    assert_eq!(plan.steps[0].state, nixa_store::task_plans::StepState::Todo, "a wait is not a failed start, and the step does not begin");
    assert_eq!(fresh(&app, planned.id).status, TaskStatus::Todo);
    let key = format!("{}{}", super::waits::PLAN, planned.id);
    assert!(tasks::task_meta(&conn, &key).unwrap().is_some(), "the wait sweep advances the plan once the waits close");
}

#[test]
fn the_start_plan_names_the_tasks_it_waits_on() {
    let (_dir, app, space) = setup();
    let task = add(&app, "Use the store", "claude");
    let input = PlanInput { space: space.name.clone(), folder: space.folder.clone(), waits: vec![3, 9], ..PlanInput::default() };
    assert_eq!(build_plan(&task, &input).dependencies, vec!["task #3", "task #9"]);
    assert!(build_plan(&task, &PlanInput::default()).dependencies.is_empty());
}
