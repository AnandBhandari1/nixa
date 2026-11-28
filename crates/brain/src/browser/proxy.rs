use crate::web::safety::{public_lookup, public_url};
use reqwest::Url;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

pub const HEAD_MAX: usize = 32 * 1024;
pub const CONNECT_MS: u64 = 15_000;
pub const HEAD_MS: u64 = 15_000;
pub const IDLE_MS: u64 = 30_000;
pub const BLOCKED: &str = "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 34\r\nConnection: close\r\n\r\nPrivate network access is blocked.";
pub const DROP_HEADERS: [&str; 5] = ["connection", "proxy-connection", "keep-alive", "proxy-authorization", "host"];

pub struct Proxy {
    pub url: String,
    pub stop: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub head: Duration,
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { head: Duration::from_millis(HEAD_MS), idle: Duration::from_millis(IDLE_MS) }
    }
}

pub async fn start() -> Result<Proxy, String> {
    start_with(Limits::default()).await
}

pub async fn start_with(limits: Limits) -> Result<Proxy, String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| format!("The browser guard did not start: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let stop = CancellationToken::new();
    let token = stop.clone();
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = token.cancelled() => return,
                a = listener.accept() => a,
            };
            let Ok((client, _)) = accepted else { continue };
            let token = token.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = token.cancelled() => {}
                    _ = serve(client, limits) => {}
                }
            });
        }
    });
    Ok(Proxy { url: format!("http://127.0.0.1:{port}"), stop })
}

pub fn close(proxy: &Proxy) {
    proxy.stop.cancel();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Tunnel { host: String },
    Plain { host: String, head: Vec<u8> },
}

fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

pub fn parse_head(head: &str) -> Result<Ask, String> {
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut words = first.split(' ');
    let (Some(method), Some(target), Some(version)) = (words.next(), words.next(), words.next()) else { return Err("bad request".into()) };
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = target.rsplit_once(':').ok_or("bad target")?;
        if port != "443" {
            return Err("port".into());
        }
        let url = public_url(&format!("https://{host}/"))?;
        return Ok(Ask::Tunnel { host: url.host_str().unwrap_or("").to_string() });
    }
    let url = Url::parse(target).map_err(|_| "bad target".to_string())?;
    if url.scheme() != "http" || url.port().is_some_and(|p| p != 80) || !url.username().is_empty() || url.password().is_some() {
        return Err("HTTPS requires CONNECT".into());
    }
    let host = public_url(&format!("http://{}/", url.host_str().unwrap_or("")))?.host_str().unwrap_or("").to_string();
    let mut path = url.path().to_string();
    if let Some(q) = url.query() {
        path.push('?');
        path.push_str(q);
    }
    let mut out = format!("{method} {path} {version}\r\nHost: {host}\r\n");
    for line in lines.filter(|l| !l.is_empty()) {
        let name = line.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        if !DROP_HEADERS.contains(&name.as_str()) {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    out.push_str("Connection: close\r\n\r\n");
    Ok(Ask::Plain { host, head: out.into_bytes() })
}

async fn reach(addr: SocketAddr) -> Result<TcpStream, String> {
    let connected = tokio::time::timeout(Duration::from_millis(CONNECT_MS), TcpStream::connect(addr)).await;
    match connected {
        Ok(Ok(stream)) => Ok(stream),
        _ => Err("upstream".into()),
    }
}

async fn read_head(client: &mut TcpStream) -> Result<(Vec<u8>, usize), bool> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = head_end(&buf) {
            return Ok((buf, end));
        }
        if buf.len() > HEAD_MAX {
            return Err(true);
        }
        match client.read(&mut chunk).await {
            Ok(0) | Err(_) => return Err(false),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

fn touch(seen: &AtomicU64, start: Instant) {
    seen.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
}

fn quiet_for(seen: &AtomicU64, start: Instant) -> Duration {
    start.elapsed().saturating_sub(Duration::from_millis(seen.load(Ordering::Relaxed)))
}

async fn one_way(mut from: OwnedReadHalf, mut to: OwnedWriteHalf, seen: Arc<AtomicU64>, start: Instant, idle: Duration) {
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        let n = match tokio::time::timeout(idle, from.read(&mut chunk)).await {
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => n,
            Err(_) if quiet_for(&seen, start) >= idle => break,
            Err(_) => continue,
        };
        touch(&seen, start);
        match tokio::time::timeout(idle, to.write_all(&chunk[..n])).await {
            Ok(Ok(())) => touch(&seen, start),
            _ => break,
        }
    }
    let _ = to.shutdown().await;
}

pub async fn pump(client: TcpStream, upstream: TcpStream, idle: Duration) {
    let start = Instant::now();
    let seen = Arc::new(AtomicU64::new(0));
    let (client_read, client_write) = client.into_split();
    let (upstream_read, upstream_write) = upstream.into_split();
    tokio::join!(
        one_way(client_read, upstream_write, seen.clone(), start, idle),
        one_way(upstream_read, client_write, seen, start, idle),
    );
}

async fn serve(mut client: TcpStream, limits: Limits) {
    let (buf, end) = match tokio::time::timeout(limits.head, read_head(&mut client)).await {
        Ok(Ok(found)) => found,
        Ok(Err(true)) => {
            let _ = client.write_all(BLOCKED.as_bytes()).await;
            return;
        }
        Ok(Err(false)) | Err(_) => return,
    };
    let head = String::from_utf8_lossy(&buf[..end - 4]).into_owned();
    let rest = buf[end..].to_vec();
    let Ok(ask) = parse_head(&head) else {
        let _ = client.write_all(BLOCKED.as_bytes()).await;
        return;
    };
    let (host, port) = match &ask {
        Ask::Tunnel { host } => (host.clone(), 443),
        Ask::Plain { host, .. } => (host.clone(), 80),
    };
    let upstream = match public_lookup(&host, port).await {
        Ok(addr) => reach(addr).await,
        Err(e) => Err(e),
    };
    let Ok(mut upstream) = upstream else {
        let _ = client.write_all(BLOCKED.as_bytes()).await;
        return;
    };
    let sent = match &ask {
        Ask::Tunnel { .. } => client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await,
        Ask::Plain { head, .. } => upstream.write_all(head).await,
    };
    if sent.is_err() || (!rest.is_empty() && upstream.write_all(&rest).await.is_err()) {
        return;
    }
    pump(client, upstream, limits.idle).await;
}
