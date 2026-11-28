use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REPLY: u64 = 16 * 1024 * 1024;
const REFUSED_PORTS: [u16; 3] = [7321, 7330, 7331];
const LOCAL_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

pub fn pick_addr(explicit: Option<String>) -> Result<String, String> {
    let addr = explicit.unwrap_or_else(nixa_protocol::paths::brain_addr);
    check_addr(addr.trim())
}

pub fn check_addr(addr: &str) -> Result<String, String> {
    let refuse = |why: &str| format!("{addr} is not the nixa-brain address of Nixa: {why}.");
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| refuse("it needs host:port"))?;
    let port: u16 = port
        .parse()
        .map_err(|_| refuse("the port is not a number"))?;
    if REFUSED_PORTS.contains(&port) {
        return Err(refuse("this port is reserved for another local app"));
    }
    if !LOCAL_HOSTS.contains(&host) {
        return Err(refuse("nixa-brain runs on this Mac only"));
    }
    Ok(addr.to_string())
}

pub fn token_in(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(nixa_protocol::paths::WINDOW_TOKEN_FILE)).ok()?;
    let token = text.trim();
    (!token.is_empty() && token.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| token.to_string())
}

pub fn read_tree(addr: &str, token: Option<&str>, name: &str) -> Result<Value, String> {
    let reply = call(addr, token, "GET", &format!("/trees/{name}"), None)?;
    reply
        .get("tree")
        .cloned()
        .ok_or_else(|| format!("Tree {name}: nixa-brain sent no tree."))
}

pub fn put_tree(
    addr: &str,
    token: Option<&str>,
    name: &str,
    tree: &Value,
) -> Result<Value, String> {
    let reply = call(
        addr,
        token,
        "PUT",
        &format!("/trees/{name}"),
        Some(&json!({"tree": tree})),
    )?;
    reply
        .get("tree")
        .cloned()
        .ok_or_else(|| format!("Tree {name}: nixa-brain sent no tree."))
}

fn call(
    addr: &str,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let addr = check_addr(addr)?;
    let target = addr
        .to_socket_addrs()
        .ok()
        .and_then(|mut all| all.next())
        .ok_or_else(|| format!("Bad nixa-brain address {addr}."))?;
    if !target.ip().is_loopback() {
        return Err(format!("{addr} does not point to this Mac."));
    }
    let fail = |e: std::io::Error| format!("nixa-brain at {addr}: {e}");
    let mut conn = TcpStream::connect_timeout(&target, TIMEOUT)
        .map_err(|e| format!("Cannot reach nixa-brain at {addr}: {e}"))?;
    conn.set_read_timeout(Some(TIMEOUT)).map_err(fail)?;
    conn.set_write_timeout(Some(TIMEOUT)).map_err(fail)?;
    let data = body.map(Value::to_string).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
        data.len(),
        token
            .map(|t| format!("{}: {t}\r\n", nixa_protocol::paths::WINDOW_TOKEN_HEADER))
            .unwrap_or_default()
    );
    conn.write_all(head.as_bytes()).map_err(fail)?;
    conn.write_all(data.as_bytes()).map_err(fail)?;
    let mut raw = Vec::new();
    conn.take(MAX_REPLY).read_to_end(&mut raw).map_err(fail)?;
    let (code, value) = parse_reply(&raw)?;
    if value.get("ok").and_then(Value::as_bool) == Some(false) || !(200..300).contains(&code) {
        return Err(value
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("nixa-brain answered {code}")));
    }
    Ok(value)
}

pub fn parse_reply(raw: &[u8]) -> Result<(u16, Value), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("nixa-brain sent a broken reply")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let code: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or("nixa-brain sent a broken status line")?;
    let chunked = head
        .lines()
        .any(|l| l.starts_with("transfer-encoding:") && l.contains("chunked"));
    let body = &raw[split + 4..];
    let body = if chunked {
        unchunk(body)?
    } else {
        body.to_vec()
    };
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok((code, Value::Null));
    }
    let value = serde_json::from_slice(&body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).trim().to_string()));
    Ok((code, value))
}

fn unchunk(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let end = body
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("nixa-brain sent a broken chunk")?;
        let size_text = String::from_utf8_lossy(&body[..end]).to_string();
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "nixa-brain sent a broken chunk size".to_string())?;
        body = &body[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size {
            return Err("nixa-brain sent a short chunk".to_string());
        }
        out.extend_from_slice(&body[..size]);
        body = body.get(size + 2..).unwrap_or(&[]);
    }
}

#[cfg(test)]
pub mod fake {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    pub struct Seen {
        pub head: String,
        pub body: String,
    }

