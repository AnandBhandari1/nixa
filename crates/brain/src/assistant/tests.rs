use super::answer::{self, Hit, Note, Personal, Web};
use super::jobs::{self, Job};
use super::route::{self, Route};
use super::when::{local_day_time, reminder_at, to_minute};
use crate::app::App;
use crate::schedules::zone::{self, DAY_S, days_from_civil};
use crate::tools::Args;
use nixa_store::schedules::RunResult;
use nixa_store::tasks::{self as store, TaskStatus};
use serde_json::{Value, json};
use std::sync::Arc;

fn args(v: Value) -> Args {
    v.as_object().cloned().unwrap_or_default()
}

fn utc(y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
    (days_from_civil(y, m, d) * DAY_S + h * 3600 + min * 60) * 1000
}

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    crate::web::setup::change(&app, &json!({"on": false})).unwrap();
    (dir, app)
}

#[test]
fn routes_keep_private_questions_local() {
    let none: Vec<String> = Vec::new();
    assert_eq!(route::route("What is the stock price of AAPL?", &none).route, Route::Web);
    assert_eq!(route::route("What is the news in AI today?", &none).route, Route::Web);
    assert_eq!(route::route("Where is my email from Sam?", &none).route, Route::Connector);
    assert_eq!(route::route("What is on my calendar?", &none).route, Route::Connector);
    assert_eq!(route::route("What did I save about the shop?", &none).route, Route::Memory);
    assert_eq!(route::route("What is the price of my car today?", &none).route, Route::Mix);
    assert_eq!(route::route("How do lists work in Rust?", &none).route, Route::Knowledge);
    let names = vec!["Robin".to_string()];
    assert_eq!(route::route("What is Robin's latest plan?", &names).route, Route::Mix);
    assert_eq!(route::route("What is robinhood stock price?", &names).route, Route::Web);
    assert!(route::route("Where is my email?", &none).personal);
}

#[test]
fn fast_lookups_and_reminders() {
    assert!(route::is_fast("What is the stock price of AAPL?", "Nixa"));
    assert!(route::is_fast("Nixa, weather in Oslo", "Nixa"));
    assert!(route::is_fast("news", "Nixa"));
    assert!(!route::is_fast("Fix the news page", "Nixa"));
    assert!(!route::is_fast("What is the plan?", "Nixa"));
    assert!(!route::is_fast("Send the latest version to the shop", "Nixa"));
    let pick = |q: &str| route::reminder(q, "Nixa");
    assert_eq!(pick("Remind me at 5 to call home"), Some(("at 5".into(), "call home".into())));
    assert_eq!(pick("remind me in 20 minutes to stretch"), Some(("in 20 minutes".into(), "stretch".into())));
    assert_eq!(pick("Nixa, remind me to talk to Sam at 5"), Some(("at 5".into(), "talk to Sam".into())));
    assert_eq!(pick("remind me to water the plants tomorrow at 9 am"), Some(("tomorrow at 9 am".into(), "water the plants".into())));
    assert_eq!(pick("remind me tomorrow"), Some(("tomorrow".into(), "Personal reminder".into())));
    assert_eq!(pick("what is the weather"), None);
    assert_eq!(route::strip_name("Nixa, check news", "Nixa"), "check news");
    assert_eq!(route::strip_name("Nixasz says hi", "Nixa"), "Nixasz says hi");
}

