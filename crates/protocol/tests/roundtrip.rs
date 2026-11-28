use nixa_protocol::wire;
use nixa_protocol::*;
use std::collections::BTreeMap;
use std::io::BufReader;
use std::os::unix::net::UnixListener;

fn id(n: u64) -> TermId {
    TermId::from_number(n)
}

fn term() -> TermInfo {
    let mut labels = BTreeMap::new();
    labels.insert("role".to_string(), "builder".to_string());
    TermInfo {
        id: id(11),
        name: "codex".to_string(),
        labels,
        parent: Some(id(1)),
        children: vec![id(12)],
        cmd: vec!["codex".to_string(), "--yolo".to_string()],
        cwd: "/tmp".to_string(),
        pid: Some(4242),
        running: true,
        exit_code: None,
        status: Some(Status::Working),
        status_note: Some("reading files".to_string()),
        keep: true,
        draft: false,
        title: Some("codex".to_string()),
        rows: 40,
        cols: 120,
        created_ms: 1_700_000_000_000,
        seq: 9,
    }
}

fn snapshot() -> Snapshot {
    let mut cells = vec![Cell::default(); 6];
    cells[0] = Cell {
        ch: 'h',
        zerowidth: None,
        fg: Color::Indexed(2),
        bg: Color::Default,
        flags: flags::BOLD,
    };
    cells[1] = Cell {
        ch: '界',
        zerowidth: None,
        fg: Color::Rgb(1, 2, 3),
        bg: Color::Indexed(0),
        flags: flags::WIDE,
    };
    cells[2] = Cell {
        ch: ' ',
        zerowidth: None,
        fg: Color::Default,
        bg: Color::Default,
        flags: flags::WIDE_SPACER,
    };
    cells[3] = Cell {
        ch: 'e',
        zerowidth: Some("\u{301}".to_string()),
        ..Cell::default()
    };
    Snapshot {
        id: id(3),
        seq: 77,
        rows: 2,
        cols: 3,
        cells,
        cursor: Cursor {
            row: 1,
            col: 2,
            visible: true,
            shape: CursorShape::Bar,
        },
        modes: Modes {
            app_cursor: true,
            bracketed_paste: true,
            ..Modes::default()
        },
        title: Some("zsh".to_string()),
        history: 100,
        scroll: 0,
    }
}

fn all_requests() -> Vec<Request> {
    let mut env = BTreeMap::new();
    env.insert("A".to_string(), "1".to_string());
    vec![
        Request::Ping,
        Request::List { parent: None, label: None },
        Request::List { parent: Some(id(1)), label: Some("role=builder".to_string()) },
        Request::Get { id: id(1) },
        Request::ChildReports { id: id(1), reader: None },
        Request::ChildReport { id: id(2), parent: Some(id(1)), text: "checks passed".into() },
        Request::ChildOutput { id: id(1), child: id(2) },
        Request::ClearParent { id: id(2) },
        Request::Create { spec: CreateSpec::default() },
        Request::Create {
            spec: CreateSpec {
                cmd: vec!["claude".to_string()],
                cwd: Some("/tmp".to_string()),
                name: Some("claude".to_string()),
                labels: BTreeMap::new(),
                parent: Some(id(2)),
                env,
                rows: Some(30),
                cols: Some(100),
                keep: true,
            },
        },
        Request::Kill { id: id(1) },
        Request::Remove { id: id(1) },
        Request::Read { id: id(1), source: ReadSource::History, lines: Some(50) },
        Request::Snapshot { id: id(1), scroll: 3, since_seq: Some(9) },
        Request::Send { id: id(1), text: "hi".to_string(), submit: true, timeout_ms: Some(5000), force: false },
        Request::Input { id: id(1), text: "l".to_string() },
        Request::Keys {
            id: id(1),
            keys: vec![Key::Enter, Key::Esc, Key::Tab, Key::Ctrl('c'), Key::Up, Key::F(3), Key::Raw("\x1b[A".to_string())],
        },
        Request::Interrupt { id: id(1) },
        Request::Resize { id: id(1), rows: 50, cols: 200 },
        Request::LabelSet { id: id(1), key: "role".to_string(), value: Some("tester".to_string()) },
        Request::LabelSet { id: id(1), key: "role".to_string(), value: None },
        Request::Rename { id: id(1), name: "new".to_string() },
        Request::StatusSet { id: id(1), status: Some(Status::Blocked), note: Some("needs yes".to_string()) },
        Request::StatusSet { id: id(1), status: None, note: None },
        Request::Wait { id: id(1), until: WaitFor::Idle, timeout_ms: Some(1000) },
        Request::Wait { id: id(1), until: WaitFor::Status { status: Status::Done }, timeout_ms: None },
        Request::Wait { id: id(1), until: WaitFor::Text { pattern: "ok".to_string(), regex: true }, timeout_ms: None },
        Request::Wait { id: id(1), until: WaitFor::Children, timeout_ms: None },
        Request::Wait { id: id(1), until: WaitFor::Exit, timeout_ms: None },
        Request::Keep { id: id(1), on: true },
        Request::Subscribe { ids: vec![], output: false },
        Request::Subscribe { ids: vec![id(1), id(36)], output: true },
        Request::Shutdown,
    ]
}

