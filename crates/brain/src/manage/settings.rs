use crate::app::{App, lock};
use crate::provider::Model;
use crate::tools::{Args, ToolResult, need, schema, spec};
use crate::turns::ToolSpec;
use crate::util::now_ms;
use crate::voice::{Command, Listen};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

mod describe;

pub const NAME: &str = "nixa_settings";
pub const KEYS: [&str; 12] = ["mic", "voice", "listen", "brain", "model", "effort", "quiet", "nixa", "headphones", "privacy", "git_voice", "agent_voice"];
pub const ON_OFF: [&str; 2] = ["on", "off"];
pub const LISTEN: [&str; 2] = ["hotkey", "always"];
pub const QUIET_MAX_MINUTES: f64 = 24.0 * 60.0;
pub const BRAIN_WAIT_MS: u64 = 10 * 60 * 1000;
pub const BRAIN_GAP_MS: u64 = 500;
pub const BRAIN_TRIES: u32 = 6;
pub const WHY: &str = "A settings change asked by the chat brain";
pub const READ_NOTE: &str = "Reading settings never gives permission. Automatic applies valid changes without approval. Manual shows the old and new value on a yes card before applying them.";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Now {
    pub mic: bool,
    pub voice: bool,
    pub listen: String,
    pub brain: Option<String>,
    pub brain_name: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub quiet_until: u64,
    pub nixa: bool,
    pub headphones: bool,
    pub privacy: bool,
    pub git_voice: bool,
    pub agent_voice: bool,
}

pub fn spec_of() -> ToolSpec {
    spec(
        NAME,
        "Read or change supported Nixa settings. describe lists Settings window sections with their supported keys, current values, local choices and availability. Optionally pass a section and key. get reads all supported settings or one key, with allowed choices. refresh_models loads the selected Chat brain's model list without changing the brain or model selection. CLI adapters may return their current list rather than fetch a new one. set changes one key: value for every key except quiet, minutes for quiet (0 ends quiet time). Automatic applies valid changes without waiting for approval. Manual shows a yes card and waits for approval. Use exact choices from get, never guessed ones. effort with an empty value means the brain's own default. A brain change happens after this reply, so read again before you say it changed. The result says verified only when the new value was read back.",
        schema(
            json!({
                "action": {"type": "string", "enum": ["describe", "get", "refresh_models", "set"]},
                "section": {"type": "string", "enum": describe::SECTIONS.map(|section| section.id), "description": "For describe: a Settings window section."},
                "key": {"type": "string", "enum": KEYS},
                "value": {"type": "string", "maxLength": 200, "description": "For set: one of the choices that get lists. Empty for effort means the brain's default."},
                "minutes": {"type": "number", "minimum": 0, "description": "For set quiet: how many minutes of quiet time. 0 ends quiet time."}
            }),
            &["action"],
        ),
    )
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let action = need(args, "action")?;
    let fields = match action.as_str() {
        "describe" => &["action", "section", "key"][..],
        "get" => &["action", "key"],
        "refresh_models" => &["action"],
        "set" => &["action", "key", "value", "minutes"],
        _ => return Err(format!("action {action} is not known. Use describe, get, refresh_models or set.")),
    };
    if args.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err("Unknown settings argument.".into());
    }
    match action.as_str() {
        "describe" => describe::call(app, args),
        "get" if args.contains_key("key") => get(app, Some(&need(args, "key")?)).await,
        "get" => get(app, None).await,
        "refresh_models" => {
            let cli = crate::cli_brains::chosen(app).is_some();
            let models = crate::agent::load_models(app).await?;
            Ok(json!({
                "status": if cli { "loaded" } else { "refreshed" },
                "modelCount": models.len(),
                "note": if cli {
                    "Loaded the CLI adapter's model list. The adapter may use cached choices."
                } else {
                    "Refreshed the list from the selected API provider."
                },
            }).to_string())
        }
        _ => set(app, args).await,
    }
}

