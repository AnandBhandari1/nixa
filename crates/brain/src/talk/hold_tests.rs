use super::hold::{ANSWER_WAIT_MS, Hold, Pass, RECORD_MAX_MS, key};
use super::speech::{self, PhoneSay, SayOptions};
use super::{Said, flow, hold};
use crate::app::{App, lock};
use crate::chat::Who;
use crate::util::now_ms;
use nixa_voice::SayResult;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

type Played = Arc<Mutex<Vec<String>>>;

fn fake_phone(app: &Arc<App>) -> Played {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PhoneSay>();
    speech::set_phone(app, Some(tx));
    lock(&app.voice.settings).voice_on = true;
    let played: Played = Arc::new(Mutex::new(Vec::new()));
    let out = played.clone();
    tokio::spawn(async move {
        while let Some(job) = rx.recv().await {
            out.lock().unwrap().push(job.text.clone());
            let _ = job.done.send(SayResult::default());
        }
    });
    played
}

fn played(list: &Played) -> Vec<String> {
    list.lock().unwrap().clone()
}

async fn wait_for(list: &Played, count: usize) -> Vec<String> {
    for _ in 0..250 {
        if list.lock().unwrap().len() >= count {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    played(list)
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

fn notice(app: &Arc<App>, text: &str) {
    speech::speak_later(app, text, SayOptions { agent: Some("claude".into()), kind: Some("blocked".into()), ..SayOptions::default() });
}

fn held(app: &App) -> Vec<String> {
    lock(&app.talk.hold).texts()
}

#[tokio::test]
async fn stopping_a_released_task_read_preserves_the_next_notice() {
    let (_dir, app) = app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PhoneSay>();
    speech::set_phone(&app, Some(tx));
    crate::app::lock(&app.voice.settings).voice_on = true;

    hold::start(&app, "p1");
    assert!(hold::read_task(&app, "Task 1: Read this.", 1).is_some());
    assert!(hold::catch(&app, "The build passed.", &SayOptions::nixa("chat")));
    hold::cancel(&app, "p1");

    let read = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await.unwrap().unwrap();
    assert_eq!(read.text, "Task 1: Read this.");
    assert_eq!(hold::stop_reads(&app), 0);
    tokio::time::timeout(Duration::from_secs(3), read.stop.cancelled()).await.unwrap();
    read.done.send(SayResult { interrupted: true, ms: 0 }).unwrap();

    let note = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await.unwrap().unwrap();
    assert_eq!(note.text, "The build passed.");
    assert!(!note.stop.is_cancelled());
    note.done.send(SayResult::default()).unwrap();
}

#[test]
fn the_hold_follows_the_recording() {
    let mut h = Hold::default();
    assert!(!h.on() && !h.holds(Pass::Notice));
    h.start("p1", 1000);
    assert!(h.holds(Pass::Notice) && h.holds(Pass::Answer) && h.holds(Pass::Last));
    assert!(!h.end("p2", 2000));
    assert!(h.end("p1", 2000));
    assert!(h.on());
    assert!(h.holds(Pass::Notice));
    assert!(!h.holds(Pass::Answer) && !h.holds(Pass::Last));
    assert!(h.answered());
    assert!(!h.on());
    assert!(!h.answered());
}

#[test]
fn two_phones_hold_until_both_stop() {
    let mut h = Hold::default();
    h.start("p1", 0);
    h.start("p2", 10);
    assert!(h.cancel("p1"));
    assert!(h.holds(Pass::Last));
    assert!(!h.answered());
    assert!(h.cancel("p2"));
    assert!(!h.on());
}

#[test]
fn one_recording_limit_for_the_hold_and_the_phone() {
    assert_eq!(RECORD_MAX_MS, 120_000);
    assert_eq!(RECORD_MAX_MS, nixa_phone::wire::TALK_MAX_MS as u64);
    assert_eq!(nixa_phone::wire::TALK_MAX_BYTES, nixa_phone::wire::TALK_RATE * 2 * 121);
}

#[test]
fn the_hold_has_time_limits() {
    let mut h = Hold::default();
    h.start("p1", 1000);
    assert!(!h.due(1000 + RECORD_MAX_MS - 1));
    assert!(h.due(1000 + RECORD_MAX_MS));
    h.expire();
    assert!(!h.on());
    assert!(!h.end("p1", 5000));
    h.start("p1", 5000);
    h.end("p1", 6000);
    assert!(!h.due(6000 + ANSWER_WAIT_MS - 1));
    assert!(h.due(6000 + ANSWER_WAIT_MS));
}

#[test]
fn repeats_are_merged_and_order_is_kept() {
    let mut h = Hold::default();
    h.start("p1", 0);
    let options = SayOptions::nixa("chat");
    let _a = h.keep("Agent 1 needs you.", &options);
    let _b = h.keep("Agent 2 is done.", &options);
    let _c = h.keep("agent 1 needs you", &options);
    assert_eq!(h.texts(), ["Agent 1 needs you.", "Agent 2 is done."]);
    assert!(!h.begin_drain());
    assert!(h.take_next().is_none());
    h.cancel("p1");
    assert!(h.begin_drain());
    assert!(!h.begin_drain());
    assert_eq!(h.take_next().map(|l| l.text), Some("Agent 1 needs you.".into()));
    h.start("p1", 10);
    assert!(h.take_next().is_none());
    assert_eq!(h.held(), 1);
    assert_eq!(key("  Done, now!  "), "done now");
}

#[tokio::test]
async fn nothing_is_spoken_while_recording_and_the_answer_goes_first() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    notice(&app, "Agent 1 needs you.");
    notice(&app, "Agent 2 is done.");
    notice(&app, "Agent 1 needs you.");
    speech::speak_later_as(&app, "An older answer.", SayOptions::nixa("chat"), Pass::Last);
    settle().await;
    assert!(played(&phone).is_empty());
    assert_eq!(held(&app), ["Agent 1 needs you.", "Agent 2 is done.", "An older answer."]);
    hold::end(&app, "p1");
    notice(&app, "The schedule ran.");
    settle().await;
    assert!(played(&phone).is_empty());
    speech::speak_later_as(&app, "Here is what you asked for.", SayOptions::nixa("chat"), Pass::Last);
    let out = wait_for(&phone, 5).await;
    assert_eq!(out, ["Here is what you asked for.", "Agent 1 needs you.", "Agent 2 is done.", "An older answer.", "The schedule ran."]);
    assert!(!hold::holding(&app));
    notice(&app, "After the hold.");
    assert_eq!(wait_for(&phone, 6).await.last().unwrap(), "After the hold.");
}

#[tokio::test]
async fn the_reply_event_goes_first() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    tokio::spawn(crate::voice::speak_replies(app.clone()));
    tokio::task::yield_now().await;
    hold::start(&app, "p1");
    notice(&app, "Agent 3 needs you.");
    hold::end(&app, "p1");
    app.send_out(json!({"type": "commentary", "text": "Looking now."}));
    settle().await;
    assert_eq!(played(&phone), ["Looking now."]);
    assert!(hold::holding(&app));
    app.send_out(json!({"type": "reply", "reply": {"speak": true, "text": "The tests pass.", "heard": "me"}}));
    assert_eq!(wait_for(&phone, 3).await, ["Looking now.", "The tests pass.", "Agent 3 needs you."]);
}

#[tokio::test]
async fn a_silent_reply_ends_the_hold() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    tokio::spawn(crate::voice::speak_replies(app.clone()));
    tokio::task::yield_now().await;
    hold::start(&app, "p1");
    notice(&app, "Agent 4 is done.");
    hold::end(&app, "p1");
    app.send_out(json!({"type": "reply", "reply": {"speak": false, "text": "", "heard": "me"}}));
    assert_eq!(wait_for(&phone, 1).await, ["Agent 4 is done."]);
}

