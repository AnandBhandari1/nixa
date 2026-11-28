use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use crate::help;
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;

const MAX_QUIET_MINUTES: f64 = 24.0 * 60.0;

pub fn on_off(word: Option<&str>, name: &str) -> Result<bool> {
    match word {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => Err(usage(format!("use: nixa voice {name} on|off"))),
    }
}

pub fn switch_call(name: &str, on: bool) -> Option<(&'static str, Value)> {
    let call = match name {
        "nixa" => ("/switches", json!({"type": "set_nixa", "on": on})),
        "headphones" => ("/switches", json!({"type": "set_headphones", "on": on})),
        "git" => ("/switches", json!({"type": "set_git_voice", "on": on})),
        "agents" => ("/announce", json!({"on": on})),
        "mic" => ("/voice", json!({"type": "set_mic", "on": on})),
        _ => return None,
    };
    Some(call)
}

pub fn listen_mode(word: Option<&str>) -> Result<(&'static str, Option<&'static str>)> {
    match word.map(|w| w.trim().to_lowercase()).as_deref() {
        Some("hotkey" | "f9") => Ok(("hotkey", None)),
        Some("always") => Ok(("always", None)),
        Some("push" | "key") => Ok(("hotkey", Some("\"push\" is now \"hotkey\"."))),
        Some("wake") => Ok(("always", Some("Wake word mode is gone, so this is \"always\"."))),
        _ => Err(usage("use: nixa voice listen hotkey|always")),
    }
}

pub fn listen_body(mode: &str) -> Value {
    json!({"type": "set_listen", "mode": mode})
}

pub fn listen_word(s: &Value) -> &'static str {
    match s["listen"].as_str() {
        Some("always") | Some("wake") => "always on",
        _ => "Right Command hotkey",
    }
}

pub fn device_body(mic: Option<String>, speaker: Option<String>) -> Result<Value> {
    if mic.is_none() && speaker.is_none() {
        return Err(usage("use: nixa voice device --mic NAME --speaker NAME (default goes back to the Mac default)"));
    }
    let pick = |name: String| if name.trim().eq_ignore_ascii_case("default") { String::new() } else { name.trim().to_string() };
    let mut body = json!({"type": "set_audio"});
    if let Some(mic) = mic {
        body["mic"] = json!(pick(mic));
    }
    if let Some(speaker) = speaker {
        body["speaker"] = json!(pick(speaker));
    }
    Ok(body)
}

pub fn quiet_minutes(word: Option<&str>) -> Result<f64> {
    let Some(word) = word.map(str::trim) else {
        return Err(usage("use: nixa voice quiet 30m|2h|off"));
    };
    if word == "off" {
        return Ok(0.0);
    }
    let minutes = if word.bytes().all(|b| b.is_ascii_digit()) {
        args::number::<f64>(word, "quiet")?
    } else {
        args::duration_ms(word)? as f64 / 60_000.0
    };
    if minutes <= 0.0 || minutes > MAX_QUIET_MINUTES {
        return Err(usage("quiet time must be more than 0 and at most 24h"));
    }
    Ok(minutes)
}