#[test]
fn old_child_reports_default_to_no_current_result() {
    let message = r#"{"reply":"child_reports","reports":[{"child":"m2","status":"blocked","exited":false,"note":"waiting"}]}"#;
    let Reply::ChildReports { reports } = wire::parse_reply(message).unwrap() else {
        panic!("missing reports");
    };
    assert!(!reports[0].current_job_result);
}

fn all_events() -> Vec<Event> {
    vec![
        Event::Created { term: term() },
        Event::Status { id: id(1), status: Some(Status::Done), note: None },
        Event::Exit { id: id(1), code: Some(0) },
        Event::Respawned { id: id(1), pid: Some(9) },
        Event::Removed { id: id(1) },
        Event::ParentCleared { id: id(2), parent: id(1) },
        Event::Label { id: id(1), labels: BTreeMap::new() },
        Event::Renamed { id: id(1), name: "x".to_string() },
        Event::Title { id: id(1), title: None },
        Event::Resized { id: id(1), rows: 1, cols: 2 },
        Event::Draft { id: id(1), active: true },
        Event::OutputChanged { id: id(1), seq: 5 },
        Event::ChildNote { id: id(1), child: id(2), status: Some(Status::Blocked), exited: false, note: "m2 is blocked".to_string() },
    ]
}

fn all_replies() -> Vec<Reply> {
    let mut out = vec![
        Reply::Ok,
        Reply::Pong { version: PROTOCOL_VERSION, pid: 1 },
        Reply::Terms { terms: vec![term()] },
        Reply::Term { term: term() },
        Reply::Text { id: id(1), text: "a\nb".to_string() },
        Reply::Snapshot { snapshot: snapshot() },
        Reply::Unchanged { id: id(1), seq: 4 },
        Reply::Sent { id: id(1), waited_ms: 30, forced: false, started: Some(true), tail: String::new() },
        Reply::Interrupted {
            report: InterruptReport {
                id: id(1),
                status_before: Some(Status::Working),
                status_after: Some(Status::Idle),
                running: true,
                exit_code: None,
                tail: "^C".to_string(),
            },
        },
        Reply::Waited {
            result: WaitResult {
                id: id(1),
                outcome: WaitOutcome::Met,
                status: Some(Status::Idle),
                matched: Some("ok".to_string()),
                waited_ms: 12,
            },
        },
        Reply::Subscribed,
        Reply::error(ErrorCode::DraftBusy, "a person is typing"),
    ];
    out.extend(all_events().into_iter().map(|event| Reply::Event { event }));
    out
}

#[test]
fn requests_round_trip() {
    for req in all_requests() {
        let line = wire::request_line(&req);
        assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
        assert_eq!(wire::parse_request(&line).unwrap(), req, "{line}");
    }
}

#[test]
fn replies_round_trip() {
    for reply in all_replies() {
        let line = wire::reply_line(&reply);
        assert!(!line[..line.len() - 1].contains('\n'));
        assert_eq!(wire::parse_reply(&line).unwrap(), reply, "{line}");
    }
}

