use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("nixa-cli-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn nixa(dir: &PathBuf, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nixa"));
    cmd.args(args)
        .env("NIXA_DATA_DIR", dir)
        .env("NIXA_ENGINE_SOCKET", dir.join("none.sock"))
        .env_remove("NIXA_TERM_ID")
        .env_remove("NIXA_BRAIN_ADDR")
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

#[test]
fn task_add_change_and_list() {
    let dir = temp_dir("task");
    let o = nixa(&dir, &["task", "add", "Fix", "the", "chat", "--desc", "Chat drops lines"], &[]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).contains("#1 [todo] Fix the chat"), "{}", out(&o));
    let o = nixa(&dir, &["task", "status", "1", "todo", "--note", "ready"], &[]);
    assert!(out(&o).contains("[todo]"), "{}", err(&o));
    let o = nixa(&dir, &["task", "comment", "#1", "looks", "good", "--kind", "progress"], &[]);
    assert!(out(&o).contains("Comment added to #1"), "{}", err(&o));
    let o = nixa(&dir, &["task", "assign", "1", "m4"], &[]);
    assert!(out(&o).contains("for m4"), "{}", err(&o));
    let o = nixa(&dir, &["task", "show", "1"], &[]);
    let shown = out(&o);
    assert!(shown.contains("Chat drops lines") && shown.contains("(progress): looks good") && shown.contains("(note): ready"), "{shown}");
    let o = nixa(&dir, &["--json", "task", "ls", "--status", "todo"], &[]);
    let list: Value = serde_json::from_str(out(&o).trim()).unwrap();
    assert_eq!(list[0]["assigned_kind"], json!("agent"));
    let o = nixa(&dir, &["task", "ls", "--status", "done"], &[]);
    assert_eq!(out(&o).trim(), "No tasks.");
    let o = nixa(&dir, &["task", "status", "1", "maybe"], &[]);
    assert_eq!(o.status.code(), Some(2));
    let o = nixa(&dir, &["task", "assign", "1", "nobody"], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("not a terminal id"));
    let o = nixa(&dir, &["task", "ls", "--space", "missing"], &[]);
    assert!(err(&o).contains("No space named missing"));
    let o = nixa(&dir, &["task", "agents"], &[]);
    assert_eq!(out(&o).trim(), "No custom agents yet.", "{}", err(&o));
    let o = nixa(&dir, &["task", "done", "1", "all", "fixed"], &[]);
    assert!(out(&o).contains("[done]"), "{}", err(&o));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn task_claim_release_and_next() {
    let dir = temp_dir("claim");
    nixa(&dir, &["task", "add", "First", "job"], &[]);
    nixa(&dir, &["task", "add", "Second", "job"], &[]);
    let o = nixa(&dir, &["task", "claim", "1"], &[]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(err(&o).contains("needs a terminal"), "{}", err(&o));
    let o = nixa(&dir, &["task", "claim", "1", "--terminal", "m3"], &[]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).contains("Claimed #1 [doing] First job") && out(&o).contains("in m3"), "{}", out(&o));
    let o = nixa(&dir, &["task", "claim", "1", "--terminal", "m4"], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("claimed by m3") && err(&o).contains("Nothing was claimed"), "{}", err(&o));
    let o = nixa(&dir, &["task", "release", "1", "--note", "not now"], &[]);
    assert!(out(&o).contains("Released #1 [todo]"), "{}", err(&o));
    let o = nixa(&dir, &["task", "show", "1"], &[]);
    assert!(out(&o).contains("(note): not now"), "{}", out(&o));
    let o = nixa(&dir, &["task", "status", "2", "blocked", "--on", "user"], &[]);
    assert!(out(&o).contains("blocked on decision"), "{}", err(&o));
    assert!(out(&o).contains("does not wait for the user"), "{}", out(&o));
    let o = nixa(&dir, &["task", "status", "2", "blocked", "--on", "nobody"], &[]);
    assert_eq!(o.status.code(), Some(2));
    let known = "Use user, decision, task, other";
    assert!(err(&o).contains(known), "{}", err(&o));
    let o = nixa(&dir, &["task", "next", "--space", "missing"], &[]);
    assert!(err(&o).contains("No space named missing"), "{}", err(&o));
    let o = nixa(&dir, &["help", "task"], &[]);
    assert!(out(&o).contains("--on user|task|other"), "{}", out(&o));
    std::fs::remove_dir_all(&dir).ok();
}

fn fake_brain(reply: Value, status: &str) -> (String, mpsc::Receiver<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    let status = status.to_string();
    std::thread::spawn(move || {
        let Ok((conn, _)) = listener.accept() else { return };
        let mut reader = BufReader::new(conn.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
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
            first.push_str(&line);
        }
        let mut body = vec![0; len];
        reader.read_exact(&mut body).unwrap();
        let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        tx.send((first.trim().to_string(), value)).unwrap();
        let text = reply.to_string();
        let mut conn = conn;
        write!(conn, "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{text}", text.len()).unwrap();
    });
    (addr, rx)
}

