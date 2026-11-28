use crate::public::{check_host, percent_decode, public_lookup, public_text};
use anyhow::{Result, bail};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

const HEAD_MAX: usize = 64 * 1024;
const HEAD_WAIT: Duration = Duration::from_secs(15);
const CONNECT_WAIT: Duration = Duration::from_secs(15);
const METHODS: [&str; 3] = ["GET", "HEAD", "CONNECT"];
const LONGEST: Duration = Duration::from_secs(600);
const BLOCKED: &str = "Private network access is blocked.";
const DROPPED: [&str; 5] = ["host", "connection", "proxy-connection", "proxy-authorization", "keep-alive"];

pub struct PublicProxy {
    pub url: String,
    stop: CancellationToken,
}

impl PublicProxy {
    pub fn close(&self) {
        self.stop.cancel();
    }
}

impl Drop for PublicProxy {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    pub target: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub authority: String,
    pub path: String,
}

pub async fn start_public_proxy() -> Result<PublicProxy> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let stop = CancellationToken::new();
    let token = stop.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                got = listener.accept() => {
                    let Ok((client, _)) = got else {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    };
                    let token = token.clone();
                    tokio::spawn(async move {
                        tokio::select! {
                            _ = token.cancelled() => {}
                            _ = tokio::time::timeout(LONGEST, handle(client)) => {}
                        }
                    });
                }
            }
        }
    });
    Ok(PublicProxy { url: format!("http://127.0.0.1:{port}"), stop })
}

fn head_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|w| w == b"\r\n\r\n").map(|at| at + 4)
}

async fn read_head(client: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = head_end(&buf) {
            let rest = buf.split_off(end);
            return Ok((buf, rest));
        }
        if buf.len() > HEAD_MAX {
            bail!("request head too long");
        }
        let n = tokio::time::timeout(HEAD_WAIT, client.read(&mut chunk)).await??;
        if n == 0 {
            bail!("closed");
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

pub fn parse_head(raw: &[u8]) -> Option<Head> {
    let text = std::str::from_utf8(raw).ok()?;
    let bare = text.replace("\r\n", "");
    if bare.contains('\r') || bare.contains('\n') {
        return None;
    }
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target, version) = (first.next()?, first.next()?, first.next()?);
    if first.next().is_some() || !METHODS.contains(&method) || !version.starts_with("HTTP/1.") {
        return None;
    }
    let mut headers = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (name, value) = line.split_once(':')?;
        if name.is_empty() || name.contains(' ') {
            return None;
        }
        headers.push((name.to_string(), value.trim().to_string()));
    }
    Some(Head { method: method.to_string(), target: target.to_string(), version: version.to_string(), headers })
}

fn split_authority(authority: &str) -> Result<(String, Option<u16>)> {
    if authority.is_empty() || authority.contains('@') {
        bail!("bad address");
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(|| anyhow::anyhow!("bad address"))?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p.parse()?),
            None if after.is_empty() => None,
            None => bail!("bad address"),
        };
        return Ok((format!("[{host}]"), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Ok((host.to_string(), Some(port.parse()?))),
        None => Ok((authority.to_string(), None)),
    }
}

pub fn http_target(target: &str) -> Result<Target> {
    let lower = target.get(..7).map(|s| s.to_ascii_lowercase());
    if lower.as_deref() != Some("http://") {
        bail!("HTTPS needs CONNECT");
    }
    let rest = &target[7..];
    let (authority, path) = match rest.find(['/', '?']) {
        Some(at) if rest.as_bytes()[at] == b'/' => (&rest[..at], rest[at..].to_string()),
        Some(at) => (&rest[..at], format!("/{}", &rest[at..])),
        None => (rest, "/".to_string()),
    };
    let (host, port) = split_authority(authority)?;
    if port.is_some_and(|p| p != 80) {
        bail!("only port 80");
    }
    check_host(&host)?;
    let path = path.split('#').next().unwrap_or("/").to_string();
    public_text(&percent_decode(&path))?;
    Ok(Target { host, port: 80, authority: authority.to_string(), path })
}

pub fn connect_target(target: &str) -> Result<Target> {
    let (host, port) = split_authority(target)?;
    if port.is_some_and(|p| p != 443) {
        bail!("only port 443");
    }
    check_host(&host)?;
    Ok(Target { host, port: 443, authority: target.to_string(), path: String::new() })
}

