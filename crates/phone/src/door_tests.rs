use crate::cert::DoorNames;
use crate::door::{Deps, Door};
use crate::files;
use crate::sink::{Pcm, PhoneSink, Synth};
use crate::wire::{PhoneCommand, PhoneEvent};
use futures_util::{SinkExt, StreamExt};
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
use tokio_tungstenite::tungstenite::{Message, protocol::frame::coding::CloseCode};
use tokio_tungstenite::{WebSocketStream, client_async};

const WAIT: Duration = Duration::from_secs(5);

fn deps(dir: &Path) -> Deps {
    let mut deps = Deps::new(dir, Arc::new(|| json!({"state": "idle", "phone": {"on": true}})));
    deps.port = 0;
    deps.setup_port = 0;
    deps.address = Arc::new(|| Some("127.0.0.1".to_string()));
    deps.names = Arc::new(|address| DoorNames { ips: vec![address.to_string()], dns: Vec::new() });
    deps
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

fn connector(door: &Door) -> TlsConnector {
    let der = door.setup_view().unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(der)).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

async fn tls(door: &Door) -> TlsStream<TcpStream> {
    let (port, _) = door.ports().unwrap();
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let name = ServerName::IpAddress(std::net::Ipv4Addr::LOCALHOST.into());
    connector(door).connect(name, tcp).await.expect("the phone trusts the door")
}

async fn get_tls(door: &Door, path: &str, host: Option<&str>, cookie: Option<&str>) -> String {
    let (port, _) = door.ports().unwrap();
    let host = host.map(str::to_string).unwrap_or(format!("127.0.0.1:{port}"));
    let mut stream = tls(door).await;
    let cookie = cookie.map(|c| format!("Cookie: {c}\r\n")).unwrap_or_default();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: NixaApp (Android)\r\n{cookie}Connection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out).await;
    String::from_utf8_lossy(&out).into_owned()
}

async fn get_plain(port: u16, path: &str) -> Vec<u8> {
    get_plain_as(port, path, &format!("127.0.0.1:{port}")).await
}

async fn get_plain_as(port: u16, path: &str, host: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out).await;
    out
}

fn body(raw: &[u8]) -> &[u8] {
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    &raw[at + 4..]
}

fn cookie_of(reply: &str) -> String {
    let line = reply.lines().find(|l| l.to_ascii_lowercase().starts_with("set-cookie:")).expect("cookie");
    line.split_once(':').unwrap().1.trim().split(';').next().unwrap().to_string()
}

type Phone = WebSocketStream<TlsStream<TcpStream>>;

