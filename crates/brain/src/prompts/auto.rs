use super::answer::{answer_by_tool, user_line_after};
use super::book::{Answer, Choice, DeepWait, Prompt, PromptKind, option_labels};
use super::card::describe;
use super::dialog::dialog_key;
use super::{deliver, notes, scan};
use crate::app::{App, lock};
use crate::deep::{Ask, AskInput, AskState};
use crate::spaces::Space;
use crate::util::{clip_flat, log, now_ms};
use regex::Regex;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

pub const ASK_KIND: &str = "prompt";
pub const PERMISSION_PICKS: [&str; 2] = ["Yes, allow once", "No, deny"];
pub const PLAN_PICKS: [&str; 2] = ["Approve the plan", "Reject the plan"];
pub const CONFIRM_MS: u64 = 2500;
pub const BUSY_TRIES: u32 = 3;
pub const PROMPT_WAIT_MS: u64 = crate::followup::hard::DEEP_RELAY_MS;
const RECHECK_MS: u64 = 10 * 60 * 1000;

static ASK_USER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(ask|tell|wait for) (the )?(user|owner)\b|\bneeds[- ]user\b|\b(user|owner) (decides|answers|must)\b").expect("ask user"));
static ASK_ME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bask me( first)?\b|\b(i|i'll|i will) (answer|decide)\b|\bdon'?t answer\b|\bdo not answer\b").expect("ask me"));
static YES_PICK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\W*(yes|allow|approve|accept|proceed|ok|okay|go ahead)\b").expect("yes pick"));
static NO_PICK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\W*(no|deny|reject|decline|cancel|stop|don'?t)\b").expect("no pick"));
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^\s*(\d{1,2})\s*[).:]?\s*(.*)$").expect("number pick"));
static TEXT_PICK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)^\s*(?:text|type|typed)\s*:\s*(.+)$").expect("text pick"));
static SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*(?:,|;|\+|\band\b)\s*").expect("split pick"));
static NOT_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("not word"));

fn norm(text: &str) -> String {
    NOT_WORD.replace_all(&text.to_lowercase(), " ").trim().to_string()
}

pub fn prompt_sig(prompt: &Prompt) -> String {
    let asked = prompt.question().map(|q| q.question.clone()).or_else(|| prompt.text.clone()).unwrap_or_default();
    let kind = match prompt.kind {
        PromptKind::Question => "question",
        PromptKind::Permission => "permission",
        PromptKind::Plan => "plan",
    };
    let detail = if prompt.kind == PromptKind::Question { String::new() } else { prompt.detail.clone().unwrap_or_default() };
    format!("{kind}|{}|{}|{}", norm(&asked), norm(&detail), option_labels(prompt).iter().map(|l| norm(l)).collect::<Vec<_>>().join("|"))
}

pub fn takes_text(prompt: &Prompt) -> bool {
    if prompt.plain {
        return true;
    }
    if prompt.kind != PromptKind::Question {
        return false;
    }
    match prompt.screen_key.as_deref() {
        Some(key) => key.to_lowercase().contains("type something"),
        None => prompt.agent == "claude",
    }
}

pub fn pick_labels(prompt: &Prompt) -> Vec<String> {
    if prompt.plain {
        return vec!["Yes".into(), "No".into()];
    }
    match prompt.kind {
        PromptKind::Permission => PERMISSION_PICKS.iter().map(|s| s.to_string()).collect(),
        PromptKind::Plan => PLAN_PICKS.iter().map(|s| s.to_string()).collect(),
        PromptKind::Question => prompt.question().map(|q| q.options.iter().map(|o| o.label.clone()).collect()).unwrap_or_default(),
    }
}

