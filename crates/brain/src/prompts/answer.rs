use super::book::{Answer, Choice, Prompt, PromptKind, Step, answer_from_click, plan};
use super::card::{describe, unread_answer};
use super::deliver::{self, Delivered};
use super::dialog::dialog_key;
use super::link::Link;
use super::{broadcast, notes, scan};
use crate::app::{App, lock};
use crate::util::{log, now_ms};
use regex::Regex;
use serde_json::json;
use std::sync::{Arc, LazyLock};

static SAID_NO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\W*(no|nope|don'?t|do not|stop|deny|reject|cancel)\b").expect("said no"));
static SAID_YES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\W*(yes|yeah|yep|ok|okay|sure|go ahead|allow|approve|do it)\b").expect("said yes"));
static ALWAYS_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(always|every time|don'?t ask again)\b").expect("always word"));

pub fn answer_problem(kind: PromptKind, choice: Option<Choice>, said: &str) -> Option<String> {
    let no = SAID_NO.is_match(said);
    let yes = SAID_YES.is_match(said);
    match choice {
        Some(Choice::Yes | Choice::Always) if no && !yes => Some("The user said no. Use choice no.".into()),
        Some(Choice::No) if yes && !no => Some("The user said yes. Use choice yes.".into()),
        Some(Choice::Always) if kind == PromptKind::Permission && !ALWAYS_WORD.is_match(said) => Some("The user did not say always. Use choice yes, which allows it once.".into()),
        _ => None,
    }
}

pub fn user_line_after(app: &App, at: u64) -> Option<(String, u64)> {
    let used = lock(&app.prompts.book).used_said_at;
    lock(&app.brain).last_user.clone().filter(|(_, said_at)| *said_at >= at && *said_at > used)
}

async fn run_step(link: &Link, prompt: &Prompt, step: Step) -> Delivered {
    let terminal = prompt.terminal.as_str();
    let question = prompt.question().cloned();
    match step {
        Step::Type(text) => deliver::type_text(link, terminal, &text).await,
        Step::Permission(choice) => deliver::answer_permission(link, terminal, choice).await,
        Step::Plan { approve, text } => deliver::answer_plan(link, terminal, approve, text.as_deref(), true).await,
        Step::Option(index) => deliver::answer_option(link, terminal, index, question.as_ref()).await,
        Step::Options(indexes) => deliver::answer_options(link, terminal, &indexes, question.as_ref()).await,
        Step::Text(text) => deliver::answer_text(link, terminal, &text).await,
    }
}

async fn deliver_answer(app: &Arc<App>, link: &Link, prompt: &Prompt, step: Step, said: &str) -> String {
    if !deliver::still_waiting(link, &prompt.terminal).await {
        lock(&app.prompts.book).remove(&prompt.id);
        return "It was already answered on screen.".into();
    }
    let shown = deliver::read_dialog(link, &prompt.terminal).await;
    let shown_key = (!shown.rows.is_empty()).then(|| dialog_key(&shown));
    if let (Some(key), Some(now_key)) = (&prompt.screen_key, &shown_key)
        && now_key != key
    {
        lock(&app.prompts.book).remove(&prompt.id);
        return "Its screen now shows a different prompt. Nothing was sent. The new prompt comes next.".into();
    }
    let done = run_step(link, prompt, step).await;
    log("prompts", &format!("answer {} to {}: {}", prompt.id, prompt.terminal, done.detail));
    if !done.ok {
        let unread = {
            let mut book = lock(&app.prompts.book);
            match book.get_mut(&prompt.id) {
                Some(open) => {
                    if done.detail.contains("no dialog") {
                        open.unread = true;
                    }
                    open.unread
                }
                None => false,
            }
        };
        return if unread { unread_answer(prompt) } else { format!("Could not send it. {}", done.detail) };
    }
    let now = now_ms();
    let next = {
        let mut book = lock(&app.prompts.book);
        book.note_answered(prompt, said, now);
        book.note_sent(&prompt.terminal, shown_key.clone(), now);
        let more = done.next && prompt.question_index + 1 < prompt.questions.len();
        match book.get_mut(&prompt.id) {
            Some(open) if more => {
                open.question_index += 1;
                open.asked_at = now;
                Some(open.clone())
            }
            _ => {
                book.remove(&prompt.id);
                None
            }
        }
    };
    let text = match next {
        Some(open) => format!("Sent {said}. Next: {}", describe(&open)),
        None => format!("Sent {said}."),
    };
    lock(&app.prompts.book).last_sent = Some(format!("Sent {said}."));
    app.info(&format!("Sent {said} to {}.", prompt.terminal));
    app.send_out(json!({"type": "prompt_sent", "id": prompt.id, "text": text, "terminal": prompt.terminal}));
    text
}

pub async fn answer(app: &Arc<App>, link: &Link, id: &str, given: &Answer) -> Result<String, String> {
    let prompt = lock(&app.prompts.book).get(id).cloned().ok_or_else(|| format!("No open prompt {}. It may be answered already.", id.trim()))?;
    if prompt.unread {
        return Ok(unread_answer(&prompt));
    }
    let (step, said) = plan(&prompt, given)?;
    {
        let mut book = lock(&app.prompts.book);
        if book.sending.is_some() {
            return Err(format!("{} is still sending another answer. Try again in a moment.", app.name()));
        }
        book.sending = Some(prompt.id.clone());
    }
    broadcast(app);
    let text = deliver_answer(app, link, &prompt, step, &said).await;
    lock(&app.prompts.book).sending = None;
    broadcast(app);
    Ok(text)
}

pub async fn answer_by_click(app: &Arc<App>, link: &Link, id: &str, indexes: &[usize]) -> String {
    let Some(prompt) = lock(&app.prompts.book).get(id).cloned() else { return format!("No open prompt {}. It may be answered already.", id.trim()) };
    if indexes.is_empty() {
        return "Pick an option first.".into();
    }
    let given = answer_from_click(&prompt, indexes);
    let result = answer(app, link, id, &given).await;
    let text = match result {
        Ok(text) => text,
        Err(error) => error,
    };
    if text.starts_with("Sent") {
        let said = text.trim_start_matches("Sent ").split(". Next:").next().unwrap_or("").trim_end_matches('.').to_string();
        let note = notes::clicked_note(&prompt.id, &said);
        scan::notify_parent(app, &prompt, true);
        let mut brain = lock(&app.brain);
        brain.carry = if brain.carry.is_empty() { note } else { format!("{}\n\n{note}", brain.carry) };
    } else {
        app.info(&text);
    }
    text
}

pub const HARD_YES: [&str; 12] = ["push", "deploy", "delete", "force", "publish", "release", "reset", "remove", "run", "script", "it", "that"];

pub async fn answer_by_tool(app: &Arc<App>, link: &Link, id: &str, given: Answer, decide: bool, reason: Option<String>) -> Result<String, String> {
    let prompt = lock(&app.prompts.book).get(id).cloned().ok_or_else(|| format!("No open prompt {}. It may be answered already.", id.trim()))?;
    let said = user_line_after(app, prompt.asked_at);
    if decide {
        if super::decide_first::hard_stop(&prompt) {
            return Err(format!("Prompt {} is a hard stop (push, deploy, delete, force or a downloaded script). Only the user answers it, on its card or in chat. Do not answer it with decide true.", prompt.id));
        }
        if let Some(ask) = super::auto::deciding(app, &prompt) {
            return Err(format!("The deep brain is deciding prompt {} as {ask}. Do not answer it: {} sends its pick once.", prompt.id, app.name()));
        }
        let mut given = given;
        if given.choice == Some(Choice::Always) {
            given.choice = Some(Choice::Yes);
        }
        let reason = reason.map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).ok_or("Pass a short reason why this option fits when decide is true.")?;
        let text = answer(app, link, id, &given).await?;
        if text.starts_with("Sent") {
            if let Some((_, at)) = said {
                lock(&app.prompts.book).used_said_at = at;
            }
            app.info(&format!("{} answered {} by itself. Why: {}", app.name(), prompt.id, crate::util::clip_flat(&reason, 200)));
        }
        return Ok(text);
    }
    let Some((said_text, said_at)) = said else {
        return Err("Do not wait for the user. Call again with decide true and a short reason.".into());
    };
    if let Some(problem) = answer_problem(prompt.kind, given.choice, &said_text) {
        return Err(problem);
    }
    if given.choice != Some(Choice::No) && super::decide_first::hard_stop(&prompt) && !crate::yes_once::says_yes_to(&said_text, &HARD_YES) {
        return Err(format!("Prompt {} is a hard stop. The user's last line is not a yes for it. Ask the user, or let them answer on its card.", prompt.id));
    }
    let text = answer(app, link, id, &given).await?;
    if text.starts_with("Sent") {
        lock(&app.prompts.book).used_said_at = said_at;
    }
    Ok(text)
}

pub async fn tick(app: &Arc<App>) {
    let link = app.prompts.link();
    scan::scan(app, &link).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_problems() {
        assert!(answer_problem(PromptKind::Permission, Some(Choice::Yes), "no, stop").is_some());
        assert!(answer_problem(PromptKind::Permission, Some(Choice::No), "yes go").is_some());
        assert!(answer_problem(PromptKind::Permission, Some(Choice::Always), "yes").is_some());
        assert!(answer_problem(PromptKind::Permission, Some(Choice::Always), "yes, always").is_none());
        assert!(answer_problem(PromptKind::Question, None, "the second one").is_none());
        assert!(answer_problem(PromptKind::Plan, Some(Choice::Yes), "ok do it").is_none());
    }
}
