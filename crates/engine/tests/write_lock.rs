use nixa_protocol::{Client, CreateSpec, ErrorCode, ReadSource, Reply, Request, TermId, WaitFor, WaitOutcome, connect};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread;
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(30);
const ASK: Duration = Duration::from_secs(5);

struct Daemon {
    child: Child,
    dir: PathBuf,
    socket: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn start(tag: &str) -> Daemon {
    let dir = PathBuf::from(format!("/tmp/mo-el-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("e.sock");
    let child = Command::new(env!("CARGO_BIN_EXE_nixa-engine"))
        .arg("--socket")
        .arg(&socket)
        .env("SHELL", "/bin/sh")
        .env_remove("NIXA_TERM_ID")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let daemon = Daemon { child, dir, socket };
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if let Ok(mut c) = connect(&daemon.socket)
            && c.set_timeout(Some(Duration::from_secs(1))).is_ok()
            && matches!(c.call(&Request::Ping), Ok(Reply::Pong { .. }))
        {
            return daemon;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("engine did not start");
}

fn client(socket: &Path, timeout: Duration) -> Client {
    let c = connect(socket).unwrap();
    c.set_timeout(Some(timeout)).unwrap();
    c
}

fn call(d: &Daemon, req: Request, timeout: Duration) -> Reply {
    client(&d.socket, timeout).request(&req).unwrap()
}

fn script(d: &Daemon, name: &str, body: &str) -> String {
    let path = d.dir.join(name);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

fn create(d: &Daemon, prog: String) -> TermId {
    let spec = CreateSpec {
        cmd: vec![prog],
        cwd: Some(d.dir.display().to_string()),
        ..CreateSpec::default()
    };
    match call(d, Request::Create { spec }, ASK) {
        Reply::Term { term } => term.id,
        other => panic!("create failed: {other:?}"),
    }
}

fn wait_text(d: &Daemon, id: &TermId, pattern: &str, ms: u64) -> WaitOutcome {
    let req = Request::Wait {
        id: id.clone(),
        until: WaitFor::Text {
            pattern: pattern.to_string(),
            regex: true,
        },
        timeout_ms: Some(ms),
    };
    match call(d, req, Duration::from_millis(ms) + ASK) {
        Reply::Waited { result } => result.outcome,
        other => panic!("wait failed: {other:?}"),
    }
}

fn still_answers(d: &Daemon, id: &TermId) {
    match call(d, Request::List { parent: None, label: None }, ASK) {
        Reply::Terms { terms } => assert!(terms.iter().any(|t| &t.id == id)),
        other => panic!("list failed: {other:?}"),
    }
    let req = Request::Read {
        id: id.clone(),
        source: ReadSource::Screen,
        lines: None,
    };
    assert!(matches!(call(d, req, ASK), Reply::Text { .. }));
}

fn long_text(lines: usize) -> String {
    let mut text: Vec<String> = (0..lines).map(|n| format!("line-{n} {}", "x".repeat(70))).collect();
    text.push("done-mark".to_string());
    text.join("\n")
}

fn seq_of(d: &Daemon, id: &TermId) -> u64 {
    match call(d, Request::Get { id: id.clone() }, ASK) {
        Reply::Term { term } => term.seq,
        other => panic!("get failed: {other:?}"),
    }
}

fn history(d: &Daemon, id: &TermId) -> String {
    let req = Request::Read {
        id: id.clone(),
        source: ReadSource::History,
        lines: None,
    };
    match call(d, req, ASK) {
        Reply::Text { text, .. } => text,
        other => panic!("read failed: {other:?}"),
    }
}

fn until_write_blocks(d: &Daemon, id: &TermId, rx: &Receiver<Reply>) {
    let until = Instant::now() + DEADLINE;
    let mut last = seq_of(d, id);
    let mut still_since = Instant::now();
    while Instant::now() < until {
        thread::sleep(Duration::from_millis(100));
        if let Ok(reply) = rx.try_recv() {
            panic!("the send ended before its write blocked: {reply:?}");
        }
        let seq = seq_of(d, id);
        if seq != last {
            last = seq;
            still_since = Instant::now();
            continue;
        }
        let text = history(d, id);
        if text.contains("line-") && !text.contains("done-mark") && still_since.elapsed() >= Duration::from_millis(700) {
            return;
        }
    }
    panic!("the send never blocked on a full pty input buffer");
}

fn send_in_thread(d: &Daemon, id: &TermId, text: String) -> Receiver<Reply> {
    let socket = d.socket.clone();
    let id = id.clone();
    let (tx, rx) = channel();
    thread::spawn(move || {
        let req = Request::Send {
            id,
            text,
            submit: true,
            timeout_ms: Some(3000),
            force: false,
        };
        if let Ok(reply) = client(&socket, DEADLINE * 2).request(&req) {
            let _ = tx.send(reply);
        }
    });
    rx
}

#[test]
fn long_send_to_echo_program_does_not_hang() {
    let d = start("echo");
    let prog = script(
        &d,
        "echo-agent.sh",
        "#!/bin/sh\nprintf '%s\\n' ready\nwhile IFS= read -r line; do printf 'got %s\\n' \"$line\"; done\n",
    );
    let id = create(&d, prog);
    assert_eq!(wait_text(&d, &id, "^ready$", 8000), WaitOutcome::Met);
    let rx = send_in_thread(&d, &id, long_text(2000));
    let reply = rx.recv_timeout(DEADLINE).expect("send did not return: the engine hung");
    assert!(matches!(reply, Reply::Sent { .. }), "{reply:?}");
    still_answers(&d, &id);
    assert_eq!(wait_text(&d, &id, "^got done-mark$", 20000), WaitOutcome::Met);
    still_answers(&d, &id);
}

#[test]
fn engine_answers_while_program_does_not_read() {
    let d = start("stuck");
    let prog = script(&d, "stuck-agent.sh", "#!/bin/sh\nprintf '%s\\n' ready\nexec sleep 60\n");
    let id = create(&d, prog);
    assert_eq!(wait_text(&d, &id, "^ready$", 8000), WaitOutcome::Met);
    let rx = send_in_thread(&d, &id, long_text(2000));
    until_write_blocks(&d, &id, &rx);
    still_answers(&d, &id);
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)), "the send ended before the kill");
    assert!(matches!(call(&d, Request::Kill { id: id.clone() }, ASK * 2), Reply::Ok));
    rx.recv_timeout(DEADLINE).expect("send did not return after the program ended");
    still_answers(&d, &id);
}

#[test]
fn submit_does_not_press_enter_after_a_person_typed() {
    let d = start("typed");
    let id = create(&d, "/bin/sh".to_string());
    assert_eq!(wait_text(&d, &id, "\\$", 8000), WaitOutcome::Met);
    for n in 0..5 {
        let rx = send_in_thread(&d, &id, format!("echo hi{n}"));
        thread::sleep(Duration::from_millis(20));
        let typed = Request::Input {
            id: id.clone(),
            text: "zz".to_string(),
        };
        assert!(matches!(call(&d, typed, ASK), Reply::Ok));
        let reply = rx.recv_timeout(DEADLINE).expect("send did not return");
        assert!(
            matches!(reply, Reply::Sent { .. } | Reply::Error { code: ErrorCode::DraftBusy, .. }),
            "{reply:?}"
        );
        let interrupt = Request::Interrupt { id: id.clone() };
        assert!(matches!(call(&d, interrupt, ASK), Reply::Interrupted { .. }));
    }
    thread::sleep(Duration::from_millis(300));
    let text = history(&d, &id);
    assert!(!text.lines().any(|l| l.trim().starts_with("hi") && l.trim().ends_with("zz")), "{text}");
}
