use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("nixa-agent-tools-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

fn nixa(dir: &Path, addr: &str, args: &[&str], term: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nixa"));
    cmd.args(args)
        .env("NIXA_DATA_DIR", dir)
        .env("NIXA_ENGINE_SOCKET", dir.join("none.sock"))
        .env("NIXA_BRAIN_ADDR", addr)
        .env_remove("NIXA_TERM_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(term) = term {
        cmd.env("NIXA_TERM_ID", term);
    }
    cmd.output().unwrap()
}

fn sent(dir: &Path, args: &[&str], term: Option<&str>) -> (String, String, Value) {
    let (addr, rx) = fake_brain(json!({"ok": true, "text": "found it"}));
    let o = nixa(dir, &addr, args, term);
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    assert_eq!(o.status.code(), Some(0), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
    let (head, body) = rx.recv().unwrap();
    (out, head, body)
}

#[test]
fn chat_history_memory_and_knowledge_use_the_agent_tools() {
    let dir = temp_dir("calls");
    let cases: [(&[&str], Value); 7] = [
        (&["chat", "history", "search", "the", "build", "--who", "user", "--limit", "5"], json!({"name": "nixa_chat", "args": {"action": "search", "query": "the build", "who": "user", "limit": 5}})),
        (&["chat", "history", "get", "42"], json!({"name": "nixa_chat", "args": {"action": "get", "id": 42}})),
        (&["chat", "history", "page", "--after", "10", "--limit", "3"], json!({"name": "nixa_chat", "args": {"action": "list", "after": 10, "limit": 3}})),
        (&["chat", "search", "release", "notes", "--source", "docs", "--project", "web"], json!({"name": "search_knowledge", "args": {"query": "release notes", "source": "docs", "project": "web"}})),
        (&["chat", "memory", "search", "tea"], json!({"name": "search_memory", "args": {"query": "tea"}})),
        (&["chat", "memory", "search"], json!({"name": "search_memory", "args": {"query": ""}})),
        (&["chat", "memory", "forget", "likes", "tea"], json!({"name": "forget", "args": {"text": "likes tea"}})),
    ];
    for (args, want) in cases {
        let (out, head, body) = sent(&dir, args, None);
        assert!(head.starts_with("POST /agent/tool "), "{args:?}: {head}");
        assert_eq!(body, want, "{args:?}");
        assert_eq!(out.trim(), "found it", "{args:?}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn agent_terminals_call_without_window_rights() {
    let dir = temp_dir("gate");
    std::fs::write(dir.join(nixa_protocol::paths::WINDOW_TOKEN_FILE), "testtoken123").unwrap();
    let (_, user_head, _) = sent(&dir, &["chat", "memory", "forget", "tea"], None);
    assert!(user_head.contains("x-nixa-window-token: testtoken123"), "{user_head}");
    let (_, agent_head, _) = sent(&dir, &["chat", "memory", "forget", "tea"], Some("m1"));
    assert!(!agent_head.to_ascii_lowercase().contains("window-token"), "{agent_head}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn bad_input_is_refused_before_any_call() {
    let dir = temp_dir("bad");
    for args in [
        &["chat", "history", "search"][..],
        &["chat", "history", "get", "x"][..],
        &["chat", "search"][..],
        &["chat", "search", "a", "--source", "web"][..],
        &["chat", "memory", "forget"][..],
        &["chat", "memory", "forget", "tea", "--all"][..],
        &["chat", "memory", "search", "tea", "--who", "user"][..],
        &["chat", "history", "search", "tea", "--source", "docs"][..],
    ] {
        let o = nixa(&dir, "127.0.0.1:9", args, None);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }
    std::fs::remove_dir_all(&dir).ok();
}