pub fn on_word(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

pub fn label(key: &str) -> &'static str {
    match key {
        "mic" => "Mic",
        "voice" => "Voice replies",
        "listen" => "Listen mode",
        "brain" => "Chat brain",
        "model" => "Brain model",
        "effort" => "Brain effort",
        "quiet" => "Quiet time",
        "nixa" => "Nixa",
        "headphones" => "Headphones",
        "privacy" => "Privacy",
        "git_voice" => "Git voice",
        "agent_voice" => "Agent voice",
        _ => "Setting",
    }
}

pub fn applies(key: &str) -> &'static str {
    match key {
        "brain" => "It changes after this reply.",
        "model" | "effort" => "It is used from the next brain step.",
        _ => "It changes right away.",
    }
}

pub fn read(app: &App) -> Now {
    let voice = crate::voice::settings(app);
    let info = crate::agent::brain_info(app).unwrap_or(Value::Null);
    let brain = app.settings().choice;
    let word = |key: &str| info.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    Now {
        mic: voice.mic_on,
        voice: voice.voice_on,
        listen: voice.listen.word().to_string(),
        brain_name: brain.as_deref().and_then(|c| crate::agent::brain_name(app, c)),
        brain,
        model: word("model"),
        effort: word("effort"),
        quiet_until: lock(&app.talk.presence).quiet_until(now_ms()),
        nixa: crate::settings_api::nixa_on(&app.dir),
        headphones: crate::settings_api::headphones(&app.dir),
        privacy: crate::settings_api::privacy_on(app),
        git_voice: crate::settings_api::git_voice(app),
        agent_voice: crate::announce::is_on(app),
    }
}

pub fn quiet_words(until: u64, now: u64) -> String {
    if until <= now {
        return "off".into();
    }
    let minutes = (until - now).div_ceil(60_000);
    format!("on for {minutes} more minute{}", if minutes == 1 { "" } else { "s" })
}

pub fn minutes_words(minutes: f64) -> String {
    if minutes <= 0.0 {
        return "off".into();
    }
    let shown = (minutes * 10.0).round() / 10.0;
    format!("on for {shown} minute{}", if shown == 1.0 { "" } else { "s" })
}

pub fn shown(now: &Now, key: &str, at: u64) -> String {
    let or_default = |v: &Option<String>, none: &str| v.clone().unwrap_or_else(|| none.to_string());
    match key {
        "mic" => on_word(now.mic).into(),
        "voice" => on_word(now.voice).into(),
        "listen" => now.listen.clone(),
        "brain" => match (&now.brain, &now.brain_name) {
            (Some(choice), Some(name)) => format!("{name} ({choice})"),
            (Some(choice), None) => choice.clone(),
            _ => "none".into(),
        },
        "model" => or_default(&now.model, "none"),
        "effort" => or_default(&now.effort, "default"),
        "quiet" => quiet_words(now.quiet_until, at),
        "nixa" => on_word(now.nixa).into(),
        "headphones" => on_word(now.headphones).into(),
        "privacy" => on_word(now.privacy).into(),
        "git_voice" => on_word(now.git_voice).into(),
        "agent_voice" => on_word(now.agent_voice).into(),
        _ => String::new(),
    }
}

pub fn raw(now: &Now, key: &str) -> Value {
    match key {
        "mic" => json!(on_word(now.mic)),
        "voice" => json!(on_word(now.voice)),
        "listen" => json!(now.listen),
        "brain" => json!(now.brain),
        "model" => json!(now.model),
        "effort" => json!(now.effort),
        "quiet" => json!(now.quiet_until),
        "nixa" => json!(on_word(now.nixa)),
        "headphones" => json!(on_word(now.headphones)),
        "privacy" => json!(on_word(now.privacy)),
        "git_voice" => json!(on_word(now.git_voice)),
        "agent_voice" => json!(on_word(now.agent_voice)),
        _ => Value::Null,
    }
}