#[tokio::test]
async fn cancel_and_a_dropped_link_end_the_hold_at_once() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    notice(&app, "Agent 5 needs you.");
    settle().await;
    assert!(played(&phone).is_empty());
    hold::cancel(&app, "p1");
    assert_eq!(wait_for(&phone, 1).await, ["Agent 5 needs you."]);
}

#[tokio::test]
async fn a_long_recording_ends_the_hold() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    lock(&app.talk.hold).start("p1", now_ms() - RECORD_MAX_MS - 1);
    notice(&app, "Agent 6 is done.");
    settle().await;
    assert!(played(&phone).is_empty());
    flow::tick(&app, now_ms());
    assert_eq!(wait_for(&phone, 1).await, ["Agent 6 is done."]);
    assert!(!hold::holding(&app));
}

#[tokio::test]
async fn a_late_answer_does_not_hold_lines_for_ever() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    notice(&app, "Agent 7 needs you.");
    hold::end(&app, "p1");
    settle().await;
    flow::tick(&app, now_ms() + ANSWER_WAIT_MS - 1000);
    settle().await;
    assert!(played(&phone).is_empty());
    flow::tick(&app, now_ms() + ANSWER_WAIT_MS + 1);
    assert_eq!(wait_for(&phone, 1).await, ["Agent 7 needs you."]);
}