pub fn ask_input(prompt: &Prompt) -> AskInput {
    let head = format!("Agent prompt {} in terminal {}", prompt.id, prompt.terminal);
    let text = prompt.text.clone().unwrap_or_default();
    let question = match prompt.kind {
        PromptKind::Permission => format!("{head} asks permission to use {}: {}", prompt.tool_name.as_deref().unwrap_or("a tool"), clip_flat(&[prompt.detail.clone().unwrap_or_default(), text].join(" "), 300)),
        PromptKind::Plan => format!("{head} has a plan ready and asks to approve it. {}", clip_flat(&text, 300)),
        PromptKind::Question => {
            let total = prompt.questions.len().max(1);
            let step = if total > 1 { format!(" (question {} of {total})", prompt.question_index + 1) } else { String::new() };
            let asked = prompt.question().map(|q| q.question.clone()).unwrap_or(text);
            format!("{head} asks{step}: {}", clip_flat(&asked, 300))
        }
    };
    let mut notes = vec![describe(prompt)];
    if prompt.question().is_some_and(|q| q.multi_select) && !prompt.plain {
        notes.push("More than one option may be picked: give their exact names joined with commas.".into());
    }
    if takes_text(prompt) {
        notes.push("A typed answer is possible: pick \"text: <your words>\".".into());
    }
    notes.push("Pick one option exactly as written. Nixa sends your pick to the agent once and allows a permission once only. Never pick \"ask the user\", always pick one of the options.".into());
    AskInput { question, options: pick_labels(prompt), notes: notes.join(" "), kind: Some(ASK_KIND.into()), wait: Some(PROMPT_WAIT_MS), ..AskInput::default() }
}

fn yes_no(pick: &str) -> Option<Choice> {
    let no = NO_PICK.is_match(pick);
    let yes = YES_PICK.is_match(pick);
    match (yes, no) {
        (true, false) => Some(Choice::Yes),
        (false, true) => Some(Choice::No),
        _ => None,
    }
}

fn option_index(labels: &[String], pick: &str) -> Option<usize> {
    if let Some(found) = NUMBER.captures(pick) {
        let number: usize = found[1].parse().unwrap_or(0);
        let rest = norm(&found[2]);
        if let Some(index) = number.checked_sub(1).filter(|i| *i < labels.len()) {
            let label = norm(&labels[index]);
            if rest.is_empty() || label == rest || label.starts_with(&rest) || rest.starts_with(&label) {
                return Some(index);
            }
            return None;
        }
    }
    let want = norm(pick);
    if want.is_empty() {
        return None;
    }
    let names: Vec<String> = labels.iter().map(|l| norm(l)).collect();
    if let Some(exact) = names.iter().position(|n| *n == want) {
        return Some(exact);
    }
    let padded = format!(" {want} ");
    if names.iter().filter(|n| !n.is_empty() && padded.contains(&format!(" {n} "))).count() > 1 {
        return None;
    }
    let close: Vec<usize> = names.iter().enumerate().filter(|(_, n)| !n.is_empty() && (want.starts_with(n.as_str()) || (want.len() >= 3 && n.starts_with(want.as_str())))).map(|(i, _)| i).collect();
    (close.len() == 1).then(|| close[0])
}

pub fn answer_of_pick(prompt: &Prompt, pick: &str) -> Result<Answer, String> {
    let pick = pick.trim();
    if pick.is_empty() {
        return Err("the pick is empty".into());
    }
    if ASK_USER.is_match(pick) {
        return Err("it says to ask the user".into());
    }
    if let Some(found) = TEXT_PICK.captures(pick) {
        let words = found[1].trim().to_string();
        if !takes_text(prompt) {
            return Err("this prompt takes no typed answer".into());
        }
        return Ok(Answer { text: Some(words), ..Answer::default() });
    }
    if prompt.plain {
        return Ok(match yes_no(pick) {
            Some(choice) => Answer { choice: Some(choice), ..Answer::default() },
            None => Answer { text: Some(pick.to_string()), ..Answer::default() },
        });
    }
    match prompt.kind {
        PromptKind::Permission | PromptKind::Plan => {
            let labels = pick_labels(prompt);
            let choice = match option_index(&labels, pick) {
                Some(0) => Some(Choice::Yes),
                Some(_) => Some(Choice::No),
                None => yes_no(pick),
            };
            choice.map(|c| Answer { choice: Some(c), ..Answer::default() }).ok_or_else(|| "it is not yes or no".into())
        }
        PromptKind::Question => {
            let question = prompt.question().ok_or("the question has no options")?;
            let labels: Vec<String> = question.options.iter().map(|o| o.label.clone()).collect();
            if labels.is_empty() {
                return Err("the question has no options".into());
            }
            if !question.multi_select {
                return option_index(&labels, pick).map(|index| Answer { options: vec![index + 1], ..Answer::default() }).ok_or_else(|| "it fits none of the options".into());
            }
            if let Some(index) = labels.iter().position(|l| norm(l) == norm(pick)) {
                return Ok(Answer { options: vec![index + 1], ..Answer::default() });
            }
            let mut options = Vec::new();
            for part in SPLIT.split(pick).map(str::trim).filter(|p| !p.is_empty()) {
                let index = option_index(&labels, part).ok_or_else(|| format!("{} fits none of the options", serde_json::to_string(part).unwrap_or_default()))?;
                if !options.contains(&(index + 1)) {
                    options.push(index + 1);
                }
            }
            if options.is_empty() {
                return Err("it fits none of the options".into());
            }
            Ok(Answer { options, ..Answer::default() })
        }
    }
}

