use nixa_protocol::{
    ErrorCode, Event, PROTOCOL_VERSION, Reply, Request, Status, TermId, TermInfo, WaitOutcome, WaitResult, wire,
};
use std::io::{BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

type Seen = Arc<Mutex<Vec<Request>>>;

struct Fake {
    dir: PathBuf,
    socket: PathBuf,
    seen: Seen,
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn term(n: u64, name: &str) -> TermInfo {
    TermInfo {
        id: TermId::from_number(n),
        name: name.to_string(),
        labels: Default::default(),
        parent: None,
        children: vec![],
        cmd: vec!["claude".to_string()],
        cwd: "/tmp".to_string(),
        pid: Some(100 + n as u32),
        running: true,
        exit_code: None,
        status: Some(Status::Working),
        status_note: None,
        keep: false,
        draft: false,
        title: None,
        rows: 24,
        cols: 80,
        created_ms: 0,
        seq: 1,
    }
}

fn answer(req: &Request) -> Vec<Reply> {
    let m1 = TermId::from_number(1);
    match req {
        Request::Ping => vec![Reply::Pong { version: PROTOCOL_VERSION, pid: 4242 }],
        Request::List { .. } => vec![Reply::Terms { terms: vec![term(1, "claude"), term(2, "build")] }],
        Request::Get { id } if *id == m1 => vec![Reply::Term { term: term(1, "claude") }],
        Request::Get { id } => vec![Reply::not_found(id)],
        Request::Create { spec } if spec.parent == Some(TermId::from_number(2)) => vec![
            Reply::error(ErrorCode::BadRequest, "Cannot use m2 as a parent: it already has a parent. Only one level of sub-agents is allowed."),
        ],
        Request::Create { spec } => {
            let mut t = term(3, spec.name.as_deref().unwrap_or(""));
            t.cmd = spec.cmd.clone();
            t.parent = spec.parent.clone();
            vec![Reply::Term { term: t }]
        }
        Request::Read { id, .. } => vec![Reply::Text { id: id.clone(), text: "line one\nline two".to_string() }],
        Request::ChildOutput { child, .. } => vec![Reply::Text { id: child.clone(), text: "saved output".into() }],
        Request::Send { id, force, .. } => vec![Reply::Sent { id: id.clone(), waited_ms: 0, forced: *force, started: None, tail: String::new() }],
        Request::Wait { id, .. } => vec![Reply::Waited {
            result: WaitResult {
                id: id.clone(),
                outcome: WaitOutcome::Timeout,
                status: Some(Status::Working),
                matched: None,
                waited_ms: 50,
            },
        }],
        Request::Subscribe { .. } => vec![
            Reply::Subscribed,
            Reply::Event { event: Event::Status { id: m1.clone(), status: Some(Status::Done), note: Some("all good".to_string()) } },
            Reply::Event { event: Event::Exit { id: m1, code: Some(0) } },
        ],
        _ => vec![Reply::Ok],
    }
}

fn serve(conn: UnixStream, seen: Seen) {
    let Ok(mut writer) = conn.try_clone() else { return };
    let mut reader = BufReader::new(conn);
    while let Ok(Some(req)) = wire::read_request(&mut reader) {
        seen.lock().unwrap().push(req.clone());
        for reply in answer(&req) {
            if wire::write_reply(&mut writer, &reply).is_err() {
                return;
            }
        }
        if matches!(req, Request::Subscribe { .. }) {
            return;
        }
    }
}

fn fake() -> Fake {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("nixa-cli-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("e.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let seen: Seen = Arc::default();
    let s2 = seen.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { break };
            let seen = s2.clone();
            std::thread::spawn(move || serve(conn, seen));
        }
    });
    Fake { dir, socket, seen }
}

