use super::confirm::{CONFIRM_VOICE_MS, End, Said, Step, WAIT_LINE, confirm_step, end_line, spoken_confirm, timeout_note};
use super::gather::{Message, Piece};
use super::hold::Pass;
use super::heard::{HeardLine, NO_AFTER_MS, NO_REPLY_TEXT, NOTHING_SAID, earlier_heard_note, keep_heard, silent_to_user, skip_heard};
use super::presence::{
    QUIET_NOT_ON, QUIET_PANEL_END, parse_quiet, quiet_end_note, quiet_end_text, quiet_start_note, quiet_start_text,
};
use super::progress::{PROGRESS_CHARS, first_sentence};
use super::speech::{self, SayOptions};
use super::words::{Reply, SmallTalk, one_letter};
use crate::agent;
use crate::app::{App, lock};
use crate::chat::{Extra, Who};
use crate::reply::AgentReply;
use crate::tools::{Args, ToolResult, need};
use crate::util::{clip, log, now_ms};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub const REPEAT_CLIP: usize = 200;

pub fn carry(app: &App, note: &str) {
    if note.is_empty() {
        return;
    }
    let mut brain = lock(&app.brain);
    brain.carry = if brain.carry.is_empty() { note.to_string() } else { format!("{}\n\n{note}", brain.carry) };
}

fn user_line(app: &App, text: &str, typed: bool, quiet: bool) {
    app.add_chat(Who::User, text, Extra { typed, quiet, ..Extra::default() });
}

fn nixa_line(app: &App, text: &str) {
    app.add_chat(Who::Nixa, text, Extra { routine: true, ..Extra::default() });
}

pub fn prompts_open(app: &App) -> usize {
    lock(&app.prompts.book).items.len()
}

fn open_confirm(app: &App) -> Option<(u64, String, u64, u64, bool)> {
    lock(&app.talk.confirms).open.as_ref().map(|a| (a.id, a.message.clone(), a.asked_at, a.ready_at, a.wait_said))
}

pub fn confirm_open(app: &App) -> bool {
    lock(&app.talk.confirms).open.is_some()
}

fn say_then_ready(app: &Arc<App>, text: String, id: Option<u64>, pass: Pass) {
    say_then_ready_as(app, text, id, pass, SayOptions::nixa("attention"));
}

fn say_then_ready_as(app: &Arc<App>, text: String, id: Option<u64>, pass: Pass, options: SayOptions) {
    let app = app.clone();
    tokio::spawn(async move {
        speech::say_as(&app, &text, options, pass).await;
        if let Some(id) = id {
            lock(&app.talk.confirms).ready(id, now_ms());
        }
    });
}

pub fn touch(app: &Arc<App>) {
    lock(&app.talk.presence).touch(now_ms());
}

pub fn touch_later(app: &Arc<App>) {
    touch(app);
}

fn stop_all(app: &Arc<App>, text: &str, typed: bool) {
    user_line(app, text, typed, false);
    lock(&app.talk.heard).stopped_at = now_ms();
    speech::stop(app);
    finish_confirm(app, false, None, End::No);
    if agent::interrupt(app) {
        log("talk", "stopped the brain");
    }
    lock(&app.talk.progress).reset();
    app.send_out(json!({"type": "stopped"}));
}

fn repeat(app: &Arc<App>, text: &str) {
    user_line(app, text, false, false);
    if speech::speaking(app) {
        speech::stop(app);
    }
    let open = open_confirm(app);
    let again = match &open {
        Some((_, message, ..)) => Some((spoken_confirm(message), false)),
        None => speech::last_said_mac(app),
    };
    let Some((again, mac_only)) = again else {
        nixa_line(app, NOTHING_SAID);
        speech::say_later_as(app, NOTHING_SAID, SayOptions::nixa("chat"), Pass::Last);
        return;
    };
    app.add_chat(Who::Info, &format!("Said again: {}", clip(&again, REPEAT_CLIP)), Extra { mac_only, ..Extra::default() });
    let id = match open {
        Some(_) => lock(&app.talk.confirms).reask(now_ms()).map(|(id, _)| id),
        None => None,
    };
    super::send_state(app);
    say_then_ready_as(app, again, id, Pass::Last, SayOptions { mac_only, ..SayOptions::nixa("attention") });
}

