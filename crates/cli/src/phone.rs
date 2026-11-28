use crate::args::usage;
use crate::ctx::{self, Ctx};
use crate::help;
use crate::http;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::io::Write;

const KEY_FILE: &str = "door-key";
const QR_MARGIN: usize = 2;
const CAMERA_LOG_SHOWN: usize = 5;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &[], false)?;
    if p.flag(&["-h", "--help"]) || p.pos.first().is_some_and(|s| s == "help") {
        out.write_all(help::PHONE.as_bytes())?;
        return Ok(0);
    }
    let yes = p.flag(&["--yes", "-y"]);
    let clear = p.flag(&["--clear"]);
    let front = p.flag(&["--front"]);
    p.finish(3, false)?;
    let words: Vec<&str> = p.pos.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] => {
            out.write_all(help::PHONE.as_bytes())?;
            Ok(2)
        }
        ["status"] => status(&ctx, out),
        ["key"] => key(&ctx, yes, out),
        ["camera"] | ["camera", "status"] => camera_status(&ctx, out),
        ["camera", "log"] if !clear => camera_log(&ctx, out),
        ["camera", "photo", question] => photo(&ctx, question, front, out),
        ["camera", "photo"] => Err(usage("use: nixa phone camera photo \"QUESTION\" [--front]")),
        _ => {
            let body = if clear && words == ["camera", "log"] { json!({"type": "camera_log_clear"}) } else { body_of(&words)? };
            let reply = http::window_call("POST", "/phone", Some(&body))?;
            let line = change_line(&body, &reply);
            if ctx.json {
                writeln!(out, "{}", safe(&reply))?;
            } else {
                writeln!(out, "{line}")?;
            }
            Ok(0)
        }
    }
}

fn on_off(word: &str, name: &str) -> Result<bool> {
    match word {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(usage(format!("use: nixa phone {name}on|off"))),
    }
}

pub fn body_of(words: &[&str]) -> Result<Value> {
    match words {
        [w @ ("on" | "off")] => Ok(json!({"type": "set_phone", "on": on_off(w, "")?})),
        ["mobile", w] => Ok(json!({"type": "set_mobile", "on": on_off(w, "mobile ")?})),
        ["remove", id] if !id.trim().is_empty() => Ok(json!({"type": "phone_remove", "id": id.trim()})),
        ["remove"] => Err(usage("use: nixa phone remove ID (see nixa phone status)")),
        ["camera", ..] => Err(usage("use: nixa phone camera status | photo \"QUESTION\" [--front] | log [--clear]")),
        other => Err(usage(format!("unknown command nixa phone {}", other.join(" ")))),
    }
}

pub fn safe(reply: &Value) -> Value {
    let mut v = reply.clone();
    if let Some(phone) = v.get_mut("phone").and_then(Value::as_object_mut) {
        phone.remove("pair_qr");
    }
    v
}

fn door_word(phone: &Value) -> String {
    if phone["on"].as_bool() != Some(true) {
        return "off".into();
    }
    match phone["door_url"].as_str() {
        Some(url) => format!("on at {url}"),
        None => "on".into(),
    }
}

fn voice_word(reply: &Value) -> &'static str {
    if reply["mobile"].as_bool() == Some(true) { "the phone (the Mac is silent)" } else { "the Mac" }
}

