use super::banter::{self, Banter};
use super::fun::*;
use super::*;
use std::collections::HashMap;

fn pair() -> Pair {
    Pair {
        from: "Nova".into(),
        to: "Kai".into(),
        first: "Kai, the 3 tests in src/login.rs pass now.".into(),
        second: "Thanks, that helps a lot.".into(),
        known: Vec::new(),
    }
}

fn names() -> Vec<String> {
    vec!["Nova".into(), "Kai".into()]
}

#[test]
fn meaning_guard_keeps_numbers_names_and_files() {
    let original = "Kai, the 3 tests in src/login.rs pass now.";
    assert!(guard(original, "Good news Kai: all 3 tests in src/login.rs are green now!", &names()).is_ok());
    assert!(guard(original, "Good news Kai: all tests in src/login.rs are green.", &names()).is_err());
    assert!(guard(original, "Good news Kai: 3 tests in the login file pass.", &names()).is_err());
    assert!(guard(original, "Good news: 3 tests in src/login.rs pass.", &names()).is_err());
    assert!(guard(original, "Kai, 3 of 4 tests in src/login.rs pass.", &names()).is_err());
    assert!(guard("Please work on task #12.", "Fancy a go at task #12?", &[]).is_ok());
    assert!(guard("Please work on \"Fix login page\".", "Could you take the login fix?", &[]).is_err());
    let kept = judge(original, Some("Kai, the tests pass."), &names(), &[]);
    assert!(!kept.changed);
    assert_eq!(kept.text, original);
    assert!(kept.reason.unwrap().contains("lost"));
    let long = judge("Hi Kai.", Some(&format!("Hi Kai, {}", "very ".repeat(40))), &names(), &[]);
    assert_eq!(long.text, "Hi Kai.");
    assert!(long.reason.unwrap().contains("too long"));
    let grown = judge("On it!", Some("On it, right away, with a big mug of tea and a lot of cheer!"), &[], &[]);
    assert!(grown.reason.unwrap().contains("too long"));
    assert!(judge("On it!", Some("On it, boss. Tea first?"), &[], &[]).changed);
}

#[test]
fn meaning_guard_keeps_negation_titles_and_whole_names() {
    assert!(guard("I cannot finish the report.", "I can finish the report.", &[]).is_err());
    assert!(guard("I cannot finish the report.", "Sadly the report won't get finished by me.", &[]).is_ok());
    assert!(guard("The build failed.", "The build did not make it.", &[]).is_ok());
    let known = vec!["Nova".to_string(), "Kai".to_string(), "fix login page".to_string()];
    assert!(guard("working on fix login page now", "busy with the login stuff", &known).is_err());
    assert!(guard("working on fix login page now", "Deep in Fix Login Page, wish me luck", &known).is_ok());
    assert!(guard("Kai finished it.", "Nova finished it, hooray.", &known).is_err());
    assert!(guard("Kai finished it.", "Kai wrapped it up, hooray.", &known).is_ok());
    assert!(guard("Ask Nova about it.", "Ask Novak about it.", &known).is_err());
    assert!(guard("Ram has the tests.", "The program has the tests.", &["Ram".into()]).is_err());
    let p = pair_of(&json!({"first": "Hi", "known": ["Nova", "", "fix login page"]})).unwrap();
    assert_eq!(p.known, vec!["Nova".to_string(), "fix login page".to_string()]);
}

#[test]
fn serious_lines_are_found() {
    for line in [
        "The build failed again.",
        "I am stuck. I need your help.",
        "Can the user approve this?",
        "Error in main.rs",
        "The tests did not pass.",
        "The job timed out.",
        "A file is missing.",
        "I am waiting for you.",
        "Page not found.",
        "The agent was killed.",
    ] {
        assert!(serious(line), "{line}");
    }
    for line in ["My work is done. Please check it.", "Here is my advice. Go ahead.", "On it!"] {
        assert!(!serious(line), "{line}");
    }
    let calm = Pair { first: "On it!".into(), second: String::new(), ..pair() };
    assert!(user_message(&calm, (&[], &[]), true).contains("no jokes"));
    assert!(!user_message(&calm, (&[], &[]), false).contains("no jokes"));
    let out = settle(&calm, Ok(json!({"first": "Right away!", "second": "", "serious": true})), (&[], &[]), false);
    assert!(out.serious);
    assert_eq!(out.first.text, "On it!");
    assert!(out.first.reason.unwrap().contains("serious"));
    let plain = settle(&calm, Ok(json!({"first": "Right away!", "second": "", "serious": true})), (&[], &[]), true);
    assert_eq!(plain.first.text, "Right away!");
    let sent = body("gemma", &calm, (&[], &[]), true);
    assert_eq!(sent["think"], false);
    assert_eq!(sent["options"]["temperature"], 0.5);
    assert_eq!(sent["format"]["required"], json!(["first", "second", "serious"]));
}