fn confirm_heard(app: &Arc<App>, said: &Said) -> Option<bool> {
    let (id, message, asked_at, ready_at, wait_said) = open_confirm(app)?;
    let talk = super::talk(app);
    let reply = if said.typed { talk.typed_reply(&said.text) } else { talk.confirm_reply(&said.text) };
    let step = confirm_step(reply, said, asked_at, ready_at);
    log("confirm", &format!("heard, step {step:?}"));
    match step {
        Step::Yes | Step::No => {
            user_line(app, &said.text, said.typed, false);
            finish_confirm(app, step == Step::Yes, Some(id), End::No);
            Some(false)
        }
        Step::Wait => {
            user_line(app, &said.text, said.typed, !said.typed);
            if reply == Some(Reply::Yes) && said.at.saturating_sub(ready_at) > CONFIRM_VOICE_MS {
                let id = lock(&app.talk.confirms).reask(now_ms()).map(|(id, _)| id);
                super::send_state(app);
                say_then_ready(app, spoken_confirm(&message), id, Pass::Last);
                return Some(true);
            }
            if !wait_said {
                if let Some(open) = lock(&app.talk.confirms).open.as_mut() {
                    open.wait_said = true;
                }
                app.info(WAIT_LINE);
            }
            Some(false)
        }
        Step::Cancel => {
            finish_confirm(app, false, Some(id), End::No);
            None
        }
        Step::Pass => None,
    }
}

fn quiet_heard(app: &Arc<App>, said: &Said, ms: u64) {
    user_line(app, &said.text, said.typed, false);
    let now = now_ms();
    let (was, until, now_quiet, held) = {
        let mut p = lock(&app.talk.presence);
        let was = p.quiet(now);
        let until = p.set_quiet(now, ms);
        let held = if ms == 0 && was { p.take() } else { Vec::new() };
        (was, until, p.quiet(now), held)
    };
    lock(&app.talk.heard).quiet_was = now_quiet;
    let reply = if ms > 0 {
        quiet_start_text(until)
    } else if was {
        quiet_end_text(&held)
    } else {
        QUIET_NOT_ON.to_string()
    };
    nixa_line(app, &reply);
    super::send_state(app);
    speech::say_later_as(app, &reply, SayOptions::nixa("chat"), Pass::Last);
    let name = app.name();
    carry(app, &if ms > 0 { quiet_start_note(until, &name) } else { quiet_end_note(&reply, &name) });
    log("presence", if ms > 0 { "quiet on" } else { "quiet off" });
}

fn says_no(text: &str) -> bool {
    matches!(nixa_voice::normalise(text).split(' ').next(), Some("no" | "nope" | "nah"))
}

pub async fn hear(app: &Arc<App>, said: Said) {
    if !turn(app, said).await {
        super::hold::answered(app);
    }
}

async fn turn(app: &Arc<App>, said: Said) -> bool {
    let text = said.text.trim().to_string();
    if text.is_empty() {
        return false;
    }
    let said = Said { text: text.clone(), ..said };
    touch(app);
    if !said.typed {
        app.send_out(json!({"type": "heard", "text": text}));
    }
    let talk = super::talk(app);
    if talk.is_stop(&text) {
        stop_all(app, &text, said.typed);
        return false;
    }
    if !said.typed && talk.wants_repeat(&text) {
        repeat(app, &text);
        return true;
    }
    if said.mac
        && let Some(follows) = confirm_heard(app, &said)
    {
        return follows;
    }
    let called = !said.typed && (talk.named(&text) || said.pressed);
    let always = crate::voice::settings(app).listen == crate::voice::Listen::Always;
    let quiet_ms = if said.typed || called || always {
        parse_quiet(&text).or_else(|| {
            let (named, rest) = talk.strip_name(&text);
            if named { parse_quiet(&rest) } else { None }
        })
    } else {
        None
    };
    if let Some(ms) = quiet_ms {
        quiet_heard(app, &said, ms);
        return true;
    }
    let open_prompts = prompts_open(app);
    let kind = if said.typed { None } else { talk.small_talk(&text) };
    let letter = open_prompts > 0 && one_letter(&text);
    let now = now_ms();
    let (last_spoke_at, last_ask_at) = {
        let s = lock(&app.talk.speaker);
        (s.last_spoke_at, s.last_ask_at)
    };
    let no_to_work =
        kind == Some(SmallTalk::Assent) && says_no(&text) && (agent::busy(app) || now.saturating_sub(last_spoke_at) < NO_AFTER_MS);
    if !letter && !no_to_work && skip_heard(kind, said.typed, confirm_open(app), open_prompts, now.saturating_sub(last_ask_at)) {
        user_line(app, &text, said.typed, true);
        log("talk", &format!("skipped small talk ({kind:?})"));
        return false;
    }
    if kind == Some(SmallTalk::Assent) {
        lock(&app.talk.speaker).last_ask_at = 0;
    }
    if speech::speaking(app) {
        speech::stop(app);
    }
    if !said.typed {
        let note = {
            let mut heard = lock(&app.talk.heard);
            let note = earlier_heard_note(&heard.before, said.at);
            keep_heard(&mut heard.before, HeardLine { text: text.clone(), at: said.at });
            note
        };
        carry(app, &note);
    }
    if super::room::heard_in_room(app, &text, said.typed, called, said.started_at, last_spoke_at, last_ask_at) {
        log("talk", "heard without the name, passed on as room talk");
        agent::room_said(app, &text);
        return true;
    }
    if said.mac {
        agent::user_said(app, &text, said.typed);
    } else {
        agent::outside_said(app, &text, said.typed);
    }
    true
}