#[tokio::test]
async fn heard_text_with_no_answer_ends_the_hold() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    notice(&app, "Agent 8 is done.");
    hold::end(&app, "p1");
    flow::hear(&app, Said::heard("thanks", now_ms(), now_ms())).await;
    assert_eq!(wait_for(&phone, 1).await, ["Agent 8 is done."]);
}

#[tokio::test]
async fn chat_lines_are_never_held() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    let heard = tokio::time::timeout(Duration::from_secs(2), flow::hear(&app, Said::typed("quiet for 10 minutes", now_ms()))).await;
    assert!(heard.is_ok());
    let nixa: Vec<String> = lock(&app.chat).list().iter().filter(|i| i.who == Who::Nixa).map(|i| i.text.clone()).collect();
    assert!(nixa[0].starts_with("Quiet until "), "{nixa:?}");
    settle().await;
    crate::voice::say(&app, "The team run started.", None);
    settle().await;
    assert!(played(&phone).is_empty());
    assert_eq!(held(&app).len(), 2);
    hold::cancel(&app, "p1");
    let out = wait_for(&phone, 2).await;
    assert!(out[0].starts_with("Quiet until "), "{out:?}");
    assert_eq!(out[1], "The team run started.");
}

#[tokio::test]
async fn a_held_line_answers_its_caller_when_spoken() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    let worker = app.clone();
    let said = tokio::spawn(async move { speech::say(&worker, "Should I push?", SayOptions::nixa("attention")).await });
    settle().await;
    assert!(!said.is_finished());
    hold::cancel(&app, "p1");
    let result = tokio::time::timeout(Duration::from_secs(5), said).await.unwrap().unwrap();
    assert!(!result.interrupted);
    assert_eq!(played(&phone), ["Should I push?"]);
}

#[tokio::test]
async fn a_line_already_waiting_is_held_when_recording_starts() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    let turn = app.talk.chain.lock().await;
    notice(&app, "Agent 9 needs you.");
    settle().await;
    assert!(held(&app).is_empty());
    hold::start(&app, "p1");
    drop(turn);
    settle().await;
    assert!(played(&phone).is_empty());
    assert_eq!(held(&app), ["Agent 9 needs you."]);
    hold::cancel(&app, "p1");
    assert_eq!(wait_for(&phone, 1).await, ["Agent 9 needs you."]);
}

#[tokio::test]
async fn a_new_recording_stops_the_drain_and_keeps_the_rest() {
    let (_dir, app) = app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PhoneSay>();
    speech::set_phone(&app, Some(tx));
    crate::app::lock(&app.voice.settings).voice_on = true;
    hold::start(&app, "p1");
    notice(&app, "First line.");
    notice(&app, "Second line.");
    settle().await;
    hold::cancel(&app, "p1");
    let first = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert_eq!(first.text, "First line.");
    hold::start(&app, "p1");
    let _ = first.done.send(SayResult::default());
    settle().await;
    assert_eq!(held(&app), ["Second line."]);
    assert!(rx.try_recv().is_err());
    hold::cancel(&app, "p1");
    let second = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert_eq!(second.text, "Second line.");
    let _ = second.done.send(SayResult::default());
}

