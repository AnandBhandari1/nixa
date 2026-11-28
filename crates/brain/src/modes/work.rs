use crate::settings_api::work::{Hold, Topic, TopicKind, Work};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;

pub const PASS_MS: u64 = 3 * 60 * 1000;
pub const TOPIC_MS: u64 = 10 * 60 * 1000;
pub const HOLD_MS: u64 = 30 * 60 * 1000;
pub const FILE: &str = "work-state.json";

pub const ACTIONS: [&str; 17] = [
    "send-work", "start-agent", "start-task", "team-run", "team-step", "hand-off", "schedule-run", "auto-start", "backup-start",
    "fix", "next-step", "close-job", "check", "answer-prompt", "app", "debate", "safe",
];

const NEW_WORK: [&str; 9] = ["send-work", "start-agent", "start-task", "team-run", "team-step", "hand-off", "schedule-run", "auto-start", "backup-start"];
const FREE: [&str; 2] = ["safe", "debate"];

fn new_work(action: &str) -> bool {
    NEW_WORK.contains(&action)
}

pub fn is_action(action: &str) -> bool {
    ACTIONS.contains(&action)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub reason: String,
    pub item: String,
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateInput {
    pub pass: bool,
    pub hold_over: bool,
}

fn held(reason: &str, item: String, why: String) -> Option<Held> {
    Some(Held { reason: reason.to_string(), item, why })
}

pub const HELD_NEXT: &str = "Nothing was run. Do not wait for the user and do not report a wait. Find another way now: do the parts that are not held, end or clear what blocks the step and try again, save new work as a task for later, or ask nixa_deep_ask for the next step. Record what you chose.";

pub fn hold_why() -> String {
    format!("Hold is on, so no new work starts. Agent questions, checks and fixes still run, and the hold ends by itself {} minutes after it began.", HOLD_MS / 60_000)
}

pub fn hold_over(at: u64, now: u64) -> bool {
    now.saturating_sub(at) > HOLD_MS
}

pub fn gate(work: &Work, action: &str, input: &GateInput) -> Option<Held> {
    if FREE.contains(&action) || input.pass {
        return None;
    }
    match &work.hold {
        Some(hold) if new_work(action) && !input.hold_over => held("keyword: hold", format!("keyword:hold:{}", hold.alias), hold_why()),
        _ => None,
    }
}

pub fn passed_by(work: &Work, action: &str, input: &GateInput) -> Option<Held> {
    if FREE.contains(&action) || input.pass || gate(work, action, input).is_some() {
        return None;
    }
    if let Some(hold) = &work.hold {
        return held("keyword: hold", format!("keyword:hold:{}", hold.alias), hold_why());
    }
    let topic = work.topic.as_ref().filter(|_| new_work(action))?;
    let kind = topic_kind(topic.kind);
    held(&format!("keyword: {kind}"), format!("keyword:{kind}:{}", topic.alias), format!("{kind} was on"))
}

pub fn topic_kind(kind: TopicKind) -> &'static str {
    match kind {
        TopicKind::Discuss => "discuss",
        TopicKind::Plan => "plan",
    }
}

static DECIDE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bdecid").expect("pattern"));
static BRAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bdeep\s*brain\b|\bbrain\b").expect("pattern"));
static LET_HIM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:ask|tell|let)\s+(?:him|it|them)\b").expect("pattern"));
static ASKS_WORK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:(?:tell|send|give|ask)\s+(?:it\s+to\s+)?(?:him|her|it|them|the agent|claude|codex|the deep brain|deep brain)|assign|start|build|install|restart|do (?:it|that|this)|go ahead|proceed)\b").expect("pattern")
});
static START_AGENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^Start .+ with the first message").expect("pattern"));
pub static YES_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^Needs your yes:\s*").expect("pattern"));
static PRESS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^Press ").expect("pattern"));
static APP_STEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:Interrupt|Run) ").expect("pattern"));

pub fn hands_to_deep(line: &str) -> bool {
    let text = line.to_lowercase();
    DECIDE.is_match(&text) && (BRAIN.is_match(&text) || LET_HIM.is_match(&text))
}

pub fn asks_work(line: &str) -> bool {
    let text = line.trim();
    if text.is_empty() || text.ends_with('?') {
        return false;
    }
    ASKS_WORK.is_match(text)
}

pub fn topic_over(work: &Work, now: u64) -> bool {
    work.topic.as_ref().is_some_and(|t| now.saturating_sub(t.at) > TOPIC_MS)
}

pub fn held_text(held: &Held, what: &str) -> String {
    let what = what.trim().trim_end_matches(['.', '?', '!']);
    format!("Not started yet ({}): {what}. Nixa goes on with the other work.", held.reason)
}

pub fn action_of_text(description: &str, checking: bool) -> &'static str {
    let text = YES_HEAD.replace(description.trim(), "");
    if START_AGENT.is_match(&text) {
        return "start-agent";
    }
    if text.starts_with("Send \"") {
        if text.contains(" to answer \"") {
            return "answer-prompt";
        }
        return if checking { "fix" } else { "send-work" };
    }
    if PRESS.is_match(&text) {
        return "answer-prompt";
    }
    if APP_STEP.is_match(&text) {
        return "app";
    }
    "send-work"
}

pub fn work_line(work: &Work, clock: fn(u64) -> String) -> String {
    let topic = work
        .topic
        .as_ref()
        .map(|t| {
            let about = if t.text.is_empty() { String::new() } else { format!(": {}", crate::util::clip(&t.text, 80)) };
            format!("Keyword {} since {}{about}.", topic_kind(t.kind), clock(t.at))
        })
        .unwrap_or_default();
    let hold = work.hold.as_ref().map(|h| format!("Hold on since {}.", clock(h.at))).unwrap_or_default();
    [topic, hold].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    pub topic: Option<Topic>,
    pub hold: Option<Hold>,
}

fn topic_of(value: &Value) -> Option<Topic> {
    let kind = match value.get("kind")?.as_str()? {
        "discuss" => TopicKind::Discuss,
        "plan" => TopicKind::Plan,
        _ => return None,
    };
    Some(Topic { kind, text: value.get("text")?.as_str()?.to_string(), at: value.get("at")?.as_u64()?, alias: value.get("alias")?.as_str()?.to_string() })
}

fn hold_of(value: &Value) -> Option<Hold> {
    Some(Hold { at: value.get("at")?.as_u64()?, alias: value.get("alias")?.as_str()?.to_string() })
}

pub fn parse_saved(text: Option<&str>) -> Saved {
    let Some(value) = text.and_then(|t| serde_json::from_str::<Value>(t).ok()) else { return Saved::default() };
    Saved {
        topic: value.get("topic").and_then(topic_of),
        hold: value.get("hold").and_then(hold_of),
    }
}

pub fn saved_text(saved: &Saved) -> String {
    let mut out = json!({});
    if let Some(map) = out.as_object_mut() {
        if let Some(topic) = &saved.topic {
            map.insert("topic".into(), json!({"kind": topic_kind(topic.kind), "text": topic.text, "at": topic.at, "alias": topic.alias}));
        }
        if let Some(hold) = &saved.hold {
            map.insert("hold".into(), json!({"at": hold.at, "alias": hold.alias}));
        }
    }
    serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".into()) + "\n"
}
