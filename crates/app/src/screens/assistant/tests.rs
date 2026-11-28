use super::api::{Call, Host};
use super::model::{self, Briefing, Reminder, Sources};
use super::view::{AssistantView, set, value};
use gpui::{App, Entity, Modifiers, TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

static OPENED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn fake_tasks(_: &mut App) {
    OPENED.lock().unwrap().push("tasks");
}

fn fake_schedules(_: &mut App) {
    OPENED.lock().unwrap().push("schedules");
}

fn host() -> Host {
    Host { tasks: fake_tasks, schedules: fake_schedules }
}

type Log = Arc<Mutex<Vec<Value>>>;

fn view_json(briefing: Value, reminders: Value) -> Value {
    json!({"briefing": briefing, "reminders": reminders, "zone": "Etc/UTC", "name": "Nixa", "sources": {"web": false, "web_name": "web search", "knowledge": false, "connectors": false}})
}

fn fake(log: &Log, briefing: Arc<Mutex<Value>>) -> Call {
    let log = log.clone();
    Arc::new(move |body: Value| {
        log.lock().unwrap().push(body.clone());
        let reminders = json!([{"id": "s2", "task": 4, "title": "Call the shop", "due": "Mon 2026-11-23 09:00", "waiting": true}]);
        match body["action"].as_str().unwrap_or_default() {
            "view" => Ok(json!({"view": view_json(briefing.lock().unwrap().clone(), reminders)})),
            "remind_cancel" => Ok(json!({"text": "Cancelled reminder #4.", "view": view_json(briefing.lock().unwrap().clone(), json!([]))})),
            "ask" if body["question"] == json!("Plan my week") => Ok(json!({"answer": null, "sent": true, "notice": "Sent to Nixa. The answer shows in Nixa's chat.", "view": view_json(Value::Null, reminders)})),
            "ask" => Ok(json!({"answer": "Live data is off: web search is not set up.\nSecond line", "sent": false, "notice": "Also in Nixa's chat.", "view": view_json(Value::Null, reminders)})),
            "briefing" => {
                let b = json!({"id": "s1", "time": body["time"], "enabled": body["enabled"], "place": body["place"], "topics": body["topics"], "next": "Mon 2026-11-23 07:30", "last": "Last: never"});
                *briefing.lock().unwrap() = b.clone();
                Ok(json!({"text": "Morning briefing saved in Schedules.", "view": view_json(b, reminders)}))
            }
            "briefing_on" => {
                let mut b = briefing.lock().unwrap();
                b["enabled"] = body["enabled"].clone();
                Ok(json!({"text": "Morning briefing is paused.", "view": view_json(b.clone(), reminders)}))
            }
            _ => Err("unknown".into()),
        }
    })
}

fn open(cx: &mut TestAppContext, call: Call) -> (Entity<AssistantView>, &mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let (view, vcx) = cx.add_window_view(move |_, cx| AssistantView::new(call, host(), cx));
    vcx.run_until_parked();
    (view, vcx)
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.run_until_parked();
    let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn drawn(vcx: &mut VisualTestContext, selector: &'static str) -> bool {
    vcx.run_until_parked();
    vcx.debug_bounds(selector).is_some()
}

fn actions(log: &Log) -> Vec<String> {
    log.lock().unwrap().iter().map(|b| b["action"].as_str().unwrap_or_default().to_string()).collect()
}

#[test]
fn model_lines_have_no_personal_defaults() {
    let state = model::parse(&view_json(json!({"id": "s1", "time": "08:00", "enabled": true, "place": "", "topics": [], "next": "Mon 08:00"}), json!([]))).unwrap();
    let b = state.briefing.clone().unwrap();
    assert_eq!(model::briefing_line(Some(&b)), "Daily at 08:00, no weather place, no news topics. On, next Mon 08:00.");
    assert_eq!(model::briefing_line(None), "Not set up. Nothing runs until you save it.");
    let b = Briefing { time: "07:30".into(), place: "Oslo".into(), topics: vec!["AI".into(), "books".into()], ..Briefing::default() };
    assert_eq!(model::briefing_line(Some(&b)), "Daily at 07:30, weather for Oslo, news about AI, books. Paused.");
    assert_eq!(model::sources_line(&state.sources), "Web search: off. Local knowledge: off. Email and calendar: no connector.");
    let on = Sources { web: true, web_name: "local search".into(), knowledge: true, connectors: true };
    assert_eq!(model::sources_line(&on), "Local search: on. Local knowledge: on. Email and calendar: connected.");
    let r = Reminder { task: 4, title: "Call".into(), due: "Mon 09:00".into(), waiting: false, ..Reminder::default() };
    assert_eq!(model::reminder_line(&r), "#4 Call · due Mon 09:00 (schedule paused)");
    assert_eq!(model::topics_of(" AI, ,books "), vec!["AI", "books"]);
    assert_eq!(model::briefing_body(" ", "", "", true), json!({"action": "briefing", "time": "08:00", "place": "", "topics": [], "enabled": true}));
    assert_eq!(model::parse(&Value::Null), None);
    assert_eq!(model::parse(&json!({})).unwrap(), model::State::default());
}

#[gpui::test]
fn ask_shows_the_answer_and_lists_reminders(cx: &mut TestAppContext) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let (view, vcx) = open(cx, fake(&log, Arc::new(Mutex::new(Value::Null))));
    assert_eq!(actions(&log), vec!["view"]);
    assert!(drawn(vcx, "assistant-reminder-4"));
    assert!(!drawn(vcx, "assistant-answer"));
    assert!(!drawn(vcx, "assistant-briefing-on"));
    click(vcx, "assistant-ask");
    assert_eq!(actions(&log), vec!["view"]);
    let input = vcx.update(|_, cx| view.read(cx).inputs.ask.clone());
    vcx.update(|_, cx| set(&input, "What is the weather?", cx));
    click(vcx, "assistant-ask");
    assert_eq!(log.lock().unwrap().last().unwrap(), &json!({"action": "ask", "question": "What is the weather?"}));
    assert!(drawn(vcx, "assistant-answer"));
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.answer.as_deref(), Some("Live data is off: web search is not set up.\nSecond line"));
        assert_eq!(v.notice, "Also in Nixa's chat.");
        assert_eq!(value(&v.inputs.ask, cx), "");
    });
    vcx.update(|_, cx| set(&input, "Plan my week", cx));
    click(vcx, "assistant-ask");
    assert_eq!(log.lock().unwrap().last().unwrap(), &json!({"action": "ask", "question": "Plan my week"}));
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.answer, None);
        assert_eq!(v.notice, "Sent to Nixa. The answer shows in Nixa's chat.");
    });
    OPENED.lock().unwrap().clear();
    click(vcx, "assistant-tasks");
    click(vcx, "assistant-schedules");
    assert_eq!(*OPENED.lock().unwrap(), vec!["tasks", "schedules"]);
}

