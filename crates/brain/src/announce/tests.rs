use super::*;
use nixa_protocol::{Status, TermId};
use std::collections::BTreeMap;

fn info(id: u64, name: &str, cmd: &[&str]) -> TermInfo {
    TermInfo {
        id: TermId::from_number(id),
        name: name.into(),
        labels: BTreeMap::new(),
        parent: None,
        children: Vec::new(),
        cmd: cmd.iter().map(|c| c.to_string()).collect(),
        cwd: "/tmp/shop".into(),
        pid: Some(100),
        running: true,
        exit_code: None,
        status: None,
        status_note: None,
        keep: false,
        draft: false,
        title: None,
        rows: 24,
        cols: 80,
        created_ms: 0,
        seq: 0,
    }
}

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn event(terminal: &str, kind: &str, time: u64) -> AgentEvent {
    AgentEvent { time, kind: kind.into(), text: format!("{terminal} {kind}"), terminal: terminal.into(), space: String::new(), job: String::new(), agent_number: None, agent: "claude".into(), offer: None }
}

#[test]
fn job_comes_from_the_tab_name() {
    let mut t = info(3, "shop · Fix login", &["/usr/local/bin/claude"]);
    assert_eq!(job_of(&t, "claude"), "Fix login");
    t.name = "claude".into();
    assert_eq!(job_of(&t, "claude"), "");
    t.name = "12".into();
    assert_eq!(job_of(&t, "claude"), "");
    t.name = "zsh".into();
    assert_eq!(job_of(&t, "unknown"), "");
    t.name = "Deep brain (shop)".into();
    assert_eq!(job_of(&t, "claude"), "Deep brain");
    t.name = "claude".into();
    t.labels.insert("nixa.schedule".into(), "task-42".into());
    assert_eq!(job_of(&t, "claude"), "Task 42");
}

#[test]
fn identity_uses_the_window_number_when_known() {
    let t = info(3, "Fix login", &["claude"]);
    let mut numbers = HashMap::new();
    let who = identity(&t, "claude", &numbers, "shop");
    assert_eq!(who.label, "Agent m3");
    numbers.insert("m3".to_string(), 2);
    let who = identity(&t, "claude", &numbers, "shop");
    assert_eq!(who.label, "Agent 2");
    assert_eq!(who.number, Some(2));
    assert_eq!(announcement(&who, "finished"), "Agent 2 finished: Fix login");
    assert_eq!(spoken(&who, "needs attention"), "Agent 2 needs attention: Fix login in shop");
    let bare = identity(&info(4, "claude", &["claude"]), "claude", &HashMap::new(), "");
    assert_eq!(spoken(&bare, "finished"), "Agent m4 finished");
    let mut own = info(5, "claude", &["claude"]);
    own.id = "nixa-claude-357".parse().unwrap();
    assert_eq!(identity(&own, "claude", &HashMap::new(), "").label, "Agent 357");
}

#[test]
fn the_window_number_map_wins_over_the_id_number_on_a_clash() {
    let (_dir, app) = app();
    let mut own = info(5, "claude", &["claude"]);
    own.id = "nixa-claude-357".parse().unwrap();
    assert!(ws(&app, "agent_numbers", &json!({"numbers": {"nixa-sh-357": 357, "nixa-claude-357": 1}})));
    let numbers = lock(&app.announce.numbers).clone();
    assert_eq!(identity(&own, "claude", &numbers, "").label, "Agent 1");
    assert_eq!(crate::announce::identity::number_of(&numbers, "nixa-sh-357"), Some(357));
}

#[test]
fn blocked_and_done_changes() {
    use Status::*;
    assert_eq!(change(Some(Working), Some(Blocked)), Some(Change::Blocked));
    assert_eq!(change(None, Some(Blocked)), Some(Change::Blocked));
    assert_eq!(change(Some(Blocked), Some(Blocked)), None);
    assert_eq!(change(Some(Working), Some(Done)), Some(Change::Finished));
    assert_eq!(change(Some(Blocked), Some(Idle)), Some(Change::Finished));
    assert_eq!(change(Some(Idle), Some(Done)), None);
    assert_eq!(change(None, Some(Done)), None);
    assert_eq!(change(Some(Working), Some(Error)), None);
    assert_eq!(change(Some(Working), None), None);
}

#[test]
fn repeats_merge_and_the_list_keeps_fifty() {
    let mut list = VecDeque::new();
    merge(&mut list, event("m1", "done", 1000));
    merge(&mut list, event("m1", "done", 3000));
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].time, 3000);
    merge(&mut list, event("m1", "blocked", 3500));
    merge(&mut list, event("m1", "done", 9000));
    assert_eq!(list.len(), 3);
    merge(&mut list, event("", "info", 9000));
    merge(&mut list, event("", "info", 9000));
    assert_eq!(list.len(), 5);
    for n in 0..80 {
        merge(&mut list, event(&format!("m{n}"), "done", 20_000 + n));
    }
    assert_eq!(list.len(), MAX_EVENTS);
    assert_eq!(list[0].terminal, "m79");
}

