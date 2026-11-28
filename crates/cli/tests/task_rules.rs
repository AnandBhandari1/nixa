use nixa_protocol::{Reply, Request, Status, TermId, TermInfo, wire};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("nixa-rules-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn term(n: u64, pid: u32) -> TermInfo {
    TermInfo {
        id: TermId::from_number(n),
        name: format!("agent {n}"),
        labels: Default::default(),
        parent: None,
        children: vec![],
        cmd: vec!["claude".to_string()],
        cwd: "/tmp".to_string(),
        pid: Some(pid),
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

fn serve(conn: UnixStream, terms: Vec<TermInfo>) {
    let Ok(mut writer) = conn.try_clone() else { return };
    let mut reader = BufReader::new(conn);
    while let Ok(Some(req)) = wire::read_request(&mut reader) {
        let reply = match req {
            Request::List { .. } => Reply::Terms { terms: terms.clone() },
            _ => Reply::Ok,
        };
        if wire::write_reply(&mut writer, &reply).is_err() {
            return;
        }
    }
}

fn engine(dir: &Path, terms: Vec<TermInfo>) -> PathBuf {
    let socket = dir.join("e.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { break };
            let terms = terms.clone();
            std::thread::spawn(move || serve(conn, terms));
        }
    });
    socket
}

fn fake_brain(reply: Value) -> (String, mpsc::Receiver<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((conn, _)) = listener.accept() else { return };
        let mut reader = BufReader::new(conn.try_clone().unwrap());
        let mut head = String::new();
        reader.read_line(&mut head).unwrap();
        let mut len = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
            head.push_str(&line);
        }
        let mut body = vec![0; len];
        reader.read_exact(&mut body).unwrap();
        let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        tx.send((head, value)).unwrap();
        let text = reply.to_string();
        let mut conn = conn;
        write!(conn, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{text}", text.len()).unwrap();
    });
    (addr, rx)
}

