use super::confirm::*;
use super::flow;
use super::gather::*;
use super::heard::*;
use super::level::*;
use super::presence::*;
use super::progress::*;
use super::words::*;
use crate::app::{App, lock};
use crate::chat::Who;
use crate::reply::AgentReply;
use crate::util::now_ms;
use serde_json::{Map, Value, json};
use std::sync::Arc;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn lines(app: &App, who: Who) -> Vec<String> {
    lock(&app.chat).list().iter().filter(|i| i.who == who).map(|i| i.text.clone()).collect()
}

fn quiet_lines(app: &App) -> Vec<String> {
    lock(&app.chat).list().iter().filter(|i| i.who == Who::User && i.quiet).map(|i| i.text.clone()).collect()
}

fn carry(app: &App) -> String {
    lock(&app.brain).carry.clone()
}

fn talk() -> Talk {
    Talk::new("Nixa")
}

#[test]
fn small_talk_kinds() {
    let t = talk();
    assert_eq!(t.small_talk("uh"), Some(SmallTalk::Noise));
    assert_eq!(t.small_talk(""), Some(SmallTalk::Noise));
    assert_eq!(t.small_talk("k"), Some(SmallTalk::Noise));
    assert_eq!(t.small_talk("thank you so much"), Some(SmallTalk::Thanks));
    assert_eq!(t.small_talk("well thanks"), Some(SmallTalk::Thanks));
    assert_eq!(t.small_talk("ok great"), Some(SmallTalk::Assent));
    assert_eq!(t.small_talk("mm hmm"), Some(SmallTalk::Assent));
    assert_eq!(t.small_talk("Nixa thanks"), None);
    assert_eq!(t.small_talk("thanks nixa"), None);
    assert_eq!(t.small_talk("run the tests"), None);
}

#[test]
fn confirm_replies() {
    let t = talk();
    assert_eq!(t.confirm_reply("yes"), Some(Reply::Yes));
    assert_eq!(t.confirm_reply("Nixa, yes do it please"), Some(Reply::Yes));
    assert_eq!(t.confirm_reply("uh go ahead"), Some(Reply::Yes));
    assert_eq!(t.confirm_reply("okay send it"), Some(Reply::Yes));
    assert_eq!(t.confirm_reply("no"), Some(Reply::No));
    assert_eq!(t.confirm_reply("no no"), Some(Reply::No));
    assert_eq!(t.confirm_reply("don't do it"), Some(Reply::No));
    assert_eq!(t.confirm_reply("ok"), Some(Reply::Unclear));
    assert_eq!(t.confirm_reply("thanks"), Some(Reply::Unclear));
    assert_eq!(t.confirm_reply("what is the weather"), None);
    assert_eq!(t.confirm_reply("hey my low yes"), Some(Reply::Yes));
    assert_eq!(t.confirm_reply("yes nixa"), Some(Reply::Yes));
}

#[test]
fn typed_replies() {
    let t = talk();
    assert_eq!(t.typed_reply("yes please"), Some(Reply::Yes));
    assert_eq!(t.typed_reply("ok go ahead"), Some(Reply::Yes));
    assert_eq!(t.typed_reply("no thanks"), Some(Reply::No));
    assert_eq!(t.typed_reply("um"), Some(Reply::Unclear));
    assert_eq!(t.typed_reply("fix the bug"), None);
}

#[test]
fn names_and_repeats() {
    let t = talk();
    assert!(t.named("hey nixa what is up"));
    assert!(t.only_name("hey nixa"));
    assert!(!t.only_name("nixa run it"));
    assert!(t.wants_repeat("sorry, can you say that again?"));
    assert!(t.wants_repeat("Nixa, repeat please"));
    assert!(t.wants_repeat("I can't hear you"));
    assert!(t.wants_repeat("what did you say"));
    assert!(!t.wants_repeat("repeat the last test run on m3"));
    assert_eq!(t.without_name("hey nixa run the tests"), vec!["run", "the", "tests"]);
    assert_eq!(t.without_name("run the tests mylo"), vec!["run", "the", "tests"]);
    assert_eq!(t.without_name("my low run it"), vec!["run", "it"]);
}

#[test]
fn an_own_name_counts_too() {
    let t = Talk::new("Nova");
    assert!(t.named("Nova, start the build"));
    assert_eq!(t.small_talk("thanks nova"), None);
    assert_eq!(t.confirm_reply("nova yes"), Some(Reply::Yes));
}