#[test]
fn only_known_resume_commands_are_safe() {
    for ok in [
        "claude --resume abc-123",
        "claude --resume abc-123 --dangerously-skip-permissions",
        "codex resume 019a-77 --dangerously-bypass-approvals-and-sandbox",
        "cursor-agent --resume c1 --yolo",
        "agyd --conversation x.y",
        "agy --conversation x",
        "omp --resume=s/1 --auto-approve",
        "opencode --session ses_1",
    ] {
        assert!(safe_command(ok), "{ok}");
    }
    for bad in [
        "claude --resume abc; rm -rf ~",
        "claude --resume $(id)",
        "claude --resume a b",
        "claude --resume",
        "bash -c claude",
        "codex resume x --other",
        "claude --resume a\nls",
    ] {
        assert!(!safe_command(bad), "{bad}");
    }
}

#[test]
fn resume_plans() {
    let mut t = info(5, "Fix login", &["claude"]);
    assert_eq!(plan(&t, None), Plan::Nothing);
    t.labels.insert(RESUME_LABEL.into(), "claude --resume abc".into());
    assert_eq!(plan(&t, None), Plan::Mark("100".into()));
    t.labels.insert(PID_LABEL.into(), "100".into());
    assert_eq!(plan(&t, None), Plan::Nothing);
    t.pid = Some(200);
    t.cmd.clear();
    assert_eq!(plan(&t, Some(Some(1))), Plan::Look { pid: 200, command: "claude --resume abc".into() });
    assert_eq!(plan(&t, Some(None)), Plan::Look { pid: 200, command: "claude --resume abc".into() });
    assert_eq!(plan(&t, None), Plan::Look { pid: 200, command: "claude --resume abc".into() });
    assert_eq!(plan(&t, Some(Some(0))), Plan::Forget);
    t.running = false;
    t.exit_code = Some(2);
    assert_eq!(plan(&t, None), Plan::Ended { command: "claude --resume abc".into() });
    t.exit_code = Some(0);
    assert_eq!(plan(&t, None), Plan::Forget);
    t.exit_code = None;
    t.labels.insert(PID_LABEL.into(), ENDED.into());
    assert_eq!(plan(&t, None), Plan::Nothing);
    t.labels.insert(RESUME_LABEL.into(), "claude --resume abc; ls".into());
    t.labels.remove(PID_LABEL);
    assert_eq!(plan(&t, None), Plan::Nothing);
    t.labels.insert(RESUME_LABEL.into(), "claude --resume abc".into());
    t.exit_code = Some(2);
    assert_eq!(plan(&t, None), Plan::Ended { command: "claude --resume abc".into() });
    t.labels.insert(crate::deep::ROLE_LABEL.into(), crate::deep::ROLE_VALUE.into());
    assert_eq!(plan(&t, None), Plan::Nothing, "the deep brain seat restarts its own terminal");
}

#[test]
fn a_typed_or_moved_shell_is_not_steady() {
    let mut t = info(5, "x", &[]);
    t.labels.insert(RESUME_LABEL.into(), "claude --resume abc".into());
    assert!(steady(&t, 100, "claude --resume abc"));
    assert!(!steady(&t, 101, "claude --resume abc"));
    t.draft = true;
    assert!(!steady(&t, 100, "claude --resume abc"));
    t.draft = false;
    t.labels.insert(RESUME_LABEL.into(), "claude --resume other".into());
    assert!(!steady(&t, 100, "claude --resume abc"));
    assert_eq!(tidy("a  \nb \n\n"), "a\nb");
}

#[test]
fn agent_names() {
    let t = info(1, "x", &["/opt/bin/cursor-agent"]);
    assert_eq!(agent_name("", &t), "cursor");
    let mut shell = info(2, "x", &[]);
    assert_eq!(agent_name("", &shell), "unknown");
    assert_eq!(agent_name("claude", &shell), "claude");
    shell.labels.insert(RESUME_LABEL.into(), "codex resume s-1".into());
    assert_eq!(agent_name("claude", &shell), "codex");
}

#[tokio::test]
async fn status_changes_announce_with_space_and_number() {
    let (dir, app) = app();
    let mut rx = app.out.subscribe();
    let mut t = info(7, "Fix login", &["claude"]);
    t.cwd = dir.path().display().to_string();
    on_event(&app, Event::Created { term: t.clone() }).await;
    lock(&app.announce.numbers).insert("m7".into(), 4);
    on_event(&app, Event::Status { id: t.id.clone(), status: Some(Status::Working), note: None }).await;
    on_event(&app, Event::Status { id: t.id.clone(), status: Some(Status::Done), note: None }).await;
    on_event(&app, Event::Status { id: t.id.clone(), status: Some(Status::Blocked), note: None }).await;
    let list = app.announce.list();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].kind, "blocked");
    assert_eq!(list[0].text, "Agent 4 needs attention: Fix login");
    assert_eq!(list[1].kind, "done");
    assert_eq!(list[1].agent_number, Some(4));
    assert_eq!(list[1].agent, "claude");
    let mut kinds = Vec::new();
    while let Ok(v) = rx.try_recv() {
        if v["type"] == "agent_event" {
            kinds.push(v["event"]["kind"].as_str().unwrap_or_default().to_string());
        }
    }
    assert_eq!(kinds, vec!["done", "blocked"]);
    on_event(&app, Event::Status { id: TermId::from_number(99), status: Some(Status::Blocked), note: None }).await;
    assert_eq!(app.announce.list().len(), 2);
}

