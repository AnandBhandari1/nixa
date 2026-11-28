use crate::app::App;
use crate::talk::hold::Pass;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spoken {
    pub text: String,
    pub unfinished: bool,
    pub last: bool,
    pub mac_only: bool,
}

pub fn spoken_line(value: &Value) -> Option<Spoken> {
    let text = |v: &Value| v.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    let mac_only = value.get("mac_only").and_then(Value::as_bool) == Some(true);
    match value.get("type").and_then(Value::as_str)? {
        "reply" => {
            let reply = value.get("reply")?;
            let unfinished = reply.get("heard").and_then(Value::as_str) == Some("unfinished");
            if reply.get("speak").and_then(Value::as_bool) != Some(true) {
                return Some(Spoken { text: String::new(), unfinished, last: true, mac_only });
            }
            Some(Spoken { text: text(reply).unwrap_or_default(), unfinished, last: true, mac_only })
        }
        "commentary" => text(value).map(|text| Spoken { text, unfinished: false, last: false, mac_only }),
        _ => None,
    }
}

pub const DEEP_REPLY_MS: u64 = 10 * 60_000;

pub fn mark_deep_reply(app: &App) {
    crate::app::lock(&app.talk.speaker).deep_reply_at = crate::util::now_ms();
}

pub fn reply_kind(app: &App, last: bool, now: u64) -> &'static str {
    let mut speaker = crate::app::lock(&app.talk.speaker);
    let deep = speaker.deep_reply_at > 0 && now.saturating_sub(speaker.deep_reply_at) < DEEP_REPLY_MS;
    if last {
        speaker.deep_reply_at = 0;
    }
    if deep { "deep" } else { "chat" }
}

pub async fn speak_replies(app: Arc<App>) {
    let mut rx = app.out.subscribe();
    loop {
        match rx.recv().await {
            Ok(value) => {
                if let Some(line) = spoken_line(&value) {
                    let pass = if line.last { Pass::Last } else { Pass::Answer };
                    let kind = reply_kind(&app, line.last, crate::util::now_ms());
                    if !line.text.is_empty() {
                        let options = crate::talk::speech::SayOptions { mac_only: line.mac_only, ..crate::talk::speech::SayOptions::nixa(kind) };
                        crate::talk::speech::speak_later_as(&app, &line.text, options, pass);
                    } else if line.last {
                        crate::talk::hold::answered(&app);
                    }
                }
            }
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        }
    }
}

pub fn say(app: &Arc<App>, text: &str, open_ms: Option<u64>) {
    if open_ms.is_none() && crate::talk::hold::catch(app, text, &crate::talk::speech::SayOptions::nixa("chat")) {
        return;
    }
    let settings = super::settings(app);
    if !settings.voice_on {
        return;
    }
    let voice = crate::voice_map::pick(app, &crate::voice_map::Ctx::nixa("chat"));
    super::set_nixa_talking(app, true);
    let done = nixa_voice::speak(text, &voice);
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let result = done.recv().unwrap_or(nixa_voice::SayResult { interrupted: true, ms: 0 });
        if !nixa_voice::is_speaking() {
            super::set_nixa_talking(&app, false);
        }
        app.send_out(json!({"type": "said", "interrupted": result.interrupted, "ms": result.ms}));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_that_speak_are_spoken() {
        let reply = json!({"type": "reply", "reply": {"speak": true, "text": " Done. ", "heard": "me"}});
        assert_eq!(spoken_line(&reply), Some(Spoken { text: "Done.".into(), unfinished: false, last: true, mac_only: false }));
        let quiet = json!({"type": "reply", "reply": {"speak": false, "text": "x", "heard": "me"}});
        assert_eq!(spoken_line(&quiet), Some(Spoken { text: String::new(), unfinished: false, last: true, mac_only: false }));
        let wait = json!({"type": "reply", "reply": {"speak": false, "text": "", "heard": "unfinished"}});
        assert_eq!(spoken_line(&wait), Some(Spoken { text: String::new(), unfinished: true, last: true, mac_only: false }));
        let said = spoken_line(&json!({"type": "commentary", "text": "Looking now."})).unwrap();
        assert_eq!((said.text.as_str(), said.last), ("Looking now.", false));
        assert_eq!(spoken_line(&json!({"type": "delta", "text": "x"})), None);
        assert_eq!(spoken_line(&json!({"type": "chat"})), None);
    }
}