    pub fn serve(replies: Vec<(u16, String)>) -> (String, mpsc::Receiver<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for (code, body) in replies {
                let Ok((mut conn, _)) = listener.accept() else {
                    return;
                };
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let (head, want) = loop {
                    let n = conn.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break (String::from_utf8_lossy(&raw).to_string(), 0);
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&raw[..at]).to_string();
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        raw.drain(..at + 4);
                        break (head, len);
                    }
                };
                while raw.len() < want {
                    let n = conn.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                }
                let _ = tx.send(Seen {
                    head,
                    body: String::from_utf8_lossy(&raw).to_string(),
                });
                let reply = format!(
                    "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = conn.write_all(reply.as_bytes());
            }
        });
        (addr, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_dev_addresses_pass() {
        for good in ["127.0.0.1:7421", "localhost:7425", "[::1]:7421"] {
            assert_eq!(check_addr(good).unwrap(), good);
        }
        for bad in [
            "127.0.0.1:7321",
            "127.0.0.1:7330",
            "localhost:7331",
            "10.0.0.2:7421",
            "0.0.0.0:7421",
            "127.0.0.1",
            "127.0.0.1:x",
        ] {
            assert!(
                check_addr(bad)
                    .unwrap_err()
                    .contains("is not the nixa-brain address of Nixa"),
                "{bad}"
            );
        }
        assert_eq!(
            pick_addr(Some(" 127.0.0.1:7425 ".to_string())).unwrap(),
            "127.0.0.1:7425"
        );
        assert!(pick_addr(Some("127.0.0.1:7321".to_string())).is_err());
    }

    #[test]
    fn plain_and_chunked_replies() {
        let (code, v) =
            parse_reply(b"HTTP/1.1 200 OK\r\ncontent-length: 11\r\n\r\n{\"ok\":true}").unwrap();
        assert_eq!((code, v), (200, json!({"ok": true})));
        let (code, v) = parse_reply(b"HTTP/1.1 400 Bad\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"ok\"\r\n7\r\n:false}\r\n0\r\n\r\n").unwrap();
        assert_eq!((code, v), (400, json!({"ok": false})));
        assert_eq!(
            parse_reply(b"HTTP/1.1 204 No\r\n\r\n").unwrap(),
            (204, Value::Null)
        );
        assert!(parse_reply(b"nothing").is_err());
        assert!(
            parse_reply(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nzz\r\n").is_err()
        );
    }

    #[test]
    fn read_and_put_go_to_the_tree_routes() {
        let tree = json!({"version": 1, "name": "t1", "boxes": {}});
        let (addr, seen) = fake::serve(vec![
            (200, json!({"ok": true, "tree": tree}).to_string()),
            (200, json!({"ok": true, "tree": tree}).to_string()),
            (
                400,
                json!({"ok": false, "error": "box r9 is missing"}).to_string(),
            ),
            (200, json!({"ok": true}).to_string()),
        ]);
        assert_eq!(read_tree(&addr, None, "t1").unwrap(), tree);
        let first = seen.recv().unwrap();
        assert!(
            !first
                .head
                .to_ascii_lowercase()
                .contains("x-nixa-window-token"),
            "{}",
            first.head
        );
        assert!(
            first.head.starts_with("GET /trees/t1 HTTP/1.1"),
            "{}",
            first.head
        );
        assert_eq!(put_tree(&addr, Some("tok123"), "t1", &tree).unwrap(), tree);
        let second = seen.recv().unwrap();
        assert!(
            second.head.contains("\r\nx-nixa-window-token: tok123\r\n"),
            "{}",
            second.head
        );
        assert!(
            second.head.starts_with("PUT /trees/t1 HTTP/1.1"),
            "{}",
            second.head
        );
        assert_eq!(
            serde_json::from_str::<Value>(&second.body).unwrap(),
            json!({"tree": tree})
        );
        assert_eq!(
            put_tree(&addr, Some("tok123"), "t1", &tree).unwrap_err(),
            "box r9 is missing"
        );
        assert!(
            read_tree(&addr, None, "t1")
                .unwrap_err()
                .contains("sent no tree")
        );
    }

    #[test]
    fn token_file_is_read_and_checked() {
        let dir = std::env::temp_dir().join(format!("mj-tok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(nixa_protocol::paths::WINDOW_TOKEN_FILE);
        assert_eq!(token_in(&dir), None);
        std::fs::write(&file, "abc123\n").unwrap();
        assert_eq!(token_in(&dir).as_deref(), Some("abc123"));
        std::fs::write(&file, "bad token\r\nx: y").unwrap();
        assert_eq!(token_in(&dir), None);
        std::fs::write(&file, "  ").unwrap();
        assert_eq!(token_in(&dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_brain_is_a_clear_error() {
        assert!(
            read_tree("127.0.0.1:1", None, "t1")
                .unwrap_err()
                .contains("Cannot reach nixa-brain")
        );
    }
}
