use crate::app::App;
use crate::privacy::redact::redact;
use crate::tools::{ToolResult, schema, spec};
use crate::turns::ToolSpec;
use crate::util::now_ms;
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const NAME: &str = "nixa_save_diagnostics";
pub const FOLDER: &str = "diagnostics";
pub const TAIL_BYTES: u64 = 200_000;
pub const LOGS: [&str; 6] = ["brain.log", "brain.log.1", "brain.log.2", "engine.log", "engine.log.1", "engine.log.2"];

pub fn spec_of() -> ToolSpec {
    spec(
        NAME,
        "Save a diagnostics file for a bug report: the Nixa version, this Mac's system, the main switches and the last part of the brain and engine logs. Keys, tokens, mail addresses, the home folder and the user name are hidden. It is a local file only; nothing is sent anywhere. Returns its path. Use it when the user asks for diagnostics or a log for a bug report.",
        schema(json!({}), &[]),
    )
}

pub fn call(app: &App) -> ToolResult {
    let path = save(&app.dir, &facts(app), now_ms()).map_err(|e| format!("The diagnostics file was not saved: {e}"))?;
    app.info(&format!("Saved a diagnostics file: {}", path.display()));
    Ok(json!({
        "saved": path.display().to_string(),
        "note": "A local file only. Keys, tokens, mail addresses, the home folder and the user name are hidden. Nothing was sent anywhere. Tell the user the path."
    })
    .to_string())
}

fn on(value: bool) -> String {
    if value { "on".into() } else { "off".into() }
}

pub fn facts(app: &App) -> Vec<(String, String)> {
    let voice = crate::voice::settings(app);
    let brain = crate::agent::brain_info(app)
        .map(|b| {
            let word = |key: &str| b.get(key).and_then(Value::as_str).unwrap_or("none").to_string();
            format!("{} ({}), model {}, effort {}", word("name"), word("kind"), word("model"), word("effort"))
        })
        .unwrap_or_else(|| "none picked".into());
    let schedules = crate::schedules::jobs::list(app).map(|l| l.len().to_string()).unwrap_or_else(|e| format!("not readable: {e:#}"));
    let go_live = super::go_live::outcome::read(&super::go_live::outcome::dir_now())
        .map(|o| super::go_live::outcome::text(&o))
        .unwrap_or_else(|| "none yet".into());
    vec![
        ("version".into(), env!("CARGO_PKG_VERSION").into()),
        ("system".into(), format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)),
        ("assistant name".into(), app.name()),
        ("chat brain".into(), brain),
        ("brain busy".into(), crate::agent::busy(app).to_string()),
        ("state".into(), crate::settings_api::state_word(app).into()),
        ("mic".into(), on(voice.mic_on)),
        ("voice replies".into(), on(voice.voice_on)),
        ("listen mode".into(), voice.listen.word().into()),
        ("decision".into(), app.decision.name()),
        ("private and confidential switch".into(), on(crate::settings_api::privacy_on(app))),
        ("terminals seen".into(), app.terminals().len().to_string()),
        ("schedules".into(), schedules),
        ("last go-live".into(), go_live),
    ]
}

pub fn tail(file: &Path, max: u64) -> String {
    let Ok(mut f) = std::fs::File::open(file) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    if f.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return text;
    }
    match text.find('\n') {
        Some(i) => text[i + 1..].to_string(),
        None => text,
    }
}

pub fn text_of(dir: &Path, facts: &[(String, String)], now: u64) -> String {
    let mut parts = vec![format!("Nixa diagnostics {}", crate::modes::clock::stamp(now))];
    for (key, value) in facts {
        parts.push(format!("{key}: {}", redact(value)));
    }
    for name in LOGS {
        let file = dir.join(name);
        if !file.is_file() {
            continue;
        }
        parts.push(format!("\n=== {name} (last {TAIL_BYTES} bytes) ===\n{}", redact(&tail(&file, TAIL_BYTES))));
    }
    format!("{}\n", parts.join("\n"))
}

pub fn save(dir: &Path, facts: &[(String, String)], now: u64) -> std::io::Result<PathBuf> {
    let text = text_of(dir, facts, now);
    let out_dir = dir.join(FOLDER);
    nixa_protocol::paths::ensure_private_dir(&out_dir)?;
    let out = out_dir.join(format!("diagnostics-{}.log", crate::extras::usage::day_key(now)));
    crate::util::atomic_write(&out, &text)?;
    Ok(out)
}