#[test]
fn wire_shape_is_stable() {
    let line = wire::request_line(&Request::Send {
        id: id(10),
        text: "go".to_string(),
        submit: true,
        timeout_ms: None,
        force: false,
    });
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["op"], "send");
    assert_eq!(v["id"], "ma");
    let v: serde_json::Value = serde_json::from_str(&wire::request_line(&Request::Create {
        spec: CreateSpec { cmd: vec!["zsh".to_string()], ..CreateSpec::default() },
    }))
    .unwrap();
    assert_eq!(v["op"], "create");
    assert_eq!(v["cmd"][0], "zsh");
    let v: serde_json::Value = serde_json::from_str(&wire::reply_line(&Reply::Event {
        event: Event::OutputChanged { id: id(1), seq: 2 },
    }))
    .unwrap();
    assert_eq!(v["reply"], "event");
    assert_eq!(v["event"]["kind"], "output_changed");
    let keys: serde_json::Value = serde_json::to_value(vec![Key::Enter, Key::Ctrl('c')]).unwrap();
    assert_eq!(keys, serde_json::json!(["enter", {"ctrl": "c"}]));
}

#[test]
fn short_hand_input_parses() {
    let req = wire::parse_request(r#"{"op":"create"}"#).unwrap();
    assert_eq!(req, Request::Create { spec: CreateSpec::default() });
    let req = wire::parse_request(r#"{"op":"read","id":"m2"}"#).unwrap();
    assert_eq!(req, Request::Read { id: id(2), source: ReadSource::Screen, lines: None });
    let req = wire::parse_request(r#"{"op":"wait","id":"m2","until":{"kind":"idle"}}"#).unwrap();
    assert_eq!(req, Request::Wait { id: id(2), until: WaitFor::Idle, timeout_ms: None });
    assert!(wire::parse_request(r#"{"op":"get","id":"x2"}"#).is_err());
    assert!(wire::parse_request(r#"{"op":"fly"}"#).is_err());
}

#[test]
fn snapshot_rows_and_text() {
    let s = snapshot();
    assert_eq!(s.row(0).len(), 3);
    assert_eq!(s.row_text(0), "h界");
    assert_eq!(s.row_text(1), "e\u{301}");
    let compact = serde_json::to_string(&Cell::default()).unwrap();
    assert_eq!(compact, r#"{"c":" "}"#);
}

#[test]
fn client_talks_over_unix_socket() {
    let dir = std::env::temp_dir().join(format!("nixa-proto-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("engine.sock");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            while let Some(req) = wire::read_request(&mut reader).unwrap() {
                match req {
                    Request::Ping => wire::write_reply(&mut writer, &Reply::Pong { version: PROTOCOL_VERSION, pid: 7 }).unwrap(),
                    Request::Get { id } => wire::write_reply(&mut writer, &Reply::not_found(&id)).unwrap(),
                    Request::Subscribe { .. } => {
                        wire::write_reply(&mut writer, &Reply::Subscribed).unwrap();
                        for seq in 1..=3 {
                            wire::write_reply(&mut writer, &Reply::Event { event: Event::OutputChanged { id: TermId::from_number(1), seq } }).unwrap();
                        }
                        break;
                    }
                    _ => wire::write_reply(&mut writer, &Reply::Ok).unwrap(),
                }
            }
        }
    });

    let mut client = connect(&path).unwrap();
    assert_eq!(client.request(&Request::Ping).unwrap(), Reply::Pong { version: PROTOCOL_VERSION, pid: 7 });
    let err = client.call(&Request::Get { id: id(5) }).unwrap_err();
    let engine_err = err.downcast_ref::<EngineError>().unwrap();
    assert_eq!(engine_err.code, ErrorCode::NotFound);
    assert_eq!(client.call(&Request::Kill { id: id(5) }).unwrap(), Reply::Ok);
    drop(client);

    let mut seen = Vec::new();
    connect(&path)
        .unwrap()
        .subscribe(vec![], true, |event| {
            if let Event::OutputChanged { seq, .. } = event {
                seen.push(seq);
            }
            seen.len() < 2
        })
        .unwrap();
    assert_eq!(seen, vec![1, 2]);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