#[test]
fn prompt_answer_problems() {
    let t = talk();
    assert_eq!(t.prompt_answer_problem("permission", Some("yes"), "yes", true), None);
    assert!(t.prompt_answer_problem("permission", Some("always"), "yes", false).is_some());
    assert_eq!(t.prompt_answer_problem("permission", Some("always"), "yes always", false), None);
    assert!(t.prompt_answer_problem("permission", Some("yes"), "ok", false).is_some());
    assert_eq!(t.prompt_answer_problem("permission", Some("yes"), "yes", false), None);
    assert!(t.prompt_answer_problem("plan", Some("yes"), "the plan looks long", false).is_some());
    assert_eq!(t.prompt_answer_problem("plan", Some("yes"), "approve the plan", false), None);
    assert_eq!(t.prompt_answer_problem("question", Some("yes"), "the second one", false), None);
    assert_eq!(t.prompt_answer_problem("permission", Some("no"), "uh", false), None);
}

#[test]
fn card_replies() {
    let t = talk();
    assert!(t.is_card_reply("2"));
    assert!(t.is_card_reply("option two."));
    assert!(t.is_card_reply("yes, always"));
    assert!(t.is_card_reply("b"));
    assert!(t.is_card_reply("nope"));
    assert!(!t.is_card_reply("the build is red"));
    assert!(one_letter("a."));
    assert!(!one_letter("ab"));
}

fn heard_at(started_at: u64, at: u64) -> Said {
    Said::heard("yes", started_at, at)
}

#[test]
fn confirm_steps() {
    let asked = 1000;
    assert_eq!(confirm_step(Some(Reply::No), &heard_at(0, 0), asked, NOT_READY), Step::No);
    assert_eq!(confirm_step(Some(Reply::Unclear), &heard_at(0, 0), asked, NOT_READY), Step::Wait);
    assert_eq!(confirm_step(Some(Reply::Yes), &Said::typed("yes", 1100), asked, NOT_READY), Step::Yes);
    assert_eq!(confirm_step(Some(Reply::Yes), &heard_at(1500, 1600), asked, NOT_READY), Step::Wait);
    assert_eq!(confirm_step(Some(Reply::Yes), &heard_at(3100, 3500), asked, NOT_READY), Step::Yes);
    assert_eq!(confirm_step(Some(Reply::Yes), &heard_at(5000, 5200), asked, 4000), Step::Yes);
    assert_eq!(confirm_step(Some(Reply::Yes), &heard_at(40_000, 40_000), asked, 4000), Step::Wait);
    assert_eq!(confirm_step(None, &heard_at(5000, 5000), asked, 4000), Step::Cancel);
    assert_eq!(confirm_step(None, &heard_at(1200, 1300), asked, NOT_READY), Step::Pass);
    assert_eq!(confirm_step(None, &Said::typed("other", 1300), asked, NOT_READY), Step::Cancel);
}

#[test]
fn spoken_confirms_stay_short() {
    assert_eq!(spoken_confirm("Push to main?"), "Push to main? Say yes or no.");
    assert_eq!(spoken_confirm("Push to main"), "Push to main? Say yes or no.");
    let long = "x".repeat(300);
    let send = format!("Send \"{long}\" to Claude (m4)?");
    assert_eq!(spoken_confirm(&send), "Send this message to Claude? Say yes or no.");
    let start = format!("Start Codex with the first message {long}");
    assert_eq!(spoken_confirm(&start), "Start Codex with a first message? Say yes or no.");
    let words = "word ".repeat(60);
    let out = spoken_confirm(&words);
    assert!(out.ends_with("… Say yes or no."), "{out}");
    assert!(out.chars().count() < 180, "{out}");
    let solid = spoken_confirm(&"y".repeat(200));
    assert_eq!(solid, format!("{}… Say yes or no.", "y".repeat(40)));
}

#[test]
fn confirms_open_one_at_a_time() {
    let mut box_ = Confirms::default();
    let (id, mut rx) = box_.ask("Push?", "risky", 10).unwrap();
    assert!(box_.ask("Other?", "", 11).is_none());
    assert_eq!(box_.expired(10 + CONFIRM_MS - 1), None);
    assert_eq!(box_.expired(10 + CONFIRM_MS), Some(id));
    assert!(box_.finish(true, Some(id + 1), End::Yes).is_none());
    let (ask, end) = box_.finish(false, Some(id), End::Timeout).unwrap();
    assert_eq!(end, End::Timeout);
    assert_eq!(end_line(&ask, end), "No answer: Push");
    assert_eq!(rx.try_recv(), Ok(false));
    assert!(timeout_note(&ask).contains("2 minutes"));
    let (id2, _rx) = box_.ask("Again?", "", 20).unwrap();
    assert_eq!(id2, id + 1);
    box_.ready(id2, 30);
    assert_eq!(box_.open.as_ref().unwrap().ready_at, 30);
    box_.reask(40);
    assert_eq!(box_.open.as_ref().unwrap().ready_at, NOT_READY);
}