#[test]
fn repeats_are_rejected_and_remembered() {
    let mut memory: HashMap<String, Vec<(String, String)>> = HashMap::new();
    remember(&mut memory, "Deep", "Here is my advice. Go ahead.", "Advice served hot. Go for it.");
    remember(&mut memory, "Deep", "Something else entirely here.", "Other.");
    let before = earlier(&memory, "deep", "Here is my advice, go ahead!");
    assert_eq!(before, vec!["Advice served hot. Go for it.".to_string()]);
    assert!(earlier(&memory, "Nova", "Here is my advice. Go ahead.").is_empty());
    let again = judge("Here is my advice. Go ahead.", Some("Advice served hot. Go for it!"), &[], &before);
    assert!(!again.changed);
    assert!(again.reason.unwrap().contains("repeated"));
    let fresh = judge("Here is my advice. Go ahead.", Some("My two cents are ready. Go ahead."), &[], &before);
    assert!(fresh.changed);
    let p = Pair { from: "Deep".into(), to: "Nixa".into(), first: "Here is my advice. Go ahead.".into(), second: "Thanks.".into(), known: Vec::new() };
    assert!(user_message(&p, (&before, &[]), false).contains("Advice served hot"));
    for i in 0..30 {
        remember(&mut memory, "Deep", "x", &format!("y{i}"));
    }
    assert_eq!(memory["deep"].len(), KEEP);
    let mut fun = Fun::default();
    let out = Outcome { first: fresh, second: Said { text: "Thanks.".into(), changed: false, reason: Some("the rewrite lost \"Nixa\"".into()) }, serious: false };
    record(&mut fun, &p, &out);
    assert_eq!((fun.rewrites, fun.fallbacks), (1, 1));
    assert_eq!(earlier(&fun.memory, "Deep", &p.first), vec!["My two cents are ready. Go ahead.".to_string()]);
}

#[test]
fn bad_replies_fall_back() {
    let p = pair();
    let down = settle(&p, Err("Ollama took too long".into()), (&[], &[]), false);
    assert_eq!(down.first.text, p.first);
    assert_eq!(down.second.text, p.second);
    assert_eq!(down.first.reason.as_deref(), Some("Ollama took too long"));
    assert!(content_of(&json!({"message": {"content": "not json"}})).is_err());
    assert!(content_of(&json!({})).is_err());
    let empty = settle(&p, Ok(json!({"first": "", "second": "Cheers, Nova!"})), (&[], &[]), false);
    assert!(!empty.first.changed);
    assert!(empty.second.changed);
    let e = event(7, &empty);
    assert_eq!(e["type"], FUN_EVENT);
    assert_eq!(e["id"], 7);
    assert_eq!(e["second"], "Cheers, Nova!");
    assert_eq!(e["changed"], true);
}

#[tokio::test]
async fn slow_or_missing_ollama_times_out() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hold = tokio::spawn(async move {
        let kept = listener.accept().await;
        tokio::time::sleep(Duration::from_secs(5)).await;
        drop(kept);
    });
    let http = reqwest::Client::new();
    let started = std::time::Instant::now();
    let got = ask(&http, &url, &json!({}), Duration::from_millis(300)).await;
    assert_eq!(got.unwrap_err(), "Ollama took too long");
    assert!(started.elapsed() < Duration::from_secs(2));
    hold.abort();
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gone = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let down = ask(&http, &gone, &json!({}), Duration::from_millis(300)).await;
    assert!(down.as_ref().unwrap_err().contains("not reachable"));
    let mut fun = Fun::default();
    note_reply(&mut fun, &down, 1000);
    assert!(resting(&fun, 1000 + DOWN_MS - 1));
    assert!(!resting(&fun, 1000 + DOWN_MS));
    note_reply(&mut fun, &Ok(json!({})), 2000);
    assert!(!resting(&fun, 2000));
    assert!(should_log(&mut fun, "Ollama is not reachable", 0));
    assert!(!should_log(&mut fun, "Ollama is not reachable", LOG_EVERY_MS - 1));
    assert!(should_log(&mut fun, "Ollama took too long", LOG_EVERY_MS - 1));
    assert!(should_log(&mut fun, "Ollama took too long", 2 * LOG_EVERY_MS));
}