#[test]
fn reminder_times() {
    let z = zone::utc();
    let now = utc(2026, 11, 22, 14, 30);
    assert_eq!(reminder_at("in 20 minutes", now, &z), Ok(now + 20 * 60_000));
    assert_eq!(reminder_at("in an hour", now, &z), Ok(now + 3_600_000));
    assert_eq!(reminder_at("in 2 days", now, &z), Ok(now + 2 * DAY_S * 1000));
    assert_eq!(reminder_at("at 5", now, &z), Ok(utc(2026, 11, 22, 17, 0)));
    assert_eq!(reminder_at("at 2", now, &z), Ok(utc(2026, 11, 23, 2, 0)));
    assert_eq!(reminder_at("5:15 pm", now, &z), Ok(utc(2026, 11, 22, 17, 15)));
    assert_eq!(reminder_at("at 9 am", now, &z), Ok(utc(2026, 11, 23, 9, 0)));
    assert_eq!(reminder_at("at 16:00", now, &z), Ok(utc(2026, 11, 22, 16, 0)));
    assert_eq!(reminder_at("at 13:00", now, &z), Ok(utc(2026, 11, 23, 13, 0)));
    assert_eq!(reminder_at("tomorrow at 9 am", now, &z), Ok(utc(2026, 11, 23, 9, 0)));
    assert_eq!(reminder_at("today at 6 pm", now, &z), Ok(utc(2026, 11, 22, 18, 0)));
    assert!(reminder_at("today at 1 pm", now, &z).is_err());
    assert!(reminder_at("today at 13:00", now, &z).is_err());
    assert_eq!(reminder_at("2026-11-23T08:00+01:00", now, &z), Ok(utc(2026, 11, 23, 7, 0)));
    assert_eq!(reminder_at("2026-11-23T08:00Z", now, &z), Ok(utc(2026, 11, 23, 8, 0)));
    assert!(reminder_at("2026-02-30T08:00Z", now, &z).is_err());
    assert!(reminder_at("2026-11-01T08:00Z", now, &z).is_err());
    assert_eq!(reminder_at("at 5 p.m.", now, &z), Ok(utc(2026, 11, 22, 17, 0)));
    assert_eq!(reminder_at("tomorrow at 9 a.m.", now, &z), Ok(utc(2026, 11, 23, 9, 0)));
    assert_eq!(reminder_at("5:15 P.M.", now, &z), Ok(utc(2026, 11, 22, 17, 15)));
    assert_eq!(route::reminder("Remind me at 5 p.m. to stretch", "Nixa"), Some(("at 5 p.m.".into(), "stretch".into())));
    assert!(reminder_at("someday", now, &z).is_err());
    assert!(reminder_at("at 25", now, &z).is_err());
    assert!(reminder_at("at 13 pm", now, &z).is_err());
    assert!(reminder_at("in 0 minutes", now, &z).is_err());
    assert!(reminder_at("in 99999999999 days", now, &z).is_err());
    assert_eq!(to_minute(now + 1), now + 60_000);
    assert_eq!(to_minute(now), now);
    assert_eq!(local_day_time(&z, utc(2026, 11, 23, 7, 5)), ("2026-11-23".into(), "07:05".into()));
    let plus3 = zone::named("Etc/GMT-3").unwrap_or_else(zone::utc);
    if plus3.name != "UTC" {
        assert_eq!(reminder_at("at 6 pm", now, &plus3), Ok(utc(2026, 11, 22, 15, 0)));
        assert_eq!(local_day_time(&plus3, utc(2026, 11, 22, 22, 0)), ("2026-11-23".into(), "01:00".into()));
    }
}

#[test]
fn jobs_read_back_and_bad_text_fails() {
    let briefing = Job::Briefing { place: "Oslo".into(), topics: vec!["AI".into(), "space".into()] };
    let text = jobs::prompt_of(&briefing, "");
    assert_eq!(text, "Morning briefing\nWeather place: Oslo\nNews topics: AI, space");
    assert_eq!(jobs::read_job(&text), Ok(briefing));
    assert_eq!(jobs::read_job("Morning briefing"), Ok(Job::Briefing { place: String::new(), topics: Vec::new() }));
    let reminder = Job::Reminder { task: 7, at: 1_800_000_000_000 };
    let text = jobs::prompt_of(&reminder, "Sun 2027-01-17 08:00");
    assert_eq!(jobs::read_job(&format!("  {text}\n\n")), Ok(reminder));
    assert!(jobs::read_job("Personal reminder\nTask number: 7").is_err());
    assert!(jobs::read_job("Do a code review").is_err());
    let topics = jobs::clean_topics(&["AI, ai, Space".into(), "".into(), "a,b,c,d,e,f".into()]);
    assert_eq!(topics, vec!["AI", "Space", "a", "b", "c"]);
}

