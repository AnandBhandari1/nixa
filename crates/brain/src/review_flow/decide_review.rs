use super::settle::{self, Facts, Settled};
use crate::app::App;
use crate::decide::{self, Answer, Answers, settings};
use crate::util::{clip, log};
use nixa_store::tasks::{Task, TaskStatus, must_task, task_comments};
use std::sync::Arc;

pub const VERDICT: &str = "verdict";
pub const FINISHED: &str = "finished";
pub const DONE: &str = "done";
pub const FIX: &str = "fix";
pub const UNCLEAR: &str = "unclear";
pub const VERDICT_TEXT: &str = "Did the agent finish every part the task asked for? done = yes, fix = a part is missing or broken, unclear = no report to judge.";
pub const FINISHED_TEXT: &str = "The agent's report says every part the task asked for is finished.";

pub fn why_not(facts: &Facts, question: &str) -> Option<&'static str> {
    if facts.unread || facts.no_base {
        return Some("Nixa could not read the changes");
    }
    if facts.checks.as_ref().is_some_and(|c| c.failed) || question.contains("failing check") {
        return Some("a check failed");
    }
    if facts.changed.is_empty() {
        return Some("there are no changed files to judge");
    }
    None
}

pub fn questions() -> Vec<serde_json::Value> {
    vec![
        decide::choice(VERDICT, VERDICT_TEXT, &[(DONE, ""), (FIX, ""), (UNCLEAR, "")]),
        decide::predicate(FINISHED, FINISHED_TEXT),
    ]
}

pub fn input(id: i64, title: &str, description: &str, report: &str, facts: &Facts) -> String {
    let mut lines = vec![format!("Task #{id}: {title}")];
    if !description.trim().is_empty() {
        lines.push(format!("Task description: {}", clip(description.trim(), 1500)));
    }
    lines.push(format!("Changed files: {}", crate::agent::data_block(&clip(&facts.changed.join(", "), 800))));
    if let Some(checks) = &facts.checks {
        lines.push(format!("Checks: {}", crate::agent::data_block(&clip(&checks.text, 400))));
    }
    lines.push(format!("Agent report: {}", if report.trim().is_empty() { "(none)".to_string() } else { crate::agent::data_block(&clip(report.trim(), 2000)) }));
    lines.join("\n")
}

fn verdict_of(answers: &Answers) -> Option<(String, f64)> {
    match answers.get(VERDICT) {
        Some(Answer::Choice { choice, confidence, .. }) => Some((choice.as_str().unwrap_or_default().to_string(), *confidence)),
        _ => None,
    }
}

pub fn close(answers: &Answers, level: f64) -> Result<(), String> {
    let finished = match answers.get(FINISHED) {
        Some(Answer::Predicate { probability, .. }) => *probability,
        _ => 0.0,
    };
    let (verdict, confidence) = verdict_of(answers).ok_or("it gave no verdict")?;
    let note = format!("verdict {verdict} ({confidence:.2}), finished {finished:.2}");
    if verdict != DONE {
        return Err(format!("it says {note}"));
    }
    if confidence < level || finished < level {
        return Err(format!("it was not sure enough: {note}, level {level:.2}"));
    }
    Ok(())
}

pub fn take(app: &Arc<App>, task: &Task, facts: Option<&Facts>, question: &str) -> bool {
    let Some(facts) = facts else { return false };
    if !crate::settings_api::nixa_on(&app.dir) {
        return false;
    }
    let set = settings::load(app);
    if !set.uses("review") {
        return false;
    }
    if let Some(why) = why_not(facts, question) {
        log("decide", &format!("task #{} review goes the current way: {why}", task.id));
        return false;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return false };
    let app = app.clone();
    let task = task.clone();
    let facts = facts.clone();
    let question = question.to_string();
    handle.spawn(async move {
        let extra = match attempt(&app, &task, &facts, &set).await {
            Ok(()) => return,
            Err(why) => why,
        };
        log("decide", &format!("task #{} review goes to the chat brain: {extra}", task.id));
        super::ask_chat(&app, &task, &format!("{question} Decision model: {extra}."));
    });
    true
}

async fn attempt(app: &Arc<App>, task: &Task, facts: &Facts, set: &settings::Settings) -> Result<(), String> {
    let report = {
        let conn = nixa_store::open(&app.db_file).map_err(|e| format!("{e:#}"))?;
        let comments = task_comments(&conn, task.id).map_err(|e| format!("{e:#}"))?;
        settle::step_report(task, &comments)
    };
    let answers = decide::ask(app, &set.model, vec![decide::text(&input(task.id, &task.title, &task.description, &report, facts))], questions()).await?;
    close(&answers, set.level)?;
    let note = answers.answers.iter().map(Answer::short).collect::<Vec<_>>().join(", ");
    let settled = finish_now(app, task, facts, &note)?;
    for effect in &settled.effects {
        if let settle::Effect::Info { text } = effect {
            app.info(text);
        }
    }
    app.info(&format!("The decision model {} closed task #{} as done ({note}).", set.model, task.id));
    app.send_out(serde_json::json!({"type": super::EVENT_KIND, "changed": true}));
    Ok(())
}

fn finish_now(app: &App, task: &Task, facts: &Facts, note: &str) -> Result<Settled, String> {
    let fail = |e: anyhow::Error| format!("{e:#}");
    let conn = nixa_store::open(&app.db_file).map_err(fail)?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let fresh = must_task(&tx, task.id).map_err(fail)?;
    if fresh.status == TaskStatus::Done || fresh.terminal_id != task.terminal_id {
        return Err("the task changed before the answer".into());
    }
    let summary = format!("Checked by the decision model: {note}. Changed files: {}.", clip(&facts.changed.join(", "), 300));
    let settled = settle::finish_checked(&tx, &fresh, facts, &summary, true).map_err(fail)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(settled)
}

#[cfg(test)]
mod tests;
