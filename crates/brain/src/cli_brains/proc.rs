use super::process_groups::{Group, Groups};
use crate::util::{clip_flat, log};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

pub const LINE_MAX: u64 = 16 * 1024 * 1024;
pub const TAIL_LINES: usize = 12;
pub const CLI_ENV: [&str; 9] = ["CODEX_HOME", "CLAUDE_CONFIG_DIR", "CURSOR_API_KEY", "CURSOR_AUTH_TOKEN", "AGENT_CLI_CREDENTIAL_STORE", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"];

pub fn cli_env_ok(name: &str) -> bool {
    crate::review_flow::checks::env_ok(name, &CLI_ENV) || (name.starts_with("NIXA_") && !name.starts_with("NIXA_BRAIN_KEY_"))
}

pub fn cli_env() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    std::env::vars_os().filter(|(k, _)| k.to_str().is_some_and(cli_env_ok)).collect()
}

#[derive(Debug, Clone, Default)]
pub struct Spec {
    pub bin: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub stdin: bool,
}

#[derive(Debug)]
pub enum Event {
    Message(Value),
    Ended(Option<i32>),
}

type Waiters = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

#[derive(Clone)]
pub struct Rpc {
    name: &'static str,
    out: mpsc::UnboundedSender<String>,
    waiters: Waiters,
    next: Arc<AtomicU64>,
}

pub struct Proc {
    pub rpc: Rpc,
    pub events: mpsc::UnboundedReceiver<Event>,
    group: Group,
    pub tail: Arc<Mutex<VecDeque<String>>>,
}

fn waiters_lock(waiters: &Waiters) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Result<Value, String>>>> {
    waiters.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_reply(message: &Value) -> bool {
    message.get("method").is_none() && message.get("id").is_some() && (message.get("result").is_some() || message.get("error").is_some())
}

pub fn error_text(error: &Value, name: &str) -> String {
    error.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("{name} gave an error."))
}

