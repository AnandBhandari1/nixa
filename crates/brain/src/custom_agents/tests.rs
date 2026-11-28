use super::actions::{self, Who, add_of, edit_line, edit_of, read_off};
use super::launch::{self, quote};
use super::live::{self, Live, RESUME_LABEL};
use super::start::{self, Step, UseInput};
use crate::app::App;
use crate::spaces::Space;
use crate::tools::Args;
use nixa_protocol::{Status, TermId, TermInfo};
use nixa_store::agents::{AgentState, CustomAgent, StartProfile};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

fn agent(id: &str, provider: &str) -> CustomAgent {
    CustomAgent {
        id: id.into(),
        title: "News".into(),
        purpose: "news researcher".into(),
        instructions: String::new(),
        role: String::new(),
        job: String::new(),
        workflow: String::new(),
        start: None,
        off: false,
        migrated_from: String::new(),
        provider: provider.into(),
        session: String::new(),
        space: "s1".into(),
        space_name: "nixa".into(),
        folder: String::new(),
        terminal: String::new(),
        created_at: 1,
        last_used: 1,
        by: "test".into(),
        skills: Vec::new(),
    }
}

fn term(id: u64, cmd: &[&str], status: Option<Status>, running: bool) -> TermInfo {
    TermInfo {
        id: TermId::from_number(id),
        name: String::new(),
        labels: BTreeMap::new(),
        parent: None,
        children: Vec::new(),
        cmd: cmd.iter().map(|s| s.to_string()).collect(),
        cwd: "/tmp".into(),
        pid: None,
        running,
        exit_code: None,
        status,
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

fn spaces() -> Vec<Space> {
    vec![
        Space {
            id: "s1".into(),
            name: "nixa".into(),
            folder: "/tmp".into(),
        },
        Space {
            id: "s2".into(),
            name: "site".into(),
            folder: "/var".into(),
        },
    ]
}

fn args(value: Value) -> Args {
    value.as_object().cloned().unwrap_or_default()
}

#[test]
fn shell_words_are_quoted() {
    assert_eq!(quote("claude"), "claude");
    assert_eq!(
        quote("model_reasoning_effort=high"),
        "model_reasoning_effort=high"
    );
    assert_eq!(quote("it's done"), "'it'\\''s done'");
    assert_eq!(quote(""), "''");
    assert_eq!(launch::join(&["a b".into(), "c".into()]), "'a b' c");
}

#[test]
fn profile_plans_put_the_prompt_where_the_program_wants_it() {
    let mut a = agent("a1", "codex");
    a.start = Some(StartProfile {
        agent: "codex".into(),
        model: "gpt-5".into(),
        effort: "high".into(),
        args: Vec::new(),
    });
    a.instructions = "Use bullets.".into();
    let plan = launch::profile_plan(&nixa_protocol::paths::data_dir(), &a, "Do the job").unwrap();
    let mut want: Vec<String> = nixa_store::agents::start_command("codex").unwrap().split_whitespace().map(str::to_string).collect();
    want.extend(["-m", "gpt-5", "-c", "model_reasoning_effort=high"].map(str::to_string));
    assert_eq!(&plan.argv[..want.len()], &want[..]);
    assert_eq!(plan.argv.len(), want.len());
    let prompt = plan.send.unwrap();
    assert!(prompt.contains("You are News, a custom agent of Nixa"));
    assert!(prompt.contains("Use bullets."));
    assert!(prompt.ends_with("Do the job"));
    a.start = Some(StartProfile {
        agent: "opencode".into(),
        model: "zai/glm".into(),
        ..StartProfile::default()
    });
    let plan = launch::profile_plan(&nixa_protocol::paths::data_dir(), &a, "").unwrap();
    assert_eq!(
        plan.argv,
        ["opencode", "--auto", "--model", "zai/glm"]
    );
    assert!(plan.send.is_some());
    a.start = Some(StartProfile {
        agent: "jcode".into(),
        model: "big".into(),
        effort: "low".into(),
        ..StartProfile::default()
    });
    let plan = launch::profile_plan(&nixa_protocol::paths::data_dir(), &a, "hi").unwrap();
    assert_eq!(plan.argv, ["jcode", "--no-update"]);
    let send = plan.send.unwrap();
    assert!(send.contains("Use the model big with low effort."));
    assert!(send.ends_with("hi"));
    a.start = Some(StartProfile {
        agent: "gemini".into(),
        ..StartProfile::default()
    });
    let gemini = launch::profile_plan(&nixa_protocol::paths::data_dir(), &a, "x");
    match nixa_store::agents::start_command("gemini") {
        Some(line) => assert!(gemini.unwrap().argv.join(" ").starts_with(line)),
        None => assert!(gemini.is_none()),
    }
    assert_eq!(launch::base_command(&nixa_protocol::paths::data_dir(), "gemini").as_deref(), nixa_store::agents::start_command("gemini"));
}

#[test]
fn messages_carry_purpose_and_instructions() {
    let mut a = agent("a3", "claude");
    assert_eq!(launch::with_instructions(&a, "go"), "go");
    a.instructions = "Name the source.".into();
    assert_eq!(
        launch::with_instructions(&a, "go"),
        "Your standing instructions as News (a3):\nName the source.\n\ngo"
    );
    let first = launch::first_message(&a, "");
    assert!(first.starts_with("You are the custom agent News (a3). Your purpose: news researcher"));
    assert!(first.contains(launch::DEEP_ANSWER_RULE));
    assert!(first.contains(&crate::status_rule::rule()), "a custom agent tells Nixa its status");
}

#[test]
fn the_provider_comes_from_the_label_or_the_command() {
    let mut t = term(
        4,
        &[
            "sh",
            "-lc",
            "claude --resume abc --dangerously-skip-permissions",
        ],
        None,
        true,
    );
    assert_eq!(live::provider_of(&t).as_deref(), Some("claude"));
    assert_eq!(live::front_program(&t), "claude");
    assert!(live::agent_running(&t));
    t.labels
        .insert(RESUME_LABEL.into(), "codex resume s-9".into());
    assert_eq!(live::provider_of(&t).as_deref(), Some("codex"));
    let shell = term(5, &[], None, true);
    assert!(!live::agent_running(&shell));
    let hooked = term(6, &[], Some(Status::Idle), true);
    assert!(live::agent_running(&hooked));
    assert!(!live::agent_running(&term(7, &["claude"], None, false)));
    assert_eq!(
        live::front_program(&term(
            8,
            &["env", "DIMCODE_UPDATE_CHECK=0", "dima"],
            None,
            true
        )),
        "dima"
    );
}

#[test]
fn views_follow_the_live_terminal() {
    let mut a = agent("a1", "claude");
    a.terminal = "m4".into();
    let mut t = term(4, &["claude"], Some(Status::Working), true);
    t.labels
        .insert(RESUME_LABEL.into(), "claude --resume sess-1".into());
    t.labels.insert("space".into(), "s2".into());
    let mut live = Live {
        spaces: spaces(),
        numbers: HashMap::from([("m4".to_string(), 3)]),
        ..Live::default()
    };
    live.terms.insert("m4".into(), t.clone());
    let (view, seen) = live::view_of(&nixa_protocol::paths::data_dir(), &a, &live, 1_000_000);
    assert_eq!(view.state, AgentState::Working);
    assert_eq!(view.number, Some(3));
    assert_eq!(view.agent.session, "sess-1");
    assert_eq!(view.agent.space, "s2");
    assert_eq!(view.agent.space_name, "site");
    assert!(view.can_resume);
    assert_eq!(view.resume, nixa_store::agents::resume_command("claude", "sess-1").unwrap());
    assert!(seen.changed() && seen.bump);
    assert!(live::view_line(&view).ends_with("working in terminal 3 (m4): its terminal reports working"));
    t.cmd = vec!["codex".into()];
    t.labels.clear();
    live.terms.insert("m4".into(), t);
    let (view, _) = live::view_of(&nixa_protocol::paths::data_dir(), &a, &live, 1_000_000);
    assert_eq!(view.state, AgentState::Closed);
    assert_eq!(view.number, None);
    assert_eq!(view.why, "no session id was saved");
    let mut blocked = term(4, &["claude"], Some(Status::Blocked), true);
    blocked.labels.insert("nixa.space".into(), "s1".into());
    live.terms.insert("m4".into(), blocked);
    assert_eq!(live::view_of(&nixa_protocol::paths::data_dir(), &a, &live, 1).0.state, AgentState::Waiting);
    let json = serde_json::to_value(live::view_of(&nixa_protocol::paths::data_dir(), &a, &live, 1).0).unwrap();
    assert_eq!(json["state"], "waiting");
    assert_eq!(json["space_name"], "nixa");
    assert_eq!(json["can_resume"], false);
}

#[test]
fn steps_pick_reuse_copy_busy_or_open() {
    let live = Live {
        spaces: spaces(),
        ..Live::default()
    };
    let mut a = agent("a1", "claude");
    let (mut view, _) = live::view_of(&nixa_protocol::paths::data_dir(), &a, &live, 1);
    let input = UseInput::default();
    assert_eq!(start::step(&view, &input, false), Step::Open);
    view.agent.off = true;
    assert_eq!(start::step(&view, &input, false), Step::Off);
    assert_eq!(
        start::step(
            &view,
            &UseInput {
                by_user: true,
                ..UseInput::default()
            },
            false
        ),
        Step::Open
    );
    view.agent.off = false;
    view.state = AgentState::Working;
    assert_eq!(start::step(&view, &input, false), Step::Busy);
    view.agent.start = Some(StartProfile {
        agent: "codex".into(),
        ..StartProfile::default()
    });
    assert_eq!(start::step(&view, &input, false), Step::Copy);
    view.state = AgentState::Free;
    view.agent.terminal = "m2".into();
    assert_eq!(start::step(&view, &input, false), Step::Send("m2".into()));
    assert_eq!(start::step(&view, &input, true), Step::Copy);
    a.space = String::new();
    assert_eq!(
        start::space_for(&a, "site", &spaces()).map(|s| s.id),
        Some("s2".into())
    );
    assert_eq!(start::space_for(&a, "", &spaces()), None);
    a.space = "s1".into();
    assert_eq!(
        start::space_for(&a, "site", &spaces()).map(|s| s.id),
        Some("s1".into())
    );
    a.start = view.agent.start.clone();
    assert_eq!(
        start::space_for(&a, "/var", &spaces()).map(|s| s.id),
        Some("s2".into())
    );
    a.space = "gone".into();
    a.space_name = "gone".into();
    a.start = None;
    assert_eq!(start::space_for(&a, "", &spaces()), None);
}

#[test]
fn stale_space_id_matches_open_space_by_name_or_folder() {
    let open = vec![
        Space {
            id: "s1".into(),
            name: "other".into(),
            folder: "/tmp".into(),
        },
        Space {
            id: "s6".into(),
            name: "nixa".into(),
            folder: "/tmp/apps/nixa".into(),
        },
    ];
    let mut a = agent("a17", "jcode");
    a.space = "s2".into();
    a.space_name = "nixa".into();
    a.folder = "/tmp/moved/nixa".into();
    assert_eq!(
        start::space_for(&a, "", &open).map(|s| s.id),
        Some("s6".into())
    );
    a.space_name = String::new();
    a.folder = "/tmp/apps/nixa".into();
    assert_eq!(
        start::space_for(&a, "", &open).map(|s| s.id),
        Some("s6".into())
    );
    a.folder = "/tmp/closed-child".into();
    assert_eq!(start::space_for(&a, "", &open), None);
    a.space = "nixa".into();
    assert_eq!(
        start::space_for(&a, "", &open).map(|s| s.id),
        Some("s6".into())
    );
    a.space = "s2".into();
    a.space_name = "closed".into();
    assert_eq!(start::space_for(&a, "", &open), None);
}

#[test]
fn launch_picks_resume_start_or_profile() {
    let fixture = tempfile::tempdir().unwrap();
    let dir = fixture.path();
    crate::git_tools::gate::write_gate(dir, std::path::Path::new("/test/nixa")).unwrap();
    let live = Live {
        spaces: spaces(),
        ..Live::default()
    };
    let mut a = agent("a1", "claude");
    a.session = "sess-2".into();
    let (view, _) = live::view_of(dir, &a, &live, 1);
    let l = start::launch_for(dir, &view, "next").unwrap();
    assert_eq!(
        l.argv,
        [
            "sh".to_string(),
            "-lc".to_string(),
            crate::git_tools::gate::gated_line(dir, &nixa_store::agents::resume_command("claude", "sess-2").unwrap())
        ]
    );
    assert!(!l.fresh && l.send.ends_with("next"));
    a.session.clear();
    let (view, _) = live::view_of(dir, &a, &live, 1);
    let l = start::launch_for(dir, &view, "").unwrap();
    assert!(l.fresh && !l.profiled);
    let command = nixa_store::agents::start_command_in(dir, "claude").unwrap();
    assert_eq!(l.argv[2], crate::git_tools::gate::gated_line(dir, command));
    assert!(l.send.contains("You are the custom agent News"));
    a.start = Some(StartProfile {
        agent: "claude".into(),
        model: "opus".into(),
        ..StartProfile::default()
    });
    let (view, _) = live::view_of(dir, &a, &live, 1);
    let l = start::launch_for(dir, &view, "").unwrap();
    assert!(l.profiled && l.send.contains("You are News"));
    let profile = format!("{command} --model opus");
    assert!(l.argv[2].starts_with(&crate::git_tools::gate::gated_line(dir, &profile)));
    let none = agent("a2", "");
    let (view, _) = live::view_of(dir, &none, &live, 1);
    assert_eq!(start::launch_for(dir, &view, ""), None);
    assert_eq!(view.why, "no provider was saved");
}

#[test]
fn start_problems_are_short_and_plain() {
    let screen = "╭── box\nsh: jcode: command not found\n$ ";
    assert_eq!(start::start_problem(screen), "sh: jcode: command not found");
    assert_eq!(start::start_problem("one\ntwo\nthree"), "two three");
    let long = format!("error {}", "x".repeat(400));
    assert_eq!(start::start_problem(&long).chars().count(), 240);
    let text = start::not_started("News", "m3", "sh: jcode: command not found");
    assert!(text.starts_with(
        "News did not start in terminal m3: its terminal closed (sh: jcode: command not found)."
    ));
    let spec = start::tab_spec(&agent("a1", "claude"), &spaces()[0], vec!["sh".into()], None);
    assert!(spec.parent.is_none());
    let parent: nixa_protocol::TermId = "m7".parse().unwrap();
    let child = start::tab_spec(&agent("a1", "claude"), &spaces()[0], vec!["sh".into()], Some(parent.clone()));
    assert_eq!(child.parent, Some(parent));
    assert_eq!(spec.labels["space"], "s1");
    assert_eq!(spec.labels["nixa.space"], "s1");
    assert_eq!(spec.labels["nixa.custom"], "a1");
    assert!(spec.labels["tab"].starts_with("ca1"));
    assert_eq!(spec.cwd.as_deref(), Some("/tmp"));
    assert_eq!(spec.name.as_deref(), Some("News"));
}

#[test]
fn a_question_on_the_screen_stops_the_first_message() {
    let update = "✨ Update available! 0.169.0 -> 0.171.0\n\n› 1. Update now (runs the installer)\n  2. Skip\n  3. Skip until next version\n\n  Press enter to continue";
    assert!(start::asks(update));
    assert!(start::asks("Overwrite the file? [y/N]"));
    assert!(!start::asks(
        "› Ask Codex to do anything\n\n  100% context left"
    ));
    assert!(start::not_sent(start::Ready::Asking).contains("did not type into it"));
    assert!(start::not_sent(start::Ready::Late).contains("did not get ready in time"));
}

#[test]
fn arguments_become_store_changes() {
    assert_eq!(read_off(Some(&json!("off"))), Ok(Some(true)));
    assert_eq!(read_off(Some(&json!("on"))), Ok(Some(false)));
    assert_eq!(read_off(Some(&json!(true))), Ok(Some(true)));
    assert_eq!(read_off(None), Ok(None));
    assert!(read_off(Some(&json!("maybe"))).is_err());
    let change = edit_of(&args(json!({"title": " New ", "instructions": "", "agent": "Codex", "effort": "high", "off": "on", "role": "none"}))).unwrap();
    assert_eq!(change.title.as_deref(), Some("New"));
    assert_eq!(change.instructions.as_deref(), Some(""));
    assert_eq!(change.profile_agent.as_deref(), Some("codex"));
    assert_eq!(change.profile_effort.as_deref(), Some("high"));
    assert_eq!(change.off, Some(false));
    assert_eq!(change.role.as_deref(), Some("none"));
    let add = add_of(
        &args(json!({"title": "T", "purpose": "P", "space": "SITE", "provider": "codex"})),
        &spaces(),
    )
    .unwrap();
    assert_eq!(
        (
            add.space.as_str(),
            add.space_name.as_str(),
            add.folder.as_str()
        ),
        ("s2", "site", "/var")
    );
    assert!(add.start.is_none());
    let missing = add_of(&args(json!({"title": "T", "space": "nope"})), &spaces()).unwrap_err();
    assert_eq!(
        missing,
        "No space matches \"nope\". Open spaces: nixa, site."
    );
    let mut a = agent("a5", "codex");
    a.role = "reviewer".into();
    a.start = Some(StartProfile {
        agent: "codex".into(),
        ..StartProfile::default()
    });
    a.off = true;
    assert_eq!(
        edit_line(&a),
        "Changed a5: \"News\" (news researcher), no instructions, role reviewer, starts as codex · default model · default effort, off."
    );
    assert!(actions::providers().contains(&"jcode"));
}

#[tokio::test]
async fn actions_add_edit_list_and_forget() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let mut rx = app.out.subscribe();
    let who = Who {
        name: "test".into(),
        by_user: true,
        caller: None,
    };
    let added = actions::action(&app, "add", &args(json!({"title": "Scout", "purpose": "find news", "provider": "codex", "agent": "codex", "model": "gpt-5"})), &who).await.unwrap();
    assert!(
        added.text.starts_with("Added. a1 \"Scout\""),
        "{}",
        added.text
    );
    assert_eq!(rx.recv().await.unwrap()["type"], super::CHANGED);
    assert_eq!(added.data["agent"]["state"], "closed");
    assert_eq!(added.data["agent"]["starts_new"], true);
    let edited = actions::action(
        &app,
        "edit",
        &args(json!({"id": "a1", "off": "off", "effort": "high"})),
        &who,
    )
    .await
    .unwrap();
    assert!(
        edited.text.contains("starts as codex · gpt-5 · high, off."),
        "{}",
        edited.text
    );
    let refused = actions::action(
        &app,
        "edit",
        &args(json!({"id": "a1", "agent": "claude", "role": "reviewer"})),
        &who,
    )
    .await
    .unwrap_err();
    assert!(refused.contains("A reviewer never runs on Claude"));
    let list = actions::action(&app, "list", &Args::new(), &who)
        .await
        .unwrap();
    assert_eq!(list.data["agents"].as_array().unwrap().len(), 1);
    assert_eq!(list.data["agents"][0]["off"], true);
    assert_eq!(list.data["workflows"], json!([]));
    assert!(
        list.data["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "claude")
    );
    let off = actions::action(
        &app,
        "use",
        &args(json!({"id": "a1"})),
        &Who {
            by_user: false,
            ..who.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(off.data["how"], "cannot");
    assert!(off.text.contains("is off. Turn it on first"));
    let found = actions::action(&app, "find", &args(json!({"query": "news"})), &who)
        .await
        .unwrap();
    assert_eq!(found.data["agents"][0]["id"], "a1");
    let none = actions::action(
        &app,
        "use",
        &args(json!({"query": "nothing like it"})),
        &who,
    )
    .await
    .unwrap();
    assert_eq!(none.data["how"], "none");
    let gone = actions::action(&app, "forget", &args(json!({"id": "a1"})), &who)
        .await
        .unwrap();
    assert_eq!(
        gone.text,
        "Forgot a1 \"Scout\". Its terminal, if open, stays open."
    );
    assert!(
        actions::action(&app, "nope", &Args::new(), &who)
            .await
            .unwrap_err()
            .starts_with("Choose one of: list, find")
    );
    assert!(
        actions::action(&app, "edit", &Args::new(), &who)
            .await
            .unwrap_err()
            .contains("edit needs id")
    );
}

#[tokio::test]
async fn routes_need_the_window_token() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let token = app.window_token.clone();
    let router = super::routes().with_state(app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    let http = reqwest::Client::new();
    let header = crate::window_key::HEADER;
    let refused = http
        .get(format!("http://{addr}/custom-agents"))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);
    let list: Value = http
        .get(format!("http://{addr}/custom-agents"))
        .header(header, &token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["ok"], true);
    assert_eq!(list["data"]["agents"], json!([]));
    let bad = http
        .post(format!("http://{addr}/custom-agents/add"))
        .header(header, &token)
        .json(&json!({"title": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let body: Value = bad.json().await.unwrap();
    assert_eq!(body["error"], "A custom agent needs a title and a purpose.");
    let added: Value = http
        .post(format!("http://{addr}/custom-agents/add"))
        .header(header, &token)
        .json(&json!({"title": "x", "purpose": "y"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(added["data"]["agent"]["id"], "a1");
    let wrong = http
        .post(format!("http://{addr}/custom-agents/forget"))
        .header(header, "nope")
        .json(&json!({"id": "a1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
}

#[tokio::test]
async fn websocket_requests_get_data_back() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    assert!(
        super::ws_request(&app, &json!({"type": "prompts", "request": "r1"}))
            .await
            .is_none()
    );
    assert!(
        super::ws_request(&app, &json!({"type": "custom_agents"}))
            .await
            .is_none()
    );
    let added = super::ws_request(&app, &json!({"type": "custom_agents", "request": "r2", "action": "add", "title": "Coder", "purpose": "code", "agent": "codex"})).await.unwrap();
    assert_eq!(added["type"], "custom_agents_reply");
    assert_eq!(added["request"], "r2");
    assert_eq!(added["ok"], true);
    let list = super::ws_request(
        &app,
        &json!({"type": "custom_agents", "request": "r3", "args": {"action": "list"}}),
    )
    .await
    .unwrap();
    assert_eq!(list["data"]["agents"][0]["start"]["agent"], "codex");
    assert_eq!(list["data"]["agents"][0]["state"], "closed");
    let bad = super::ws_request(
        &app,
        &json!({"type": "custom_agents", "request": "r4", "action": "edit"}),
    )
    .await
    .unwrap();
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["error"], "edit needs id.");
    assert!(super::is_ws(&json!({"type": "custom_agents"})));
}

#[test]
fn skip_check_flags_are_found() {
    let words = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
    let claude = launch::shell(&nixa_protocol::paths::data_dir(), "claude --dangerously-skip-permissions 'look at the tests'");
    assert!(nixa_store::agents::skips_agent_checks(&claude));
    assert!(nixa_store::agents::skips_agent_checks(&words("gemini --yolo")));
    assert!(nixa_store::agents::skips_agent_checks(&words(
        "omo --permission-preset full-access"
    )));
    assert!(nixa_store::agents::skips_agent_checks(&words(
        "codex resume s1 --dangerously-bypass-approvals-and-sandbox"
    )));
    assert!(!nixa_store::agents::skips_agent_checks(&words("claude --model opus")));
}

#[test]
fn start_lines_read_the_brain_data_folder() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(launch::base_command(dir.path(), "claude").as_deref(), Some("claude --dangerously-skip-permissions"));
    std::fs::write(dir.path().join(nixa_store::agents::PROGRAMS_FILE), r#"{"checksOn": ["claude"]}"#).unwrap();
    assert_eq!(launch::base_command(dir.path(), "claude").as_deref(), Some("claude"));
    assert_eq!(launch::base_command(dir.path(), "codex").as_deref(), Some("codex --dangerously-bypass-approvals-and-sandbox"));
    let other = tempfile::tempdir().unwrap();
    assert_eq!(launch::base_command(other.path(), "claude").as_deref(), Some("claude --dangerously-skip-permissions"));
}

#[test]
fn a_new_stage_model_starts_a_fresh_session() {
    let mut a = agent("a7", "claude");
    a.start = Some(StartProfile { agent: "claude".into(), model: "sonnet".into(), ..StartProfile::default() });
    let other = crate::schedules::runner::Want { model: "claude::opus".into(), effort: String::new() };
    let same = crate::schedules::runner::Want { model: "claude::sonnet".into(), effort: String::new() };
    assert!(start::model_changes(&a, Some(&other), None));
    assert!(!start::model_changes(&a, Some(&same), None));
    assert!(!start::model_changes(&a, None, None));
    let dir = tempfile::tempdir().unwrap();
    let conn = nixa_store::open(&dir.path().join("nixa.db")).unwrap();
    let mut b = a.clone();
    b.start = Some(StartProfile { agent: "claude".into(), model: "opus".into(), effort: "high".into(), ..StartProfile::default() });
    b.session = "s1".into();
    start::save_session_model(&conn, &b, "t9", "s1");
    a.session = "s1".into();
    let session = start::session_model(&conn, &a);
    assert_eq!(session, Some(("opus".to_string(), "high".to_string())));
    let mut replaced = a.clone();
    replaced.session = "s2".into();
    assert_eq!(start::session_model(&conn, &replaced), None);
    let opus = crate::schedules::runner::Want { model: "claude::opus".into(), effort: "high".into() };
    assert!(start::model_changes(&replaced, Some(&opus), None));
    let mut fresh = b.clone();
    fresh.session = String::new();
    start::save_session_model(&conn, &fresh, "t9", "");
    let mut pending = a.clone();
    pending.session = String::new();
    pending.terminal = "t9".into();
    assert_eq!(start::session_model(&conn, &pending), Some(("opus".to_string(), "high".to_string())));
    let mut edited = pending.clone();
    edited.session = "s2".into();
    assert_eq!(start::session_model(&conn, &edited), None);
    assert!(start::model_changes(&edited, Some(&opus), None));
    start::attach_session(&conn, &pending, "s3");
    pending.session = "s3".into();
    pending.terminal = String::new();
    assert_eq!(start::session_model(&conn, &pending), Some(("opus".to_string(), "high".to_string())));
    let mut pending_older = b.clone();
    pending_older.session = "s1".into();
    pending_older.terminal = "t9".into();
    start::save_session_model(&conn, &pending_older, "t9", "");
    let mut older = a.clone();
    older.session = "s1".into();
    older.terminal = "t9".into();
    assert_eq!(start::session_model(&conn, &older), None);
    assert!(start::model_changes(&older, Some(&opus), None));
    let back = crate::schedules::runner::Want { model: "claude::sonnet".into(), effort: String::new() };
    assert!(start::model_changes(&a, Some(&back), session.as_ref()));
    let again = crate::schedules::runner::Want { model: "claude::opus".into(), effort: "high".into() };
    assert!(!start::model_changes(&a, Some(&again), session.as_ref()));
    let view = live::AgentView { agent: a, state: AgentState::Free, number: None, resume: "claude --resume s1".into(), can_resume: true, starts_new: false, why: String::new(), busy: Vec::new(), unsure: String::new(), sure: true };
    let (fresh, next) = start::for_model(view.clone(), Step::Send("t1".into()), true);
    assert_eq!(next, Step::Copy);
    assert!(fresh.resume.is_empty() && fresh.starts_new);
    let (kept, next) = start::for_model(view, Step::Send("t1".into()), false);
    assert_eq!(next, Step::Send("t1".into()));
    assert_eq!(kept.resume, "claude --resume s1");
}

#[test]
fn only_a_joined_deep_brain_seat_may_use_agents() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let space = crate::spaces::Space { id: "s1".into(), name: "One".into(), folder: "/tmp/one".into() };
    crate::app::lock(&app.deep).join("deep-seat-1", "claude", &space, None, 1).unwrap();
    let who = super::routes::deep_seat_who(&app, "deep-seat-1").expect("the deep brain seat is allowed");
    assert!(!who.by_user);
    assert_eq!(who.caller.as_deref(), Some("deep-seat-1"));
    assert!(super::routes::deep_seat_who(&app, "agent-term-2").is_none());
}

#[tokio::test]
async fn another_agent_terminal_cannot_use_agents() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let router = super::routes().with_state(app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    let http = reqwest::Client::new();
    for body in [json!({"id": "a1"}), json!({"id": "a1", "terminal": "agent-term-2"})] {
        let refused = http.post(format!("http://{addr}/custom-agents/use")).json(&body).send().await.unwrap();
        assert!(matches!(refused.status().as_u16(), 401 | 403), "{}", refused.status());
    }
    let edit = http.post(format!("http://{addr}/custom-agents/add")).json(&json!({"terminal": "agent-term-2"})).send().await.unwrap();
    assert_eq!(edit.status(), 401);
}