pub fn hear_later(app: &Arc<App>, said: Said) {
    let app = app.clone();
    tokio::spawn(async move { hear(&app, said).await });
}

pub fn not_called(app: &Arc<App>, text: &str) -> bool {
    let talk = super::talk(app);
    let card = prompts_open(app) > 0 && talk.is_card_reply(text);
    if !confirm_open(app) && !card {
        return false;
    }
    let now = now_ms();
    hear_later(app, Said::heard(text, now, now));
    true
}

pub fn on_speech_start(app: &App) {
    lock(&app.talk.gather).speech_start();
}

pub fn on_speech_end(app: &App) {
    lock(&app.talk.gather).speech_end(now_ms());
}

pub fn on_heard(app: &Arc<App>, text: &str, stt_ms: u64, audio_ms: u64) {
    let now = now_ms();
    let captured_at = now.saturating_sub(stt_ms);
    let started_at = captured_at.saturating_sub(audio_ms);
    if super::talk(app).is_stop(text.trim()) {
        lock(&app.talk.gather).drop_all();
        hear_later(app, Said::heard(text, started_at, captured_at));
        return;
    }
    let message = lock(&app.talk.gather).phrase(now, Piece { text: text.to_string(), started_at, captured_at });
    if let Some(message) = message {
        gathered(app, message);
    }
}

pub fn on_held(app: &Arc<App>, text: &str, stt_ms: u64, audio_ms: u64) {
    let captured_at = now_ms().saturating_sub(stt_ms);
    let said = Said { pressed: true, ..Said::heard(text, captured_at.saturating_sub(audio_ms), captured_at) };
    hear_later(app, said);
}

pub fn level_event(level: f32, talking: bool) -> serde_json::Value {
    json!({"type": "level", "mic": (f64::from(level) * 100.0).round() / 100.0, "talking": talking})
}

pub fn on_level(app: &App, rms: f32, talking: bool) {
    let sent = lock(&app.talk.meter).push_rms(now_ms(), rms, talking);
    if let Some((level, talking)) = sent {
        app.send_out(level_event(level, talking));
    }
}

pub fn mic_off(app: &App) {
    lock(&app.talk.gather).drop_all();
    let (level, talking) = lock(&app.talk.meter).reset(now_ms());
    app.send_out(level_event(level, talking));
}

fn gathered(app: &Arc<App>, message: Message) {
    let talk = super::talk(app);
    if message.parts.iter().any(|p| talk.is_stop(p)) {
        return;
    }
    if message.parts.len() > 1 {
        log("talk", &format!("joined {} heard parts", message.parts.len()));
    }
    hear_later(app, Said::heard(&message.text(), message.started_at, message.captured_at));
}

pub fn open_ask(app: &Arc<App>, message: &str, why: &str) -> Option<(u64, oneshot::Receiver<bool>)> {
    let (id, answer) = lock(&app.talk.confirms).ask(message, why, now_ms())?;
    log("confirm", &format!("asked {id}"));
    super::send_state(app);
    say_then_ready(app, spoken_confirm(message), Some(id), Pass::Notice);
    Some((id, answer))
}

pub async fn ask_confirm(app: &Arc<App>, message: &str, why: &str, cancel: Option<CancellationToken>) -> bool {
    let Some((id, answer)) = open_ask(app, message, why) else { return false };
    wait_ask(app, id, answer, cancel).await
}

pub async fn wait_ask(app: &Arc<App>, id: u64, answer: oneshot::Receiver<bool>, cancel: Option<CancellationToken>) -> bool {
    match cancel {
        Some(token) => tokio::select! {
            ok = answer => ok.unwrap_or(false),
            _ = token.cancelled() => {
                finish_confirm(app, false, Some(id), End::Cancelled);
                false
            }
        },
        None => answer.await.unwrap_or(false),
    }
}

