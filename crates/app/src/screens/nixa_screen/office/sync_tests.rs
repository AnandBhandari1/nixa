use super::call::{self, Call};
use super::leave::{self, Leave};
use super::plan;
use super::soap;
use super::sync::{self, Heard, Sync};
use super::talk::{self, Phase};
use crate::screens::nixa_screen::map::{Agent, NIXA, St};
use serde_json::json;

fn heard(who: &str, text: &str, state: &str) -> Heard {
    Heard { who: who.into(), text: text.into(), state: state.into(), chat: false }
}

fn playing() -> talk::Playing {
    talk::Playing {
        talk: talk::talk_of(NIXA, "a1", None, Some("Please check the build."), &[], 0),
        start: 1000,
        walk: 1000,
        say1: talk::say1_ms("Please check the build."),
        say2: talk::say2_ms("Great work, thank you!"),
        said: 0,
        from_at: (0.0, 0.0),
        to_at: (100.0, 0.0),
        lead: 0,
        open: false,
    }
}

#[test]
fn skip_timer_is_slower_and_bounded() {
    assert_eq!(sync::timer_ms(""), 1800);
    assert_eq!(sync::timer_ms(&"a".repeat(50)), 3000);
    assert_eq!(sync::timer_ms(&"a".repeat(200)), 6000);
    let mut s = Sync::default();
    sync::post(&mut s, "m1", "Hello there.", 500, false);
    let line = s.line.as_ref().unwrap();
    assert_eq!(sync::shown_at(line), 500);
    assert_eq!(sync::done_at(line), 500 + sync::timer_ms("Hello there."));
}

#[test]
fn heard_reads_the_voice_event() {
    let h = sync::heard(&json!({"type": "office_voice", "who": "m1", "text": "Hi.", "state": "start", "src": "office"})).unwrap();
    assert_eq!(h, heard("m1", "Hi.", "start"));
    assert!(sync::heard(&json!({"who": "nixa", "text": "Hi.", "state": "start", "src": "chat"})).unwrap().chat);
    assert!(sync::heard(&json!({"who": "m1", "text": "Hi.", "state": "maybe"})).is_none());
    assert!(sync::heard(&json!({"who": "", "text": "Hi.", "state": "end"})).is_none());
}

#[test]
fn line_waits_for_start_then_holds_until_end() {
    let mut s = Sync::default();
    sync::post(&mut s, "m1", "Hi there.", 1000, true);
    let line = s.line.clone().unwrap();
    assert_eq!(sync::shown_at(&line), 1000 + sync::WAIT_MS);
    assert!(!sync::on_event(&mut s, &heard("m2", "Hi there.", "start"), 1500));
    assert!(!sync::on_event(&mut s, &heard("m1", "Other.", "start"), 1500));
    assert!(sync::on_event(&mut s, &heard("m1", "Hi  there.", "start"), 1800));
    let line = s.line.clone().unwrap();
    assert_eq!(sync::shown_at(&line), 1800);
    assert!(sync::open(&line, 9000));
    assert_eq!(sync::done_at(&line), 1000 + sync::LINE_MS);
    assert!(sync::on_event(&mut s, &heard("m1", "Hi there.", "end"), 7000));
    let line = s.line.clone().unwrap();
    assert!(!sync::open(&line, 7100));
    assert_eq!(sync::done_at(&line), 7000 + sync::AFTER_MS);
    assert!(!sync::on_event(&mut s, &heard("m1", "Hi there.", "end"), 7100));
}