#[test]
fn answer_texts_name_sources() {
    assert!(answer::live_text("web search", "AAPL stock", &Web::Off(String::new()), "now").starts_with("Live data is off: web search is not set up."));
    assert_eq!(answer::live_text("web search", "x", &Web::Off("Web search is off. The user can turn it on in Settings.".into()), "now"), "Live data is off: Web search is off. The user can turn it on in Settings. I cannot check a price, headline or forecast.");
    let text = answer::live_text("web search (Exa)", "AAPL share price", &Web::Text("1. Page (2026-11-22)\nhttps://example.com\nAAPL 1.00".into()), "now");
    assert!(text.contains("Retrieved now via web search (Exa).") && text.contains("not a checked real time quote"), "{text}");
    assert!(answer::live_text("w", "x", &Web::Text(" ".into()), "now").contains("no results"));
    assert_eq!(super::sources::web_result(Err(crate::web::OFF.into())), Web::Off(String::new()));
    assert_eq!(super::sources::web_result(Err(crate::web::NO_EXA_KEY.into())), Web::Off(super::sources::NO_KEY.into()));
    assert_eq!(super::sources::web_result(Err("timeout".into())), Web::Failed);
    assert_eq!(super::sources::web_result(Ok("hits".into())), Web::Text("hits".into()));
    assert!(answer::live_text("web search", "x", &Web::Failed, "now").contains("failed"));
    assert!(answer::live_text("web search", "x", &Web::Hits(Vec::new()), "now").contains("no results"));
    let hit = Hit { title: "Market page".into(), url: "https://example.com/q".into(), text: "AAPL at 1.00".into(), date: String::new() };
    let text = answer::live_text("Search", "AAPL stock price", &Web::Hits(vec![hit]), "Sun 2026-11-22 10:00");
    assert!(text.contains("Source: Market page (https://example.com/q), via Search; published time unknown, retrieved Sun 2026-11-22 10:00."));
    assert!(text.contains("not a checked real time quote"));
    assert_eq!(answer::memory_text(&[], "now"), None);
    assert_eq!(answer::memory_text(&["a  b".into(), "c".into(), "d".into()], "now").unwrap(), "Memory (saved notes, not live; read now): a b; c");
    let note = Note { text: "Lists grow.".into(), source: "docs".into(), path: "guide.md".into(), indexed: String::new() };
    assert_eq!(answer::knowledge_text(&[note]).unwrap(), "Lists grow. Source: local docs, guide.md; indexed time unknown, not live.");
    assert_eq!(answer::personal_text(None, "now"), answer::NO_CONNECTOR);
    assert!(answer::personal_text(Some(Err("x".into())), "now").contains("failed"));
    let found = Personal { name: "mail.search".into(), text: "One mail".into(), observed: "today".into() };
    assert_eq!(answer::personal_text(Some(Ok(found)), "now"), "One mail Source: mail.search; seen today, retrieved now.");
    assert_eq!(answer::short("é".repeat(5).as_str(), 3), "ééé");
    assert_eq!(jobs::work_line(&[]), "Uncommitted work: no spaces yet.");
    assert_eq!(jobs::work_line(&[("shop".into(), None)]), "Uncommitted work: no git folders in your spaces.");
    assert_eq!(jobs::work_line(&[("shop".into(), Some(2)), ("site".into(), Some(0))]), "Uncommitted work: shop 2 changed files, site none. Source: git status, checked now.");
}

#[tokio::test]
async fn lookups_use_memory_and_say_when_live_data_is_off() {
    let (_dir, app) = app();
    app.memory.add("The shop key is in the blue box");
    let text = super::lookup::answer(&app, "Where is my shop key?", None).await;
    assert!(text.starts_with("Memory (saved notes, not live; read "), "{text}");
    assert!(text.contains("blue box"));
    let text = super::lookup::answer(&app, "What is the stock price of AAPL?", None).await;
    assert!(text.starts_with("Live data is off"), "{text}");
    let text = super::lookup::answer(&app, "What is my shop worth today?", None).await;
    assert!(text.ends_with(answer::SPLIT_PUBLIC), "{text}");
    let text = super::lookup::answer(&app, "Where is my email from Sam?", None).await;
    assert!(text.ends_with(answer::NO_CONNECTOR), "{text}");
    assert_eq!(super::quick(&app, "What is the weather?", true).await, None);
    assert!(super::quick(&app, "What is the weather?", false).await.unwrap().starts_with("Live data is off"));
    assert!(!super::wants_fast(&app, "What is the weather?"));
    assert!(super::wants_fast(&app, "remind me in 5 minutes to stand up"));
}

