use super::chips::{self, PHONE_ID, VOICE_ID};
use super::feed::set;
use super::info::{self, Door, Phone, PhoneState};
use super::qr;
use super::{current, feed, hub};
use gpui::TestAppContext;
use nixa_ui::Theme;
use serde_json::json;

fn rows(n: usize) -> Vec<String> {
    (0..n)
        .map(|y| {
            (0..n)
                .map(|x| if (x + y) % 2 == 0 { '1' } else { '0' })
                .collect()
        })
        .collect()
}

fn door_json() -> serde_json::Value {
    json!({
        "on": true,
        "address": "192.168.1.20",
        "pairQr": rows(5),
        "rootUrl": "http://192.168.1.20:7431/nixa-root.crt",
        "rootQr": rows(3),
        "rootSha256": "ab".repeat(32),
        "phones": [
            {"id": "p1", "name": "Pixel", "pairedAt": 1000, "lastSeen": 2000, "connected": true, "voice": true},
            {"id": "p2", "name": "Android tablet", "paired_at": 3000, "connected": false},
            {"name": "no id"}
        ]
    })
}

fn theme() -> Theme {
    nixa_ui::theme::default_theme()
}

#[test]
fn the_door_reads_both_key_styles() {
    let door = info::parse_door(&door_json()).unwrap();
    assert!(door.on);
    assert_eq!(door.address.as_deref(), Some("192.168.1.20"));
    assert_eq!(door.pair_qr.len(), 5);
    assert_eq!(door.root_qr.len(), 3);
    assert_eq!(
        door.root_url.as_deref(),
        Some("http://192.168.1.20:7431/nixa-root.crt")
    );
    assert_eq!(door.root_sha256.as_deref().map(str::len), Some(64));
    assert_eq!(door.phones.len(), 2);
    let p1 = &door.phones[0];
    assert_eq!(
        (p1.paired_at, p1.last_seen, p1.connected, p1.voice),
        (1000, Some(2000), true, true)
    );
    assert_eq!(door.phones[1].paired_at, 3000);
    let snake = info::parse_door(
        &json!({"on": false, "setup_url": "u", "setup_qr": rows(2), "pair_qr": ["11", "1"]}),
    )
    .unwrap();
    assert_eq!(snake.root_url.as_deref(), Some("u"));
    assert_eq!(snake.root_qr.len(), 2);
    assert!(snake.pair_qr.is_empty());
    let old_name = info::parse_door(&json!({"openQr": rows(3)})).unwrap();
    assert_eq!(old_name.pair_qr.len(), 3);
    assert!(info::parse_door(&json!("x")).is_none());
}

#[test]
fn bad_codes_are_not_drawn() {
    assert!(info::qr_rows(Some(&json!(["10", "0x"]))).is_empty());
    assert!(info::qr_rows(Some(&json!(["10", 5]))).is_empty());
    assert!(info::qr_rows(Some(&json!([]))).is_empty());
    assert!(info::qr_rows(None).is_empty());
    assert_eq!(info::qr_rows(Some(&json!(["10", "01"]))).len(), 2);
}

#[test]
fn messages_take_the_door_and_the_voice_place() {
    let mut state = PhoneState::default();
    assert!(!info::take(
        &mut state,
        &json!({"type": "chat", "phone": door_json()})
    ));
    assert!(info::take(
        &mut state,
        &json!({"type": "phone", "phone": door_json()})
    ));
    assert!(state.door_on());
    assert!(!info::take(
        &mut state,
        &json!({"type": "phone", "phone": door_json()})
    ));
    assert!(info::take(
        &mut state,
        &json!({"type": "state", "mobile": true})
    ));
    assert!(state.mobile);
    assert!(state.door.is_some());
    assert!(info::take(
        &mut state,
        &json!({"type": "snapshot", "snapshot": {"mobile": false, "phone": {"on": false}}})
    ));
    assert!(!state.mobile);
    assert!(!state.door_on());
    assert!(!info::take(&mut state, &json!({"type": "snapshot"})));
    assert!(!info::take(
        &mut state,
        &json!({"type": "state", "listen": "wake"})
    ));
    let reply =
        info::parse_reply(&json!({"ok": true, "phone": door_json(), "mobile": true})).unwrap();
    assert!(reply.mobile && reply.door.is_some());
    assert!(info::parse_reply(&json!({"ok": true})).is_none());
    let only_voice = info::parse_reply(&json!({"mobile": false})).unwrap();
    assert!(only_voice.door.is_none());
}

#[test]
fn messages_to_the_brain() {
    assert_eq!(
        info::set_door(true),
        json!({"type": "set_phone", "on": true})
    );
    assert_eq!(
        info::set_voice(false),
        json!({"type": "set_mobile", "on": false})
    );
    assert_eq!(info::new_key(), json!({"type": "phone_new_key"}));
    assert_eq!(
        info::remove("p1"),
        json!({"type": "phone_remove", "id": "p1"})
    );
}

#[test]
fn row_notes_and_labels() {
    let clock = |at: u64| format!("t{at}");
    let mut p = Phone {
        id: "p".into(),
        name: "P".into(),
        paired_at: 5,
        connected: true,
        voice: true,
        ..Phone::default()
    };
    assert_eq!(info::row_note(&p, clock), "connected now, sound on");
    p.voice = false;
    assert_eq!(info::row_note(&p, clock), "connected now, sound off");
    p.connected = false;
    assert_eq!(info::row_note(&p, clock), "paired t5");
    p.last_seen = Some(9);
    assert_eq!(info::row_note(&p, clock), "last seen t9");
    assert_eq!(info::clock(0, 0), "00:00");
    assert_eq!(info::clock(90 * 60_000, 330), "07:00");
    assert_eq!(info::clock(30 * 60_000, -60), "23:30");
    let mut state = PhoneState::default();
    assert!(info::footer(&state, "Nova").starts_with("Voice: Mac."));
    state.mobile = true;
    assert!(info::footer(&state, "Nova").contains("last 2 minutes"));
}

