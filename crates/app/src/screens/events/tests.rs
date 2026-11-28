use super::model::{Events, Source, ago, item_of, offer_of, row_body, row_meta, row_text};
use super::view::ENDED;
use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Duration;

fn agent(terminal: &str, kind: &str, time: u64) -> Item {
    item_of(&json!({"time": time, "kind": kind, "text": format!("Agent {terminal} x"), "terminal": terminal, "space": "shop", "job": "Fix login"})).unwrap()
}

fn offer_json(terminal: &str) -> serde_json::Value {
    json!({"id": "r1", "terminal": terminal, "title": "Resume agent 2: Fix login?", "body": "shop: it stopped. Click to resume it in a new tab.", "command": "claude --resume abc-1", "space": "s1", "folder": "/tmp/shop", "name": "Fix login", "timeout_ms": 20000})
}

#[test]
fn agent_events_merge_and_keep_fifty() {
    let mut list = Events::default();
    assert!(list.add_agent(agent("m1", "done", 1000)));
    assert!(list.add_agent(agent("m1", "done", 2000)));
    assert_eq!(list.items.len(), 1);
    assert!(list.add_agent(agent("m1", "blocked", 2500)));
    for n in 0..70 {
        list.add_agent(agent(&format!("m{}", n + 10), "done", 10_000 + n));
    }
    assert_eq!(list.items.len(), model::MAX_EVENTS);
    assert_eq!(list.items[0].terminal, "m79");
}

#[test]
fn a_toast_and_an_agent_event_become_one_row() {
    let mut list = Events::default();
    list.add_agent(agent("m2", "resume", 5000));
    let offer = offer_of(&offer_json("m2")).unwrap();
    let item = list.post(&resume_notice(offer.clone()), 6000).unwrap();
    assert_eq!(list.items.len(), 1);
    assert_eq!(item.text, "Agent m2 x");
    assert_eq!(item.body, "shop: it stopped. Click to resume it in a new tab. Runs: claude --resume abc-1");
    assert_eq!(item.source, Source::Notification);
    assert_eq!(item.action, Some(Action::Resume(offer)));
    assert!(!list.add_agent(agent("m2", "resume", 7000)));
    assert_eq!(list.items.len(), 1);
}

#[test]
fn toast_kinds_follow_the_level_and_finished_words() {
    let mut list = Events::default();
    assert!(list.post(&Notice::new(Level::Info, " "), 1).is_none());
    let e = list.post(&Notice::new(Level::Error, "Build failed"), 1).unwrap();
    assert_eq!(e.kind, "blocked");
    let mut n = Notice::new(Level::Info, "Agent 3 finished");
    n.terminal = "m3".into();
    assert_eq!(list.post(&n, 2).unwrap().kind, "done");
    let e = list.post(&Notice::new(Level::Warning, "").body("Only a body"), 3).unwrap();
    assert_eq!((e.kind.as_str(), e.text.as_str(), e.title.as_str()), ("working", "Only a body", "Notice"));
    assert_eq!(list.post(&Notice::new(Level::Success, "Saved"), 4).unwrap().kind, "done");
}

#[test]
fn clear_and_undo_keep_new_events() {
    let mut list = Events::default();
    assert_eq!(list.clear(), None);
    list.add_agent(agent("m1", "done", 1000));
    list.add_agent(agent("m2", "done", 2000));
    let token = list.clear().unwrap();
    assert!(list.items.is_empty());
    list.add_agent(agent("m3", "blocked", 3000));
    assert!(list.undo_clear());
    let order: Vec<&str> = list.items.iter().map(|e| e.terminal.as_str()).collect();
    assert_eq!(order, vec!["m3", "m2", "m1"]);
    assert!(!list.undo_clear());
    assert!(!list.expire_undo(token));
    let token = list.clear().unwrap();
    assert!(list.expire_undo(token));
    assert!(!list.undo_clear());
}

#[test]
fn row_words() {
    let mut numbers = HashMap::new();
    let item = agent("m4", "blocked", 0);
    assert_eq!(row_text(&item, &numbers), "Fix login needs attention");
    numbers.insert("m4".to_string(), 2);
    assert_eq!(row_text(&item, &numbers), "Agent #2 needs attention: Fix login");
    assert_eq!(row_meta(&item, &numbers), "shop · Agent #2");
    let mut toast = Events::default();
    let mut n = Notice::new(Level::Info, "Agent 2 finished").body("tests pass / 3 files");
    n.terminal = "m4".into();
    let shown = toast.post(&n, 0).unwrap();
    assert_eq!(row_body(&shown), "tests pass");
    let plain = toast.post(&Notice::new(Level::Info, "Saved").body("a / b"), 0).unwrap();
    assert_eq!(row_body(&plain), "a / b");
    assert_eq!(row_text(&plain, &numbers), "Saved");
    assert_eq!(row_meta(&plain, &numbers), "");
}