#[test]
fn line_never_holds_past_twelve_seconds() {
    let mut s = Sync::default();
    sync::post(&mut s, "m1", "Hi.", 0, true);
    sync::on_event(&mut s, &heard("m1", "Hi.", "start"), 200);
    let line = s.line.clone().unwrap();
    assert!(sync::open(&line, sync::LINE_MS - 1));
    assert!(!sync::open(&line, sync::LINE_MS));
    assert_eq!(sync::done_at(&line), sync::LINE_MS);
    sync::post(&mut s, "m1", "Hi.", 0, true);
    let quiet = s.line.clone().unwrap();
    assert_eq!(sync::done_at(&quiet), sync::WAIT_MS + sync::timer_ms("Hi."));
    sync::on_event(&mut s, &heard("m1", "Hi.", "skip"), 400);
    assert_eq!(sync::done_at(s.line.as_ref().unwrap()), 400 + sync::timer_ms("Hi."));
}

#[test]
fn walk_and_talk_holds_the_bubble_until_voice_ends() {
    let mut p = playing();
    let mut s = Sync::default();
    let a = p.start + p.walk;
    let line = talk::due_line(&mut p, a + 10).expect("first line");
    let talk::Line::First(who, text) = line else { panic!("first") };
    sync::post(&mut s, &who, &text, a + 10, true);
    talk::hold(&mut p, s.line.as_ref().unwrap(), a + 20);
    assert!(p.lead > 2000);
    sync::on_event(&mut s, &heard(&who, &text, "start"), a + 900);
    talk::hold(&mut p, s.line.as_ref().unwrap(), a + 950);
    assert_eq!(p.lead, 900);
    assert!(p.open);
    let late = a + 10_000;
    talk::hold(&mut p, s.line.as_ref().unwrap(), late);
    assert!(matches!(talk::phase(&p, late), Phase::Say1(_)));
    sync::on_event(&mut s, &heard(&who, &text, "end"), late);
    talk::hold(&mut p, s.line.as_ref().unwrap(), late + 1);
    assert!(!p.open);
    assert!(matches!(talk::phase(&p, late + sync::AFTER_MS + 5), Phase::Say2(_)));
    let second = talk::due_line(&mut p, late + sync::AFTER_MS + 5);
    assert!(matches!(second, Some(talk::Line::Second(_, _))));
    assert_eq!(p.lead, 0);
}

#[test]
fn walk_and_talk_caps_a_stuck_voice_at_twelve_seconds() {
    let mut p = playing();
    let mut s = Sync::default();
    let a = p.start + p.walk;
    let Some(talk::Line::First(who, text)) = talk::due_line(&mut p, a) else { panic!("first") };
    sync::post(&mut s, &who, &text, a, true);
    sync::on_event(&mut s, &heard(&who, &text, "start"), a + 100);
    talk::hold(&mut p, s.line.as_ref().unwrap(), a + 200);
    assert_eq!(p.say1, sync::LINE_MS);
    assert!(matches!(talk::phase(&p, a + sync::LINE_MS + 1), Phase::Say2(_)));
}

fn show_of(lines: &[(&str, &str)]) -> soap::Show {
    let scene = soap::Scene {
        place: soap::Place::from_word("tea"),
        cast: vec!["m1".into(), "m2".into()],
        mood: "fun".into(),
        lines: lines
            .iter()
            .map(|(w, t)| soap::SoapLine { who: w.to_string(), text: t.to_string(), cue: soap::Cue::from_word("") })
            .collect(),
    };
    let mut at = 1000;
    let times = scene
        .lines
        .iter()
        .map(|l| {
            let len = soap::line_ms(&l.text);
            let out = (at, len);
            at += len + soap::GAP_MS;
            out
        })
        .collect();
    soap::Show { scene, place: soap::Place::from_word("tea"), start: 0, lines_at: 1000, times, cast: Vec::new(), hangs: Vec::new(), said: 0, lead: 0, open: false }
}

