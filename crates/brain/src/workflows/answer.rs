use regex::Regex;
use std::sync::LazyLock;

pub const ASK_OPTIONS: [&str; 3] = ["Go on from this box", "Go to the next box", "Cancel the run"];
pub const GATE_OPTIONS: [&str; 4] = ["pass", "revise", "fail", "Cancel the run"];
pub const CANCEL_OPTION: &str = "Cancel the run";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("answer pattern")
}

static NOT_BEFORE: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:not|never|cannot|can't|don't|dont|won't|no)\s+(?:[a-z']+\s+){0,3}$"));
static CANCEL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^\s*cancel(?:\s+the)?(?:\s+(?:workflow\s+)?run)?\s*[.!]?\s*$"));
static NUMBERED: LazyLock<Regex> = LazyLock::new(|| re(r"^\(?(\d+)\)?(?:[.):]\s*(.*))?$"));
static GATE_CHOICES: LazyLock<Vec<(&'static str, Regex)>> =
    LazyLock::new(|| vec![("pass", re(r"\bpass\b")), ("revise", re(r"\brevise\b")), ("fail", re(r"\bfail\b"))]);
const GO_ON: &str = r"\bgo on\b|\bagain\b|\bretry\b|\bthis box\b|\bcontinue\b";
static ASK_CHOICES: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| vec![("next", re(r"\bnext\b")), ("on", re(GO_ON))]);
static BLOCK_CHOICES: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    vec![
        ("next", re(r"\b(?:go|move|skip|jump)\s+(?:on\s+)?to\s+(?:the\s+)?next\s+(?:box|stage)\b|\bskip\s+(?:this|the)\s+(?:box|stage)\b")),
        ("on", re(GO_ON)),
    ]
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateWord {
    Pass,
    Revise,
    Fail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskWord {
    GoOn,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Cancel,
    Gate(GateWord),
    Ask(AskWord),
}

fn trim_quotes(text: &str) -> String {
    let low = text.trim().to_lowercase();
    let start = low.trim_start_matches(|c: char| matches!(c, '"' | '\'' | '\u{60}') || c.is_whitespace());
    start.trim_end_matches(|c: char| matches!(c, '"' | '\'' | '\u{60}' | '.' | '!') || c.is_whitespace()).to_string()
}

pub fn option_of<'a>(text: &str, options: &[&'a str]) -> Option<&'a str> {
    let low = trim_quotes(text);
    if let Some(found) = NUMBERED.captures(&low) {
        let number: usize = found[1].parse().unwrap_or(0);
        if let Some(option) = number.checked_sub(1).and_then(|at| options.get(at)) {
            let rest = found.get(2).map(|m| m.as_str()).unwrap_or("");
            if rest.is_empty() || rest == option.to_lowercase() {
                return Some(option);
            }
        }
    }
    options.iter().copied().find(|option| option.to_lowercase() == low)
}

fn choice_in(text: &str, choices: &[(&'static str, Regex)]) -> Option<&'static str> {
    let low = text.to_lowercase();
    let mut best: Option<(usize, &'static str)> = None;
    for (choice, pattern) in choices {
        let hit = pattern.find_iter(&low).find(|m| !NOT_BEFORE.is_match(&low[..m.start()]));
        if let Some(hit) = hit
            && best.is_none_or(|(at, _)| hit.start() < at)
        {
            best = Some((hit.start(), choice));
        }
    }
    best.map(|(_, choice)| choice)
}

pub fn read_answer(text: &str, gate: bool, agent_block: bool) -> Result<Answer, String> {
    let options: &[&str] = if gate { &GATE_OPTIONS } else { &ASK_OPTIONS };
    let option = option_of(text, options);
    if option == Some(CANCEL_OPTION) || CANCEL.is_match(text) {
        return Ok(Answer::Cancel);
    }
    if gate {
        let word = option.or_else(|| choice_in(text, &GATE_CHOICES));
        return match word {
            Some("pass") => Ok(Answer::Gate(GateWord::Pass)),
            Some("revise") => Ok(Answer::Gate(GateWord::Revise)),
            Some("fail") => Ok(Answer::Gate(GateWord::Fail)),
            _ => Err("Answer pass, revise, fail or Cancel the run.".into()),
        };
    }
    let pick = match option {
        Some("Go to the next box") => Some("next"),
        Some("Go on from this box") => Some("on"),
        _ if agent_block => Some(choice_in(text, &BLOCK_CHOICES).unwrap_or("on")),
        _ => choice_in(text, &ASK_CHOICES),
    };
    match pick {
        Some("next") => Ok(Answer::Ask(AskWord::Next)),
        Some(_) => Ok(Answer::Ask(AskWord::GoOn)),
        None => Err("Answer Go on from this box, Go to the next box or Cancel the run.".into()),
    }
}