#[tokio::test]
async fn a_keyword_answer_goes_first() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    notice(&app, "Agent 1 needs attention.");
    hold::end(&app, "p1");
    crate::modes::effects::answer_now(&app, "Plan mode is on.");
    assert_eq!(wait_for(&phone, 2).await, ["Plan mode is on.", "Agent 1 needs attention."]);
}

#[tokio::test]
async fn a_stop_ends_the_answer_wait() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    notice(&app, "Agent 3 is done.");
    hold::end(&app, "p1");
    crate::voice::handle(&app, crate::voice::Command::Stop).await;
    assert_eq!(wait_for(&phone, 1).await, ["Agent 3 is done."]);
}

#[tokio::test]
async fn a_window_interrupt_ends_the_answer_wait() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    notice(&app, "Agent 4 needs attention.");
    hold::end(&app, "p1");
    crate::server::interrupt_turn(&app);
    assert_eq!(wait_for(&phone, 1).await, ["Agent 4 needs attention."]);
}

#[tokio::test]
async fn turning_nixa_off_drops_the_held_lines() {
    let (_dir, app) = app();
    let phone = fake_phone(&app);
    hold::start(&app, "p1");
    notice(&app, "Agent 5 is done.");
    settle().await;
    assert_eq!(held(&app), ["Agent 5 is done."]);
    hold::end(&app, "p1");
    crate::settings_api::set_nixa(&app, false).unwrap();
    assert!(held(&app).is_empty());
    assert!(!hold::holding(&app));
    settle().await;
    assert!(played(&phone).is_empty());
}

#[test]
fn a_line_given_back_after_the_hold_ended_keeps_the_drain_going() {
    let mut hold = Hold::default();
    hold.start("p1", 0);
    drop(hold.keep("Line 1", &SayOptions::default()));
    drop(hold.keep("Line 2", &SayOptions::default()));
    hold.cancel("p1");
    assert!(hold.begin_drain());
    let first = hold.take_next().unwrap();
    hold.start("p1", 1);
    hold.cancel("p1");
    assert!(!hold.begin_drain());
    assert!(hold.give_back(first));
    assert_eq!(hold.take_next().unwrap().text, "Line 1");
    assert_eq!(hold.take_next().unwrap().text, "Line 2");
    assert!(hold.take_next().is_none());
}

#[test]
fn a_line_given_back_while_recording_waits() {
    let mut hold = Hold::default();
    drop(hold.keep("Line 1", &SayOptions::default()));
    assert!(hold.begin_drain());
    let first = hold.take_next().unwrap();
    hold.start("p1", 0);
    assert!(!hold.give_back(first));
    assert_eq!(hold.texts(), ["Line 1"]);
    hold.cancel("p1");
    assert!(hold.begin_drain());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lines_sent_close_together_keep_their_order() {
    let (_dir, app) = app();
    hold::start(&app, "p1");
    let lines: Vec<String> = (1..=30).map(|n| format!("Agent {n} is done.")).collect();
    for line in &lines {
        notice(&app, line);
    }
    settle().await;
    assert_eq!(held(&app), lines);
}

#[test]
fn the_reply_to_a_deep_brain_answer_uses_the_deep_voice_once() {
    let (_dir, app) = app();
    let now = crate::util::now_ms();
    assert_eq!(crate::voice::speech::reply_kind(&app, true, now), "chat");
    crate::voice::speech::mark_deep_reply(&app);
    assert_eq!(crate::voice::speech::reply_kind(&app, false, now), "deep");
    assert_eq!(crate::voice::speech::reply_kind(&app, true, now), "deep");
    assert_eq!(crate::voice::speech::reply_kind(&app, true, now), "chat");
    crate::voice::speech::mark_deep_reply(&app);
    assert_eq!(crate::voice::speech::reply_kind(&app, true, now + crate::voice::speech::DEEP_REPLY_MS + 1), "chat");
}
