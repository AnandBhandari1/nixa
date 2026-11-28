use super::*;
use nixa_phone::DoorNames;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn test_door(app: &App) -> Door {
    let mut deps = Deps::new(&app.dir, Arc::new(|| json!({"state": "idle"})));
    deps.port = 0;
    deps.setup_port = 0;
    deps.address = Arc::new(|| Some("127.0.0.1".to_string()));
    deps.names = Arc::new(|_| DoorNames { ips: vec!["127.0.0.1".to_string()], dns: Vec::new() });
    let (door, _events) = Door::new(deps).unwrap();
    *lock(&app.phone.door) = Some(door.clone());
    door
}

#[test]
fn the_phone_cannot_flip_mobile_mode() {
    let (_dir, app) = app();
    on_command(&app, "p1", PhoneCommand::SetMobile(true));
    assert!(!saved(&app).mobile);
    set_mobile(&app, true).unwrap();
    on_command(&app, "p1", PhoneCommand::SetMobile(false));
    assert!(saved(&app).mobile);
}

#[test]
fn no_door_gives_an_error_and_off() {
    let (_dir, app) = app();
    let now = view(&app);
    assert_eq!(now["phone"]["on"], json!(false));
    assert_eq!(now["mobile"], json!(false));
    assert!(now["phone"]["error"].as_str().unwrap().starts_with(NOT_STARTED));
}

#[test]
fn window_kinds_and_snapshots_go_to_the_phone() {
    let (_dir, app) = app();
    let level = json!({"type": "level", "mic": 0.4, "talking": true});
    assert_eq!(relayed(&app, &level), Some(level.clone()));
    let snap = relayed(&app, &json!({"type": "chat", "item": {}})).unwrap();
    assert_eq!(snap["type"], json!("snapshot"));
    assert_eq!(snap["snapshot"]["name"], json!(app.name()));
    assert!(snap["snapshot"]["items"].is_array());
    assert_eq!(relayed(&app, &json!({"type": "models"})), None);
}

#[test]
fn say_options_read_the_message() {
    let options = say_options(&json!({"text": "Done", "agent": "nixa", "kind": "git"}));
    assert_eq!(options.agent.as_deref(), Some("nixa"));
    assert_eq!(options.kind.as_deref(), Some("git"));
    assert!(options.voice.is_none());
    assert_eq!(say_options(&json!({"text": "x"})).agent.as_deref(), Some("nixa"));
}

#[tokio::test]
async fn phone_say_is_known_and_needs_text() {
    let (_dir, app) = app();
    assert!(ws(&app, "phone_say", &json!({"text": "Commit message ready", "agent": "nixa", "kind": "git"})));
    assert!(!ws(&app, "phone_say", &json!({"text": "  "})));
    assert!(!ws(&app, "other", &json!({})));
}

#[tokio::test]
async fn mobile_is_saved_and_needs_an_open_door_for_the_voice() {
    let (_dir, app) = app();
    set_mobile(&app, true).unwrap();
    assert!(saved(&app).mobile);
    assert!(!voice_on_phone(&app));
    let _door = test_door(&app);
    set_door(&app, true).await.unwrap();
    assert!(voice_on_phone(&app));
    let now = view(&app);
    assert_eq!(now["phone"]["on"], json!(true));
    assert_eq!(now["phone"]["address"], json!("127.0.0.1"));
    assert!(now["phone"]["root_sha256"].as_str().is_some_and(|s| s.len() == 64));
    assert!(now["phone"]["pair_qr"].as_array().is_some_and(|rows| !rows.is_empty()));
    set_door(&app, false).await.unwrap();
    assert!(!saved(&app).on);
    assert!(!voice_on_phone(&app));
    assert_eq!(view(&app)["phone"]["on"], json!(false));
}