fn nixa(dir: &Path, socket: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nixa"));
    cmd.args(args)
        .env("NIXA_DATA_DIR", dir)
        .env("NIXA_ENGINE_SOCKET", socket)
        .env("NIXA_BRAIN_ADDR", "127.0.0.1:9")
        .env_remove("NIXA_TERM_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn ok(o: &Output) -> String {
    assert_eq!(o.status.code(), Some(0), "{}", err(o));
    out(o)
}

fn refused(o: &Output, text: &str) {
    assert_eq!(o.status.code(), Some(1), "{}", out(o));
    assert!(err(o).contains(text), "{}", err(o));
}

#[test]
fn agents_follow_the_task_rules() {
    let dir = temp_dir("agent");
    let live = engine(&dir, vec![term(1, std::process::id()), term(2, 1)]);
    let none = dir.join("none.sock");
    let user = |args: &[&str]| nixa(&dir, &none, args, &[]);
    let m1 = |args: &[&str]| nixa(&dir, &live, args, &[("NIXA_TERM_ID", "m1")]);
    assert!(ok(&user(&["task", "add", "Build", "it"])).contains("#1 [todo] Build it"));
    assert!(ok(&user(&["task", "add", "Other"])).contains("#2 [todo] Other"));
    assert!(ok(&user(&["task", "claim", "2", "--terminal", "m2"])).contains("in m2"));
    assert!(ok(&m1(&["task", "claim", "1"])).contains("Claimed #1 [doing] Build it"));
    assert!(ok(&m1(&["task", "link", "1", "--branch", "feat/x"])).contains("branch feat/x"));
    assert!(ok(&m1(&["task", "files", "claim", "1", "src/a.rs"])).contains("Held for task #1 in terminal m1: src/a.rs."));
    assert!(ok(&user(&["task", "files"])).contains("#1 in m1: src/a.rs"));
    assert!(ok(&m1(&["task", "files", "release", "1"])).contains("Freed 1 held file list for task #1 in terminal m1."));
    refused(&m1(&["task", "status", "1", "review"]), "needs a result note");
    assert!(ok(&m1(&["task", "status", "1", "review", "--note", "Tests pass"])).contains("[review]"));
    refused(&m1(&["task", "release", "2"]), "not yours. Terminal m2 holds it");
    refused(&m1(&["task", "link", "2", "--pr", "https://example.com/pr/1"]), "not yours");
    refused(&m1(&["task", "claim", "2", "--terminal", "m3"]), "only for its own terminal m1");
    refused(&nixa(&dir, &live, &["task", "done", "2"], &[]), "not yours");
    assert!(ok(&user(&["task", "done", "1", "checked"])).contains("[done]"));
    refused(&m1(&["task", "status", "1", "doing"]), "closed task");
    refused(&m1(&["task", "assign", "1", "m4"]), "closed task");
    assert!(ok(&m1(&["task", "comment", "1", "late", "note"])).contains("Comment added to #1"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_named_terminal_is_checked() {
    let dir = temp_dir("check");
    let live = engine(&dir, vec![term(1, std::process::id()), term(2, 1)]);
    let none = dir.join("none.sock");
    assert!(ok(&nixa(&dir, &none, &["task", "add", "Build"], &[])).contains("#1 [todo]"));
    refused(&nixa(&dir, &live, &["task", "comment", "1", "hi"], &[("NIXA_TERM_ID", "m2")]), "does not come from a program inside terminal m2");
    refused(&nixa(&dir, &live, &["task", "comment", "1", "hi"], &[("NIXA_TERM_ID", "m7")]), "not an open Nixa terminal");
    refused(&nixa(&dir, &none, &["task", "comment", "1", "hi"], &[("NIXA_TERM_ID", "m1")]), "nixa-engine does not answer");
    assert!(ok(&nixa(&dir, &live, &["task", "comment", "1", "hi"], &[])).contains("Comment added to #1"));
    let shown = ok(&nixa(&dir, &none, &["task", "show", "1"], &[]));
    assert!(shown.contains("- m1 (note): hi"), "{shown}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn step_plans_and_workflows_check_the_caller() {
    let dir = temp_dir("plan");
    let live = engine(&dir, vec![term(1, std::process::id())]);
    let none = dir.join("none.sock");
    let user = |args: &[&str]| nixa(&dir, &none, args, &[]);
    let m1 = |args: &[&str]| nixa(&dir, &live, args, &[("NIXA_TERM_ID", "m1")]);
    ok(&user(&["task", "add", "Release"]));
    assert!(ok(&user(&["task", "plan", "1"])).contains("Task #1 has no step plan."));
    assert!(ok(&m1(&["task", "plan", "1"])).contains("Task #1 has no step plan."));
    assert!(ok(&user(&["task", "plan", "1", "--clear"])).contains("Task #1 had no step plan."));
    refused(&user(&["task", "plan", "1", "--step", "user | Read the brief", "--run"]), "cannot reach nixa-brain");
    let o = user(&["task", "plan", "1", "--done", "0"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(ok(&user(&["task", "workflow", "1"])).contains("No workflow on this task."));
    refused(&user(&["task", "workflow", "1", "set", "nope"]), "There is no workflow named nope");
    refused(&user(&["task", "workflow", "1", "pause"]), "has no workflow run");
    refused(&m1(&["task", "agents", "save", "--terminal", "m2"]), "saves only its own terminal m1");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn automatic_lets_agents_take_the_step() {
    let dir = temp_dir("auto");
    let live = engine(&dir, vec![term(1, std::process::id())]);
    let none = dir.join("none.sock");
    let user = |args: &[&str]| nixa(&dir, &none, args, &[]);
    let m1 = |args: &[&str]| nixa(&dir, &live, args, &[("NIXA_TERM_ID", "m1")]);
    ok(&user(&["task", "add", "Build"]));
    ok(&m1(&["task", "claim", "1"]));
    let done = ok(&m1(&["task", "done", "1", "all", "good"]));
    assert!(done.contains("[review]") && done.contains("Nixa checks the work"), "{done}");
    assert!(ok(&m1(&["task", "plan", "1", "--clear"])).contains("Task #1 had no step plan."));
    refused(&m1(&["task", "workflow", "1", "pause"]), "has no workflow run");
    refused(&m1(&["task", "trees", "delete", "watch"]), "needs the Nixa window");
    assert!(ok(&user(&["task", "done", "1", "checked"])).contains("[done]"));
    refused(&m1(&["task", "assign", "1", "m4"]), "nixa deep ask");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn only_user_calls_carry_the_window_token() {
    let dir = temp_dir("token");
    std::fs::write(dir.join(nixa_protocol::paths::WINDOW_TOKEN_FILE), "testtoken123").unwrap();
    let live = engine(&dir, vec![term(1, std::process::id())]);
    let none = dir.join("none.sock");
    ok(&nixa(&dir, &none, &["task", "add", "Release"], &[]));
    let (addr, rx) = fake_brain(json!({ "ok": true }));
    let o = nixa(&dir, &none, &["task", "trees", "delete", "watch"], &[("NIXA_BRAIN_ADDR", &addr)]);
    assert!(ok(&o).contains("Deleted rule tree watch."));
    let (head, _) = rx.recv().unwrap();
    assert!(head.starts_with("DELETE /trees/watch ") && head.contains("x-nixa-window-token: testtoken123"), "{head}");
    let (addr, rx) = fake_brain(json!({ "ok": true, "text": "Step 1 is done." }));
    let o = nixa(&dir, &live, &["task", "plan", "1", "--done", "1"], &[("NIXA_BRAIN_ADDR", &addr), ("NIXA_TERM_ID", "m1")]);
    assert_eq!(ok(&o).trim(), "Step 1 is done.");
    let (head, body) = rx.recv().unwrap();
    assert!(head.starts_with("POST /tasks/1/plan ") && !head.contains("x-nixa-window-token"), "{head}");
    assert_eq!(body, json!({ "done": [1], "terminal": "m1" }));
    let (addr, rx) = fake_brain(json!({ "ok": true }));
    let o = nixa(&dir, &none, &["task", "plan", "1", "--step", "user | Read it"], &[("NIXA_BRAIN_ADDR", &addr)]);
    assert!(ok(&o).contains("Task #1: plan saved."));
    let (head, body) = rx.recv().unwrap();
    assert!(head.starts_with("POST /tasks/1/plan ") && head.contains("x-nixa-window-token: testtoken123"), "{head}");
    assert_eq!(body, json!({ "steps": ["user | Read it"] }));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn task_edit_follows_the_task_rules() {
    let dir = temp_dir("edit");
    let live = engine(&dir, vec![term(1, std::process::id())]);
    let none = dir.join("none.sock");
    let user = |args: &[&str]| nixa(&dir, &none, args, &[]);
    let m1 = |args: &[&str]| nixa(&dir, &live, args, &[("NIXA_TERM_ID", "m1")]);
    for (folder, name) in [("open-place", "home"), ("shut-place", "shop")] {
        let path = dir.join(folder);
        std::fs::create_dir_all(&path).unwrap();
        ok(&user(&["space", "add", &path.to_string_lossy(), "--name", name]));
    }
    let conn = nixa_store::open(&dir.join(nixa_store::DB_FILE)).unwrap();
    let shop = nixa_store::spaces::list_spaces(&conn).unwrap().into_iter().find(|s| s.name == "shop").unwrap();
    nixa_store::spaces::set_space(&conn, &shop.space_id, &nixa_store::spaces::SpacePatch { locked: Some(true), ..Default::default() }).unwrap();
    ok(&user(&["task", "add", "Old", "title", "--space", "home"]));
    let o = user(&["task", "edit", "1"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    let edited = ok(&user(&["task", "edit", "1", "--title", "New title", "--desc", "More words"]));
    assert!(edited.contains("#1 [todo] New title"), "{edited}");
    assert!(ok(&user(&["task", "show", "1"])).contains("More words"));
    ok(&m1(&["task", "claim", "1"]));
    let kept = ok(&m1(&["task", "edit", "1", "--title", "Agent title", "--space", "shop"]));
    assert!(kept.contains("Agent title") && kept.contains("space home") && kept.contains("shop is locked"), "{kept}");
    assert!(ok(&user(&["task", "edit", "1", "--space", "shop"])).contains("shop is locked"));
    ok(&user(&["task", "done", "1", "checked"]));
    refused(&m1(&["task", "edit", "1", "--title", "Late"]), "closed task");
    std::fs::remove_dir_all(&dir).ok();
}
