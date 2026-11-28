use super::declutter;
use super::{BROWSER, EYES, HANDS, READS, SEEN_MS, Seen};
use crate::tools::Args;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

pub const MAX_TEXT: usize = 2000;
pub const MAX_URL: usize = 2000;
pub const MAX_QUESTION: usize = 1000;
pub const MAX_OFFSET: u64 = 600;
pub const TEXT_ROLES: [&str; 3] = ["AXTextField", "AXTextArea", "AXComboBox"];
pub const BROWSER_APPS: [&str; 5] = ["Safari", "Google Chrome", "Google Chrome Canary", "Firefox", "Microsoft Edge"];

static CAPTURE_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").expect("capture id"));

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub tool: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Flags {
    pub may_read: bool,
}

pub fn is_read(action: &str) -> bool {
    READS.contains(&action)
}

pub fn is_capture_id(text: &str) -> bool {
    CAPTURE_ID.is_match(text)
}

fn keys_for(action: &str) -> &'static [&'static str] {
    match action {
        "windows" => &["action"],
        "open" => &["action", "url"],
        "shot" => &["action", "name"],
        "vision" => &["action", "capture", "name", "question", "rubric"],
        "read" => &["action", "window", "offset"],
        "type" => &["action", "observation", "element", "text"],
        _ => &["action", "observation", "element"],
    }
}

fn actions_for(tool: &str) -> &'static [&'static str] {
    if tool == EYES {
        &["windows", "read", "shot", "vision"]
    } else if tool == BROWSER {
        &["open", "read", "click", "type"]
    } else if tool == HANDS {
        &["click", "type"]
    } else {
        &[]
    }
}

fn open_step(tool: &str, args: &Args) -> Result<Step, String> {
    let raw = args.get("url").and_then(Value::as_str).map(str::trim).unwrap_or("");
    let rest = raw.strip_prefix("https://").or_else(|| raw.strip_prefix("http://")).ok_or("url must be an http:// or https:// URL without credentials.")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if raw.chars().count() > MAX_URL || host.is_empty() || host.contains('@') || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("url must be an http:// or https:// URL without credentials.".into());
    }
    crate::web::safety::public_url(raw)?;
    Ok(Step { tool: tool.into(), action: "open".into(), url: Some(raw.to_string()), ..Step::default() })
}

fn window_name(value: Option<&Value>) -> Result<Option<String>, String> {
    let Some(value) = value else { return Ok(None) };
    let text = value.as_str().filter(|t| t.chars().count() <= 80 && !t.chars().any(char::is_control));
    let text = text.ok_or("Name one Nixa window in plain words, for example main, sidebar, popup, Agent map or Tasks.")?;
    Ok(Some(text.trim().to_string()))
}

pub fn clean_question(value: Option<&Value>) -> Result<String, String> {
    let text = value.and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).ok_or("question is needed: say what to look for in the picture.")?;
    if text.chars().count() > MAX_QUESTION {
        return Err(format!("Keep the question under {MAX_QUESTION} letters."));
    }
    if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("The question has control characters. Use plain words.".into());
    }
    Ok(text.to_string())
}

pub fn rubric_of(value: Option<&Value>) -> Result<Option<String>, String> {
    let Some(value) = value else { return Ok(None) };
    value
        .as_str()
        .filter(|r| declutter::RUBRICS.contains(r))
        .map(|r| Some(r.to_string()))
        .ok_or_else(|| format!("rubric must be one of: {}.", declutter::RUBRICS.join(", ")))
}

fn read_step(tool: &str, args: &Args) -> Result<Step, String> {
    let window = args.get("window").cloned().ok_or(if tool == BROWSER { "Choose a browser window id from nixa_eyes windows." } else { "Choose a window id from windows, or screen, or nixa." })?;
    let fine = match &window {
        Value::String(s) => tool != BROWSER && (s == "screen" || s == "nixa"),
        Value::Number(n) => n.as_u64().is_some_and(|n| n > 0 && n <= i64::MAX as u64),
        _ => false,
    };
    if !fine {
        return Err(if tool == BROWSER { "Choose a browser window id from nixa_eyes windows." } else { "Choose a window id from windows, or screen, or nixa." }.into());
    }
    let offset = match args.get("offset") {
        None => 0,
        Some(v) => v.as_u64().filter(|n| *n <= MAX_OFFSET).ok_or(format!("The control offset must be between 0 and {MAX_OFFSET}."))?,
    };
    Ok(Step { tool: tool.into(), action: "read".into(), window: Some(window), offset: Some(offset), ..Step::default() })
}

