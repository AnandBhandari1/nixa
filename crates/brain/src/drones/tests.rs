use super::*;
use serde_json::json;

fn task_subject(id: i64, status: &str, extra: &[(&str, Value)]) -> DroneSubject {
    let mut facts = FactMap::new();
    facts.insert("subject".into(), json!("task"));
    facts.insert("task.id".into(), json!(id));
    facts.insert("task.title".into(), json!("Fix build"));
    facts.insert("task.status".into(), json!(status));
    facts.insert("task.assigned".into(), json!(""));
    facts.insert("task.hasAssignee".into(), json!(false));
    facts.insert("task.hasAgent".into(), json!(false));
    facts.insert("task.agentTerminal".into(), json!(""));
    facts.insert("task.agentState".into(), json!(""));
    facts.insert("task.agentMinutesInState".into(), Value::Null);
    facts.insert("task.blockedOn".into(), json!(""));
    facts.insert("task.needsUser".into(), json!(false));
    facts.insert("task.minutesSinceUpdate".into(), json!(0));
    facts.insert("task.space".into(), json!("nixa"));
    facts.insert("task.spaceAutoStart".into(), json!(true));
    facts.insert("task.spaceHasRoom".into(), json!(true));
    facts.insert("task.closedClaims".into(), json!(0));
    for (k, v) in extra {
        facts.insert((*k).into(), v.clone());
    }
    DroneSubject {
        kind: "task",
        key: format!("task #{id}"),
        facts,
    }
}

fn term_subject(id: &str, state: &str, extra: &[(&str, Value)]) -> DroneSubject {
    let mut facts = FactMap::new();
    facts.insert("subject".into(), json!("terminal"));
    facts.insert("terminal.id".into(), json!(id));
    facts.insert("terminal.name".into(), json!("builder"));
    facts.insert("terminal.agent".into(), json!("codex"));
    facts.insert("terminal.where".into(), json!("nixa"));
    facts.insert("terminal.state".into(), json!(state));
    facts.insert("terminal.minutesInState".into(), json!(0));
    facts.insert("terminal.hasTask".into(), json!(false));
    facts.insert("terminal.hasClaim".into(), json!(false));
    facts.insert("terminal.nixaJob".into(), json!(false));
    facts.insert("terminal.minutesSinceSent".into(), Value::Null);
    facts.insert("terminal.startedAfterSend".into(), json!(false));
    for (k, v) in extra {
        facts.insert((*k).into(), v.clone());
    }
    DroneSubject {
        kind: "terminal",
        key: format!("terminal {id}"),
        facts,
    }
}

#[test]
fn default_tree_is_valid_and_catches_stalls() {
    let tree = default_drone_tree();
    check_tree(&tree).unwrap();

    // Closed file claims
    let w = walk_subject(
        &tree,
        &task_subject(1, "doing", &[("task.closedClaims", json!(2))]),
    );
    assert_eq!(w.end, "report");
    assert_eq!(w.step.as_deref(), Some("t-claims-report"));
    assert!(w.text.unwrap().contains("2 file claim(s)"));

    // Doing with finished agent for 2+ min
    let w = walk_subject(
        &tree,
        &task_subject(
            2,
            "doing",
            &[
                ("task.hasAgent", json!(true)),
                ("task.agentTerminal", json!("nixa-codex-1")),
                ("task.agentState", json!("done")),
                ("task.agentMinutesInState", json!(3)),
            ],
        ),
    );
    assert_eq!(w.step.as_deref(), Some("a-finished-report"));

    // Inbox unassigned for 5+ min
    let w = walk_subject(
        &tree,
        &task_subject(3, "inbox", &[("task.minutesSinceUpdate", json!(6))]),
    );
    assert_eq!(w.step.as_deref(), Some("b-inbox-report"));

    // Working task is quiet
    let w = walk_subject(
        &tree,
        &task_subject(4, "doing", &[("task.agentState", json!("working"))]),
    );
    assert_eq!(w.end, "wait");

    // Idle Nixa job terminal for 15+ min
    let w = walk_subject(
        &tree,
        &term_subject(
            "nixa-codex-2",
            "done",
            &[
                ("terminal.nixaJob", json!(true)),
                ("terminal.minutesInState", json!(20)),
            ],
        ),
    );
    assert_eq!(w.step.as_deref(), Some("d-idle-report"));
}

