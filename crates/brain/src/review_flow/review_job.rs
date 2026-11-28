use super::checks::{ChecksOutcome, short_checks};
use crate::tools::{Args, ToolResult, schema, spec};
use crate::turns::ToolSpec;
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub const NAME: &str = "review_job";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Done,
    NeedsUser,
}

impl Outcome {
    pub fn parse(word: &str) -> Option<Outcome> {
        match word.trim() {
            "done" => Some(Outcome::Done),
            "needs-user" => Some(Outcome::NeedsUser),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::NeedsUser => "needs-user",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Review {
    pub outcome: Outcome,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Check {
    pub checking: HashSet<String>,
    pub outcomes: HashMap<String, ChecksOutcome>,
    pub reviews: HashMap<String, Review>,
    pub deep_first: HashSet<String>,
    pub picks: HashMap<String, String>,
}

impl Check {
    pub fn begin(&mut self, terminal: &str, deep_first: bool) {
        self.checking.insert(terminal.to_string());
        self.reviews.remove(terminal);
        self.picks.remove(terminal);
        if deep_first {
            self.deep_first.insert(terminal.to_string());
        } else {
            self.deep_first.remove(terminal);
        }
    }

    pub fn end(&mut self, terminal: &str) -> Option<Review> {
        self.checking.remove(terminal);
        self.deep_first.remove(terminal);
        self.picks.remove(terminal);
        self.outcomes.remove(terminal);
        self.reviews.remove(terminal)
    }
}

pub fn spec_of() -> ToolSpec {
    spec(
        NAME,
        "Record done or needs-user for a terminal in the current Check. terminal accepts its space-agent-number id (like nixa-claude-12), an older space-agent-xxxxxx id or a legacy id like m7. It also accepts a displayed number (11) or label (terminal 11). done requires passing checks and proof. The job never waits for the user: Nixa or the deep brain decides every choice, and a step that physically needs the user (a sign-in, a key, or a test on the user's own device) becomes a note for the user while the job goes on another way. The result takes effect after the check turn succeeds.",
        schema(
            json!({
                "terminal": {"type": ["string", "integer"], "description": "The checked terminal's id or displayed number, for example demo-codex-12, older id demo-codex-a1b2c3, legacy id m7, 11, or terminal 11."},
                "outcome": {"type": "string", "enum": ["done", "needs-user"]},
                "reason": {"type": "string"}
            }),
            &["terminal", "outcome", "reason"],
        ),
    )
}

static NUMBERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:terminal\s+)?#?(\d+)$").expect("pattern"));
static DEEP_WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bdeep brain\b").expect("pattern"));
static DEEP_ANSWER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bd\d{1,6}\b").expect("pattern"));
static USER_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:log ?in|sign[- ]?in|password|(?:api|ssh|access|license|secret) key|token|2fa|one[- ]time code|own (?:device|phone|computer)|(?:device|phone) test)\b").expect("pattern")
});
static PICK_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*(?:option\s*)?([1-4])\b").expect("pattern"));
static SEND_PICK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*(?:(?:option\s*)?[1-4]\s*[).:]?\s*)?(?:send|next step|next\b|fix\b)").expect("pattern"));
static FIX_PICK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*(?:option\s*)?(?:[1-4][).:]?\s*)?(?:needs a fix|send\b|next step|next\b|fix\b)").expect("pattern"));

pub const DEEP_FIRST_REFUSED: &str = "This choice is not the user's. Ask the deep brain with nixa_deep_ask and act on its pick, or, when Nixa decides, pick the recommended or safest option yourself and act on it. Push, deploy, publish, production and money choices are decided the same way. Record done with the proof, or send the agent the next step with nixa_term_send.";
pub const PHYSICAL_STEP: &str = "The job does not wait for the user, even for a step that physically needs the user. Write that step for the user as a task note with nixa_task action comment, ask the deep brain with nixa_deep_ask for another way (a mock, a saved key, or skip that step), then send the agent the next step with nixa_term_send, or record done and name the note in the reason.";
pub const FIX_PICKED: &str = "The deep brain picked a fix or the next step for this Check, so the job is not waiting for the user. Send the agent the next step with nixa_term_send, then record done or keep the job open.";