fn nixa(f: &Fake, args: &[&str], env: &[(&str, &str)], stdin: &str) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nixa"));
    cmd.arg("--socket")
        .arg(&f.socket)
        .args(args)
        .env_remove("NIXA_TERM_ID")
        .env_remove("NIXA_PARENT_ID")
        .env_remove("NIXA_ENGINE_SOCKET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

fn last(f: &Fake) -> Request {
    f.seen.lock().unwrap().last().cloned().unwrap()
}

#[test]
fn child_report_uses_its_parent_and_reads_archived_output() {
    let f = fake();
    let env = [("NIXA_TERM_ID", "m2"), ("NIXA_PARENT_ID", "m1")];
    let o = nixa(&f, &["term", "report", "checks", "passed"], &env, "");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(last(&f), Request::ChildReport {
        id: TermId::from_number(2),
        parent: Some(TermId::from_number(1)),
        text: "checks passed".into(),
    });
    let o = nixa(&f, &["term", "report", "-"], &[env[0]], "older child\n");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(last(&f), Request::ChildReport {
        id: TermId::from_number(2),
        parent: None,
        text: "older child\n".into(),
    });
    let o = nixa(&f, &["term", "reports", ".", "--output", "m2"], &[("NIXA_TERM_ID", "m1")], "");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(text(&o.stdout), "saved output\n");
    assert_eq!(last(&f), Request::ChildOutput { id: TermId::from_number(1), child: TermId::from_number(2) });
    let o = nixa(&f, &["term", "report", "outside"], &[], "");
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn ls_human_and_json() {
    let f = fake();
    let o = nixa(&f, &["term", "ls"], &[], "");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let out = text(&o.stdout);
    assert!(out.starts_with("ID  STATUS   RUN      NAME"), "{out}");
    assert!(out.contains("m2  working  running  build"), "{out}");
    let o = nixa(&f, &["term", "ls", "--json"], &[], "");
    let reply = wire::parse_reply(&text(&o.stdout)).unwrap();
    assert!(matches!(reply, Reply::Terms { terms } if terms.len() == 2));
}

#[test]
fn engine_errors_exit_one() {
    let f = fake();
    let o = nixa(&f, &["term", "get", "m9"], &[], "");
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("no terminal m9"));
    let o = nixa(&f, &["--json", "term", "get", "m9"], &[], "");
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stdout).contains("\"code\":\"not_found\""));
    let o = nixa(&f, &["term", "get", "q1"], &[], "");
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn status_uses_env_id_and_note() {
    let f = fake();
    let o = nixa(&f, &["term", "status", "blocked", "needs", "a", "yes"], &[("NIXA_TERM_ID", "m1")], "");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(text(&o.stdout), "m1 status blocked\n");
    assert_eq!(
        last(&f),
        Request::StatusSet { id: TermId::from_number(1), status: Some(Status::Blocked), note: Some("needs a yes".to_string()) }
    );
    nixa(&f, &["term", "status", "m2", "clear"], &[], "");
    assert_eq!(last(&f), Request::StatusSet { id: TermId::from_number(2), status: None, note: None });
    let o = nixa(&f, &["term", "status", "done"], &[], "");
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn run_sets_parent_cwd_and_cmd() {
    let f = fake();
    let o = nixa(&f, &["term", "run", "--name", "job", "-l", "role=test", "echo", "--hi"], &[("NIXA_TERM_ID", "m1")], "");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(text(&o.stdout), "m3\n");
    let Request::Create { spec } = last(&f) else { panic!("no create") };
    assert_eq!(spec.cmd, vec!["echo".to_string(), "--hi".to_string()]);
    assert_eq!(spec.parent, Some(TermId::from_number(1)));
    assert_eq!(spec.name.as_deref(), Some("job"));
    assert_eq!(spec.labels.get("role").map(String::as_str), Some("test"));
    assert_eq!(spec.cwd, std::env::current_dir().ok().map(|d| d.display().to_string()));
    let o = nixa(&f, &["term", "run"], &[], "");
    assert_eq!(o.status.code(), Some(2));
    nixa(&f, &["term", "new", "--no-parent"], &[("NIXA_TERM_ID", "m1")], "");
    let Request::Create { spec } = last(&f) else { panic!("no create") };
    assert!(spec.cmd.is_empty() && spec.parent.is_none());
}

#[test]
fn new_and_run_report_the_nested_parent_error() {
    let f = fake();
    for args in [
        &["term", "new", "--parent", "m2"][..],
        &["term", "run", "--parent", "m2", "--", "echo"][..],
        &["term", "new"][..],
    ] {
        let out = nixa(&f, args, &[("NIXA_TERM_ID", "m2")], "");
        assert_eq!(out.status.code(), Some(1));
        assert!(text(&out.stderr).contains("m2 as a parent: it already has a parent"));
        assert!(matches!(last(&f), Request::Create { spec } if spec.parent == Some(TermId::from_number(2))));
    }
}

#[test]
fn run_applies_the_configured_start_permissions() {
    let f = fake();
    let env = [("NIXA_DATA_DIR", f.dir.to_str().unwrap())];
    for (args, expected) in [
        (
            &["term", "run", "--", "codex", "--ask-for-approval", "always", "--model", "fixture"][..],
            &["codex", "--dangerously-bypass-approvals-and-sandbox", "--model", "fixture"][..],
        ),
        (
            &["term", "run", "--", "claude", "--permission-mode", "plan"][..],
            &["claude", "--dangerously-skip-permissions"][..],
        ),
    ] {
        let out = nixa(&f, args, &env, "");
        assert!(out.status.success(), "{}", text(&out.stderr));
        let Request::Create { spec } = last(&f) else { panic!("no create") };
        assert_eq!(spec.cmd, expected);
    }
    std::fs::write(f.dir.join("programs.json"), r#"{"checksOff":["claude"],"checksOn":["claude"]}"#).unwrap();
    let out = nixa(&f, &["term", "run", "--", "claude", "--dangerously-skip-permissions"], &env, "");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let Request::Create { spec } = last(&f) else { panic!("no create") };
    assert_eq!(spec.cmd, ["claude"]);
}

#[test]
fn parent_none_clears_the_relationship() {
    let f = fake();
    let out = nixa(&f, &["term", "parent", "m2", "--none"], &[], "");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "m2 has no parent\n");
    assert_eq!(last(&f), Request::ClearParent { id: TermId::from_number(2) });
    let out = nixa(&f, &["term", "parent", "m2"], &[], "");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(f.seen.lock().unwrap().len(), 1);
}

#[test]
fn send_reads_stdin_and_flags() {
    let f = fake();
    let o = nixa(&f, &["term", "send", "m1"], &[], "hello\n");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(text(&o.stdout), "m1 sent\n");
    let Request::Send { text: t, submit, force, .. } = last(&f) else { panic!("no send") };
    assert_eq!((t.as_str(), submit, force), ("hello\n", true, false));
    nixa(&f, &["term", "send", "m1", "draft", "text", "--no-submit"], &[], "");
    assert_eq!(
        last(&f),
        Request::Send { id: TermId::from_number(1), text: "draft text".to_string(), submit: false, timeout_ms: None, force: false }
    );
    nixa(&f, &["term", "send", "m1", "fix", "it", "-s", "-f", "-t", "5s"], &[], "");
    assert_eq!(
        last(&f),
        Request::Send { id: TermId::from_number(1), text: "fix it".to_string(), submit: true, timeout_ms: Some(5_000), force: true }
    );
}

#[test]
fn send_submit_reads_piped_stdin() {
    let f = fake();
    let o = nixa(&f, &["term", "send", "m1", "--submit"], &[], "echo hi");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(
        last(&f),
        Request::Send { id: TermId::from_number(1), text: "echo hi".to_string(), submit: true, timeout_ms: None, force: false }
    );
}

#[test]
fn send_huge_timeout_does_not_panic() {
    let f = fake();
    let max = u64::MAX.to_string();
    let o = nixa(&f, &["term", "send", "m1", "hi", "--timeout", &max], &[], "");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let Request::Send { timeout_ms, .. } = last(&f) else { panic!("no send") };
    assert_eq!(timeout_ms, Some(u64::MAX));
}

#[test]
fn keys_and_read() {
    let f = fake();
    let o = nixa(&f, &["term", "keys", "m1", "ctrl+c", "enter", "raw:y"], &[], "");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let Request::Keys { keys, .. } = last(&f) else { panic!("no keys") };
    assert_eq!(keys.len(), 3);
    let o = nixa(&f, &["term", "keys", "m1", "nope"], &[], "");
    assert_eq!(o.status.code(), Some(2));
    let o = nixa(&f, &["term", "read", "m1", "--history", "-n", "5"], &[], "");
    assert_eq!(text(&o.stdout), "line one\nline two\n");
}

#[test]
fn wait_timeout_exits_124() {
    let f = fake();
    let o = nixa(&f, &["term", "wait", "m1", "--text", "ready", "--timeout", "50ms"], &[], "");
    assert_eq!(o.status.code(), Some(124));
    assert_eq!(text(&o.stdout), "m1 timed out after 50ms\n");
    let o = nixa(&f, &["term", "wait", "m1", "--idle", "--exit"], &[], "");
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn events_stop_after_count() {
    let f = fake();
    let o = nixa(&f, &["term", "events", "m1", "--count", "1"], &[], "");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(text(&o.stdout), "m1 status done: all good\n");
    let o = nixa(&f, &["--json", "term", "events"], &[], "");
    let lines: Vec<String> = text(&o.stdout).lines().map(String::from).collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains("\"kind\":\"exit\""));
}

#[test]
fn engine_status_and_missing_engine() {
    let f = fake();
    let o = nixa(&f, &["engine", "status"], &[], "");
    assert_eq!(o.status.code(), Some(0));
    assert!(text(&o.stdout).contains("running, pid 4242, protocol 1, 2 terminals"));
    let gone = f.dir.join("none.sock");
    let o = Command::new(env!("CARGO_BIN_EXE_nixa"))
        .args(["--socket", gone.to_str().unwrap(), "term", "ls"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("nixa engine start"));
    let o = Command::new(env!("CARGO_BIN_EXE_nixa"))
        .args(["--socket", gone.to_str().unwrap(), "engine", "start", "--bin", "/usr/bin/true", "--wait", "3s"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("exited early"), "{}", text(&o.stderr));
}

#[test]
fn task_and_deep_show_help() {
    let f = fake();
    let o = nixa(&f, &["task"], &[], "");
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stdout).starts_with("nixa task:"));
    let o = nixa(&f, &["deep", "ask"], &[], "");
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stderr).contains("needs a question"));
    let o = nixa(&f, &["help", "deep"], &[], "");
    assert!(text(&o.stdout).contains("decide ID --pick"));
}