#[tokio::test]
async fn reminders_are_tasks_with_one_schedule_and_speak_once() {
    let (_dir, app) = app();
    let out = super::call(&app, super::REMIND, &args(json!({"text": "Call the shop", "when": "in 20 minutes", "task": 3}))).await.unwrap().unwrap();
    assert!(out.starts_with("Reminder saved as task #"), "{out}");
    let conn = crate::schedules::jobs::conn(&app).unwrap();
    let tasks = store::list_tasks(&conn, &Default::default()).unwrap();
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.title, "Call the shop");
    assert_eq!(task.status, TaskStatus::Todo);
    assert!(task.space_id.is_empty() && task.assigned.is_empty());
    assert!(task.description.contains("Follow up on task #3."));
    let list = jobs::assistant_schedules(&app);
    assert_eq!(list.len(), 1);
    let s = &list[0];
    assert_eq!(s.name, format!("Reminder #{}", task.id));
    assert!(s.enabled && s.next_run.is_some());
    let view = super::view(&app);
    assert_eq!(view["reminders"][0]["task"], json!(task.id));
    assert_eq!(view["reminders"][0]["title"], json!("Call the shop"));
    assert_eq!(view["briefing"], Value::Null);
    let run = jobs::run_schedule(&app, s).await;
    assert_eq!(run.result, RunResult::Started, "{}", run.error);
    assert!(run.done_at.is_some() && std::path::Path::new(&run.report).exists());
    let said: Vec<String> = crate::app::lock(&app.chat).list().iter().map(|i| i.text.clone()).collect();
    assert!(said.iter().any(|t| t.starts_with("Reminder: Call the shop.")), "{said:?}");
    let again = jobs::execute(&app, &jobs::read_job(&s.prompt).unwrap()).await.unwrap();
    assert!(again.ends_with("was already given."), "{again}");
    store::set_status(&conn, task.id, TaskStatus::Done, None, false).unwrap();
    let job = Job::Reminder { task: task.id, at: 1 };
    let skipped = jobs::execute(&app, &job).await.unwrap();
    assert!(skipped.contains("skipped: the task is done"), "{skipped}");
    assert_eq!(super::view(&app)["reminders"], json!([]));
    let bad = super::call(&app, super::REMIND, &args(json!({"text": "x", "when": "someday"}))).await.unwrap();
    assert!(bad.is_err());
    let gone = jobs::execute(&app, &Job::Reminder { task: 999, at: 1 }).await.unwrap();
    assert!(gone.contains("the task is gone"));
}

#[tokio::test]
async fn briefing_has_no_default_place_and_can_pause() {
    let (_dir, app) = app();
    assert!(jobs::briefing_on(&app, false).is_err());
    let out = jobs::save_briefing(&app, &Args::new()).unwrap();
    assert!(out.contains("daily at 08:00"), "{out}");
    assert!(out.contains("No weather place set.") && out.contains("No news topics set."), "{out}");
    let s = jobs::find(&app, jobs::BRIEFING_NAME).unwrap();
    assert_eq!(s.prompt, "Morning briefing");
    assert_eq!(s.repeat, nixa_store::schedules::Repeat::Daily);
    let out = jobs::save_briefing(&app, &args(json!({"time": "07:30", "place": "Oslo", "topics": ["AI", "space"]}))).unwrap();
    assert!(out.contains("Weather for Oslo.") && out.contains("News about AI, space."), "{out}");
    let out = jobs::save_briefing(&app, &args(json!({"enabled": false}))).unwrap();
    assert!(out.contains("07:30") && out.contains("Oslo") && out.contains("Paused"), "{out}");
    assert_eq!(jobs::assistant_schedules(&app).len(), 1);
    let s = jobs::find(&app, jobs::BRIEFING_NAME).unwrap();
    assert!(!s.enabled && s.next_run.is_none());
    jobs::briefing_on(&app, true).unwrap();
    assert!(jobs::find(&app, jobs::BRIEFING_NAME).unwrap().next_run.is_some());
    assert!(jobs::save_briefing(&app, &args(json!({"time": "8am"}))).is_err());
    let view = super::view(&app);
    assert_eq!(view["briefing"]["place"], json!("Oslo"));
    assert_eq!(view["briefing"]["topics"], json!(["AI", "space"]));
    assert_eq!(view["briefing"]["enabled"], json!(true));
    let run = jobs::run_schedule(&app, &s).await;
    assert_eq!(run.result, RunResult::Started);
    let report = std::fs::read_to_string(&run.report).unwrap();
    assert!(report.contains("Weather: Live data is off"), "{report}");
    assert!(report.contains("0 open tasks."), "{report}");
    assert!(report.contains("## Tasks\n\nNone"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&run.report).unwrap().permissions().mode() & 0o777, 0o600);
    let mut broken = s.clone();
    broken.prompt = "Review the code".into();
    let failed = jobs::run_schedule(&app, &broken).await;
    assert_eq!(failed.result, RunResult::Failed);
    assert_eq!(failed.error, jobs::BAD_JOB);
}

