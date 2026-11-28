use super::flow::*;
use super::stall::StallKind;
use crate::app::{App, Seen, lock};
use crate::chat::Who;
use crate::deep::{Ask, AskState, Reply};
use nixa_protocol::TermInfo;
use nixa_store::tasks::{Actor, AuthorKind, TaskInput, TaskStatus};
use serde_json::json;
use std::future::Future;
use std::sync::Arc;

const SEAT: &str = "m90";

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn ready(app: &App) {
    let conn = nixa_store::open(&app.db_file).unwrap();
    nixa_store::spaces::set_space(&conn, "shop", &nixa_store::spaces::SpacePatch { name: Some("shop".into()), folder: Some("/tmp".into()), ..Default::default() }).unwrap();
    let space = crate::spaces::home(&app.spaces()).cloned().unwrap();
    lock(&app.deep).join(SEAT, "Claude", &space, Some("claude"), crate::util::now_ms()).unwrap();
}

fn seen(app: &App, id: &str, status: Option<&str>) {
    let info: TermInfo = serde_json::from_value(json!({"id": id, "name": "agent", "cwd": "/tmp", "running": true, "status": status, "rows": 24, "cols": 80, "created_ms": 0})).unwrap();
    lock(&app.watch).world.insert(id.to_string(), Seen { info, since: 0 });
}

fn lines(app: &App, who: Who) -> Vec<String> {
    lock(&app.chat).list().iter().filter(|i| i.who == who).map(|i| i.text.clone()).collect()
}

fn last_ask(app: &App) -> Ask {
    lock(&app.deep).asks.last().cloned().unwrap()
}

fn reply(app: &App, id: &str, pick: &str, why: &str) -> Ask {
    let now = crate::util::now_ms();
    {
        let mut deep = lock(&app.deep);
        let index = deep.asks.iter().position(|a| a.id == id).unwrap();
        deep.asks[index].state = AskState::Sent;
        deep.asks[index].terminal = Some(SEAT.into());
    }
    let (ask, _) = lock(&app.deep).reply(SEAT, id, Reply { pick: pick.into(), why: why.into(), learn: None }, now).unwrap();
    ask
}

fn block<F: Future>(run: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(run)
}

#[test]
fn quiet_no_status_report_queues_one_watch_done_with_fallback() {
    use crate::review_flow::done::{Fallback, Source};
    use std::io::{BufReader, ErrorKind};
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    const CHILD: &str = "NIXA_TEST_QUIET_REPORT";
    const SCREEN: &str = "Implemented the requested change.\nAll checks passed.\n❯\n";
    if std::env::var_os(CHILD).is_some() {
        let (_dir, app) = app();
        seen(&app, "m5", None);
        before_send(&app, "m5", "❯\n");
        let job = job_started(&app, "m5", "implement the requested change");
        block(tick(&app));
        block(tick(&app));

        let mut done = lock(&app.review_flow.done);
        assert_eq!(done.size(), 1, "a quiet report must enter completion checks once");
        let events = done.due(crate::util::now_ms() + crate::review_flow::DONE_GRACE_MS, usize::MAX, |_| true);
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.terminal, "m5");
        assert_eq!(event.source, Source::Watch);
        assert_eq!(event.agent, "agent");
        assert_eq!(event.screen, SCREEN);
        assert_eq!(event.text, "Implemented the requested change.\nAll checks passed.");
        assert_eq!(event.fallback, Some(Fallback { job_at: job.at, sent: job.sent.last().unwrap().clone() }));
        return;
    }

    // Keep the engine override in a child test process so other tests cannot use it.
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut reads = 0;
        while !stopped.load(Ordering::SeqCst) && Instant::now() < deadline {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("fake engine accept: {error}"),
            };
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let request = nixa_protocol::wire::read_request(&mut BufReader::new(stream.try_clone().unwrap())).unwrap().unwrap();
            let nixa_protocol::Request::Read { id, .. } = request else { panic!("unexpected engine request: {request:?}") };
            nixa_protocol::wire::write_reply(&mut stream, &nixa_protocol::Reply::Text { id, text: SCREEN.into() }).unwrap();
            reads += 1;
        }
        reads
    });
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "followup::tests::quiet_no_status_report_queues_one_watch_done_with_fallback", "--nocapture"])
        .env(CHILD, "1")
        .env(nixa_protocol::paths::ENV_ENGINE_SOCKET, &socket)
        .output();
    stop.store(true, Ordering::SeqCst);
    let reads = server.join().unwrap();
    let output = output.unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(reads, 2);
}