async fn phone_socket(door: &Door, cookie: &str, origin: bool) -> Result<Phone, String> {
    let (port, _) = door.ports().unwrap();
    let mut req = format!("wss://127.0.0.1:{port}/ws").into_client_request().unwrap();
    req.headers_mut().insert("cookie", HeaderValue::from_str(cookie).unwrap());
    if origin {
        req.headers_mut().insert("origin", HeaderValue::from_str(&format!("https://127.0.0.1:{port}")).unwrap());
    }
    client_async(req, tls(door).await).await.map(|(ws, _)| ws).map_err(|e| e.to_string())
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

async fn paired(door: &Door, dir: &Path) -> String {
    let key = files::read_door_key(&files::key_path(dir)).unwrap();
    let reply = get_tls(door, &format!("/k/{key}"), None, None).await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    let cookie = cookie_of(&reply);
    assert!(cookie.starts_with("nixa_phone="));
    cookie
}

fn tone_synth() -> Synth {
    Arc::new(|_text, _voice, _stop| Some(Pcm { rate: 16_000, pcm: vec![1u8; 16_000 * 2] }))
}

#[tokio::test]
async fn a_phone_pairs_talks_and_hears_over_the_door() {
    let dir = tempfile::tempdir().unwrap();
    let (door, mut rx) = Door::new(deps(dir.path())).unwrap();
    door.start().await;
    let info = door.info();
    assert!(info.on, "{info:?}");
    assert_eq!(info.error, None);
    let (port, setup_port) = door.ports().unwrap();
    assert_eq!(info.door_url.as_deref(), Some(format!("https://127.0.0.1:{port}/").as_str()));
    assert!(info.pair_qr.as_ref().is_some_and(|rows| rows.len() >= 21));
    assert_eq!(info.root_url.as_deref(), Some(format!("http://127.0.0.1:{setup_port}{}", crate::routes::ROOT_PATH).as_str()));
    let der = door.setup_view().unwrap();
    assert_eq!(info.root_sha256.as_deref(), Some(crate::secret::hex(&crate::secret::sha256_bytes(&der)).as_str()));

    let root = get_plain(setup_port, crate::routes::ROOT_PATH).await;
    assert_eq!(body(&root), der.as_slice());
    for host in ["evil.example:1".to_string(), format!("127.0.0.1:{port}"), format!("evil.example:{setup_port}")] {
        let other = get_plain_as(setup_port, crate::routes::ROOT_PATH, &host).await;
        assert!(String::from_utf8_lossy(&other).starts_with("HTTP/1.1 404"), "{host}");
    }
    for path in ["/", "/other", "/check"] {
        assert!(String::from_utf8_lossy(&get_plain(setup_port, path).await).starts_with("HTTP/1.1 404"), "{path}");
    }

    let key = files::read_door_key(&files::key_path(dir.path())).unwrap();
    for path in ["/", "/check", "/app.js", "/index.html", "/wake.js"] {
        assert!(get_tls(&door, path, None, None).await.starts_with("HTTP/1.1 404"), "{path}");
    }
    let wrong = format!("/k/{}", "A".repeat(43));
    assert!(get_tls(&door, &wrong, None, None).await.starts_with("HTTP/1.1 404"));
    assert!(get_tls(&door, &format!("/k/{key}"), Some("evil.example:1"), None).await.starts_with("HTTP/1.1 404"));
    let cookie = paired(&door, dir.path()).await;
    for path in ["/", "/app.js"] {
        assert!(get_tls(&door, path, None, Some(&cookie)).await.starts_with("HTTP/1.1 404"), "{path}");
    }
    let again = get_tls(&door, &format!("/k/{key}"), None, Some(&cookie)).await;
    assert!(again.starts_with("HTTP/1.1 404"), "{again}");
    assert!(get_tls(&door, &format!("/k/{key}"), None, None).await.starts_with("HTTP/1.1 404"));
    let next_key = files::read_door_key(&files::key_path(dir.path())).unwrap();
    assert_ne!(next_key, key);
    assert!(door.info().pair_qr.is_some());
    let phones = files::read_phones(&files::phones_path(dir.path()));
    assert_eq!(phones.len(), 1);
    assert_eq!(phones[0].name, "Android phone");

    assert!(phone_socket(&door, &cookie, false).await.is_err());
    assert!(phone_socket(&door, "nixa_phone=00000000.bad", true).await.is_err());
    let mut ws = phone_socket(&door, &cookie, true).await.unwrap();
    let first = next_json(&mut ws).await;
    assert_eq!(first, json!({"type": "snapshot", "snapshot": {"state": "idle"}}));
    send(&mut ws, json!({"type": "here", "t": 42})).await;
    assert_eq!(next_json(&mut ws).await, json!({"type": "here", "t": 42}));
    assert_eq!(door.clients(), 1);
    let mut second = phone_socket(&door, &cookie, true).await.unwrap();
    next_json(&mut second).await;
    let replaced = loop {
        match tokio::time::timeout(WAIT, ws.next()).await.unwrap() {
            Some(Ok(Message::Close(frame))) => break frame,
            Some(Ok(_)) => continue,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(replaced.unwrap().code, CloseCode::from(4002));
    drop(ws);
    let mut ws = second;
    for _ in 0..50 {
        if door.clients() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(door.clients(), 1);

    door.relay(&json!({"type": "chat", "text": "not for phones"}));
    door.relay(&json!({"type": "level", "mic": 0.5, "talking": true}));
    assert_eq!(next_json(&mut ws).await["type"], "level");

    send(&mut ws, json!({"type": "voice_ready"})).await;
    let phone = match event(&mut rx, |e| matches!(e, PhoneEvent::Voice { .. })).await {
        PhoneEvent::Voice { phone, ready } => {
            assert!(ready);
            phone
        }
        _ => unreachable!(),
    };
    assert_eq!(door.audio_ready(), 1);

    let sink = PhoneSink::new(door.clone(), tone_synth()).with_timing(500, 3000);
    let mut job = sink.play("Hello there. This is Nixa.", "Toby", true);
    let start = next_json(&mut ws).await;
    assert_eq!(start["type"], "audio_start");
    assert_eq!(start["rate"], 16_000);
    let line = start["line"].as_u64().unwrap();
    let mut frames = 0;
    let end = loop {
        match tokio::time::timeout(WAIT, ws.next()).await.unwrap().unwrap().unwrap() {
            Message::Binary(bytes) => {
                assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as u64, line);
                frames += 1;
            }
            Message::Text(text) => break serde_json::from_str::<Value>(text.as_str()).unwrap(),
            _ => {}
        }
    };
    assert_eq!(end["type"], "audio_end");
    assert_eq!(end["parts"].as_u64().unwrap(), frames);
    assert_eq!(frames, 2);
    send(&mut ws, json!({"type": "played", "line": line})).await;
    let said = job.wait().await;
    assert!(!said.interrupted);
    assert!(sink.take_missed().is_empty());

    send(&mut ws, json!({"type": "text", "text": "  build the app  "})).await;
    match event(&mut rx, |e| matches!(e, PhoneEvent::Command { .. })).await {
        PhoneEvent::Command { command, .. } => assert_eq!(command, PhoneCommand::Text("build the app".into())),
        _ => unreachable!(),
    }

    send(&mut ws, json!({"type": "talk_start", "auto": false})).await;
    event(&mut rx, |e| matches!(e, PhoneEvent::TalkStart { auto: false, .. })).await;
    assert!(door.talking());
    ws.send(Message::Binary(vec![1, 0, 2].into())).await.unwrap();
    ws.send(Message::Binary(vec![0].into())).await.unwrap();
    send(&mut ws, json!({"type": "talk_end"})).await;
    match event(&mut rx, |e| matches!(e, PhoneEvent::Talk { .. })).await {
        PhoneEvent::Talk { pcm, auto, .. } => {
            assert_eq!(pcm, vec![1, 2]);
            assert!(!auto);
        }
        _ => unreachable!(),
    }

    send(&mut ws, json!({"type": "check", "name": "mic", "ok": true, "detail": "fine"})).await;
    for _ in 0..50 {
        if door.info().phones[0].checks.contains_key("mic") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let row = door.info().phones[0].clone();
    assert!(row.connected && row.voice);
    assert_eq!(row.checks["mic"].detail, "fine");

    assert!(door.remove(&phone));
    let closed = loop {
        match tokio::time::timeout(WAIT, ws.next()).await.unwrap() {
            Some(Ok(Message::Close(frame))) => break frame,
            Some(Ok(_)) => continue,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(closed.unwrap().code, CloseCode::from(4001));
    assert!(phone_socket(&door, &cookie, true).await.is_err());

    door.stop().await;
    assert_eq!(door.ports(), None);
    assert!(!door.info().on);
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

#[tokio::test]
async fn new_key_forgets_phones_and_quiet_phones_are_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let mut d = deps(dir.path());
    d.gone_ms = 300;
    let (door, mut rx) = Door::new(d).unwrap();
    door.start().await;
    let cookie = paired(&door, dir.path()).await;
    let mut ws = phone_socket(&door, &cookie, true).await.unwrap();
    next_json(&mut ws).await;
    assert_eq!(door.clients(), 1);
    send(&mut ws, json!({"type": "talk_start", "auto": true})).await;
    event(&mut rx, |e| matches!(e, PhoneEvent::TalkStart { auto: true, .. })).await;
    ws.send(Message::Binary(vec![0u8; crate::wire::PEEK_BYTES].into())).await.unwrap();
    event(&mut rx, |e| matches!(e, PhoneEvent::TalkPeek { .. })).await;
    event(&mut rx, |e| matches!(e, PhoneEvent::TalkCancel { .. })).await;
    assert_eq!(door.clients(), 0);

    let old_key = files::read_door_key(&files::key_path(dir.path())).unwrap();
    let old_qr = door.info().pair_qr;
    door.new_key().await.unwrap();
    assert!(door.info().phones.is_empty());
    assert_ne!(door.info().pair_qr, old_qr);
    assert!(get_tls(&door, &format!("/k/{old_key}"), None, None).await.starts_with("HTTP/1.1 404"));
    assert!(phone_socket(&door, &cookie, true).await.is_err());
    let cookie = paired(&door, dir.path()).await;
    let mut ws = phone_socket(&door, &cookie, true).await.unwrap();
    next_json(&mut ws).await;
    drop(ws);

    door.refresh().await;
    assert!(door.ports().is_some());
    assert_eq!(door.info().error, None);
    let mut ws = phone_socket(&door, &cookie, true).await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "snapshot");
    door.stop().await;
}

#[tokio::test]
async fn missed_lines_are_kept_two_minutes_at_most_three() {
    let dir = tempfile::tempdir().unwrap();
    let (door, _rx) = Door::new(deps(dir.path())).unwrap();
    let clock = Arc::new(AtomicU64::new(1_000_000));
    let now = clock.clone();
    let sink = PhoneSink::new(door, tone_synth()).with_clock(Arc::new(move || now.load(Ordering::SeqCst)));
    for n in 0..4 {
        let mut job = sink.play(&format!("line {n}"), "Toby", true);
        assert!(job.wait().await.interrupted);
    }
    let mut job = sink.play("not kept", "Toby", false);
    assert!(job.wait().await.interrupted);
    let kept: Vec<String> = sink.take_missed().into_iter().map(|m| m.text).collect();
    assert_eq!(kept, vec!["line 1", "line 2", "line 3"]);
    assert!(sink.take_missed().is_empty());
    let mut job = sink.play("old", "Toby", true);
    job.wait().await;
    clock.fetch_add(crate::sink::MISSED_MS + 1, Ordering::SeqCst);
    assert!(sink.take_missed().is_empty());
    let mut job = sink.play("cleared", "Toby", true);
    job.wait().await;
    sink.clear_missed();
    assert!(sink.take_missed().is_empty());
}

#[tokio::test]
async fn the_door_reports_no_wifi_and_refuses_live_ports() {
    let dir = tempfile::tempdir().unwrap();
    let mut d = deps(dir.path());
    d.address = Arc::new(|| None);
    let (door, _rx) = Door::new(d).unwrap();
    door.start().await;
    let info = door.info();
    assert!(info.on);
    assert_eq!(info.error.as_deref(), Some("No Wi-Fi address. Join a Wi-Fi network."));
    assert_eq!(info.door_url, None);
    door.stop().await;
    assert_eq!(door.info().error, None);
    let mut live = deps(dir.path());
    live.port = 7330;
    assert!(Door::new(live).is_err());
}