#[tokio::test]
async fn gate_is_off_unless_switch_and_office() {
    assert!(gate_open(true, true));
    assert!(!gate_open(false, true));
    assert!(!gate_open(true, false));
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let mut rx = app.out.subscribe();
    let ask = json!({"id": 3, "from_name": "Nova", "to_name": "Kai", "first": "Done with #4.", "second": "Nice.", "on": true, "office": false});
    assert!(super::ws(&app, FUN_KIND, &ask));
    let got = rx.recv().await.unwrap();
    assert_eq!(got["id"], 3);
    assert_eq!(got["first"], "Done with #4.");
    assert_eq!(got["changed"], false);
    assert!(super::ws(&app, GATE_KIND, &json!({"on": true, "office": true})));
    assert_eq!(fun::status(&lock(&app.soap.fun), now_ms())["on"], true);
    assert!(super::ws(&app, GATE_KIND, &json!({"on": false, "office": true})));
    assert_eq!(fun::status(&lock(&app.soap.fun), now_ms())["on"], false);
    assert!(super::ws(&app, GATE_KIND, &json!({"on": true, "office": true})));
    assert_eq!(fun::status(&lock(&app.soap.fun), now_ms() + STALE_MS)["on"], false);
    assert!(super::ws(&app, banter::BANTER_KIND, &json!({"who": "m1", "to": "m2", "owner": "Ravi", "task": "Fix login", "on": false, "office": true})));
}

fn chat() -> Banter {
    Banter { who: "m1".into(), name: "Maya".into(), to: "m2".into(), to_name: "Omar".into(), owner: "Ravi".into(), task: "Fix login page".into() }
}

#[test]
fn banter_lines_use_real_task_and_skip_recent() {
    let b = chat();
    let (first, second) = banter::choose(&b, &[], 0);
    assert_eq!(first, "Ravi is wrestling \"Fix login page\" again.");
    assert!(!second.is_empty());
    let (next, _) = banter::choose(&b, std::slice::from_ref(&first), 0);
    assert_ne!(next, first);
    assert!(next.contains("Fix login page") && next.contains("Ravi"));
    for seed in 0..20 {
        let (one, two) = banter::choose(&b, &[], seed);
        assert!(one.contains("Ravi") && !one.contains('{') && !two.contains('{'));
    }
    let good = json!({"first": "Ravi is in a staring contest with the login page.", "second": "Ha, tea time."});
    assert!(banter::check(&good, &b, &[]).is_some());
    assert!(banter::check(&json!({"first": "Someone is busy.", "second": "Yes."}), &b, &[]).is_none());
    assert!(banter::check(&json!({"first": "Ravi fixed 9 bugs.", "second": "Wow."}), &b, &[]).is_none());
    let used = vec!["Ravi is in a staring contest with the login page.".to_string()];
    assert!(banter::check(&good, &b, &used).is_none());
    assert!(banter::check(&json!({"first": "Ravi is taming \"Fix login page\" again.", "second": "Go Ravi."}), &b, &[]).is_some());
    assert!(banter::check(&json!({"first": "Ravi is taming \"Fix signup form\" again.", "second": "Go."}), &b, &[]).is_none());
    assert!(banter::check(&json!({"first": "Ravi is lost in the Billing Report.", "second": "Oh no."}), &b, &[]).is_none());
    let seven = Banter { task: "Fix bug 7".into(), ..chat() };
    assert!(banter::check(&json!({"first": "Ravi and Fix bug 7 again.", "second": "Brave."}), &seven, &[]).is_some());
    assert!(banter::check(&json!({"first": "Ravi and Fix bug 7, round 3.", "second": "Brave."}), &seven, &[]).is_none());
    assert!(banter::banter_of(&json!({"who": "m1", "to": "m1", "owner": "Ravi", "task": "x"})).is_none());
    assert_eq!(banter::banter_of(&json!({"who": "m1", "name": "Maya", "to": "m2", "to_name": "Omar", "owner": "Ravi", "task": "Fix login page"})), Some(b));
}

#[test]
fn scene_lines_skip_recent_repeats() {
    let mut made = json!({"lines": [{"who": "m1", "text": "Who took my mug?"}, {"who": "m2", "text": "Not me!"}, {"who": "m1", "text": "Tea time."}]});
    scene::drop_repeats(&mut made, &["who took my mug".into()]);
    assert_eq!(made["lines"].as_array().unwrap().len(), 2);
    scene::drop_repeats(&mut made, &["Not me".into(), "tea time".into()]);
    assert_eq!(made["lines"].as_array().unwrap().len(), 2);
    let mut story = memory::Story::default();
    memory::add_lines(&mut story, (0..40).map(|i| format!("line {i}")).collect());
    assert_eq!(story.lines.len(), memory::KEEP_LINES);
    assert!(scene::user_message(&sample_ask(), &[], &["Who took my mug?".into()]).contains("never reuse"));
}