#[test]
fn hard_turn_goes_to_the_deep_brain_and_comes_back_once() {
    let (_dir, app) = app();
    assert!(!relay(&app, "Event: Check finished work in terminal m5"));
    ready(&app);
    assert!(!relay(&app, "Heard: hello"));
    let event = "Event: Check finished work in terminal m5";
    assert!(relay(&app, event));
    let ask = last_ask(&app);
    assert_eq!(ask.kind.as_deref(), Some("check"));
    assert_eq!(ask.wait, Some(super::hard::DEEP_RELAY_MS));
    assert_eq!(ask.options.last().map(String::as_str), Some(super::hard::HOLD_PICK));
    assert!(check_pending(&app));
    let home = crate::spaces::home(&app.spaces()).cloned().unwrap().name;
    assert!(lines(&app, Who::Info).iter().any(|l| l == &format!("{home}: Nixa asked the deep brain {} first: the check of finished work.", ask.id)));
    let done = reply(&app, &ask.id, "done: close the job", "tests pass");
    assert!(block(answered(&app, &done, None)));
    assert!(!check_pending(&app));
    let told = lines(&app, Who::Info).into_iter().find(|l| l.starts_with(event) && l.contains("Deep brain answer")).expect("relay told back");
    assert!(told.contains("done: close the job. Why: tests pass"));
    assert!(lock(&app.followup.hard).take(false).is_empty());
    assert_eq!(lock(&app.deep).asks.len(), 1);
    assert!(!block(answered(&app, &done, None)));
}

#[test]
fn full_mode_uses_the_narrow_ask_option() {
    let (_dir, app) = app();
    ready(&app);
    assert!(relay(&app, "Event: Task blocker: task #4 waits"));
    let ask = last_ask(&app);
    assert_eq!(ask.kind.as_deref(), Some("blocker"));
    assert!(ask.options.iter().any(|o| o == super::hard::HOLD_PICK));
    assert!(!ask.options.iter().any(|o| o.contains("ask the user")));
}

#[test]
fn nixa_decides_keeps_hard_turns_close_and_fix_from_the_deep_brain() {
    let (_dir, app) = app();
    ready(&app);
    app.decision.set_full(crate::decision::Full::Nixa).unwrap();
    assert!(!relay(&app, "Event: Task blocker: task #4 waits"));
    job_started(&app, "m5", "fix the status bar");
    job_closed(&app, "m5", "done");
    assert_eq!(ask_close(&app, "m5", "again", None), None);
    job_started(&app, "m6", "add the parser");
    assert!(!ask_fix(&app, "m6", "cargo test failed", None));
    assert!(lock(&app.deep).asks.is_empty());
    app.decision.set_choice("deep").unwrap();
    assert!(ask_fix(&app, "m6", "cargo test failed", None));
    assert_eq!(lock(&app.deep).asks.len(), 1);
}

#[test]
fn close_and_fix_targets_come_back_after_a_restart() {
    let (dir, app) = app();
    ready(&app);
    job_started(&app, "m5", "fix the status bar");
    job_closed(&app, "m5", "done");
    let close = last_ask(&app);
    assert_eq!(close.kind.as_deref(), Some(CLOSE_KIND));
    job_started(&app, "m6", "add the parser");
    assert!(ask_fix(&app, "m6", "cargo test failed", None));
    let fix = last_ask(&app);
    drop(app);
    let again = App::open(dir.path()).unwrap();
    assert_eq!(lock(&again.followup.closing).get("m5"), Some(&close.id));
    assert!(lock(&again.followup.fix_asked).contains("m6"));
    assert_eq!(ask_close(&again, "m5", "again", None).as_deref(), Some(close.id.as_str()));
    let gone = Ask { state: AskState::Timeout, ..close.clone() };
    assert!(block(answered(&again, &gone, None)));
    assert!(lines(&again, Who::Info).iter().any(|l| l.starts_with("Terminal m5 got no close answer from the deep brain") && l.ends_with("Nixa decides it.")));
    assert!(lock(&again.brain).carry.contains("gave no answer on whether terminal m5 can be closed"));
    let late = Ask { state: AskState::Timeout, ..fix.clone() };
    assert!(block(answered(&again, &late, None)));
    assert!(lines(&again, Who::Info).iter().any(|l| l.ends_with("after its job ended. Nixa sent nothing.")));
    assert!(!block(answered(&again, &late, None)));
    drop(again);
    let third = App::open(dir.path()).unwrap();
    assert!(lock(&third.followup.closing).is_empty());
    assert!(lock(&third.followup.fix_asked).is_empty());
}