#[test]
fn soap_line_holds_and_pushes_later_lines() {
    let mut show = show_of(&[("m1", "Tea again?"), ("m2", "Always.")]);
    let second = show.times[1].0;
    let (who, text) = soap::due(&mut show, 1010).expect("line");
    let mut s = Sync::default();
    sync::post(&mut s, &who, &text, 1010, true);
    sync::on_event(&mut s, &heard(&who, &text, "start"), 1500);
    soap::hold(&mut show, s.line.as_ref().unwrap(), 1600);
    assert_eq!(show.lead, 500);
    assert!(show.open);
    assert_eq!(show.times[0].1, 1010 + sync::LINE_MS - 1000);
    assert!(show.times[1].0 > second);
    sync::on_event(&mut s, &heard(&who, &text, "end"), 4000);
    soap::hold(&mut show, s.line.as_ref().unwrap(), 4001);
    assert_eq!(show.times[0].1, 3000 + sync::AFTER_MS);
    assert_eq!(show.times[1].0, 1000 + 3000 + sync::AFTER_MS + soap::GAP_MS);
    assert_eq!(soap::line_at(&show, show.times[1].0 + 1).map(|l| l.0), Some(1));
}

#[test]
fn sentences_split_on_ends() {
    assert_eq!(call::sentences("Hi there. How are you?  Fine!\nNew line"), vec!["Hi there.", "How are you?", "Fine!", "New line"]);
    assert_eq!(call::sentences("Version 1.2 is out."), vec!["Version 1.2 is out."]);
    assert!(call::sentences("   ").is_empty());
}

#[test]
fn mic_talk_shows_user_then_nixa_sentences_in_voice_order() {
    let mut c = Call::default();
    assert!(!call::on(&c, 0));
    assert!(!call::reply(&mut c, "Hi.", 0));
    assert!(call::user(&mut c, "  what is   up ", 1000));
    assert_eq!(c.user.as_ref().unwrap().0, "what is up");
    assert!(call::on(&c, 2000));
    assert!(call::thinking(&c, 2000));
    assert!(call::dolly(&c, 1000 + call::ZOOM_MS) > 0.99);
    assert!(call::reply(&mut c, "All good. Two tasks are done.", 3000));
    assert!(call::line(&c, 3500).is_none());
    assert!(call::voice(&mut c, "All good. Two tasks are done.", "start", 4000));
    let first = call::line(&c, 4100).unwrap();
    assert_eq!(first.text, "All good.");
    assert!(first.talking);
    let second = call::line(&c, 4000 + call::SENTENCE_MIN_MS + 100).unwrap();
    assert_eq!(second.text, "Two tasks are done.");
    let held = call::line(&c, 20_000).unwrap();
    assert_eq!(held.text, "Two tasks are done.");
    assert!(call::voice(&mut c, "All good. Two tasks are done.", "end", 21_000));
    assert!(!call::line(&c, 21_100).unwrap().talking);
    assert!(call::line(&c, 21_000 + call::LINGER_MS).is_none());
    let quiet = call::quiet_at(&c);
    assert_eq!(quiet, 21_000 + call::LINGER_MS + call::QUIET_MS);
    assert!(call::on(&c, quiet - 1));
    assert!(!call::on(&c, quiet));
    assert_eq!(call::dolly(&c, quiet + call::ZOOM_MS), 0.0);
}

#[test]
fn mic_talk_without_voice_uses_the_reply_timer() {
    let mut c = Call::default();
    call::user(&mut c, "hello", 0);
    call::reply(&mut c, "Hello! I am here.", 100);
    assert!(call::thinking(&c, 200));
    let begin = 100 + sync::WAIT_MS;
    assert_eq!(call::line(&c, begin + 10).unwrap().text, "Hello!");
    assert_eq!(call::line(&c, begin + sync::timer_ms("Hello!") + 10).unwrap().text, "I am here.");
    let end = begin + sync::timer_ms("Hello!") + sync::timer_ms("I am here.");
    assert!(call::line(&c, end).is_none());
    assert_eq!(call::quiet_at(&c), end + call::QUIET_MS);
}

