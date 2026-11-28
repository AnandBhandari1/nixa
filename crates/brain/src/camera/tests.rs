use super::gate;
use super::*;
use crate::app::lock;
use nixa_phone::CameraPhone;

fn phone(id: &str, name: &str) -> CameraPhone {
    CameraPhone { id: id.into(), name: name.into(), until: 9_000_000 }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nixa-camera-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn any_connected_phone_is_picked_with_no_limits() {
    let one = [phone("p1", "Pixel")];
    let two = [phone("p1", "Pixel"), phone("p2", "Tab")];
    let pick = |phones: &[CameraPhone], name: Option<&str>| gate::pick(phones, name).map(|p| p.id);
    assert_eq!(pick(&[], None), Err(gate::NO_PHONE.to_string()));
    assert_eq!(pick(&one, None), Ok("p1".to_string()));
    assert_eq!(pick(&two, None), Ok("p1".to_string()));
    assert_eq!(pick(&two, Some("tab")), Ok("p2".to_string()));
    assert!(pick(&two, Some("Watch")).unwrap_err().contains("No phone named"));
}

#[test]
fn lens_defaults_to_rear() {
    assert_eq!(lens_of(None), Ok(Lens::Rear));
    assert_eq!(lens_of(Some("front")), Ok(Lens::Front));
    assert_eq!(lens_of(Some(" Rear ")), Ok(Lens::Rear));
    assert!(lens_of(Some("side")).is_err());
    assert_eq!(lens_word(Lens::Front), "front");
}

#[test]
fn phone_reasons_are_plain() {
    assert_eq!(why_text("screen_off"), SCREEN_OFF);
    assert_eq!(why_text("camera_busy"), SCREEN_OFF);
    assert_eq!(why_text("denied"), DENIED);
    assert_eq!(why_text("x"), PHONE_FAILED);
}

#[test]
fn frames_are_private_and_bad_ones_are_dropped() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp("frames");
    let jpeg = |index| Frame { index, at: 0, jpeg: vec![0xFF, 0xD8, 0xFF, 0xE0, index] };
    let frames = vec![jpeg(2), Frame { index: 1, at: 0, jpeg: b"not a jpeg".to_vec() }, jpeg(0), jpeg(0)];
    let files = write_frames(&dir, 7, &frames).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("00000007-0.jpg"));
    assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777, 0o600);
    remove_files(&files);
    assert!(files.iter().all(|f| !f.exists()));
    assert_eq!(write_frames(&dir, 8, &[Frame { index: 0, at: 0, jpeg: vec![1, 2, 3] }]), Err(BAD_FRAMES.to_string()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_sweep_removes_old_frames_only() {
    let dir = temp("sweep");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.jpg"), b"x").unwrap();
    let now = now_ms();
    assert_eq!(sweep(&dir, now, TMP_KEEP_MS), 0);
    assert!(dir.join("a.jpg").exists());
    assert_eq!(sweep(&dir, now + TMP_KEEP_MS + 5_000, TMP_KEEP_MS), 1);
    assert!(!dir.join("a.jpg").exists());
    std::fs::write(dir.join("b.jpg"), b"x").unwrap();
    assert_eq!(sweep(&dir, now, 0), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_log_is_text_kept_seven_days_and_can_be_cleared() {
    let dir = temp("log");
    let now = 100 * log::KEEP_MS;
    let line = |at, q: &str| log::Line { at, phone: "Pixel".into(), camera: "rear".into(), question: q.into(), answer: "A person walks.".into(), model: "m".into(), brain: "b".into(), frames: 3, cloud: false };
    log::add(&dir, line(now - log::KEEP_MS - 1, "old"), now - log::KEEP_MS - 1).unwrap();
    log::add(&dir, line(now, "new"), now).unwrap();
    let lines = log::read(&dir, now);
    assert_eq!(lines.iter().map(|l| l.question.as_str()).collect::<Vec<_>>(), vec!["new"]);
    let text = std::fs::read_to_string(log::file_of(&dir)).unwrap();
    assert!(!text.contains(".jpg") && text.contains("\"camera\":\"rear\""));
    log::clear(&dir).unwrap();
    assert!(log::read(&dir, now).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_tool_is_only_in_the_chat_brain_list() {
    let dir = temp("tools");
    let app = crate::app::App::open(&dir).unwrap();
    assert_eq!(specs().len(), 1);
    assert_eq!(specs()[0].name, NAME);
    let specs = crate::tools::specs(&app);
    let chat: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    assert!(chat.contains(&NAME));
    assert!(!crate::nixa_tools::NAMES.contains(&NAME));
    for other in ["nixa_agent_start", "nixa_schedules", "nixa_workflows", "nixa_deep_ask", "nixa_debate"] {
        assert_ne!(other, NAME);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_privacy_report_lists_camera_looks() {
    use crate::privacy::report::{CameraLooks, Reach, camera_path};
    let cloud = crate::brains::SavedBrain { id: "c".into(), name: "Cloud".into(), kind: crate::provider::Kind::Openai, base_url: "https://api.example.com/v1".into(), model: "m".into(), effort: None, key_env: None, added_at: 0 };
    let off = camera_path(CameraLooks { allow: false, cloud: false }, false, &[]);
    assert!(!off.in_use);
    assert_eq!(camera_path(CameraLooks { allow: true, cloud: false }, false, std::slice::from_ref(&cloud)).reach, Reach::LocalNetwork);
    assert_eq!(camera_path(CameraLooks { allow: true, cloud: true }, true, std::slice::from_ref(&cloud)).reach, Reach::LocalNetwork);
    let on = camera_path(CameraLooks { allow: true, cloud: true }, false, std::slice::from_ref(&cloud));
    assert_eq!(on.reach, Reach::Internet);
    assert!(on.detail.contains("api.example.com"));
}

#[tokio::test]
async fn the_log_clear_command_works_and_no_switches_are_left() {
    let dir = temp("settings");
    let app = crate::app::App::open(&dir).unwrap();
    assert!(apply(&app, "camera_allow", &json!({"on": true})).is_err());
    assert!(!is_command("camera_cloud"));
    log::add(&dir.join(FOLDER), log::Line { at: now_ms(), question: "q".into(), ..Default::default() }, now_ms()).unwrap();
    let seen = view(&app);
    assert!(seen.get("allow").is_none() && seen.get("sessions").is_none());
    assert_eq!(seen["log"].as_array().map(Vec::len), Some(1));
    apply(&app, "camera_log_clear", &json!({})).unwrap();
    assert_eq!(view(&app)["log"].as_array().map(Vec::len), Some(0));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_look_without_a_phone_door_says_so() {
    let dir = temp("refuse");
    let app = crate::app::App::open(&dir).unwrap();
    let args: crate::tools::Args = serde_json::from_value(json!({"question": "what is happening"})).unwrap();
    set_looks(&app, true).unwrap();
    assert_eq!(call(&app, &args).await, Err(NO_DOOR.to_string()));
    let bad: crate::tools::Args = serde_json::from_value(json!({"question": "q", "camera": "side"})).unwrap();
    assert!(call(&app, &bad).await.unwrap_err().contains("rear or front"));
    std::fs::remove_dir_all(&dir).ok();
}

async fn fake_ollama() -> String {
    let router = axum::Router::new().route(
        "/api/tags",
        axum::routing::get(|| async {
            axum::Json(json!({"models": [
                {"name": "text-1:latest", "capabilities": ["completion", "tools"]},
                {"name": "eye-1:latest", "capabilities": ["completion", "vision"]}
            ]}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    base
}

#[tokio::test]
async fn the_model_stays_local_only_while_privacy_is_on() {
    use crate::brains::Checked;
    use crate::provider::Kind;
    use nixa_store::models::{self, By, ModelEdit, model_rows};
    let dir = temp("model");
    let app = crate::app::App::open(&dir).unwrap();
    assert!(model_pick(&app).unwrap_err().contains("No local vision model"));
    let conn = crate::models::open(&app).unwrap();
    models::update(&conn, |cache| {
        models::pick::ensure_map(cache, crate::util::now_ms() as i64, &models::lists::fixed_models())
    })
    .unwrap();
    let id = model_rows(&models::load(&conn).unwrap())
        .into_iter()
        .find(|r| r.listed)
        .map(|r| r.id)
        .expect("a listed model");
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
    let local = Checked { name: "Local".into(), kind: Kind::Ollama, base_url: fake_ollama().await, key_env: None, model: model.clone() };
    let local_id = app.brains.add(local).unwrap().id;
    let cache = models::load(&conn).unwrap();
    assert!(crate::desktop::vision::pairs(&crate::desktop::vision::vision_rows(&cache), &app.brains.list(), false).is_empty());
    crate::desktop::vision::refresh_server(&app, false).await;
    assert_eq!(model_pick(&app), Ok("eye-1:latest on Local".to_string()));
    crate::settings_api::set_privacy(&app, true).unwrap();
    assert_eq!(model_pick(&app), Ok("eye-1:latest on Local".to_string()));
    app.brains.remove(&local_id).unwrap();
    let cloud = Checked {
        name: "Cloud".into(),
        kind: Kind::Openai,
        base_url: "https://cloud-vision.test.invalid/v1".into(),
        key_env: None,
        model: model.clone(),
    };
    app.brains.add(cloud).unwrap();
    crate::settings_api::set_privacy(&app, false).unwrap();
    let cache = models::load(&conn).unwrap();
    let queue = crate::desktop::vision::pairs(&crate::desktop::vision::vision_rows(&cache), &app.brains.list(), false);
    assert!(queue.iter().any(|(_, b)| b.name == "Cloud"));
    let picked = model_pick(&app);
    assert!(picked.is_ok(), "model pick with cloud and privacy off: {:?}", picked);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_fresh_install_has_no_camera_folder_yet() {
    let dir = temp("fresh");
    let app = crate::app::App::open(&dir).unwrap();
    assert!(!dir.join(FOLDER).exists());
    assert_eq!(view(&app)["log"].as_array().map(Vec::len), Some(0));
    assert_eq!(view(&app)["model_ok"], json!(false));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn frame_files_never_land_in_chat_or_log_text() {
    let dir = temp("private");
    let app = crate::app::App::open(&dir).unwrap();
    let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    let files = write_frames(
        &tmp_dir(&app),
        9,
        &[
            Frame { index: 0, at: 0, jpeg: jpeg.clone() },
            Frame { index: 1, at: 0, jpeg: jpeg.clone() },
            Frame { index: 2, at: 0, jpeg },
        ],
    )
    .unwrap();
    assert_eq!(files.len(), 1);
    remove_files(&files);
    assert!(files.iter().all(|p| !p.exists()));
    log::add(&dir.join(FOLDER), log::Line { at: now_ms(), question: "q".into(), answer: "Someone sits.".into(), ..Default::default() }, now_ms()).unwrap();
    let text = std::fs::read_to_string(log::file_of(&dir.join(FOLDER))).unwrap();
    assert!(!text.contains(".jpg"));
    app.add_chat(crate::chat::Who::Nixa, "Someone sits.", crate::chat::Extra::default());
    assert!(!lock(&app.chat).list().iter().any(|i| i.text.contains(".jpg")));
    std::fs::write(tmp_dir(&app).join("old.jpg"), b"x").unwrap();
    assert_eq!(sweep(&tmp_dir(&app), now_ms(), 0), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn log_writes_do_not_lose_lines_when_they_run_at_once() {
    let dir = temp("loglock");
    let now = now_ms();
    let workers: Vec<_> = (0..8)
        .map(|w| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                for i in 0..10 {
                    log::add(&dir, log::Line { at: now, question: format!("{w}-{i}"), ..Default::default() }, now).unwrap();
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    assert_eq!(log::read(&dir, now).len(), 80);
    let adder = {
        let dir = dir.clone();
        std::thread::spawn(move || log::add(&dir, log::Line { at: now, question: "late".into(), ..Default::default() }, now).unwrap())
    };
    log::clear(&dir).unwrap();
    adder.join().unwrap();
    assert!(log::read(&dir, now).len() <= 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn clear_and_prune_remove_temp_copies() {
    let dir = temp("logtemp");
    std::fs::create_dir_all(&dir).unwrap();
    let stray = dir.join("log.tmp123.4");
    std::fs::write(&stray, "[{\"answer\":\"The child is dancing.\"}]").unwrap();
    log::clear(&dir).unwrap();
    assert!(!stray.exists());
    std::fs::write(&stray, "x").unwrap();
    assert_eq!(log::prune(&dir, now_ms()), Ok(true));
    assert!(!stray.exists());
    assert_eq!(log::prune(&dir, now_ms()), Ok(false));
    let old = log::Line { at: 1, question: "old".into(), ..Default::default() };
    std::fs::write(log::file_of(&dir), serde_json::to_string(&vec![old]).unwrap()).unwrap();
    assert_eq!(log::prune(&dir, now_ms()), Ok(true));
    assert!(log::read(&dir, now_ms()).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn frames_are_gone_within_ten_minutes() {
    assert!(TMP_KEEP_MS + SWEEP_EVERY.as_millis() as u64 <= 10 * 60_000);
    let dir = temp("sweep10");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 0]).unwrap();
    assert_eq!(sweep(&dir, now_ms() + 8 * 60_000, TMP_KEEP_MS), 0);
    assert_eq!(sweep(&dir, now_ms() + TMP_KEEP_MS, TMP_KEEP_MS), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn each_phone_shows_its_camera_support() {
    let dir = temp("caps");
    let app = crate::app::App::open(&dir).unwrap();
    let row = |id: &str| nixa_phone::PhoneRow { id: id.into(), name: id.into(), paired_at: 1, last_seen: None, connected: false, voice: false, checks: Default::default() };
    let info = nixa_phone::PhoneInfo { phones: vec![row("old"), row("new")], ..Default::default() };
    assert!(!supported(&info, &caps(&app)));
    assert_eq!(set_caps(&app, "new", true), Ok(true));
    assert_eq!(set_caps(&app, "new", true), Ok(false));
    let seen = phone_view(&info, &caps(&app));
    assert_eq!(seen["phones"][0]["camera"], json!(false));
    assert_eq!(seen["phones"][1]["camera"], json!(true));
    assert!(supported(&info, &caps(&app)));
    forget_caps(&app, "new").unwrap();
    assert!(!supported(&info, &caps(&app)));
    assert_eq!(view(&app)["supported"], json!(false));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn camera_changes_are_pushed_to_the_window() {
    let dir = temp("push");
    let app = crate::app::App::open(&dir).unwrap();
    let mut rx = app.out.subscribe();
    crate::settings_api::set_privacy(&app, false).unwrap();
    let mut phone_events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if event["type"] == "phone" {
            phone_events.push(event);
        }
    }
    assert!(phone_events.iter().any(|e| e["camera"]["privacy"] == json!(false)));
    assert!(phone_events.iter().all(|e| e["camera"]["supported"].is_boolean()));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn phone_looks_are_off_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    assert!(!looks_on(&app));
    let args = serde_json::json!({"question": "Is the door shut?"}).as_object().cloned().unwrap();
    assert_eq!(call(&app, &args).await.unwrap_err(), LOOKS_OFF);
    apply(&app, "camera_looks", &serde_json::json!({"on": true})).unwrap();
    assert!(looks_on(&app));
    assert_eq!(view(&app)["looks"], true);
    assert_ne!(call(&app, &args).await.unwrap_err(), LOOKS_OFF);
}
