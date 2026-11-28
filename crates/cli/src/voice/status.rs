use super::switches::{now_ms, quiet_left};
use crate::ctx::{self, Ctx};
use crate::help;
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;

fn on(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}

fn word(v: &Value) -> &'static str {
    if on(v) { "on" } else { "off" }
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

pub fn view(switches: &Value, voice: &Value, talk: Option<&Value>, devices: Option<&Value>, now: u64) -> Value {
    let s = &voice["settings"];
    let listen = if matches!(s["listen"].as_str(), Some("always") | Some("wake")) { "always" } else { "hotkey" };
    let quiet = talk.and_then(|t| quiet_left(t, now));
    json!({
        "nixa": on(&switches["nixa"]),
        "state": switches["state"],
        "mic": on(&s["micOn"]),
        "listen": listen,
        "listening": on(&voice["live"]["holding"]),
        "speech": on(&s["voiceOn"]),
        "voice": voice["voice"],
        "speaking": on(&voice["speaking"]),
        "agentVoice": on(&switches["agentVoice"]),
        "gitVoice": on(&switches["gitVoice"]),
        "headphones": on(&switches["headphones"]),
        "quietKnown": talk.is_some(),
        "quietMinutesLeft": quiet,
        "quietUntil": talk.and_then(|t| t["presence"]["quietUntil"].as_u64()).filter(|_| quiet.is_some()),
        "micDevice": s["micDevice"],
        "speakerDevice": s["speakerDevice"],
        "mics": devices.map(|d| names(&d["mics"])),
        "speakers": devices.map(|d| names(&d["speakers"])),
    })
}

pub fn lines(v: &Value) -> Vec<String> {
    let state = v["state"].as_str().map(|s| format!(" ({s})")).unwrap_or_default();
    let voice = v["voice"].as_str().filter(|s| !s.is_empty()).map(|s| format!(", voice {s}")).unwrap_or_default();
    let quiet = match (v["quietKnown"].as_bool(), v["quietMinutesLeft"].as_u64()) {
        (Some(false), _) => "unknown (the window token is missing)".to_string(),
        (_, Some(min)) => format!("on, {min} more min"),
        _ => "off".into(),
    };
    let device = |key: &str, empty: &str| v[key].as_str().filter(|s| !s.is_empty()).unwrap_or(empty).to_string();
    let list = |key: &str| match v[key].as_array() {
        None => "unknown".to_string(),
        Some(l) if l.is_empty() => "none found".to_string(),
        Some(l) => l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
    };
    vec![
        format!("Nixa:          {}{state}", word(&v["nixa"])),
        format!("mic:           {}", word(&v["mic"])),
        format!("listen mode:   {}", listen_line(v)),
        format!("Nixa speech:   {}{voice}", word(&v["speech"])),
        format!("agent voice:   {}", word(&v["agentVoice"])),
        format!("git voice:     {}", word(&v["gitVoice"])),
        format!("headphones:    {}", word(&v["headphones"])),
        format!("quiet:         {quiet}"),
        format!("mic in use:    {}", device("micDevice", "Mac default")),
        format!("speaker:       {}", device("speakerDevice", "Mac output")),
        format!("mics:          {}", list("mics")),
        format!("speakers:      {}", list("speakers")),
    ]
}

fn listen_line(v: &Value) -> String {
    match (v["listen"].as_str(), on(&v["listening"])) {
        (Some("always"), _) => "always on".into(),
        (_, true) => "Right Command hotkey (listening now)".into(),
        _ => "Right Command hotkey".into(),
    }
}

pub fn status(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &[], false)?;
    if p.flag(&["-h", "--help"]) {
        out.write_all(help::VOICE.as_bytes())?;
        return Ok(0);
    }
    p.finish(0, false)?;
    let switches = http::get("/switches")?;
    let voice = http::get("/voice")?;
    let talk = http::window_call("GET", "/talk", None).ok();
    let devices = http::get("/voice/devices").ok();
    let v = view(&switches["switches"], &voice["voice"], talk.as_ref().map(|t| &t["talk"]), devices.as_ref(), now_ms());
    if ctx.json {
        writeln!(out, "{v}")?;
        return Ok(0);
    }
    for line in lines(&v) {
        writeln!(out, "{line}")?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_view_of_every_switch() {
        let switches = json!({"nixa": true, "state": "idle", "agentVoice": true, "gitVoice": false, "headphones": true});
        let voice = json!({"settings": {"micOn": false, "voiceOn": true, "listen": "hotkey", "micDevice": null, "speakerDevice": "Desk"}, "live": {"holding": true}, "voice": "Ava", "speaking": false});
        let talk = json!({"presence": {"quietUntil": 30 * 60_000}});
        let devices = json!({"mics": ["A"], "speakers": []});
        let v = view(&switches, &voice, Some(&talk), Some(&devices), 0);
        assert_eq!(v["listen"], "hotkey");
        assert_eq!(v["quietMinutesLeft"], 30);
        let text = lines(&v).join("\n");
        assert!(text.contains("Nixa:          on (idle)"));
        assert!(text.contains("listen mode:   Right Command hotkey (listening now)"));
        assert!(text.contains("Nixa speech:   on, voice Ava"));
        assert!(text.contains("headphones:    on"));
        assert!(text.contains("quiet:         on, 30 more min"));
        assert!(text.contains("speaker:       Desk"));
        assert!(text.contains("speakers:      none found"));
        let old = json!({"settings": {"listen": "wake"}});
        assert_eq!(view(&switches, &old, None, None, 0)["listen"], "always");
        assert!(lines(&view(&switches, &old, None, None, 0)).join("\n").contains("listen mode:   always on"));
        let bare = view(&switches, &voice, None, None, 0);
        let text = lines(&bare).join("\n");
        assert!(text.contains("quiet:         unknown"));
        assert!(text.contains("mics:          unknown"));
    }
}