#[tokio::test]
async fn phone_routes_need_the_window_token() {
    let (_dir, app) = app();
    let _door = test_door(&app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = routes().with_state(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    let http = reqwest::Client::new();
    let header = crate::window_key::HEADER;
    assert_eq!(http.get(format!("{base}/phone")).send().await.unwrap().status(), 401);
    let seen: Value = http.get(format!("{base}/phone")).header(header, &app.window_token).send().await.unwrap().json().await.unwrap();
    assert_eq!((seen["ok"].clone(), seen["phone"]["on"].clone()), (json!(true), json!(false)));
    let moved: Value = http.post(format!("{base}/phone")).header(header, &app.window_token).json(&json!({"type": "set_mobile", "on": true})).send().await.unwrap().json().await.unwrap();
    assert_eq!(moved["mobile"], json!(true));
    let wrong = http.post(format!("{base}/phone")).header(header, &app.window_token).json(&json!({"type": "phone_remove", "id": "nope"})).send().await.unwrap();
    assert_eq!(wrong.status(), 400);
    let body: Value = wrong.json().await.unwrap();
    assert_eq!((body["ok"].clone(), body["error"].clone()), (json!(false), json!("No such phone.")));
}

#[test]
fn mac_voice_makes_pcm_for_the_phone() {
    if !Path::new("/usr/bin/say").exists() {
        return;
    }
    let pcm = mac_synth("Hello from the phone door.", None, &CancellationToken::new()).expect("say made a wav");
    assert_eq!(pcm.rate, SAY_RATE);
    assert!(pcm.pcm.len() > 1000);
    let stopped = CancellationToken::new();
    stopped.cancel();
    assert!(mac_synth("Hello", None, &stopped).is_none());
}

#[tokio::test]
async fn phone_talk_events_hold_and_free_spoken_lines() {
    let (_dir, app) = app();
    let sink = PhoneSink::new(test_door(&app), synth());
    let start = || PhoneEvent::TalkStart { phone: "p1".into(), auto: true };
    on_event(&app, &sink, start());
    assert!(crate::talk::hold::holding(&app));
    crate::voice::say(&app, "Agent 1 needs you.", None);
    assert_eq!(lock(&app.talk.hold).held(), 1);
    on_event(&app, &sink, PhoneEvent::TalkCancel { phone: "p1".into() });
    assert!(!crate::talk::hold::holding(&app));
    on_event(&app, &sink, start());
    on_event(&app, &sink, PhoneEvent::Talk { phone: "p1".into(), pcm: Vec::new(), started_at: now_ms(), auto: true });
    assert!(crate::talk::hold::holding(&app));
    crate::voice::say(&app, "Agent 2 is done.", None);
    for _ in 0..1000 {
        if !crate::talk::hold::holding(&app) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!crate::talk::hold::holding(&app));
}

#[tokio::test]
async fn push_to_talk_cuts_the_line_playing_on_the_phone() {
    let (_dir, app) = app();
    let sink = PhoneSink::new(test_door(&app), synth());
    let (tx, mut rx) = unbounded_channel::<PhoneSay>();
    crate::talk::speech::set_phone(&app, Some(tx));
    crate::app::lock(&app.voice.settings).voice_on = true;
    crate::talk::speech::speak_later(&app, "A long reply for the phone.", SayOptions::nixa("chat"));
    let job = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert!(!job.stop.is_cancelled());
    on_event(&app, &sink, PhoneEvent::TalkStart { phone: "p1".into(), auto: false });
    assert!(job.stop.is_cancelled());
    let _ = job.done.send(nixa_voice::SayResult { interrupted: true, ms: 0 });
    on_event(&app, &sink, PhoneEvent::TalkCancel { phone: "p1".into() });
}

#[tokio::test]
async fn a_phone_line_cut_before_it_starts_is_never_played() {
    let (_dir, app) = app();
    let made = Arc::new(AtomicUsize::new(0));
    let count = made.clone();
    let counting: Synth = Arc::new(move |_, _, _| {
        count.fetch_add(1, Ordering::SeqCst);
        None
    });
    let door = test_door(&app);
    let sink = PhoneSink::new(door.clone(), counting);
    let (tx, rx) = unbounded_channel::<PhoneSay>();
    tokio::spawn(play_loop(door, sink, rx));
    let stop = CancellationToken::new();
    stop.cancel();
    let (done, result) = tokio::sync::oneshot::channel();
    tx.send(PhoneSay { text: "Agent 2 is done.".into(), voice: "v".into(), options: SayOptions::default(), done, stop }).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), result).await.unwrap().unwrap();
    assert!(result.interrupted);
    assert_eq!(made.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn push_to_talk_cuts_a_line_that_passed_the_hold_check() {
    let (_dir, app) = app();
    let sink = PhoneSink::new(test_door(&app), synth());
    let (tx, mut rx) = unbounded_channel::<PhoneSay>();
    crate::talk::speech::set_phone(&app, Some(tx));
    crate::app::lock(&app.voice.settings).voice_on = true;
    let speaking = {
        let app = app.clone();
        tokio::spawn(async move { crate::talk::speech::say(&app, "A reply that starts now.", SayOptions::nixa("chat")).await })
    };
    let job = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    on_event(&app, &sink, PhoneEvent::TalkStart { phone: "p1".into(), auto: false });
    assert!(job.stop.is_cancelled());
    let _ = job.done.send(nixa_voice::SayResult { interrupted: true, ms: 0 });
    assert!(tokio::time::timeout(Duration::from_secs(5), speaking).await.unwrap().unwrap().interrupted);
    on_event(&app, &sink, PhoneEvent::TalkCancel { phone: "p1".into() });
}

#[test]
fn wake_checks_cut_only_for_real_user_words() {
    assert!(matches!(peek_step("", Some("hello there")), Peek::Quiet));
    assert!(matches!(peek_step("a . .", None), Peek::Quiet));
    assert!(matches!(peek_step("the build is green now", Some("The build is green now, and the tests pass.")), Peek::Own));
    assert!(matches!(peek_step("stop please", Some("The build is green now.")), Peek::Cut));
    assert!(matches!(peek_step("stop please", None), Peek::Cut));
    assert!(real_words("no"));
    assert!(!real_words("a"));
    assert!(own_voice("Green now", "the build is green now"));
    assert!(!own_voice("wait stop that", "the build is green now"));
}

#[tokio::test]
async fn wake_checks_run_only_while_nixa_speaks() {
    let (_dir, app) = app();
    assert!(!peek_wanted(&app));
    lock(&app.talk.speaker).active = 1;
    let open = nixa_voice::mic_open_during_voice(crate::settings_api::headphones(&app.dir), nixa_voice::echo_cancelled(), voice_on_phone(&app));
    assert_eq!(peek_wanted(&app), !open);
    lock(&app.talk.speaker).active = 0;
}

#[test]
fn phone_view_and_state_event_carry_camera() {
    let (_dir, app) = app();
    let now = view(&app);
    assert!(now["camera"].is_object());
    assert!(now["camera"].get("allow").is_none());
    assert!(now["camera"]["log"].as_array().is_some_and(|l| l.is_empty()));
    let event = state_event(&app);
    assert_eq!(event["type"], json!("phone"));
    assert!(event["camera"].is_object());
}

#[test]
fn phone_door_ws_knows_camera_settings_only_not_looks() {
    assert!(!is_ws("camera_allow"));
    assert!(!is_ws("camera_cloud"));
    assert!(is_ws("camera_log_clear"));
    assert!(!is_ws("nixa_phone_look"));
}

#[test]
fn caps_on_the_door_reaches_camera_state() {
    let (_dir, app) = app();
    let sink = PhoneSink::new(test_door(&app), synth());
    on_event(&app, &sink, PhoneEvent::Caps { phone: "p9".into(), camera: true });
    assert_eq!(crate::camera::caps(&app).get("p9"), Some(&true));
    on_event(&app, &sink, PhoneEvent::Caps { phone: "p9".into(), camera: false });
    assert_eq!(crate::camera::caps(&app).get("p9"), Some(&false));
}

#[test]
fn camera_lives_in_the_phone_state_event_not_the_chat_snapshot() {
    let (_dir, app) = app();
    let snap = relayed(&app, &json!({"type": "chat", "item": {}})).unwrap();
    assert_eq!(snap["type"], json!("snapshot"));
    assert!(snap["snapshot"].get("camera").is_none());
    assert!(state_event(&app)["camera"].is_object());
    let phone = relayed(&app, &json!({"type": "phone"})).unwrap();
    assert_eq!(phone["type"], json!("snapshot"));
    assert!(phone["snapshot"].get("camera").is_none());
}