pub fn origin_head(head: &Head, target: &Target) -> String {
    let mut out = format!("{} {} {}\r\n", head.method, target.path, head.version);
    for (name, value) in &head.headers {
        if !DROPPED.contains(&name.to_ascii_lowercase().as_str()) {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    out.push_str(&format!("Host: {}\r\nConnection: close\r\n\r\n", target.authority));
    out
}

async fn reply(client: &mut TcpStream, status: &str, body: &str) {
    let text = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = client.write_all(text.as_bytes()).await;
    let _ = client.shutdown().await;
}

async fn dial(target: &Target) -> Result<TcpStream, &'static str> {
    let addr = public_lookup(&target.host, target.port).await.map_err(|_| "403 Forbidden")?;
    match tokio::time::timeout(CONNECT_WAIT, TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => Ok(stream),
        _ => Err("502 Bad Gateway"),
    }
}

async fn handle(mut client: TcpStream) {
    let Ok((raw, rest)) = read_head(&mut client).await else {
        return;
    };
    let Some(head) = parse_head(&raw) else {
        return reply(&mut client, "400 Bad Request", "Bad request.").await;
    };
    let is_connect = head.method.eq_ignore_ascii_case("CONNECT");
    let target = if is_connect { connect_target(&head.target) } else { http_target(&head.target) };
    let Ok(target) = target else {
        return reply(&mut client, "403 Forbidden", BLOCKED).await;
    };
    let mut upstream = match dial(&target).await {
        Ok(stream) => stream,
        Err(status) => return reply(&mut client, status, if status.starts_with("403") { BLOCKED } else { "" }).await,
    };
    if is_connect {
        if client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.is_err() {
            return;
        }
    } else if upstream.write_all(origin_head(&head, &target).as_bytes()).await.is_err() {
        return reply(&mut client, "502 Bad Gateway", "").await;
    }
    if !rest.is_empty() && upstream.write_all(&rest).await.is_err() {
        return;
    }
    let _ = copy_bidirectional(&mut client, &mut upstream).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_are_parsed() {
        let head = parse_head(b"GET http://example.com/a?b=1 HTTP/1.1\r\nHost: example.com\r\nProxy-Connection: keep-alive\r\nAccept: */*\r\n\r\n").unwrap();
        assert_eq!(head.method, "GET");
        assert_eq!(head.headers.len(), 3);
        assert!(parse_head(b"GET / HTTP/2\r\n\r\n").is_none());
        assert!(parse_head(b"GET /\r\n\r\n").is_none());
        assert!(parse_head(b"GET / HTTP/1.1\r\nbad header\r\n\r\n").is_none());
        assert!(parse_head(b"GET http://example.com/ HTTP/1.1\nHost: example.com\r\n\r\n").is_none());
        assert!(parse_head(b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\nX-Smuggle: 1\r\n\r\n").is_none());
        assert!(parse_head(b"GET http://example.com/ HTTP/1.1\r\nHost: a\rb\r\n\r\n").is_none());
        assert!(parse_head(b"POST http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n").is_none());
        assert!(parse_head(b"get http://example.com/ HTTP/1.1\r\n\r\n").is_none());
        assert!(parse_head(b"CONNECT example.com:443 HTTP/1.1\r\n\r\n").is_some());
        let target = http_target(&head.target).unwrap();
        assert_eq!(target, Target { host: "example.com".into(), port: 80, authority: "example.com".into(), path: "/a?b=1".into() });
        let out = origin_head(&head, &target);
        assert!(out.starts_with("GET /a?b=1 HTTP/1.1\r\n"));
        assert!(out.contains("Accept: */*\r\n"));
        assert!(!out.to_ascii_lowercase().contains("proxy-connection"));
        assert!(out.ends_with("Host: example.com\r\nConnection: close\r\n\r\n"));
    }

    #[test]
    fn private_targets_are_refused() {
        assert!(http_target("http://example.com").is_ok());
        assert!(http_target("http://example.com?q=1").unwrap().path == "/?q=1");
        assert!(http_target("https://example.com/").is_err());
        assert!(http_target("http://example.com:8080/").is_err());
        assert!(http_target("http://user@example.com/").is_err());
        assert!(http_target("http://127.0.0.1/").is_err());
        assert!(http_target("http://router.local/").is_err());
        assert!(http_target("http://example.com/?q=my%20password").is_err());
        assert!(connect_target("example.com:443").is_ok());
        assert!(connect_target("example.com").is_ok());
        assert!(connect_target("example.com:22").is_err());
        assert!(connect_target("10.0.0.1:443").is_err());
        assert!(connect_target("[::1]:443").is_err());
    }

    #[tokio::test]
    async fn the_proxy_refuses_private_places() {
        let proxy = start_public_proxy().await.unwrap();
        let addr = proxy.url.trim_start_matches("http://").to_string();
        for request in ["GET http://127.0.0.1/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", "CONNECT 192.168.1.1:443 HTTP/1.1\r\n\r\n"] {
            let mut stream = TcpStream::connect(&addr).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut out = String::new();
            stream.read_to_string(&mut out).await.unwrap();
            assert!(out.starts_with("HTTP/1.1 403"), "{out}");
            assert!(out.contains(BLOCKED));
        }
        proxy.close();
    }
}
