use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Efforts = BTreeMap<String, Vec<String>>;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentModels {
    pub models: Vec<String>,
    pub efforts: Efforts,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoodBad {
    pub good: u64,
    pub bad: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelUsage {
    pub jobs: u64,
    pub at: i64,
    pub capabilities: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelDetails {
    pub company: String,
    pub plan: String,
    pub capabilities: Vec<String>,
    pub capabilities_by: String,
    pub cost: String,
    pub public_score: Option<f64>,
    pub public_source: String,
    pub public_at: i64,
    pub public_names: Vec<String>,
    pub own_score: Option<f64>,
    pub good: u64,
    pub bad: u64,
    pub usage: Option<ModelUsage>,
    pub results: BTreeMap<String, GoodBad>,
    pub secondary: Option<String>,
    pub secondary_auto: bool,
    pub disabled: bool,
    pub pinned: bool,
    pub pins: Vec<String>,
    pub moved_by: String,
    pub moved_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CachedModels {
    pub at: i64,
    pub models: Vec<String>,
    pub efforts: Efforts,
    pub error: String,
    pub details: BTreeMap<String, ModelDetails>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Selection {
    pub agent: String,
    pub named: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelLaunch {
    pub model: String,
    pub effort: String,
    pub task: String,
    pub capabilities: Vec<String>,
    pub selection: Selection,
    pub folder: String,
    pub space: String,
    pub name: String,
    pub member: String,
    pub at: i64,
    pub handled: bool,
    pub result: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PublicRefresh {
    pub at: i64,
    pub attempt_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelCache {
    pub agents: BTreeMap<String, CachedModels>,
    pub ranking: Option<Vec<String>>,
    pub public_refresh: PublicRefresh,
    pub launches: BTreeMap<String, ModelLaunch>,
}

pub const COMPANIES: [&str; 10] = ["Anthropic", "OpenAI", "Google", "Moonshot", "Z.ai", "Cursor", "xAI", "DeepSeek", "Alibaba", "MiniMax"];
pub const PLANS: [&str; 8] = ["Claude plan", "ChatGPT plan", "Antigravity plan", "Google plan", "Cursor plan", "Z.ai plan", "Copilot plan", "Free"];
pub const PROVIDER_AGENTS: [&str; 2] = ["omp", "opencode"];
const EFFORT_ORDER: [&str; 9] = ["default", "config", "none", "minimal", "low", "medium", "high", "xhigh", "max"];

const COMPANY_BY_NAME: [(&[&str], &[&str], &str); 10] = [
    (&["opus", "sonnet", "haiku"], &["claude"], "Anthropic"),
    (&["gpt", "codex", "chatgpt"], &[], "OpenAI"),
    (&[], &["gemini", "gemma"], "Google"),
    (&["kimi"], &[], "Moonshot"),
    (&["glm"], &[], "Z.ai"),
    (&["composer"], &[], "Cursor"),
    (&["grok"], &[], "xAI"),
    (&["deepseek"], &[], "DeepSeek"),
    (&["qwen"], &[], "Alibaba"),
    (&["minimax"], &[], "MiniMax"),
];

const COMPANY_BY_PROVIDER: [(&str, &str); 13] = [
    ("anthropic", "Anthropic"),
    ("openai", "OpenAI"),
    ("openai-codex", "OpenAI"),
    ("google", "Google"),
    ("google-antigravity", "Google"),
    ("google-gemini-cli", "Google"),
    ("zai", "Z.ai"),
    ("kimi-code", "Moonshot"),
    ("moonshot", "Moonshot"),
    ("moonshotai", "Moonshot"),
    ("xai", "xAI"),
    ("deepseek", "DeepSeek"),
    ("cursor", "Cursor"),
];

const COMPANY_BY_AGENT: [(&str, &str); 5] = [("claude", "Anthropic"), ("codex", "OpenAI"), ("kimi", "Moonshot"), ("gemini", "Google"), ("agy", "Google")];

const PLAN_BY_AGENT: [(&str, &str); 6] = [
    ("claude", "Claude plan"),
    ("codex", "ChatGPT plan"),
    ("agy", "Antigravity plan"),
    ("kimi", "Kimi plan"),
    ("cursor", "Cursor plan"),
    ("gemini", "Google plan"),
];

const PLAN_BY_PROVIDER: [(&str, &str); 9] = [
    ("anthropic", "Claude plan"),
    ("openai-codex", "ChatGPT plan"),
    ("zai", "Z.ai plan"),
    ("kimi-code", "Kimi plan"),
    ("google-antigravity", "Antigravity plan"),
    ("google-gemini-cli", "Google plan"),
    ("github-copilot", "Copilot plan"),
    ("cursor", "Cursor plan"),
    ("ollama", "Free"),
];

fn lookup(table: &[(&str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

pub fn model_id(agent: &str, model: &str) -> String {
    format!("{agent}::{model}")
}

pub fn split_id(id: &str) -> Option<(&str, &str)> {
    let (agent, model) = id.split_once("::")?;
    (!agent.is_empty()).then_some((agent, model))
}

pub fn model_value(model: &str) -> String {
    let model = model.trim();
    if model.eq_ignore_ascii_case("default") { String::new() } else { model.to_string() }
}

pub fn provider_of(model: &str) -> String {
    let value = model.trim();
    match value.find('/') {
        Some(at) if at > 0 => value[..at].to_string(),
        _ => String::new(),
    }
}

pub fn providers_of(models: &[String]) -> Vec<String> {
    let mut out: Vec<String> = models.iter().map(|m| provider_of(m)).filter(|p| !p.is_empty()).collect();
    out.sort();
    out.dedup();
    out
}

pub fn is_free_model(agent: &str, model: &str) -> bool {
    let model = model.trim();
    agent == "opencode" && (model.ends_with("-free") || model == "opencode/big-pickle")
}

pub fn is_zen_model(agent: &str, model: &str) -> bool {
    agent == "opencode" && model.trim().starts_with("opencode/")
}

pub fn is_kimi_model(agent: &str, model: &str) -> bool {
    agent == "kimi" || model.trim().starts_with("kimi-code/")
}

fn digit_after(name: &str, first: char) -> bool {
    let mut chars = name.chars();
    chars.next() == Some(first) && chars.next().is_some_and(|c| c.is_ascii_digit())
}

fn company_of_name(name: &str) -> Option<&'static str> {
    for (starts, contains, company) in COMPANY_BY_NAME {
        let digit = match company {
            "OpenAI" => digit_after(name, 'o'),
            "Moonshot" => digit_after(name, 'k'),
            _ => false,
        };
        if digit || starts.iter().any(|s| name.starts_with(s)) || contains.iter().any(|c| name.contains(c)) {
            return Some(company);
        }
    }
    None
}

pub fn guess_company(agent: &str, model: &str) -> Option<String> {
    let value = model_value(model);
    let name = value.rsplit('/').next().unwrap_or_default().to_lowercase();
    if !name.is_empty()
        && let Some(company) = company_of_name(&name)
    {
        return Some(company.to_string());
    }
    if let Some(company) = lookup(&COMPANY_BY_PROVIDER, &provider_of(&value)) {
        return Some(company.to_string());
    }
    if !value.is_empty() && PROVIDER_AGENTS.contains(&agent) {
        return None;
    }
    lookup(&COMPANY_BY_AGENT, agent).map(str::to_string)
}

pub fn guess_plan(agent: &str, model: &str) -> Option<String> {
    if let Some(plan) = lookup(&PLAN_BY_AGENT, agent) {
        return Some(plan.to_string());
    }
    let value = model_value(model);
    if is_free_model(agent, &value) {
        return Some("Free".into());
    }
    lookup(&PLAN_BY_PROVIDER, &provider_of(&value)).map(str::to_string)
}

pub fn efforts_for(list: &CachedModels, model: &str) -> Vec<String> {
    list.efforts.get(model).or_else(|| list.efforts.get("*")).cloned().unwrap_or_else(|| vec!["default".into()])
}

fn effort_place(effort: &str) -> usize {
    EFFORT_ORDER.iter().position(|e| *e == effort).unwrap_or(EFFORT_ORDER.len())
}

pub fn ordered_efforts(efforts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for effort in efforts {
        if !out.contains(effort) {
            out.push(effort.clone());
        }
    }
    out.sort_by(|a, b| effort_place(a).cmp(&effort_place(b)).then_with(|| a.cmp(b)));
    out
}

pub fn first_effort(efforts: &[String]) -> String {
    ordered_efforts(efforts).into_iter().next().unwrap_or_else(|| "default".into())
}
