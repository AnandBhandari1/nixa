use super::auto::{answer_of_pick, pick_labels, send_pick, user_will_answer};
use super::book::{Prompt, PromptKind};
use super::card::describe;
use super::scan;
use crate::app::{App, lock};
use crate::decide::{self, Answer, Answers, settings};
use crate::spaces::Space;
use crate::util::{clip_flat, log, now_ms};
use regex::Regex;
use std::sync::{Arc, LazyLock};

pub const WAITING: &str = "waiting";
pub const PICK: &str = "pick";
pub const SAFE: &str = "safe";

static RISKY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(push|force|delete|deleting|remove|removing|rm|rmdir|drop|truncate|reset|wipe|erase|overwrite|destroy|kill|uninstall|merge|rebase|deploy|publish|release|pay|payment|purchase|buy|charge|refund|transfer|invoice|send|email|post|tweet|message|password|credential|token|secret|key|sudo|chmod|chown|production|prod|database|migrate|account|curl|wget|sh|python\d*(?:\.\d+)?|node|ruby|perl|php|ssh|scp|sftp|rsync|nc|netcat|eval|exec|osascript)\b").expect("risky words")
});

static PIPED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:curl|wget)\b[^|\n]*\|\s*(?:sudo\s+)?(?:sh|bash|zsh|python\d*|node|ruby|perl)\b").expect("piped shell"));

pub fn hard_stop(prompt: &Prompt) -> bool {
    let text = prompt_text(prompt);
    crate::risk::hard_stop_text(&text) || PIPED.is_match(&text)
}

pub fn prompt_text(prompt: &Prompt) -> String {
    let mut parts = vec![describe(prompt)];
    parts.extend(prompt.text.clone());
    parts.extend(prompt.tool_name.clone());
    parts.extend(prompt.detail.clone());
    for q in &prompt.questions {
        parts.push(q.question.clone());
        parts.extend(q.header.clone());
        parts.extend(q.options.iter().flat_map(|o| std::iter::once(o.label.clone()).chain(o.description.clone())));
    }
    parts.extend(pick_labels(prompt));
    parts.join("\n")
}

pub fn risky(prompt: &Prompt) -> Option<String> {
    RISKY.find(&prompt_text(prompt)).map(|m| m.as_str().to_lowercase())
}

pub fn why_not(prompt: &Prompt) -> Option<String> {
    if prompt.unread {
        return Some("its options cannot be read".into());
    }
    if prompt.question().is_some_and(|q| q.multi_select) {
        return Some("it allows more than one option".into());
    }
    if pick_labels(prompt).len() < 2 {
        return Some("it has no options to pick from".into());
    }
    risky(prompt).map(|word| format!("it has the risky word \"{word}\""))
}

pub const WAITING_TEXT: &str = "The agent is waiting for an answer right now.";
pub const PICK_TEXT: &str = "Which option number should be picked? Pick the option that lets the agent go on with only this one step.";
pub const SAFE_TEXT: &str = "This request only reads, edits, builds or tests files inside the project. It does not delete data, push code, pay money, send messages or touch production.";

pub fn numbers(prompt: &Prompt) -> Vec<String> {
    (1..=pick_labels(prompt).len()).map(|n| n.to_string()).collect()
}

pub fn questions(prompt: &Prompt) -> Vec<serde_json::Value> {
    let numbers = numbers(prompt);
    let choices: Vec<(&str, &str)> = numbers.iter().map(|n| (n.as_str(), "")).collect();
    vec![decide::predicate(WAITING, WAITING_TEXT), decide::choice(PICK, PICK_TEXT, &choices), decide::predicate(SAFE, SAFE_TEXT)]
}

pub fn input(prompt: &Prompt, space: Option<&str>) -> String {
    let mut lines = vec!["An agent working on a coding task in a terminal asks the user:".to_string()];
    if let Some(space) = space {
        lines.insert(0, format!("Space: {space}"));
    }
    let mut asked_lines = Vec::new();
    if prompt.kind == PromptKind::Permission {
        asked_lines.push(format!("{} request:", prompt.tool_name.as_deref().unwrap_or("Tool")));
    }
    asked_lines.extend(prompt.detail.clone().filter(|d| !d.trim().is_empty()));
    let asked = prompt.question().map(|q| q.question.clone()).or_else(|| prompt.text.clone()).unwrap_or_default();
    if !asked.trim().is_empty() {
        asked_lines.push(asked);
    }
    asked_lines.push("Options:".into());
    asked_lines.extend(pick_labels(prompt).iter().enumerate().map(|(i, l)| format!("{}. {l}", i + 1)));
    lines.push(crate::agent::data_block(&asked_lines.join("\n")));
    lines.join("\n")
}