fn input_step(tool: &str, action: &str, args: &Args, seen: &HashMap<String, Seen>, now: u64) -> Result<Step, String> {
    let observation = args.get("observation").and_then(Value::as_str).unwrap_or("").to_string();
    let saved = seen.get(&observation).filter(|s| now.saturating_sub(s.at) <= SEEN_MS).ok_or("Read the window again; this observation is missing or too old.")?;
    let target = saved.value.get("target").cloned().unwrap_or(Value::Null);
    let app = target.get("app").and_then(Value::as_str).unwrap_or("");
    if tool == BROWSER && !BROWSER_APPS.contains(&app) {
        return Err("nixa_browser input works only on an observed browser window (Safari, Chrome, Firefox or Edge).".into());
    }
    let wanted = args.get("element").and_then(Value::as_str).unwrap_or("");
    let element = saved.value.get("elements").and_then(Value::as_array).and_then(|list| list.iter().find(|e| e.get("id").and_then(Value::as_str) == Some(wanted))).cloned();
    let element = element.filter(|e| e.get("enabled").and_then(Value::as_bool) == Some(true));
    let element = element.ok_or("Choose an enabled control from the observation.")?;
    let role = element.get("role").and_then(Value::as_str).unwrap_or("");
    let typed = if action == "type" {
        if !TEXT_ROLES.contains(&role) {
            return Err("Typing works only in observed text fields.".into());
        }
        let text = args.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or("text is needed.")?;
        if text.chars().count() > MAX_TEXT {
            return Err(format!("Typing is limited to {MAX_TEXT} characters."));
        }
        Some(text.to_string())
    } else {
        None
    };
    Ok(Step { tool: tool.into(), action: action.into(), observation: Some(observation), target: Some(target), element: Some(element), text: typed, ..Step::default() })
}

pub fn prepare(tool: &str, args: &Args, flags: Flags, seen: &HashMap<String, Seen>, now: u64) -> Result<Step, String> {
    if !flags.may_read {
        return Err("Privacy is on: use a local brain, or the user turns privacy off, before screen text comes back.".into());
    }
    let action = args.get("action").and_then(Value::as_str).unwrap_or("").to_string();
    let allowed = actions_for(tool);
    if !allowed.contains(&action.as_str()) || args.keys().any(|k| !keys_for(&action).contains(&k.as_str())) {
        return Err("This desktop step or argument is not supported. There is no shell, script, key or file step.".into());
    }
    match action.as_str() {
        "windows" => Ok(Step { tool: tool.into(), action, ..Step::default() }),
        "open" => open_step(tool, args),
        "shot" => Ok(Step { tool: tool.into(), action, name: window_name(args.get("name"))?, ..Step::default() }),
        "vision" => {
            let rubric = rubric_of(args.get("rubric"))?;
            let question = match (&rubric, args.get("question")) {
                (Some(_), None) => declutter::QUESTION.to_string(),
                (_, value) => clean_question(value)?,
            };
            let question = Some(question);
            match (args.get("capture"), args.get("name")) {
                (Some(capture), None) => {
                    let capture = capture.as_str().filter(|c| is_capture_id(c)).ok_or("capture must be an id from a nixa_eyes shot or read. File paths are not taken.")?;
                    Ok(Step { tool: tool.into(), action, capture: Some(capture.into()), question, rubric, ..Step::default() })
                }
                (None, Some(name)) => Ok(Step { tool: tool.into(), action, name: window_name(Some(name))?, question, rubric, ..Step::default() }),
                _ => Err("Give either capture (the id from a shot) or name (a Nixa window to capture first), not both and not neither.".into()),
            }
        }
        "read" => read_step(tool, args),
        _ => input_step(tool, &action, args, seen, now),
    }
}

pub fn app_scoped(step: &Step) -> bool {
    let named = step.name.as_deref().is_some_and(|n| !n.trim().is_empty());
    step.tool == EYES
        && ((step.action == "shot" && named)
            || (step.action == "read" && step.window.as_ref().and_then(Value::as_str) == Some("nixa"))
            || (step.action == "vision" && step.capture.is_none() && named))
}