#[test]
fn quiet_words() {
    assert_eq!(parse_quiet("quiet for 20 minutes"), Some(20 * 60_000));
    assert_eq!(parse_quiet("Nixa, be quiet for half an hour please"), Some(30 * 60_000));
    assert_eq!(parse_quiet("stay quiet for two hours"), Some(2 * 3_600_000));
    assert_eq!(parse_quiet("quiet mode for 1.5h"), Some(5_400_000));
    assert_eq!(parse_quiet("ok quiet time"), Some(DEFAULT_QUIET_MS));
    assert_eq!(parse_quiet("Talk again."), Some(0));
    assert_eq!(parse_quiet("quiet for a while"), None);
    assert_eq!(parse_quiet("make the tests quiet"), None);
    assert_eq!(parse_quiet("quiet for zero minutes"), None);
}

#[test]
fn presence_quiet_and_held() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    let mut p = Presence::new(Some(file.clone()), 1000);
    assert!(!p.present(0));
    p.touch(5000);
    assert!(p.present(5500));
    p.touch(7000);
    assert_eq!(Presence::new(Some(file), 1000).last_seen(), 5000);
    let until = p.set_quiet(7000, 500);
    assert_eq!(until, 7500);
    assert!(p.quiet(7499) && !p.quiet(7500));
    assert_eq!(p.quiet_until(7600), 0);
    p.hold(7000, "  one   line ", false);
    p.hold(7001, "one line", false);
    p.hold(7002, "two", true);
    assert_eq!(p.held(), 2);
    for i in 0..40 {
        p.hold(8000 + i, &format!("line {i}"), false);
    }
    assert_eq!(p.held(), HELD_MAX);
    let snap = p.snapshot(7100);
    assert_eq!(snap["held"], json!(HELD_MAX));
    assert_eq!(snap["quietUntil"], json!(7500));
    assert_eq!(snap["lastSeen"], json!(7000));
    assert_eq!(p.take().len(), HELD_MAX);
    assert_eq!(p.held(), 0);
}

#[test]
fn quiet_texts() {
    assert_eq!(quiet_end_text(&[]), "Quiet time is over. Nothing happened.");
    let one = vec![Held { at: 1, text: "Build is done.".into(), mac_only: false }];
    assert_eq!(quiet_end_text(&one), "Quiet time is over. 1 thing happened: \"Build is done.\".");
    let two = vec![Held { at: 1, text: "a".into(), mac_only: false }, Held { at: 2, text: "b".into(), mac_only: false }];
    assert_eq!(quiet_end_text(&two), "Quiet time is over. 2 things happened. The first: \"a\". The rest is in the chat.");
    assert!(quiet_start_text(0).starts_with("Quiet until "));
}

#[test]
fn gather_joins_phrases_after_a_pause() {
    let mut g = Gather::new(2300, 180_000);
    let piece = |text: &str, at: u64| Piece { text: text.into(), started_at: at, captured_at: at + 500 };
    g.speech_start();
    g.speech_end(1000);
    assert!(g.phrase(1500, piece("run the tests", 0)).is_none());
    assert!(g.pending());
    assert!(g.due(3000).is_none());
    g.speech_start();
    assert!(g.due(4000).is_none());
    g.speech_end(4500);
    assert!(g.phrase(4600, piece(" and then push ", 3000)).is_none());
    assert!(g.due(6800).is_none());
    let message = g.due(6900).unwrap();
    assert_eq!(message.text(), "run the tests and then push");
    assert_eq!(message.parts.len(), 2);
    assert_eq!((message.started_at, message.captured_at), (0, 3500));
    assert!(!g.pending());
    assert!(g.phrase(10, piece("   ", 0)).is_none());
    assert!(g.flush().is_none());
}

#[test]
fn gather_caps_long_talk() {
    let mut g = Gather::new(2300, 10_000);
    g.speech_start();
    assert!(g.phrase(100, Piece { text: "a".into(), started_at: 0, captured_at: 100 }).is_none());
    assert!(g.due(5000).is_none());
    assert_eq!(g.due(10_100).unwrap().text(), "a");
    g.speech_start();
    assert!(g.phrase(100, Piece { text: "b".into(), started_at: 0, captured_at: 100 }).is_none());
    assert_eq!(g.phrase(10_000, Piece { text: "c".into(), started_at: 9000, captured_at: 10_000 }).unwrap().text(), "b c");
    g.phrase(20_000, Piece { text: "d".into(), started_at: 19_000, captured_at: 20_000 });
    g.drop_all();
    assert!(!g.pending() && !g.talking());
    assert_eq!(pause_wait(3000, 700), 2300);
    assert_eq!(pause_wait(500, 700), 0);
}