#[test]
fn the_phone_chip_follows_the_door() {
    let t = theme();
    let mut state = PhoneState::default();
    let off = chips::phone_chip(&state, "Nova", &t);
    assert_eq!(
        (off.id.as_ref(), off.label.as_str(), off.color),
        (PHONE_ID, "Phone", t.dim)
    );
    assert!(off.tip.contains("open the phone door"));
    state.door = Some(Door {
        on: true,
        ..Door::default()
    });
    let open = chips::phone_chip(&state, "Nova", &t);
    assert_eq!((open.label.as_str(), open.color), ("Phone", t.blue));
    assert!(open.tip.contains("pairing code for the Nova app"));
    state.door = info::parse_door(&door_json());
    let live = chips::phone_chip(&state, "Nova", &t);
    assert_eq!((live.label.as_str(), live.color), ("Phone · 1", t.green));
    assert!(live.tip.starts_with("1 phone is connected in the Nova app"));
}

#[test]
fn the_voice_switch_shows_mac_or_phone() {
    let t = theme();
    let mut state = PhoneState { mobile: true, ..PhoneState::default() };
    let quiet = chips::voice_spec(&state, "Nova", &t, VOICE_ID);
    assert_eq!(
        (quiet.id.as_ref(), quiet.label.as_str(), quiet.color),
        (VOICE_ID, "Voice: Phone", t.yellow)
    );
    assert!(quiet.tip.contains("no phone has sound on"));
    state.door = info::parse_door(&door_json());
    let loud = chips::voice_spec(&state, "Nova", &t, VOICE_ID);
    assert_eq!(loud.color, t.green);
    state.mobile = false;
    let mac = chips::voice_spec(&state, "Nova", &t, VOICE_ID);
    assert_eq!((mac.label.as_str(), mac.color), ("Voice: Mac", t.dim));
}

#[test]
fn qr_runs_join_dark_cells() {
    let code = vec![
        "1101".to_string(),
        "0000".to_string(),
        "0111".to_string(),
        "1111".to_string(),
    ];
    assert_eq!(
        qr::runs(&code),
        vec![(0, 0, 2), (3, 0, 1), (1, 2, 3), (0, 3, 4)]
    );
    assert_eq!(qr::cell(29, 220.), 7.);
    assert_eq!(qr::cell(200, 220.), 2.);
}

#[gpui::test]
fn the_hub_feeds_and_sets(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert_eq!(current(cx), PhoneState::default());
        assert!(feed(
            &json!({"type": "phone", "phone": door_json(), "mobile": true}),
            cx
        ));
        assert!(current(cx).mobile);
        assert!(!feed(&json!({"type": "chat"}), cx));
        set(PhoneState::default(), cx);
        assert!(!current(cx).mobile);
        assert!(hub(cx).read(cx).known);
    });
}

#[test]
fn the_real_door_gives_the_certificate_code_and_fingerprint() {
    let url = "http://192.168.1.20:7431/nixa-root.crt";
    let door = info::parse_door(&json!({
        "on": true,
        "address": "192.168.1.20",
        "door_url": "https://192.168.1.20:7430/",
        "pair_qr": rows(25),
        "root_url": url,
        "root_sha256": "0a1b".repeat(16),
        "phones": [{"id": "p1", "name": "Pixel", "paired_at": 1000, "connected": true, "voice": true, "checks": {}}]
    }))
    .unwrap();
    assert_eq!(door.root_url.as_deref(), Some(url));
    assert_eq!(door.root_qr, qr::make(url));
    assert!(door.root_qr.len() >= 21);
    assert!(door.root_qr.iter().all(|r| r.len() == door.root_qr.len()));
    assert_eq!(door.phones.len(), 1);
    let lines = info::fingerprint(door.root_sha256.as_deref().unwrap());
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("0A:1B:0A:1B"));
    assert_eq!(lines[0].split(':').count(), 16);
    let bare = info::parse_door(&json!({"on": true, "address": "192.168.1.20"})).unwrap();
    assert!(bare.root_url.is_none() && bare.root_qr.is_empty() && bare.root_sha256.is_none());
}

#[test]
fn the_camera_object_is_read_and_a_missing_one_hides_the_parts() {
    let reply = json!({"ok": true, "mobile": false, "phone": door_json(), "camera": {
        "privacy": true, "model": "llava on Local Ollama",
        "log": [
            {"at": 5_000, "phone": "Pixel", "camera": "front", "question": "Is the child dancing?", "answer": "The child is dancing.", "model": "llava", "brain": "Local", "frames": 3, "cloud": false},
            {"at": 6_000, "phone": "Pixel", "question": "Who is there?", "answer": "One person.", "model": "llava"},
            {"phone": "no time"}
        ]
    }});
    let state = info::parse_reply(&reply).unwrap();
    let camera = state.camera.unwrap();
    assert!(camera.privacy);
    assert_eq!(camera.model, "llava on Local Ollama");
    assert_eq!(camera.log.len(), 2);
    assert_eq!((camera.log[0].camera.as_str(), camera.log[1].camera.as_str()), ("front", "rear"));
    assert_eq!(info::parse_reply(&json!({"ok": true, "phone": door_json()})).unwrap().camera, None);
    let mut state = PhoneState::default();
    assert!(info::take(&mut state, &json!({"type": "phone", "camera": {"privacy": true}})));
    assert!(state.camera.is_some_and(|c| c.privacy));
    assert_eq!(info::camera_log_clear(), json!({"type": "camera_log_clear"}));
}

