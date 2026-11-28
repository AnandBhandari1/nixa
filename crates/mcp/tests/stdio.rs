use nixa_protocol::{
    ChildReport, Reply, Request, Status, TermId, TermInfo, WaitFor, WaitOutcome, WaitResult, wire,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nixa-mcp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn term(id: &str) -> TermInfo {
    TermInfo {
        id: id.parse::<TermId>().unwrap(),
        name: "helper".into(),
        labels: BTreeMap::new(),
        parent: None,
        children: Vec::new(),
        cmd: vec!["claude".into()],
        cwd: "/tmp".into(),
        pid: Some(42),
        running: true,
        exit_code: None,
        status: Some(Status::Working),
        status_note: None,
        keep: false,
        draft: false,
        title: None,
        rows: 24,
        cols: 80,
        created_ms: 1,
        seq: 1,
    }
}

fn answer(req: &Request) -> Reply {
    match req {
        Request::Ping => Reply::Pong { version: 1, pid: 7 },
        Request::List { .. } => Reply::Terms { terms: vec![term("m1")] },
        Request::Get { id } if id.as_str() == "m1" => Reply::Term { term: term("m1") },
        Request::Get { id } if id.as_str() == "m2" => {
            let mut child = term("m2");
            child.parent = Some(TermId::from_number(1));
            Reply::Term { term: child }
        }
        Request::Get { id } => Reply::not_found(id),
        Request::Create { spec } => {
            let mut new = term("m3");
            new.cmd = spec.cmd.clone();
            new.parent = spec.parent.clone();
            Reply::Term { term: new }
        }
        Request::Read { id, .. } => Reply::Text { id: id.clone(), text: "hello from m1\n\n".into() },
        Request::Send { id, .. } => Reply::Sent { id: id.clone(), waited_ms: 0, forced: false, started: None, tail: String::new() },
        Request::Keys { .. } => Reply::Ok,
        Request::Wait { id, until: until @ (WaitFor::ChildAny | WaitFor::ChildAll), .. } => {
            Reply::Waited {
                result: WaitResult {
                    id: id.clone(),
                    outcome: WaitOutcome::Met,
                    status: Some(Status::Working),
                    matched: matches!(until, WaitFor::ChildAny).then(|| "demo-codex-abc123".into()),
                    waited_ms: 0,
                },
            }
        }
        Request::ChildReports { .. } => Reply::ChildReports {
            reports: vec![ChildReport {
                child: "demo-codex-abc123".parse().unwrap(),
                status: Some(Status::Blocked),
                exited: false,
                current_job_result: false, read: false,
                note: "Needs a decision".into(),
            }],
        },
        _ => Reply::error(nixa_protocol::ErrorCode::Unsupported, "not in the fake engine"),
    }
}

fn fake_engine(socket: &PathBuf) -> Arc<Mutex<Vec<Request>>> {
    let listener = UnixListener::bind(socket).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let log = Arc::clone(&log);
            thread::spawn(move || {
                let mut writer = stream.try_clone().unwrap();
                let mut reader = BufReader::new(stream);
                while let Ok(Some(req)) = wire::read_request(&mut reader) {
                    let reply = answer(&req);
                    log.lock().unwrap().push(req);
                    if wire::write_reply(&mut writer, &reply).is_err() {
                        break;
                    }
                }
            });
        }
    });
    seen
}

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

fn server(socket: &PathBuf, data: &PathBuf, mode: &str) -> Server {
    server_in_term(socket, data, mode, None)
}

