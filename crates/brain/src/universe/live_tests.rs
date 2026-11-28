use super::*;
use nixa_protocol::{CreateSpec, Reply, Request, client};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Engine {
    child: Child,
    dir: PathBuf,
    socket: PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = client::connect(&self.socket).and_then(|mut c| c.call(&Request::Shutdown));
        std::thread::sleep(Duration::from_millis(200));
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start_engine() -> Option<Engine> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("nixa-engine");
    if !bin.exists() {
        eprintln!("SKIPPED live universe test: nixa-engine is not built next to the test binary; build it with cargo build -p nixa-engine first");
        return None;
    }
    let dir = PathBuf::from(format!("/tmp/mo-uv{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    nixa_protocol::paths::ensure_private_dir(&dir).expect("live test data folder");
    let socket = dir.join("e.sock");
    let child = Command::new(bin).arg("--socket").arg(&socket).env("NIXA_DATA_DIR", &dir).stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("nixa-engine is built but did not start");
    let engine = Engine { child, dir, socket: socket.clone() };
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if client::connect(&socket).and_then(|mut c| c.call(&Request::Ping)).is_ok() {
            return Some(engine);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("nixa-engine is built but did not answer a ping within 10 s");
}

fn create(socket: &Path, script: &str) -> String {
    let spec = CreateSpec { cmd: vec!["sh".into(), "-c".into(), script.into()], cwd: Some("/tmp".into()), ..CreateSpec::default() };
    match client::connect(socket).and_then(|mut c| c.call(&Request::Create { spec })).expect("create") {
        Reply::Term { term } => term.id.to_string(),
        other => panic!("unexpected reply {other:?}"),
    }
}

use std::path::Path;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_message_to_another_agent_shows_as_a_flow() {
    let Some(engine) = start_engine() else { return };
    let data = tempfile::tempdir().unwrap();
    let app = App::open(data.path()).unwrap();
    app.universe.set_socket(Some(engine.socket.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let service = routes().with_state(app.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, service).await.ok() });
    let mut rx = app.out.subscribe();

    let reviewer = create(&engine.socket, "sleep 30");
    let note = format!(
        "sleep 0.5; curl -s -o /dev/null -w '%{{http_code}}' -X POST -H 'content-type: application/json' -d \"{{\\\"terminal\\\":\\\"$NIXA_TERM_ID\\\",\\\"to\\\":\\\"{reviewer}\\\"}}\" http://127.0.0.1:{port}{} ; sleep 30",
        route::ROUTE
    );
    let worker = create(&engine.socket, &note);
    let until = Instant::now() + Duration::from_secs(15);
    let mut seen = None;
    while Instant::now() < until && seen.is_none() {
        if let Ok(Ok(v)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
            && v["type"] == FLOW_KIND
        {
            seen = Some(v);
        }
    }
    let v = seen.expect("a flow from the worker terminal");
    assert_eq!((v["from"].as_str(), v["to"].as_str()), (Some(worker.as_str()), Some(reviewer.as_str())));

    let http = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}{}", route::ROUTE);
    let outside = http.post(&url).json(&json!({"terminal": worker, "to": reviewer})).send().await.unwrap();
    assert_eq!(outside.status(), reqwest::StatusCode::FORBIDDEN);
    let gone = http.post(&url).json(&json!({"terminal": worker, "to": "mzz9"})).send().await.unwrap();
    assert_eq!(gone.status(), reqwest::StatusCode::NOT_FOUND);
    let fake = http.post(&url).json(&json!({"terminal": "mzz8", "to": reviewer})).send().await.unwrap();
    assert_eq!(fake.status(), reqwest::StatusCode::FORBIDDEN);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let more = std::iter::from_fn(|| rx.try_recv().ok()).filter(|v| v["type"] == FLOW_KIND).count();
    assert_eq!(more, 0);
}