#[test]
fn level_meter_smooths_and_throttles() {
    assert_eq!(loudness(&[]), 0.0);
    assert_eq!(loudness(&[0; 512]), 0.0);
    assert_eq!(loudness(&[i16::MAX; 512]), 1.0);
    let mid = loudness(&[1000; 512]);
    assert!(mid > 0.6 && mid < 0.7, "{mid}");
    let mut m = Meter::new(66);
    let first = m.push(0, &[i16::MAX; 512], true).unwrap();
    assert_eq!(first, (0.6, true));
    assert!(m.push(10, &[i16::MAX; 512], true).is_none());
    let next = m.push(70, &[i16::MAX; 512], true).unwrap();
    assert_eq!(next, (0.94, true));
    let down = m.push(140, &[0; 512], true).unwrap();
    assert_eq!(down, (0.8, true));
    assert_eq!(m.reset(141), (0.0, false));
    assert!(m.push_rms(300, 0.0, false).is_none());
    assert_eq!(flow::level_event(0.6, true), json!({"type": "level", "mic": 0.6, "talking": true}));
}

#[test]
fn progress_waits_and_drops_repeats() {
    let mut p = Progress::default();
    assert_eq!(p.offer(0, false, false, "Reading files.").0, Offer::Inactive);
    assert_eq!(p.offer(0, true, false, "  ").0, Offer::Empty);
    assert_eq!(p.offer(0, true, false, "Reading the files."), (Offer::Published, Some("Reading the files.".into())));
    assert_eq!(p.offer(1000, true, false, "reading the FILES").0, Offer::Duplicate);
    assert_eq!(p.offer(1000, true, false, "Found the bug in main.").0, Offer::Queued);
    assert_eq!(p.offer(1000, true, false, "found the bug in main").0, Offer::Duplicate);
    assert_eq!(p.flush(15_000, true, false), None);
    assert_eq!(p.flush(PROGRESS_MS, true, true), None);
    assert_eq!(p.flush(PROGRESS_MS, true, false), Some("Found the bug in main.".into()));
    assert_eq!(p.last(), Some("Found the bug in main."));
    assert_eq!(p.offer(50_000, true, false, "Next step now."), (Offer::Published, Some("Next step now.".into())));
    assert_eq!(p.offer(51_000, true, false, "Queued line.").0, Offer::Queued);
    assert_eq!(p.flush(52_000, false, false), None);
    assert!(!p.pending());
    assert_eq!(p.offer(53_000, true, false, "Reading the files.").0, Offer::Published);
    let long: String = "a".repeat(400);
    let (_, text) = p.offer(200_000, true, false, &long);
    assert_eq!(text.unwrap().chars().count(), PROGRESS_CHARS);
    assert_eq!(first_sentence("Found it. Now fixing."), "Found it.");
    assert_eq!(first_sentence("Version 1.2 is out"), "Version 1.2 is out");
    assert_eq!(key("Hello,  World!"), "hello world");
}

#[test]
fn heard_helpers() {
    let mut lines = Vec::new();
    for (i, text) in ["one", "two", "three"].iter().enumerate() {
        keep_heard(&mut lines, HeardLine { text: text.to_string(), at: i as u64 * 1000 });
    }
    assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["two", "three"]);
    let note = earlier_heard_note(&lines, 4000);
    assert!(note.starts_with("Note: The user's last heard lines before the next Heard line, oldest first: 3 s before: \"two\"; 2 s before: \"three\"."), "{note}");
    assert!(note.contains("go on from them"));
    let one = earlier_heard_note(&lines[1..], 2100);
    assert!(one.contains("last heard line") && one.contains("1 s before") && one.contains("go on from it"), "{one}");
    assert_eq!(earlier_heard_note(&lines, 10 * 60_000), "");
    assert!(skip_heard(Some(SmallTalk::Noise), false, true, 1, 0));
    assert!(skip_heard(Some(SmallTalk::Thanks), false, false, 0, 0));
    assert!(!skip_heard(Some(SmallTalk::Thanks), true, false, 0, 0));
    assert!(skip_heard(Some(SmallTalk::Assent), false, false, 0, 40_000));
    assert!(!skip_heard(Some(SmallTalk::Assent), false, false, 0, 10_000));
    assert!(!skip_heard(Some(SmallTalk::Assent), false, true, 0, 40_000));
    assert!(!skip_heard(None, false, false, 0, 40_000));
    assert!(wake_allows(false, false, false, false, true));
    assert!(!wake_allows(false, false, false, false, false));
    let quiet = AgentReply { calls: Vec::new(), speak: false, text: String::new(), heard: "me".into(), failed: false };
    assert!(silent_to_user(true, &quiet, 5000));
    assert!(!silent_to_user(true, &quiet, 1000));
    assert!(!silent_to_user(false, &quiet, 5000));
    let wait = AgentReply { heard: "unfinished".into(), ..quiet.clone() };
    assert!(!silent_to_user(true, &wait, 5000));
    let spoken = AgentReply { calls: Vec::new(), speak: true, text: "Done.".into(), ..quiet };
    assert!(!silent_to_user(true, &spoken, 5000));
}

