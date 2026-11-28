use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use crate::help;
use anyhow::{Result, bail};
use nixa_protocol::paths;
use nixa_voice::{EchoMic, FileState, Model, Step, check, fetch, level, missing_models, models, models_dir, models_url, open_echo_mic};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

mod status;
mod switches;
mod voices;

const METER: Duration = Duration::from_millis(500);
const MAX_SECONDS: u64 = 600;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::VOICE.as_bytes())?;
        return Ok(2);
    };
    match sub.as_str() {
        "models" => models_cmd(ctx, rest, out),
        "mic" if rest.first().is_some_and(|a| a == "on" || a == "off") => switches::switch(ctx, "mic", rest, out),
        "mic" => mic(ctx, rest, out),
        "status" => status::status(ctx, rest, out),
        "nixa" | "agents" | "git" | "headphones" => switches::switch(ctx, sub, rest, out),
        "listen" => switches::listen(ctx, rest, out),
        "devices" => switches::devices(ctx, rest, out),
        "device" => switches::device(ctx, rest, out),
        "quiet" => switches::quiet(ctx, rest, out),
        "stop" => switches::stop(ctx, rest, out),
        "voices" => voices::list(ctx, rest, out),
        "play" => voices::play(ctx, rest, out),
        "help" | "-h" | "--help" => {
            out.write_all(help::VOICE.as_bytes())?;
            Ok(0)
        }
        other => Err(usage(format!("unknown command nixa voice {other}"))),
    }
}

fn is_nixa_support_name(name: &std::ffi::OsStr) -> bool {
    name.eq_ignore_ascii_case(paths::APP_DATA_NAME) || name.eq_ignore_ascii_case(paths::DEV_DATA_NAME)
}

fn foreign_app_dir(dir: &Path, home: Option<&Path>) -> bool {
    let canon = real(dir);
    if let Some(home) = home {
        let support = real(&home.join("Library/Application Support"));
        if let Ok(rest) = canon.strip_prefix(&support) {
            let Some(std::path::Component::Normal(app)) = rest.components().next() else {
                return true;
            };
            if !is_nixa_support_name(app) {
                return true;
            }
        }
    }
    let mut parts = canon.components();
    while let Some(part) = parts.next() {
        if let std::path::Component::Normal(name) = part
            && name == ".config"
        {
            match parts.next() {
                Some(std::path::Component::Normal(app)) if app.eq_ignore_ascii_case("nixa") => {}
                _ => return true,
            }
        }
    }
    false
}

fn real(path: &Path) -> PathBuf {
    let mut at = path.to_path_buf();
    let mut tail = Vec::new();
    while !at.exists() {
        let Some(name) = at.file_name().map(|n| n.to_os_string()) else { break };
        tail.push(name);
        if !at.pop() {
            break;
        }
    }
    let mut base = std::fs::canonicalize(&at).unwrap_or(at);
    for name in tail.into_iter().rev() {
        base.push(name);
    }
    base
}

fn pick_dir(p: &mut args::Parsed) -> Result<PathBuf> {
    let dir = p.value(&["--dir"]).map(PathBuf::from).unwrap_or_else(models_dir);
    let dir = if dir.is_absolute() { dir } else { std::env::current_dir()?.join(dir) };
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if foreign_app_dir(&dir, home.as_deref()) {
        bail!("{} belongs to another local app; use the Nixa data folder", dir.display());
    }
    Ok(dir)
}

