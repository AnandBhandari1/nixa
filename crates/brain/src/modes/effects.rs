use super::journal::{DecisionInput, quoted};
use super::keywords::{Match, MatchState, match_line};
use super::{clock, complaints, work};
use crate::app::{App, lock};
use crate::chat::{Extra, Who};
use crate::settings_api::work::{Hold, Topic, TopicKind};
use crate::util::{log, now_ms};
use regex::Regex;
use std::sync::Arc;

pub fn carry(app: &App, note: String) {
    let mut brain = lock(&app.brain);
    brain.carry = if brain.carry.is_empty() { note } else { format!("{}\n\n{note}", brain.carry) };
}

pub fn answer_now(app: &Arc<App>, reply: &str) {
    app.add_chat(Who::Nixa, reply, Extra::default());
    crate::talk::speech::say_later_as(app, reply, crate::talk::speech::SayOptions::nixa("chat"), crate::talk::hold::Pass::Last);
}

pub fn strip_name(name: &str, text: &str) -> String {
    let names: Vec<String> = [name, crate::app::DEFAULT_NAME].iter().map(|n| regex::escape(n.trim())).filter(|n| !n.is_empty()).collect();
    let Ok(lead) = Regex::new(&format!(r"(?i)^\s*(?:(?:hey|hi|ok|okay)[\s,]+)?(?:{})\b[\s,.:;!-]*", names.join("|"))) else { return text.trim().to_string() };
    lead.replace(text.trim(), "").trim().to_string()
}

fn last_said(app: &App) -> Option<String> {
    lock(&app.chat).list().iter().rev().find(|i| i.who == Who::Nixa).map(|i| i.text.clone())
}

fn row(said: &str, item: String, reason: &str) -> DecisionInput {
    DecisionInput { kind: "keyword".into(), choice: said.to_string(), item: Some(item), reason: Some(reason.to_string()), ..DecisionInput::default() }
}

pub fn run_keyword(app: &Arc<App>, found: &Match, said: &str) -> bool {
    let Match { row: keyword, alias, rest } = found;
    log("modes", &format!("keyword {} heard ({alias})", keyword.id));
    let item = format!("keyword:{}:{alias}", keyword.id);
    match keyword.effect.as_str() {
        "hold" => {
            let was = {
                let mut now = lock(&app.switches.work);
                let was = now.hold.is_some();
                if !was {
                    now.hold = Some(Hold { at: now_ms(), alias: alias.clone() });
                }
                was
            };
            lock(&app.modes.live).pass = None;
            super::save(app);
            if !was {
                super::work_row(app, row(said, item, "hold on"));
            }
            carry(app, hold_note(alias));
            answer_now(app, &hold_reply(was));
            super::send_state(app);
            true
        }
        "go-on" => {
            lock(&app.switches.work).hold = None;
            super::save(app);
            super::work_row(app, row(said, item, "hold off"));
            let count = waited_line(crate::prompts::scan::untold_count(app));
            carry(app, format!("Note: the user said {}: hold is off. Work runs again.{count}", quoted(alias)));
            answer_now(app, &format!("Going on.{count}"));
            crate::prompts::scan::tell_untold(app);
            super::send_state(app);
            true
        }
        "plan-topic" => {
            let topic = Topic { kind: TopicKind::Discuss, text: rest.trim().to_string(), at: now_ms(), alias: alias.clone() };
            lock(&app.switches.work).topic = Some(topic.clone());
            lock(&app.modes.live).pass = None;
            super::save(app);
            super::work_row(app, row(said, item, "discuss on"));
            app.info(DISCUSS_INFO);
            let about = if topic.text.is_empty() { String::new() } else { format!(" Topic: {}.", quoted(&topic.text)) };
            carry(app, discuss_note(&about));
            super::send_state(app);
            false
        }
        "act" => {
            let was = lock(&app.switches.work).topic.take();
            super::give_pass(app, alias);
            super::save(app);
            let hold_on = lock(&app.switches.work).hold.is_some();
            let rests = if hold_on { " Hold stays on for the rest.".to_string() } else { String::new() };
            let reason = match &was {
                Some(t) => format!("{} off, this job runs", work::topic_kind(t.kind)),
                None => "this job runs".into(),
            };
            super::work_row(app, row(said, item, &reason));
            app.info(&format!("Assign: this job runs now.{rests}"));
            let talk = was
                .as_ref()
                .map(|t| {
                    let about = if t.text.is_empty() { String::new() } else { format!(" about {}", quoted(&t.text)) };
                    format!("the talk{about} since {} is done. ", clock::clock(t.at))
                })
                .unwrap_or_default();
            carry(app, format!("Note: Keyword assign: {talk}the user says to assign the agreed work now; start it in this turn.{rests}"));
            super::send_state(app);
            false
        }
        "review-now" => {
            super::work_row(app, row(said, item, "check now"));
            let what = if rest.is_empty() { String::new() } else { format!(" ({rest})") };
            carry(app, format!("Note: Keyword review: check the agents that finished{what} now and report. Close a job or send one fix. Start no new work from this line."));
            super::send_state(app);
            false
        }
        _ => false,
    }
}