impl Rpc {
    pub fn write(&self, message: &Value) {
        let _ = self.out.send(message.to_string());
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    pub fn reply(&self, id: &Value, result: Value) {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    pub fn reply_error(&self, id: &Value, message: &str) {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}}));
    }

    pub async fn request(&self, method: &str, params: Value, ms: u64) -> Result<Value, String> {
        if self.out.is_closed() {
            return Err(format!("The {} brain is not running.", self.name));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        waiters_lock(&self.waiters).insert(id, tx);
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        match tokio::time::timeout(Duration::from_millis(ms), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(format!("The {} brain stopped.", self.name)),
            Err(_) => {
                waiters_lock(&self.waiters).remove(&id);
                Err(format!("The {} brain did not answer {method} in time.", self.name))
            }
        }
    }

    pub fn settle(&self, message: &Value) -> bool {
        if !is_reply(message) {
            return false;
        }
        let Some(id) = message.get("id").and_then(Value::as_u64) else { return true };
        let Some(waiter) = waiters_lock(&self.waiters).remove(&id) else { return true };
        let result = match message.get("error") {
            Some(error) if !error.is_null() => Err(error_text(error, self.name)),
            _ => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = waiter.send(result);
        true
    }

    pub fn drop_all(&self) {
        waiters_lock(&self.waiters).clear();
    }
}

pub fn tail_text(tail: &Mutex<VecDeque<String>>) -> String {
    let lines: Vec<String> = tail.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect();
    clip_flat(&lines.join(" "), 300)
}

pub(super) fn spawn(groups: &Groups, name: &'static str, spec: &Spec, rpc: bool) -> Result<Proc, String> {
    start(groups, name, spec, rpc, None)
}

pub(super) fn spawn_with_input(groups: &Groups, name: &'static str, spec: &Spec, input: String) -> Result<Proc, String> {
    start(groups, name, spec, false, Some(input))
}

fn start(groups: &Groups, name: &'static str, spec: &Spec, rpc: bool, input: Option<String>) -> Result<Proc, String> {
    let mut command = Command::new(&spec.bin);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(cli_env())
        .stdin(if spec.stdin { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    let (mut child, group) = groups.spawn(&mut command).map_err(|e| format!("{name} could not run ({}): {e}", spec.bin.display()))?;
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (event_tx, events) = mpsc::unbounded_channel();
    let waiters: Waiters = Arc::new(Mutex::new(HashMap::new()));
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let link = Rpc { name, out: out_tx, waiters: waiters.clone(), next: Arc::new(AtomicU64::new(0)) };
    if let Some(text) = input {
        if let Some(mut stdin) = child.stdin.take() {
            tokio::spawn(async move {
                let _ = stdin.write_all(text.as_bytes()).await;
                let _ = stdin.shutdown().await;
            });
        }
    } else if let Some(mut stdin) = child.stdin.take() {
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.write_all(b"\n").await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let tail = tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                tail.push_back(clip_flat(&line, 300));
                while tail.len() > TAIL_LINES {
                    tail.pop_front();
                }
            }
        });
    }
    let stdout = child.stdout.take();
    let reader_link = link.clone();
    let reader_events = event_tx.clone();
    let reader = tokio::spawn(async move {
        let Some(stdout) = stdout else { return };
        let mut reader = BufReader::new(stdout);
        loop {
            let mut buf = Vec::new();
            let read = (&mut reader).take(LINE_MAX + 1).read_until(b'\n', &mut buf).await;
            match read {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if buf.len() as u64 > LINE_MAX {
                log("cli-brain", &format!("{name} sent a line over the size limit"));
                break;
            }
            let text = String::from_utf8_lossy(&buf);
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let Ok(message) = serde_json::from_str::<Value>(text) else { continue };
            if rpc && reader_link.settle(&message) {
                continue;
            }
            if reader_events.send(Event::Message(message)).is_err() {
                break;
            }
        }
    });
    let child_group = group.clone();
    tokio::spawn(async move {
        let status = child.wait().await.ok().and_then(|s| s.code());
        drop(child_group);
        let _ = reader.await;
        waiters_lock(&waiters).clear();
        let _ = event_tx.send(Event::Ended(status));
    });
    Ok(Proc { rpc: link, events, group, tail })
}

impl Proc {
    pub(super) fn stop(&self) {
        self.group.stop();
    }
}

pub(super) async fn run_once(groups: &Groups, name: &'static str, spec: &Spec, ms: u64) -> Result<(Option<i32>, String), String> {
    let mut command = Command::new(&spec.bin);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(cli_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    let (child, _group) = groups.spawn(&mut command).map_err(|e| format!("{name} could not run ({}): {e}", spec.bin.display()))?;
    match tokio::time::timeout(Duration::from_millis(ms), child.wait_with_output()).await {
        Ok(Ok(out)) => Ok((out.status.code(), String::from_utf8_lossy(&out.stdout).to_string())),
        Ok(Err(e)) => Err(format!("{name} failed: {e}")),
        Err(_) => Err(format!("{name} did not answer in time.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Spec {
        Spec { bin: "/bin/sh".into(), args: vec!["-c".into(), script.into()], cwd: std::env::temp_dir(), env: Vec::new(), stdin: true }
    }

    #[tokio::test]
    async fn request_and_reply_over_lines() {
        let script = "read line; echo '{\"note\":1}'; echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}'; read rest";
        let mut proc = spawn(&Groups::default(), "Test", &sh(script), true).unwrap();
        let got = proc.rpc.request("hello", json!({}), 5000).await.unwrap();
        assert_eq!(got["ok"], true);
        match proc.events.recv().await {
            Some(Event::Message(m)) => assert_eq!(m["note"], 1),
            other => panic!("{other:?}"),
        }
        proc.stop();
        loop {
            if let Some(Event::Ended(_)) = proc.events.recv().await {
                break;
            }
        }
    }

    #[tokio::test]
    async fn error_reply_and_end_drop_waiters() {
        let script = "read line; echo '{\"id\":1,\"error\":{\"message\":\"nope\"}}'; read line";
        let proc = spawn(&Groups::default(), "Test", &sh(script), true).unwrap();
        assert_eq!(proc.rpc.request("a", json!({}), 5000).await.unwrap_err(), "nope");
        let ended = proc.rpc.request("b", json!({}), 5000).await.unwrap_err();
        assert!(ended.contains("stopped"), "{ended}");
    }

    #[tokio::test]
    async fn times_out_and_runs_once() {
        let proc = spawn(&Groups::default(), "Test", &sh("sleep 5"), true).unwrap();
        let late = proc.rpc.request("a", json!({}), 100).await.unwrap_err();
        assert!(late.contains("in time"));
        proc.stop();
        let (code, out) = run_once(&Groups::default(), "Test", &sh("echo hi; exit 3"), 5000).await.unwrap();
        assert_eq!(code, Some(3));
        assert_eq!(out.trim(), "hi");
        assert!(run_once(&Groups::default(), "Test", &sh("sleep 5"), 100).await.is_err());
    }

    #[test]
    fn knows_replies() {
        assert!(is_reply(&json!({"id": 1, "result": null})));
        assert!(!is_reply(&json!({"id": 1, "method": "x"})));
        assert!(!is_reply(&json!({"method": "x"})));
    }
}