#[test]
fn validation_rejects_loops_and_unknown_facts() {
    let mut tree = default_drone_tree();
    tree.steps.insert(
        "fine".into(),
        DroneStep::If {
            label: None,
            off: None,
            test: t("subject"),
            then: "subject".into(),
            otherwise: "subject".into(),
        },
    );
    assert!(check_tree(&tree).unwrap_err().contains("loop"));

    let mut bad_fact = default_drone_tree();
    bad_fact.steps.insert(
        "x".into(),
        DroneStep::Report {
            label: None,
            off: None,
            text: "Bad {task.madeUp}".into(),
        },
    );
    assert!(check_tree(&bad_fact).unwrap_err().contains("unknown slot"));
}

#[test]
fn store_edits_undo_and_finding_reminders() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = DroneStore::open(dir.path());
    assert!(dir.path().join(TREE_FILE).exists());

    // Turn off a step and verify walk stops with "off"
    store
        .set_on(false, "tester", Some("b-inbox-report"), 1000)
        .unwrap();
    let sub = task_subject(7, "inbox", &[("task.minutesSinceUpdate", json!(10))]);
    let walks = store.dry_run(std::slice::from_ref(&sub), None).unwrap();
    assert_eq!(walks[0].end, "off");

    // Undo restores it
    store.undo("tester", 2000).unwrap();
    let walks = store.dry_run(std::slice::from_ref(&sub), None).unwrap();
    assert_eq!(walks[0].end, "report");

    // Timing update
    store.set_timing("tester", Some(5), Some(20), 3000).unwrap();
    assert_eq!(store.every_minutes(), 5);
    assert_eq!(store.remind_minutes(), 20);
    assert_eq!(store.history(10).len(), 3);

    // First report fires, second before remindAfter is silent, third at remindAfter reminds once
    let (_, msg1) = store.record_walk(std::slice::from_ref(&sub), 10_000, true);
    assert!(msg1.unwrap().contains("New:"));
    let (_, msg2) = store.record_walk(
        std::slice::from_ref(&sub),
        10_000 + 5 * DRONE_MINUTE_MS,
        true,
    );
    assert!(msg2.is_none());
    let sub = task_subject(
        7,
        "inbox",
        &[
            ("task.minutesSinceUpdate", json!(31)),
            ("task.title", json!("Check the build")),
        ],
    );
    let (_, msg3) = store.record_walk(
        std::slice::from_ref(&sub),
        10_000 + 21 * DRONE_MINUTE_MS,
        true,
    );
    let reminder = msg3.unwrap();
    assert!(reminder.contains("Reminder, no change for 20 minutes:"));
    assert!(!reminder.contains("New:"));
    assert!(reminder.contains("Check the build"));
    let (_, msg4) = store.record_walk(&[sub], 10_000 + 60 * DRONE_MINUTE_MS, true);
    assert!(msg4.is_none());

    // Resolved clears findings
    let (_, _) = store.record_walk(&[], 10_000 + 65 * DRONE_MINUTE_MS, true);
    assert!(store.findings.is_empty());
}

#[test]
fn explicit_null_test_survives_json_round_trip() {
    let step: DroneStep = serde_json::from_value(json!({
        "kind": "if",
        "fact": "terminal.minutesSinceSent",
        "is": null,
        "then": "missing",
        "else": "present"
    }))
    .unwrap();
    let DroneStep::If { test, .. } = &step else {
        panic!("expected an if step");
    };
    assert_eq!(test.is, Some(Value::Null));
    assert!(passes(test, &FactMap::new()));
    assert!(!passes(
        test,
        &FactMap::from([("terminal.minutesSinceSent".into(), json!(0))])
    ));
    assert_eq!(
        serde_json::from_value::<DroneStep>(json!(step)).unwrap(),
        step
    );
}

#[test]
fn unavailable_edit_log_leaves_the_tree_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = DroneStore::open(dir.path());
    let original = fs::read(&store.file).unwrap();
    fs::create_dir(&store.edits_file).unwrap();
    let error = store.set_on(false, "tester", None, 1).unwrap_err();
    assert!(error.contains("edit log"), "{error}");
    assert!(store.tree.on);
    assert_eq!(fs::read(&store.file).unwrap(), original);
}

fn response_text(reply: &Value) -> &str {
    reply["text"].as_str().expect("text in response")
}

