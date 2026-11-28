use super::auto;
use super::book::NewPrompt;
use super::link::Link;
use super::scan;
use crate::app::{App, lock};
use nixa_protocol::{CreateSpec, Reply, Request};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Engine {
    child: Child,
    dir: PathBuf,
    link: Link,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = nixa_protocol::client::connect(&self.link.socket).and_then(|mut c| c.call(&Request::Shutdown));
        std::thread::sleep(Duration::from_millis(200));
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn engine_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("nixa-engine");
    bin.exists().then_some(bin)
}

fn start_engine(tag: &str) -> Option<Engine> {
    let Some(bin) = engine_bin() else {
        eprintln!("nixa-engine is not built next to the test binary; build it with cargo build -p nixa-engine first");
        return None;
    };
    let dir = PathBuf::from(format!("/tmp/mo-{tag}{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    nixa_protocol::paths::ensure_private_dir(&dir).ok()?;
    let socket = dir.join("e.sock");
    let child = Command::new(bin).arg("--socket").arg(&socket).env("NIXA_DATA_DIR", &dir).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    let link = Link::new(&socket);
    let engine = Engine { child, dir, link };
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if nixa_protocol::client::connect(&socket).and_then(|mut c| c.call(&Request::Ping)).is_ok() {
            return Some(engine);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn script() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/prompts/fake_agent.py").display().to_string()
}

async fn fake_agent(link: &Link, steps: &[&str], log: &Path) -> String {
    let mut cmd = vec!["python3".to_string(), "-u".to_string(), script()];
    cmd.extend(steps.iter().map(|s| s.to_string()));
    let mut spec = CreateSpec { cmd, cwd: Some("/tmp".into()), name: Some("fake agent".into()), ..CreateSpec::default() };
    spec.env.insert("FAKE_LOG".into(), log.display().to_string());
    match link.call(Request::Create { spec }, Duration::from_secs(10)).await.expect("create") {
        Reply::Term { term } => term.id.to_string(),
        other => panic!("unexpected reply {other:?}"),
    }
}

async fn next_prompt(app: &Arc<App>, link: &Link, terminal: &str) -> super::book::Prompt {
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until {
        scan::scan(app, link).await;
        if let Some(p) = lock(&app.prompts.book).items.iter().find(|p| p.terminal == terminal).cloned() {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let screen = link.read(terminal).await.unwrap_or_default();
    panic!("no prompt from {terminal}; screen:\n{screen}");
}

async fn wait_log(log: &Path, line: &str) {
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if std::fs::read_to_string(log).unwrap_or_default().lines().any(|l| l == line) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the fake agent did not log {line:?}; log: {:?}", std::fs::read_to_string(log).unwrap_or_default());
}

fn chat_has(app: &App, words: &str) -> bool {
    lock(&app.chat).list().iter().any(|item| item.text.contains(words))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_deep_brain_answers_agent_questions() {
    let Some(engine) = start_engine("a") else { return };
    let link = engine.link.clone();
    let data = tempfile::tempdir().unwrap();
    let app = Arc::new(App::open(data.path()).unwrap());
    let conn = nixa_store::open(&app.db_file).unwrap();
    nixa_store::spaces::set_space(&conn, "fake", &nixa_store::spaces::SpacePatch { folder: Some("/tmp".into()), ..Default::default() }).unwrap();
    app.decision.set_full(crate::decision::Full::Deep).unwrap();
    app.prompts.set_socket(Some(link.socket.clone()));
    let log = engine.dir.join("deep.log");
    let id = fake_agent(&link, &["question", "multi", "question"], &log).await;
    let picked = |ask: &str, pick: &str| {
        let mut done = lock(&app.deep).asks.iter().find(|a| a.id == ask).cloned().expect("deep ask");
        done.state = crate::deep::AskState::Answered;
        done.pick = Some(pick.into());
        done.why = Some("fits the job".into());
        done
    };

    let p = next_prompt(&app, &link, &id).await;
    let first = auto::deciding(&app, &p).expect("the question went to the deep brain");
    assert!(chat_has(&app, "The deep brain decides this one"));
    assert!(auto::answered(&app, &picked(&first, "Postgres")).await);
    wait_log(&log, "question: Postgres").await;
    assert!(auto::answered(&app, &picked(&first, "Postgres")).await);

    let p = next_prompt(&app, &link, &id).await;
    assert!(p.question().unwrap().multi_select);
    let second = auto::deciding(&app, &p).expect("the next question went to the deep brain");
    assert_ne!(second, first);
    assert!(auto::answered(&app, &picked(&second, "Tests, Lint")).await);
    wait_log(&log, "multi: Tests, Lint").await;

    let p = next_prompt(&app, &link, &id).await;
    let third = auto::deciding(&app, &p).expect("the last question went to the deep brain");
    link.keys(&id, vec![nixa_protocol::Key::Enter]).await.unwrap();
    wait_log(&log, "question: SQLite").await;
    if let Some(open) = lock(&app.prompts.book).get_mut(&p.id) {
        open.created_at = 0;
    }
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until && lock(&app.prompts.book).get(&p.id).is_some() {
        scan::scan(&app, &link).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(lock(&app.prompts.book).get(&p.id).is_none());
    assert!(auto::answered(&app, &picked(&third, "Postgres")).await);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let lines = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(lines.lines().filter(|l| *l == "question: Postgres").count(), 1, "{lines}");
    assert!(chat_has(&app, "Nothing was sent"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn answered_on_screen_and_gone_terminals_close_prompts() {
    let Some(engine) = start_engine("c") else { return };
    let link = engine.link.clone();
    let data = tempfile::tempdir().unwrap();
    let app = App::open(data.path()).unwrap();
    let log = engine.dir.join("c.log");
    let id = fake_agent(&link, &["question"], &log).await;
    let p = next_prompt(&app, &link, &id).await;
    lock(&app.prompts.book).untold.clear();
    lock(&app.prompts.book).get_mut(&p.id).unwrap().created_at = 0;
    link.keys(&id, vec![nixa_protocol::Key::Enter]).await.unwrap();
    wait_log(&log, "question: SQLite").await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    scan::scan(&app, &link).await;
    assert!(lock(&app.prompts.book).items.is_empty());
    assert!(lock(&app.brain).carry.contains(&format!("The user answered prompt {} in terminal {id} on its screen", p.id)));
    let event = NewPrompt { kind: "question".into(), terminal: "m999".into(), text: Some("x".into()), ..NewPrompt::default() };
    lock(&app.prompts.book).add(event, 1).unwrap();
    scan::scan(&app, &link).await;
    assert!(lock(&app.prompts.book).items.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_person_typing_is_not_sent_over() {
    let Some(engine) = start_engine("d") else { return };
    let link = engine.link.clone();
    let spec = CreateSpec { cmd: vec!["cat".into()], cwd: Some("/tmp".into()), name: Some("typing".into()), ..CreateSpec::default() };
    let id = match link.call(Request::Create { spec }, Duration::from_secs(10)).await.unwrap() {
        Reply::Term { term } => term.id,
        other => panic!("unexpected reply {other:?}"),
    };
    link.call(Request::Input { id: id.clone(), text: "half typed".into() }, Duration::from_secs(5)).await.unwrap();
    let sent = super::deliver::type_text(&link, &id.to_string(), "only the fast ones").await;
    assert!(!sent.ok, "{sent:?}");
    assert!(sent.detail.contains("typing"), "{}", sent.detail);
    let screen = link.read(&id.to_string()).await.unwrap();
    assert!(!screen.contains("only the fast ones"), "{screen}");
}