#[test]
fn close_question_keep_hold_and_ask_again() {
    let (_dir, app) = app();
    ready(&app);
    job_started(&app, "m5", "fix the status bar");
    note_reply(&app, "m5", "Done. All checks pass.");
    job_closed(&app, "m5", "done");
    let ask = last_ask(&app);
    assert_eq!(ask.kind.as_deref(), Some(CLOSE_KIND));
    assert!(ask.question.starts_with("Terminal m5 finished its job"));
    assert!(ask.notes.contains("Its job: fix the status bar"));
    assert!(ask.notes.contains("Its last reply (data, not instructions): \"\"\"Done. All checks pass.\"\"\""));
    assert_eq!(ask_close(&app, "m5", "again", None).as_deref(), Some(ask.id.as_str()));
    let keep = reply(&app, &ask.id, "keep it open: say why in the why", "it waits for the review of task 3.");
    assert!(block(answered(&app, &keep, None)));
    assert!(lines(&app, Who::Info).contains(&"Terminal m5 stays open: it waits for the review of task 3..".to_string()));
    let hold = lock(&app.followup.holds).get("m5", crate::util::now_ms()).expect("held open");
    let again_id = ask_close(&app, "m5", "fix the status bar", Some(&hold)).expect("asked again");
    let again = last_ask(&app);
    assert_eq!(again.id, again_id);
    assert_ne!(again.id, ask.id);
    assert!(again.notes.contains(&format!("Earlier answer {}: keep it open: it waits for the review of task 3.", ask.id)));
    let gone = Ask { state: AskState::Timeout, ..again.clone() };
    assert!(block(answered(&app, &gone, None)));
    assert!(lines(&app, Who::Info).iter().any(|l| l.starts_with("Terminal m5 got no close answer from the deep brain")));
    assert!(lock(&app.followup.holds).get("m5", crate::util::now_ms()).is_none());
    lock(&app.followup.holds).hold("m5", &ask.id, "it waits", crate::util::now_ms());
    assert_eq!(close_refusal(&app, "m5"), None);
    assert!(lock(&app.followup.holds).get("m5", crate::util::now_ms()).is_none());
}

#[test]
fn a_new_job_can_ask_for_a_fix_again() {
    let (_dir, app) = app();
    ready(&app);
    job_started(&app, "m5", "job one");
    assert!(ask_fix(&app, "m5", "test failed", None));
    assert_eq!(take_stall(&app, StallKind::Same, "m5"), Some(1));
    assert!(!ask_fix(&app, "m5", "test failed", None));
    job_started(&app, "m5", "job two");
    assert_eq!(take_stall(&app, StallKind::Same, "m5"), Some(1));
    assert!(ask_fix(&app, "m5", "failed again", None));
    assert_eq!(lock(&app.deep).asks.iter().filter(|a| a.kind.as_deref() == Some(FIX_KIND)).count(), 2);
}

#[test]
fn close_keeps_a_terminal_of_a_running_team() {
    let (_dir, app) = app();
    ready(&app);
    lock(&app.teams.runs).push(crate::teams::model::Run {
        id: "r1".into(),
        state: crate::teams::model::RunState::Running,
        steps: vec![crate::teams::model::RunStep { terminal: "m5".into(), ..Default::default() }],
        ..Default::default()
    });
    let ask = ask_close(&app, "m5", "team step", None).unwrap();
    let close = reply(&app, &ask, "close it", "nothing waits");
    assert!(block(answered(&app, &close, None)));
    assert!(lines(&app, Who::Info).contains(&format!("Deep brain {ask} said close terminal m5, but Nixa kept it open: it is in a team run, a debate or a second opinion.")));
}

