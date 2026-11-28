use super::qr;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Phone {
    pub id: String,
    pub name: String,
    pub paired_at: u64,
    pub last_seen: Option<u64>,
    pub connected: bool,
    pub voice: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Door {
    pub on: bool,
    pub address: Option<String>,
    pub pair_qr: Vec<String>,
    pub root_url: Option<String>,
    pub root_qr: Vec<String>,
    pub root_sha256: Option<String>,
    pub error: Option<String>,
    pub phones: Vec<Phone>,
}


#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CameraLook {
    pub at: u64,
    pub phone: String,
    pub camera: String,
    pub question: String,
    pub answer: String,
    pub model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Camera {
    pub privacy: bool,
    pub model: String,
    pub log: Vec<CameraLook>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhoneState {
    pub door: Option<Door>,
    pub mobile: bool,
    pub camera: Option<Camera>,
}

impl PhoneState {
    pub fn phones(&self) -> &[Phone] {
        self.door
            .as_ref()
            .map(|d| d.phones.as_slice())
            .unwrap_or_default()
    }

    pub fn live(&self) -> usize {
        self.phones().iter().filter(|p| p.connected).count()
    }

    pub fn sound(&self) -> bool {
        self.phones().iter().any(|p| p.voice)
    }

    pub fn door_on(&self) -> bool {
        self.door.as_ref().is_some_and(|d| d.on)
    }
}

fn field<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| v.get(*k)).filter(|x| !x.is_null())
}

