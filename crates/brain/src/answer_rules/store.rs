use crate::risk::{hard_stop_text, risky_text};
use crate::util::{now_ms, write_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FILE: &str = "answer-rules.json";
pub const MAX_RULES: usize = 200;
pub const MAX_TEXT: usize = 400;
pub const MAX_FOLDER: usize = 1000;
pub const ALL: &str = "*";
pub const MIN_KEY_WORDS: usize = 2;
pub const STOP_WORDS: [&str; 40] = [
    "the", "and", "you", "your", "for", "are", "can", "should", "would", "will", "shall", "want",
    "this", "that", "with", "now", "please", "does", "did", "has", "have", "was", "were", "not",
    "but", "any", "all", "its", "may", "might", "could", "what", "which", "then", "there", "here",
    "okay", "yes", "also", "just",
];
pub const SHORT_ASK: &str = "The question needs at least 2 clear words of 3 letters or more, such as \"run tests\". Short words like \"it\" or \"do\" fit too many questions.";
pub const DANGER_WORDS: [&str; 16] = [
    "delete",
    "remove",
    "drop",
    "overwrite",
    "kill",
    "uninstall",
    "erase",
    "force",
    "reset",
    "wipe",
    "destroy",
    "truncate",
    "rm",
    "purge",
    "revoke",
    "password",
];

static WRITE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleStatus {
    Trial,
    Offer,
    Live,
    Off,
}

impl RuleStatus {
    pub fn parse(word: &str) -> Option<RuleStatus> {
        match word.trim() {
            "trial" => Some(RuleStatus::Trial),
            "offer" => Some(RuleStatus::Offer),
            "live" => Some(RuleStatus::Live),
            "off" => Some(RuleStatus::Off),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            RuleStatus::Trial => "trial",
            RuleStatus::Offer => "offer",
            RuleStatus::Live => "live",
            RuleStatus::Off => "off",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerRule {
    pub id: u64,
    pub project: String,
    pub ask: String,
    pub answer: String,
    pub status: RuleStatus,
    #[serde(default)]
    pub hits: u64,
    #[serde(default)]
    pub answered: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_hit: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct RulesFile {
    version: u32,
    rules: Vec<AnswerRule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuleInput {
    pub id: Option<u64>,
    pub project: String,
    pub ask: String,
    pub answer: String,
    pub status: RuleStatus,
    pub expected: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Answer,
    Watch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub rule: AnswerRule,
    pub act: Act,
}

pub fn file_of(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

pub fn stamp(rule: &AnswerRule) -> String {
    json!([rule.project, rule.ask, rule.answer, rule.status.word()]).to_string()
}

pub fn load(dir: &Path) -> Result<Vec<AnswerRule>, String> {
    let file = file_of(dir);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{FILE} could not be read: {e}")),
    };
    let parsed: RulesFile = serde_json::from_str(&text)
        .map_err(|e| format!("{FILE} is broken: {e}. Fix it or remove it."))?;
    Ok(parsed.rules)
}

fn store(dir: &Path, rules: &[AnswerRule]) -> Result<(), String> {
    write_json(
        &file_of(dir),
        &RulesFile {
            version: 1,
            rules: rules.to_vec(),
        },
    )
    .map_err(|e| format!("{FILE} was not saved: {e}"))
}

fn lock_write() -> std::sync::MutexGuard<'static, ()> {
    WRITE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn clean_project(raw: &str) -> Result<String, String> {
    let project = raw.trim();
    if project == ALL {
        return Ok(ALL.into());
    }
    if !project.starts_with('/')
        || project.contains(['\n', '\r'])
        || project.chars().count() > MAX_FOLDER
    {
        return Err(
            "The project is * for all projects, or a full project folder that starts with /."
                .into(),
        );
    }
    let trimmed = project.trim_end_matches('/');
    Ok(if trimmed.is_empty() {
        "/".into()
    } else {
        trimmed.to_string()
    })
}

fn clean_text(raw: &str, what: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(format!("{what} is empty."));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(format!("{what} is longer than {MAX_TEXT} characters."));
    }
    Ok(text.to_string())
}

pub fn input_of(body: &Value) -> Result<RuleInput, String> {
    let field = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let status = match body.get("status").and_then(Value::as_str) {
        None => RuleStatus::Trial,
        Some(word) => RuleStatus::parse(word).ok_or("The status is trial, offer, live or off.")?,
    };
    Ok(RuleInput {
        id: body.get("id").and_then(Value::as_u64),
        project: field("project"),
        ask: field("ask"),
        answer: field("answer"),
        status,
        expected: body
            .get("expected")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub fn hard_stop_rule(input: &RuleInput) -> bool {
    hard_stop_text(&input.ask) || hard_stop_text(&input.answer)
}

pub fn save(dir: &Path, input: &RuleInput) -> Result<AnswerRule, String> {
    let project = clean_project(&input.project)?;
    let ask = clean_text(&input.ask, "The question")?;
    if key_words(&ask).len() < MIN_KEY_WORDS {
        return Err(SHORT_ASK.into());
    }
    let answer = clean_text(&input.answer, "The answer")?;
    let _guard = lock_write();
    let mut rules = load(dir)?;
    let saved = match input.id {
        Some(id) => {
            let Some(rule) = rules.iter_mut().find(|r| r.id == id) else {
                return Err(format!(
                    "Rule {id} is gone. It was removed elsewhere. Your draft is kept: save it as a new rule."
                ));
            };
            if input
                .expected
                .as_deref()
                .is_some_and(|want| want != stamp(rule))
            {
                return Err(format!(
                    "Rule {id} changed elsewhere. Your draft is kept. Cancel and open the rule again to see the new text."
                ));
            }
            rule.project = project;
            rule.ask = ask;
            rule.answer = answer;
            rule.status = input.status;
            rule.clone()
        }
        None => {
            if rules.len() >= MAX_RULES {
                return Err(format!(
                    "There are already {MAX_RULES} answer rules. Remove one first."
                ));
            }
            let id = rules.iter().map(|r| r.id).max().unwrap_or(0) + 1;
            let rule = AnswerRule {
                id,
                project,
                ask,
                answer,
                status: input.status,
                hits: 0,
                answered: 0,
                last_hit: None,
            };
            rules.push(rule.clone());
            rule
        }
    };
    store(dir, &rules)?;
    Ok(saved)
}

pub fn remove(dir: &Path, id: u64, expected: Option<&str>) -> Result<AnswerRule, String> {
    let _guard = lock_write();
    let mut rules = load(dir)?;
    let Some(at) = rules.iter().position(|r| r.id == id) else {
        return Err(format!("Rule {id} is already gone."));
    };
    if expected.is_some_and(|want| want != stamp(&rules[at])) {
        return Err(format!(
            "Rule {id} changed elsewhere. Look at it again before you remove it."
        ));
    }
    let gone = rules.remove(at);
    store(dir, &rules)?;
    Ok(gone)
}

pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(str::to_string)
        .collect()
}

pub fn key_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in words(text) {
        if word.chars().count() >= 3 && !STOP_WORDS.contains(&word.as_str()) && !out.contains(&word)
        {
            out.push(word);
        }
    }
    out
}

pub fn needs_user(question: &str) -> bool {
    risky_text(question)
        || words(question)
            .iter()
            .any(|w| DANGER_WORDS.iter().any(|d| w.starts_with(d)))
}

pub fn covers(project: &str, folder: &str) -> bool {
    if project == ALL {
        return true;
    }
    let folder = folder.trim_end_matches('/');
    folder == project
        || project == "/"
        || folder
            .strip_prefix(project)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub fn asks(rule_ask: &str, question: &str) -> bool {
    let wanted = key_words(rule_ask);
    if wanted.len() < MIN_KEY_WORDS {
        return false;
    }
    let have = key_words(question);
    wanted.iter().all(|w| have.contains(w)) && wanted.len() * 2 >= have.len()
}

pub fn pick<'a>(rules: &'a [AnswerRule], folder: &str, question: &str) -> Option<&'a AnswerRule> {
    rules
        .iter()
        .filter(|r| {
            r.status != RuleStatus::Off && covers(&r.project, folder) && asks(&r.ask, question)
        })
        .max_by_key(|r| {
            (
                if r.project == ALL {
                    0
                } else {
                    r.project.len() + 1
                },
                key_words(&r.ask).len(),
                std::cmp::Reverse(r.id),
            )
        })
}

pub fn act_of(status: RuleStatus, question: &str) -> Act {
    if status == RuleStatus::Live && !needs_user(question) {
        Act::Answer
    } else {
        Act::Watch
    }
}

pub fn check(dir: &Path, folder: &str, question: &str) -> Result<Option<Hit>, String> {
    let rules = load(dir)?;
    Ok(pick(&rules, folder, question).map(|rule| Hit {
        rule: rule.clone(),
        act: act_of(rule.status, question),
    }))
}

pub fn decide(dir: &Path, folder: &str, question: &str) -> Result<Option<Hit>, String> {
    let _guard = lock_write();
    let mut rules = load(dir)?;
    let Some(id) = pick(&rules, folder, question).map(|r| r.id) else {
        return Ok(None);
    };
    let Some(rule) = rules.iter_mut().find(|r| r.id == id) else {
        return Ok(None);
    };
    rule.hits += 1;
    rule.last_hit = Some(now_ms());
    let act = act_of(rule.status, question);
    if act == Act::Answer {
        rule.answered += 1;
    }
    let hit = Hit {
        rule: rule.clone(),
        act,
    };
    store(dir, &rules)?;
    Ok(Some(hit))
}
