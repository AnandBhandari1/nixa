use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REPLY: u64 = 16 * 1024 * 1024;
const REFUSED_PORTS: [u16; 3] = [7321, 7330, 7331];
const LOCAL_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

pub fn brain_addr() -> Result<String> {
    check_addr(nixa_protocol::paths::brain_addr().trim())
}

pub fn check_addr(addr: &str) -> Result<String> {
    let refuse = |why: &str| {
        anyhow!(
            "{addr} is not the nixa-brain address of Nixa: {why}. Set NIXA_BRAIN_ADDR, default {}",
            nixa_protocol::paths::DEFAULT_BRAIN_ADDR
        )
    };
    let (host, port) = addr.rsplit_once(':').ok_or_else(|| refuse("it needs host:port"))?;
    let port: u16 = port.parse().map_err(|_| refuse("the port is not a number"))?;
    if REFUSED_PORTS.contains(&port) {
        return Err(refuse("this port is reserved for another local app"));
    }
    if !LOCAL_HOSTS.contains(&host) {
        return Err(refuse("nixa-brain runs on this Mac only, use 127.0.0.1"));
    }
    Ok(addr.to_string())
}

pub fn get(path: &str) -> Result<Value> {
    call("GET", path, None, TIMEOUT)
}

pub fn get_within(path: &str, timeout: Duration) -> Result<Value> {
    call("GET", path, None, timeout)
}

pub fn post(path: &str, body: &Value) -> Result<Value> {
    call("POST", path, Some(body), TIMEOUT)
}

pub fn post_within(path: &str, body: &Value, timeout: Duration) -> Result<Value> {
    call("POST", path, Some(body), timeout)
}

pub fn notify_within(path: &str, body: &Value, timeout: Duration) {
    let Ok(addr) = brain_addr() else { return };
    let Some(target) = addr.to_socket_addrs().ok().and_then(|mut all| all.next()) else { return };
    if !target.ip().is_loopback() {
        return;
    }
    let Ok(mut conn) = TcpStream::connect_timeout(&target, timeout) else { return };
    let _ = conn.set_nodelay(true);
    let _ = conn.set_read_timeout(Some(timeout));
    let _ = conn.set_write_timeout(Some(timeout));
    let data = body.to_string();
    let auth = auth_lines(false);
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nContent-Type: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{data}",
        data.len()
    );
    if conn.write_all(req.as_bytes()).is_ok() {
        let mut head = [0u8; 16];
        let _ = conn.read(&mut head);
    }
}

pub fn auth_lines(window: bool) -> String {
    let mut lines = nixa_protocol::paths::gate_token()
        .map(|token| format!("{}: {token}\r\n", nixa_protocol::paths::GATE_TOKEN_HEADER))
        .unwrap_or_default();
    if window && let Some(token) = nixa_protocol::paths::window_token() {
        lines.push_str(&format!("{}: {token}\r\n", nixa_protocol::paths::WINDOW_TOKEN_HEADER));
    }
    lines
}

pub fn window_call(method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    send(method, path, body, TIMEOUT, true)
}

