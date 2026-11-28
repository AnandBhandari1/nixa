use super::*;
use crate::app::App;
use nixa_store::tasks::{Actor, AuthorKind, Task, TaskInput, TaskStatus, add_task, claim_task, get_task};
use std::time::Duration;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn actor(terminal: &str) -> Actor {
    let mut a = Actor::new(AuthorKind::Agent, "codex");
    a.terminal = terminal.into();
    a
}

fn new_task(app: &App, maker: &str) -> Task {
    let conn = nixa_store::open(&app.db_file).unwrap();
    let input = TaskInput { title: "Fix the login page".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() };
    add_task(&conn, &input, &actor(maker)).unwrap()
}

async fn next_flow(rx: &mut broadcast::Receiver<Value>, to: &str) -> Option<Value> {
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < until {
        if let Ok(Ok(v)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
            && v["type"] == FLOW_KIND
            && v["to"] == to
        {
            return Some(v);
        }
    }
    None
}

#[test]
fn same_flows_are_sent_once_and_nixa_echoes_are_dropped() {
    let mut r = Recent::default();
    assert!(pass(&mut r, "m1", "m2", 1000, false));
    assert!(!pass(&mut r, "m1", "m2", 1500, false));
    assert!(!pass(&mut r, NIXA, "m2", 1600, true));
    assert!(pass(&mut r, NIXA, "m3", 1600, true));
    assert!(pass(&mut r, "m2", "m1", 1700, false));
    assert!(pass(&mut r, "m1", "m2", 1000 + SAME_MS, false));
    assert!(pass(&mut r, NIXA, "m2", 1000 + 2 * SAME_MS + 1, true));
    assert!(!pass(&mut r, "m1", "m1", 9000, false));
    assert!(!pass(&mut r, "", "m1", 9000, false));
    assert!(!pass(&mut r, "m1", "", 9000, false));
    assert!(pass(&mut r, "m1", "m2", 50_000, false));
    assert_eq!(r.pairs.len(), 1);
}

#[tokio::test]
async fn a_flow_goes_out_to_every_window() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    assert!(flow(&app, " m1 ", "m2"));
    assert!(!flow(&app, "m1", "m2"));
    let v = rx.recv().await.unwrap();
    assert_eq!(v["type"], "universe_flow");
    assert_eq!((v["from"].as_str(), v["to"].as_str()), (Some("m1"), Some("m2")));
    assert!(v["at"].as_u64().unwrap() > 0);
    assert!(rx.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_brain_send_shows_nixa_to_the_agent() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    tokio::spawn(run(app.clone()));
    tokio::time::sleep(Duration::from_millis(100)).await;
    sent("m7q", "Please fix the login page.\nMore lines here.");
    let v = next_flow(&mut rx, "m7q").await.expect("flow from nixa");
    assert_eq!(v["from"], NIXA);
}

#[test]
fn task_hand_offs_name_the_giver() {
    let (_dir, app) = app();
    let mut task = new_task(&app, "m1");
    assert_eq!(task.created_terminal, "m1");
    task.terminal_id = String::new();
    assert_eq!(handoff(None, &task), None);
    task.terminal_id = "m2".into();
    assert_eq!(handoff(None, &task), Some(("m1".into(), "m2".into())));
    assert_eq!(handoff(Some(""), &task), Some(("m1".into(), "m2".into())));
    assert_eq!(handoff(Some("m2"), &task), None);
    assert_eq!(handoff(Some("m3"), &task), Some(("m3".into(), "m2".into())));
    task.created_terminal = "m2".into();
    assert_eq!(handoff(None, &task), Some((NIXA.into(), "m2".into())));
    task.created_terminal = String::new();
    assert_eq!(handoff(None, &task), Some((NIXA.into(), "m2".into())));
    task.terminal_id = nixa_store::tasks::starting_holder(task.id);
    assert_eq!(handoff(Some("m3"), &task), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_claimed_task_shows_a_flow_from_its_maker() {
    let (_dir, app) = app();
    let task = new_task(&app, "m5");
    let mut rx = app.out.subscribe();
    tokio::spawn(run(app.clone()));
    tokio::time::sleep(Duration::from_millis(100)).await;
    app.send_out(json!({"type": crate::queue::CHANGED, "why": "test", "id": task.id}));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let conn = nixa_store::open(&app.db_file).unwrap();
    claim_task(&conn, task.id, &actor("m6"), "").unwrap();
    assert_eq!(get_task(&conn, task.id).unwrap().unwrap().terminal_id, "m6");
    app.send_out(json!({"type": crate::queue::CHANGED, "why": "test", "id": task.id}));
    let v = next_flow(&mut rx, "m6").await.expect("hand-off flow");
    assert_eq!(v["from"], "m5");
    app.send_out(json!({"type": crate::queue::CHANGED, "why": "test", "id": null}));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let again = std::iter::from_fn(|| rx.try_recv().ok()).filter(|v| v["type"] == FLOW_KIND).count();
    assert_eq!(again, 0);
}

#[test]
fn a_meteor_plays_once_and_only_when_fresh() {
    let mut s = Sound::default();
    assert!(!pick(&mut s, Direction::Incoming, 1000, 1050, false));
    assert!(!pick(&mut s, Direction::Incoming, 1000, 1050, true));
    assert!(pick(&mut s, Direction::Outgoing, 2000, 2050, true));
    assert!(!pick(&mut s, Direction::Outgoing, 2000, 2060, true));
    assert!(!pick(&mut s, Direction::Incoming, 2100, 2110, true));
    let later = 2050 + METEOR_MS + 500;
    assert!(!pick(&mut s, Direction::Incoming, 100, later, true));
    assert!(!pick(&mut s, Direction::Incoming, later + 1000, later, true));
    assert!(pick(&mut s, Direction::Incoming, later, later + 10, true));
    assert_eq!(s.played, 2);
}

#[test]
fn meteor_sound_needs_voice_on_and_a_calm_moment() {
    assert!(!allowed(&Quiet::default()));
    let calm = Quiet { nixa_on: true, voice_on: true, ..Quiet::default() };
    assert!(allowed(&calm));
    for loud in [
        Quiet { voice_on: false, ..calm },
        Quiet { nixa_on: false, ..calm },
        Quiet { speaking: true, ..calm },
        Quiet { user_talking: true, ..calm },
        Quiet { holding: true, ..calm },
        Quiet { phone: true, ..calm },
        Quiet { quiet_time: true, ..calm },
    ] {
        assert!(!allowed(&loud), "{loud:?}");
    }
}

#[tokio::test]
async fn with_voice_off_the_meteor_message_is_silent() {
    let (_dir, app) = app();
    assert!(!crate::voice::settings(&app).voice_on);
    let at = now_ms();
    assert!(ws(&app, METEOR_KIND, &json!({"type": METEOR_KIND, "direction": "incoming", "at": at})));
    assert!(ws(&app, METEOR_KIND, &json!({"type": METEOR_KIND, "stop": true})));
    assert!(ws(&app, METEOR_KIND, &json!({"type": METEOR_KIND, "direction": "sideways", "at": at})));
    assert!(!ws(&app, "universe_flow", &json!({})));
    assert_eq!(lock(&app.universe.sound).played, 0);
    assert!(!app.dir.join(sound::SOUNDS_DIR).exists());
}

#[test]
fn the_meteor_sound_is_a_short_quiet_wave() {
    for direction in [Direction::Incoming, Direction::Outgoing] {
        let wav = wave(direction);
        let samples = (sound::RATE as u64 * METEOR_MS / 1000) as usize;
        assert_eq!(wav.len(), 44 + samples * 2);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize, wav.len() - 8);
        assert_eq!(wave(direction), wav);
        let peak = wav[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        assert!(peak > 50 && peak < i16::MAX as u16, "{direction:?} peak {peak}");
        assert_eq!(i16::from_le_bytes([wav[44], wav[45]]), 0);
    }
    assert_ne!(wave(Direction::Incoming), wave(Direction::Outgoing));
}

#[test]
fn sound_files_are_written_once_in_the_data_folder() {
    let dir = tempfile::tempdir().unwrap();
    let file = sound::sound_file(dir.path(), Direction::Outgoing).unwrap();
    assert_eq!(file, dir.path().join("sounds/meteor-outgoing.wav"));
    assert_eq!(std::fs::read(&file).unwrap(), wave(Direction::Outgoing));
    let when = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    sound::sound_file(dir.path(), Direction::Outgoing).unwrap();
    assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), when);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(dir.path().join("sounds")).unwrap().permissions().mode() & 0o777, 0o700);
}

