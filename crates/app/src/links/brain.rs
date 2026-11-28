use anyhow::{Result, anyhow, bail};
use nixa_protocol::paths;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

pub const NOT_CONNECTED: &str = "Nixa brain not connected";
pub const STOPPED: &str = "The brain link was stopped.";
pub const TIMEOUT: Duration = Duration::from_secs(15);
pub const RETRY: Duration = Duration::from_secs(3);
const TICK: Duration = Duration::from_millis(5);
const PING_EVERY: Duration = Duration::from_secs(5);
const DEAD_AFTER: Duration = Duration::from_secs(15);
const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const MAX_HTTP: u64 = 16 * 1024 * 1024;
const REFUSED_PORTS: [u16; 3] = [7321, 7330, 7331];
const LOCAL_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

#[derive(Debug, Clone, PartialEq)]
pub enum BrainMsg {
    Up(Value),
    Down(String),
    Event(Value),
}

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub call: Duration,
    pub retry: Duration,
    pub ping_every: Duration,
    pub dead_after: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            call: TIMEOUT,
            retry: RETRY,
            ping_every: PING_EVERY,
            dead_after: DEAD_AFTER,
        }
    }
}

#[derive(Clone)]
pub struct Link {
    shared: Arc<Shared>,
}

struct Shared {
    addr: String,
    timing: Timing,
    state: Mutex<State>,
    counter: AtomicU64,
    closed: AtomicBool,
}

#[derive(Default)]
struct State {
    out: Option<Sender<String>>,
    pending: HashMap<String, Sender<Result<Value, String>>>,
    subs: Vec<Sub>,
    hello: Option<Value>,
    told_down: bool,
}

struct Sub {
    kinds: Vec<String>,
    tx: Sender<BrainMsg>,
    wake: Option<futures::channel::mpsc::UnboundedSender<()>>,
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn check_addr(addr: &str) -> Result<()> {
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("brain address {addr} needs host:port"))?;
    let port: u16 = port
        .parse()
        .map_err(|_| anyhow!("brain address {addr} has no port number"))?;
    if REFUSED_PORTS.contains(&port) {
        bail!(
            "port {port} is reserved for another local app. Set NIXA_BRAIN_ADDR, default {}",
            paths::DEFAULT_BRAIN_ADDR
        );
    }
    if !LOCAL_HOSTS.contains(&host) {
        bail!("nixa-brain runs on this Mac only. Use 127.0.0.1, not {host}.");
    }
    Ok(())
}

pub fn start() -> Result<Link> {
    start_at(&paths::brain_addr(), Timing::default())
}

pub fn start_at(addr: &str, timing: Timing) -> Result<Link> {
    check_addr(addr)?;
    let shared = Arc::new(Shared {
        addr: addr.to_string(),
        timing,
        state: Mutex::new(State::default()),
        counter: AtomicU64::new(0),
        closed: AtomicBool::new(false),
    });
    let worker = shared.clone();
    thread::Builder::new()
        .name("nixa-brain-link".into())
        .spawn(move || run(&worker))?;
    Ok(Link { shared })
}

impl Link {
    pub fn addr(&self) -> &str {
        &self.shared.addr
    }

    pub fn connected(&self) -> bool {
        lock(&self.shared).hello.is_some()
    }

    pub fn subscribe(&self, kinds: &[&str]) -> Receiver<BrainMsg> {
        self.subscribe_inner(kinds, None)
    }

    pub fn subscribe_wake(
        &self,
        kinds: &[&str],
        wake: futures::channel::mpsc::UnboundedSender<()>,
    ) -> Receiver<BrainMsg> {
        self.subscribe_inner(kinds, Some(wake))
    }

