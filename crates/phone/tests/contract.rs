use futures_util::{SinkExt, StreamExt};
use nixa_phone::sink::{Pcm, PhoneSink, Synth};
use nixa_phone::{Deps, Door, DoorNames, PhoneCommand, PhoneEvent, files, qr};
use rustls::pki_types::{CertificateDer, ServerName};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, client_async};

const WAIT: Duration = Duration::from_secs(5);
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const AGENT: &str = "NixaApp (Android)";
const ROOT_PATH: &str = "/nixa-root.crt";

type Phone = WebSocketStream<TlsStream<TcpStream>>;

struct Scan {
    host: String,
    port: u16,
    key: String,
    pin: String,
}

fn deps(dir: &Path, clock: Arc<AtomicU64>) -> Deps {
    let mut deps = Deps::new(dir, Arc::new(|| json!({"state": "idle", "talk": "hold", "phone": {"on": true}})));
    deps.port = 0;
    deps.setup_port = 0;
    deps.address = Arc::new(|| Some("127.0.0.1".to_string()));
    deps.names = Arc::new(|address| DoorNames { ips: vec![address.to_string()], dns: Vec::new() });
    deps.now = Arc::new(move || clock.load(Ordering::SeqCst));
    deps
}

fn real_now() -> u64 {
    nixa_phone::now_ms()
}

async fn open_door(dir: &Path, clock: Arc<AtomicU64>) -> (Door, UnboundedReceiver<PhoneEvent>) {
    let (door, rx) = Door::new(deps(dir, clock)).unwrap();
    door.start().await;
    assert_eq!(door.info().error, None);
    (door, rx)
}

async fn http_get(port: u16, method: &str, path: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out).await;
    out
}

fn split(raw: &[u8]) -> (String, Vec<u8>) {
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (String::from_utf8_lossy(&raw[..at]).into_owned(), raw[at + 4..].to_vec())
}

fn scanned_text(door: &Door, dir: &Path, root: &[u8]) -> String {
    let (port, _) = door.ports().unwrap();
    let key = files::read_door_key(&files::key_path(dir)).unwrap();
    let text = qr::pair_text("127.0.0.1", port, &key, root);
    assert_eq!(Some(qr::qr_rows(&text).unwrap()), door.info().pair_qr, "the QR on screen holds {text}");
    text
}

fn read_qr(text: &str) -> Result<Scan, String> {
    let rest = text.strip_prefix("https://").ok_or("not a door link")?;
    let (host, rest) = rest.split_once('/').ok_or("no path")?;
    let (path, _) = rest.split_once('#').unwrap_or((rest, ""));
    let key = path.strip_prefix("k/").ok_or("no key")?.to_string();
    let (ip, port) = host.split_once(':').ok_or("no port")?;
    let pin = qr::pin_of(text).ok_or("no #r part")?.to_string();
    Ok(Scan { host: ip.to_string(), port: port.parse().map_err(|_| "bad port")?, key, pin })
}

async fn download_root(setup_port: u16) -> Vec<u8> {
    let (head, body) = split(&http_get(setup_port, "GET", ROOT_PATH).await);
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    body
}

fn trust(scan: &Scan, root: &[u8]) -> Result<Vec<u8>, String> {
    if qr::root_pin(root) != scan.pin {
        return Err("The Mac's certificate does not match the code. Scan the code again.".into());
    }
    Ok(root.to_vec())
}

fn connector(root: &[u8]) -> TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(root.to_vec())).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

async fn tls(root: &[u8], port: u16) -> Result<TlsStream<TcpStream>, String> {
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.map_err(|e| e.to_string())?;
    let name = ServerName::IpAddress(std::net::Ipv4Addr::LOCALHOST.into());
    connector(root).connect(name, tcp).await.map_err(|e| e.to_string())
}

async fn leaf(root: &[u8], port: u16) -> Vec<u8> {
    let stream = tls(root, port).await.unwrap();
    stream.get_ref().1.peer_certificates().unwrap()[0].to_vec()
}

async fn get_door(root: &[u8], port: u16, path: &str, host: Option<&str>, cookie: Option<&str>) -> Result<String, String> {
    let mut stream = tls(root, port).await?;
    let host = host.map(str::to_string).unwrap_or(format!("127.0.0.1:{port}"));
    let cookie = cookie.map(|c| format!("Cookie: {c}\r\n")).unwrap_or_default();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {AGENT}\r\n{cookie}Connection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out).await;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn cookie_of(reply: &str) -> Option<String> {
    let line = reply.lines().find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))?;
    Some(line.split_once(':')?.1.trim().split(';').next()?.to_string())
}

