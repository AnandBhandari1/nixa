use super::*;
use nixa_protocol::{CreateSpec, Request, Status};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

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
        eprintln!("nixa-engine is not built next to the test binary; build it with cargo build -p nixa-engine first");
        return None;
    }
    let dir = PathBuf::from(format!("/tmp/mo-an{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    nixa_protocol::paths::ensure_private_dir(&dir).ok()?;
    let socket = dir.join("e.sock");
    let child = Command::new(bin).arg("--socket").arg(&socket).env("NIXA_DATA_DIR", &dir).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    let engine = Engine { child, dir, socket: socket.clone() };
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if client::connect(&socket).and_then(|mut c| c.call(&Request::Ping)).is_ok() {
            return Some(engine);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

async fn create(app: &App, cmd: &str, name: &str, keep: bool, resume: Option<&str>) -> String {
    let mut spec = CreateSpec { cmd: vec!["sh".into(), "-c".into(), cmd.into()], cwd: Some("/tmp".into()), name: Some(name.into()), keep, ..CreateSpec::default() };
    if let Some(command) = resume {
        spec.labels.insert(RESUME_LABEL.into(), command.into());
    }
    match app.announce.link().call(Request::Create { spec }, Duration::from_secs(10)).await.expect("create") {
        Reply::Term { term } => term.id.to_string(),
        other => panic!("unexpected reply {other:?}"),
    }
}

async fn labels(app: &App, id: &str) -> BTreeMap<String, String> {
    app.announce.link().get(id).await.expect("get").labels
}

async fn wait_for(rx: &mut tokio::sync::broadcast::Receiver<Value>, seconds: u64, want: impl Fn(&Value) -> bool) -> Option<Value> {
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(value)) if want(&value) => return Some(value),
            _ => {}
        }
    }
    None
}

use nixa_protocol::Reply;
use std::collections::BTreeMap;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_engine_resume_and_announce() {
    let Some(engine) = start_engine() else { return };
    let data = tempfile::tempdir().unwrap();
    let app = App::open(data.path()).unwrap();
    crate::settings_api::set_nixa(&app, false).unwrap();
    app.announce.set_socket(Some(engine.socket.clone()));
    *lock(&app.announce.timing) = Timing { settle_ms: 1500, gap_ms: 600 };
    let mut rx = app.out.subscribe();
    tokio::spawn(run(app.clone()));
    tokio::time::sleep(Duration::from_millis(300)).await;

    let crash = create(&app, "sleep 2; exit 3", "Fix login", true, Some("claude --resume abc-123")).await;
    let quit = create(&app, "sleep 2; exit 0", "Docs", true, Some("codex resume s-9")).await;
    let dead = create(&app, "sleep 2; exit 2", "Tests", false, Some("claude --resume dead-1")).await;
    let plain = create(&app, "sleep 1; exit 4", "Plain", true, None).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let first = labels(&app, &crash).await;
    assert!(first.get(PID_LABEL).is_some_and(|p| !p.is_empty() && p != ENDED), "pid marked while the agent runs: {first:?}");

    let offer = wait_for(&mut rx, 20, |v| v["type"] == "resume_offer" && v["offer"]["terminal"] == crash.as_str()).await.expect("resume offer for the crashed agent");
    assert_eq!(offer["offer"]["command"], "claude --resume abc-123 --dangerously-skip-permissions");
    assert_eq!(offer["offer"]["name"], "Fix login");
    let after = labels(&app, &crash).await;
    let now = app.announce.link().get(&crash).await.unwrap();
    assert_eq!(after.get(PID_LABEL).cloned(), now.pid.map(|p| p.to_string()));
    let ended = app.announce.list().into_iter().find(|e| e.terminal == dead && e.kind == "resume");
    let ended = match ended {
        Some(e) => e,
        None => {
            let v = wait_for(&mut rx, 10, |v| v["type"] == "agent_event" && v["event"]["terminal"] == dead.as_str()).await.expect("offer for the ended agent");
            serde_json::from_value(v["event"].clone()).unwrap()
        }
    };
    assert_eq!(
        ended.offer.as_ref().map(|o| o.command.as_str()),
        Some("claude --resume dead-1 --dangerously-skip-permissions")
    );
    assert_eq!(labels(&app, &dead).await.get(PID_LABEL).map(String::as_str), Some(ENDED));

    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let l = labels(&app, &quit).await;
        if !l.contains_key(RESUME_LABEL) && !l.contains_key(PID_LABEL) {
            break;
        }
        assert!(Instant::now() < until, "a clean quit forgets the resume label: {l:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(app.announce.list().iter().all(|e| e.terminal != plain && e.terminal != quit));

    let offers = app.announce.list().iter().filter(|e| e.kind == "resume").count();
    refresh(&app).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(app.announce.list().iter().filter(|e| e.kind == "resume").count(), offers, "no second offer after a refresh");

    let id = crate::prompts::link::term_id(&plain).unwrap();
    for status in [Status::Working, Status::Done] {
        app.announce.link().call(Request::StatusSet { id: id.clone(), status: Some(status), note: None }, Duration::from_secs(5)).await.unwrap();
    }
    let done = wait_for(&mut rx, 5, |v| v["type"] == "agent_event" && v["event"]["kind"] == "done").await.expect("done is announced");
    assert_eq!(done["event"]["terminal"], plain.as_str());
    let shown = id.own_number().map(|n| n.to_string()).unwrap_or_else(|| plain.to_string());
    assert_eq!(done["event"]["text"], format!("Agent {shown} finished: Plain"));
    drop(engine);
}