    fn subscribe_inner(
        &self,
        kinds: &[&str],
        wake: Option<futures::channel::mpsc::UnboundedSender<()>>,
    ) -> Receiver<BrainMsg> {
        let (tx, rx) = channel();
        let mut state = lock(&self.shared);
        if let Some(hello) = &state.hello {
            tx.send(BrainMsg::Up(hello.clone())).ok();
            if let Some(w) = &wake {
                let _ = w.unbounded_send(());
            }
        }
        state.subs.push(Sub {
            kinds: kinds.iter().map(|k| k.to_string()).collect(),
            tx,
            wake,
        });
        rx
    }

    pub fn send(&self, kind: &str, fields: Value) -> Result<()> {
        let text = message(kind, None, fields)?;
        let state = lock(&self.shared);
        let out = state.out.as_ref().ok_or_else(|| anyhow!(NOT_CONNECTED))?;
        out.send(text).map_err(|_| anyhow!(NOT_CONNECTED))
    }

    pub fn request(&self, kind: &str, args: Value) -> Result<Value> {
        self.request_for(kind, args, self.shared.timing.call)
    }

    pub fn request_for(&self, kind: &str, args: Value, wait: Duration) -> Result<Value> {
        let id = self.next_id();
        let text = message(kind, Some(&id), args)?;
        let (tx, rx) = channel();
        {
            let mut state = lock(&self.shared);
            let out = state.out.clone().ok_or_else(|| anyhow!(NOT_CONNECTED))?;
            state.pending.insert(id.clone(), tx);
            if out.send(text).is_err() {
                state.pending.remove(&id);
                bail!(NOT_CONNECTED);
            }
        }
        match rx.recv_timeout(wait) {
            Ok(Ok(data)) => Ok(data),
            Ok(Err(error)) => Err(anyhow!(error)),
            Err(RecvTimeoutError::Timeout) => {
                lock(&self.shared).pending.remove(&id);
                bail!(
                    "The brain did not answer \"{kind}\" in {} seconds.",
                    wait.as_secs_f32().round() as u64
                )
            }
            Err(RecvTimeoutError::Disconnected) => bail!(NOT_CONNECTED),
        }
    }

    pub fn stop(&self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        let mut state = lock(&self.shared);
        state.out = None;
        state.hello = None;
        fail_all(&mut state, STOPPED);
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        http(
            &self.shared.addr,
            "GET",
            path,
            None,
            self.shared.timing.call,
        )
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        http(
            &self.shared.addr,
            "POST",
            path,
            Some(body),
            self.shared.timing.call,
        )
    }

    fn next_id(&self) -> String {
        let n = self.shared.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        format!("app-{}-{n}", base36(ms))
    }
}

fn base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

pub fn message(kind: &str, request: Option<&str>, fields: Value) -> Result<String> {
    let kind = kind.trim();
    if kind.is_empty() {
        bail!("a brain message needs a type");
    }
    let mut map = match fields {
        Value::Object(map) => map,
        Value::Null => Map::new(),
        other => bail!("brain message fields must be an object, not {other}"),
    };
    map.insert("type".into(), Value::String(kind.to_string()));
    match request {
        Some(id) => map.insert("request".into(), Value::String(id.to_string())),
        None => map.remove("request"),
    };
    Ok(Value::Object(map).to_string())
}

pub fn is_reply(value: &Value) -> bool {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    value.get("request").and_then(Value::as_str).is_some()
        && kind.len() > "_reply".len()
        && kind.ends_with("_reply")
}

pub fn reply_result(value: &Value) -> Result<Value, String> {
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(value.get("data").cloned().unwrap_or(Value::Null));
    }
    Err(value
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "The brain said no.".to_string()))
}

fn fail_all(state: &mut State, why: &str) {
    for (_, waiter) in state.pending.drain() {
        waiter.send(Err(why.to_string())).ok();
    }
}

fn fan(state: &mut State, kind: Option<&str>, msg: &BrainMsg) {
    state.subs.retain(|sub| {
        let wanted = match kind {
            Some(kind) => sub.kinds.is_empty() || sub.kinds.iter().any(|k| k == kind),
            None => true,
        };
        if !wanted {
            return true;
        }
        let ok = sub.tx.send(msg.clone()).is_ok();
        if ok && let Some(wake) = &sub.wake {
            let _ = wake.unbounded_send(());
        }
        ok
    });
}