#[test]
fn deep_decide_posts_the_answer() {
    let dir = temp_dir("deep");
    let (addr, rx) = fake_brain(json!({"ok": true, "ask": {"id": "d3"}}), "200 OK");
    let o = nixa(&dir, &["deep", "decide", "d3", "--pick", "2", "--why", "tests first", "--learn", "nixa: tests"], &[("NIXA_BRAIN_ADDR", &addr), ("NIXA_TERM_ID", "m5")]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(out(&o).trim(), "Answered d3.");
    let (line, body) = rx.recv().unwrap();
    assert!(line.starts_with("POST /deep/d3/reply "), "{line}");
    assert_eq!(body, json!({"terminal": "m5", "pick": "2", "why": "tests first", "learn": "nixa: tests"}));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn deep_join_and_errors() {
    let dir = temp_dir("join");
    let seat = json!({"ok": true, "seat": {"id": "nixa", "name": "claude", "spaceName": "nixa"}});
    let (addr, rx) = fake_brain(seat, "200 OK");
    let o = nixa(&dir, &["deep", "join", "--as", "nixa"], &[("NIXA_BRAIN_ADDR", &addr), ("NIXA_TERM_ID", "m9")]);
    assert_eq!(out(&o).trim(), "Joined: deep brain nixa (claude) of space nixa.", "{}", err(&o));
    let (line, body) = rx.recv().unwrap();
    assert!(line.starts_with("POST /deep/join "));
    assert_eq!(body, json!({"terminal": "m9", "as": "nixa"}));
    let (addr, _rx) = fake_brain(json!({"ok": false, "error": "Terminal m9 does not run."}), "403 Forbidden");
    let o = nixa(&dir, &["deep", "leave"], &[("NIXA_BRAIN_ADDR", &addr), ("NIXA_TERM_ID", "m9")]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("Terminal m9 does not run."), "{}", err(&o));
    let o = nixa(&dir, &["deep", "join"], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("NIXA_TERM_ID"));
    let o = nixa(&dir, &["deep", "decide", "d3", "--why", "x"], &[("NIXA_TERM_ID", "m9")]);
    assert_eq!(o.status.code(), Some(2));
    let o = nixa(&dir, &["deep", "decide", "../x", "--pick", "1", "--why", "x"], &[("NIXA_TERM_ID", "m9")]);
    assert_eq!(o.status.code(), Some(2));
    let o = nixa(&dir, &["deep", "ls"], &[("NIXA_BRAIN_ADDR", "127.0.0.1:7321")]);
    assert_eq!(o.status.code(), Some(1));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn deep_ask_start_and_digest_keep_the_caller_and_private_token() {
    let dir = temp_dir("deep-access");
    std::fs::write(dir.join(nixa_protocol::paths::WINDOW_TOKEN_FILE), "testtoken123").unwrap();
    for (args, path) in [
        (vec!["deep", "ask", "Which agent?", "--space", "shop"], "/deep/ask"),
        (vec!["deep", "start", "--space", "shop", "--brain", "codex"], "/deep/start"),
        (vec!["deep", "digest", "--space", "shop", "--json"], "/deep/digest"),
    ] {
        let (addr, rx) = fake_brain(json!({"ok": true, "text": "digest", "ask": {"id": "d1"}}), "200 OK");
        let o = nixa(&dir, &args, &[("NIXA_BRAIN_ADDR", &addr), ("NIXA_TERM_ID", "m9")]);
        assert_eq!(o.status.code(), Some(0), "{}", err(&o));
        let (head, body) = rx.recv().unwrap();
        assert!(head.starts_with(&format!("POST {path} ")), "{head}");
        assert!(!head.contains("x-nixa-window-token") && !head.contains("testtoken123"), "{head}");
        assert_eq!(body["terminal"], "m9");
        assert_eq!(body["space"], "shop");
        if path == "/deep/digest" {
            assert_eq!(serde_json::from_str::<Value>(out(&o).trim()).unwrap()["text"], "digest");
        }
    }
    let (addr, rx) = fake_brain(json!({"ok": true, "text": "Rules: 11 rules."}), "200 OK");
    let o = nixa(&dir, &["deep", "digest"], &[("NIXA_BRAIN_ADDR", &addr)]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(out(&o).trim(), "Rules: 11 rules.");
    let (head, body) = rx.recv().unwrap();
    assert!(head.contains("x-nixa-window-token: testtoken123"), "{head}");
    assert!(body.get("terminal").is_none());
    std::fs::remove_dir_all(&dir).ok();
}