#[test]
fn resume_toasts_show_the_command_and_skipped_checks() {
    let mut raw = offer_json("m2");
    raw["command"] = json!("claude --resume abc-1 --dangerously-skip-permissions");
    let notice = resume_notice(offer_of(&raw).unwrap());
    assert_eq!(notice.title, "Resume agent 2: Fix login? (permission checks off)");
    assert!(notice.body.ends_with("Runs: claude --resume abc-1 --dangerously-skip-permissions"));
    raw["title"] = json!("Resume agent 2: Fix login? (permission checks off)");
    raw["body"] = json!("shop: it stopped. Click to resume it in a new tab: claude --resume abc-1 --dangerously-skip-permissions");
    let notice = resume_notice(offer_of(&raw).unwrap());
    assert_eq!(notice.title, "Resume agent 2: Fix login? (permission checks off)");
    assert_eq!(notice.body, "shop: it stopped. Click to resume it in a new tab: claude --resume abc-1 --dangerously-skip-permissions");
    let plain = resume_notice(offer_of(&offer_json("m2")).unwrap());
    assert_eq!(plain.title, "Resume agent 2: Fix login?");
}

#[test]
fn ago_words() {
    assert_eq!(ago(0, 44_000), "just now");
    assert_eq!(ago(0, 5 * 60_000), "5m ago");
    assert_eq!(ago(0, 3 * 3_600_000), "3h ago");
    assert_eq!(ago(0, 50 * 3_600_000), "2d ago");
    assert_eq!(ago(10_000, 0), "just now");
}

#[test]
fn only_safe_resume_commands_run() {
    assert_eq!(
        resume_argv("claude --resume abc-1").unwrap(),
        vec!["sh", "-lc", nixa_store::agents::resume_command("claude", "abc-1").unwrap().as_str()]
    );
    for ok in ["codex resume 019a --dangerously-bypass-approvals-and-sandbox", "cursor-agent --resume c1 --yolo", "agyd --conversation x.y", "omp --resume=s/1 --auto-approve", "opencode --session ses_1 --auto", "claude --resume a --dangerously-skip-permissions"] {
        assert!(resume_argv(ok).is_some(), "{ok}");
    }
    for bad in ["claude --resume abc && rm -rf ~", "claude --resume $(id)", "claude --resume", "claude --resume a --yolo --yolo", "omp --resume=", "omp --resume=a;b", "vim --resume a", "claude  --resume a", "claude --resume a --other"] {
        assert!(resume_argv(bad).is_none(), "{bad}");
    }
    assert!(offer_of(&json!({"terminal": "m1", "command": "rm -rf /", "id": "x", "title": "t", "body": "b"})).is_none());
}

type Seen = Rc<RefCell<Vec<EventsEvent>>>;

fn open(cx: &mut TestAppContext, items: Vec<Item>) -> (Entity<EventsView>, &mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut v = EventsView::empty(cx);
        v.clock = Some(10_000);
        v
    });
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|_, app| {
        app.subscribe(&view, move |_, event: &EventsEvent, _| sink.borrow_mut().push(event.clone())).detach();
    });
    view.update(cx, |v, cx| {
        v.load(items, cx);
        v.show(cx);
    });
    cx.run_until_parked();
    (view, cx, seen)
}

fn shown(cx: &mut VisualTestContext, selector: &str) -> bool {
    cx.update(|window, app| window.draw(app).clear());
    cx.debug_bounds(Box::leak(selector.to_string().into_boxed_str())).is_some()
}

fn click(cx: &mut VisualTestContext, selector: &str) {
    cx.update(|window, app| window.draw(app).clear());
    let bounds = cx.debug_bounds(Box::leak(selector.to_string().into_boxed_str())).unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn clear_then_undo_within_fifteen_seconds(cx: &mut TestAppContext) {
    let (view, cx, _) = open(cx, vec![agent("m1", "done", 9000), agent("m2", "blocked", 8000)]);
    assert!(shown(cx, "event-row-1"));
    assert!(!shown(cx, "events-undo"));
    click(cx, "events-clear");
    view.read_with(cx, |v, _| assert!(v.events.items.is_empty()));
    assert!(shown(cx, "events-empty"));
    click(cx, "events-undo");
    view.read_with(cx, |v, _| assert_eq!(v.events.items.len(), 2));
    click(cx, "events-clear");
    cx.executor().advance_clock(Duration::from_millis(model::UNDO_MS + 100));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.events.undo.is_none());
        assert!(v.events.items.is_empty());
    });
}