#[tokio::test]
async fn http_routes_read_dry_run_toggle_timing_and_undo() {
    use nixa_store::tasks::{Actor, AuthorKind, TaskInput, add_task};

    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let conn = nixa_store::open(&app.db_file).unwrap();
    let task = add_task(
        &conn,
        &TaskInput {
            title: "Check the build".into(),
            ..TaskInput::default()
        },
        &Actor::new(AuthorKind::User, "tester"),
    )
    .unwrap();
    let locked_folder = dir.path().join("locked");
    nixa_store::spaces::set_space(
        &conn,
        "locked",
        &nixa_store::spaces::SpacePatch {
            name: Some("Locked project".into()),
            folder: Some(locked_folder.display().to_string()),
            locked: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    add_task(
        &conn,
        &TaskInput {
            title: "Task in a locked space".into(),
            space_id: "locked".into(),
            ..TaskInput::default()
        },
        &Actor::new(AuthorKind::User, "tester"),
    )
    .unwrap();
    for (id, running, cwd) in [
        ("m1", true, locked_folder.as_path()),
        ("m2", false, dir.path()),
    ] {
        let info = serde_json::from_value(json!({
            "id": id, "name": "test shell", "cwd": cwd, "running": running,
            "rows": 24, "cols": 80, "created_ms": 0
        }))
        .unwrap();
        lock(&app.watch)
            .world
            .insert(id.into(), crate::app::Seen { info, since: 0 });
    }
    let tree_file = dir.path().join(TREE_FILE);
    let original_tree = fs::read(&tree_file).unwrap();
    let router = crate::server::router(app.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}/drones", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut token = reqwest::header::HeaderMap::new();
    token.insert(crate::window_key::HEADER, app.window_token.parse().unwrap());
    let client = reqwest::Client::builder()
        .no_proxy()
        .default_headers(token)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let get = |action: &str| client.get(format!("{base}{action}"));
    let post = |action: &str, body: Value| client.post(format!("{base}/{action}")).json(&body);
    let status = |code| reqwest::StatusCode::from_u16(code).unwrap();

    let initial = http_json(get(""), status(200)).await;
    assert_eq!(initial["drones"]["on"], true);
    assert_eq!(initial["drones"]["tree"]["start"], "subject");
    assert_eq!(initial["drones"]["findings"], json!([]));

    let facts = http_json(get("/facts"), status(200)).await;
    let facts: Value = serde_json::from_str(response_text(&facts)).unwrap();
    assert!(facts["facts"]["task.id"].is_string());

    let dry = http_json(get("/dry_run"), status(200)).await;
    let dry: Value = serde_json::from_str(response_text(&dry)).unwrap();
    assert_eq!(dry["walked"], 1);
    let candidate = http_json(
        post(
            "dry_run",
            json!({"id": "subject", "step": {"kind": "report", "text": "Task {task.id}"}}),
        ),
        status(200),
    )
    .await;
    let candidate: Value = serde_json::from_str(response_text(&candidate)).unwrap();
    assert_eq!(candidate["results"][0]["text"], format!("Task {}", task.id));
    assert_eq!(fs::read(&tree_file).unwrap(), original_tree);
    assert!(!dir.path().join(EDITS_FILE).exists());
    assert_eq!(
        http_json(get(""), status(200)).await["drones"],
        initial["drones"]
    );

    http_json(get("/off"), status(405)).await;
    http_json(post("off", json!({"id": 7})), status(400)).await;
    http_json(post("off", json!([])), status(400)).await;
    http_json(
        client.post(format!("{base}/off")).body("invalid"),
        status(400),
    )
    .await;
    assert_eq!(http_json(get(""), status(200)).await["drones"]["on"], true);

    http_json(post("off", json!({})), status(200)).await;
    assert_eq!(http_json(get(""), status(200)).await["drones"]["on"], false);
    http_json(post("on", json!({})), status(200)).await;
    assert_eq!(http_json(get(""), status(200)).await["drones"]["on"], true);

    let error = http_json(post("timing", json!({"every": 0})), status(400)).await;
    assert!(error["error"].as_str().unwrap().contains("from 1 to 60"));
    http_json(
        post("timing", json!({"every": 5, "remind": 30})),
        status(200),
    )
    .await;
    let timed = http_json(get(""), status(200)).await;
    assert_eq!(timed["drones"]["every"], 5);
    assert_eq!(timed["drones"]["remindAfter"], 30);

    let undo = http_json(post("undo", json!({})), status(200)).await;
    assert!(response_text(&undo).contains("Undid timing edit"));
    assert_eq!(
        http_json(get(""), status(200)).await["drones"],
        initial["drones"]
    );
    server.abort();
}

async fn http_json(request: reqwest::RequestBuilder, status: reqwest::StatusCode) -> Value {
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), status, "{}", response.url());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["ok"], status.is_success(), "{body}");
    body
}

