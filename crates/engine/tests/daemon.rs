use nixa_protocol::{
    ChildReport, Client, CreateSpec, ErrorCode, Event, Key, ReadSource, Reply, Request, Status,
    TermId, TermInfo, WaitFor, WaitOutcome, connect,
};
use nixa_protocol::request::EndedSnapshot;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: PathBuf,
    socket: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("me-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn start_in(dir: &Path) -> Daemon {
    let socket = dir.join("engine.sock");
    let child = Command::new(env!("CARGO_BIN_EXE_nixa-engine"))
        .arg("--socket")
        .arg(&socket)
        .env("SHELL", "/bin/sh")
        .env_remove("NIXA_TERM_ID")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let daemon = Daemon {
        child,
        dir: dir.to_path_buf(),
        socket,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(mut c) = connect(&daemon.socket)
            && matches!(c.call(&Request::Ping), Ok(Reply::Pong { .. }))
        {
            return daemon;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("engine did not start");
}

fn client(d: &Daemon) -> Client {
    connect(&d.socket).unwrap()
}

fn call(d: &Daemon, req: Request) -> Reply {
    client(d).request(&req).unwrap()
}

fn create(d: &Daemon, spec: CreateSpec) -> TermInfo {
    match call(d, Request::Create { spec }) {
        Reply::Term { term } => term,
        other => panic!("create failed: {other:?}"),
    }
}

fn sh(d: &Daemon) -> TermInfo {
    create(
        d,
        CreateSpec {
            cmd: vec!["/bin/sh".to_string()],
            cwd: Some(d.dir.display().to_string()),
            ..CreateSpec::default()
        },
    )
}

fn get(d: &Daemon, id: &TermId) -> TermInfo {
    match call(d, Request::Get { id: id.clone() }) {
        Reply::Term { term } => term,
        other => panic!("get failed: {other:?}"),
    }
}

fn send(d: &Daemon, id: &TermId, text: &str) -> Reply {
    call(
        d,
        Request::Send {
            id: id.clone(),
            text: text.to_string(),
            submit: true,
            timeout_ms: Some(3000),
            force: false,
        },
    )
}

fn wait(d: &Daemon, id: &TermId, until: WaitFor, ms: u64) -> (WaitOutcome, Option<String>) {
    match call(
        d,
        Request::Wait {
            id: id.clone(),
            until,
            timeout_ms: Some(ms),
        },
    ) {
        Reply::Waited { result } => (result.outcome, result.matched),
        other => panic!("wait failed: {other:?}"),
    }
}

fn wait_text(d: &Daemon, id: &TermId, pattern: &str) -> (WaitOutcome, Option<String>) {
    wait(
        d,
        id,
        WaitFor::Text {
            pattern: pattern.to_string(),
            regex: true,
        },
        8000,
    )
}

fn read(d: &Daemon, id: &TermId, source: ReadSource) -> String {
    match call(
        d,
        Request::Read {
            id: id.clone(),
            source,
            lines: None,
        },
    ) {
        Reply::Text { text, .. } => text,
        other => panic!("read failed: {other:?}"),
    }
}

fn events(d: &Daemon, ids: Vec<TermId>) -> Receiver<Event> {
    let c = client(d);
    let (tx, rx) = channel();
    thread::spawn(move || {
        let _ = c.subscribe(ids, false, |e| tx.send(e).is_ok());
    });
    thread::sleep(Duration::from_millis(200));
    rx
}

fn expect_event(rx: &Receiver<Event>, what: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(8);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(e) if what(&e) => return e,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    panic!("event not seen");
}

#[test]
fn shell_echo_read_kill() {
    let dir = test_dir("echo");
    let d = start_in(&dir);
    let mode = fs::metadata(&d.socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let t = sh(&d);
    assert!(t.id.as_str().contains("-sh-") || t.id.as_str().contains("-shell-"));
    assert!(t.running);
    assert!(t.pid.is_some());
    assert_eq!((t.rows, t.cols), (24, 80));

    assert!(matches!(send(&d, &t.id, "echo hi"), Reply::Sent { forced: false, .. }));
    assert_eq!(wait_text(&d, &t.id, "^hi$").0, WaitOutcome::Met);
    let screen = read(&d, &t.id, ReadSource::Screen);
    assert!(screen.lines().any(|l| l.trim() == "hi"), "{screen}");

    send(&d, &t.id, "echo \"$NIXA_TERM_ID-$TERM_PROGRAM\"");
    let want = format!("{}-nixa", t.id);
    let (outcome, matched) = wait_text(&d, &t.id, &format!("^{want}$"));
    assert_eq!(outcome, WaitOutcome::Met);
    assert_eq!(matched.as_deref(), Some(want.as_str()));
    assert!(read(&d, &t.id, ReadSource::History).contains(&want));

    let seq = match call(
        &d,
        Request::Snapshot {
            id: t.id.clone(),
            scroll: 0,
            since_seq: None,
        },
    ) {
        Reply::Snapshot { snapshot } => {
            assert_eq!(snapshot.cells.len(), 24 * 80);
            assert!((0..24).any(|r| snapshot.row_text(r).trim() == "hi"));
            snapshot.seq
        }
        other => panic!("{other:?}"),
    };
    let again = call(
        &d,
        Request::Snapshot {
            id: t.id.clone(),
            scroll: 0,
            since_seq: Some(seq),
        },
    );
    assert!(matches!(again, Reply::Unchanged { .. }), "{again:?}");

    assert_eq!(call(&d, Request::Kill { id: t.id.clone() }), Reply::Ok);
    let after = get(&d, &t.id);
    assert!(!after.running);
    assert!(after.exit_code.is_some());
    assert!(matches!(send(&d, &t.id, "echo no"), Reply::Error { code: ErrorCode::Exited, .. }));

    assert_eq!(call(&d, Request::Remove { id: t.id.clone() }), Reply::Ok);
    assert!(matches!(
        call(&d, Request::Get { id: t.id.clone() }),
        Reply::Error { code: ErrorCode::NotFound, .. }
    ));
    assert_eq!(call(&d, Request::Shutdown), Reply::Ok);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn status_from_program_and_request() {
    let dir = test_dir("status");
    let d = start_in(&dir);
    let t = sh(&d);
    let rx = events(&d, vec![t.id.clone()]);

    send(&d, &t.id, "printf '\\033]7421;status=blocked;note=need a yes\\007'");
    assert_eq!(
        wait(&d, &t.id, WaitFor::Status { status: Status::Blocked }, 8000).0,
        WaitOutcome::Met
    );
    let info = get(&d, &t.id);
    assert_eq!(info.status, Some(Status::Blocked));
    assert_eq!(info.status_note.as_deref(), Some("need a yes"));
    expect_event(&rx, |e| matches!(e, Event::Status { status: Some(Status::Blocked), .. }));

    send(&d, &t.id, "printf '\\033]7421;status=working\\007'");
    assert_eq!(
        wait(&d, &t.id, WaitFor::Status { status: Status::Working }, 8000).0,
        WaitOutcome::Met
    );
    assert_eq!(wait(&d, &t.id, WaitFor::Idle, 300).0, WaitOutcome::Timeout);

    let set = call(
        &d,
        Request::StatusSet {
            id: t.id.clone(),
            status: Some(Status::Done),
            note: None,
        },
    );
    assert_eq!(set, Reply::Ok);
    assert_eq!(wait(&d, &t.id, WaitFor::Idle, 3000).0, WaitOutcome::Met);
    assert_eq!(get(&d, &t.id).status, Some(Status::Done));
    let _ = call(&d, Request::Shutdown);
}

#[test]
fn parent_gets_child_notes() {
    let dir = test_dir("child");
    let d = start_in(&dir);
    let parent = sh(&d);
    let child = create(
        &d,
        CreateSpec {
            cmd: vec!["/bin/sh".to_string()],
            parent: Some(parent.id.clone()),
            name: Some("helper".to_string()),
            ..CreateSpec::default()
        },
    );
    assert_eq!(child.parent.as_ref(), Some(&parent.id));
    assert_eq!(get(&d, &parent.id).children, vec![child.id.clone()]);
    let rx = events(&d, vec![parent.id.clone()]);

    call(
        &d,
        Request::StatusSet {
            id: child.id.clone(),
            status: Some(Status::Blocked),
            note: Some("pick a file".to_string()),
        },
    );
    let note = expect_event(&rx, |e| matches!(e, Event::ChildNote { exited: false, .. }));
    if let Event::ChildNote { child: c, status, note, .. } = note {
        assert_eq!(c, child.id);
        assert_eq!(status, Some(Status::Blocked));
        assert!(note.contains("helper") && note.contains("pick a file"), "{note}");
    }

    assert_eq!(wait(&d, &parent.id, WaitFor::Children, 300).0, WaitOutcome::Timeout);
    send(&d, &child.id, "printf 'last output\\n'; exit 4");
    expect_event(&rx, |e| matches!(e, Event::ChildNote { exited: true, .. }));
    assert_eq!(wait(&d, &parent.id, WaitFor::Children, 3000).0, WaitOutcome::Met);
    assert_eq!(call(&d, Request::Get { id: child.id.clone() }), Reply::not_found(&child.id));
    let output = call(&d, Request::ChildOutput { id: parent.id.clone(), child: child.id });
    assert!(matches!(output, Reply::Text { text, .. } if text.contains("last output")));

    match call(
        &d,
        Request::List {
            parent: Some(parent.id.clone()),
            label: None,
        },
    ) {
        Reply::Terms { terms } => assert!(terms.is_empty()),
        other => panic!("{other:?}"),
    }
    let _ = call(&d, Request::Shutdown);
}

fn wait_and_read_child_reports(
    d: &Daemon,
    id: &TermId,
    until: WaitFor,
    complete: impl FnOnce(),
) -> (WaitOutcome, Option<String>, Vec<ChildReport>) {
    thread::scope(|scope| {
        let (started, ready) = channel();
        let waiter = scope.spawn(move || {
            let mut client = client(d);
            started.send(()).unwrap();
            let reply = client
                .call(&Request::Wait {
                    id: id.clone(),
                    until,
                    timeout_ms: Some(3000),
                })
                .unwrap();
            let reports = client
                .call(&Request::ChildReports { id: id.clone(), reader: None })
                .unwrap();
            let Reply::Waited { result } = reply else {
                panic!("wait failed: {reply:?}")
            };
            let Reply::ChildReports { reports } = reports else {
                panic!("cannot read reports: {reports:?}")
            };
            (result.outcome, result.matched, reports)
        });
        ready.recv().unwrap();
        complete();
        waiter.join().unwrap()
    })
}

#[test]
fn child_waits_settle_reports_without_writing_to_the_parent() {
    let d = start_in(&test_dir("child-report"));
    let parent = sh(&d);
    let start_child = || {
        create(
            &d,
            CreateSpec {
                cmd: vec!["/bin/sh".into()],
                parent: Some(parent.id.clone()),
                ..CreateSpec::default()
            },
        )
    };
    let first = start_child();
    let second = start_child();
    assert_eq!(
        wait(&d, &parent.id, WaitFor::ChildAny, 50).0,
        WaitOutcome::Timeout
    );

    let (outcome, matched, reports) =
        wait_and_read_child_reports(&d, &parent.id, WaitFor::ChildAny, || {
            assert_eq!(
                call(
                    &d,
                    Request::StatusSet {
                        id: first.id.clone(),
                        status: Some(Status::Done),
                        note: Some("child finished\nverified report".into()),
                    }
                ),
                Reply::Ok
            );
        });
    assert_eq!(
        (outcome, matched),
        (WaitOutcome::Met, Some(first.id.to_string()))
    );
    assert!(
        reports.iter().any(|report| {
            report.child == first.id
                && report.status == Some(Status::Done)
                && !report.exited
                && report.note.contains("child finished verified report")
        }),
        "{reports:?}"
    );

    assert_eq!(
        wait(&d, &parent.id, WaitFor::ChildAll, 50).0,
        WaitOutcome::Timeout
    );
    let (outcome, _, reports) =
        wait_and_read_child_reports(&d, &parent.id, WaitFor::ChildAll, || {
            assert!(matches!(send(&d, &second.id, "exit 4"), Reply::Sent { .. }));
        });
    assert_eq!(outcome, WaitOutcome::Met);
    assert!(
        reports.iter().any(|report| {
            report.child == second.id && report.exited && report.note.contains("ended with code 4")
        }),
        "{reports:?}"
    );
    assert!(get(&d, &first.id).running);
    assert!(!read(&d, &parent.id, ReadSource::History).contains("verified report"));
    let _ = call(&d, Request::Shutdown);
}

#[test]
fn immediately_exiting_child_is_reported() {
    let d = start_in(&test_dir("child-fast-exit"));
    let parent = sh(&d);
    let child = create(
        &d,
        CreateSpec {
            cmd: vec!["/bin/sh".into(), "-c".into(), "exit 3".into()],
            parent: Some(parent.id.clone()),
            ..CreateSpec::default()
        },
    );
    let (outcome, matched, reports) =
        wait_and_read_child_reports(&d, &parent.id, WaitFor::ChildAny, || {});
    assert_eq!(
        (outcome, matched),
        (WaitOutcome::Met, Some(child.id.to_string()))
    );
    assert!(
        reports.iter().any(|report| {
            report.child == child.id && report.exited && report.note.contains("ended with code 3")
        }),
        "{reports:?}"
    );
    let _ = call(&d, Request::Shutdown);
}

#[test]
fn remove_ended_refuses_live_and_changed_terminals_and_emits_removed() {
    let dir = test_dir("remove-ended");
    let daemon = start_in(&dir);
    let term = sh(&daemon);
    let id = &term.id;
    let snapshot = |term: &TermInfo| EndedSnapshot {
        space_label: term.labels.get("nixa.space").cloned(),
        window_space_label: term.labels.get("space").cloned(),
        cwd: term.cwd.clone(),
        seq: term.seq,
        exit_code: term.exit_code.unwrap_or(0),
    };
    let remove = |expected| call(&daemon, Request::RemoveEnded { id: id.clone(), expected });

    assert!(matches!(remove(snapshot(&term)), Reply::Error { code: ErrorCode::BadRequest, .. }));
    assert!(get(&daemon, id).running);
    send(&daemon, id, "printf 'final report\\n'; exit 7");
    assert_eq!(wait(&daemon, id, WaitFor::Exit, 5000).0, WaitOutcome::Met);
    let ended = get(&daemon, id);
    assert_eq!(ended.exit_code, Some(7));
    assert!(read(&daemon, id, ReadSource::History).lines().any(|line| line.trim() == "final report"));

    let wrong_output = EndedSnapshot { seq: ended.seq + 1, ..snapshot(&ended) };
    assert!(matches!(remove(wrong_output), Reply::Error { code: ErrorCode::BadRequest, .. }));
    assert_eq!(call(&daemon, Request::LabelSet { id: id.clone(), key: "nixa.space".into(), value: Some("other".into()) }), Reply::Ok);
    assert!(matches!(remove(snapshot(&ended)), Reply::Error { code: ErrorCode::BadRequest, .. }));
    let relabeled = get(&daemon, id);
    assert_eq!(call(&daemon, Request::LabelSet { id: id.clone(), key: "space".into(), value: Some("moved".into()) }), Reply::Ok);
    assert!(matches!(remove(snapshot(&relabeled)), Reply::Error { code: ErrorCode::BadRequest, .. }));
    let fresh = get(&daemon, id);
    let rx = events(&daemon, vec![id.clone()]);
    assert_eq!(remove(snapshot(&fresh)), Reply::Ok);
    expect_event(&rx, |event| matches!(event, Event::Removed { id: removed } if removed == id));
    assert!(matches!(remove(snapshot(&fresh)), Reply::Error { code: ErrorCode::NotFound, .. }));
}

#[test]
fn a_working_terminal_whose_program_ended_is_no_longer_working_and_can_be_closed() {
    let daemon = start_in(&test_dir("ended-working"));
    let term = sh(&daemon);
    let id = &term.id;
    let set = call(&daemon, Request::StatusSet { id: id.clone(), status: Some(Status::Working), note: None });
    assert_eq!(set, Reply::Ok);
    send(&daemon, id, "exit 1");
    assert_eq!(wait(&daemon, id, WaitFor::Exit, 5000).0, WaitOutcome::Met);
    let ended = get(&daemon, id);
    assert_eq!(ended.status, Some(Status::Error));
    let expected = EndedSnapshot {
        space_label: ended.labels.get("nixa.space").cloned(),
        window_space_label: ended.labels.get("space").cloned(),
        cwd: ended.cwd.clone(),
        seq: ended.seq,
        exit_code: ended.exit_code.unwrap_or(0),
    };
    assert_eq!(call(&daemon, Request::RemoveEnded { id: id.clone(), expected }), Reply::Ok);
    let _ = call(&daemon, Request::Shutdown);
}

#[test]
fn parent_kill_and_remove_never_end_the_child() {
    let d = start_in(&test_dir("child-lifetime"));
    let parent = sh(&d);
    let child = create(&d, CreateSpec {
        cmd: vec!["/bin/sh".into()],
        parent: Some(parent.id.clone()),
        ..CreateSpec::default()
    });
    assert!(matches!(call(&d, Request::Kill { id: parent.id.clone() }), Reply::Ok));
    assert_eq!(wait(&d, &parent.id, WaitFor::Exit, 3000).0, WaitOutcome::Met);
    let info = get(&d, &child.id);
    assert!(info.running);
    assert_eq!(info.parent.as_ref(), Some(&parent.id));
    assert!(matches!(call(&d, Request::Remove { id: parent.id }), Reply::Ok));
    let info = get(&d, &child.id);
    assert!(info.running);
    assert!(info.parent.is_none());
    assert!(matches!(call(&d, Request::Kill { id: child.id.clone() }), Reply::Ok));
    assert_eq!(wait(&d, &child.id, WaitFor::Exit, 3000).0, WaitOutcome::Met);
    let _ = call(&d, Request::Shutdown);
}

#[test]
fn send_waits_for_draft() {
    let dir = test_dir("draft");
    let d = start_in(&dir);
    let t = sh(&d);
    let input = |text: &str| {
        call(
            &d,
            Request::Input {
                id: t.id.clone(),
                text: text.to_string(),
            },
        )
    };
    assert_eq!(input("abc"), Reply::Ok);
    assert!(get(&d, &t.id).draft);
    let busy = call(
        &d,
        Request::Send {
            id: t.id.clone(),
            text: "echo x".to_string(),
            submit: true,
            timeout_ms: Some(300),
            force: false,
        },
    );
    assert!(matches!(busy, Reply::Error { code: ErrorCode::DraftBusy, .. }), "{busy:?}");

    let socket = d.socket.clone();
    let id = t.id.clone();
    let clearer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(400));
        connect(&socket)
            .unwrap()
            .call(&Request::Input {
                id,
                text: "\x15".to_string(),
            })
            .unwrap();
    });
    match send(&d, &t.id, "echo after-draft") {
        Reply::Sent { waited_ms, forced, .. } => {
            assert!(waited_ms >= 300, "{waited_ms}");
            assert!(!forced);
        }
        other => panic!("{other:?}"),
    }
    clearer.join().unwrap();
    assert_eq!(wait_text(&d, &t.id, "^after-draft$").0, WaitOutcome::Met);

    input("typing");
    let forced = call(
        &d,
        Request::Send {
            id: t.id.clone(),
            text: "".to_string(),
            submit: false,
            timeout_ms: Some(100),
            force: true,
        },
    );
    assert!(matches!(forced, Reply::Sent { forced: true, .. }), "{forced:?}");
    let _ = call(&d, Request::Shutdown);
}

#[test]
fn keys_interrupt_and_keep() {
    let dir = test_dir("keep");
    let d = start_in(&dir);
    let t = sh(&d);
    send(&d, &t.id, "sleep 30");
    thread::sleep(Duration::from_millis(300));
    match call(&d, Request::Interrupt { id: t.id.clone() }) {
        Reply::Interrupted { report } => {
            assert!(report.running);
            assert!(report.tail.contains("sleep 30"), "{}", report.tail);
        }
        other => panic!("{other:?}"),
    }
    let keys = call(
        &d,
        Request::Keys {
            id: t.id.clone(),
            keys: vec![Key::Raw("echo back-from-keys".to_string()), Key::Enter],
        },
    );
    assert_eq!(keys, Reply::Ok);
    assert_eq!(wait_text(&d, &t.id, "^back-from-keys$").0, WaitOutcome::Met);

    let k = create(
        &d,
        CreateSpec {
            cmd: vec!["/bin/sh".to_string(), "-c".to_string(), "sleep 0.3; exit 3".to_string()],
            cwd: Some(dir.display().to_string()),
            keep: true,
            ..CreateSpec::default()
        },
    );
    let first_pid = k.pid;
    assert_eq!(wait(&d, &k.id, WaitFor::Exit, 5000).0, WaitOutcome::Met);
    let deadline = Instant::now() + Duration::from_secs(5);
    let fresh = loop {
        let info = get(&d, &k.id);
        if info.running {
            break info;
        }
        assert!(Instant::now() < deadline, "no fresh shell");
        thread::sleep(Duration::from_millis(50));
    };
    assert_ne!(fresh.pid, first_pid);
    assert!(fresh.cmd.is_empty());
    send(&d, &k.id, "echo fresh-$((40+2))");
    assert_eq!(wait_text(&d, &k.id, "^fresh-42$").0, WaitOutcome::Met);

    call(&d, Request::Kill { id: k.id.clone() });
    thread::sleep(Duration::from_millis(500));
    assert!(!get(&d, &k.id).running);
    let _ = call(&d, Request::Shutdown);
}

#[test]
fn ids_are_never_reused() {
    let dir = test_dir("ids");
    let mut ids = std::collections::HashSet::new();
    {
        let d = start_in(&dir);
        assert!(ids.insert(sh(&d).id));
        let second = sh(&d);
        assert!(ids.insert(second.id.clone()));
        call(&d, Request::Remove { id: second.id });
        assert!(ids.insert(sh(&d).id));
        let _ = call(&d, Request::Shutdown);
    }
    let d = start_in(&dir);
    assert!(ids.insert(sh(&d).id));
    for id in ids {
        assert!(dir.join("terminal-ids").join(id.as_str()).is_file());
    }
    let bad = call(
        &d,
        Request::Create {
            spec: CreateSpec {
                cmd: vec!["/no/such/program".to_string()],
                ..CreateSpec::default()
            },
        },
    );
    assert!(matches!(bad, Reply::Error { code: ErrorCode::BadRequest, .. }), "{bad:?}");
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn huge_timeouts_get_a_reply() {
    let dir = test_dir("huge");
    let d = start_in(&dir);
    let t = sh(&d);
    let waited = call(
        &d,
        Request::Wait {
            id: t.id.clone(),
            until: WaitFor::Idle,
            timeout_ms: Some(u64::MAX),
        },
    );
    assert!(matches!(waited, Reply::Waited { .. }), "{waited:?}");
    let sent = call(
        &d,
        Request::Send {
            id: t.id.clone(),
            text: "echo big".to_string(),
            submit: true,
            timeout_ms: Some(u64::MAX),
            force: false,
        },
    );
    assert!(matches!(sent, Reply::Sent { .. }), "{sent:?}");
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn submit_after_exit_says_exited() {
    let dir = test_dir("gone");
    let d = start_in(&dir);
    for _ in 0..5 {
        let t = create(
            &d,
            CreateSpec {
                cmd: vec!["/bin/sh".to_string(), "-c".to_string(), "sleep 0.03".to_string()],
                ..CreateSpec::default()
            },
        );
        let reply = call(
            &d,
            Request::Send {
                id: t.id.clone(),
                text: "x".to_string(),
                submit: true,
                timeout_ms: Some(1000),
                force: false,
            },
        );
        assert!(
            matches!(reply, Reply::Error { code: ErrorCode::Exited, .. } | Reply::Sent { .. }),
            "{reply:?}"
        );
    }
    let gone = create(
        &d,
        CreateSpec {
            cmd: vec!["/bin/sh".to_string(), "-c".to_string(), "exit 0".to_string()],
            ..CreateSpec::default()
        },
    );
    assert_eq!(wait(&d, &gone.id, WaitFor::Exit, 5000).0, WaitOutcome::Met);
    let reply = send(&d, &gone.id, "x");
    assert!(matches!(reply, Reply::Error { code: ErrorCode::Exited, .. }), "{reply:?}");
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

fn output_events(d: &Daemon, id: &TermId) -> Receiver<(Instant, Event)> {
    let c = client(d);
    let (tx, rx) = channel();
    let ids = vec![id.clone()];
    thread::spawn(move || {
        let _ = c.subscribe(ids, true, |e| tx.send((Instant::now(), e)).is_ok());
    });
    thread::sleep(Duration::from_millis(200));
    rx
}

fn until_running(d: &Daemon, id: &TermId, old_pid: Option<u32>) -> TermInfo {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let info = get(d, id);
        if info.running && info.pid != old_pid {
            return info;
        }
        assert!(Instant::now() < deadline, "no fresh shell");
        thread::sleep(Duration::from_millis(50));
    }
}

fn keep_sh(d: &Daemon) -> TermInfo {
    create(
        d,
        CreateSpec {
            cmd: vec!["/bin/sh".to_string()],
            cwd: Some(d.dir.display().to_string()),
            keep: true,
            ..CreateSpec::default()
        },
    )
}

#[test]
fn output_changed_is_throttled() {
    let dir = test_dir("burst");
    let d = start_in(&dir);
    let t = sh(&d);
    thread::sleep(Duration::from_millis(300));
    let rx = output_events(&d, &t.id);
    let seq_before = get(&d, &t.id).seq;
    send(&d, &t.id, "seq 1 200000; echo burst-$((1+1))");
    assert_eq!(wait_text(&d, &t.id, "^burst-2$").0, WaitOutcome::Met);
    thread::sleep(Duration::from_millis(300));
    let seq_after = get(&d, &t.id).seq;
    let seen: Vec<(Instant, u64)> = rx
        .try_iter()
        .filter_map(|(at, e)| match e {
            Event::OutputChanged { seq, .. } => Some((at, seq)),
            _ => None,
        })
        .collect();
    assert!(!seen.is_empty());
    let span = seen.last().unwrap().0.duration_since(seen[0].0);
    let most = span.as_millis() as usize / 16 + 3;
    assert!(seen.len() <= most, "{} events in {span:?}", seen.len());
    let chunks = (seq_after - seq_before) as usize;
    assert!(seen.len() < chunks, "{} events for {chunks} chunks", seen.len());
    assert_eq!(seen.last().unwrap().1, seq_after, "the last output change was not sent");
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn keep_starts_in_the_shells_folder() {
    let dir = test_dir("cwd");
    let d = start_in(&dir);
    let t = keep_sh(&d);
    send(&d, &t.id, "mkdir -p deeper && cd deeper && echo in-$((1+1))");
    assert_eq!(wait_text(&d, &t.id, "^in-2$").0, WaitOutcome::Met);
    thread::sleep(Duration::from_millis(200));
    send(&d, &t.id, "exit");
    let fresh = until_running(&d, &t.id, t.pid);
    let want = dir.join("deeper").canonicalize().unwrap();
    assert_eq!(Path::new(&fresh.cwd).canonicalize().unwrap(), want);
    send(&d, &t.id, "pwd");
    assert_eq!(wait_text(&d, &t.id, "deeper$").0, WaitOutcome::Met);
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn kill_and_keep_rules() {
    let dir = test_dir("rules");
    let d = start_in(&dir);
    let t = keep_sh(&d);
    assert_eq!(call(&d, Request::Kill { id: t.id.clone() }), Reply::Ok);
    thread::sleep(Duration::from_millis(1200));
    let killed = get(&d, &t.id);
    assert!(!killed.running, "kill must not start a fresh shell");
    assert!(killed.keep);
    assert_eq!(call(&d, Request::Keep { id: t.id.clone(), on: true }), Reply::Ok);
    let fresh = until_running(&d, &t.id, t.pid);
    send(&d, &t.id, "echo again-$((2+2))");
    assert_eq!(wait_text(&d, &t.id, "^again-4$").0, WaitOutcome::Met);
    assert_eq!(call(&d, Request::Keep { id: t.id.clone(), on: false }), Reply::Ok);
    send(&d, &t.id, "exit 5");
    assert_eq!(wait(&d, &t.id, WaitFor::Exit, 5000).0, WaitOutcome::Met);
    thread::sleep(Duration::from_millis(800));
    let ended = get(&d, &t.id);
    assert!(!ended.running, "keep off must not start a fresh shell");
    assert_eq!(ended.exit_code, Some(5));
    assert_ne!(ended.pid, fresh.pid);
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn keep_stops_when_the_shell_keeps_ending() {
    let dir = test_dir("loop");
    let d = start_in(&dir);
    let rx = events(&d, Vec::new());
    let mut env = std::collections::BTreeMap::new();
    env.insert("SHELL".to_string(), "/usr/bin/false".to_string());
    let t = create(
        &d,
        CreateSpec {
            cmd: vec!["/bin/sh".to_string(), "-c".to_string(), "exit 0".to_string()],
            cwd: Some(dir.display().to_string()),
            env,
            keep: true,
            ..CreateSpec::default()
        },
    );
    let id = t.id.clone();
    let gave_up = expect_event(&rx, |e| {
        matches!(e, Event::Status { id: i, status: Some(Status::Error), .. } if *i == id)
    });
    assert!(matches!(gave_up, Event::Status { note: Some(n), .. } if n.contains("keep stopped")));
    thread::sleep(Duration::from_millis(500));
    let respawns = rx
        .try_iter()
        .filter(|e| matches!(e, Event::Respawned { .. }))
        .count();
    assert_eq!(respawns, 0, "no fresh shell after giving up");
    let info = get(&d, &t.id);
    assert!(!info.running);
    assert_eq!(info.status, Some(Status::Error));
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn interrupt_tail_is_the_last_lines() {
    let dir = test_dir("tail");
    let d = start_in(&dir);
    let t = sh(&d);
    send(&d, &t.id, "i=1; while [ $i -le 40 ]; do echo row$i; i=$((i+1)); done; sleep 30");
    assert_eq!(wait_text(&d, &t.id, "^row40$").0, WaitOutcome::Met);
    thread::sleep(Duration::from_millis(200));
    match call(&d, Request::Interrupt { id: t.id.clone() }) {
        Reply::Interrupted { report } => {
            let lines: Vec<&str> = report.tail.lines().collect();
            assert!(lines.len() <= 10, "{}", report.tail);
            assert!(lines.contains(&"row40"), "{}", report.tail);
            assert!(!lines.contains(&"row30"), "{}", report.tail);
            assert!(lines.last().is_some_and(|l| !l.trim().is_empty()), "{}", report.tail);
        }
        other => panic!("{other:?}"),
    }
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "macos")]
#[test]
fn a_zsh_prompt_redraw_keeps_reflowed_output_and_the_draft() {
    let dir = test_dir("zsh-resize");
    let d = start_in(&dir);
    let prompt = format!(
        "PROMPT='Prompt start\nLONG-LEFT:{}> '\nRPROMPT='RIGHT-MARK'\n",
        "p".repeat(45)
    );
    fs::write(dir.join(".zshrc"), prompt).unwrap();
    let term = create(
        &d,
        CreateSpec {
            cmd: vec!["/bin/zsh".into(), "-d".into(), "-i".into()],
            cwd: Some(dir.display().to_string()),
            env: [
                ("HOME".into(), dir.display().to_string()),
                ("ZDOTDIR".into(), dir.display().to_string()),
            ].into(),
            rows: Some(30),
            cols: Some(92),
            ..CreateSpec::default()
        },
    );
    assert_eq!(wait_text(&d, &term.id, "RIGHT-MARK").0, WaitOutcome::Met);
    send(&d, &term.id, r"printf 'first output\nend output\n'");
    assert_eq!(
        wait_text(&d, &term.id, "(?m)^end output$").0,
        WaitOutcome::Met
    );
    assert!(matches!(
        call(&d, Request::Input { id: term.id.clone(), text: "draft".into() }),
        Reply::Ok
    ));

    for (rows, cols) in [(13, 40), (24, 90), (13, 40)] {
        assert!(matches!(
            call(&d, Request::Resize { id: term.id.clone(), rows, cols }),
            Reply::Ok
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        let text = loop {
            let text = read(&d, &term.id, ReadSource::Screen);
            if text.lines().any(|line| line.ends_with("RIGHT-MARK") && line.chars().count() == usize::from(cols - 1))
                && text.contains("draft")
            {
                break text;
            }
            assert!(Instant::now() < deadline, "no redraw at {rows}x{cols}: {text:?}");
            thread::sleep(Duration::from_millis(10));
        };
        assert!(text.contains("first output\nend output"), "{text:?}");
        let (_, prompt) = text.split_once("end output\n").unwrap();
        assert_eq!(prompt.matches("Prompt start").count(), 1, "{text:?}");
        assert_eq!(prompt.matches("LONG-LEFT:").count(), 1, "{text:?}");
    }
    call(&d, Request::Remove { id: term.id });
    drop(d);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn tui_paste_debounce_and_swallowed_first_enter_still_submits() {
    let dir = test_dir("tui-submit");
    let script = dir.join("tui.py");
    fs::write(
        &script,
        r#"import os, select, sys, termios, time, tty
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
def draw(prompt, body=""):
    out = "\x1b[2J\x1b[H"
    if body:
        out += body + "\r\n"
    out += "────────\r\n❯ " + prompt + "\r\n────────\r\n"
    os.write(1, out.encode())
try:
    os.write(1, b"\x1b[?1049h\x1b[?2004h\x1b[?1004h")
    draw("", "TUI_READY")
    buf = ""
    raw = b""
    saw_focus = False
    saw_bracketed = False
    swallowed = 0
    while True:
        r, _, _ = select.select([fd], [], [], 5.0)
        if not r:
            break
        chunk = os.read(fd, 4096)
        if not chunk:
            break
        raw += chunk
        while raw:
            if raw.startswith(b"\x1b[I"):
                saw_focus = True
                raw = raw[3:]
                continue
            if raw.startswith(b"\x1b[200~"):
                end = raw.find(b"\x1b[201~")
                if end == -1:
                    break
                saw_bracketed = True
                pasted = raw[6:end].decode("utf-8", "replace")
                raw = raw[end + 6:]
                time.sleep(0.08)
                buf += pasted
                draw(buf, "TUI_READY")
                continue
            if raw[:1] == b"\r":
                raw = raw[1:]
                if buf and swallowed == 0:
                    swallowed += 1
                    continue
                if buf:
                    draw("", f"SUBMITTED:{buf}|focus={int(saw_focus)}|bracket={int(saw_bracketed)}")
                    buf = ""
                continue
            buf += raw[:1].decode("utf-8", "replace")
            raw = raw[1:]
            draw(buf, "TUI_READY")
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
"#,
    )
    .unwrap();
    let d = start_in(&dir);
    let term = create(
        &d,
        CreateSpec {
            cmd: vec!["python3".into(), script.display().to_string()],
            cwd: Some(dir.display().to_string()),
            ..CreateSpec::default()
        },
    );
    assert_eq!(wait_text(&d, &term.id, "TUI_READY").0, WaitOutcome::Met);
    send(&d, &term.id, "move the s2 tasks to s6");
    assert_eq!(
        wait_text(&d, &term.id, "SUBMITTED:move the s2 tasks to s6\\|focus=1\\|bracket=1").0,
        WaitOutcome::Met,
        "screen was: {:?}",
        read(&d, &term.id, ReadSource::Screen)
    );
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn busy_tui_holds_message_in_prompt_until_idle_then_engine_nudges_enter() {
    let dir = test_dir("tui-busy-nudge");
    let script = dir.join("busy_tui.py");
    fs::write(
        &script,
        r#"import os, select, sys, termios, time, tty
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
def draw(prompt, body=""):
    out = "\x1b[2J\x1b[H"
    if body:
        out += body + "\r\n"
    out += "╰─ " + prompt + "\r\n"
    os.write(1, out.encode())
try:
    os.write(1, b"\x1b[?2004h\x1b]7421;status=working\x07")
    draw("", "BUSY_READY")
    buf = ""
    raw = b""
    busy = True
    started = None
    while True:
        if busy and started is not None and time.monotonic() - started >= 1.1:
            busy = False
            os.write(1, b"\x1b]7421;status=idle\x07")
        r, _, _ = select.select([fd], [], [], 0.05)
        if not r:
            continue
        chunk = os.read(fd, 4096)
        if not chunk:
            break
        raw += chunk
        while raw:
            if raw.startswith(b"\x1b[200~"):
                end = raw.find(b"\x1b[201~")
                if end == -1:
                    break
                buf += raw[6:end].decode("utf-8", "replace")
                raw = raw[end + 6:]
                if started is None:
                    started = time.monotonic()
                draw(buf, "BUSY_READY")
                continue
            if raw[:1] == b"\r":
                raw = raw[1:]
                if not busy and buf:
                    draw("", f"SUBMITTED:{buf}")
                    buf = ""
                continue
            buf += raw[:1].decode("utf-8", "replace")
            raw = raw[1:]
            draw(buf, "BUSY_READY")
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
"#,
    )
    .unwrap();
    let d = start_in(&dir);
    let term = create(
        &d,
        CreateSpec {
            cmd: vec!["python3".into(), script.display().to_string()],
            cwd: Some(dir.display().to_string()),
            ..CreateSpec::default()
        },
    );
    assert_eq!(wait_text(&d, &term.id, "BUSY_READY").0, WaitOutcome::Met);
    send(&d, &term.id, "fix the emulator");
    assert_eq!(
        wait_text(&d, &term.id, "SUBMITTED:fix the emulator").0,
        WaitOutcome::Met,
        "screen was: {:?}",
        read(&d, &term.id, ReadSource::Screen)
    );
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn kitty_keyboard_tui_receives_csi_u_enter() {
    let dir = test_dir("tui-kitty");
    let script = dir.join("kitty_tui.py");
    fs::write(
        &script,
        r#"import os, select, sys, termios, tty
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
def draw(prompt, body=""):
    out = "\x1b[2J\x1b[H"
    if body:
        out += body + "\r\n"
    out += "❯ " + prompt + "\r\n"
    os.write(1, out.encode())
try:
    os.write(1, b"\x1b[?2004h\x1b[>8u")
    draw("", "KITTY_READY")
    buf = ""
    raw = b""
    while True:
        r, _, _ = select.select([fd], [], [], 5.0)
        if not r:
            break
        chunk = os.read(fd, 4096)
        if not chunk:
            break
        raw += chunk
        while raw:
            if raw.startswith(b"\x1b[200~"):
                end = raw.find(b"\x1b[201~")
                if end == -1:
                    break
                buf += raw[6:end].decode("utf-8", "replace")
                raw = raw[end + 6:]
                draw(buf, "KITTY_READY")
                continue
            if raw.startswith(b"\x1b[13u"):
                raw = raw[5:]
                if buf:
                    draw("", f"SUBMITTED:{buf}")
                    buf = ""
                continue
            if raw[:1] == b"\x1b" and len(raw) < 6:
                break
            raw = raw[1:]
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
"#,
    )
    .unwrap();
    let d = start_in(&dir);
    let term = create(
        &d,
        CreateSpec {
            cmd: vec!["python3".into(), script.display().to_string()],
            cwd: Some(dir.display().to_string()),
            ..CreateSpec::default()
        },
    );
    assert_eq!(wait_text(&d, &term.id, "KITTY_READY").0, WaitOutcome::Met);
    send(&d, &term.id, "hello kitty");
    assert_eq!(
        wait_text(&d, &term.id, "SUBMITTED:hello kitty").0,
        WaitOutcome::Met,
        "screen was: {:?}",
        read(&d, &term.id, ReadSource::Screen)
    );
    let _ = call(&d, Request::Shutdown);
    drop(d);
    let _ = fs::remove_dir_all(&dir);
}