fn server_in_term(socket: &PathBuf, data: &PathBuf, mode: &str, own: Option<&str>) -> Server {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nixa-mcp"));
    command
        .env("NIXA_ENGINE_SOCKET", socket)
        .env("NIXA_DATA_DIR", data)
        .env("NIXA_MCP_MODE", mode)
        .env_remove("NIXA_TERM_ID");
    if let Some(id) = own {
        command.env("NIXA_TERM_ID", id);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    Server { child, stdin, stdout, next: 0 }
}

impl Server {
    fn line(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn reply(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn ask(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.line(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let reply = self.reply();
        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["id"], id);
        reply
    }

    fn call(&mut self, name: &str, arguments: Value) -> (bool, String) {
        let reply = self.ask("tools/call", json!({ "name": name, "arguments": arguments }));
        let result = &reply["result"];
        assert_eq!(result["content"][0]["type"], "text");
        (
            result["isError"].as_bool().unwrap(),
            result["content"][0]["text"].as_str().unwrap().to_string(),
        )
    }

    fn start(&mut self) {
        let init = self.ask("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }));
        assert_eq!(init["result"]["serverInfo"]["name"], "nixa-mcp");
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        self.line(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    }

    fn tool_names(&mut self) -> Vec<String> {
        let list = self.ask("tools/list", json!({}));
        let tools = list["result"]["tools"].as_array().unwrap().clone();
        for tool in &tools {
            assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
            assert!(!tool["description"].as_str().unwrap().is_empty());
        }
        tools.iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    }

    fn stop(mut self) {
        drop(self.stdin);
        let status = self.child.wait().unwrap();
        assert!(status.success());
    }
}

#[test]
fn tools_list_and_calls_over_stdio() {
    let dir = temp_dir("calls");
    let socket = dir.join("engine.sock");
    let seen = fake_engine(&socket);
    let mut s = server(&socket, &dir, "full");
    s.start();

    let names = s.tool_names();
    for name in ["nixa_term_list", "nixa_term_read", "nixa_term_send", "nixa_term_wait", "nixa_agent_start"] {
        assert!(names.contains(&name.to_string()), "{name} missing");
    }
    assert!(names.iter().all(|n| n.starts_with("nixa_")));

    let (failed, text) = s.call("nixa_term_list", json!({}));
    assert!(!failed, "{text}");
    let listed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["terminals"][0]["terminal"], "m1");
    assert_eq!(listed["terminals"][0]["status"], "working");

    let (failed, text) = s.call("nixa_term_read", json!({ "terminal": "m1", "lines": 5 }));
    assert!(!failed, "{text}");
    assert_eq!(text, "hello from m1");

    let (failed, text) = s.call("nixa_term_send", json!({ "terminal": "m1", "message": "run the tests" }));
    assert!(!failed, "{text}");
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["sent"], true);
    let (failed, text) = s.call("nixa_term_send", json!({ "terminal": "m1", "message": "run the tests" }));
    assert!(!failed, "{text}");
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["sent"], true);

    let (failed, text) = s.call("nixa_term_keys", json!({ "terminal": "m1", "keys": ["esc", "2"] }));
    assert!(!failed, "{text}");
    let pressed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(pressed["pressed"], true);
    assert!(pressed.get("confirmationRequired").is_none());

    let (failed, text) = s.call("nixa_term_get", json!({ "terminal": "m9" }));
    assert!(failed);
    assert!(text.contains("no terminal m9"), "{text}");

    let (failed, text) = s.call("nixa_term_read", json!({ "terminal": "m1", "bogus": 1 }));
    assert!(failed);
    assert!(text.contains("Bad arguments"), "{text}");

    let (failed, text) = s.call("nixa_task_add", json!({ "title": "Check the read tool", "status": "todo" }));
    assert!(!failed, "{text}");
    assert!(text.starts_with("#1 todo (no space): Check the read tool"), "{text}");
    let (failed, text) = s.call("nixa_task_update", json!({ "id": 1, "note": "started" }));
    assert!(!failed, "{text}");
    let (failed, text) = s.call("nixa_task_get", json!({ "id": 1 }));
    assert!(!failed, "{text}");
    assert!(text.contains("note by Nixa: \"started\""), "{text}");
    let (failed, text) = s.call("nixa_task_list", json!({ "status": ["todo"] }));
    assert!(!failed, "{text}");
    assert!(text.starts_with("1 open"), "{text}");

    s.stop();

    let seen = seen.lock().unwrap();
    let sends: Vec<&Request> = seen.iter().filter(|r| matches!(r, Request::Send { .. })).collect();
    assert_eq!(sends.len(), 2);
    match sends[0] {
        Request::Send { text, submit, force, .. } => {
            assert_eq!(text, "run the tests");
            assert!(*submit);
            assert!(!*force);
        }
        _ => unreachable!(),
    }
    assert!(seen.iter().any(|r| matches!(r, Request::Read { lines: Some(5), .. })));
    assert!(seen.iter().any(|r| matches!(r, Request::Keys { keys, .. } if keys.len() == 2)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn worker_mode_and_no_engine() {
    let dir = temp_dir("worker");
    let socket = dir.join("missing.sock");
    let mut s = server(&socket, &dir, "worker");
    s.start();
    let names = s.tool_names();
    assert!(names.contains(&"nixa_term_read".to_string()));
    assert!(names.contains(&"nixa_term_child_reports".to_string()));
    assert!(!names.contains(&"nixa_term_send".to_string()));

    let (failed, text) = s.call("nixa_term_send", json!({ "terminal": "m1", "message": "hi" }));
    assert!(failed);
    assert!(text.contains("not allowed in worker mode"), "{text}");

    let (failed, text) = s.call("nixa_term_list", json!({}));
    assert!(failed);
    assert!(text.contains("cannot reach nixa-engine"), "{text}");

    let reply = s.ask("nope/method", json!({}));
    assert_eq!(reply["error"]["code"], -32601);
    s.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn child_waits_and_reports_use_engine_requests() {
    let dir = temp_dir("child-waits");
    let socket = dir.join("engine.sock");
    let seen = fake_engine(&socket);
    let mut s = server(&socket, &dir, "worker");
    s.start();

    let list = s.ask("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    let wait = tools.iter().find(|tool| tool["name"] == "nixa_term_wait").unwrap();
    let modes = wait["inputSchema"]["properties"]["until"]["enum"].as_array().unwrap();
    for name in ["children", "child_any", "child_all"] {
        assert!(modes.contains(&json!(name)));
    }

    let parent = "demo-shell-a1b2c3";
    for (mode, matched) in [("child_any", json!("demo-codex-abc123")), ("child_all", Value::Null)] {
        let (failed, text) = s.call("nixa_term_wait", json!({"terminal": parent, "until": mode}));
        assert!(!failed, "{text}");
        let result: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(result["terminal"], parent);
        assert_eq!(result["outcome"], "met");
        assert_eq!(result["matched"], matched);
    }

    let (failed, text) = s.call("nixa_term_child_reports", json!({"terminal": "m1"}));
    assert!(!failed, "{text}");
    let result: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result, json!({
        "terminal": "m1",
        "reports": [{
            "child": "demo-codex-abc123",
            "status": "blocked",
            "exited": false,
            "current_job_result": false,
            "note": "Needs a decision"
        }]
    }));

    s.stop();
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.as_slice(),
        [
            Request::Wait { id: parent.parse().unwrap(), until: WaitFor::ChildAny, timeout_ms: Some(5000) },
            Request::Wait { id: parent.parse().unwrap(), until: WaitFor::ChildAll, timeout_ms: Some(5000) },
            Request::ChildReports { id: "m1".parse().unwrap(), reader: None },
        ],
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn child_cannot_start_a_terminal_or_agent() {
    let dir = temp_dir("nested-start");
    let socket = dir.join("engine.sock");
    let seen = fake_engine(&socket);
    let mut server = server_in_term(&socket, &dir, "full", Some("m2"));
    server.start();

    for (name, args) in [
        ("nixa_term_start", json!({})),
        ("nixa_term_start", json!({"cmd": ["claude"], "task": "Run checks"})),
        ("nixa_agent_start", json!({"agent": "claude"})),
    ] {
        let (failed, text) = server.call(name, args);
        assert!(failed, "{text}");
        assert!(text.contains("m2 as a parent: it already has a parent"), "{text}");
    }
    server.stop();
    assert_eq!(*seen.lock().unwrap(), vec![Request::Get { id: TermId::from_number(2) }; 3]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_start_uses_the_configured_permissions() {
    let dir = temp_dir("start-permissions");
    let socket = dir.join("engine.sock");
    let seen = fake_engine(&socket);
    let mut server = server(&socket, &dir, "full");
    server.start();

    for (cmd, expected) in [
        (
            json!(["codex", "--ask-for-approval", "always", "--model", "fixture"]),
            json!(["codex", "--dangerously-bypass-approvals-and-sandbox", "--model", "fixture"]),
        ),
        (
            json!(["claude", "--permission-mode", "plan"]),
            json!(["claude", "--dangerously-skip-permissions"]),
        ),
    ] {
        let (failed, text) = server.call("nixa_term_start", json!({"cmd": cmd}));
        assert!(!failed, "{text}");
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["cmd"], expected);
    }

    let program = dir.join("claude");
    std::fs::File::create(&program).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let agents = json!({"agents": {"claude": {"run": [program, "--permission-mode", "plan"]}}});
    std::fs::write(dir.join("agents.json"), agents.to_string()).unwrap();
    let (failed, text) = server.call("nixa_agent_start", json!({"agent": "claude"}));
    assert!(!failed, "{text}");
    let started: Value = serde_json::from_str(&text).unwrap();
    let cmd = started["cmd"].as_array().unwrap();
    assert!(cmd.starts_with(&[json!(program), json!("--dangerously-skip-permissions")]));
    assert!(!cmd.contains(&json!("--permission-mode")));

    std::fs::write(dir.join("programs.json"), r#"{"checksOff":["claude"],"checksOn":["claude"]}"#).unwrap();
    let (failed, text) = server.call("nixa_term_start", json!({"cmd": ["claude", "--dangerously-skip-permissions"]}));
    assert!(!failed, "{text}");
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["cmd"], json!(["claude"]));
    server.stop();
    assert_eq!(seen.lock().unwrap().len(), 4);
    std::fs::remove_dir_all(dir).unwrap();
}