pub fn brains(app: &App) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = app.brains.list().into_iter().map(|b| (crate::brains::choice_of(&b.id), b.name)).collect();
    for cli in crate::agent::cli_list(app) {
        if cli.get("installed").and_then(Value::as_bool) == Some(true)
            && let (Some(choice), Some(name)) = (cli.get("choice").and_then(Value::as_str), cli.get("name").and_then(Value::as_str))
        {
            out.push((choice.to_string(), name.to_string()));
        }
    }
    out
}

async fn models(app: &Arc<App>) -> Result<Vec<Model>, String> {
    let known = lock(&app.brain).models.clone();
    if !known.is_empty() {
        return Ok(known);
    }
    crate::agent::load_models(app).await
}

pub fn efforts_of(models: &[Model], model: Option<&str>) -> Vec<String> {
    model.and_then(|m| models.iter().find(|x| x.id == m)).map(|m| m.efforts.clone()).unwrap_or_default()
}

pub fn choices(key: &str, brains: &[(String, String)], models: &[Model], model_now: Option<&str>) -> Vec<String> {
    match key {
        "mic" | "voice" | "nixa" | "headphones" | "privacy" | "git_voice" | "agent_voice" => ON_OFF.iter().map(|s| s.to_string()).collect(),
        "listen" => LISTEN.iter().map(|s| s.to_string()).collect(),
        "brain" => brains.iter().map(|(choice, _)| choice.clone()).collect(),
        "model" => models.iter().map(|m| m.id.clone()).collect(),
        "effort" => {
            let mut out = vec![String::new()];
            out.extend(efforts_of(models, model_now));
            out
        }
        _ => Vec::new(),
    }
}

pub fn check_value(key: &str, value: &str, brains: &[(String, String)], models: &[Model], model_now: Option<&str>) -> Result<String, String> {
    let value = value.trim();
    let low = value.to_lowercase();
    let found = match key {
        "mic" | "voice" | "nixa" | "headphones" | "privacy" | "git_voice" | "agent_voice" => ON_OFF.iter().find(|w| **w == low).map(|w| w.to_string()),
        "listen" => LISTEN.iter().find(|w| **w == low).map(|w| w.to_string()),
        "brain" => brains.iter().find(|(choice, name)| choice == value || name.eq_ignore_ascii_case(value)).map(|(choice, _)| choice.clone()),
        "model" => {
            if models.is_empty() {
                return Err("The model list of this brain could not be read, so the model cannot be checked. Nothing changed.".into());
            }
            models.iter().find(|m| m.id == value || m.name.eq_ignore_ascii_case(value)).map(|m| m.id.clone())
        }
        "effort" => {
            if low.is_empty() || low == "default" {
                Some(String::new())
            } else {
                efforts_of(models, model_now).into_iter().find(|e| e.eq_ignore_ascii_case(value))
            }
        }
        _ => return Err(format!("{key} is not a setting this tool changes.")),
    };
    found.ok_or_else(|| {
        let allowed = choices(key, brains, models, model_now).into_iter().map(|c| if c.is_empty() { "empty for default".to_string() } else { c }).collect::<Vec<_>>();
        format!("{value} is not a choice for {}. Choices: {}. Nothing changed.", label(key), if allowed.is_empty() { "none known".into() } else { allowed.join(", ") })
    })
}

pub fn check_minutes(minutes: Option<f64>) -> Result<f64, String> {
    let minutes = minutes.ok_or("Setting quiet needs minutes. 0 ends quiet time.")?;
    if !minutes.is_finite() || minutes < 0.0 || minutes > QUIET_MAX_MINUTES {
        return Err(format!("minutes must be from 0 to {QUIET_MAX_MINUTES}. Nothing changed."));
    }
    Ok(minutes)
}