#[tokio::test]
async fn a_stop_word_stops_and_closes_the_card() {
    let (_dir, app) = app();
    let (id, rx) = lock(&app.talk.confirms).ask("Push?", "", now_ms()).unwrap();
    flow::hear(&app, Said::heard("stop", now_ms(), now_ms())).await;
    assert_eq!(rx.await, Ok(false));
    assert!(lock(&app.talk.confirms).open.is_none());
    assert_eq!(lines(&app, Who::User), ["stop"]);
    assert!(lines(&app, Who::Info).contains(&"No: Push".to_string()));
    assert!(lock(&app.talk.heard).stopped_at > 0);
    assert!(id > 0);
}

#[tokio::test]
async fn a_repeat_with_nothing_said() {
    let (_dir, app) = app();
    flow::hear(&app, Said::heard("say that again", now_ms(), now_ms())).await;
    assert_eq!(lines(&app, Who::Nixa), [NOTHING_SAID]);
}

#[tokio::test]
async fn a_repeat_says_the_last_line_again() {
    let (_dir, app) = app();
    super::speech::say(&app, "The build is green.", super::speech::SayOptions::nixa("chat")).await;
    flow::hear(&app, Said::heard("what did you say", now_ms(), now_ms())).await;
    assert_eq!(lines(&app, Who::Info), ["Said again: The build is green."]);
}

#[tokio::test]
async fn spoken_and_typed_confirm_answers() {
    let (_dir, app) = app();
    let at = now_ms();
    let (id, rx) = lock(&app.talk.confirms).ask("Push to main?", "", at).unwrap();
    lock(&app.talk.confirms).ready(id, at);
    flow::hear(&app, Said::heard("yes do it", at + 10, at + 20)).await;
    assert_eq!(rx.await, Ok(true));
    assert!(lines(&app, Who::Info).contains(&"Yes: Push to main".to_string()));

    let (id, rx) = lock(&app.talk.confirms).ask("Delete the branch?", "", at).unwrap();
    lock(&app.talk.confirms).ready(id, at);
    flow::hear(&app, Said::typed("no", at + 10)).await;
    assert_eq!(rx.await, Ok(false));
    assert!(lines(&app, Who::Info).contains(&"No: Delete the branch".to_string()));
}

#[tokio::test]
async fn an_unclear_answer_waits_and_says_how_once() {
    let (_dir, app) = app();
    let at = now_ms();
    let (id, _rx) = lock(&app.talk.confirms).ask("Push?", "", at).unwrap();
    lock(&app.talk.confirms).ready(id, at);
    flow::hear(&app, Said::heard("ok", at + 10, at + 20)).await;
    flow::hear(&app, Said::heard("hmm ok", at + 30, at + 40)).await;
    assert_eq!(lines(&app, Who::Info).iter().filter(|l| l.as_str() == WAIT_LINE).count(), 1);
    assert_eq!(quiet_lines(&app), ["ok", "hmm ok"]);
    assert!(lock(&app.talk.confirms).open.is_some());
}

#[tokio::test]
async fn a_late_yes_asks_again() {
    let (_dir, app) = app();
    let at = now_ms();
    let (id, _rx) = lock(&app.talk.confirms).ask("Push?", "", at - 60_000).unwrap();
    lock(&app.talk.confirms).ready(id, at - 50_000);
    flow::hear(&app, Said::heard("yes", at - 100, at)).await;
    let open = lock(&app.talk.confirms).open.clone().unwrap();
    assert!(open.asked_at >= at);
    assert_eq!(open.ready_at, NOT_READY);
}

#[tokio::test]
async fn a_new_topic_cancels_an_open_card() {
    let (_dir, app) = app();
    let at = now_ms();
    let (id, rx) = lock(&app.talk.confirms).ask("Push?", "", at).unwrap();
    lock(&app.talk.confirms).ready(id, at);
    flow::finish_confirm(&app, false, Some(id + 5), End::No);
    assert!(lock(&app.talk.confirms).open.is_some());
    lock(&app.talk.confirms).open.as_mut().unwrap().deadline = 0;
    flow::tick(&app, now_ms());
    assert_eq!(rx.await, Ok(false));
    assert!(lines(&app, Who::Info).contains(&"No answer: Push".to_string()));
    assert!(carry(&app).contains("Nobody answered this card"));
}

#[tokio::test]
async fn ask_confirm_waits_for_the_window() {
    let (_dir, app) = app();
    let worker = app.clone();
    let asked = tokio::spawn(async move { flow::ask_confirm(&worker, "Run the deploy?", "risky", None).await });
    let id = loop {
        if let Some(open) = lock(&app.talk.confirms).open.clone() {
            break open.id;
        }
        tokio::task::yield_now().await;
    };
    assert!(super::ws(&app, "confirm", &json!({"id": id, "approve": true})));
    assert!(asked.await.unwrap());
    let token = tokio_util::sync::CancellationToken::new();
    let worker = app.clone();
    let cancel = token.clone();
    let asked = tokio::spawn(async move { flow::ask_confirm(&worker, "Again?", "", Some(cancel)).await });
    while lock(&app.talk.confirms).open.is_none() {
        tokio::task::yield_now().await;
    }
    assert!(!flow::ask_confirm(&app, "Second?", "", None).await);
    token.cancel();
    assert!(!asked.await.unwrap());
    assert!(lines(&app, Who::Info).contains(&"Cancelled: Again".to_string()));
}