fn on_text(shared: &Shared, text: &str) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(kind) = value
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let mut state = lock(shared);
    if is_reply(&value) {
        let id = value
            .get("request")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(waiter) = state.pending.remove(id) {
            waiter.send(reply_result(&value)).ok();
        }
        return;
    }
    if kind == "hello" {
        state.hello = Some(value.clone());
        state.told_down = false;
        fan(&mut state, None, &BrainMsg::Up(value));
        return;
    }
    fan(&mut state, Some(&kind), &BrainMsg::Event(value));
}

fn went_down(shared: &Shared, why: &str) {
    let mut state = lock(shared);
    state.out = None;
    let was_up = state.hello.take().is_some();
    fail_all(&mut state, NOT_CONNECTED);
    if shared.closed.load(Ordering::SeqCst) {
        state.subs.clear();
        return;
    }
    if was_up || !state.told_down {
        state.told_down = true;
        fan(&mut state, None, &BrainMsg::Down(why.to_string()));
    }
}

fn run(shared: &Shared) {
    loop {
        if shared.closed.load(Ordering::SeqCst) {
            went_down(shared, STOPPED);
            return;
        }
        let why = match session(shared) {
            Ok(()) => "nixa-brain closed the link".to_string(),
            Err(error) => error.to_string(),
        };
        went_down(shared, &why);
        let until = Instant::now() + shared.timing.retry;
        while Instant::now() < until {
            if shared.closed.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(TICK);
        }
    }
}

fn resolve(addr: &str) -> Result<SocketAddr> {
    addr.to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("brain address {addr} does not resolve"))
}

fn connect(addr: &str, wait: Duration) -> Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&resolve(addr)?, wait)
        .map_err(|e| anyhow!("cannot reach nixa-brain at {addr}: {e}"))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(wait))?;
    stream.set_write_timeout(Some(wait))?;
    Ok(stream)
}

fn session(shared: &Shared) -> Result<()> {
    let stream = connect(&shared.addr, shared.timing.call)?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let url = format!("ws://{}/ws", shared.addr);
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(url)?;
    if let Some(token) = paths::window_token() {
        request
            .headers_mut()
            .insert(paths::WINDOW_TOKEN_HEADER, token.parse()?);
    }
    let (mut ws, _) = tungstenite::client::client_with_config(request, stream, Some(config))
        .map_err(|e| anyhow!("nixa-brain refused the link: {e}"))?;
    ws.get_ref().set_read_timeout(Some(TICK))?;
    let (tx, rx) = channel();
    lock(shared).out = Some(tx);
    let result = pump(shared, &mut ws, &rx);
    ws.close(None).ok();
    ws.flush().ok();
    result
}

fn pump(shared: &Shared, ws: &mut WebSocket<TcpStream>, rx: &Receiver<String>) -> Result<()> {
    let mut heard = Instant::now();
    let mut pinged = Instant::now();
    loop {
        if shared.closed.load(Ordering::SeqCst) {
            return Ok(());
        }
        while let Ok(text) = rx.try_recv() {
            ws.send(Message::text(text))?;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                heard = Instant::now();
                on_text(shared, text.as_str());
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => heard = Instant::now(),
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }
        if pinged.elapsed() >= shared.timing.ping_every {
            ws.send(Message::Ping(Default::default()))?;
            pinged = Instant::now();
        }
        if heard.elapsed() >= shared.timing.dead_after {
            bail!("nixa-brain stopped answering");
        }
    }
}