pub fn hold_note(alias: &str) -> String {
    format!(
        "Note: the user said {}: hold is on. Start no new work: no new agent, task, team run or schedule. Agent questions, checks and fixes still run, and the hold ends by itself {} minutes after it began or when they say go on.",
        quoted(alias),
        work::HOLD_MS / 60_000
    )
}

pub fn hold_reply(was: bool) -> String {
    if was {
        return "Hold is already on. No new work starts; it ends by itself, or say go on.".into();
    }
    format!("Hold is on. No new work starts. Agent questions and checks still run, and the hold ends by itself in {} minutes.", work::HOLD_MS / 60_000)
}

pub const DISCUSS_INFO: &str = "Discuss: we talk it through, and the work goes on.";

pub fn discuss_note(about: &str) -> String {
    format!("Note: Keyword discuss: talk it through with the user.{about} Code holds no step for it: work goes on and each step goes in the decision journal. Do not wait for assign.")
}

pub fn waited_line(waiting: usize) -> String {
    match waiting {
        0 => String::new(),
        1 => " 1 agent question waited.".into(),
        n => format!(" {n} agent questions waited."),
    }
}

fn hands_over(app: &Arc<App>, rest: &str, said: &str) {
    let deep = app.decision.deep_mode();
    if !(work::asks_work(rest) || (deep && work::hands_to_deep(rest))) {
        return;
    }
    let Some(was) = lock(&app.switches.work).topic.take() else { return };
    super::save(app);
    let kind = work::topic_kind(was.kind);
    let name = if was.kind == TopicKind::Plan { "Plan" } else { "Discuss" };
    super::work_row(app, row(said, format!("keyword:{kind}:{}", was.alias), &format!("{kind} off, the user asked for the work")));
    app.info(&format!("{name} is off: you asked for the work."));
    let deep_part = if deep { "; a choice in it goes to the deep brain with nixa_deep_ask" } else { "" };
    carry(
        app,
        format!("Note: the user asked for the work in their own words, so {name} is off. Do what they asked now{deep_part}. Do not wait for assign and do not ask them to say it again."),
    );
    super::send_state(app);
}

pub fn heard(app: &Arc<App>, text: &str, typed: bool) -> bool {
    noted(app, text, typed);
    let rest = strip_name(&app.name(), text);
    let rows = lock(&app.switches.keywords).rows.clone();
    let state = MatchState { held: super::any_held(app), hold_on: lock(&app.switches.work).hold.is_some() };
    if let Some(found) = match_line(&rest, &rows, state) {
        return run_keyword(app, &found, text);
    }
    if lock(&app.switches.work).topic.is_some() {
        hands_over(app, &rest, text);
    }
    false
}

pub fn room_heard(app: &Arc<App>, text: &str) {
    noted(app, text, false);
}

fn noted(app: &Arc<App>, text: &str, typed: bool) {
    super::restore(app);
    if complaints::is_complaint(text) {
        let nixa = last_said(app);
        match complaints::add(&app.modes.complaints_file, text, typed, nixa.as_deref(), now_ms()) {
            Ok(_) => {
                app.info("Saved as feedback for review.");
                log("modes", &format!("complaint saved ({} chars)", text.chars().count()));
            }
            Err(e) => log("modes", &format!("could not save a complaint: {e}")),
        }
    }
    super::correction(app, text);
}