#[tokio::test]
async fn quiet_time_holds_lines_and_reports_after() {
    let (_dir, app) = app();
    flow::hear(&app, Said::typed("quiet for 10 minutes", now_ms())).await;
    let nixa = lines(&app, Who::Nixa);
    assert!(nixa[0].starts_with("Quiet until "), "{nixa:?}");
    assert!(lock(&app.talk.presence).quiet(now_ms()));
    assert!(carry(&app).contains("asked for quiet until"));
    assert!(super::speech::speak(&app, "Build finished.", super::speech::SayOptions::nixa("update")).await.is_none());
    assert_eq!(lock(&app.talk.presence).held(), 1);
    flow::hear(&app, Said::typed("talk again", now_ms())).await;
    let nixa = lines(&app, Who::Nixa);
    assert_eq!(nixa.last().unwrap(), "Quiet time is over. 1 thing happened: \"Build finished.\".");
    flow::hear(&app, Said::typed("talk again", now_ms())).await;
    assert_eq!(lines(&app, Who::Nixa).last().unwrap(), QUIET_NOT_ON);
}

#[tokio::test]
async fn quiet_from_the_panel_ends_with_a_summary() {
    let (_dir, app) = app();
    assert!(super::ws(&app, "set_quiet", &json!({"minutes": 5})));
    flow::tick(&app, now_ms());
    lock(&app.talk.presence).hold(now_ms(), "Tests passed.", false);
    assert!(super::ws(&app, "set_quiet", &json!({"minutes": 0})));
    assert!(lines(&app, Who::Info).contains(&QUIET_PANEL_END.to_string()));
    flow::tick(&app, now_ms());
    assert_eq!(lines(&app, Who::Nixa).last().unwrap(), "Quiet time is over. 1 thing happened: \"Tests passed.\".");
}

#[tokio::test]
async fn quiet_words_from_the_room_need_the_name_in_hotkey_mode() {
    let (_dir, app) = app();
    lock(&app.voice.settings).listen = crate::voice::Listen::Hotkey;
    flow::hear(&app, Said::heard("be quiet for 5 minutes", now_ms(), now_ms())).await;
    assert!(!lock(&app.talk.presence).quiet(now_ms()));
    flow::hear(&app, Said::heard("Nixa quiet for 5 minutes", now_ms(), now_ms())).await;
    assert!(lock(&app.talk.presence).quiet(now_ms()));
}

#[tokio::test]
async fn small_talk_is_kept_out_of_the_brain() {
    let (_dir, app) = app();
    flow::hear(&app, Said::heard("thanks", now_ms(), now_ms())).await;
    flow::hear(&app, Said::heard("okay", now_ms(), now_ms())).await;
    assert_eq!(quiet_lines(&app), ["thanks", "okay"]);
    lock(&app.talk.speaker).last_ask_at = now_ms();
    flow::hear(&app, Said::heard("yes", now_ms(), now_ms())).await;
    assert_eq!(lock(&app.talk.speaker).last_ask_at, 0);
    assert!(lines(&app, Who::User).contains(&"yes".to_string()));
    assert!(!quiet_lines(&app).contains(&"yes".to_string()));
}

#[tokio::test]
async fn heard_lines_carry_the_earlier_ones() {
    let (_dir, app) = app();
    let at = now_ms();
    flow::hear(&app, Said::heard("open the api folder", at - 5000, at - 4000)).await;
    lock(&app.brain).carry.clear();
    flow::hear(&app, Said::heard("and run the tests there", at - 1000, at)).await;
    let note = carry(&app);
    assert!(note.contains("4 s before: \"open the api folder\""), "{note}");
    assert_eq!(lock(&app.talk.heard).before.len(), 2);
}

#[tokio::test]
async fn not_called_lines_pass_only_for_an_open_card() {
    let (_dir, app) = app();
    assert!(!flow::not_called(&app, "yes"));
    lock(&app.talk.confirms).ask("Push?", "", now_ms()).unwrap();
    assert!(flow::not_called(&app, "yes"));
}

#[tokio::test]
async fn heard_phrases_are_gathered() {
    let (_dir, app) = app();
    flow::on_speech_start(&app);
    flow::on_speech_end(&app);
    flow::on_heard(&app, "fix the", 100, 900);
    assert!(lock(&app.talk.gather).pending());
    flow::on_heard(&app, "stop", 100, 300);
    assert!(!lock(&app.talk.gather).pending());
}