#[test]
fn flow_notes_name_two_terminals() {
    let (a, b) = parse_note(&json!({"terminal": "m1", "to": " m2a "})).unwrap();
    assert_eq!((a.to_string(), b.to_string()), ("m1".into(), "m2a".into()));
    assert!(parse_note(&json!({"to": "m2"})).unwrap_err().contains("NIXA_TERM_ID"));
    assert!(parse_note(&json!({"terminal": "m1"})).unwrap_err().contains("to is needed"));
    assert!(parse_note(&json!({"terminal": "nixa", "to": "m2"})).unwrap_err().contains("not a terminal id"));
    assert!(parse_note(&json!({"terminal": "m1", "to": "m1"})).unwrap_err().contains("itself"));
}

#[tokio::test]
async fn screen_flows_have_the_shape_the_window_reads() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    screen_flow(&app, NIXA, "m3", "send", None);
    screen_flow(&app, "", "m3", "send", None);
    crate::watch::run_actions(&app, vec![crate::deep::Action::Notify { line: "x".into(), loud: false }]).await;
    let mut flows = Vec::new();
    while let Ok(value) = rx.try_recv() {
        if value["type"] == SCREEN_FLOW_KIND {
            flows.push(value);
        }
    }
    assert_eq!(flows, vec![json!({"type": "flow", "from": "nixa", "to": "m3", "kind": "send"})]);
    assert_eq!(screen_flow_event(DEEP, NIXA, "deep", None), json!({"type": "flow", "from": "deep", "to": "nixa", "kind": "deep"}));
    assert_eq!(
        screen_flow_event(NIXA, "m3", "send", Some("\n  Fix   the login  \nmore")),
        json!({"type": "flow", "from": "nixa", "to": "m3", "kind": "send", "text": "Fix the login"})
    );
    assert_eq!(screen_flow_event(NIXA, "m3", "send", Some("   ")), json!({"type": "flow", "from": "nixa", "to": "m3", "kind": "send"}));
}