pub fn new_shown(key: &str, value: &str, minutes: f64, brains: &[(String, String)]) -> String {
    match key {
        "quiet" => minutes_words(minutes),
        "effort" if value.is_empty() => "default".into(),
        "brain" => brains.iter().find(|(c, _)| c == value).map(|(c, n)| format!("{n} ({c})")).unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

pub fn verified(key: &str, wanted: &str, minutes: f64, after: &Now, at: u64) -> bool {
    match key {
        "mic" => on_word(after.mic) == wanted,
        "voice" => on_word(after.voice) == wanted,
        "listen" => after.listen == wanted,
        "nixa" => on_word(after.nixa) == wanted,
        "headphones" => on_word(after.headphones) == wanted,
        "privacy" => on_word(after.privacy) == wanted,
        "git_voice" => on_word(after.git_voice) == wanted,
        "agent_voice" => on_word(after.agent_voice) == wanted,
        "model" => after.model.as_deref() == Some(wanted),
        "effort" => after.effort.as_deref().unwrap_or_default() == wanted,
        "quiet" => if minutes <= 0.0 { after.quiet_until == 0 } else { after.quiet_until > at },
        _ => false,
    }
}

async fn get(app: &Arc<App>, key: Option<&str>) -> ToolResult {
    if let Some(key) = key
        && !KEYS.contains(&key)
    {
        return Err(format!("{key} is not a known setting. Keys: {}.", KEYS.join(", ")));
    }
    let wanted: Vec<&str> = KEYS.iter().copied().filter(|k| key.is_none_or(|w| w == *k)).collect();
    let now = read(app);
    let at = now_ms();
    let need_models = wanted.iter().any(|k| *k == "model" || *k == "effort");
    let (list, model_error) = if need_models {
        match models(app).await {
            Ok(list) => (list, None),
            Err(error) => (Vec::new(), Some(error)),
        }
    } else {
        (Vec::new(), None)
    };
    let saved = brains(app);
    let rows: Vec<Value> = wanted
        .iter()
        .map(|k| {
            let mut row = json!({"key": k, "label": label(k), "value": shown(&now, k, at), "current": raw(&now, k), "applies": applies(k)});
            if *k == "quiet" {
                row["set_with"] = json!("minutes, 0 ends quiet time");
            } else {
                row["choices"] = json!(choices(k, &saved, &list, now.model.as_deref()));
            }
            if *k == "brain" {
                row["names"] = json!(saved.iter().map(|(c, n)| json!({"choice": c, "name": n})).collect::<Vec<_>>());
            }
            row
        })
        .collect();
    let mut out = json!({"settings": rows, "note": READ_NOTE});
    if let Some(error) = model_error {
        out["model_list_error"] = json!(error);
    }
    Ok(out.to_string())
}

pub(super) async fn switch_brain(
    app: &Arc<App>, choice: &str, approved: &Option<String>, wait: Duration,
) -> Result<(), String> {
    let until = tokio::time::Instant::now() + wait;
    while crate::agent::busy(app) {
        let remaining = until.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("The current chat brain was still busy when the switch timed out.".into());
        }
        tokio::time::sleep(remaining.min(Duration::from_millis(BRAIN_GAP_MS))).await;
    }
    crate::agent::pick_if_idle(app, choice, approved)?;
    crate::server::send_brain(app);
    Ok(())
}

pub fn retry_switch(tries: u32, error: &str) -> bool {
    tries < BRAIN_TRIES && error.contains("busy")
}

fn defer_brain(app: &Arc<App>, choice: String, approved: Option<String>) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut tries = 0;
        let error = loop {
            match switch_brain(&app, &choice, &approved, Duration::from_millis(BRAIN_WAIT_MS)).await {
                Ok(()) => return,
                Err(error) if retry_switch(tries, &error) => tries += 1,
                Err(error) => break error,
            }
        };
        app.info(&format!("The chat brain did not change: {error}"));
        crate::modes::note(&app, "auto-yes", &format!("switch the chat brain to {choice}"), None, Some(format!("the switch did not happen: {error}").as_str()));
        crate::agent::event(&app, &format!("Event: The chat brain switch to {choice} did not happen. {error} Decide the next step yourself; do not ask the user."));
    });
}

