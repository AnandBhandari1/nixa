//! K1 contract: fake phone over WSS, door, brain, fake vision on localhost.

use futures_util::{SinkExt, StreamExt};
use nixa_brain::brains::Checked;
use nixa_brain::camera::{self, FOLDER};
use nixa_brain::provider::Kind;
use nixa_phone::wire::FRAME_TAG;
use nixa_phone::{Deps, Door, DoorNames, PhoneEvent};
use nixa_store::models::{self, By, ModelEdit, model_rows};
use rustls::pki_types::{CertificateDer, ServerName};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, client_async};

const WAIT: Duration = Duration::from_secs(20);
const ROOT: &str = "/nixa-root.crt";
const AGENT: &str = "NixaApp (Android)";
const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 50, 0, 100, 3, 0, 0, 0];

type Ws = WebSocketStream<TlsStream<TcpStream>>;

async fn vision_base(listed: Vec<String>) -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let tags = listed.clone();
    let router = axum::Router::new()
        .route(
            "/api/tags",
            axum::routing::get(move || {
                let tags = tags.clone();
                async move { axum::Json(json!({"models": tags.iter().map(|name| json!({"name": name, "capabilities": ["completion", "vision"]})).collect::<Vec<_>>()})) }
            }),
        )
        .route(
            "/v1/models",
            axum::routing::get(move || {
                let listed = listed.clone();
                async move {
                    axum::Json(json!({
                        "data": listed.iter().map(|id| json!({"id": id})).collect::<Vec<_>>()
                    }))
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            axum::routing::post({
                move |axum::Json(_body): axum::Json<Value>| {
                    seen.fetch_add(1, Ordering::SeqCst);
                    async move { axum::Json(json!({"choices": [{"message": {"content": "A person sits still."}}]})) }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    (base, calls)
}

fn tls(root: &[u8]) -> TlsConnector {
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

async fn pair(port: u16, root: &[u8], key: &str) -> String {
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let name = ServerName::IpAddress(std::net::Ipv4Addr::LOCALHOST.into());
    let mut stream = tls(root).connect(name, tcp).await.unwrap();
    let req = format!("GET /k/{key} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUser-Agent: {AGENT}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw).await;
    String::from_utf8_lossy(&raw)
        .lines()
        .find_map(|line| line.strip_prefix("set-cookie: ").or_else(|| line.strip_prefix("Set-Cookie: ")))
        .expect("pair cookie")
        .split(';')
        .next()
        .unwrap()
        .trim()
        .to_string()
}

async fn connect(root: &[u8], port: u16, cookie: &str) -> Ws {
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let name = ServerName::IpAddress(std::net::Ipv4Addr::LOCALHOST.into());
    let stream = tls(root).connect(name, tcp).await.unwrap();
    let mut req = format!("wss://127.0.0.1:{port}/ws").into_client_request().unwrap();
    let host = format!("127.0.0.1:{port}");
    req.headers_mut().insert("Cookie", HeaderValue::from_str(cookie).unwrap());
    req.headers_mut().insert("User-Agent", HeaderValue::from_static(AGENT));
    req.headers_mut().insert("Host", HeaderValue::from_str(&host).unwrap());
    req.headers_mut()
        .insert("Origin", HeaderValue::from_str(&format!("https://{host}")).unwrap());
    client_async(req, stream).await.unwrap().0
}

async fn send_json(ws: &mut Ws, value: &Value) {
    ws.send(Message::Text(value.to_string().into())).await.unwrap();
}

async fn next_json(ws: &mut Ws) -> Value {
    loop {
        if let Message::Text(text) = tokio::time::timeout(WAIT, ws.next()).await.unwrap().unwrap().unwrap() {
            return serde_json::from_str(text.as_ref()).unwrap();
        }
    }
}

fn frame(id: u32, index: u8) -> Vec<u8> {
    let mut out = FRAME_TAG.to_vec();
    out.extend_from_slice(&id.to_le_bytes());
    out.push(index);
    out.extend_from_slice(&5u64.to_le_bytes());
    out.extend_from_slice(JPEG);
    out.push(index);
    out
}

async fn look_with_yes(app: &Arc<nixa_brain::app::App>, question: &str) -> Result<String, String> {
    let args: nixa_brain::tools::Args = serde_json::from_value(json!({"question": question})).unwrap();
    let held = camera::call(app, &args).await?;
    assert!(held.contains("\"held\""), "{held}");
    tokio::time::sleep(Duration::from_millis(5)).await;
    nixa_brain::app::lock(&app.brain).last_user = Some(("yes photo".into(), nixa_brain::util::now_ms()));
    camera::call(app, &args).await
}

fn test_deps(dir: &std::path::Path) -> Deps {
    let app_dir = dir.join("data");
    let mut deps = Deps::new(&app_dir, Arc::new(|| json!({"state": "idle"})));
    deps.port = 0;
    deps.setup_port = 0;
    deps.address = Arc::new(|| Some("127.0.0.1".to_string()));
    deps.names = Arc::new(|_| DoorNames { ips: vec!["127.0.0.1".to_string()], dns: Vec::new() });
    deps
}

#[tokio::test]
async fn k1_camera_look_contract_over_wss_and_brain() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let app = nixa_brain::app::App::open(&dir.path().join("data")).unwrap();
    let conn = nixa_brain::models::open(&app).unwrap();
    models::update(&conn, |cache| {
        let _ = models::pick::ensure_map(cache, nixa_brain::util::now_ms() as i64, &models::lists::fixed_models());
        Ok(())
    })
    .unwrap();
    let id = model_rows(&models::load(&conn).unwrap()).into_iter().find(|r| r.listed).map(|r| r.id).expect("listed model");
    models::update(&conn, |cache| {
        models::pick::edit_model(
            cache,
            &id,
            &ModelEdit { capabilities: Some(vec!["vision".into()]), pins: Some(vec!["vision".into()]), ..ModelEdit::default() },
            By::User,
        )?;
        Ok(())
    })
    .unwrap();
    let model = model_rows(&models::load(&conn).unwrap()).into_iter().find(|r| r.id == id).map(|r| r.model).expect("vision row");
    let listed = nixa_brain::desktop::vision::vision_rows(&models::load(&conn).unwrap())
        .into_iter()
        .map(|r| r.model)
        .collect::<Vec<_>>();
    let (vision_url, vision_calls) = vision_base(listed).await;
    app.brains
        .add(Checked { name: "Vision".into(), kind: Kind::Ollama, base_url: vision_url, key_env: None, model: model.clone() })
        .unwrap();
    camera::set_looks(&app, true).unwrap();
    let (door, mut events) = Door::new(test_deps(dir.path())).unwrap();
    *nixa_brain::app::lock(&app.phone.door) = Some(door.clone());
    door.start().await;
    let app_events = app.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                PhoneEvent::LookFrames { phone, id, frames } => camera::frames_came(&app_events, &phone, id, frames),
                PhoneEvent::LookFail { phone, id, why } => camera::look_failed(&app_events, &phone, id, &why),
                _ => {}
            }
        }
    });
    let (port, setup) = door.ports().expect("ports");
    let mut plain = TcpStream::connect(("127.0.0.1", setup)).await.unwrap();
    plain.write_all(format!("GET {ROOT} HTTP/1.1\r\nHost: 127.0.0.1:{setup}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    let _ = plain.read_to_end(&mut raw).await;
    let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let root = raw[start..].to_vec();
    let key = nixa_phone::files::read_door_key(&nixa_phone::files::key_path(&dir.path().join("data"))).unwrap();
    let cookie = pair(port, &root, &key).await;
    let mut ws = connect(&root, port, &cookie).await;
    assert_eq!(next_json(&mut ws).await["type"], "snapshot");
    send_json(&mut ws, &json!({"type": "caps", "camera": true})).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(door.camera_phones().len(), 1);
    let handler = tokio::spawn(async move {
        let mut looks = 0;
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tick.tick().await;
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    send_json(&mut ws, &json!({"type": "caps", "camera": true})).await;
                }
                msg = next_json(&mut ws) => {
                    if msg.get("type").and_then(Value::as_str) != Some("look_request") {
                        continue;
                    }
                    let id = msg["id"].as_u64().unwrap() as u32;
                    looks += 1;
                    if looks == 1 {
                        assert_eq!(msg["frames"], json!(1));
                        ws.send(Message::Binary(frame(id, 0).into())).await.unwrap();
                        send_json(&mut ws, &json!({"type": "look_end", "id": id, "frames": 1})).await;
                    } else {
                        send_json(&mut ws, &json!({"type": "look_fail", "id": id, "why": "denied"})).await;
                        return;
                    }
                }
            }
        }
    });
    let answer = look_with_yes(&app, "what is happening").await.unwrap();
    assert!(answer.contains("A person sits still."), "{answer}");
    assert!(vision_calls.load(Ordering::SeqCst) >= 1);
    assert!(camera::view(&app)["log"].as_array().is_some_and(|l| !l.is_empty()));
    assert!(app.dir.join(FOLDER).join("tmp").read_dir().map(|mut d| d.next().is_none()).unwrap_or(true));
    let err = look_with_yes(&app, "try again").await.unwrap_err();
    handler.await.unwrap();
    assert!(err.contains("camera permission"));
    door.stop().await;
}