async fn pair(root: &[u8], scan: &Scan) -> Result<String, String> {
    let reply = get_door(root, scan.port, &format!("/k/{}", scan.key), None, None).await?;
    if !reply.starts_with("HTTP/1.1 200") {
        return Err("The Mac did not pair this phone".into());
    }
    cookie_of(&reply).ok_or_else(|| "no cookie".into())
}

async fn connect(root: &[u8], port: u16, cookie: &str, origin: Option<&str>, host: Option<&str>) -> Result<Phone, String> {
    let mut req = format!("wss://127.0.0.1:{port}/ws").into_client_request().unwrap();
    let h = req.headers_mut();
    h.insert("cookie", HeaderValue::from_str(cookie).unwrap());
    h.insert("user-agent", HeaderValue::from_static(AGENT));
    let own = format!("https://127.0.0.1:{port}");
    h.insert("origin", HeaderValue::from_str(origin.unwrap_or(&own)).unwrap());
    if let Some(host) = host {
        h.insert("host", HeaderValue::from_str(host).unwrap());
    }
    let stream = tls(root, port).await?;
    client_async(req, stream).await.map(|(ws, _)| ws).map_err(|e| e.to_string())
}

async fn next_json(ws: &mut Phone) -> Value {
    loop {
        let msg = tokio::time::timeout(WAIT, ws.next()).await.expect("message in time").expect("open").unwrap();
        if let Message::Text(text) = msg {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

async fn send(ws: &mut Phone, value: Value) {
    ws.send(Message::Text(value.to_string().into())).await.unwrap();
}

async fn event(rx: &mut UnboundedReceiver<PhoneEvent>, want: impl Fn(&PhoneEvent) -> bool) -> PhoneEvent {
    tokio::time::timeout(WAIT, async {
        loop {
            let got = rx.recv().await.expect("events");
            if want(&got) {
                return got;
            }
        }
    })
    .await
    .expect("event in time")
}

async fn close_code(ws: &mut Phone) -> Option<u16> {
    loop {
        match tokio::time::timeout(WAIT, ws.next()).await.ok()? {
            Some(Ok(Message::Close(frame))) => return frame.map(|f| u16::from(f.code)),
            Some(Ok(_)) => continue,
            _ => return None,
        }
    }
}

fn tone() -> Synth {
    Arc::new(|_text, _voice, _stop| Some(Pcm { rate: 16_000, pcm: vec![1u8; 16_000 * 2] }))
}

async fn paired_phone(door: &Door, dir: &Path) -> (Scan, Vec<u8>, String) {
    let (_, setup_port) = door.ports().unwrap();
    let root = download_root(setup_port).await;
    let scan = read_qr(&scanned_text(door, dir, &root)).unwrap();
    let root = trust(&scan, &root).unwrap();
    let cookie = pair(&root, &scan).await.unwrap();
    (scan, root, cookie)
}

#[tokio::test]
async fn the_app_pairs_with_the_root_pin_and_talks_over_the_door() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicU64::new(real_now()));
    let (door, mut rx) = open_door(dir.path(), clock).await;
    let (port, setup_port) = door.ports().unwrap();

    let (head, root) = split(&http_get(setup_port, "GET", ROOT_PATH).await);
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head.to_ascii_lowercase().contains("application/x-x509-ca-cert"));
    assert!(root.starts_with(&[0x30]));
    let text = String::from_utf8_lossy(&root).to_ascii_uppercase();
    assert!(!text.contains("PRIVATE") && !text.contains("BEGIN"));
    for (method, path) in [("GET", "/"), ("GET", "/app.js"), ("GET", "/k/x"), ("POST", ROOT_PATH), ("GET", "/nixa-root.key")] {
        let (head, _) = split(&http_get(setup_port, method, path).await);
        assert!(head.starts_with("HTTP/1.1 404"), "{method} {path}: {head}");
    }
    assert_eq!(door.info().root_pin.as_deref(), Some(qr::root_pin(&root).as_str()));

    let qr_text = scanned_text(&door, dir.path(), &root);
    let scan = read_qr(&qr_text).unwrap();
    assert_eq!((scan.host.as_str(), scan.port), ("127.0.0.1", port));
    assert_eq!(scan.key.len(), 43);
    assert_eq!(scan.pin.len(), 43);
    let root = trust(&scan, &root).unwrap();

    for path in ["/", "/app.js", "/index.html", "/k/", "/nixa-root.crt"] {
        assert!(get_door(&root, port, path, None, None).await.unwrap().starts_with("HTTP/1.1 404"), "{path}");
    }
    let wrong = Scan { key: "A".repeat(43), ..read_qr(&qr_text).unwrap() };
    assert!(pair(&root, &wrong).await.is_err());
    let foreign_host = get_door(&root, port, &format!("/k/{}", scan.key), Some("evil.example:1"), None).await.unwrap();
    assert!(foreign_host.starts_with("HTTP/1.1 404"));

    let cookie = pair(&root, &scan).await.unwrap();
    assert!(cookie.starts_with("nixa_phone="), "{cookie}");
    for path in ["/", "/app.js"] {
        assert!(get_door(&root, port, path, None, Some(&cookie)).await.unwrap().starts_with("HTTP/1.1 404"), "{path}");
    }
    assert!(pair(&root, &scan).await.is_err(), "a used door key pairs no second phone");
    assert_eq!(door.info().phones.len(), 1);
    let fresh = read_qr(&scanned_text(&door, dir.path(), &root)).unwrap();
    assert_ne!(fresh.key, scan.key);
    let second = pair(&root, &fresh).await.unwrap();
    assert_ne!(second, cookie);
    assert_eq!(door.info().phones.len(), 2);

    assert!(connect(&root, port, &cookie, Some("https://evil.example"), None).await.is_err());
    assert!(connect(&root, port, &cookie, None, Some("evil.example:1")).await.is_err());
    assert!(connect(&root, port, "old_phone=00000000.bad", None, None).await.is_err());
    let live_name = cookie.replacen("nixa_phone=", "old_phone=", 1);
    assert!(connect(&root, port, &live_name, None, None).await.is_err());
    let mut ws = connect(&root, port, &cookie, None, None).await.unwrap();

    let first = next_json(&mut ws).await;
    assert_eq!(first["type"], "snapshot");
    assert!(first["snapshot"].is_object());
    assert_eq!(first["snapshot"]["state"], "idle");
    assert!(first["snapshot"].get("phone").is_none());

    send(&mut ws, json!({"type": "here", "t": 7})).await;
    assert_eq!(next_json(&mut ws).await, json!({"type": "here", "t": 7}));
    assert_eq!(door.clients(), 1);

    door.relay(&json!({"type": "chat", "text": "not relayed"}));
    for msg in [
        json!({"type": "level", "mic": 0.4, "talking": true}),
        json!({"type": "heard", "text": "hello"}),
        json!({"type": "say", "text": "hi", "kind": "reply"}),
    ] {
        door.relay(&msg);
        assert_eq!(next_json(&mut ws).await, msg);
    }
    door.relay(&json!({"type": "snapshot", "snapshot": {"state": "busy", "phone": {"on": true}}}));
    assert_eq!(next_json(&mut ws).await, json!({"type": "snapshot", "snapshot": {"state": "busy"}}));

    send(&mut ws, json!({"type": "text", "text": " run the tests "})).await;
    let got = event(&mut rx, |e| matches!(e, PhoneEvent::Command { .. })).await;
    assert!(matches!(got, PhoneEvent::Command { command: PhoneCommand::Text(t), .. } if t == "run the tests"));
    send(&mut ws, json!({"type": "set_mobile", "on": true})).await;
    let got = event(&mut rx, |e| matches!(e, PhoneEvent::Command { .. })).await;
    assert!(matches!(got, PhoneEvent::Command { command: PhoneCommand::SetMobile(true), .. }));

    send(&mut ws, json!({"type": "voice_ready"})).await;
    let phone = match event(&mut rx, |e| matches!(e, PhoneEvent::Voice { .. })).await {
        PhoneEvent::Voice { phone, ready } => {
            assert!(ready);
            phone
        }
        _ => unreachable!(),
    };

    send(&mut ws, json!({"type": "talk_start", "auto": false})).await;
    event(&mut rx, |e| matches!(e, PhoneEvent::TalkStart { auto: false, .. })).await;
    assert!(door.talking());
    ws.send(Message::Binary(vec![3, 0, 4, 0].into())).await.unwrap();
    send(&mut ws, json!({"type": "talk_end"})).await;
    match event(&mut rx, |e| matches!(e, PhoneEvent::Talk { .. })).await {
        PhoneEvent::Talk { pcm, auto, .. } => {
            assert_eq!(pcm, vec![3, 4]);
            assert!(!auto);
        }
        _ => unreachable!(),
    }
    send(&mut ws, json!({"type": "talk_start", "auto": false})).await;
    event(&mut rx, |e| matches!(e, PhoneEvent::TalkStart { .. })).await;
    ws.send(Message::Binary(vec![1, 0].into())).await.unwrap();
    send(&mut ws, json!({"type": "talk_cancel"})).await;
    event(&mut rx, |e| matches!(e, PhoneEvent::TalkCancel { .. })).await;
    assert!(!door.talking());

    let sink = PhoneSink::new(door.clone(), tone()).with_timing(500, 3000);
    let mut job = sink.play("Hello. This is Nixa.", "Toby", true);
    let start = next_json(&mut ws).await;
    assert_eq!(start["type"], "audio_start");
    assert_eq!(start["rate"], 16_000);
    let line = start["line"].as_u64().unwrap();
    let mut parts = 0;
    let end = loop {
        match tokio::time::timeout(WAIT, ws.next()).await.unwrap().unwrap().unwrap() {
            Message::Binary(bytes) => {
                assert!(bytes.len() > 8);
                assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as u64, line);
                parts += 1;
            }
            Message::Text(text) => break serde_json::from_str::<Value>(text.as_str()).unwrap(),
            _ => {}
        }
    };
    assert_eq!(end["type"], "audio_end");
    assert_eq!(end["line"].as_u64(), Some(line));
    assert_eq!(end["parts"].as_u64(), Some(parts));
    send(&mut ws, json!({"type": "played", "line": line})).await;
    assert!(!job.wait().await.interrupted);

    assert!(door.remove(&phone));
    assert_eq!(close_code(&mut ws).await, Some(4001));
    assert!(connect(&root, port, &cookie, None, None).await.is_err());
    assert_eq!(door.info().phones.len(), 1);

    let old_key = scan.key.clone();
    door.new_key().await.unwrap();
    assert!(door.info().phones.is_empty());
    assert!(connect(&root, port, &second, None, None).await.is_err());
    let old = Scan { key: old_key, ..read_qr(&qr_text).unwrap() };
    assert!(pair(&root, &old).await.is_err());
    let fresh = read_qr(&scanned_text(&door, dir.path(), &root)).unwrap();
    assert_ne!(fresh.key, old.key);
    assert_eq!(fresh.pin, old.pin);
    let cookie = pair(&root, &fresh).await.unwrap();
    let mut ws = connect(&root, port, &cookie, None, None).await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "snapshot");

    door.stop().await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    assert!(TcpStream::connect(("127.0.0.1", setup_port)).await.is_err());
}