fn word(v: &Value, keys: &[&str]) -> Option<String> {
    field(v, keys)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn flag(v: &Value, keys: &[&str]) -> Option<bool> {
    field(v, keys).and_then(Value::as_bool)
}

fn time(v: &Value, keys: &[&str]) -> Option<u64> {
    let x = field(v, keys)?;
    x.as_u64()
        .or_else(|| x.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
        .filter(|t| *t > 0)
}

pub fn qr_rows(v: Option<&Value>) -> Vec<String> {
    let Some(list) = v.and_then(Value::as_array) else {
        return Vec::new();
    };
    let rows: Vec<String> = list
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let n = rows.len();
    let square = n > 0
        && rows.len() == list.len()
        && rows
            .iter()
            .all(|r| r.len() == n && r.bytes().all(|b| b == b'0' || b == b'1'));
    if square { rows } else { Vec::new() }
}

pub fn parse_phone(v: &Value) -> Option<Phone> {
    let id = word(v, &["id"])?;
    Some(Phone {
        name: word(v, &["name"]).unwrap_or_else(|| "Phone".into()),
        paired_at: time(v, &["pairedAt", "paired_at"]).unwrap_or_default(),
        last_seen: time(v, &["lastSeen", "last_seen"]),
        connected: flag(v, &["connected"]).unwrap_or(false),
        voice: flag(v, &["voice", "sound"]).unwrap_or(false),
        id,
    })
}

pub fn parse_door(v: &Value) -> Option<Door> {
    if !v.is_object() {
        return None;
    }
    let phones = field(v, &["phones"])
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(parse_phone).collect())
        .unwrap_or_default();
    let root_url = word(v, &["rootUrl", "root_url", "setupUrl", "setup_url"]);
    let root_qr = match (
        qr_rows(field(v, &["rootQr", "root_qr", "setupQr", "setup_qr"])),
        &root_url,
    ) {
        (rows, Some(url)) if rows.is_empty() => qr::make(url),
        (rows, _) => rows,
    };
    Some(Door {
        on: flag(v, &["on"]).unwrap_or(false),
        address: word(v, &["address"]),
        pair_qr: qr_rows(field(v, &["pairQr", "pair_qr", "openQr", "open_qr"])),
        root_url,
        root_qr,
        root_sha256: word(v, &["rootSha256", "root_sha256"]),
        error: word(v, &["error"]),
        phones,
    })
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string()
}

pub fn parse_camera(v: &Value) -> Option<Camera> {
    if !v.is_object() {
        return None;
    }
    let list = |key: &str| v.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let log = list("log")
        .iter()
        .filter_map(|l| {
            Some(CameraLook {
                at: time(l, &["at"])?,
                phone: text(l, "phone"),
                camera: if text(l, "camera") == "front" { "front".into() } else { "rear".into() },
                question: text(l, "question"),
                answer: text(l, "answer"),
                model: text(l, "model"),
            })
        })
        .collect();
    Some(Camera {
        privacy: flag(v, &["privacy"]).unwrap_or(false),
        model: text(v, "model"),
        log,
    })
}

pub fn parse_reply(v: &Value) -> Option<PhoneState> {
    if !v.is_object() {
        return None;
    }
    let door = field(v, &["phone", "door"]);
    let mobile = flag(v, &["mobile", "voice_on_phone", "voiceOnPhone"]);
    if door.is_none() && mobile.is_none() {
        return None;
    }
    Some(PhoneState {
        door: door.and_then(parse_door),
        mobile: mobile.unwrap_or(false),
        camera: field(v, &["camera"]).and_then(parse_camera),
    })
}

pub fn take(state: &mut PhoneState, msg: &Value) -> bool {
    let kind = msg.get("type").and_then(Value::as_str).unwrap_or_default();
    let body = match kind {
        "phone" | "state" | "phone_reply" => msg,
        "snapshot" => match msg.get("snapshot") {
            Some(s) => s,
            None => return false,
        },
        _ => return false,
    };
    let door = field(body, &["phone", "door"]).and_then(parse_door);
    let mobile = flag(body, &["mobile", "voice_on_phone", "voiceOnPhone"]);
    let camera = field(body, &["camera"]).and_then(parse_camera);
    if door.is_none() && mobile.is_none() && camera.is_none() {
        return false;
    }
    let mut next = state.clone();
    if camera.is_some() {
        next.camera = camera;
    }
    if let Some(door) = door {
        next.door = Some(door);
    }
    if let Some(mobile) = mobile {
        next.mobile = mobile;
    }
    if next == *state {
        return false;
    }
    *state = next;
    true
}

pub fn set_door(on: bool) -> Value {
    json!({"type": "set_phone", "on": on})
}

pub fn set_voice(on: bool) -> Value {
    json!({"type": "set_mobile", "on": on})
}

pub fn new_key() -> Value {
    json!({"type": "phone_new_key"})
}

pub fn camera_log_clear() -> Value {
    json!({"type": "camera_log_clear"})
}

pub fn remove(id: &str) -> Value {
    json!({"type": "phone_remove", "id": id})
}

pub fn clock(at: u64, offset_min: i32) -> String {
    let minutes = (at as i64).div_euclid(60_000) + i64::from(offset_min);
    let minutes = minutes.rem_euclid(24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

pub fn row_note(phone: &Phone, clock: impl Fn(u64) -> String) -> String {
    if phone.connected {
        return if phone.voice {
            "connected now, sound on".into()
        } else {
            "connected now, sound off".into()
        };
    }
    match phone.last_seen {
        Some(at) => format!("last seen {}", clock(at)),
        None if phone.paired_at > 0 => format!("paired {}", clock(phone.paired_at)),
        None => "paired".into(),
    }
}

pub fn footer(state: &PhoneState, assistant: &str) -> String {
    if state.mobile {
        "Voice: Phone. The Mac is silent. Lines missed while the phone is locked play on the phone later (last 2 minutes).".into()
    } else {
        format!(
            "Voice: Mac. The phone app shows {assistant}. Switch Voice to Phone to hear {assistant} on the phone."
        )
    }
}

pub fn fingerprint(hex: &str) -> Vec<String> {
    let pairs: Vec<String> = hex
        .as_bytes()
        .chunks(2)
        .map(|p| String::from_utf8_lossy(p).to_uppercase())
        .collect();
    pairs.chunks(16).map(|line| line.join(":")).collect()
}
