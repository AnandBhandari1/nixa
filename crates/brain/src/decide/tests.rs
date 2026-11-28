use super::*;

#[test]
fn builds_the_request() {
    let r = request("jev-1.14", vec![text("screen")], vec![predicate("waits", "The agent waits for an answer."), choice("pick", "Which option?", &[("1", "Yes"), ("2", "")])]);
    assert_eq!(r["model"], json!({"id": "jev-1.14", "providerID": "opencode-go"}));
    assert_eq!(r["input"][0], json!({"type": "text", "text": "screen"}));
    assert_eq!(r["questions"][0]["type"], "predicate");
    assert_eq!(r["questions"][1]["choices"], json!([{"value": "1", "description": "Yes"}, {"value": "2"}]));
    let s = score("risk", "How risky?", &[("low", ""), ("high", "")]);
    assert_eq!(s["levels"], json!([{"label": "low"}, {"label": "high"}]));
}

#[test]
fn reads_all_answer_types() {
    let body = json!({"data": {"answers": [
        {"type": "predicate", "name": "waits", "probability": 0.95},
        {"type": "choice", "name": "pick", "choice": "1", "confidence": 0.9, "probabilities": [{"value": "1", "probability": 0.9}, {"value": "2", "probability": 0.1}]},
        {"type": "score", "name": "risk", "score": 0.2, "confidence": 0.8, "probabilities": [{"value": 0, "label": "low", "probability": 0.8}]},
        {"type": "error", "name": "safe", "reason": "refusal", "message": "no"}
    ], "cost": 0.00004, "tokens": {"input": 812, "output": 0}}});
    let a = parse(&body).unwrap();
    assert_eq!(a.answers.len(), 4);
    assert_eq!(a.input_tokens, 812);
    assert!((a.cost - 0.00004).abs() < 1e-9);
    assert_eq!(a.get("waits"), Some(&Answer::Predicate { name: "waits".into(), probability: 0.95 }));
    assert!(matches!(a.get("pick"), Some(Answer::Choice { choice, .. }) if choice == "1"));
    assert!(matches!(a.get("risk"), Some(Answer::Score { probabilities, .. }) if probabilities[0].0 == "low"));
    assert!(matches!(a.get("safe"), Some(Answer::Failed { reason, .. }) if reason == "refusal"));
    assert!(!a.all_sure(0.85));
    assert_eq!(a.get("pick").unwrap().short(), "pick=1 (0.90)");
}

#[test]
fn sure_level_counts_both_sides_of_a_predicate() {
    let no = Answer::Predicate { name: "x".into(), probability: 0.05 };
    let unsure = Answer::Predicate { name: "x".into(), probability: 0.6 };
    assert!(no.sure() >= 0.95 - 1e-9);
    assert!(unsure.sure() < 0.85);
    let all = Answers { answers: vec![no], ..Answers::default() };
    assert!(all.all_sure(0.85));
    assert!(!Answers::default().all_sure(0.85));
}

#[test]
fn a_reply_without_answers_is_an_error() {
    assert!(parse(&json!({"error": "x"})).is_err());
}

#[test]
fn counts_per_day() {
    let u = count(Usage::default(), "2026-11-26", true, 0.01);
    let u = count(u, "2026-11-26", false, 0.0);
    assert_eq!((u.calls, u.failed), (2, 1));
    let u = count(u, "2026-11-27", true, 0.02);
    assert_eq!((u.day.as_str(), u.calls, u.failed), ("2026-11-27", 1, 0));
    assert!((u.cost - 0.02).abs() < 1e-9);
}

#[tokio::test]
async fn times_out_on_a_silent_server() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let began = std::time::Instant::now();
    let out = tokio::time::timeout(Duration::from_secs(25), send(&base, "p", &json!({}))).await.unwrap();
    assert!(out.unwrap_err().contains("20 s"));
    assert!(began.elapsed() < Duration::from_secs(22));
}

#[test]
fn settings_default_to_the_current_setup() {
    let s = settings::Settings::default();
    assert!(!s.on);
    assert_eq!(s.route(), "Current setup");
    assert!(!s.uses("questions"));
    assert_eq!(s.model, "jev-1.14");
    assert!((s.level - 0.85).abs() < 1e-9);
}

