use super::*;
use crate::chat::Who;
use nixa_voice::{Ignored, Via, VoiceEvent};
use std::time::Duration;
use tokio::sync::broadcast::Receiver;
use tokio_util::sync::CancellationToken;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn heard(text: &str, stop: bool) -> VoiceEvent {
    VoiceEvent::Heard { text: text.into(), via: Via::Mic, ms: 40, audio_ms: 900, named: true, stop }
}

fn drain(rx: &mut Receiver<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    while let Ok(v) = rx.try_recv() {
        out.push(v);
    }
    out
}

fn kinds(list: &[Value]) -> Vec<String> {
    list.iter().filter_map(|v| v.get("type").and_then(Value::as_str).map(str::to_string)).collect()
}

#[tokio::test]
async fn heard_text_goes_into_the_chat() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    on_event(&app, heard("Nixa, what is the time?", false));
    assert!(lock(&app.talk.gather).pending());
    crate::talk::flow::tick(&app, crate::util::now_ms() + 3000);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let items = lock(&app.chat).list().to_vec();
    let user = items.iter().find(|i| i.who == Who::User).expect("user line");
    assert_eq!(user.text, "Nixa, what is the time?");
    assert!(!user.typed && !user.quiet);
    let out = drain(&mut rx);
    assert!(out.iter().any(|v| v["type"] == "heard" && v["text"] == "Nixa, what is the time?"), "{out:?}");
    assert_eq!(lock(&app.voice.live).heard, 1);
}

#[tokio::test]
async fn a_stop_word_stops_the_brain() {
    let (_dir, app) = app();
    let token = CancellationToken::new();
    lock(&app.brain).cancel = Some(token.clone());
    let mut rx = app.out.subscribe();
    on_event(&app, heard("stop", true));
    assert!(token.is_cancelled());
    assert!(kinds(&drain(&mut rx)).contains(&"stopped".to_string()));
    assert!(lock(&app.chat).list().iter().any(|i| i.who == Who::User && i.text == "stop"));
}

#[tokio::test]
async fn a_too_short_press_says_nothing_heard() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    on_event(&app, VoiceEvent::Ignored { text: String::new(), why: Ignored::TooShort });
    assert!(drain(&mut rx).iter().any(|v| v["text"] == events::NOTHING_HEARD));
}

#[tokio::test]
async fn level_and_talking_go_to_the_socket() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    on_event(&app, VoiceEvent::Level { level: 0.333, talking: true });
    on_event(&app, VoiceEvent::SpeechStart);
    on_event(&app, VoiceEvent::SpeechStart);
    on_event(&app, VoiceEvent::SpeechEnd);
    let out = drain(&mut rx);
    assert!(out.contains(&json!({"type": "level", "mic": 0.6, "talking": true})), "{out:?}");
    let talking: Vec<&Value> = out.iter().filter(|v| v["type"] == "talking").collect();
    assert_eq!(talking, vec![&json!({"type": "talking", "who": "user", "on": true}), &json!({"type": "talking", "who": "user", "on": false})]);
}

#[tokio::test]
async fn commands_save_and_tell_the_chat() {
    let (dir, app) = app();
    let mut rx = app.out.subscribe();
    assert!(ws(&app, "set_listen", &json!({"mode": "always"})));
    assert!(!ws(&app, "say", &json!({"text": "hi"})));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let saved = settings::load(&dir.path().join(settings::FILE));
    let mut on = saved.clone();
    on.mic_on = true;
    settings::save(&dir.path().join("start.json"), &on).unwrap();
    assert!(!settings::load_at_start(&dir.path().join("start.json")).mic_on);
    assert_eq!(saved.listen, Listen::Always);
    assert!(lock(&app.chat).list().iter().any(|i| i.who == Who::Info && i.text.starts_with("Mic: always on.")));
    let out = drain(&mut rx);
    let state = out.iter().find(|v| v["type"] == "voice").expect("voice state");
    assert_eq!(state["voice"]["settings"]["listen"], "always");
    assert_eq!(state["voice"]["settings"]["micOn"], false);
}

#[tokio::test]
async fn mic_on_without_models_says_so_and_opens_nothing() {
    let (_dir, app) = app();
    lock(&app.voice.settings).listen = Listen::Always;
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    *lock(&app.voice.events) = Some(tx);
    *lock(&app.voice.models) = PathBuf::from("/nonexistent-nixa-models");
    handle(&app, Command::Mic(true)).await;
    let error = lock(&app.voice.live).error.clone().expect("an error");
    assert!(error.contains("nonexistent-nixa-models") || error.to_lowercase().contains("vad"), "{error}");
    assert!(lock(&app.chat).list().iter().any(|i| i.text.starts_with("The mic did not start")));
    assert!(!lock(&app.voice.live).listening);
    handle(&app, Command::Mic(false)).await;
    assert_eq!(lock(&app.voice.live).error, None);
}

#[tokio::test]
async fn stop_and_interrupt_messages_stop_the_brain_turn() {
    for kind in ["stop", "interrupt"] {
        let (_dir, app) = app();
        let token = CancellationToken::new();
        lock(&app.brain).cancel = Some(token.clone());
        assert!(ws(&app, kind, &json!({})));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(token.is_cancelled(), "{kind} left the brain turn running");
        assert!(lock(&app.brain).cancel.is_none());
        assert!(!lock(&app.voice.live).nixa_talking);
    }
}

#[tokio::test]
async fn hold_without_a_listener_asks_for_the_mic() {
    let (_dir, app) = app();
    handle(&app, Command::HoldStart).await;
    assert!(lock(&app.chat).list().iter().any(|i| i.text.starts_with("Turn on the mic")));
}

#[tokio::test]
async fn voice_off_speaks_nothing() {
    let (_dir, app) = app();
    let mut rx = app.out.subscribe();
    say(&app, "This must stay silent.", None);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let out = drain(&mut rx);
    assert!(!out.iter().any(|v| v["type"] == "talking" || v["type"] == "said"), "{out:?}");
    assert!(!lock(&app.voice.live).nixa_talking);
}

#[tokio::test]
async fn the_view_shows_settings_and_live_state() {
    let (_dir, app) = app();
    let v = view(&app);
    assert_eq!(v["settings"]["voiceOn"], false);
    assert_eq!(v["voice"], "Pippa");
    assert_eq!(v["live"]["listening"], false);
}