pub fn space_for(app: &App, prompt: &Prompt) -> Option<Space> {
    let spaces = app.spaces();
    app.space_of_terminal(&prompt.terminal).or_else(|| prompt.cwd.as_deref().and_then(|c| crate::spaces::for_folder(&spaces, c).cloned()))
}

pub fn may_route(app: &App, prompt: &Prompt) -> Result<Space, String> {
    if prompt.unread {
        return Err("its options cannot be read".into());
    }
    if lock(&app.deep).owns(&prompt.terminal) {
        return Err("it comes from a deep brain terminal".into());
    }
    if user_line_after(app, prompt.asked_at).is_some_and(|(said, _)| ASK_ME.is_match(&said)) {
        return Err("the user said they answer it".into());
    }
    let space = space_for(app, prompt).ok_or("no space is known for its terminal")?;
    if !app.decision.deep_mode() {
        return Err("the deep brain does not decide in this space".into());
    }
    Ok(space)
}

pub fn user_will_answer(app: &App, prompt: &Prompt) -> bool {
    user_line_after(app, prompt.asked_at).is_some_and(|(said, _)| ASK_ME.is_match(&said))
}

pub fn deciding(app: &App, prompt: &Prompt) -> Option<String> {
    let sig = prompt_sig(prompt);
    let book = lock(&app.prompts.book);
    book.deep.iter().find(|w| w.terminal == prompt.terminal && (w.prompt == prompt.id || w.sig == sig)).map(|w| w.ask.clone())
}

pub fn current(app: &App, wait: &DeepWait) -> Option<Prompt> {
    let book = lock(&app.prompts.book);
    if let Some(open) = book.get(&wait.prompt).filter(|p| prompt_sig(p) == wait.sig) {
        return Some(open.clone());
    }
    book.items.iter().find(|p| p.terminal == wait.terminal && prompt_sig(p) == wait.sig).cloned()
}

pub fn route_now(app: &App, prompt: &Prompt) -> Option<String> {
    if !app.decision.deep_mode() || space_for(app, prompt).is_none() {
        return None;
    }
    let space = match may_route(app, prompt) {
        Ok(space) => space,
        Err(why) => {
            log("prompts", &format!("prompt {} not sent to the deep brain: {why}", prompt.id));
            return None;
        }
    };
    let sig = prompt_sig(prompt);
    let now = now_ms();
    {
        let mut book = lock(&app.prompts.book);
        book.deep.retain(|w| now.saturating_sub(w.at) < crate::deep::TIMEOUT_MS);
        if let Some(wait) = book.deep.iter_mut().find(|w| w.terminal == prompt.terminal && w.sig == sig) {
            wait.prompt = prompt.id.clone();
            return Some(wait.ask.clone());
        }
    }
    match crate::deep_service::ask(app, ask_input(prompt), &space) {
        Ok((made, _)) => {
            lock(&app.prompts.book).deep.push(DeepWait { ask: made.id.clone(), prompt: prompt.id.clone(), terminal: prompt.terminal.clone(), sig, at: now });
            log("prompts", &format!("prompt {} goes to the deep brain as {}", prompt.id, made.id));
            app.info(&format!("Asked the deep brain of space {} as {} to answer prompt {} in {}.", space.name, made.id, prompt.id, prompt.terminal));
            Some(made.id)
        }
        Err(error) => {
            log("prompts", &format!("prompt {} not sent to the deep brain: {error}", prompt.id));
            None
        }
    }
}

