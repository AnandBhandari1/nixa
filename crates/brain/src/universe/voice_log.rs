use serde_json::{Value, json};

pub const KIND: &str = "office_voice";
pub const START: &str = "start";
pub const END: &str = "end";
pub const SKIP: &str = "skip";
pub const OFFICE: &str = "office";
pub const CHAT: &str = "chat";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spoken {
    pub who: String,
    pub text: String,
    pub src: String,
    pub state: String,
    pub at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VoiceLog {
    pub now: Option<Spoken>,
    pub last: Option<Spoken>,
    pub started: u64,
    pub ended: u64,
    pub skipped: u64,
}

pub fn event(who: &str, text: &str, state: &str, src: &str) -> Value {
    json!({"type": KIND, "who": who, "text": text, "state": state, "src": src})
}

pub fn note(log: &mut VoiceLog, who: &str, text: &str, state: &str, src: &str, at: u64) {
    let spoken = Spoken { who: who.into(), text: text.into(), src: src.into(), state: state.into(), at };
    match state {
        START => {
            log.started += 1;
            log.now = Some(spoken);
        }
        END => {
            log.ended += 1;
            if log.now.as_ref().is_some_and(|n| n.who == who && n.src == src) {
                log.now = None;
            }
            log.last = Some(spoken);
        }
        _ => {
            log.skipped += 1;
            log.last = Some(spoken);
        }
    }
}

fn spoken_json(s: &Spoken, now: u64) -> Value {
    json!({"who": s.who, "text": s.text, "src": s.src, "state": s.state, "at": s.at, "age_ms": now.saturating_sub(s.at)})
}

pub fn status(log: &VoiceLog, voice_on: bool, now: u64) -> Value {
    json!({
        "voice_on": voice_on,
        "speaking": log.now.as_ref().map(|s| spoken_json(s, now)),
        "last": log.last.as_ref().map(|s| spoken_json(s, now)),
        "started": log.started,
        "ended": log.ended,
        "skipped": log.skipped,
    })
}
