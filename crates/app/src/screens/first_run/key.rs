use crate::screens::settings::parts::chat_brain::{kind_title, parse_own};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Brain {
    Loading,
    Down(String),
    Missing,
    Ready { name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Saving {
    Idle,
    Checking,
    Failed(String),
}

pub const KINDS: [&str; 2] = ["anthropic", "openai"];

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn parse(brains: &Value, health: &Value) -> Brain {
    let Some(choice) = text(brains, "choice") else {
        return Brain::Missing;
    };
    let own = parse_own(brains);
    let name = health
        .get("brain")
        .and_then(|b| text(b, "name"))
        .or_else(|| own.iter().find(|b| b.choice == choice).map(|b| b.name.clone()))
        .unwrap_or(choice);
    Brain::Ready { name }
}

pub fn unique_name(kind: &str, brains: &Value) -> String {
    let taken: Vec<String> = parse_own(brains).iter().map(|b| b.name.to_lowercase()).collect();
    let base = kind_title(kind).to_string();
    if !taken.contains(&base.to_lowercase()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken.contains(&name.to_lowercase()))
        .unwrap_or(base)
}

pub fn body(kind: &str, name: &str, key: &str) -> Value {
    json!({"kind": kind, "name": name, "baseUrl": "", "apiKey": key.trim()})
}

pub fn key_problem(key: &str) -> Option<&'static str> {
    let key = key.trim();
    if key.is_empty() {
        return Some("Paste your API key first.");
    }
    if key.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'') {
        return Some("The key has a space or a quote in it. Paste it again.");
    }
    None
}

pub fn key_help(kind: &str) -> &'static str {
    match kind {
        "anthropic" => "Make a key in your Anthropic account (console.anthropic.com).",
        _ => "Make a key in your OpenAI account (platform.openai.com).",
    }
}