#[gpui::test]
fn briefing_saves_and_pauses(cx: &mut TestAppContext) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let (view, vcx) = open(cx, fake(&log, Arc::new(Mutex::new(Value::Null))));
    let (time, place, topics) = vcx.update(|_, cx| {
        let v = view.read(cx);
        (v.inputs.time.clone(), v.inputs.place.clone(), v.inputs.topics.clone())
    });
    vcx.update(|_, cx| {
        set(&time, "07:30", cx);
        set(&place, "Oslo", cx);
        set(&topics, "AI, books", cx);
    });
    click(vcx, "assistant-save-briefing");
    assert_eq!(log.lock().unwrap().last().unwrap(), &json!({"action": "briefing", "time": "07:30", "place": "Oslo", "topics": ["AI", "books"], "enabled": true}));
    assert!(drawn(vcx, "assistant-briefing-on"));
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.notice, "Morning briefing saved in Schedules.");
        assert_eq!(v.state.as_ref().unwrap().briefing.as_ref().unwrap().place, "Oslo");
        assert_eq!(value(&v.inputs.topics, cx), "AI, books");
    });
    click(vcx, "assistant-briefing-on");
    assert_eq!(log.lock().unwrap().last().unwrap(), &json!({"action": "briefing_on", "enabled": false}));
    vcx.update(|_, cx| assert!(!view.read(cx).state.as_ref().unwrap().briefing.as_ref().unwrap().enabled));
    click(vcx, "assistant-save-briefing");
    assert_eq!(log.lock().unwrap().last().unwrap()["enabled"], json!(false));
}

#[gpui::test]
fn a_saved_briefing_fills_the_form(cx: &mut TestAppContext) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let saved = json!({"id": "s9", "time": "06:15", "enabled": true, "place": "Lima", "topics": ["books"], "next": "x", "last": "y"});
    let (view, vcx) = open(cx, fake(&log, Arc::new(Mutex::new(saved))));
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(value(&v.inputs.time, cx), "06:15");
        assert_eq!(value(&v.inputs.place, cx), "Lima");
        assert_eq!(value(&v.inputs.topics, cx), "books");
    });
    assert!(drawn(vcx, "assistant-briefing-on"));
}

#[gpui::test]
fn a_missing_brain_shows_the_error(cx: &mut TestAppContext) {
    let call: Call = Arc::new(|_| Err("cannot reach nixa-brain at 127.0.0.1:7421: connection refused".into()));
    let (view, vcx) = open(cx, call);
    assert!(drawn(vcx, "assistant-error"));
    let input = vcx.update(|_, cx| view.read(cx).inputs.ask.clone());
    vcx.update(|_, cx| set(&input, "news", cx));
    click(vcx, "assistant-ask");
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.notice, "Could not connect to Nixa.");
        assert!(!v.asking);
        assert_eq!(value(&v.inputs.ask, cx), "news");
    });
}

#[test]
fn commands_open_the_window() {
    let list = super::commands();
    assert_eq!(list[0].0, super::TITLE);
    assert_eq!(list.len(), 2);
}

#[gpui::test]
fn brain_rejection_and_reminder_cancellation_are_shown(cx: &mut TestAppContext) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let make = fake(&log, Arc::new(Mutex::new(Value::Null)));
    let call: Call = Arc::new(move |body| {
        if body["action"] == "ask" {
            return Err("Question is too long.".into());
        }
        make(body)
    });
    let (view, vcx) = open(cx, call);
    let input = vcx.update(|_, cx| view.read(cx).inputs.ask.clone());
    vcx.update(|_, cx| set(&input, "news", cx));
    click(vcx, "assistant-ask");
    vcx.update(|_, cx| {
        let view = view.read(cx);
        assert_eq!(view.notice, "Nixa could not complete the request.");
        assert_eq!(view.error.as_deref(), Some("Question is too long."));
    });
    click(vcx, "assistant-reminder-cancel-4");
    assert_eq!(log.lock().unwrap().last().unwrap(), &json!({"action": "remind_cancel", "id": "s2"}));
    vcx.update(|_, cx| {
        let view = view.read(cx);
        assert!(view.state.as_ref().unwrap().reminders.is_empty());
        assert_eq!(view.notice, "Cancelled reminder #4.");
    });
}