pub fn http(
    addr: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
    wait: Duration,
) -> Result<Value> {
    check_addr(addr)?;
    if !path.starts_with('/') || path.contains(['\r', '\n', ' ']) {
        bail!("bad brain path {path:?}");
    }
    let mut conn = connect(addr, wait)?;
    let data = body.map(Value::to_string).unwrap_or_default();
    let token = paths::window_token()
        .map(|t| format!("{}: {t}\r\n", paths::WINDOW_TOKEN_HEADER))
        .unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nContent-Type: application/json\r\n{token}Content-Length: {}\r\nConnection: close\r\n\r\n",
        data.len()
    );
    conn.write_all(head.as_bytes())?;
    conn.write_all(data.as_bytes())?;
    let mut raw = Vec::new();
    conn.take(MAX_HTTP + 1).read_to_end(&mut raw)?;
    if raw.len() as u64 > MAX_HTTP {
        bail!("nixa-brain sent more than {MAX_HTTP} bytes");
    }
    let (code, value) = parse_http(&raw)?;
    if !(200..300).contains(&code) || value.get("ok").and_then(Value::as_bool) == Some(false) {
        let text = value
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("nixa-brain answered {code}"));
        bail!(text);
    }
    Ok(value)
}

pub fn parse_http(raw: &[u8]) -> Result<(u16, Value)> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("nixa-brain sent a broken reply"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let code: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| anyhow!("nixa-brain sent a broken status line"))?;
    let chunked = head
        .lines()
        .any(|l| l.starts_with("transfer-encoding:") && l.contains("chunked"));
    let rest = &raw[split + 4..];
    let body = if chunked {
        unchunk(rest)?
    } else {
        rest.to_vec()
    };
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok((code, Value::Null));
    }
    let value = serde_json::from_slice(&body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).to_string()));
    Ok((code, value))
}

