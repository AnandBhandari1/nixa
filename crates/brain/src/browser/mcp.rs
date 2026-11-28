use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub const MAX_LINE: usize = 32 * 1024 * 1024;
pub const PROTOCOL: &str = "2025-06-18";
pub const ENV_KEYS: [&str; 3] = ["HOME", "TMPDIR", "LANG"];
pub const EXTRA_PATH: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

pub struct Session {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u64,
    pub dir: PathBuf,
}

pub fn path_list(program: &Path) -> String {
    let mut dirs: Vec<String> = Vec::new();
    if let Some(parent) = program.parent().filter(|p| !p.as_os_str().is_empty()) {
        dirs.push(parent.display().to_string());
    }
    if let Ok(path) = std::env::var("PATH") {
        dirs.extend(path.split(':').filter(|d| !d.is_empty()).map(str::to_string));
    }
    dirs.extend(EXTRA_PATH.iter().map(|d| d.to_string()));
    let mut seen = Vec::new();
    for dir in dirs {
        if !seen.contains(&dir) {
            seen.push(dir);
        }
    }
    seen.join(":")
}

pub fn find_program(word: &str) -> Option<PathBuf> {
    if word.contains('/') {
        let path = PathBuf::from(word);
        return crate::cli_brains::is_executable(&path).then_some(path);
    }
    path_list(Path::new("")).split(':').map(|d| Path::new(d).join(word)).find(|p| crate::cli_brains::is_executable(p))
}

async fn read_line(output: &mut BufReader<ChildStdout>) -> Result<Option<Vec<u8>>, String> {
    let mut line = Vec::new();
    loop {
        let buf = output.fill_buf().await.map_err(|e| format!("The browser tool stopped: {e}"))?;
        if buf.is_empty() {
            return Ok(if line.is_empty() { None } else { Some(line) });
        }
        if let Some(end) = buf.iter().position(|b| *b == b'\n') {
            line.extend_from_slice(&buf[..end]);
            output.consume(end + 1);
            return Ok(Some(line));
        }
        let n = buf.len();
        line.extend_from_slice(buf);
        output.consume(n);
        if line.len() > MAX_LINE {
            return Err("The browser tool sent a reply that is too long.".into());
        }
    }
}

async fn send(session: &mut Session, message: &Value) -> Result<(), String> {
    let mut text = message.to_string();
    text.push('\n');
    session.input.write_all(text.as_bytes()).await.map_err(|e| format!("The browser tool closed: {e}"))?;
    session.input.flush().await.map_err(|e| format!("The browser tool closed: {e}"))
}

async fn request(session: &mut Session, method: &str, params: Value) -> Result<Value, String> {
    session.next += 1;
    let id = session.next;
    send(session, &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await?;
    loop {
        let Some(line) = read_line(&mut session.output).await? else { return Err("The browser tool closed.".into()) };
        let Ok(message) = serde_json::from_slice::<Value>(&line) else { continue };
        if message.get("method").is_some() {
            if let Some(asked) = message.get("id").cloned() {
                send(session, &json!({"jsonrpc": "2.0", "id": asked, "error": {"code": -32601, "message": "Not supported"}})).await?;
            }
            continue;
        }
        if message.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(error.get("message").and_then(Value::as_str).unwrap_or("The browser tool failed.").to_string());
        }
        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
    }
}

pub fn text_of(result: &Value) -> Result<String, String> {
    let parts: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter(|p| p.get("type").and_then(Value::as_str) == Some("text")).filter_map(|p| p.get("text").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    let text = parts.join("\n");
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(if text.is_empty() { "The browser tool failed.".into() } else { text });
    }
    Ok(text)
}

pub async fn start(launch: Launch, wait: Duration) -> Result<Session, String> {
    let mut command = Command::new(&launch.program);
    command.args(&launch.args).current_dir(&launch.cwd).env_clear().env("PATH", path_list(&launch.program));
    for key in ENV_KEYS {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).process_group(0);
    let mut child = command.spawn().map_err(|e| format!("The browser tool did not start ({}): {e}", launch.program.display()))?;
    let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.start_kill();
        return Err("The browser tool did not open its pipes.".into());
    };
    let mut session = Session { child, input, output: BufReader::new(output), next: 0, dir: launch.cwd };
    let hello = json!({"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "nixa", "version": env!("CARGO_PKG_VERSION")}});
    let ready = tokio::time::timeout(wait, async {
        request(&mut session, "initialize", hello).await?;
        send(&mut session, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await
    })
    .await;
    match ready {
        Ok(Ok(())) => Ok(session),
        Ok(Err(e)) => {
            close(session).await;
            Err(e)
        }
        Err(_) => {
            close(session).await;
            Err("The browser tool did not answer in time.".into())
        }
    }
}

pub async fn call(session: &mut Session, tool: &str, input: Value) -> Result<String, String> {
    let result = request(session, "tools/call", json!({"name": tool, "arguments": input})).await?;
    text_of(&result)
}

async fn signal_group(pid: Option<u32>, signal: &str) {
    if let Some(pid) = pid {
        let _ = Command::new("/bin/kill").args([signal, "--", &format!("-{pid}")]).stdout(Stdio::null()).stderr(Stdio::null()).status().await;
    }
}

pub async fn close(mut session: Session) {
    let pid = session.child.id();
    let _ = session.input.shutdown().await;
    if tokio::time::timeout(Duration::from_secs(3), session.child.wait()).await.is_err() {
        signal_group(pid, "-TERM").await;
        if tokio::time::timeout(Duration::from_secs(2), session.child.wait()).await.is_err() {
            let _ = session.child.start_kill();
        }
    }
    signal_group(pid, "-KILL").await;
    let _ = std::fs::remove_dir_all(&session.dir);
}
