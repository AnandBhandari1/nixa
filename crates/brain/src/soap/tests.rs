use super::scene::*;
use super::*;

fn ask() -> Ask {
    parse_ask(&json!({
        "cast": [
            {"id": "m3", "name": "Maya", "role": "staff", "space": "nixa", "status": "idle", "at": "sofa"},
            {"id": "m5", "name": "Ravi", "role": "staff", "space": "nixa", "status": "working", "at": "desk"},
            {"id": "m7", "name": "Lena", "role": "deep", "space": "web", "status": "done", "at": "tea"},
            {"id": "m9", "name": "Omar", "role": "staff", "space": "web", "status": "idle", "at": "window"}
        ],
        "work": [{"who": "m5", "name": "Ravi", "task": "Fix login page"}],
        "hour": 21
    }))
    .unwrap()
}

fn reply_of(content: &str) -> Value {
    json!({"message": {"role": "assistant", "content": content}})
}

#[test]
fn ask_is_parsed_and_cut() {
    let long = "x".repeat(200);
    let mut cast: Vec<Value> = (0..12).map(|i| json!({"id": format!("m{i}"), "name": format!("N{i}")})).collect();
    cast.push(json!({"id": "m1", "name": "again"}));
    let work: Vec<Value> = (0..9).map(|_| json!({"who": "m1", "task": long})).collect();
    let got = parse_ask(&json!({"cast": cast, "work": work, "hour": 99})).unwrap();
    assert_eq!(got.cast.len(), CAST_MAX);
    assert_eq!(got.work.len(), WORK_MAX);
    assert!(got.work.iter().all(|w| w.task.chars().count() == TASK_CHARS));
    assert_eq!(got.hour, 12);
    assert!(parse_ask(&json!({"cast": [{"id": "m1"}]})).is_err());
    assert!(parse_ask(&json!({})).is_err());
}

#[test]
fn prompt_has_cast_work_hour_and_story() {
    let text = user_message(&ask(), &["Maya, Ravi at the tea (silly). Maya: Who took my mug?".into()], &[]);
    for want in ["id m3: Maya", "Ravi works on \"Fix login page\"", "21:00", "Who took my mug?"] {
        assert!(text.contains(want), "{want}");
    }
    assert!(user_message(&ask(), &[], &[]).contains("first scene"));
    assert!(SYSTEM.contains("12 words") && SYSTEM.contains("office safe"));
    let sent = body("gemma", &ask(), &[], &[]);
    assert_eq!(sent["model"], "gemma");
    assert_eq!(sent["stream"], false);
    assert_eq!(sent["think"], false);
    assert_eq!(sent["keep_alive"], "2m");
    assert_eq!(sent["options"]["num_predict"], 400);
    assert_eq!(sent["messages"][0]["role"], "system");
    assert_eq!(sent["format"]["properties"]["lines"]["items"]["properties"]["who"]["enum"], json!(["m3", "m5", "m7", "m9"]));
}

#[test]
fn good_reply_is_kept() {
    let content = r#"{"where":"tea","cast":["m3","m5"],"mood":"giggly","lines":[{"who":"m3","text":"Ravi, is the login page fixed?","act":"sip"},{"who":"m5","text":"Almost, Maya. Tea first.","act":"laugh"}]}"#;
    let made = parse_reply(&reply_of(content), &ask()).unwrap();
    assert_eq!(made["where"], "tea");
    assert_eq!(made["cast"], json!(["m3", "m5"]));
    assert_eq!(made["mood"], "giggly");
    assert_eq!(made["lines"].as_array().unwrap().len(), 2);
    assert_eq!(made["lines"][1]["act"], "laugh");
    assert_eq!(summary(&made, &ask()), "Maya, Ravi at the tea (giggly). Maya: Ravi, is the login page fixed?");
}

#[test]
fn unknown_ids_places_and_acts_are_cleaned() {
    let content = r#"{"where":"roof","cast":["m3","zz"],"mood":"","lines":[{"who":"zz","text":"Hello","act":"none"},{"who":"m3","text":"Hi Lena","act":"dance"},{"who":"m7","text":"Hi Maya","act":"wave"}]}"#;
    let made = parse_reply(&reply_of(content), &ask()).unwrap();
    assert_eq!(made["where"], "sofa");
    assert_eq!(made["mood"], "cozy");
    assert_eq!(made["cast"], json!(["m3", "m7"]));
    assert_eq!(made["lines"].as_array().unwrap().len(), 2);
    assert_eq!(made["lines"][0]["act"], "none");
}