#[test]
fn mic_talk_never_goes_through_the_fun_layer() {
    let mut stage = talk::Stage::default();
    call::user(&mut stage.call, "how is the build", 0);
    call::reply(&mut stage.call, "The build is green.", 10);
    assert!(stage.queue.is_empty() && stage.now.is_none());
    let plan = plan::build(&[], &[], "Nixa", false);
    let gate = soap::Gate { office: true, open: true, on: true, up: true, idle: false, talk: true };
    assert!(super::fun::asks(&mut stage, &plan, &[] as &[Agent], gate, 20).is_empty());
    let fun_msg = json!({"id": 1, "first": "A funny line."});
    assert!(!super::fun::apply(&mut stage, &fun_msg));
    call::voice(&mut stage.call, "The build is green.", "start", 30);
    assert_eq!(call::line(&stage.call, 40).unwrap().text, "The build is green.");
}

fn office_with(id: &str) -> plan::Plan {
    let room = super::Room {
        id: "s".into(),
        name: "S".into(),
        color: 0,
        agents: vec![Agent {
            id: id.into(),
            title: "Worker".into(),
            program: "claude".into(),
            mono: "C".into(),
            status: St::Idle,
            parent: None,
            seq: 1,
            number: Some(1),
            persona: None,
        }],
        deep: Vec::new(),
    };
    plan::build(&[&room], &[], "Nixa", false)
}

#[test]
fn agent_leave_walks_out_and_back() {
    let plan = office_with("m1");
    let p = plan::find(&plan, "m1").unwrap().clone();
    let mut l = Leave::default();
    assert!(!leave::take(&mut l, &json!({"id": "m1", "line": "Back soon."}), true, 0));
    let msg = json!({"id": "m1", "name": "Ada", "until": 0, "line": "", "to": "m2", "to_name": "Bo"});
    assert!(leave::take(&mut l, &msg, false, 1000));
    assert!(!leave::take(&mut l, &msg, false, 1100));
    let trip = leave::trip_of(&l, "m1").unwrap().clone();
    let (text, _) = leave::said(&trip, "m1", &plan, &p, 1000 + talk::SEAT_MS + 10).unwrap();
    assert_eq!(text, "Ada is on leave.");
    let speak_end = 1000 + talk::SEAT_MS + leave::SAY_MS;
    assert_eq!(leave::ask_line(&trip, speak_end + 10).unwrap().0, "Asking Bo.");
    let mid = leave::spot(&trip, &plan, &p, speak_end + 200).unwrap();
    assert!(matches!(mid.pose, super::sprite::Pose::Walk { .. }));
    let far = 1000 + talk::SEAT_MS + leave::SAY_MS + talk::WALK_MAX + 10;
    assert_eq!(leave::spot(&trip, &plan, &p, far).unwrap().at, leave::GONE);
    assert!(leave::gone(&trip, &plan, &p, far));
    leave::prune(&mut l, &plan, far);
    assert_eq!(l.trips.len(), 1);
    assert!(leave::take(&mut l, &json!({"id": "m1", "name": "Ada", "line": "I am back!"}), true, 50_000));
    let back = leave::trip_of(&l, "m1").unwrap().clone();
    assert_eq!(leave::said(&back, "m1", &plan, &p, 50_010).unwrap().0, "I am back!");
    let walk_in = leave::spot(&back, &plan, &p, 50_100).unwrap();
    assert!(matches!(walk_in.pose, super::sprite::Pose::Walk { .. }));
    assert!(leave::spot(&back, &plan, &p, 50_000 + leave::back_ms(&plan, &p) + 1).is_none());
    leave::prune(&mut l, &plan, 50_000 + leave::back_ms(&plan, &p) + leave::SAY_MS + 1);
    assert!(l.trips.is_empty());
}

