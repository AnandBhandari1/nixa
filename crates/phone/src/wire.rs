use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const TALK_RATE: usize = 16_000;
pub const TALK_MAX_MS: usize = 120_000;
pub const PEEK_MS: usize = 1_000;
pub const PEEK_BYTES: usize = TALK_RATE * 2 * PEEK_MS / 1000;
pub const TALK_MAX_BYTES: usize = TALK_RATE * 2 * (TALK_MAX_MS + 1000) / 1000;
pub const TEXT_MAX: usize = 2000;
pub const DETAIL_MAX: usize = 200;
pub const CHECK_NAMES: [&str; 6] = ["secure", "connected", "live", "rtt", "mic", "sound"];
pub const RELAYED: [&str; 4] = ["snapshot", "level", "heard", "say"];
pub const MAX_MESSAGE: usize = 64 * 1024;
pub const FRAME_TAG: &[u8; 4] = b"NXF1";
pub const FRAME_HEADER: usize = 17;
pub const FRAME_MAX: usize = 60_000;
pub const LOOK_FRAMES: u8 = 1;
pub const LOOK_SPAN_MS: u64 = 0;
pub const LOOK_MAX_SIDE: u32 = 1024;
pub const LOOK_FAILS: [&str; 3] = ["camera_busy", "denied", "error"];
const JPEG_START: [u8; 2] = [0xFF, 0xD8];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lens {
    #[default]
    Rear,
    Front,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub index: u8,
    pub at: u64,
    pub jpeg: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PhoneCommand {
    Text(String),
    SetMobile(bool),
    Stop,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PhoneEvent {
    Changed,
    Voice { phone: String, ready: bool },
    TalkStart { phone: String, auto: bool },
    TalkPeek { phone: String, pcm: Vec<i16> },
    Talk { phone: String, pcm: Vec<i16>, started_at: u64, auto: bool },
    TalkCancel { phone: String },
    Command { phone: String, command: PhoneCommand },
    Caps { phone: String, camera: bool },
    LookFrames { phone: String, id: u32, frames: Vec<Frame> },
    LookFail { phone: String, id: u32, why: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayedHow {
    Played,
    Gone,
    Stopped,
    Timeout,
}

#[derive(Clone, Debug, PartialEq)]
pub enum In {
    Here(Value),
    TextOnly(bool),
    Voice(bool),
    Command(PhoneCommand),
    TalkStart(bool),
    TalkEnd,
    TalkCancel,
    Played(u32),
    Check { name: String, ok: bool, detail: String },
    Caps { camera: bool },
    LookEnd { id: u32, frames: u8 },
    LookFail { id: u32, why: String },
}

fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub fn parse_in(text: &str) -> Option<In> {
    let msg: Value = serde_json::from_str(text).ok()?;
    let kind = msg.get("type")?.as_str()?;
    let flag = |key: &str| msg.get(key).and_then(Value::as_bool);
    let id = || msg.get("id").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    Some(match kind {
        "here" => In::Here(msg.get("t").filter(|t| t.is_number()).cloned().unwrap_or(json!(0))),
        "text_only" => In::TextOnly(flag("on") == Some(true)),
        "voice_ready" => In::Voice(true),
        "voice_off" => In::Voice(false),
        "text" => {
            let text = cut(msg.get("text")?.as_str()?.trim(), TEXT_MAX);
            if text.is_empty() {
                return None;
            }
            In::Command(PhoneCommand::Text(text))
        }
        "set_mobile" => In::Command(PhoneCommand::SetMobile(flag("on")?)),
        "stop" => In::Command(PhoneCommand::Stop),
        "talk_start" => In::TalkStart(flag("auto") == Some(true)),
        "talk_end" => In::TalkEnd,
        "talk_cancel" => In::TalkCancel,
        "played" => In::Played(u32::try_from(msg.get("line")?.as_u64()?).ok()?),
        "check" => {
            let name = msg.get("name")?.as_str()?;
            if !CHECK_NAMES.contains(&name) {
                return None;
            }
            let detail = msg.get("detail").and_then(Value::as_str).map(|d| cut(d, DETAIL_MAX)).unwrap_or_default();
            In::Check { name: name.to_string(), ok: flag("ok")?, detail }
        }
        "caps" => In::Caps { camera: flag("camera")? },
        "look_end" => In::LookEnd { id: id()?, frames: u8::try_from(msg.get("frames")?.as_u64()?).ok()? },
        "look_fail" => {
            let why = msg.get("why")?.as_str()?;
            let why = if LOOK_FAILS.contains(&why) { why } else { "error" };
            In::LookFail { id: id()?, why: why.to_string() }
        }
        _ => return None,
    })
}

pub fn look_request(id: u32, lens: Lens) -> String {
    json!({"type": "look_request", "id": id, "camera": lens, "frames": LOOK_FRAMES, "span_ms": LOOK_SPAN_MS, "max_side": LOOK_MAX_SIDE, "max_bytes": FRAME_MAX}).to_string()
}

pub fn look_cancel(id: u32) -> String {
    json!({"type": "look_cancel", "id": id}).to_string()
}

pub fn is_frame(bytes: &[u8]) -> bool {
    bytes.starts_with(FRAME_TAG)
}

pub fn parse_frame(bytes: &[u8]) -> Option<(u32, Frame)> {
    if !is_frame(bytes) || bytes.len() > FRAME_MAX || bytes.len() < FRAME_HEADER + JPEG_START.len() {
        return None;
    }
    let id = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let index = bytes[8];
    let at = u64::from_le_bytes(bytes[9..FRAME_HEADER].try_into().ok()?);
    let jpeg = &bytes[FRAME_HEADER..];
    if index >= LOOK_FRAMES || !jpeg.starts_with(&JPEG_START) {
        return None;
    }
    Some((id, Frame { index, at, jpeg: jpeg.to_vec() }))
}

pub fn here(t: &Value) -> String {
    json!({"type": "here", "t": t}).to_string()
}

pub fn audio_start(line: u32, rate: u32) -> String {
    json!({"type": "audio_start", "line": line, "rate": rate}).to_string()
}

pub fn audio_end(line: u32, parts: u32) -> String {
    json!({"type": "audio_end", "line": line, "parts": parts}).to_string()
}

pub fn audio_stop(line: u32) -> String {
    json!({"type": "audio_stop", "line": line}).to_string()
}

pub fn snapshot(snapshot: &Value) -> String {
    let mut snap = snapshot.clone();
    if let Some(map) = snap.as_object_mut() {
        map.remove("phone");
    }
    json!({"type": "snapshot", "snapshot": snap}).to_string()
}

pub fn relayed(msg: &Value) -> Option<String> {
    let kind = msg.get("type")?.as_str()?;
    if !RELAYED.contains(&kind) {
        return None;
    }
    if kind == "snapshot" {
        return Some(snapshot(msg.get("snapshot").unwrap_or(&Value::Null)));
    }
    Some(msg.to_string())
}

pub fn audio_frame(line: u32, part: u32, pcm: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(8 + pcm.len());
    frame.extend_from_slice(&line.to_le_bytes());
    frame.extend_from_slice(&part.to_le_bytes());
    frame.extend_from_slice(pcm);
    frame
}

pub fn to_i16(bytes: &[u8]) -> Vec<i16> {
    bytes.as_chunks::<2>().0.iter().map(|pair| i16::from_le_bytes(*pair)).collect()
}

pub fn phone_name(agent: &str) -> &'static str {
    if agent.contains("iPhone") {
        "iPhone"
    } else if agent.contains("iPad") {
        "iPad"
    } else if agent.contains("Android") {
        "Android phone"
    } else {
        "Phone"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_messages_are_parsed_strictly() {
        assert_eq!(parse_in(r#"{"type":"here","t":12.5}"#), Some(In::Here(json!(12.5))));
        assert_eq!(parse_in(r#"{"type":"here","t":"x"}"#), Some(In::Here(json!(0))));
        assert_eq!(parse_in(r#"{"type":"text_only","on":true}"#), Some(In::TextOnly(true)));
        assert_eq!(parse_in(r#"{"type":"text_only","on":"yes"}"#), Some(In::TextOnly(false)));
        assert_eq!(parse_in(r#"{"type":"voice_ready"}"#), Some(In::Voice(true)));
        assert_eq!(parse_in(r#"{"type":"text","text":"  hi  "}"#), Some(In::Command(PhoneCommand::Text("hi".into()))));
        assert_eq!(parse_in(r#"{"type":"text","text":"   "}"#), None);
        assert_eq!(parse_in(r#"{"type":"set_mobile","on":false}"#), Some(In::Command(PhoneCommand::SetMobile(false))));
        assert_eq!(parse_in(r#"{"type":"set_mobile"}"#), None);
        assert_eq!(parse_in(r#"{"type":"stop"}"#), Some(In::Command(PhoneCommand::Stop)));
        assert_eq!(parse_in(r#"{"type":"talk_start","auto":true}"#), Some(In::TalkStart(true)));
        assert_eq!(parse_in(r#"{"type":"played","line":3}"#), Some(In::Played(3)));
        assert_eq!(parse_in(r#"{"type":"played","line":-1}"#), None);
        assert_eq!(parse_in(r#"{"type":"check","name":"mic","ok":true}"#), Some(In::Check { name: "mic".into(), ok: true, detail: String::new() }));
        assert_eq!(parse_in(r#"{"type":"check","name":"disk","ok":true}"#), None);
        assert_eq!(parse_in(r#"{"type":"clean"}"#), None);
        assert_eq!(parse_in("not json"), None);
        let long = format!(r#"{{"type":"text","text":"{}"}}"#, "é".repeat(3000));
        match parse_in(&long) {
            Some(In::Command(PhoneCommand::Text(text))) => assert_eq!(text.chars().count(), TEXT_MAX),
            other => panic!("{other:?}"),
        }
    }

    fn frame_bytes(id: u32, index: u8, at: u64, jpeg: &[u8]) -> Vec<u8> {
        let mut out = FRAME_TAG.to_vec();
        out.extend_from_slice(&id.to_le_bytes());
        out.push(index);
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(jpeg);
        out
    }

    #[test]
    fn camera_messages_are_parsed_strictly() {
        assert_eq!(parse_in(r#"{"type":"caps","camera":true}"#), Some(In::Caps { camera: true }));
        assert_eq!(parse_in(r#"{"type":"caps"}"#), None);
        assert_eq!(parse_in(r#"{"type":"camera_session","on":true,"until":5000}"#), None);
        assert_eq!(parse_in(r#"{"type":"look_end","id":7,"frames":3}"#), Some(In::LookEnd { id: 7, frames: 3 }));
        assert_eq!(parse_in(r#"{"type":"look_end","id":7,"frames":300}"#), None);
        assert_eq!(parse_in(r#"{"type":"look_end","id":5000000000,"frames":3}"#), None);
        assert_eq!(parse_in(r#"{"type":"look_fail","id":7,"why":"denied"}"#), Some(In::LookFail { id: 7, why: "denied".into() }));
        assert_eq!(parse_in(r#"{"type":"look_fail","id":7,"why":"<script>"}"#), Some(In::LookFail { id: 7, why: "error".into() }));
        assert_eq!(parse_in(r#"{"type":"look_fail","id":7}"#), None);
    }

    #[test]
    fn mac_camera_messages_have_the_contract_shape() {
        let req: Value = serde_json::from_str(&look_request(9, Lens::default())).unwrap();
        assert_eq!(req, json!({"type": "look_request", "id": 9, "camera": "rear", "frames": 1, "span_ms": 0, "max_side": 1024, "max_bytes": 60000}));
        let front: Value = serde_json::from_str(&look_request(9, Lens::Front)).unwrap();
        assert_eq!(front["camera"], "front");
        assert_eq!(serde_json::from_str::<Value>(&look_cancel(9)).unwrap(), json!({"type": "look_cancel", "id": 9}));
    }

    #[test]
    fn frames_need_the_tag_a_good_index_a_jpeg_start_and_the_size_limit() {
        let good = frame_bytes(0x0102_0304, 0, 99, &[0xFF, 0xD8, 1]);
        assert_eq!(&good[..9], &[b'N', b'X', b'F', b'1', 4, 3, 2, 1, 0]);
        assert_eq!(parse_frame(&good), Some((0x0102_0304, Frame { index: 0, at: 99, jpeg: vec![0xFF, 0xD8, 1] })));
        assert_eq!(parse_frame(&frame_bytes(1, 1, 0, &[0xFF, 0xD8])), None);
        assert_eq!(parse_frame(&frame_bytes(1, 0, 0, &[0x89, b'P'])), None);
        assert_eq!(parse_frame(&good[..FRAME_HEADER]), None);
        let mut big = vec![0xFF, 0xD8];
        big.resize(FRAME_MAX - FRAME_HEADER + 1, 0);
        assert_eq!(parse_frame(&frame_bytes(1, 0, 0, &big)), None);
        big.pop();
        assert!(parse_frame(&frame_bytes(1, 0, 0, &big)).is_some());
        assert!(!is_frame(&[1, 0, 0, 0]));
    }

    #[test]
    fn only_four_kinds_are_relayed_and_phone_info_is_removed() {
        let snap = json!({"type": "snapshot", "snapshot": {"state": "idle", "phone": {"on": true}}});
        let out: Value = serde_json::from_str(&relayed(&snap).unwrap()).unwrap();
        assert_eq!(out, json!({"type": "snapshot", "snapshot": {"state": "idle"}}));
        assert!(relayed(&json!({"type": "level", "mic": 0.2, "talking": false})).is_some());
        assert!(relayed(&json!({"type": "heard", "text": "hi"})).is_some());
        assert!(relayed(&json!({"type": "say", "text": "hi"})).is_some());
        assert!(relayed(&json!({"type": "chat", "text": "secret"})).is_none());
        assert!(relayed(&json!({"text": "x"})).is_none());
    }

    #[test]
    fn audio_frames_have_a_little_endian_header() {
        let frame = audio_frame(2, 7, &[1, 2, 3, 4]);
        assert_eq!(frame, vec![2, 0, 0, 0, 7, 0, 0, 0, 1, 2, 3, 4]);
        assert_eq!(to_i16(&[1, 0, 255, 255, 9]), vec![1, -1]);
        assert_eq!(PEEK_BYTES, 32_000);
    }

    #[test]
    fn phone_names_come_from_the_browser() {
        assert_eq!(phone_name("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0)"), "iPhone");
        assert_eq!(phone_name("Mozilla/5.0 (Linux; Android 15)"), "Android phone");
        assert_eq!(phone_name(""), "Phone");
    }
}