#[gpui::test]
fn row_click_goes_to_a_live_agent_only(cx: &mut TestAppContext) {
    let (view, cx, seen) = open(cx, vec![agent("m1", "done", 9000), agent("m2", "blocked", 8000)]);
    view.update(cx, |v, _| v.set_live(BTreeSet::from(["m2".to_string()])));
    click(cx, "event-row-0");
    assert!(seen.borrow().is_empty());
    view.read_with(cx, |v, _| {
        assert_eq!(v.notice.as_deref(), Some(ENDED));
        assert!(v.open);
    });
    assert!(shown(cx, "events-notice"));
    click(cx, "event-row-1");
    assert_eq!(*seen.borrow(), vec![EventsEvent::Focus { terminal: "m2".into() }, EventsEvent::Close]);
    view.read_with(cx, |v, _| assert!(!v.open));
}

#[gpui::test]
fn resume_row_and_close_button(cx: &mut TestAppContext) {
    let mut item = agent("m5", "resume", 9000);
    let offer = offer_of(&offer_json("m5")).unwrap();
    item.action = Some(Action::Resume(offer.clone()));
    let (view, cx, seen) = open(cx, vec![item]);
    click(cx, "event-row-0");
    assert_eq!(*seen.borrow(), vec![EventsEvent::Resume(offer), EventsEvent::Close]);
    view.update(cx, |v, cx| v.show(cx));
    seen.borrow_mut().clear();
    click(cx, "events-close");
    assert_eq!(*seen.borrow(), vec![EventsEvent::Close]);
    view.update(cx, |v, cx| v.run(EventsCommand::AgentVoiceOff, cx));
    assert_eq!(seen.borrow().last(), Some(&EventsEvent::SetAgentVoice(false)));
}

#[gpui::test]
fn opening_marks_events_seen(cx: &mut TestAppContext) {
    let (view, cx, _) = open(cx, vec![agent("m1", "done", 9000)]);
    view.update(cx, |v, cx| {
        assert_eq!(v.unseen(), 0);
        v.add_agent(agent("m2", "done", 11_000), cx);
        assert_eq!(v.unseen(), 1);
        v.close(cx);
        v.clock = Some(12_000);
        v.show(cx);
        assert_eq!(v.unseen(), 0);
    });
}

#[gpui::test]
fn brain_messages_fill_the_list_and_the_toasts(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let screens = cx.update(new);
    let (toast_view, vcx) = cx.add_window_view(|_, _| ToastsView::default());
    let screens = Screens { events: screens.events, toasts: toast_view.clone() };
    let acts: Rc<RefCell<Vec<ToastEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = acts.clone();
    vcx.update(|_, app| {
        app.subscribe(&toast_view, move |_, e: &ToastEvent, _| sink.borrow_mut().push(e.clone())).detach();
    });
    vcx.update(|_, app| {
        assert!(apply(&screens, &json!({"type": "agent_event", "event": {"time": model::now_ms(), "kind": "resume", "text": "Agent m5 can be resumed", "terminal": "m5"}}), app));
        assert!(apply(&screens, &json!({"type": "resume_offer", "offer": offer_json("m5")}), app));
        assert!(apply(&screens, &json!({"type": "notice", "text": "Deep brain asks", "loud": true}), app));
        assert!(!apply(&screens, &json!({"type": "notice", "text": "quiet", "loud": false}), app));
        assert!(apply(&screens, &json!({"type": "announce", "on": false, "events": [{"time": 0, "kind": "done", "text": "old", "terminal": "m1"}]}), app));
        assert!(!apply(&screens, &json!({"type": "resume_offer", "offer": {"terminal": "m6", "command": "sh -c x", "id": "1", "title": "t", "body": "b"}}), app));
    });
    vcx.run_until_parked();
    screens.events.read_with(vcx, |v, _| {
        assert_eq!(v.agent_voice, Some(false));
        let kinds: Vec<&str> = v.events.items.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds.len(), 3, "{kinds:?}");
        assert!(v.events.items.iter().any(|e| e.terminal == "m5" && matches!(e.action, Some(Action::Resume(_)))));
    });
    toast_view.read_with(vcx, |v, _| assert_eq!(v.items.len(), 2));
    let first = toast_view.read_with(vcx, |v, _| v.items[0].id);
    vcx.update(|window, app| window.draw(app).clear());
    click(vcx, &format!("toast-body-{first}"));
    assert!(matches!(acts.borrow().first(), Some(ToastEvent::Act(Action::Resume(_)))));
    toast_view.read_with(vcx, |v, _| assert_eq!(v.items.len(), 1));
    vcx.executor().advance_clock(Duration::from_millis(model::TOAST_MS + 100));
    vcx.run_until_parked();
    toast_view.read_with(vcx, |v, _| assert!(v.items.is_empty()));
}

