use super::*;
use crate::app::Seen;
use crate::review_flow::done::Source;
use nixa_store::tasks::{Actor, AuthorKind, TaskInput, TaskStatus, add_task, claim_task};
use serde_json::json;

fn setup() -> (tempfile::TempDir, Arc<App>, TermInfo, DoneEvent) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let folder = dir.path().join("project");
    std::fs::create_dir(&folder).unwrap();
    let conn = nixa_store::open(&app.db_file).unwrap();
    nixa_store::spaces::set_space(
        &conn,
        "shop",
        &nixa_store::spaces::SpacePatch {
            name: Some("Shop".into()),
            folder: Some(folder.display().to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    let term: TermInfo = serde_json::from_value(json!({
        "id": "m5", "name": "worker", "cwd": folder, "running": true, "status": "done",
        "rows": 24, "cols": 80, "created_ms": 1, "pid": 123, "seq": 3
    }))
    .unwrap();
    lock(&app.watch).world.insert(
        "m5".into(),
        Seen {
            info: term.clone(),
            since: now_ms(),
        },
    );
    let job = flow::job_started(&app, "m5", "Implement the requested change.");
    let event = DoneEvent {
        terminal: "m5".into(),
        agent: "worker".into(),
        cwd: term.cwd.clone(),
        text: "Completed the change.".into(),
        screen: String::new(),
        source: Source::Watch,
        at: job.sent_at,
        fallback: None,
    };
    drop(conn);
    (dir, app, term, event)
}

fn opt_in(app: &App) {
    auto_checks::set(&app.dir, true).unwrap();
    app.decision.set_choice("nixa").unwrap();
}

fn take(app: &App) -> Option<(u64, Completion)> {
    lock(&app.followup.recovery)
        .entries
        .get_mut("m5")?
        .next
        .take()
}

#[test]
fn an_automatic_check_is_queued_once_per_completion() {
    let (_dir, app, _term, event) = setup();
    queue(&app, &event, now_ms());
    assert!(take(&app).is_none());
    auto_checks::set(&app.dir, true).unwrap();
    queue(&app, &event, now_ms());
    assert!(take(&app).is_some());
    queue(&app, &event, now_ms());
    assert!(take(&app).is_none(), "duplicate completion");

    let conn = nixa_store::open(&app.db_file).unwrap();
    let task = add_task(
        &conn,
        &TaskInput {
            title: "Managed task".into(),
            status: Some(TaskStatus::Todo),
            ..Default::default()
        },
        &Actor::new(AuthorKind::User, "you"),
    )
    .unwrap();
    assert!(
        claim_task(
            &conn,
            task.id,
            &Actor::in_terminal(AuthorKind::Agent, "worker", "m5"),
            ""
        )
        .unwrap()
        .ok
    );
    let job = flow::job_started(&app, "m5", "Work on the board task.");
    queue(
        &app,
        &DoneEvent {
            at: job.sent_at,
            ..event
        },
        now_ms(),
    );
    assert!(take(&app).is_none(), "task-owned terminal");
}

#[test]
fn two_successful_completions_are_checked_in_the_same_job() {
    let (_dir, app, _term, first) = setup();
    opt_in(&app);
    let revision = lock(&app.followup.jobs)["m5"].revision;
    let passed = checks::ChecksOutcome {
        results: vec![checks::CheckResult {
            package: "demo".into(),
            script: "test".into(),
            ok: true,
            ms: 1,
            summary: "test passed".into(),
            tail: String::new(),
            timed_out: false,
        }],
        ..Default::default()
    };

    let second = DoneEvent {
        at: first.at + 1,
        ..first.clone()
    };
    for (index, event) in [&first, &second].into_iter().enumerate() {
        queue(&app, event, now_ms());
        let (_, completion) = take(&app).expect("each completion queues a check");
        assert_eq!(completion.revision, revision);
        let mut outcome = passed.clone();
        outcome.results[0].summary = format!("{} tests passed", index + 1);
        finish(&app, completion, false, Some(outcome));
        queue(&app, event, now_ms());
        assert!(
            take(&app).is_none(),
            "a replay must not queue another check"
        );
    }
    queue(&app, &first, now_ms());
    assert!(
        take(&app).is_none(),
        "an older completion must stay consumed"
    );
    assert_eq!(lock(&app.followup.jobs)["m5"].revision, revision);
    assert_eq!(
        lock(&app.chat)
            .list()
            .iter()
            .filter(|item| item.text.contains("project checks passed"))
            .count(),
        2
    );
}

#[test]
fn a_new_completion_supersedes_the_previous_attempt() {
    let (_dir, app, _term, first) = setup();
    opt_in(&app);
    queue(&app, &first, now_ms());
    let (_, active) = take(&app).unwrap();
    let second = DoneEvent {
        at: first.at + 1,
        ..first
    };
    queue(&app, &second, now_ms());

    assert!(!active.current(&app));
    again(&app, active, 60_000);
    let (_, next) = take(&app).unwrap();
    assert_eq!(next.event, second);
    assert!(next.current(&app));
}

#[test]
fn later_attempts_require_the_same_job_and_terminal() {
    let (_dir, app, term, event) = setup();
    opt_in(&app);
    queue(&app, &event, now_ms());
    let (_, mut completion) = take(&app).unwrap();
    completion.terminal = Some(completion.match_terminal(&app, term.clone()).unwrap());
    assert!(completion.match_terminal(&app, term.clone()).is_some());
    assert!(
        completion
            .match_terminal(
                &app,
                TermInfo {
                    draft: true,
                    ..term.clone()
                }
            )
            .is_none()
    );
    assert!(
        completion
            .match_terminal(
                &app,
                TermInfo {
                    seq: term.seq + 1,
                    ..term.clone()
                }
            )
            .is_none()
    );
    assert!(
        completion
            .match_terminal(
                &app,
                TermInfo {
                    status: Some(Status::Working),
                    ..term.clone()
                }
            )
            .is_none()
    );
    flow::job_started(&app, "m5", "Implement the requested change.");
    assert!(
        completion.match_terminal(&app, term).is_none(),
        "new job with the same text"
    );
}

#[test]
fn unread_changes_back_off_three_times_and_then_wait_for_the_user() {
    let (_dir, app, _term, event) = setup();
    opt_in(&app);
    queue(&app, &event, now_ms());
    for delay in [60_000, 120_000, 240_000] {
        let (_, completion) = take(&app).unwrap();
        let before = now_ms();
        finish(&app, completion, true, None);
        let after = now_ms();
        let due = lock(&app.followup.recovery).entries["m5"]
            .next
            .as_ref()
            .unwrap()
            .0;
        assert!((before + delay..=after + delay).contains(&due));
    }
    let (_, completion) = take(&app).unwrap();
    finish(&app, completion, true, None);
    assert!(take(&app).is_none());
    assert!(lock(&app.chat).list().iter().any(|item| {
        item.text
            .contains("project changes could not be read after three retries")
    }));
}

#[test]
fn a_pending_deep_fix_loses_authority_when_decision_changes() {
    let (_dir, app, term, event) = setup();
    opt_in(&app);
    app.decision.set_choice("deep").unwrap();
    let space = app.space_of_terminal("m5").unwrap();
    lock(&app.deep)
        .join("m90", "Reviewer", &space, Some("claude"), now_ms())
        .unwrap();
    queue(&app, &event, now_ms());
    let (_, mut completion) = take(&app).unwrap();
    completion.terminal = Some(completion.match_terminal(&app, term).unwrap());
    assert!(flow::ask_fix(
        &app,
        "m5",
        "a check failed",
        Some(completion)
    ));
    let mut ask = lock(&app.deep).asks.last().cloned().unwrap();
    ask.state = crate::deep::AskState::Answered;
    ask.pick = Some("send a fix".into());
    ask.why = Some("Diagnose the failing parser check.".into());
    app.decision.set_choice("nixa").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert!(runtime.block_on(flow::answered(&app, &ask, None)));
    assert_eq!(lock(&app.followup.jobs)["m5"].rounds, 0);
    assert!(
        lock(&app.chat)
            .list()
            .iter()
            .any(|item| item.text.contains("job or its recovery settings changed"))
    );
}