#[tokio::test]
async fn the_app_refuses_a_wrong_pin_a_changed_root_and_another_door() {
    let dir = tempfile::tempdir().unwrap();
    let other_dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicU64::new(real_now()));
    let (door, _rx) = open_door(dir.path(), clock.clone()).await;
    let (other, _orx) = open_door(other_dir.path(), clock).await;
    let (port, setup_port) = door.ports().unwrap();
    let (other_port, other_setup) = other.ports().unwrap();

    let root = download_root(setup_port).await;
    let other_root = download_root(other_setup).await;
    assert_ne!(root, other_root);
    let text = scanned_text(&door, dir.path(), &root);

    let mut wrong_pin = read_qr(&text).unwrap();
    wrong_pin.pin = qr::root_pin(&other_root);
    assert!(trust(&wrong_pin, &root).is_err());

    let good = read_qr(&text).unwrap();
    let mut changed = root.clone();
    let last = changed.len() - 1;
    changed[last] ^= 0x01;
    assert!(trust(&good, &changed).is_err());
    assert!(trust(&good, &other_root).is_err());

    let no_pin = text.split('#').next().unwrap().to_string();
    assert!(read_qr(&no_pin).is_err());

    let saved = trust(&good, &root).unwrap();
    let swapped = Scan { port: other_port, ..read_qr(&text).unwrap() };
    assert!(tls(&saved, other_port).await.is_err());
    assert!(pair(&saved, &swapped).await.is_err());
    assert!(tls(&saved, port).await.is_ok());
    assert!(tls(&other_root, port).await.is_err());

    door.stop().await;
    other.stop().await;
}

#[tokio::test]
async fn a_renewed_door_certificate_keeps_the_paired_app_working() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicU64::new(real_now() - 370 * DAY_MS));
    let (door, _rx) = open_door(dir.path(), clock.clone()).await;
    let (scan, root, cookie) = paired_phone(&door, dir.path()).await;
    let pin = door.info().root_pin.unwrap();
    let before = leaf(&root, scan.port).await;
    let mut ws = connect(&root, scan.port, &cookie, None, None).await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "snapshot");
    drop(ws);

    clock.store(real_now(), Ordering::SeqCst);
    door.refresh().await;
    assert_eq!(door.info().error, None);
    let (port, setup_port) = door.ports().unwrap();
    let after = leaf(&root, port).await;
    assert_ne!(before, after, "the door certificate was renewed");
    assert_eq!(download_root(setup_port).await, root);
    assert_eq!(door.info().root_pin.as_deref(), Some(pin.as_str()));
    let mut ws = connect(&root, port, &cookie, None, None).await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "snapshot");
    door.stop().await;
}