#[tokio::test]
async fn the_progress_tool_answers_with_a_status() {
    let (_dir, app) = app();
    let mut args = Map::new();
    args.insert("text".into(), json!("Reading the logs. Then more."));
    let out = super::call(&app, NAME, &args).unwrap().unwrap();
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), json!({"status": "inactive"}));
    lock(&app.brain).busy = true;
    let out = super::call(&app, NAME, &args).unwrap().unwrap();
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), json!({"status": "published"}));
    assert_eq!(lines(&app, Who::Nixa), ["Reading the logs."]);
    args.insert("text".into(), json!("x".repeat(PROGRESS_CHARS + 1)));
    assert!(super::call(&app, NAME, &args).unwrap().is_err());
    assert!(super::call(&app, "nixa_other", &args).is_none());
    assert_eq!(super::specs()[0].name, NAME);
    flow::turn_ended(&app);
    assert!(lock(&app.talk.progress).last().is_some());
}

#[tokio::test]
async fn a_silent_reply_to_the_user_gets_a_line() {
    let (_dir, app) = app();
    let reply = AgentReply { calls: Vec::new(), speak: false, text: String::new(), heard: "me".into(), failed: false };
    assert!(flow::no_reply(&app, true, &reply));
    assert_eq!(lines(&app, Who::Nixa), [NO_REPLY_TEXT]);
    lock(&app.talk.heard).stopped_at = now_ms();
    assert!(!flow::no_reply(&app, true, &reply));
}

#[tokio::test]
async fn the_talk_view_shows_state() {
    let (_dir, app) = app();
    lock(&app.talk.confirms).ask("Push?", "risky", 5).unwrap();
    let view = super::view(&app);
    assert_eq!(view["type"], json!("talk"));
    assert_eq!(view["confirm"]["message"], json!("Push?"));
    assert_eq!(view["confirm"]["why"], json!("risky"));
    assert!(view["confirm"].get("readyAt").is_none());
    assert_eq!(view["presence"]["held"], json!(0));
    assert!(super::ws(&app, "talk_state", &json!({})));
    assert!(!super::ws(&app, "other", &json!({})));
}