#[tokio::test]
async fn nixa_drones_dispatches_through_the_brain_tool_registry() {
    use crate::turns::ToolCall;

    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let specs = crate::tools::specs(&app);
    assert!(specs.iter().any(|spec| spec.name == tools::NAME));
    let call = |args: Value| {
        let app = app.clone();
        async move {
            crate::tools::call(
                &app,
                &ToolCall {
                    id: "test-call".into(),
                    name: "nixa_drones".into(),
                    args: args.as_object().unwrap().clone(),
                },
            )
            .await
        }
    };

    let (reply, is_error) = call(json!({"action": "off", "id": "d-idle"})).await;
    assert!(!is_error, "{reply}");
    assert!(reply.contains("Step d-idle turned off"));

    let (reply, is_error) = call(json!({"action": "read"})).await;
    assert!(!is_error, "{reply}");
    let view: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(view["tree"]["steps"]["d-idle"]["off"], true);
    assert_eq!(view["on"], true);

    for args in [
        json!({"action": "missing"}),
        json!({"action": false}),
        json!({"action": "off", "id": " "}),
    ] {
        let (reply, is_error) = call(args).await;
        assert!(is_error, "{reply}");
        assert!(reply.starts_with("Tool error:"), "{reply}");
    }
    assert!(lock(&app.drones.store).tree.on);
}

#[test]
fn a_parked_blocked_task_is_told_once_and_stays_quiet_until_it_changes() {
    assert_eq!(parked_note(&[(1, "Placed by the deep brain"), (2, "Kept parked at the owner's request.")]), Some(2));
    assert_eq!(parked_note(&[(1, "Parked for now."), (2, "No longer parked, start it.")]), None);
    assert_eq!(parked_note(&[(1, "Parked for now."), (2, "Auto-start queued.")]), Some(1));
    assert_eq!(parked_note(&[(1, "Waiting on the API key.")]), None);
    let task_108 = [
        (1, "Placed by the deep brain d250: space nixa, agent a5, status blocked."),
        (2, "Kept parked at the owner's request, separate from the current one-phone camera work. The deep brain confirms this should stay parked until the owner unblocks it."),
        (3, "Deep brain d300: keep parked. Not unblocked, not given away, not cancelled. The owner parked this future idea until they unblock it."),
        (4, "Nixa drone check: still blocked on purpose. The deep brain said to keep this task parked, so I'm leaving it blocked and will not start it. Drones stay quiet until its status changes."),
    ];
    assert_eq!(parked_note(&task_108), Some(2));
    assert_eq!(parked_note(&task_108[..2]), parked_note(&task_108));
    assert_eq!(parked_note(&[(1, "Keep #108 parked."), (2, "It stays parked.")]), Some(1));
    assert_eq!(parked_note(&[(1, "Parked."), (2, "Unpark it."), (3, "Parked again.")]), Some(3));

    let dir = tempfile::tempdir().unwrap();
    let mut store = DroneStore::open(dir.path());
    let blocked = |parked: bool, mark: &str| {
        task_subject(
            108,
            "blocked",
            &[
                ("task.blockedOn", json!("other")),
                ("task.minutesSinceUpdate", json!(90)),
                ("task.parked", json!(parked)),
                ("task.parkedMark", json!(mark)),
            ],
        )
    };
    let start = 10_000;
    let (_, msg) = store.record_walk(&[blocked(true, "blocked|other|2")], start, true);
    let msg = msg.unwrap();
    assert!(msg.contains("is blocked on other") && msg.contains("parked on purpose"));
    for hour in 1..4 {
        let (_, msg) = store.record_walk(&[blocked(true, "blocked|other|2")], start + hour * 60 * DRONE_MINUTE_MS, true);
        assert!(msg.is_none(), "no reminder for a parked task");
    }
    let mut store = DroneStore::open(dir.path());
    let (_, msg) = store.record_walk(&[blocked(true, "blocked|other|2")], start, true);
    assert!(msg.is_none(), "quiet after a brain restart");
    let (_, msg) = store.record_walk(&[blocked(true, "blocked|other|5")], start, true);
    assert!(msg.unwrap().contains("parked on purpose"), "told again once after the parking note changes");

    let (_, msg) = store.record_walk(&[blocked(false, "")], start, true);
    let msg = msg.unwrap();
    assert!(msg.contains("is blocked on other") && !msg.contains("parked"));
    let (_, msg) = store.record_walk(&[blocked(false, "")], start + 20 * DRONE_MINUTE_MS, true);
    assert!(msg.unwrap().contains("Reminder"), "a real blocker still gets its reminder");
}