pub fn change_line(body: &Value, reply: &Value) -> String {
    match body["type"].as_str() {
        Some("set_phone") => format!("Phone door is {}.", door_word(&reply["phone"])),
        Some("set_mobile") => format!("Nixa speaks on {}.", voice_word(reply)),
        Some("camera_log_clear") => "Camera log cleared.".into(),
        Some("phone_remove") => format!("Removed phone {}. {} phones paired.", body["id"].as_str().unwrap_or(""), reply["phone"]["phones"].as_array().map_or(0, Vec::len)),
        _ => String::new(),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn ago(at: u64, now: u64) -> String {
    let min = now.saturating_sub(at) / 60_000;
    match min {
        0 => "just now".into(),
        m if m < 120 => format!("{m} min ago"),
        m if m < 48 * 60 => format!("{} h ago", m / 60),
        m => format!("{} days ago", m / (24 * 60)),
    }
}

pub fn row_note(row: &Value, now: u64) -> String {
    if row["connected"].as_bool() == Some(true) {
        return if row["voice"].as_bool() == Some(true) { "connected now, sound on".into() } else { "connected now, sound off".into() };
    }
    match (row["last_seen"].as_u64(), row["paired_at"].as_u64()) {
        (Some(at), _) => format!("last seen {}", ago(at, now)),
        (None, Some(at)) if at > 0 => format!("paired {}", ago(at, now)),
        _ => "paired".into(),
    }
}

pub fn status_lines(reply: &Value, now: u64) -> Vec<String> {
    let phone = &reply["phone"];
    let mut out = vec![format!("phone door: {}", door_word(phone)), format!("Nixa speaks on: {}", voice_word(reply))];
    if let Some(error) = phone["error"].as_str() {
        out.push(format!("problem: {error}"));
    }
    let rows = phone["phones"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        out.push("phones: none paired. Run: nixa phone key --yes".into());
    } else {
        out.push(format!("phones: {}", rows.len()));
        for row in &rows {
            out.push(format!("  {}  {}  {}", row["id"].as_str().unwrap_or(""), row["name"].as_str().unwrap_or(""), row_note(row, now)));
        }
    }
    out
}

fn privacy_note(camera: &Value) -> &'static str {
    if camera["privacy"].as_bool() == Some(true) { " Only a vision model on this Mac is used." } else { "" }
}

pub fn photo_input(question: &str, front: bool) -> Value {
    json!({"question": question, "camera": if front { "front" } else { "rear" }})
}

fn photo(ctx: &Ctx, question: &str, front: bool, out: &mut dyn Write) -> Result<i32> {
    let reply = http::post_within("/agent/tool", &json!({"name": "nixa_phone_look", "args": photo_input(question, front)}), std::time::Duration::from_secs(60))?;
    let ok = reply.get("ok").and_then(Value::as_bool) == Some(true);
    if ctx.json {
        writeln!(out, "{reply}")?;
    } else {
        writeln!(out, "{}", reply.get("text").or_else(|| reply.get("reason")).and_then(Value::as_str).unwrap_or(""))?;
    }
    Ok(if ok { 0 } else { 1 })
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn log_line(line: &Value, now: u64) -> String {
    let phone = line["phone"].as_str().unwrap_or("");
    let camera = line["camera"].as_str().filter(|c| !c.is_empty()).unwrap_or("rear");
    let frames = line["frames"].as_u64().unwrap_or(0);
    let place = if line["cloud"].as_bool() == Some(true) { ", cloud" } else { "" };
    format!(
        "  {}  {phone}  {camera} camera  {frames} {}  {} on {}{place}\n    Q: {}\n    A: {}",
        ago(line["at"].as_u64().unwrap_or(0), now),
        if frames == 1 { "frame" } else { "frames" },
        line["model"].as_str().unwrap_or(""),
        line["brain"].as_str().unwrap_or(""),
        one_line(line["question"].as_str().unwrap_or("")),
        one_line(line["answer"].as_str().unwrap_or("")),
    )
}

pub fn camera_lines(reply: &Value, now: u64, log_max: usize) -> Vec<String> {
    let camera = &reply["camera"];
    if !camera.is_object() {
        return vec!["camera looks: this brain has no camera looks.".into()];
    }
    let on = |v: &Value| if v.as_bool() == Some(true) { "on" } else { "off" };
    let mut out = vec![format!("privacy: {}{}", on(&camera["privacy"]), privacy_note(camera))];
    let model = camera["model"].as_str().unwrap_or("");
    out.push(if camera["model_ok"].as_bool() == Some(false) { format!("vision model: none. {model}") } else { format!("vision model: {model}") });
    let log = camera["log"].as_array().cloned().unwrap_or_default();
    if log.is_empty() {
        out.push("camera log: empty".into());
    } else {
        let shown = log.len().min(log_max);
        out.push(if shown < log.len() { format!("camera log: {} looks, newest {shown} (all: nixa phone camera log)", log.len()) } else { format!("camera log: {} looks", log.len()) });
        out.extend(log.iter().take(shown).map(|l| log_line(l, now)));
    }
    out
}

fn camera_status(ctx: &Ctx, out: &mut dyn Write) -> Result<i32> {
    let reply = http::window_call("GET", "/phone", None)?;
    if ctx.json {
        writeln!(out, "{}", safe(&reply))?;
        return Ok(0);
    }
    for line in camera_lines(&reply, now_ms(), CAMERA_LOG_SHOWN) {
        writeln!(out, "{line}")?;
    }
    Ok(0)
}

fn camera_log(ctx: &Ctx, out: &mut dyn Write) -> Result<i32> {
    let reply = http::window_call("GET", "/phone", None)?;
    if ctx.json {
        writeln!(out, "{}", json!({"log": reply["camera"]["log"]}))?;
        return Ok(0);
    }
    let log = reply["camera"]["log"].as_array().cloned().unwrap_or_default();
    if log.is_empty() {
        writeln!(out, "camera log: empty")?;
    }
    let now = now_ms();
    for line in &log {
        writeln!(out, "{}", log_line(line, now))?;
    }
    Ok(0)
}

fn status(ctx: &Ctx, out: &mut dyn Write) -> Result<i32> {
    let reply = http::window_call("GET", "/phone", None)?;
    if ctx.json {
        writeln!(out, "{}", safe(&reply))?;
        return Ok(0);
    }
    for line in status_lines(&reply, now_ms()) {
        writeln!(out, "{line}")?;
    }
    Ok(0)
}

pub fn pair_link(phone: &Value, key: &str) -> Option<String> {
    let url = phone["door_url"].as_str()?;
    let pin = phone["root_pin"].as_str()?;
    let base = url.trim_end_matches('/');
    Some(format!("{base}/k/{key}#r={pin}"))
}

pub fn is_key(text: &str) -> bool {
    text.len() == 43 && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn qr_text(rows: &[String]) -> Vec<String> {
    let size = rows.len();
    if size == 0 || rows.iter().any(|r| r.len() != size || r.bytes().any(|b| b != b'0' && b != b'1')) {
        return Vec::new();
    }
    let width = size + QR_MARGIN * 2;
    let light = |y: usize, x: usize| -> bool {
        if y < QR_MARGIN || x < QR_MARGIN || y >= size + QR_MARGIN || x >= size + QR_MARGIN {
            return true;
        }
        rows[y - QR_MARGIN].as_bytes()[x - QR_MARGIN] == b'0'
    };
    (0..width)
        .step_by(2)
        .map(|y| {
            (0..width)
                .map(|x| match (light(y, x), y + 1 >= width || light(y + 1, x)) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                })
                .collect()
        })
        .collect()
}

fn read_key() -> Option<String> {
    let text = std::fs::read_to_string(nixa_protocol::paths::data_dir().join(KEY_FILE)).ok()?;
    let key = text.trim();
    is_key(key).then(|| key.to_string())
}

fn key(ctx: &Ctx, yes: bool, out: &mut dyn Write) -> Result<i32> {
    if !yes {
        bail!("A new pairing key removes every paired phone. Each phone must pair again.\nRun: nixa phone key --yes");
    }
    let reply = http::window_call("POST", "/phone", Some(&json!({"type": "phone_new_key"})))?;
    let phone = &reply["phone"];
    let key = read_key();
    let link = key.as_deref().and_then(|k| pair_link(phone, k));
    let rows: Vec<String> = phone["pair_qr"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    if ctx.json {
        writeln!(out, "{}", json!({"ok": true, "secret": true, "key": key, "pair_link": link, "pair_qr": rows, "phone": safe(&reply)["phone"]}))?;
        return Ok(0);
    }
    writeln!(out, "New pairing key made. All old phones are removed.")?;
    writeln!(out, "This is a secret. Anyone with it can pair a phone. Do not share it or paste it in a chat.")?;
    match (&link, &key) {
        (Some(link), _) => writeln!(out, "pair link: {link}")?,
        (None, Some(key)) => writeln!(out, "key: {key}\nTurn the phone door on (nixa phone on) to get a pair link.")?,
        (None, None) => writeln!(out, "The key could not be read here. Open the Phone panel in the Nixa window to pair.")?,
    }
    let qr = qr_text(&rows);
    if !qr.is_empty() {
        writeln!(out, "Scan this with the Nixa phone app:")?;
        for line in qr {
            writeln!(out, "{line}")?;
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_match_the_brain() {
        assert_eq!(body_of(&["on"]).unwrap(), json!({"type": "set_phone", "on": true}));
        assert_eq!(body_of(&["off"]).unwrap(), json!({"type": "set_phone", "on": false}));
        assert_eq!(body_of(&["mobile", "on"]).unwrap(), json!({"type": "set_mobile", "on": true}));
        assert_eq!(body_of(&["remove", " ab12cd34 "]).unwrap(), json!({"type": "phone_remove", "id": "ab12cd34"}));
        assert!(body_of(&["mobile", "maybe"]).is_err());
        assert!(body_of(&["remove"]).is_err());
        assert!(body_of(&["fly"]).is_err());
    }

    #[test]
    fn status_hides_the_pair_code() {
        let reply = json!({"ok": true, "mobile": false, "phone": {"on": true, "door_url": "https://10.0.0.2:7430/", "pair_qr": ["1"], "phones": [
            {"id": "p1", "name": "Pixel", "paired_at": 0, "connected": true, "voice": false},
            {"id": "p2", "name": "Tab", "paired_at": 0, "last_seen": 0, "connected": false}
        ]}});
        assert!(safe(&reply)["phone"].get("pair_qr").is_none());
        let text = status_lines(&reply, 5 * 60_000).join("\n");
        assert!(text.contains("phone door: on at https://10.0.0.2:7430/"));
        assert!(text.contains("Nixa speaks on: the Mac"));
        assert!(text.contains("p1  Pixel  connected now, sound off"));
        assert!(text.contains("p2  Tab  last seen 5 min ago"));
        assert_eq!(change_line(&json!({"type": "set_mobile"}), &json!({"mobile": true})), "Nixa speaks on the phone (the Mac is silent).");
    }

    #[test]
    fn camera_bodies_match_the_brain() {
        assert!(body_of(&["camera", "allow", "on"]).is_err());
        assert!(body_of(&["camera", "look"]).is_err());
        assert_eq!(photo_input("is the door shut?", false), json!({"question": "is the door shut?", "camera": "rear"}));
        assert_eq!(photo_input("who is there?", true)["camera"], "front");
        assert_eq!(change_line(&json!({"type": "camera_log_clear"}), &json!({})), "Camera log cleared.");
    }

    #[test]
    fn camera_status_prints_the_log() {
        let now = 60 * 60_000;
        let reply = json!({"camera": {"privacy": false, "model": "llava on Ollama", "model_ok": true,
            "log": [{"at": now - 2 * 60_000, "phone": "Pixel", "camera": "front", "question": "Who is\nthere?", "answer": "A person sits.", "model": "llava", "brain": "Ollama", "frames": 1, "cloud": false}]}});
        let text = camera_lines(&reply, now, 5).join("\n");
        assert!(text.contains("privacy: off"));
        assert!(text.contains("vision model: llava on Ollama"));
        assert!(text.contains("2 min ago  Pixel  front camera  1 frame  llava on Ollama"));
        assert!(text.contains("Q: Who is there?"));
        let none = json!({"camera": {"model": "No local vision model is set up.", "model_ok": false, "log": []}});
        let text = camera_lines(&none, now, 5).join("\n");
        assert!(text.contains("vision model: none. No local vision model is set up."));
        assert!(text.contains("camera log: empty"));
    }

    #[test]
    fn pair_link_and_qr() {
        let key = "a".repeat(43);
        assert!(is_key(&key));
        assert!(!is_key("short"));
        let phone = json!({"door_url": "https://10.0.0.2:7430/", "root_pin": "pin"});
        assert_eq!(pair_link(&phone, &key).unwrap(), format!("https://10.0.0.2:7430/k/{key}#r=pin"));
        assert!(pair_link(&json!({}), &key).is_none());
        let qr = qr_text(&["10".into(), "01".into()]);
        assert_eq!(qr.len(), 3);
        assert!(qr.iter().all(|l| l.chars().count() == 6));
        assert!(qr_text(&["1x".into(), "00".into()]).is_empty());
    }
}
