use super::map::{Level, MAX_KEY, VoiceMap, cut, normalize_agent, voice_key};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

pub const FALLBACK_VOICE: &str = "Pippa";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Ctx {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl Ctx {
    pub fn of(value: &Value) -> Ctx {
        let pick = |key: &str| value.get(key).and_then(Value::as_str).map(|v| cut(v, MAX_KEY)).filter(|v| !v.is_empty());
        Ctx { terminal: pick("terminal"), model: pick("model"), agent: pick("agent"), space: pick("space"), kind: pick("kind") }
    }

    pub fn nixa(kind: &str) -> Ctx {
        Ctx { agent: Some("nixa".into()), kind: Some(kind.into()), ..Ctx::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    Terminal,
    Model,
    Agent,
    Space,
    Kind,
    Default,
    Automatic,
    BuiltIn,
}

impl Tier {
    pub fn number(self) -> u8 {
        match self {
            Tier::Terminal => 1,
            Tier::Model | Tier::Agent => 2,
            Tier::Space => 3,
            Tier::Kind => 4,
            Tier::Default => 5,
            Tier::Automatic => 6,
            Tier::BuiltIn => 7,
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Tier::Terminal | Tier::Agent => "agent",
            Tier::Model => "model",
            Tier::Space => "space",
            Tier::Kind => "kind of speech",
            Tier::Default => "default voice",
            Tier::Automatic => "automatic agent voice",
            Tier::BuiltIn => "built-in voice",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rule {
    pub tier: Tier,
    pub level: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resolved {
    pub voice: String,
    pub rule: Rule,
}

#[derive(Debug, Clone, Default)]
pub struct Deps {
    pub known: Option<Vec<String>>,
    pub fallback: Vec<String>,
    pub nixa_voice: String,
    pub numbers: HashMap<String, u32>,
    pub personas: HashMap<String, String>,
}

pub fn builtin(deps: &Deps, agent: &str) -> String {
    if agent == "nixa" && !deps.nixa_voice.trim().is_empty() {
        return deps.nixa_voice.clone();
    }
    nixa_voice::voice_for(agent).to_string()
}

fn canonical(known: Option<&[String]>, voice: &str) -> Option<String> {
    match known {
        Some(list) if !list.is_empty() => list.iter().find(|name| name.eq_ignore_ascii_case(voice)).cloned(),
        _ => Some(voice.to_string()),
    }
}

fn candidates(map: &VoiceMap, ctx: &Ctx) -> Vec<(Tier, String, Option<String>)> {
    let key = |level: Level, value: &Option<String>| value.as_deref().map(|v| voice_key(level, v)).unwrap_or_default();
    let terminal = key(Level::Terminal, &ctx.terminal);
    let model = key(Level::Model, &ctx.model);
    let agent = key(Level::Agent, &ctx.agent);
    let space = key(Level::Space, &ctx.space);
    let kind = key(Level::Kind, &ctx.kind);
    let found = |level: Level, k: &str| map.get(level, k).cloned();
    vec![
        (Tier::Terminal, terminal.clone(), found(Level::Terminal, &terminal)),
        (Tier::Model, model.clone(), found(Level::Model, &model)),
        (Tier::Agent, agent.clone(), found(Level::Agent, &agent)),
        (Tier::Space, space.clone(), found(Level::Space, &space)),
        (Tier::Kind, kind.clone(), found(Level::Kind, &kind)),
        (Tier::Kind, "chat".into(), if kind == "deep" { found(Level::Kind, "chat") } else { None }),
        (Tier::Default, String::new(), map.default.clone()),
    ]
}

fn skipped(broken: &[String]) -> String {
    if broken.is_empty() { String::new() } else { format!("; {} is not available", broken.join(", ")) }
}

pub fn terminal_number(ctx: &Ctx, numbers: &HashMap<String, u32>) -> (Ctx, Option<u32>) {
    let mut ctx = ctx.clone();
    let Some(terminal) = ctx.terminal.as_deref().map(|t| voice_key(Level::Terminal, t)).filter(|t| !t.is_empty()) else {
        return (ctx, None);
    };
    let low = terminal.to_lowercase();
    let bare = ["terminal", "term", "#"].iter().find_map(|p| low.strip_prefix(p)).unwrap_or(&low).trim();
    let spoken = (!bare.is_empty() && bare.chars().all(|c| c.is_ascii_digit())).then(|| bare.parse::<u32>().ok()).flatten();
    let number = numbers.get(&terminal).copied().or(spoken).filter(|n| *n > 0);
    if spoken.is_some()
        && let Some(address) = numbers.iter().filter(|(_, n)| Some(**n) == number).map(|(a, _)| a.clone()).min()
    {
        ctx.terminal = Some(address);
    } else {
        ctx.terminal = Some(terminal);
    }
    (ctx, number)
}

pub fn resolve(map: &VoiceMap, ctx: &Ctx, deps: &Deps) -> Resolved {
    let asked = ctx.terminal.as_deref().map(|t| voice_key(Level::Terminal, t));
    let (ctx, number) = terminal_number(ctx, &deps.numbers);
    let known = deps.known.as_deref();
    let mut broken: Vec<String> = Vec::new();
    let persona = [ctx.terminal.as_deref(), asked.as_deref()].into_iter().flatten().find_map(|t| deps.personas.get(t));
    let mut own_checked = false;
    for (tier, key, voice) in candidates(map, &ctx) {
        if !own_checked && matches!(tier, Tier::Space | Tier::Kind | Tier::Default) {
            own_checked = true;
            if let Some(name) = persona.and_then(|v| canonical(known, v)) {
                let text = format!("Automatic: own voice of this agent{}", skipped(&broken));
                return Resolved { voice: name, rule: Rule { tier: Tier::Automatic, level: Tier::Automatic.number(), key: ctx.terminal.clone(), text } };
            }
        }
        let Some(voice) = voice else { continue };
        let Some(name) = canonical(known, &voice) else {
            broken.push(voice);
            continue;
        };
        let place = if key.is_empty() { tier.words().to_string() } else { format!("{} {key}", tier.words()) };
        let text = format!("Level {}: {place}{}", tier.number(), skipped(&broken));
        return Resolved { voice: name, rule: Rule { tier, level: tier.number(), key: (!key.is_empty()).then_some(key), text } };
    }
    if !own_checked && let Some(name) = persona.and_then(|v| canonical(known, v)) {
        let text = format!("Automatic: own voice of this agent{}", skipped(&broken));
        return Resolved { voice: name, rule: Rule { tier: Tier::Automatic, level: Tier::Automatic.number(), key: ctx.terminal.clone(), text } };
    }
    if let Some(n) = number {
        let list = match known {
            Some(list) if !list.is_empty() => list,
            _ => deps.fallback.as_slice(),
        };
        if !list.is_empty() {
            let index = (n as usize - 1) % list.len();
            let text = format!("Automatic: agent {n} uses voice {}{}", index + 1, skipped(&broken));
            return Resolved { voice: list[index].clone(), rule: Rule { tier: Tier::Automatic, level: Tier::Automatic.number(), key: ctx.terminal.clone(), text } };
        }
    }
    let agent = ctx.agent.as_deref().map(normalize_agent).filter(|a| !a.is_empty()).unwrap_or_else(|| "nixa".into());
    let voice = Some(builtin(deps, &agent)).filter(|v| !v.trim().is_empty()).unwrap_or_else(|| FALLBACK_VOICE.into());
    let text = format!("Built-in voice for {agent}{}", skipped(&broken));
    Resolved { voice, rule: Rule { tier: Tier::BuiltIn, level: Tier::BuiltIn.number(), key: None, text } }
}