#[test]
fn long_text_and_too_many_lines_are_cut() {
    let long = "word ".repeat(40);
    let ids = ["m3", "m5", "m7", "m9"];
    let lines: Vec<Value> = (0..12)
        .map(|i| {
            let text = if i == 0 { long.clone() } else { format!("line {i}") };
            json!({"who": ids[i % 4], "text": text, "act": "nod"})
        })
        .collect();
    let raw = json!({"where": "window", "cast": ["m3", "m5", "m7", "m9"], "mood": "a very very long mood word here", "lines": lines});
    let made = clean(&raw, &ask()).unwrap();
    let kept = made["lines"].as_array().unwrap();
    assert!(kept.len() <= LINES_MAX);
    assert_eq!(made["cast"].as_array().unwrap().len(), SCENE_CAST_MAX);
    assert!(kept.iter().all(|l| made["cast"].as_array().unwrap().contains(&l["who"])));
    assert!(kept[0]["text"].as_str().unwrap().chars().count() <= TEXT_CHARS);
    assert!(made["mood"].as_str().unwrap().chars().count() <= MOOD_CHARS);
}

#[test]
fn broken_replies_are_errors() {
    assert!(parse_reply(&reply_of("not json {"), &ask()).is_err());
    assert!(parse_reply(&json!({"done": true}), &ask()).is_err());
    let one = r#"{"where":"tea","cast":["m3"],"mood":"x","lines":[{"who":"m3","text":"Alone","act":"none"}]}"#;
    assert!(parse_reply(&reply_of(one), &ask()).is_err());
    let empty = r#"{"where":"tea","cast":["m3","m5"],"mood":"x","lines":[{"who":"m3","text":"","act":"none"},{"who":"m5","text":"Hi","act":"none"}]}"#;
    assert!(parse_reply(&reply_of(empty), &ask()).is_err());
}

#[test]
fn story_resets_on_a_new_date_and_keeps_twelve() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(memory::FILE);
    let mut story = memory::load(&path, "2026-01-01");
    assert!(story.scenes.is_empty());
    for i in 0..15 {
        memory::add(&mut story, format!("scene {i}"));
    }
    assert_eq!(story.scenes.len(), memory::KEEP);
    assert_eq!(story.scenes[0], "scene 3");
    memory::save(&path, &story).unwrap();
    assert_eq!(memory::load(&path, "2026-01-01").scenes.len(), memory::KEEP);
    let next = memory::load(&path, "2026-01-02");
    assert_eq!(next.date, "2026-01-02");
    assert!(next.scenes.is_empty());
}

#[test]
fn gate_limits_requests() {
    let mut gate = Gate::default();
    assert!(open_gate(&mut gate, 1_000));
    assert!(!open_gate(&mut gate, 1_000 + GAP_MS + 1));
    gate.running = false;
    assert!(!open_gate(&mut gate, 1_000 + GAP_MS - 1));
    assert!(open_gate(&mut gate, 1_000 + GAP_MS));
}

#[test]
fn settings_default_and_model_name() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    assert_eq!(model(&app), DEFAULT_MODEL);
    assert_eq!(url(&app), DEFAULT_URL);
    assert!(!used(&app));
    assert_eq!(set_model(&app, " qwen3:4b ").unwrap(), "qwen3:4b");
    assert_eq!(model(&app), "qwen3:4b");
    assert!(set_model(&app, "").is_err());
    assert!(set_model(&app, "bad name;rm").is_err());
    assert_eq!(model(&app), "qwen3:4b");
    app.save_settings(|s| s.office_soap_url = Some("http://localhost:9999/".into())).unwrap();
    assert_eq!(url(&app), "http://localhost:9999");
}

#[test]
fn tags_and_events() {
    let tags = json!({"models": [{"name": "b:1"}, {"model": "a:2"}, {"name": "b:1"}]});
    assert_eq!(names_of(&tags), vec!["a:2", "b:1"]);
    let ok = models_event("a:2", DEFAULT_URL, Ok(vec!["a:2".into()]));
    assert_eq!(ok["type"], MODELS_KIND);
    assert!(ok["error"].is_null());
    let bad = models_event("a:2", DEFAULT_URL, Err("down".into()));
    assert_eq!(bad["error"], "down");
    assert_eq!(scene_event(Err("no".into())), json!({"type": SCENE_EVENT, "error": "no"}));
    assert_eq!(sample_ask().cast.len(), 3);
}

#[tokio::test]
async fn bad_cast_sends_an_error_event() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let mut rx = app.out.subscribe();
    assert!(ws(&app, SOAP_KIND, &json!({"cast": []})));
    let got = rx.recv().await.unwrap();
    assert_eq!(got["type"], SCENE_EVENT);
    assert!(got["error"].is_string());
    assert!(!used(&app));
    assert!(!ws(&app, "something_else", &json!({})));
}

#[test]
fn a_speaker_never_says_their_own_name() {
    use crate::soap::scene::no_own_name;
    let names = ["Hugo", "Wren", "Sami", "Ada"];
    assert_eq!(no_own_name("Hugo: Hugo, did the kettle behave?", "Hugo", &names), "Did the kettle behave?");
    assert_eq!(no_own_name("Wren: Another screenshot, Wren.", "Wren", &names), "Another screenshot.");
    assert_eq!(no_own_name("Sami, another screenshot?", "Sami", &names), "Another screenshot?");
    assert_eq!(no_own_name("Ada, another screenshot?", "Sami", &names), "Ada, another screenshot?");
    assert_eq!(no_own_name("Ada: tea time", "Ada", &names), "Tea time");
}