#[test]
fn deep_answers_become_short_lines() {
    let answered = deep::deep_notice(&json!({"id": "d1", "question": "Who builds the login page?", "state": "answered", "pick": "a2", "why": "it is free", "spaceName": "shop"})).unwrap();
    assert_eq!(answered.title, "The deep brain of shop answered: Who builds the login page?");
    assert_eq!(answered.body, "Pick: a2. Why: it is free");
    assert_eq!(answered.level, Level::Info);
    let late = deep::deep_notice(&json!({"id": "d2", "question": "Next step?", "state": "timeout"})).unwrap();
    assert_eq!(late.title, "The deep brain did not answer: Next step?");
    assert_eq!(late.level, Level::Warning);
    assert!(deep::deep_notice(&json!({"id": "d3", "question": "q", "state": "sent"})).is_none());
    assert!(deep::deep_notice(&json!({"id": "d4", "question": " ", "state": "answered"})).is_none());
    assert_eq!(deep::clip(&"a".repeat(10), 5), "aaaa…");
}

#[gpui::test]
fn a_deep_answer_adds_a_row_and_no_toast(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let screens = cx.update(new);
    cx.update(|app| {
        assert!(apply(&screens, &json!({"type": "deep", "ask": {"id": "d1", "question": "Who builds it?", "state": "answered", "pick": "a2", "why": "free"}}), app));
        assert!(!apply(&screens, &json!({"type": "deep", "ask": {"id": "d2", "question": "x", "state": "waiting"}}), app));
        assert!(!apply(&screens, &json!({"type": "deep"}), app));
    });
    screens.events.read_with(cx, |v, _| {
        assert_eq!(v.events.items.len(), 1);
        assert_eq!(v.events.items[0].title, "The deep brain answered: Who builds it?");
        assert_eq!(v.events.items[0].body, "Pick: a2. Why: free");
    });
    screens.toasts.read_with(cx, |v, _| assert!(v.items.is_empty()));
}

#[gpui::test]
fn toast_close_does_not_run_its_action(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let (toast_view, vcx) = cx.add_window_view(|_, _| ToastsView::default());
    let acts: Rc<RefCell<Vec<ToastEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = acts.clone();
    vcx.update(|_, app| {
        app.subscribe(&toast_view, move |_, e: &ToastEvent, _| sink.borrow_mut().push(e.clone())).detach();
    });
    let offer = offer_of(&offer_json("m7")).unwrap();
    let id = toast_view.update(vcx, |v, cx| v.push(&resume_notice(offer), cx)).unwrap();
    vcx.run_until_parked();
    click(vcx, &format!("toast-close-{id}"));
    assert!(acts.borrow().is_empty());
    toast_view.read_with(vcx, |v, _| assert!(v.items.is_empty()));
    let id = toast_view.update(vcx, |v, cx| v.push(&resume_notice(offer_of(&offer_json("m7")).unwrap()), cx)).unwrap();
    vcx.executor().advance_clock(Duration::from_millis(19_000));
    vcx.run_until_parked();
    toast_view.read_with(vcx, |v, _| assert_eq!(v.items.len(), 1));
    vcx.executor().advance_clock(Duration::from_millis(1_500));
    vcx.run_until_parked();
    toast_view.read_with(vcx, |v, _| assert!(v.items.iter().all(|t| t.id != id)));
}

#[gpui::test]
fn an_agent_error_adds_a_row_and_a_toast(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let screens = new(cx);
        let event = json!({"type": "agent_event", "event": {
            "time": model::now_ms(), "kind": "error", "text": "Failed",
            "terminal": "m4", "job": "Build"
        }});
        assert!(apply(&screens, &event, cx));
        assert_eq!(screens.events.read(cx).events.items[0].kind, "error");
        assert_eq!(screens.toasts.read(cx).items[0].title, "Build reported an error");
    });
}