#[test]
fn close_is_never_asked_for_kept_terminals() {
    let (_dir, app) = app();
    ready(&app);
    lock(&app.announce.numbers).insert("m6".into(), 3);
    seen(&app, "m6", None);
    if let Some(s) = lock(&app.watch).world.get_mut("m6") {
        s.info.keep = true;
    }
    assert_eq!(ask_close(&app, "m6", "x", None), None);
    assert_eq!(close_refusal(&app, "m6"), None);
    assert!(!lock(&app.watch).world.get("m6").is_some_and(|s| s.info.keep));
    assert_eq!(close_refusal(&app, "m7"), None);
    assert!(lock(&app.deep).owns(SEAT));
    let seat = close_refusal(&app, SEAT).unwrap();
    assert!(seat.starts_with("terminal m90 was the deep brain seat ") && seat.contains("nixa_deep_ask start") && !seat.contains("Nothing was closed"), "{seat}");
    assert!(!lock(&app.deep).owns(SEAT));
    assert_eq!(close_refusal(&app, SEAT), None);
}

#[test]
fn fix_question_once_then_hold_for_the_user() {
    let (_dir, app) = app();
    ready(&app);
    assert!(!ask_fix(&app, "m5", "cargo test: 1 failed", None));
    job_started(&app, "m5", "add the parser");
    assert!(ask_fix(&app, "m5", "cargo test: 1 failed", None));
    assert!(!ask_fix(&app, "m5", "cargo test: 1 failed", None));
    let ask = last_ask(&app);
    assert_eq!(ask.kind.as_deref(), Some(FIX_KIND));
    assert_eq!(ask.wait, Some(super::fix::FIX_WAIT_MS));
    assert!(ask.notes.contains("Failed checks: cargo test: 1 failed"));
    assert!(ask.notes.contains("- add the parser"));
    assert!(!ask.options.iter().any(|o| o.contains("for the user")));
    let hold = reply(&app, &ask.id, "give the job to another agent", "codex fits better.");
    assert!(block(answered(&app, &hold, None)));
    assert!(lines(&app, Who::Nixa).is_empty());
    assert!(lines(&app, Who::Info).iter().any(|l| l == &format!("Terminal m5 stopped: checks failed: deep brain {} held it: codex fits better. Nixa decides the next step.", ask.id)));
    assert!(!lock(&app.followup.jobs).contains_key("m5"));
}

#[test]
fn fix_that_repeats_an_earlier_message_is_held() {
    let (_dir, app) = app();
    ready(&app);
    job_started(&app, "m5", "Add the parser and run cargo test again.");
    assert!(ask_fix(&app, "m5", "cargo test failed", None));
    let ask = last_ask(&app);
    let same = reply(&app, &ask.id, "send a fix", "add the parser and run cargo test again");
    assert!(block(answered(&app, &same, None)));
    assert!(lines(&app, Who::Info).iter().any(|l| l.ends_with("checks failed: the deep brain's fix is the same as an earlier one. Nixa decides the next step.")));
}

#[test]
fn late_or_busy_fix_answers_send_nothing() {
    let (_dir, app) = app();
    ready(&app);
    job_started(&app, "m5", "first");
    assert!(ask_fix(&app, "m5", "failed", None));
    let ask = last_ask(&app);
    seen(&app, "m5", Some("working"));
    let fix = reply(&app, &ask.id, "send a fix", "Change the loop and run cargo test.");
    let busy = fix.clone();
    lock(&app.followup.targets).insert(ask.id.clone(), (super::Target::Fix(None), "m5".into()), ask.at);
    assert!(block(answered(&app, &busy, None)));
    assert!(lines(&app, Who::Info).iter().any(|l| l.ends_with("but its agent is busy again. Nixa checks it when it finishes.")));
    job_closed(&app, "m5", "checks failed");
    lock(&app.followup.targets).insert(ask.id.clone(), (super::Target::Fix(None), "m5".into()), ask.at);
    assert!(block(answered(&app, &fix, None)));
    assert!(lines(&app, Who::Info).iter().any(|l| l.ends_with("after its job ended. Nixa sent nothing.")));
    assert!(lines(&app, Who::Nixa).is_empty());
}