#[tokio::test]
async fn talk_routes_need_the_window_token() {
    let (_dir, app) = app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = super::routes().with_state(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    let http = reqwest::Client::new();
    let header = crate::window_key::HEADER;
    assert_eq!(http.get(format!("{base}/talk")).send().await.unwrap().status(), 401);
    let seen = http.get(format!("{base}/talk")).header(header, &app.window_token).send().await.unwrap();
    assert_eq!(seen.status(), 200);
    let quiet = http.post(format!("{base}/talk/quiet")).header(header, &app.window_token).json(&json!({"minutes": 2})).send().await.unwrap();
    let body: Value = quiet.json().await.unwrap();
    assert!(body["talk"]["presence"]["quietUntil"].as_u64().is_some());
    let heard = http.post(format!("{base}/talk/hear")).header(header, &app.window_token).json(&json!({"text": "thanks"})).send().await.unwrap();
    assert_eq!(heard.status(), 200);
    assert_eq!(quiet_lines(&app), ["thanks"]);
    let empty = http.post(format!("{base}/talk/hear")).header(header, &app.window_token).json(&json!({"text": " "})).send().await.unwrap();
    assert_eq!(empty.status(), 400);
}

#[tokio::test]
async fn automatic_talk_ask_says_yes_without_a_card() {
    let (_dir, app) = app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = super::routes().with_state(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    let http = reqwest::Client::new();
    let asked = http.post(format!("{base}/talk/ask")).header(crate::window_key::HEADER, &app.window_token).json(&json!({"message": "Push?"})).send().await.unwrap();
    assert_eq!(asked.status(), 200);
    let body: Value = asked.json().await.unwrap();
    assert_eq!(body["approve"], json!(true));
    assert!(body["talk"]["confirm"].is_null());
    let conn = crate::modes::store(&app).unwrap();
    let rows = crate::modes::journal::since(&conn, 0).unwrap();
    let row = rows.iter().find(|r| r.kind == "confirm").unwrap();
    assert_eq!(row.choice, "Push?");
    assert_eq!(row.outcome.as_deref(), Some("yes"));
}

#[tokio::test]
async fn spoken_confirms_go_to_the_journal() {
    let (_dir, app) = app();
    let at = now_ms();
    let (id, rx) = lock(&app.talk.confirms).ask("Push the branch?", "the checks passed", at).unwrap();
    lock(&app.talk.confirms).ready(id, at);
    assert!(flow::finish_confirm(&app, true, Some(id), End::Yes));
    assert_eq!(rx.await, Ok(true));
    let conn = crate::modes::store(&app).unwrap();
    let rows = crate::modes::journal::since(&conn, 0).unwrap();
    let row = rows.iter().find(|r| r.kind == "confirm").unwrap();
    assert_eq!(row.choice, "Push the branch?");
    assert_eq!(row.reason.as_deref(), Some("the checks passed"));
    assert_eq!(row.outcome.as_deref(), Some("yes"));
}

#[tokio::test]
async fn unnamed_speech_in_always_mode_is_room_talk() {
    let (_dir, app) = app();
    lock(&app.voice.settings).listen = crate::voice::Listen::Always;
    lock(&app.brain).busy = true;
    let at = now_ms();
    flow::hear(&app, Said::heard("push the release branch", at, at)).await;
    let queue = lock(&app.brain).queue.clone();
    assert!(queue.iter().any(|q| q.contains("Heard (no name): \"push the release branch\"")), "{queue:?}");
    assert_eq!(lock(&app.brain).last_user, None);
    assert!(lines(&app, Who::User).contains(&"push the release branch".to_string()));
    lock(&app.talk.speaker).last_spoke_at = now_ms();
    flow::hear(&app, Said::heard("and the docs too", now_ms(), now_ms())).await;
    let queue = lock(&app.brain).queue.clone();
    assert!(queue.iter().any(|q| q.contains("Heard: \"and the docs too\"")), "{queue:?}");
}

#[tokio::test]
async fn unnamed_speech_in_hotkey_mode_stays_a_heard_line() {
    let (_dir, app) = app();
    lock(&app.brain).busy = true;
    let at = now_ms();
    flow::hear(&app, Said::heard("push the release branch", at, at)).await;
    let queue = lock(&app.brain).queue.clone();
    assert!(queue.iter().any(|q| q.contains("Heard: \"push the release branch\"")), "{queue:?}");
}

#[test]
fn quiet_summary_never_quotes_a_mac_only_line() {
    let camera = Held { at: 1, text: "The child is dancing.".into(), mac_only: true };
    let build = Held { at: 2, text: "Build is done.".into(), mac_only: false };
    let mixed = quiet_end_text(&[camera.clone(), build]);
    assert!(mixed.contains("Build is done.") && !mixed.contains("dancing"), "{mixed}");
    let only = quiet_end_text(std::slice::from_ref(&camera));
    assert_eq!(only, "Quiet time is over. 1 thing happened. It is in the chat on the Mac.");
    let mut p = Presence::new(None, 1000);
    p.hold(5, "secret", true);
    assert!(p.take()[0].mac_only);
}

#[tokio::test]
async fn a_held_mac_only_line_keeps_its_flag() {
    let (_dir, app) = app();
    flow::hear(&app, Said::typed("quiet for 10 minutes", now_ms())).await;
    let options = super::speech::SayOptions { mac_only: true, ..super::speech::SayOptions::nixa("chat") };
    assert!(super::speech::speak(&app, "The child is dancing.", options).await.is_none());
    flow::hear(&app, Said::typed("talk again", now_ms())).await;
    let nixa = lines(&app, Who::Nixa);
    assert!(!nixa.last().unwrap().contains("dancing"), "{nixa:?}");
}

#[tokio::test]
async fn say_that_again_keeps_the_mac_only_flag() {
    let (_dir, app) = app();
    let options = super::speech::SayOptions { mac_only: true, ..super::speech::SayOptions::nixa("chat") };
    super::speech::say_clean(&app, "The child is dancing.", options, super::hold::Pass::Notice, None).await;
    assert_eq!(super::speech::last_said_mac(&app), Some(("The child is dancing.".to_string(), true)));
    flow::hear(&app, Said::heard("say that again", now_ms(), now_ms())).await;
    let again = lock(&app.chat).list().iter().rev().find(|i| i.who == Who::Info && i.text.starts_with("Said again")).cloned().unwrap();
    assert!(again.mac_only);
}

#[tokio::test]
async fn the_user_speaking_stops_the_line_and_drops_the_rest() {
    use super::speech::{SayOptions, barge_in, last_said, say, set_phone};
    let (_dir, app) = app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    set_phone(&app, Some(tx));
    crate::app::lock(&app.voice.settings).voice_on = true;
    let speak = |text: &'static str| {
        let app = app.clone();
        tokio::spawn(async move { say(&app, text, SayOptions::nixa("chat")).await })
    };
    let first = speak("The first reply is playing now.");
    let job = rx.recv().await.unwrap();
    let second = speak("The second reply waits its turn.");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    barge_in(&app);
    assert!(job.stop.is_cancelled(), "the line on the phone must stop at once");
    let _ = job.done.send(nixa_voice::SayResult { interrupted: true, ms: 0 });
    assert!(first.await.unwrap().interrupted);
    assert!(second.await.unwrap().interrupted, "the waiting line must not play after the user spoke");
    assert!(rx.try_recv().is_err(), "the dropped line never reaches the phone");
    assert!(last_said(&app).is_some(), "the stopped reply is kept as the last thing said");
    assert!(!lock(&app.voice.live).nixa_talking);
}