fn unchunk(mut rest: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| anyhow!("nixa-brain sent a broken chunk"))?;
        let size_text = String::from_utf8_lossy(&rest[..end]);
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| anyhow!("nixa-brain sent a broken chunk size"))?;
        rest = &rest[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if rest.len() < size {
            bail!("nixa-brain sent a short chunk");
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::BufRead;
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    fn quick() -> Timing {
        Timing {
            call: Duration::from_secs(2),
            retry: Duration::from_millis(100),
            ping_every: Duration::from_millis(100),
            dead_after: Duration::from_millis(900),
        }
    }

    fn listener() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        (listener, addr)
    }

    fn accept(listener: &TcpListener) -> WebSocket<TcpStream> {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        tungstenite::accept(stream).unwrap()
    }

    fn say(ws: &mut WebSocket<TcpStream>, value: Value) {
        ws.send(Message::text(value.to_string())).unwrap();
    }

    fn hear(ws: &mut WebSocket<TcpStream>) -> Option<Value> {
        loop {
            match ws.read() {
                Ok(Message::Text(text)) => return serde_json::from_str(text.as_str()).ok(),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    fn drain(ws: &mut WebSocket<TcpStream>) {
        while hear(ws).is_some() {}
    }

    fn next(rx: &Receiver<BrainMsg>) -> BrainMsg {
        rx.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    fn up(rx: &Receiver<BrainMsg>) -> Value {
        match next(rx) {
            BrainMsg::Up(hello) => hello,
            other => panic!("expected Up, got {other:?}"),
        }
    }

    fn request_id(value: &Value) -> String {
        value["request"].as_str().unwrap().to_string()
    }

    #[test]
    fn requests_get_their_own_reply_and_events_fan_out() {
        let (listener, addr) = listener();
        let server = thread::spawn(move || {
            let mut ws = accept(&listener);
            say(&mut ws, json!({"type": "hello", "name": "Nixa"}));
            let first = hear(&mut ws).unwrap();
            assert_eq!(first["type"], "tasks");
            assert_eq!(first["action"], "list");
            say(&mut ws, json!({"type": "tasks_changed", "n": 1}));
            say(&mut ws, json!({"type": "chat_item", "text": "hi"}));
            say(
                &mut ws,
                json!({"type": "tasks_reply", "request": "someone-else", "ok": true, "data": 0}),
            );
            say(
                &mut ws,
                json!({"type": "reply", "request": request_id(&first), "reply": {"text": "chat"}}),
            );
            say(
                &mut ws,
                json!({"type": "tasks_reply", "request": request_id(&first), "ok": true, "data": {"n": 5}}),
            );
            let second = hear(&mut ws).unwrap();
            assert_ne!(request_id(&first), request_id(&second));
            say(
                &mut ws,
                json!({"type": "tasks_reply", "request": request_id(&second), "ok": false, "error": "nope"}),
            );
            let said = hear(&mut ws).unwrap();
            assert_eq!(said, json!({"type": "say", "text": "hello"}));
            drain(&mut ws);
        });
        let link = start_at(&addr, quick()).unwrap();
        let all = link.subscribe(&[]);
        let tasks = link.subscribe(&["tasks_changed"]);
        assert_eq!(up(&all)["name"], "Nixa");
        assert_eq!(up(&tasks)["name"], "Nixa");
        assert!(link.connected());
        assert_eq!(
            link.request("tasks", json!({"action": "list"})).unwrap(),
            json!({"n": 5})
        );
        assert_eq!(
            next(&all),
            BrainMsg::Event(json!({"type": "tasks_changed", "n": 1}))
        );
        assert_eq!(
            next(&all),
            BrainMsg::Event(json!({"type": "chat_item", "text": "hi"}))
        );
        assert!(matches!(next(&all), BrainMsg::Event(v) if v["type"] == "reply"));
        assert_eq!(
            next(&tasks),
            BrainMsg::Event(json!({"type": "tasks_changed", "n": 1}))
        );
        let error = link.request("tasks", json!({"action": "add"})).unwrap_err();
        assert_eq!(error.to_string(), "nope");
        link.send("say", json!({"text": "hello"})).unwrap();
        let late = link.subscribe(&["nothing"]);
        assert_eq!(up(&late)["name"], "Nixa");
        thread::sleep(Duration::from_millis(100));
        assert!(tasks.try_recv().is_err());
        link.stop();
        server.join().unwrap();
    }

    #[test]
    fn a_silent_brain_times_out_and_a_late_reply_is_dropped() {
        let (listener, addr) = listener();
        let server = thread::spawn(move || {
            let mut ws = accept(&listener);
            say(&mut ws, json!({"type": "hello"}));
            let slow = hear(&mut ws).unwrap();
            let go = hear(&mut ws).unwrap();
            assert_eq!(go["type"], "go");
            say(
                &mut ws,
                json!({"type": "pull_reply", "request": request_id(&slow), "ok": true, "data": "late"}),
            );
            let third = hear(&mut ws).unwrap();
            say(
                &mut ws,
                json!({"type": "third_reply", "request": request_id(&third), "ok": true, "data": "third"}),
            );
            drain(&mut ws);
        });
        let link = start_at(&addr, quick()).unwrap();
        up(&link.subscribe(&[]));
        let start = Instant::now();
        let error = link
            .request_for("pull", json!({}), Duration::from_millis(300))
            .unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(
            error.to_string().contains("did not answer \"pull\""),
            "{error}"
        );
        link.send("go", Value::Null).unwrap();
        assert_eq!(link.request("third", json!({})).unwrap(), json!("third"));
        link.stop();
        server.join().unwrap();
    }

    #[test]
    fn no_brain_means_not_connected_and_one_down() {
        let link = start_at("127.0.0.1:1", quick()).unwrap();
        let rx = link.subscribe(&[]);
        assert!(
            matches!(next(&rx), BrainMsg::Down(why) if why.contains("cannot reach nixa-brain"))
        );
        assert_eq!(
            link.request("tasks", json!({})).unwrap_err().to_string(),
            NOT_CONNECTED
        );
        assert_eq!(
            link.send("say", json!({"text": "x"}))
                .unwrap_err()
                .to_string(),
            NOT_CONNECTED
        );
        assert!(!link.connected());
        thread::sleep(Duration::from_millis(400));
        assert!(rx.try_recv().is_err());
        link.stop();
    }

    #[test]
    fn the_link_comes_back_after_the_brain_restarts() {
        let (listener, addr) = listener();
        let server = thread::spawn(move || {
            let mut first = accept(&listener);
            say(&mut first, json!({"type": "hello", "n": 1}));
            hear(&mut first).unwrap();
            drop(first);
            let mut second = accept(&listener);
            say(&mut second, json!({"type": "hello", "n": 2}));
            let asked = hear(&mut second).unwrap();
            say(
                &mut second,
                json!({"type": "tasks_reply", "request": request_id(&asked), "ok": true, "data": 2}),
            );
            drain(&mut second);
        });
        let link = start_at(&addr, quick()).unwrap();
        let rx = link.subscribe(&[]);
        assert_eq!(up(&rx)["n"], 1);
        assert_eq!(
            link.request("tasks", json!({})).unwrap_err().to_string(),
            NOT_CONNECTED
        );
        assert!(matches!(next(&rx), BrainMsg::Down(_)));
        assert_eq!(up(&rx)["n"], 2);
        assert_eq!(link.request("tasks", json!({})).unwrap(), json!(2));
        link.stop();
        server.join().unwrap();
    }

    #[test]
    fn a_hung_brain_is_dropped_by_the_ping() {
        let (listener, addr) = listener();
        let server = thread::spawn(move || {
            let mut ws = accept(&listener);
            say(&mut ws, json!({"type": "hello"}));
            thread::sleep(Duration::from_millis(2000));
            drop(ws);
        });
        let timing = Timing {
            dead_after: Duration::from_millis(400),
            ..quick()
        };
        let link = start_at(&addr, timing).unwrap();
        let rx = link.subscribe(&[]);
        up(&rx);
        let start = Instant::now();
        assert_eq!(
            next(&rx),
            BrainMsg::Down("nixa-brain stopped answering".into())
        );
        assert!(start.elapsed() < Duration::from_millis(1500));
        link.stop();
        server.join().unwrap();
    }

    #[test]
    fn stop_fails_waiting_requests_and_closes_subscribers() {
        let (listener, addr) = listener();
        let server = thread::spawn(move || {
            let mut ws = accept(&listener);
            say(&mut ws, json!({"type": "hello"}));
            drain(&mut ws);
        });
        let link = start_at(&addr, quick()).unwrap();
        let rx = link.subscribe(&[]);
        up(&rx);
        let asker = link.clone();
        let waiting = thread::spawn(move || asker.request("speak", json!({})));
        thread::sleep(Duration::from_millis(200));
        link.stop();
        assert_eq!(waiting.join().unwrap().unwrap_err().to_string(), STOPPED);
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Err(RecvTimeoutError::Disconnected) => break,
                _ if Instant::now() > end => panic!("subscriber not closed"),
                _ => {}
            }
        }
        server.join().unwrap();
        assert_eq!(
            link.request("x", json!({})).unwrap_err().to_string(),
            NOT_CONNECTED
        );
    }

    #[test]
    fn only_local_dev_addresses_are_used() {
        for bad in [
            "127.0.0.1:7321",
            "127.0.0.1:7330",
            "localhost:7331",
            "10.0.0.2:7421",
            "127.0.0.1",
            "127.0.0.1:x",
        ] {
            assert!(check_addr(bad).is_err(), "{bad}");
            assert!(start_at(bad, quick()).is_err(), "{bad}");
        }
        for good in ["127.0.0.1:7421", "localhost:7421", "[::1]:7421"] {
            assert!(check_addr(good).is_ok(), "{good}");
        }
        assert!(http("127.0.0.1:7321", "GET", "/health", None, TIMEOUT).is_err());
    }

    #[test]
    fn message_shapes() {
        let said: Value = serde_json::from_str(
            &message("say", None, json!({"text": "hi", "request": "x"})).unwrap(),
        )
        .unwrap();
        assert_eq!(said, json!({"type": "say", "text": "hi"}));
        let asked: Value = serde_json::from_str(
            &message(
                "tasks",
                Some("app-1"),
                json!({"type": "other", "action": "ls"}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            asked,
            json!({"type": "tasks", "request": "app-1", "action": "ls"})
        );
        assert!(message(" ", None, Value::Null).is_err());
        assert!(message("say", None, json!(3)).is_err());
        assert!(is_reply(&json!({"type": "tasks_reply", "request": "a"})));
        assert!(!is_reply(&json!({"type": "tasks_changed", "request": "a"})));
        assert!(!is_reply(&json!({"type": "reply", "request": "a"})));
        assert!(!is_reply(&json!({"type": "_reply", "request": "a"})));
        assert!(!is_reply(&json!({"type": "tasks_reply"})));
        assert_eq!(reply_result(&json!({"ok": true})), Ok(Value::Null));
        assert_eq!(
            reply_result(&json!({"ok": false})),
            Err("The brain said no.".into())
        );
        assert_eq!(base36(0), "0");
        assert_eq!(base36(36 * 36 + 35), "10z");
    }

    fn http_once(listener: TcpListener, answer: &'static str) -> JoinHandle<String> {
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let mut out = stream;
            out.write_all(answer.as_bytes()).unwrap();
            format!("{head}{}", String::from_utf8(body).unwrap())
        })
    }

    #[test]
    fn plain_http_calls() {
        let (listener, addr) = listener();
        let seen = http_once(
            listener,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 22\r\n\r\n{\"ok\":true,\"items\":[]}",
        );
        assert_eq!(
            http(
                &addr,
                "POST",
                "/chat",
                Some(&json!({"text": "hi"})),
                TIMEOUT
            )
            .unwrap(),
            json!({"ok": true, "items": []})
        );
        let seen = seen.join().unwrap();
        assert!(seen.starts_with("POST /chat HTTP/1.1\r\n"), "{seen}");
        assert!(seen.ends_with("{\"text\":\"hi\"}"), "{seen}");

        let (listener, addr) = super::tests::listener();
        let seen = http_once(
            listener,
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7;x=1\r\n{\"ok\":t\r\n6\r\nrue,\"n\r\n5\r\n\":3}\n\r\n0\r\n\r\n",
        );
        assert_eq!(
            http(&addr, "GET", "/health", None, TIMEOUT).unwrap(),
            json!({"ok": true, "n": 3})
        );
        assert!(seen.join().unwrap().starts_with("GET /health HTTP/1.1\r\n"));

        let (listener, addr) = super::tests::listener();
        let seen = http_once(
            listener,
            "HTTP/1.1 400 Bad Request\r\nContent-Length: 34\r\n\r\n{\"ok\":false,\"error\":\"text needed\"}",
        );
        assert_eq!(
            http(&addr, "POST", "/chat", Some(&json!({})), TIMEOUT)
                .unwrap_err()
                .to_string(),
            "text needed"
        );
        seen.join().unwrap();

        let (listener, addr) = super::tests::listener();
        let seen = http_once(
            listener,
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n",
        );
        assert_eq!(
            http(&addr, "DELETE", "/brains/b1", None, TIMEOUT)
                .unwrap_err()
                .to_string(),
            "nixa-brain answered 500"
        );
        assert!(
            seen.join()
                .unwrap()
                .starts_with("DELETE /brains/b1 HTTP/1.1\r\n")
        );

        let link = start_at(&addr, quick()).unwrap();
        link.stop();
        assert!(link.get("/a b").is_err());
        assert!(link.get("health").is_err());
        assert!(parse_http(b"HTTP/1.1 200 OK\r\n").is_err());
        assert!(
            parse_http(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n").is_err()
        );
    }
}