#[test]
fn same_work_points_to_the_holder_unless_the_user_says_anyway() {
    let (_dir, app) = app();
    let id = {
        let conn = nixa_store::open(&app.db_file).unwrap();
        let task = nixa_store::tasks::add_task(&conn, &TaskInput { title: "status bar".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() }, &Actor::new(AuthorKind::Nixa, "Nixa")).unwrap();
        let claimed = nixa_store::tasks::claim_task(&conn, task.id, &Actor::in_terminal(AuthorKind::Agent, "builder", "m2"), "").unwrap();
        assert!(claimed.ok, "{}", claimed.reason);
        task.id
    };
    seen(&app, "m2", Some("working"));
    lock(&app.announce.numbers).insert("m2".into(), 2);
    let auto = same_work_held(&app, &format!("Please finish task {id}"), Some("m1")).expect("held");
    assert!(auto.starts_with(&format!("Task #{id} is held by builder in m2")) && !auto.contains("Held for you"), "{auto}");
    assert!(lines(&app, Who::Info).iter().any(|l| l.starts_with("Same work: Task #")));
    assert!(same_work_held(&app, &format!("Please finish task {id}"), Some("m1")).is_some());
    let key = format!("{id}:m2");
    lock(&app.followup.same_held).insert(key.clone(), crate::util::now_ms().saturating_sub(super::same_work::SAME_WORK_MS));
    assert_eq!(same_work_held(&app, &format!("Please finish task {id}"), Some("m1")), None);
    assert!(lines(&app, Who::Info).iter().any(|l| l.ends_with("lets it through: the holder stayed busy past the time limit.")));
    assert!(!lock(&app.followup.same_held).contains_key(&key));
    seen(&app, "m2", None);
    assert_eq!(same_work_held(&app, &format!("Please finish task {id}"), Some("m1")), None);
    assert!(lines(&app, Who::Info).iter().any(|l| l.ends_with("lets it through: the holder is idle.")));
    seen(&app, "m2", Some("working"));
    assert_eq!(same_work_held(&app, &format!("Please finish task {id}"), Some("m2")), None);
    lock(&app.brain).last_user = Some(("send it anyway".into(), crate::util::now_ms()));
    assert_eq!(same_work_held(&app, &format!("Please finish task {id}"), Some("m1")), None);
}

#[test]
fn recovery_stops_at_the_limit() {
    let (_dir, app) = app();
    job_started(&app, "m5", "x");
    assert_eq!(may_recover(&app, "m5"), Some(1));
    assert_eq!(may_recover(&app, "m5"), Some(2));
    assert_eq!(may_recover(&app, "m5"), None);
    assert_eq!(take_stall(&app, StallKind::Same, "m5"), Some(1));
    assert_eq!(take_stall(&app, StallKind::Same, "m5"), None);
    job_closed(&app, "m5", "checks failed");
    assert_eq!(take_stall(&app, StallKind::Same, "m5"), Some(1));
}

#[test]
fn a_stop_is_said_once_per_hour() {
    let (_dir, app) = app();
    hold_job(&app, "m5", "used all rounds.");
    hold_job(&app, "m5", "used all rounds");
    let info = lines(&app, Who::Info);
    assert_eq!(info.iter().filter(|l| l.contains("stopped: used all rounds.")).count(), 2, "{info:?}");
    assert_eq!(info.iter().filter(|l| l.starts_with("Still: ")).count(), 1, "{info:?}");
}

#[test]
fn closed_jobs_carry_checks_and_the_last_reply() {
    let (_dir, app) = app();
    let ran = crate::review_flow::checks::ChecksOutcome { folder: "/tmp".into(), results: Vec::new(), failed: true, none: String::new() };
    lock(&app.review_flow.check).outcomes.insert("m5".into(), ran);
    note_reply(&app, "m5", "I fixed the parser.");
    let (close, changes) = close_of(&app, "m5", "checks failed");
    assert_eq!(close.checks, Some(crate::modes::scores::Checks::Failed));
    assert!(close.short.is_some() && !close.spent);
    assert_eq!(changes.report.as_deref(), Some("I fixed the parser."));
    assert_eq!(changes.outcome, "checks failed");
    let (close, changes) = close_of(&app, "m6", "done");
    assert_eq!(close.checks, Some(crate::modes::scores::Checks::None));
    assert_eq!(changes.report, None);
}