#[test]
fn settings_change_and_check_values() {
    let s = settings::apply(&settings::Settings::default(), &json!({"on": true, "review": false, "model": "gemini-decision-1", "level": 0.9})).unwrap();
    assert!(s.uses("questions") && !s.uses("review") && s.uses("drones") && !s.uses("other"));
    assert_eq!(s.route(), "Decision model first");
    assert_eq!(s.model, "gemini-decision-1");
    assert!(settings::apply(&s, &json!({"model": "gpt-5"})).is_err());
    assert!(settings::apply(&s, &json!({"level": 1.5})).is_err());
    assert!(settings::apply(&s, &json!({"on": "yes"})).is_err());
}

#[test]
fn saved_values_are_read_back_and_bad_ones_ignored() {
    let mut saved = serde_json::Map::new();
    saved.insert("decide.on".into(), json!(true));
    saved.insert("decide.level".into(), json!(7));
    saved.insert("decide.drones".into(), json!(false));
    let s = settings::from_saved(&saved);
    assert!(s.on && !s.drones);
    assert!((s.level - 0.85).abs() < 1e-9);
}

#[test]
fn test_route_builds_questions() {
    let q = routes::test_questions(&json!({"yes_no": "The agent waits.", "choices": ["1", " ", "2"]})).unwrap();
    assert_eq!(q.len(), 2);
    assert_eq!(q[1]["choices"], json!([{"value": "1"}, {"value": "2"}]));
    assert!(routes::test_questions(&json!({})).is_err());
}

#[test]
fn a_model_not_loaded_yet_is_tried_again() {
    assert!(super::not_loaded(r#"The decision call failed: 404 {"_tag":"ModelNotFoundError","modelID":"jev-1.14"}"#));
    assert!(!super::not_loaded("The decision model did not answer in 20 s."));
}

#[tokio::test]
async fn privacy_mode_stops_decision_calls_and_the_report_shows_the_path() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    settings::save(&app, &json!({"on": true})).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    tokio::spawn(async move {
        while listener.accept().await.is_ok() {
            seen.fetch_add(1, Ordering::SeqCst);
        }
    });
    lock(&app.cli.live).opencode = Some((base, "pw".into()));
    crate::settings_api::set_privacy(&app, true).unwrap();
    let refused = ask(&app, DEFAULT_MODEL, vec![text("terminal screen")], vec![predicate("waits", "Does it wait?")]).await.unwrap_err();
    assert_eq!(refused, PRIVATE);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    let report = crate::privacy::report::report(&crate::privacy::inputs(&app, dir.path()));
    let path = report.paths.iter().find(|p| p.area == "Decisions").expect("decision path");
    assert_eq!(path.reach, crate::privacy::report::Reach::Internet);
    assert!(!path.in_use && path.detail.contains("Privacy mode"), "{path:?}");
    crate::settings_api::set_privacy(&app, false).unwrap();
    let report = crate::privacy::report::report(&crate::privacy::inputs(&app, dir.path()));
    assert!(report.paths.iter().any(|p| p.area == "Decisions" && p.in_use));
    let _ = ask(&app, DEFAULT_MODEL, vec![text("x")], vec![predicate("waits", "Does it wait?")]).await;
    assert!(hits.load(Ordering::SeqCst) >= 1, "with privacy off the call goes out");
}

#[test]
fn every_use_is_off_when_the_decision_model_is_off() {
    let off = settings::Settings::default();
    assert!(!off.on);
    for name in settings::USES {
        assert!(!off.uses(name), "{name} must stay off while the main switch is off");
    }
    let on = settings::apply(&off, &json!({"on": true})).unwrap();
    for name in settings::USES {
        assert!(on.uses(name), "{name} is on by default once the main switch is on");
    }
    let one_off = settings::apply(&on, &json!({"routing": false})).unwrap();
    assert!(!one_off.uses("routing") && one_off.uses("chat"));
}

#[test]
fn a_picture_is_a_file_part() {
    let found = nixa_desktop::picture::Picture { media_type: "image/png", data: "iVBORw0KGgo=".into(), bytes: 8 };
    assert_eq!(picture(&found, "shot.png"), json!({"type": "file", "data": "iVBORw0KGgo=", "mime": "image/png", "name": "shot.png"}));
}