pub fn finish_confirm(app: &App, ok: bool, id: Option<u64>, end: End) -> bool {
    let done = lock(&app.talk.confirms).finish(ok, id, end);
    let Some((ask, end)) = done else { return false };
    log("confirm", end.word());
    app.info(&end_line(&ask, end));
    crate::modes::record(
        app,
        crate::modes::journal::DecisionInput { kind: "confirm".into(), choice: ask.message.clone(), reason: Some(ask.why.clone()).filter(|w| !w.is_empty()), outcome: Some(end.word().to_lowercase()), ..Default::default() },
    );
    if end == End::Timeout {
        carry(app, &timeout_note(&ask));
    }
    super::send_state(app);
    true
}

pub fn set_quiet_panel(app: &App, minutes: f64) {
    let ms = if minutes.is_finite() && minutes > 0.0 { (minutes * 60_000.0).round() as u64 } else { 0 };
    let until = lock(&app.talk.presence).set_quiet(now_ms(), ms);
    app.info(&if ms > 0 { quiet_start_text(until) } else { QUIET_PANEL_END.to_string() });
    log("presence", if ms > 0 { "quiet on from the panel" } else { "quiet off from the panel" });
    super::send_state(app);
}

fn progress_active(app: &App) -> bool {
    agent::busy(app)
}

fn progress_blocked(app: &App, now: u64) -> bool {
    let away = {
        let p = lock(&app.talk.presence);
        p.quiet(now)
    };
    away || speech::busy(app) || confirm_open(app) || speech::user_talking(app)
}

fn publish(app: &Arc<App>, sentence: &str) {
    nixa_line(app, sentence);
    app.send_out(json!({"type": "progress", "text": sentence}));
    log("progress", &format!("update, {} chars", sentence.chars().count()));
    speech::say_later(app, sentence, SayOptions::nixa("chat"));
}

pub fn progress_call(app: &Arc<App>, args: &Args) -> ToolResult {
    let text = need(args, "text")?;
    if text.chars().count() > PROGRESS_CHARS {
        return Err(format!("Keep progress within {PROGRESS_CHARS} characters."));
    }
    let now = now_ms();
    let (active, blocked) = (progress_active(app), progress_blocked(app, now));
    let (offer, published) = lock(&app.talk.progress).offer(now, active, blocked, &first_sentence(&text));
    if let Some(sentence) = published {
        publish(app, &sentence);
    }
    Ok(json!({"status": offer.word()}).to_string())
}

pub fn turn_ended(app: &App) {
    lock(&app.talk.progress).reset();
    finish_confirm(app, false, None, End::Cancelled);
}

pub fn no_reply(app: &Arc<App>, user_turn: bool, reply: &AgentReply) -> bool {
    let since_stop = now_ms().saturating_sub(lock(&app.talk.heard).stopped_at);
    if !silent_to_user(user_turn, reply, since_stop) {
        return false;
    }
    log("talk", "the brain gave no reply to the user");
    nixa_line(app, NO_REPLY_TEXT);
    speech::say_later_as(app, NO_REPLY_TEXT, SayOptions::nixa("chat"), Pass::Last);
    true
}

fn quiet_end(app: &Arc<App>, now: u64) {
    let quiet = lock(&app.talk.presence).quiet(now);
    let ended = {
        let mut heard = lock(&app.talk.heard);
        let ended = heard.quiet_was && !quiet;
        heard.quiet_was = quiet;
        ended
    };
    if !ended {
        return;
    }
    let held = lock(&app.talk.presence).take();
    let text = quiet_end_text(&held);
    nixa_line(app, &text);
    super::send_state(app);
    speech::speak_later(app, &text, SayOptions::nixa("chat"));
}

pub fn tick(app: &Arc<App>, now: u64) {
    super::hold::tick(app, now);
    let message = lock(&app.talk.gather).due(now);
    if let Some(message) = message {
        gathered(app, message);
    }
    let expired = lock(&app.talk.confirms).expired(now);
    if let Some(id) = expired {
        finish_confirm(app, false, Some(id), End::Timeout);
    }
    quiet_end(app, now);
    let (active, blocked) = (progress_active(app), progress_blocked(app, now));
    let sentence = lock(&app.talk.progress).flush(now, active, blocked);
    if let Some(sentence) = sentence {
        publish(app, &sentence);
    }
}