fn state_word(s: &FileState) -> String {
    match s {
        FileState::Ok => "ok".into(),
        FileState::Missing => "missing".into(),
        FileState::Size { found } => format!("wrong size ({found} bytes)"),
        FileState::Hash => "wrong hash".into(),
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

fn model_json(dir: &Path, m: &Model, hash: bool) -> Value {
    let files: Vec<Value> = check(dir, m, hash)
        .iter()
        .map(|(f, s)| json!({"path": f.path, "size": f.size, "sha256": f.sha256, "state": state_word(s), "ok": *s == FileState::Ok}))
        .collect();
    let ready = files.iter().all(|f| f["ok"] == json!(true));
    json!({"name": m.name, "what": m.what, "download": m.download, "download_size": m.size, "ready": ready, "files": files})
}

fn models_cmd(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--dir"], false)?;
    if p.flag(&["-h", "--help"]) {
        out.write_all(help::VOICE.as_bytes())?;
        return Ok(0);
    }
    let action = p.pos.first().cloned().unwrap_or_else(|| "status".into());
    match action.as_str() {
        "status" => {
            let hash = p.flag(&["--check"]);
            let dir = pick_dir(&mut p)?;
            p.finish(1, false)?;
            status(&ctx, &dir, hash, out)
        }
        "fetch" => {
            let dry = p.flag(&["--dry-run", "-n"]);
            let dir = pick_dir(&mut p)?;
            p.finish(1, false)?;
            if dry { plan(&ctx, &dir, out) } else { fetch_cmd(&ctx, &dir, out) }
        }
        "help" => {
            out.write_all(help::VOICE.as_bytes())?;
            Ok(0)
        }
        other => Err(usage(format!("unknown command nixa voice models {other}"))),
    }
}

fn status(ctx: &Ctx, dir: &Path, hash: bool, out: &mut dyn Write) -> Result<i32> {
    let list = models();
    let all: Vec<Value> = list.iter().map(|m| model_json(dir, m, hash)).collect();
    let ready = all.iter().all(|m| m["ready"] == json!(true));
    if ctx.json {
        writeln!(out, "{}", json!({"dir": dir, "ready": ready, "checked_hash": hash, "models": all}))?;
        return Ok(if ready { 0 } else { 1 });
    }
    writeln!(out, "voice models in {}", dir.display())?;
    for m in &all {
        writeln!(out, "{}  {}: {}", m["name"].as_str().unwrap_or(""), m["what"].as_str().unwrap_or(""), if m["ready"] == json!(true) { "ready" } else { "not ready" })?;
        for f in m["files"].as_array().into_iter().flatten() {
            writeln!(out, "    {:<10} {}", f["state"].as_str().unwrap_or(""), f["path"].as_str().unwrap_or(""))?;
        }
    }
    if !ready {
        writeln!(out, "Get them with: nixa voice models fetch")?;
    } else if !hash {
        writeln!(out, "Sizes match. Add --check to check every hash too.")?;
    }
    Ok(if ready { 0 } else { 1 })
}

fn plan(ctx: &Ctx, dir: &Path, out: &mut dyn Write) -> Result<i32> {
    let need = missing_models(dir, &models(), false);
    let base = models_url();
    let total: u64 = need.iter().map(|m| m.size).sum();
    if ctx.json {
        let list: Vec<Value> = need.iter().map(|m| json!({"name": m.name, "url": format!("{base}/{}", m.download), "size": m.size, "sha256": m.sha256})).collect();
        writeln!(out, "{}", json!({"dir": dir, "dry_run": true, "download": list, "bytes": total}))?;
        return Ok(0);
    }
    if need.is_empty() {
        writeln!(out, "All voice models are in {}. Nothing to fetch.", dir.display())?;
        return Ok(0);
    }
    writeln!(out, "Dry run. nixa voice models fetch would download into {}:", dir.display())?;
    for m in &need {
        writeln!(out, "  {}  {}/{}  {}  sha256 {}", m.name, base, m.download, mb(m.size), m.sha256)?;
    }
    writeln!(out, "Total {}. Each download is checked for size and hash before it is used.", mb(total))?;
    Ok(0)
}

fn step_line(step: &Step) -> Option<String> {
    match step {
        Step::Ready { name } => Some(format!("{name}: already here")),
        Step::Start { name, size, from: 0 } => Some(format!("{name}: downloading {}", mb(*size))),
        Step::Start { name, size, from } => Some(format!("{name}: going on from {} of {}", mb(*from), mb(*size))),
        Step::Bytes { name, done, size } => Some(format!("{name}: {}%", done * 100 / (*size).max(1))),
        Step::Unpack { name } => Some(format!("{name}: unpacking")),
        Step::Check { name } => Some(format!("{name}: checking size and hash")),
        Step::Done { name } => Some(format!("{name}: ready")),
    }
}

fn fetch_cmd(ctx: &Ctx, dir: &Path, out: &mut dyn Write) -> Result<i32> {
    let json = ctx.json;
    let mut last_pct: Option<(String, u64)> = None;
    let mut on = |step: &Step| {
        if json {
            return;
        }
        if let Step::Bytes { name, done, size } = step {
            let pct = done * 100 / (*size).max(1);
            let tenth = pct / 10;
            if last_pct.as_ref().is_some_and(|(n, t)| n == name && *t == tenth) && pct != 100 {
                return;
            }
            last_pct = Some((name.clone(), tenth));
        }
        if let Some(line) = step_line(step) {
            eprintln!("{line}");
        }
    };
    let fetched = fetch(dir, &models(), &models_url(), &mut on)?;
    if json {
        writeln!(out, "{}", json!({"dir": dir, "fetched": fetched}))?;
    } else if fetched.is_empty() {
        writeln!(out, "All voice models are in {}.", dir.display())?;
    } else {
        writeln!(out, "Fetched {} into {}.", fetched.join(", "), dir.display())?;
    }
    Ok(0)
}

fn mic(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--device", "--seconds"], false)?;
    if p.flag(&["-h", "--help"]) {
        out.write_all(help::VOICE.as_bytes())?;
        return Ok(0);
    }
    let device = p.value(&["--device"]);
    let headphones = p.flag(&["--headphones"]);
    let no_echo = p.flag(&["--no-echo-cancel"]);
    let seconds = match p.value(&["--seconds"]) {
        Some(v) => args::number::<u64>(&v, "--seconds")?,
        None => 5,
    };
    p.finish(0, false)?;
    if seconds == 0 || seconds > MAX_SECONDS {
        return Err(usage(format!("--seconds must be 1 to {MAX_SECONDS}")));
    }
    let echo = nixa_voice::use_echo_cancel(!no_echo, headphones);
    let (tx, rx) = channel();
    let failed = Arc::new(AtomicBool::new(false));
    let opened = open_echo_mic(device.as_deref(), echo, tx, failed.clone());
    let mic: EchoMic = match opened {
        Ok(m) => m,
        Err(e) if echo => bail!("{e:#}\nTry: nixa voice mic --no-echo-cancel"),
        Err(e) => return Err(e),
    };
    let ready = nixa_voice::ready_line(&mic);
    if ctx.json {
        writeln!(out, "{}", json!({"type": "ready", "echo_cancel": mic.echo_cancel, "device": mic.name, "missing": mic.missing, "rate": mic.rate, "channels": mic.channels}))?;
    } else {
        writeln!(out, "{ready}")?;
        if mic.missing {
            writeln!(out, "the chosen mic is not connected, using {}", mic.name)?;
        }
    }
    let until = Instant::now() + Duration::from_secs(seconds);
    let mut meter: Vec<i16> = Vec::new();
    let mut samples = 0usize;
    let mut peak = 0f32;
    let mut next = Instant::now() + METER;
    while Instant::now() < until && !failed.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(frame) => {
                samples += frame.len();
                meter.extend_from_slice(&frame);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if Instant::now() >= next {
            next += METER;
            let l = level(&meter);
            peak = peak.max(l);
            meter.clear();
            if ctx.json {
                writeln!(out, "{}", json!({"type": "level", "level": l}))?;
            } else {
                writeln!(out, "level {l:.3}")?;
            }
            out.flush()?;
        }
    }
    let changed = failed.load(Ordering::SeqCst);
    drop(mic);
    let ms = samples as u64 * 1000 / u64::from(nixa_voice::SAMPLE_RATE);
    if ctx.json {
        writeln!(out, "{}", json!({"type": "done", "audio_ms": ms, "peak": peak, "setup_changed": changed}))?;
    } else {
        writeln!(out, "heard {ms} ms of 16 kHz audio, peak level {peak:.3}")?;
        if changed {
            writeln!(out, "the audio setup changed, so the mic stopped early")?;
        }
    }
    Ok(if changed || samples == 0 { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_folders_are_refused() {
        let home = Path::new("/tmp/nixa-live-home");
        assert!(foreign_app_dir(&home.join("Library/Application Support/OtherApp/data/voice-models"), Some(home)));
        assert!(foreign_app_dir(&home.join("Library/Application Support/OtherApp"), Some(home)));
        assert!(foreign_app_dir(&home.join("Library/Application Support"), Some(home)));
        assert!(foreign_app_dir(Path::new("/tmp/projects/other/.config/other/voice-models"), Some(home)));
        assert!(!foreign_app_dir(&home.join("Library/Application Support/Nixa/voice-models"), Some(home)));
        assert!(!foreign_app_dir(&home.join("Library/Application Support/Nixa Dev/voice-models"), Some(home)));
        assert!(!foreign_app_dir(Path::new("/tmp/projects/nixa/.config/nixa/voice-models"), Some(home)));
        assert!(!foreign_app_dir(Path::new("/tmp/custom/voice-models"), Some(home)));
        let env_home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
        assert!(!foreign_app_dir(&models_dir(), env_home.as_deref()));
    }

    #[test]
    fn steps_read_plainly() {
        assert_eq!(step_line(&Step::Start { name: "stt".into(), size: 2_000_000, from: 0 }).unwrap(), "stt: downloading 2.0 MB");
        assert_eq!(step_line(&Step::Bytes { name: "stt".into(), done: 1, size: 2 }).unwrap(), "stt: 50%");
        assert!(step_line(&Step::Start { name: "vad".into(), size: 10, from: 5 }).unwrap().contains("going on"));
    }
}