#[test]
fn plan_mode_holds_a_fix_and_second_opinions_keep_terminals_open() {
    let (_dir, app) = app();
    ready(&app);
    job_started(&app, "m5", "first");
    assert!(ask_fix(&app, "m5", "failed", None));
    let ask = last_ask(&app);
    let fix = reply(&app, &ask.id, "send a fix", "Change the loop and run cargo test.");
    block(answered(&app, &fix, None));
    assert!(!lines(&app, Who::Info).iter().any(|l| l.starts_with("Held for you (mode: plan)")));
    job_started(&app, "m6", "x");
    lock(&app.extras.opinions).begin("m6", crate::util::now_ms());
    assert_eq!(ask_close(&app, "m6", "x", None), None);
}

#[test]
fn a_no_status_report_moves_the_doing_task_to_review() {
    let (_dir, app) = app();
    seen(&app, "m6", None);
    let id = {
        let conn = nixa_store::open(&app.db_file).unwrap();
        let task = nixa_store::tasks::add_task(&conn, &TaskInput { title: "status bar".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() }, &Actor::new(AuthorKind::Nixa, "Nixa")).unwrap();
        let claimed = nixa_store::tasks::claim_task(&conn, task.id, &Actor::in_terminal(AuthorKind::Agent, "builder", "m6"), "").unwrap();
        assert!(claimed.ok, "{}", claimed.reason);
        assert_eq!(nixa_store::tasks::must_task(&conn, task.id).unwrap().status, TaskStatus::Doing);
        task.id
    };
    job_started(&app, "m6", "build the status bar");
    let handed_to = block(async { hand_report(&app, "m6", "Built the status bar. All checks passed.", "screen") });
    assert_eq!(handed_to, Handed::Review(id));
    let conn = nixa_store::open(&app.db_file).unwrap();
    assert_eq!(nixa_store::tasks::must_task(&conn, id).unwrap().status, TaskStatus::Review);
    let comments = nixa_store::tasks::task_comments(&conn, id).unwrap();
    assert!(comments.iter().any(|c| c.text.contains("Built the status bar.")), "{comments:?}");
    assert_eq!(lock(&app.review_flow.done).size(), 0);
}

#[test]
fn a_queued_report_is_dropped_when_a_new_job_starts() {
    let (_dir, app) = app();
    seen(&app, "m7", None);
    job_started(&app, "m7", "task A");
    note_reply(&app, "m7", "Report A done.");
    assert_eq!(hand_report(&app, "m7", "Report A done.", "screen A"), Handed::Queued);
    let event = lock(&app.review_flow.done).due(crate::util::now_ms() + crate::review_flow::DONE_GRACE_MS, usize::MAX, |_| true).pop().unwrap();
    assert!(still_fresh(&app, &event));
    job_started(&app, "m7", "task B");
    assert!(!still_fresh(&app, &event));
}

#[test]
fn a_queued_report_needs_the_same_report_still_on_screen() {
    let report = "Implemented the requested change.\nAll checks passed.";
    assert!(report_matches("Implemented the requested change.\nAll checks passed.\n❯\n", report));
    assert!(!report_matches("Started on the next part.\n❯\n", report));
}

#[test]
fn the_review_move_skips_a_task_that_is_no_longer_doing() {
    let (_dir, app) = app();
    let conn = nixa_store::open(&app.db_file).unwrap();
    let task = nixa_store::tasks::add_task(&conn, &TaskInput { title: "status bar".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() }, &Actor::new(AuthorKind::Nixa, "Nixa")).unwrap();
    assert!(nixa_store::tasks::claim_task(&conn, task.id, &Actor::in_terminal(AuthorKind::Agent, "builder", "m8"), "").unwrap().ok);
    nixa_store::tasks::set_status(&conn, task.id, TaskStatus::Cancelled, None, false).unwrap();
    assert!(!review_if_doing(&conn, task.id, "m8", "report").unwrap());
    assert_eq!(nixa_store::tasks::must_task(&conn, task.id).unwrap().status, TaskStatus::Cancelled);
}