pub fn window_call_within(method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value> {
    send(method, path, body, timeout, true)
}

fn call(method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value> {
    let wants = matches!(path, "/deep/ask" | "/deep/start" | "/deep/digest" | "/agent/tools" | "/agent/tool");
    send(method, path, body, timeout, wants && !in_terminal())
}

fn in_terminal() -> bool {
    std::env::var_os(nixa_protocol::paths::ENV_TERM_ID).is_some_and(|v| !v.is_empty())
}

fn send(method: &str, path: &str, body: Option<&Value>, timeout: Duration, token: bool) -> Result<Value> {
    send_as(in_terminal(), brain_addr, method, path, body, timeout, token)
}

fn send_as(terminal: bool, addr: impl FnOnce() -> Result<String>, method: &str, path: &str, body: Option<&Value>, timeout: Duration, token: bool) -> Result<Value> {
    if token && terminal {
        bail!("{method} {path} needs the Nixa window. Agent terminals (NIXA_TERM_ID is set) do not get window rights. Ask the user to do it in the Nixa window.");
    }
    let addr = addr()?;
    let target = addr
        .to_socket_addrs()
        .ok()
        .and_then(|mut all| all.next())
        .ok_or_else(|| anyhow!("bad nixa-brain address {addr}"))?;
    if !target.ip().is_loopback() {
        bail!("{addr} does not point to this Mac, nixa-brain is not called there");
    }
    let mut conn = TcpStream::connect_timeout(&target, timeout)
        .map_err(|e| anyhow!("cannot reach nixa-brain at {addr}: {e}\nStart it with: nixa brain start"))?;
    let _ = conn.set_nodelay(true);
    conn.set_read_timeout(Some(timeout))?;
    conn.set_write_timeout(Some(timeout))?;
    let data = body.map(Value::to_string).unwrap_or_default();
    let auth = auth_lines(token);
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nContent-Type: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n",
        data.len()
    );
    conn.write_all(head.as_bytes())?;
    conn.write_all(data.as_bytes())?;
    let mut raw = Vec::new();
    conn.take(MAX_REPLY).read_to_end(&mut raw)?;
    let (code, value) = parse_reply(&raw)?;
    if value.get("ok").and_then(Value::as_bool) == Some(false) || !(200..300).contains(&code) {
        let message = value
            .get("error")
            .or_else(|| value.get("reason"))
            .or_else(|| value.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("nixa-brain answered {code}"));
        bail!(message);
    }
    Ok(value)
}

pub fn parse_reply(raw: &[u8]) -> Result<(u16, Value)> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("nixa-brain sent a broken reply"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let body = &raw[split + 4..];
    let code: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| anyhow!("nixa-brain sent a broken status line"))?;
    let chunked = head
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"));
    let body = if chunked { unchunk(body)? } else { body.to_vec() };
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok((code, Value::Null));
    }
    let value = serde_json::from_slice(&body).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).to_string()));
    Ok((code, value))
}

fn unchunk(mut body: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = body
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| anyhow!("nixa-brain sent a broken chunk"))?;
        let size_text = String::from_utf8_lossy(&body[..end]).to_string();
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| anyhow!("nixa-brain sent a broken chunk size"))?;
        body = &body[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size {
            bail!("nixa-brain sent a short chunk");
        }
        out.extend_from_slice(&body[..size]);
        body = body.get(size + 2..).unwrap_or(&[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_local_dev_addresses_pass() {
        for good in ["127.0.0.1:7421", "localhost:7422", "[::1]:7421"] {
            assert_eq!(check_addr(good).unwrap(), good);
        }
        for bad in ["127.0.0.1:7321", "127.0.0.1:7330", "localhost:7331", "192.168.1.20:7421", "0.0.0.0:7421", "127.0.0.1", "127.0.0.1:x", "127.0.0.1:99999"] {
            let err = check_addr(bad).unwrap_err().to_string();
            assert!(err.contains("is not the nixa-brain address of Nixa"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_terminal_never_sends_the_window_token() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let body = json!({"action": "new_key"});
        let err = send_as(true, || Ok(addr.clone()), "POST", "/phone", Some(&body), Duration::from_secs(2), true).unwrap_err().to_string();
        assert!(err.contains("NIXA_TERM_ID"), "{err}");
        assert!(listener.accept().is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock), "a request was sent");
    }

    #[test]
    fn plain_and_chunked_replies() {
        let (code, v) = parse_reply(b"HTTP/1.1 200 OK\r\ncontent-length: 11\r\n\r\n{\"ok\":true}").unwrap();
        assert_eq!((code, v), (200, json!({"ok": true})));
        let (code, v) = parse_reply(b"HTTP/1.1 400 Bad Request\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"ok\"\r\n7\r\n:false}\r\n0\r\n\r\n").unwrap();
        assert_eq!((code, v), (400, json!({"ok": false})));
        assert!(parse_reply(b"nothing").is_err());
    }
}
