use std::collections::{HashMap, HashSet};

pub const QUIET_MS: u64 = 15 * 60 * 1000;
pub const READ_MS: u64 = 30 * 1000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub read_at: u64,
    pub agent: String,
    pub key: Option<String>,
    pub since: u64,
    pub warned: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Memory {
    pub seen: HashMap<String, Seen>,
}

pub struct Look<'a> {
    pub terminal: &'a str,
    pub number: Option<u32>,
    pub agent: &'a str,
    pub name: &'a str,
    pub tab: &'a str,
    pub working: bool,
    pub screen: &'a str,
}

pub const QUIET_AUTO: &str = "It may be stuck. Do not wait for the user: read its screen with nixa_term_read, then send a nudge or a fix, interrupt and restart it, give the step to another agent, or ask the deep brain with nixa_deep_ask. Record what you chose in one short line.";

fn spinner(c: char) -> bool {
    ('\u{2800}'..='\u{28ff}').contains(&c) || "✻✶✳✢✽·∙●◐◓◑◒◴◵◶◷○◌◎".contains(c)
}

pub fn quiet_key(screen: &str) -> String {
    let lines: Vec<&str> = screen.split('\n').collect();
    let body = if lines.len() > 20 { &lines[..lines.len() - 10] } else { &lines[..] };
    let text: String = body.join("\n").chars().filter(|c| !c.is_ascii_digit() && !spinner(*c)).collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn quiet_note(look: &Look, minutes: u64) -> String {
    let term = match look.number {
        Some(n) => format!("terminal {n} ({})", look.terminal),
        None => format!("terminal {}", look.terminal),
    };
    let tab = if look.tab.is_empty() { look.name } else { look.tab };
    format!(
        "Team: The {} in tab \"{tab}\", {term} is working, but its screen has not changed for {minutes} minutes. {}",
        look.name,
        QUIET_AUTO
    )
}

pub fn due(memory: &Memory, terminal: &str, now: u64, read_ms: u64) -> bool {
    memory.seen.get(terminal).is_none_or(|s| now.saturating_sub(s.read_at) >= read_ms)
}

pub fn check(memory: &mut Memory, look: &Look, now: u64, quiet_ms: u64) -> Option<String> {
    let item = memory.seen.entry(look.terminal.to_string()).or_insert_with(|| Seen { read_at: now, agent: look.agent.to_string(), key: None, since: now, warned: false });
    item.read_at = now;
    item.agent = look.agent.to_string();
    if !look.working {
        item.key = None;
        item.since = now;
        item.warned = false;
        return None;
    }
    let key = quiet_key(look.screen);
    if item.key.as_deref() != Some(key.as_str()) {
        item.key = Some(key);
        item.since = now;
        item.warned = false;
        return None;
    }
    if !item.warned && now.saturating_sub(item.since) >= quiet_ms {
        item.warned = true;
        let minutes = (now - item.since + 30_000) / 60_000;
        return Some(quiet_note(look, minutes));
    }
    None
}

pub fn keep(memory: &mut Memory, alive: &HashSet<String>) {
    memory.seen.retain(|terminal, _| alive.contains(terminal));
}