#[test]
fn a_fast_chunk_still_shows_every_sentence() {
    let mut c = Call::default();
    assert!(call::user(&mut c, "status?", 0));
    let text = "One is done. Two is done too. Three is last.";
    assert!(call::voice(&mut c, text, "start", 1000));
    assert_eq!(call::line(&c, 1100).unwrap().text, "One is done.");
    assert!(call::voice(&mut c, text, "end", 1500));
    let next = call::line(&c, 1510).unwrap();
    assert_eq!(next.text, "Two is done too.");
    assert!(!next.talking);
    assert_eq!(call::line(&c, 1500 + call::CATCH_MS + 10).unwrap().text, "Three is last.");
    assert_eq!(call::line(&c, 1500 + 2 * call::CATCH_MS + call::LINGER_MS - 10).unwrap().text, "Three is last.");
    assert!(call::line(&c, 1500 + 2 * call::CATCH_MS + call::LINGER_MS).is_none());
}

#[test]
fn a_leaving_agent_whose_terminal_closed_still_walks_out() {
    let plan = office_with("m1");
    let mut l = Leave::default();
    leave::remember(&mut l, &plan, 500);
    let empty = office_with("m9");
    assert!(plan::find(&empty, "m1").is_none());
    let msg = json!({"id": "m1", "name": "Ada", "until": 0, "line": "Ada went to the beach.", "to": "m2", "to_name": "Bo"});
    assert!(leave::take(&mut l, &msg, false, 1000));
    let trip = leave::trip_of(&l, "m1").unwrap().clone();
    let ghost = trip.who.clone().expect("snapshot kept");
    assert_eq!(ghost.desk, None);
    assert_eq!(leave::ghosts(&l, &empty, 1000 + talk::SEAT_MS).len(), 1);
    assert_eq!(leave::said(&trip, &ghost.short, &empty, &ghost, 1000 + talk::SEAT_MS + 10).unwrap().0, "Ada went to the beach.");
    assert_eq!(leave::ask_line(&trip, 1000 + talk::SEAT_MS + leave::SAY_MS + 10).unwrap().0, "Asking Bo.");
    leave::prune(&mut l, &empty, 1000 + talk::SEAT_MS + leave::SAY_MS + 10);
    assert_eq!(l.trips.len(), 1, "kept while it walks out");
    leave::prune(&mut l, &empty, 1000 + 120_000);
    assert!(l.trips.is_empty());
}

#[test]
fn leave_from_the_brain_list_seeds_away_desks() {
    let plan = office_with("m1");
    let p = plan::find(&plan, "m1").unwrap().clone();
    let mut l = Leave::default();
    let reply = json!({"ok": true, "items": [{"terminal": "m1", "name": "Ada", "until": 99, "known": true}]});
    let away = leave::away_of(&reply);
    assert_eq!(away, vec![leave::Away { id: "m1".into(), name: "Ada".into(), until: 99 }]);
    assert!(leave::seed(&mut l, &away));
    let trip = leave::trip_of(&l, "m1").unwrap().clone();
    assert!(leave::gone(&trip, &plan, &p, 1_000_000));
    assert!(!leave::seed(&mut l, &away));
    assert!(leave::seed(&mut l, &[]));
    assert!(l.trips.is_empty());
}

#[test]
fn stale_leave_events_do_not_play_a_walk() {
    let plan = office_with("m1");
    let p = plan::find(&plan, "m1").unwrap().clone();
    let mut l = Leave::default();
    assert!(leave::stale(&mut l, &json!({"id": "m1", "line": "bye"}), false));
    assert!(leave::gone(leave::trip_of(&l, "m1").unwrap(), &plan, &p, 1_000_000));
    assert!(leave::stale(&mut l, &json!({"id": "m1", "line": "back"}), true));
    assert!(l.trips.is_empty());
}

#[test]
fn agents_on_leave_are_not_cast() {
    let plan = office_with("m1");
    let mut l = Leave::default();
    assert!(leave::take(&mut l, &json!({"id": "m1"}), false, 0));
    let cast = leave::cast_plan(&plan, &l);
    assert!(!soap::is_free(plan::find(&cast, "m1").unwrap()));
    assert_eq!(cast.people.len(), plan.people.len());
}