#[tokio::test]
async fn switch_and_numbers_by_websocket() {
    let (_dir, app) = app();
    assert!(is_on(&app));
    let mut rx = app.out.subscribe();
    assert!(ws(&app, "set_agent_voice", &json!({"on": false})));
    assert!(!is_on(&app));
    assert_eq!(rx.try_recv().unwrap(), json!({"type": "agent_voice", "on": false}));
    let conn = nixa_store::open(&app.db_file).unwrap();
    assert_eq!(nixa_store::settings::get_setting(&conn, VOICE_KEY).unwrap(), Some(json!(false)));
    nixa_store::settings::set_setting_bool(&conn, VOICE_KEY, true).unwrap();
    assert!(is_on(&app), "the menu and tab bar switch in the store is the one switch");
    assert!(!ws(&app, "set_agent_voice", &json!({"on": "no"})));
    assert!(ws(&app, "agent_numbers", &json!({"numbers": {"m1": 1, "m2": 0, "m3": "x", "m4": 3}})));
    let numbers = lock(&app.announce.numbers).clone();
    assert_eq!(numbers.len(), 2);
    assert_eq!(numbers.get("m4"), Some(&3));
    assert!(!ws(&app, "something_else", &json!({})));
    assert_eq!(view(&app)["on"], json!(true));
    assert_eq!(view(&app)["numbers"], json!({"m1": 1, "m4": 3}));
}

#[test]
fn offers_name_the_space_and_job() {
    let (dir, app) = app();
    let folder = dir.path().join("shop");
    std::fs::create_dir_all(&folder).unwrap();
    let db = dir.path().join(nixa_store::DB_FILE);
    let conn = nixa_store::open(&db).unwrap();
    nixa_store::spaces::set_space(&conn, "s9", &nixa_store::spaces::SpacePatch { name: Some("shop".into()), folder: Some(folder.display().to_string()), ..Default::default() }).unwrap();
    let mut t = info(8, "Fix login", &[]);
    t.cwd = folder.display().to_string();
    let offer = resume::offer_for(&app, &t, "claude", "claude --resume abc");
    assert_eq!(offer.title, "Resume agent m8: Fix login? (permission checks off)");
    assert_eq!(offer.body, "shop: it stopped. Click to resume it in a new tab: claude --resume abc --dangerously-skip-permissions");
    assert_eq!(offer.space, "s9");
    assert_eq!(offer.name, "Fix login");
    assert_eq!(offer.folder, t.cwd);
    assert_eq!(offer.timeout_ms, resume::OFFER_MS);
    std::fs::write(app.dir.join("programs.json"), r#"{"checksOff":["claude"],"checksOn":["claude"]}"#).unwrap();
    let checked = resume::offer_for(&app, &t, "claude", "claude --resume abc --dangerously-skip-permissions");
    assert_eq!(checked.title, "Resume agent m8: Fix login?");
    assert!(checked.body.ends_with("claude --resume abc"));
}

#[test]
fn automatic_resume_spec_and_brain_texts() {
    let (dir, app) = app();
    let mut t = info(8, "Fix login", &[]);
    t.cwd = dir.path().display().to_string();
    let offer = resume::offer_for(&app, &t, "claude", "claude --resume abc");
    let space = crate::spaces::Space { id: "s9".into(), name: "shop".into(), folder: t.cwd.clone() };
    let spec = resume::resume_spec(&offer, Some(&space));
    assert_eq!(spec.cmd, vec!["sh".to_string(), "-lc".into(), offer.command.clone()]);
    assert_eq!(spec.cwd.as_deref(), Some(t.cwd.as_str()));
    assert_eq!(spec.name.as_deref(), Some("Fix login"));
    assert_eq!(spec.labels.get(resume::LABEL), Some(&offer.command));
    assert_eq!(spec.labels.get(nixa_protocol::id::SPACE_SLUG_LABEL).map(String::as_str), Some("shop"));
    assert!(SPACE_LABELS.iter().all(|k| spec.labels.get(*k).map(String::as_str) == Some("s9")));
    assert_eq!(resume::auto_key("c", 10), resume::auto_key("c", resume::AUTO_WINDOW_MS - 1));
    assert_ne!(resume::auto_key("c", 10), resume::auto_key("c", resume::AUTO_WINDOW_MS + 1));
    let again = resume::again_text(&offer);
    assert!(again.starts_with("Event: ") && again.contains("Do not wait for the user") && again.contains("nixa_term_new"));
    let failed = resume::failed_text(&offer, "engine down");
    assert!(failed.contains("engine down") && !failed.contains("Click"));
}