pub fn deep_said(reason: &str) -> bool {
    DEEP_WORDS.is_match(reason) && DEEP_ANSWER.is_match(reason)
}

pub fn user_only(reason: &str) -> bool {
    USER_ONLY.is_match(reason)
}

pub fn pick_number(pick: &str) -> Option<u32> {
    PICK_NUMBER.captures(pick).and_then(|c| c[1].parse().ok())
}

pub fn send_pick(pick: &str) -> bool {
    SEND_PICK.is_match(pick)
}

pub fn outcome_refused(pick: Option<&str>, outcome: Outcome) -> Option<&'static str> {
    let pick = pick?;
    if outcome != Outcome::NeedsUser {
        return None;
    }
    let number = pick_number(pick);
    (number == Some(2) || number == Some(3) || send_pick(pick) || FIX_PICK.is_match(pick)).then_some(FIX_PICKED)
}

fn given_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) if n.is_u64() => n.to_string(),
        _ => String::new(),
    }
}

pub fn record(check: &mut Check, numbers: &HashMap<String, u32>, assistant: &str, args: &Args) -> ToolResult {
    let given = given_of(args.get("terminal"));
    let matches: Vec<String> = if check.checking.contains(&given) {
        vec![given.clone()]
    } else if let Some(found) = NUMBERED.captures(&given) {
        let wanted: u32 = found[1].parse().unwrap_or(0);
        let mut list: Vec<String> = check.checking.iter().filter(|t| numbers.get(*t) == Some(&wanted)).cloned().collect();
        list.sort();
        list
    } else {
        Vec::new()
    };
    if matches.len() != 1 {
        let mut current: Vec<String> = check
            .checking
            .iter()
            .map(|t| format!("terminal {} ({t})", numbers.get(t).map(|n| n.to_string()).unwrap_or_else(|| "?".into())))
            .collect();
        current.sort();
        let current = if current.is_empty() { "none".to_string() } else { current.join(", ") };
        return Err(format!("Review a current Check terminal by id or number. Current Check terminals: {current}."));
    }
    let terminal = matches[0].clone();
    let reason = args.get("reason").and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let outcome = args.get("outcome").and_then(Value::as_str).and_then(Outcome::parse);
    let Some(outcome) = outcome.filter(|_| !reason.is_empty()) else {
        return Err("Give outcome done or needs-user and the proof or reason.".into());
    };
    if outcome == Outcome::Done
        && let Some(ran) = check.outcomes.get(&terminal)
        && ran.failed
    {
        let other = if check.deep_first.contains(&terminal) { "ask the deep brain with nixa_deep_ask" } else { "record needs-user with the reason" };
        return Err(format!(
            "{assistant} ran the checks and they failed: {}. The job is not done. Send one fix to the agent with nixa_term_send, or {other}.",
            short_checks(ran)
        ));
    }
    if outcome == Outcome::NeedsUser && check.deep_first.contains(&terminal) {
        crate::util::log("review_job", &format!("refused needs-user for {terminal}"));
        return Err(if user_only(&reason) { PHYSICAL_STEP } else { DEEP_FIRST_REFUSED }.into());
    }
    if let Some(refused) = outcome_refused(check.picks.get(&terminal).map(String::as_str), outcome) {
        crate::util::log("review_job", &format!("refused {terminal} {}", outcome.word()));
        return Err(refused.into());
    }
    check.reviews.insert(terminal.clone(), Review { outcome, reason });
    Ok(format!("Review recorded for {terminal}: {}. It takes effect only after this check turn succeeds.", outcome.word()))
}