#[test]
fn office_lines_pick_the_speaker_and_a_short_text() {
    let long = "a ".repeat(200);
    let (who, text) = office_line(&json!({"who": "m3", "text": long})).unwrap();
    assert_eq!(who, "m3");
    assert_eq!(text.chars().count(), LINE_MAX);
    assert!(office_line(&json!({"who": "", "text": "hi"})).is_none());
    assert!(office_line(&json!({"who": "m3"})).is_none());
    assert_eq!(office_ctx(NIXA).agent.as_deref(), Some("nixa"));
    assert_eq!(office_ctx("m3").terminal.as_deref(), Some("m3"));
}

#[tokio::test]
async fn office_say_is_taken_by_the_universe() {
    let (_dir, app) = app();
    assert!(ws(&app, OFFICE_SAY_KIND, &json!({"who": "m3", "text": "On it!"})));
}

#[test]
fn voice_log_keeps_the_speaking_line_and_counts() {
    let mut log = voice_log::VoiceLog::default();
    voice_log::note(&mut log, "m3", "On it!", voice_log::START, voice_log::OFFICE, 10);
    let on = voice_log::status(&log, true, 110);
    assert_eq!(on["speaking"]["who"], "m3");
    assert_eq!(on["speaking"]["age_ms"], 100);
    voice_log::note(&mut log, "m3", "On it!", voice_log::END, voice_log::OFFICE, 900);
    voice_log::note(&mut log, "m4", "Hi.", voice_log::SKIP, voice_log::OFFICE, 950);
    let off = voice_log::status(&log, true, 1000);
    assert!(off["speaking"].is_null());
    assert_eq!(off["last"]["who"], "m4");
    assert_eq!((off["started"].as_u64(), off["ended"].as_u64(), off["skipped"].as_u64()), (Some(1), Some(1), Some(1)));
    assert_eq!(
        voice_log::event("nixa", "Hi.", voice_log::START, voice_log::CHAT),
        json!({"type": "office_voice", "who": "nixa", "text": "Hi.", "state": "start", "src": "chat"})
    );
}

#[tokio::test]
async fn office_say_with_voice_off_sends_skip() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    lock(&app.voice.settings).voice_on = false;
    assert!(ws(&app, OFFICE_SAY_KIND, &json!({"who": "m3", "text": "On it!"})));
    let got = rx.try_recv().expect("skip event");
    assert_eq!(got["type"], "office_voice");
    assert_eq!(got["state"], "skip");
}

#[test]
fn office_audio_count_covers_only_the_parts_that_play() {
    let live = std::sync::atomic::AtomicUsize::new(0);
    super::office_part(&live, true);
    assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 1);
    super::office_part(&live, false);
    super::office_part(&live, false);
    assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 0);
    super::office_part(&live, true);
    super::office_done(&live);
    assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 0);
}