fn probability(answers: &Answers, name: &str) -> f64 {
    match answers.get(name) {
        Some(Answer::Predicate { probability, .. }) => *probability,
        _ => 0.0,
    }
}

pub fn verdict(answers: &Answers, level: f64) -> Result<String, String> {
    let waiting = probability(answers, WAITING);
    let safe = probability(answers, SAFE);
    let (pick, confidence) = match answers.get(PICK) {
        Some(Answer::Choice { choice, confidence, .. }) => (choice.as_str().map(str::to_string).unwrap_or_else(|| choice.to_string()), *confidence),
        _ => return Err("it gave no pick".into()),
    };
    let note = format!("pick \"{pick}\" ({confidence:.2}), waiting {waiting:.2}, safe {safe:.2}");
    if waiting < level || confidence < level || safe < level {
        return Err(format!("it was not sure enough: {note}, level {level:.2}"));
    }
    Ok(pick)
}

pub fn note_of(answers: &Answers) -> String {
    answers.answers.iter().map(Answer::short).collect::<Vec<_>>().join(", ")
}

pub fn take(app: &Arc<App>, prompt: &Prompt, space: Option<&Space>) -> bool {
    if !crate::settings_api::nixa_on(&app.dir) {
        return false;
    }
    let set = settings::load(app);
    if !set.uses("questions") {
        return false;
    }
    if lock(&app.deep).owns(&prompt.terminal) || user_will_answer(app, prompt) {
        return false;
    }
    if let Some(why) = why_not(prompt) {
        log("decide", &format!("prompt {} goes the current way: {why}", prompt.id));
        return false;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return false };
    lock(&app.prompts.book).told_auto.insert(prompt.id.clone(), now_ms());
    let app = app.clone();
    let prompt = prompt.clone();
    let space = space.cloned();
    handle.spawn(async move {
        let extra = match attempt(&app, &prompt, &set).await {
            Ok(()) => return,
            Err(why) => why,
        };
        log("decide", &format!("prompt {} goes to the chat brain: {extra}", prompt.id));
        let still = lock(&app.prompts.book).get(&prompt.id).cloned();
        if let Some(open) = still {
            scan::tell_now(&app, &open, space.as_ref(), &format!(" Decision model: {extra}."));
        }
    });
    true
}

async fn attempt(app: &Arc<App>, prompt: &Prompt, set: &settings::Settings) -> Result<(), String> {
    let task = space_task(app, prompt);
    let answers = decide::ask(app, &set.model, vec![decide::text(&input(prompt, task.as_deref()))], questions(prompt)).await?;
    let pick = verdict(&answers, set.level)?;
    let open = lock(&app.prompts.book).get(&prompt.id).cloned().ok_or("the prompt closed before the answer")?;
    if let Some(why) = why_not(&open) {
        return Err(format!("the prompt changed and {why}"));
    }
    let given = answer_of_pick(&open, &pick).map_err(|e| format!("its pick \"{pick}\" did not fit: {e}"))?;
    let reason = format!("decision model {}: {}", set.model, note_of(&answers));
    let sent = send_pick(app, &open, given, reason).await?;
    if !sent.starts_with("Sent") {
        return Err(format!("its pick was not sent: {sent}"));
    }
    let label = pick.parse::<usize>().ok().and_then(|n| pick_labels(&open).get(n.wrapping_sub(1)).cloned()).unwrap_or_else(|| pick.clone());
    app.info(&format!("The decision model {} answered prompt {} in {}: {} ({}).", set.model, open.id, open.terminal, clip_flat(&label, 120), note_of(&answers)));
    scan::notify_parent(app, &open, true);
    Ok(())
}

fn space_task(app: &App, prompt: &Prompt) -> Option<String> {
    app.space_of_terminal(&prompt.terminal).map(|s| s.name)
}

#[cfg(test)]
mod tests;
