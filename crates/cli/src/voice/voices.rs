use crate::args::usage;
use crate::ctx::{self, Ctx};
use crate::help;
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;

const SECTIONS: [(&str, &str); 5] = [("terminals", "terminal"), ("models", "model"), ("agents", "agent"), ("spaces", "space"), ("kinds", "kind")];

pub fn play_body(name: Option<&str>) -> Result<Value> {
    let name = name.map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| usage("use: nixa voice play NAME (see nixa voice voices)"))?;
    Ok(json!({"action": "play", "voice": name}))
}

fn join(v: &Value) -> String {
    let all: Vec<&str> = v.as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if all.is_empty() { "none".into() } else { all.join(", ") }
}

pub fn lines(data: &Value) -> Vec<String> {
    let mut out = vec![format!("voices: {}", join(&data["voices"]))];
    if data["voiceOn"].as_bool() == Some(false)
        && let Some(note) = data["note"].as_str().filter(|n| !n.is_empty())
    {
        out.push(note.to_string());
    }
    if let Some(v) = data["fallback"]["voice"].as_str().filter(|v| !v.is_empty()) {
        out.push(format!("used when no rule fits: {v}"));
    }
    let map = &data["map"];
    let mut rules = Vec::new();
    if let Some(v) = map["default"].as_str() {
        rules.push(format!("  default: {v}"));
    }
    for (section, level) in SECTIONS {
        for (key, voice) in map[section].as_object().into_iter().flatten() {
            rules.push(format!("  {level} {key}: {}", voice.as_str().unwrap_or("")));
        }
    }
    if rules.is_empty() {
        out.push("rules: none, the built-in voices are used".into());
    } else {
        out.push("rules:".into());
        out.extend(rules);
    }
    let builtin: Vec<String> = data["builtin"].as_object().into_iter().flatten().filter_map(|(a, v)| v.as_str().filter(|v| !v.is_empty()).map(|v| format!("{a} {v}"))).collect();
    if !builtin.is_empty() {
        out.push(format!("built in: {}", builtin.join(", ")));
    }
    if let Some(h) = data["humor"].as_str() {
        out.push(format!("humor: {h}"));
    }
    out
}

pub fn list(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &[], false)?;
    if p.flag(&["-h", "--help"]) {
        out.write_all(help::VOICE.as_bytes())?;
        return Ok(0);
    }
    p.finish(0, false)?;
    let reply = http::window_call("GET", "/voices", None)?;
    if ctx.json {
        writeln!(out, "{}", reply["data"])?;
        return Ok(0);
    }
    for line in lines(&reply["data"]) {
        writeln!(out, "{line}")?;
    }
    Ok(0)
}

pub fn play(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &[], false)?;
    if p.flag(&["-h", "--help"]) {
        out.write_all(help::VOICE.as_bytes())?;
        return Ok(0);
    }
    let name = p.pos.join(" ");
    let body = play_body(Some(&name))?;
    let reply = http::window_call("POST", "/voices", Some(&body))?;
    if ctx.json {
        writeln!(out, "{}", reply["data"])?;
    } else {
        writeln!(out, "Playing a sample in {}.", reply["data"]["voice"].as_str().unwrap_or(&name))?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_needs_a_name() {
        assert_eq!(play_body(Some(" Ava ")).unwrap(), json!({"action": "play", "voice": "Ava"}));
        assert!(play_body(Some(" ")).is_err());
        assert!(play_body(None).is_err());
    }

    #[test]
    fn voices_and_rules_read_plainly() {
        let data = json!({
            "voices": ["Ava", "Tom"], "voiceOn": false, "note": "Voice is off.",
            "fallback": {"voice": "Ava"}, "humor": "light",
            "map": {"default": "Tom", "terminals": {}, "models": {}, "agents": {"codex": "Ava"}, "spaces": {}, "kinds": {}},
            "builtin": {"nixa": "Ava"}
        });
        let text = lines(&data).join("\n");
        assert!(text.contains("voices: Ava, Tom"));
        assert!(text.contains("Voice is off."));
        assert!(text.contains("  default: Tom"));
        assert!(text.contains("  agent codex: Ava"));
        assert!(text.contains("built in: nixa Ava"));
        let empty = lines(&json!({"voices": [], "map": {}}));
        assert!(empty.iter().any(|l| l.starts_with("rules: none")));
    }
}