pub fn route(app: &Arc<App>, prompt: &Prompt) -> Option<String> {
    let made = route_now(app, prompt)?;
    if !cfg!(test)
        && let Ok(handle) = tokio::runtime::Handle::try_current()
    {
        let app = app.clone();
        handle.spawn(async move { crate::watch::deep_tick(&app).await });
    }
    Some(made)
}

fn take_wait(app: &App, ask: &str) -> Option<DeepWait> {
    let mut book = lock(&app.prompts.book);
    let at = book.deep.iter().position(|w| w.ask == ask)?;
    Some(book.deep.remove(at))
}

fn tell_again(app: &Arc<App>, prompt: &Prompt) {
    let space = app.space_of_terminal(&prompt.terminal);
    scan::tell(app, prompt, space.as_ref());
}

fn fallback(app: &Arc<App>, prompt: &Prompt, why: &str) {
    let space = space_for(app, prompt);
    if !crate::settings_api::nixa_on(&app.dir) || !crate::agent::has_brain(app) {
        lock(&app.prompts.book).untold.insert(prompt.id.clone());
        return;
    }
    if user_line_after(app, prompt.asked_at).is_some_and(|(said, _)| ASK_ME.is_match(&said)) {
        return;
    }
    {
        let mut book = lock(&app.prompts.book);
        book.untold.remove(&prompt.id);
        book.told_auto.insert(prompt.id.clone(), now_ms());
    }
    let text = scan::told_text(app, prompt, space.as_ref());
    let next = " Nixa Chat owns this decision now. Read the current prompt and its task context, then use the prompt answer tool if the same prompt is still open. If it cannot be answered safely, keep independent work moving and record the blocker. Do not send the same question back to the deep brain.";
    crate::watch::tell_event(app, &format!("{text} Deep brain handoff: {why}.{next}"));
}

pub async fn revisit(app: &Arc<App>) {
    if !crate::settings_api::nixa_on(&app.dir) {
        return;
    }
    let now = now_ms();
    let prompts = lock(&app.prompts.book).items.clone();
    for prompt in prompts {
        if user_line_after(app, prompt.asked_at).is_some_and(|(said, _)| ASK_ME.is_match(&said)) {
            continue;
        }
        let (wait, told, untold) = {
            let book = lock(&app.prompts.book);
            if book.get(&prompt.id).is_none() || book.sending.as_deref() == Some(&prompt.id) {
                continue;
            }
            let wait = book.deep.iter().find(|wait| wait.terminal == prompt.terminal && wait.sig == prompt_sig(&prompt)).cloned();
            (wait, book.told_auto.get(&prompt.id).copied(), book.untold.contains(&prompt.id))
        };
        if let Some(wait) = wait {
            let saved = lock(&app.deep).asks.iter().find(|ask| ask.id == wait.ask).cloned();
            match saved {
                Some(ask) if matches!(ask.state, AskState::Answered | AskState::Timeout) => { answered(app, &ask).await; }
                _ if now.saturating_sub(wait.at) >= PROMPT_WAIT_MS + crate::deep::JOIN_MS
                    && take_wait(app, &wait.ask).is_some() =>
                {
                    fallback(app, &prompt, "the decision deadline passed");
                }
                _ => {}
            }
        } else if told.is_none() {
            let space = space_for(app, &prompt);
            scan::tell(app, &prompt, space.as_ref());
        } else if crate::agent::has_brain(app) && (untold || told.is_some_and(|at| now.saturating_sub(at) >= RECHECK_MS)) {
            fallback(app, &prompt, "the agent's prompt is still open");
        }
    }
}