async fn apply(app: &Arc<App>, key: &str, wanted: &str, minutes: f64, before: &Now) -> Result<(), String> {
    match key {
        "mic" => crate::voice::handle(app, Command::Mic(wanted == "on")).await,
        "voice" => crate::voice::handle(app, Command::Voice { on: Some(wanted == "on"), name: None }).await,
        "listen" => {
            let mode = Listen::parse(wanted).ok_or("The listen mode is hotkey or always.")?;
            crate::voice::handle(app, Command::Listen(mode)).await;
        }
        "nixa" => crate::settings_api::set_nixa(app, wanted == "on")?,
        "headphones" => {
            crate::settings_api::set_headphones(app, wanted == "on")?;
            crate::settings_api::restart_voice(app);
        }
        "privacy" => crate::settings_api::set_privacy(app, wanted == "on")?,
        "git_voice" => crate::settings_api::set_git_voice(app, wanted == "on")?,
        "agent_voice" => crate::announce::set_on(app, wanted == "on").map_err(|e| e.to_string())?,
        "quiet" => crate::talk::flow::set_quiet_panel(app, minutes),
        "model" => {
            crate::agent::set_model(app, Some(wanted.to_string()), None)?;
            crate::server::send_brain(app);
        }
        "effort" => {
            crate::agent::set_model(app, None, Some(Some(wanted.to_string()).filter(|e| !e.is_empty())))?;
            crate::server::send_brain(app);
        }
        "brain" => defer_brain(app, wanted.to_string(), before.brain.clone()),
        other => return Err(format!("{other} is not a setting this tool changes.")),
    }
    Ok(())
}

async fn set(app: &Arc<App>, args: &Args) -> ToolResult {
    let key = need(args, "key")?;
    if !KEYS.contains(&key.as_str()) {
        return Err(format!("{key} is not a known setting. Keys: {}.", KEYS.join(", ")));
    }
    let before = read(app);
    let saved = brains(app);
    let (wanted, minutes) = if key == "quiet" {
        if args.contains_key("value") {
            return Err("Quiet time takes minutes, not value.".into());
        }
        (String::new(), check_minutes(args.get("minutes").and_then(Value::as_f64))?)
    } else {
        if args.contains_key("minutes") {
            return Err("Only quiet time takes minutes.".into());
        }
        let value = args.get("value").and_then(Value::as_str).ok_or("value must be text. Use empty text explicitly for default effort.")?.trim();
        if value.chars().count() > 200 {
            return Err("Keep setting values within 200 characters.".into());
        }
        if value.is_empty() && key != "effort" {
            return Err(format!("Setting {} needs value.", label(&key)));
        }
        let list = if key == "model" || key == "effort" { models(app).await.unwrap_or_default() } else { Vec::new() };
        if key == "effort" && before.model.is_none() {
            return Err("Pick a model first, then its effort. Nothing changed.".into());
        }
        (check_value(&key, value, &saved, &list, before.model.as_deref())?, 0.0)
    };
    let at = now_ms();
    let old = shown(&before, &key, at);
    let new = new_shown(&key, &wanted, minutes, &saved);
    if key != "quiet" && old == new {
        return Ok(json!({"status": "unchanged", "key": key, "value": old, "reason": "It already has this value. Nothing changed."}).to_string());
    }
    let again = read(app);
    if raw(&again, &key) != raw(&before, &key) || ((key == "model" || key == "effort") && again.brain != before.brain) || (key == "effort" && again.model != before.model) {
        return Err("The setting or its brain changed meanwhile. Nothing was applied. Read it again.".into());
    }
    let applied_at = now_ms();
    let failure = apply(app, &key, &wanted, minutes, &before).await.err();
    let after = read(app);
    let seen = shown(&after, &key, now_ms());
    let (status, reason) = match (&failure, key.as_str()) {
        (Some(error), _) => ("failed", error.clone()),
        (None, "brain") => ("pending", "The chat brain changes after this reply. Read the settings again before you say it changed.".to_string()),
        (None, _) if verified(&key, &wanted, minutes, &after, applied_at) => ("verified", "The new value was read back.".to_string()),
        (None, _) => ("failed", "The new value was not read back. Do not say it changed.".to_string()),
    };
    Ok(json!({"status": status, "key": key, "old": old, "asked": new, "now": seen, "current": raw(&after, &key), "reason": reason}).to_string())
}