fn word(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn flag(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}

fn device_name(v: &Value, empty: &str) -> String {
    v.as_str().filter(|s| !s.is_empty()).unwrap_or(empty).to_string()
}

pub fn switch_line(name: &str, reply: &Value) -> String {
    let sw = &reply["switches"];
    let s = &reply["voice"]["settings"];
    match name {
        "nixa" => format!("Nixa is {}.", word(flag(&sw["nixa"]))),
        "headphones" => format!("Headphones are {}.", word(flag(&sw["headphones"]))),
        "git" => format!("Git voice is {}.", word(flag(&sw["gitVoice"]))),
        "agents" => format!("Agent voice is {}.", word(flag(&reply["on"]))),
        "mic" => format!("Mic is {}. Listen mode is {}.", word(flag(&s["micOn"])), listen_word(s)),
        _ => String::new(),
    }
}

pub fn listen_line(reply: &Value) -> String {
    let s = &reply["voice"]["settings"];
    format!("Listen mode is {}. Mic is {}.", listen_word(s), word(flag(&s["micOn"])))
}

pub fn device_line(reply: &Value) -> String {
    let s = &reply["voice"]["settings"];
    format!("Mic: {}. Speaker: {}.", device_name(&s["micDevice"], "Mac default"), device_name(&s["speakerDevice"], "Mac output"))
}

pub fn quiet_left(talk: &Value, now_ms: u64) -> Option<u64> {
    let until = talk["presence"]["quietUntil"].as_u64()?;
    (until > now_ms).then(|| (until - now_ms).div_ceil(60_000))
}

pub fn quiet_line(talk: &Value, now_ms: u64) -> String {
    match quiet_left(talk, now_ms) {
        Some(min) => format!("Quiet for {min} more min. Nixa does not speak until then."),
        None => "Quiet time is off.".into(),
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn print(ctx: &Ctx, out: &mut dyn Write, reply: &Value, line: String) -> Result<i32> {
    if ctx.json {
        writeln!(out, "{reply}")?;
    } else {
        writeln!(out, "{line}")?;
    }
    Ok(0)
}

fn parsed(ctx: &Ctx, argv: &[String], with_value: &[&str]) -> Result<Option<(args::Parsed, Ctx)>> {
    let (mut p, ctx) = ctx::command(ctx, argv, with_value, false)?;
    if p.flag(&["-h", "--help"]) {
        return Ok(None);
    }
    Ok(Some((p, ctx)))
}

fn help_out(out: &mut dyn Write) -> Result<i32> {
    out.write_all(help::VOICE.as_bytes())?;
    Ok(0)
}

pub fn switch(ctx: &Ctx, name: &str, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((p, ctx)) = parsed(ctx, argv, &[])? else { return help_out(out) };
    p.finish(1, false)?;
    let on = on_off(p.pos.first().map(String::as_str), name)?;
    let Some((path, body)) = switch_call(name, on) else {
        return Err(usage(format!("unknown command nixa voice {name}")));
    };
    let reply = http::window_call("POST", path, Some(&body))?;
    print(&ctx, out, &reply, switch_line(name, &reply))
}

pub fn listen(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((p, ctx)) = parsed(ctx, argv, &[])? else { return help_out(out) };
    p.finish(1, false)?;
    let (mode, note) = listen_mode(p.pos.first().map(String::as_str))?;
    let reply = http::window_call("POST", "/voice", Some(&listen_body(mode)))?;
    let line = match note {
        Some(note) => format!("{note} {}", listen_line(&reply)),
        None => listen_line(&reply),
    };
    print(&ctx, out, &reply, line)
}

pub fn device(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((mut p, ctx)) = parsed(ctx, argv, &["--mic", "--speaker"])? else { return help_out(out) };
    let body = device_body(p.value(&["--mic"]), p.value(&["--speaker"]))?;
    p.finish(0, false)?;
    let reply = http::window_call("POST", "/voice", Some(&body))?;
    print(&ctx, out, &reply, device_line(&reply))
}

pub fn devices_lines(devices: &Value, settings: &Value) -> Vec<String> {
    let list = |key: &str| -> String {
        let names: Vec<&str> = devices[key].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if names.is_empty() { "none found".into() } else { names.join(", ") }
    };
    vec![
        format!("mic in use: {}", device_name(&settings["micDevice"], "Mac default")),
        format!("speaker in use: {}", device_name(&settings["speakerDevice"], "Mac output")),
        format!("mics: {}", list("mics")),
        format!("speakers: {}", list("speakers")),
    ]
}

pub fn devices(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((p, ctx)) = parsed(ctx, argv, &[])? else { return help_out(out) };
    p.finish(0, false)?;
    let mut lists = http::get("/voice/devices")?;
    let voice = http::get("/voice")?;
    let settings = voice["voice"]["settings"].clone();
    if ctx.json {
        lists["mic"] = settings["micDevice"].clone();
        lists["speaker"] = settings["speakerDevice"].clone();
        writeln!(out, "{lists}")?;
        return Ok(0);
    }
    for line in devices_lines(&lists, &settings) {
        writeln!(out, "{line}")?;
    }
    Ok(0)
}

pub fn quiet(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((p, ctx)) = parsed(ctx, argv, &[])? else { return help_out(out) };
    p.finish(1, false)?;
    let minutes = quiet_minutes(p.pos.first().map(String::as_str))?;
    let reply = http::window_call("POST", "/talk/quiet", Some(&json!({"minutes": minutes})))?;
    print(&ctx, out, &reply, quiet_line(&reply["talk"], now_ms()))
}

pub fn stop(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((p, ctx)) = parsed(ctx, argv, &[])? else { return help_out(out) };
    p.finish(0, false)?;
    let reply = http::window_call("POST", "/voice", Some(&json!({"type": "stop"})))?;
    let line = if flag(&reply["voice"]["speaking"]) { "Asked Nixa to stop. It is still finishing a sound." } else { "Stopped. Nixa is not speaking." };
    print(&ctx, out, &reply, line.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_bodies_match_the_brain() {
        assert_eq!(switch_call("nixa", true), Some(("/switches", json!({"type": "set_nixa", "on": true}))));
        assert_eq!(switch_call("headphones", false), Some(("/switches", json!({"type": "set_headphones", "on": false}))));
        assert_eq!(switch_call("git", true), Some(("/switches", json!({"type": "set_git_voice", "on": true}))));
        assert_eq!(switch_call("agents", false), Some(("/announce", json!({"on": false}))));
        assert_eq!(switch_call("mic", true), Some(("/voice", json!({"type": "set_mic", "on": true}))));
        assert_eq!(switch_call("push-talk", false), None);
        assert_eq!(switch_call("other", true), None);
        assert!(on_off(Some("on"), "mic").unwrap());
        assert!(!on_off(Some("off"), "mic").unwrap());
        assert!(on_off(Some("yes"), "mic").is_err());
        assert!(on_off(None, "mic").is_err());
    }

    #[test]
    fn listen_and_device_bodies() {
        assert_eq!(listen_mode(Some("hotkey")).unwrap(), ("hotkey", None));
        assert_eq!(listen_mode(Some("F9")).unwrap(), ("hotkey", None));
        assert_eq!(listen_mode(Some("always")).unwrap(), ("always", None));
        assert_eq!(listen_mode(Some("push")).unwrap().0, "hotkey");
        assert!(listen_mode(Some("push")).unwrap().1.is_some());
        assert_eq!(listen_mode(Some("wake")).unwrap().0, "always");
        assert!(listen_mode(Some("wake")).unwrap().1.unwrap().contains("gone"));
        assert!(listen_mode(Some("loud")).is_err());
        assert!(listen_mode(None).is_err());
        assert_eq!(listen_body("hotkey"), json!({"type": "set_listen", "mode": "hotkey"}));
        assert_eq!(device_body(Some("USB Mic".into()), None).unwrap(), json!({"type": "set_audio", "mic": "USB Mic"}));
        assert_eq!(device_body(None, Some("default".into())).unwrap(), json!({"type": "set_audio", "speaker": ""}));
        assert!(device_body(None, None).is_err());
    }

    #[test]
    fn quiet_times() {
        assert_eq!(quiet_minutes(Some("off")).unwrap(), 0.0);
        assert_eq!(quiet_minutes(Some("30m")).unwrap(), 30.0);
        assert_eq!(quiet_minutes(Some("2h")).unwrap(), 120.0);
        assert_eq!(quiet_minutes(Some("45")).unwrap(), 45.0);
        assert_eq!(quiet_minutes(Some("90s")).unwrap(), 1.5);
        assert!(quiet_minutes(Some("0")).is_err());
        assert!(quiet_minutes(Some("25h")).is_err());
        assert!(quiet_minutes(Some("soon")).is_err());
        assert!(quiet_minutes(None).is_err());
        let talk = json!({"presence": {"quietUntil": 10 * 60_000}});
        assert_eq!(quiet_left(&talk, 60_000), Some(9));
        assert_eq!(quiet_left(&talk, 11 * 60_000), None);
        assert_eq!(quiet_line(&json!({"presence": {}}), 0), "Quiet time is off.");
    }

    #[test]
    fn lines_read_the_reply() {
        let reply = json!({"switches": {"nixa": true, "gitVoice": false}, "voice": {"settings": {"micOn": true, "listen": "always", "micDevice": null, "speakerDevice": "Desk"}}});
        assert_eq!(switch_line("nixa", &reply), "Nixa is on.");
        assert_eq!(switch_line("git", &reply), "Git voice is off.");
        assert_eq!(switch_line("mic", &reply), "Mic is on. Listen mode is always on.");
        assert_eq!(listen_line(&reply), "Listen mode is always on. Mic is on.");
        let hotkey = json!({"voice": {"settings": {"micOn": false, "listen": "hotkey"}}});
        assert_eq!(listen_line(&hotkey), "Listen mode is Right Command hotkey. Mic is off.");
        assert_eq!(device_line(&reply), "Mic: Mac default. Speaker: Desk.");
        let lines = devices_lines(&json!({"mics": ["A", "B"], "speakers": []}), &reply["voice"]["settings"]);
        assert_eq!(lines[2], "mics: A, B");
        assert_eq!(lines[3], "speakers: none found");
    }
}