pub(super) async fn send_pick(app: &Arc<App>, prompt: &Prompt, given: Answer, reason: String) -> Result<String, String> {
    let link = app.prompts.link();
    let mut tries = 0;
    loop {
        tries += 1;
        match answer_by_tool(app, &link, &prompt.id, given.clone(), true, Some(reason.clone())).await {
            Err(error) if error.contains("still sending") && tries < BUSY_TRIES => tokio::time::sleep(Duration::from_millis(super::TICK_MS)).await,
            other => return other,
        }
    }
}

fn confirm_later(app: &Arc<App>, prompt: &Prompt, ask: &str) {
    let Some(key) = prompt.screen_key.clone() else { return };
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    let app = app.clone();
    let prompt = prompt.clone();
    let ask = ask.to_string();
    handle.spawn(async move {
        tokio::time::sleep(Duration::from_millis(CONFIRM_MS)).await;
        let link = app.prompts.link();
        let shown = deliver::read_dialog(&link, &prompt.terminal).await;
        if !shown.rows.is_empty() && dialog_key(&shown) == key {
            log("prompts", &format!("prompt {} still shows after the deep brain answer {ask}", prompt.id));
            app.info(&format!("Prompt {} in {} still shows the same question after the deep brain answer {ask}. Check its screen.", prompt.id, prompt.terminal));
        } else {
            log("prompts", &format!("prompt {} resolved in {} after the deep brain answer {ask}", prompt.id, prompt.terminal));
        }
    });
}

pub async fn answered(app: &Arc<App>, ask: &Ask) -> bool {
    if ask.kind.as_deref() != Some(ASK_KIND) {
        return false;
    }
    let Some(wait) = take_wait(app, &ask.id) else {
        log("prompts", &format!("deep brain answer {} has no open prompt", ask.id));
        return true;
    };
    let Some(prompt) = current(app, &wait) else {
        if ask.state == AskState::Answered {
            app.info(&notes::deep_late_note(&ask.id, &wait.prompt));
        }
        return true;
    };
    if ask.state != AskState::Answered {
        fallback(app, &prompt, &format!("{} did not answer prompt {} in time", ask.id, prompt.id));
        return true;
    }
    if let Err(why) = may_route(app, &prompt) {
        app.info(&format!("The deep brain answered {} for prompt {}, but it was not sent: {why}.", ask.id, prompt.id));
        fallback(app, &prompt, &why);
        return true;
    }
    let pick = ask.pick.clone().unwrap_or_default();
    let given = match answer_of_pick(&prompt, &pick) {
        Ok(given) => given,
        Err(why) => {
            fallback(app, &prompt, &format!("{} gave no usable answer: {why}", ask.id));
            return true;
        }
    };
    let reason = format!("deep brain {}: {}", ask.id, clip_flat(ask.why.as_deref().unwrap_or("its pick"), 200));
    match send_pick(app, &prompt, given, reason).await {
        Ok(text) if text.starts_with("Sent") => {
            app.info(&format!("The deep brain {} answered prompt {} in {}: {}.", ask.id, prompt.id, prompt.terminal, clip_flat(&pick, 120)));
            let next = text.contains(". Next:").then(|| lock(&app.prompts.book).get(&prompt.id).cloned()).flatten();
            match next {
                Some(open) => tell_again(app, &open),
                None => confirm_later(app, &prompt, &ask.id),
            }
        }
        Ok(text) => {
            app.info(&format!("The deep brain answer {} for prompt {} was not sent: {text}", ask.id, prompt.id));
            let still = lock(&app.prompts.book).get(&prompt.id).cloned();
            if let Some(open) = still {
                fallback(app, &open, &format!("{} gave an answer that could not be delivered", ask.id));
            }
        }
        Err(error) => fallback(app, &prompt, &format!("{} gave an answer that could not be delivered: {error}", ask.id)),
    }
    true
}

#[cfg(test)]
mod tests;