#[tokio::test]
async fn window_actions_answer_and_fall_back_to_chat() {
    let (_dir, app) = app();
    let out = super::action(&app, &json!({"action": "ask", "question": "remind me at 9 am to stretch"})).await.unwrap();
    assert!(out["answer"].as_str().unwrap().starts_with("Reminder saved as task #"));
    assert_eq!(out["view"]["reminders"].as_array().unwrap().len(), 1);
    let out = super::action(&app, &json!({"action": "ask", "question": "news today"})).await.unwrap();
    assert!(out["answer"].as_str().unwrap().starts_with("Live data is off"));
    assert!(crate::privacy::privacy_on(&app));
    for q in ["What is the weather where your owner lives?", "What is the latest version on this computer?"] {
        assert!(route::is_fast(q, "Nixa"), "{q}");
        assert!(super::private(&app, q), "{q}");
        let out = super::action(&app, &json!({"action": "ask", "question": q})).await.unwrap();
        assert_eq!(out["answer"], Value::Null, "{q}");
        assert_eq!(out["sent"], json!(true), "{q}");
    }
    assert!(!super::private(&app, "news today"));
    let names = crate::privacy::names_of(&app);
    crate::app::lock(&app.privacy.guard).guard("Where does your owner live?", true, crate::util::now_ms(), &names);
    assert!(super::private(&app, "And today?"));
    crate::app::lock(&app.privacy.guard).last = None;
    assert!(!super::private(&app, "And today?"));
    let out = super::action(&app, &json!({"action": "ask", "question": "Plan my week"})).await.unwrap();
    assert_eq!(out["sent"], json!(true));
    assert_eq!(out["notice"], json!(super::SENT));
    let out = super::action(&app, &json!({"action": "briefing", "time": "06:45", "topics": "AI, books"})).await.unwrap();
    assert_eq!(out["view"]["briefing"]["topics"], json!(["AI", "books"]));
    let out = super::action(&app, &json!({"action": "briefing_on", "enabled": false})).await.unwrap();
    assert_eq!(out["view"]["briefing"]["enabled"], json!(false));
    assert!(super::action(&app, &json!({"action": "nope"})).await.is_err());
    assert!(super::action(&app, &json!({})).await.is_err());
    let names: Vec<String> = super::specs().into_iter().map(|s| s.name).collect();
    assert_eq!(names, super::NAMES.to_vec());
    assert!(super::call(&app, "other", &Args::new()).await.is_none());
}

#[tokio::test]
async fn cancellation_closes_the_reminder_task_and_removes_only_its_schedule() {
    let (_dir, app) = app();
    jobs::save_briefing(&app, &Args::new()).unwrap();
    let briefing = jobs::find(&app, jobs::BRIEFING_NAME).unwrap();
    assert!(super::action(&app, &json!({"action": "remind_cancel", "id": briefing.id})).await.is_err());

    let out = super::action(&app, &json!({"action": "remind", "text": "Stretch", "when": "in 20 minutes"})).await.unwrap();
    let reminder = &out["view"]["reminders"][0];
    let id = reminder["id"].as_str().unwrap();
    let task = reminder["task"].as_i64().unwrap();
    let out = super::action(&app, &json!({"action": "remind_cancel", "id": id})).await.unwrap();
    assert_eq!(out["view"]["reminders"], json!([]));
    assert_eq!(out["view"]["briefing"]["id"], briefing.id);
    let conn = crate::schedules::jobs::conn(&app).unwrap();
    assert_eq!(store::must_task(&conn, task).unwrap().status, TaskStatus::Cancelled);
    assert!(nixa_store::schedules::get_schedule(&conn, id).unwrap().is_none());
}

#[test]
fn the_model_route_never_sends_a_personal_question_to_the_web() {
    use super::route::{route, route_of_word, with_model};
    let personal = route("what is my car number", &[]);
    assert!(personal.personal);
    assert_eq!(with_model(personal.clone(), Route::Web), personal);
    assert_eq!(with_model(personal, Route::Mix).route, Route::Mix);
    let mail = route("check my email", &[]);
    assert_eq!(with_model(mail.clone(), Route::Knowledge), mail);
    let plain = route("how does a compiler work", &[]);
    let web = with_model(plain, Route::Web);
    assert_eq!(web.route, Route::Web);
    assert!(web.live && !web.personal);
    assert_eq!(route_of_word("memory"), Some(Route::Memory));
    assert_eq!(route_of_word("connector"), None);
}
