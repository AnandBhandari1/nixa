use super::model::{MAX_BODY, PANELS, audit_dir, browser_rows, card_tag, clip, desk_card_of, desk_rows, iso_time, loaded_of, talk_card_of};
use super::view::{empty_text, footer_text, shows_reads};
use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

type Sent = Arc<Mutex<Vec<(String, String, Option<Value>)>>>;

struct Fake {
    call: Call,
    sent: Sent,
    up: Arc<AtomicBool>,
    fail_post: Arc<AtomicBool>,
    state: Arc<Mutex<Value>>,
}

fn fake(desktop: Value) -> Fake {
    let sent: Sent = Arc::new(Mutex::new(Vec::new()));
    let up = Arc::new(AtomicBool::new(true));
    let fail_post = Arc::new(AtomicBool::new(false));
    let state = Arc::new(Mutex::new(desktop));
    let (log, on, fail, data) = (sent.clone(), up.clone(), fail_post.clone(), state.clone());
    let call: Call = Arc::new(move |method, path, body| {
        log.lock().unwrap().push((method.to_string(), path.to_string(), body.clone()));
        if !on.load(Ordering::SeqCst) {
            return Err("Nixa brain not connected".into());
        }
        match (method, path) {
            ("GET", DESKTOP_PATH) => Ok(data.lock().unwrap().clone()),
            ("GET", TALK_PATH) => Ok(json!({"ok": true, "talk": {"type": "talk", "confirm": null}})),
            ("GET", BROWSER_PATH) => Ok(json!({"ok": true, "browser": {"calls": [{"at": 0, "browser": "own", "action": "open", "tool": "browser_step", "verdict": "done", "approved": true, "ms": 40}]}})),
            ("POST", _) if fail.load(Ordering::SeqCst) => Err("no open card".into()),
            ("POST", _) => Ok(json!({"ok": true})),
            _ => Err("not found".into()),
        }
    });
    Fake { call, sent, up, fail_post, state }
}

fn recent() -> Value {
    json!([
        {"id": "a", "at": "2026-01-01T10:00:00Z", "phase": "held", "step": {"tool": "nixa_eyes", "action": "windows"}},
        {"id": "a", "at": "2026-01-01T10:00:01Z", "phase": "result", "step": {"tool": "nixa_eyes", "action": "windows"}, "result": "2 windows"},
        {"id": "c", "at": "2026-01-01T10:02:00Z", "phase": "cancelled", "step": {"tool": "nixa_hands", "action": "click"}},
        {"id": "d", "phase": "held", "step": {"tool": "other", "action": "click"}},
        {"id": "f", "at": "2026-01-01T10:03:00Z", "phase": "refused", "attempt": {"tool": "nixa_eyes", "what": "windows"}, "error": "Privacy is on"},
        {"id": "e", "phase": "held"},
        "junk"
    ])
}

fn held(id: &str) -> Value {
    json!({"id": id, "tool": "nixa_hands", "action": "click", "message": format!("Desktop step {id}\nnixa_hands: click\nAllow this step?"), "step": {"tool": "nixa_hands", "action": "click"}, "at": 1})
}

fn desktop(card: Value) -> Value {
    json!({"ok": true, "card": card, "recent": recent(), "allowReads": false})
}

fn browser_ask(id: u64) -> Value {
    browser_ask_at(id, 100)
}

fn browser_ask_at(id: u64, asked_at: u64) -> Value {
    json!({"id": id, "message": "Browser own: click\nSite: https://example.com\nElement: More\nAllow this one step?", "why": "a browser step", "asked_at": asked_at})
}

#[test]
fn panel_names_and_texts() {
    assert_eq!(panel_of("nixa-eyes"), Some(Panel::Eyes));
    assert_eq!(panel_of("nixa_hands"), Some(Panel::Hands));
    assert_eq!(panel_of(" Browser "), Some(Panel::Browser));
    assert_eq!(panel_of("desk"), None);
    for panel in PANELS {
        assert_eq!(panel_of(key(panel)), Some(panel));
        assert!(title(panel).starts_with("Nixa's "));
        assert!(!model::help(panel).is_empty());
    }
    assert_eq!(commands().len(), 3);
    assert_eq!(empty_text(Panel::Hands), "No hands steps yet.");
    assert_eq!(iso_time(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso_time(1_767_225_600_000 + 3_723_000), "2026-01-01T01:02:03Z");
    assert_eq!(iso_time(951_782_400_000), "2000-02-29T00:00:00Z");
}

#[test]
fn the_desktop_card_comes_from_the_desktop_event() {
    let card = desk_card_of(&held("a1b2-c3")).unwrap();
    assert_eq!(card.key, CardKey::Desk("a1b2-c3".into()));
    assert_eq!(card.note, "nixa_hands click");
    assert_eq!(event_of(&json!({"type": "desktop_card", "card": held("x1")})), Event::Desk(desk_card_of(&held("x1"))));
    assert_eq!(event_of(&json!({"type": "desktop_card", "card": null})), Event::Desk(None));
    assert_eq!(event_of(&json!({"type": "desktop_card", "card": {"id": "", "message": "m"}})), Event::Desk(None));
    assert_eq!(event_of(&json!({"type": "chat"})), Event::Other);
    assert_eq!(event_of(&json!({"type": "talk", "speaking": true})), Event::Other);
}

#[test]
fn only_a_browser_talk_card_with_a_number_id_is_shown() {
    let card = talk_card_of(&browser_ask(4)).unwrap();
    assert_eq!(card.key, CardKey::Talk(4));
    assert!(talk_card_of(&json!({"id": 5, "message": "Push the branch?"})).is_none());
    assert!(talk_card_of(&json!({"id": "5", "message": "Browser own: click"})).is_none());
    assert!(talk_card_of(&json!({"message": "Browser own: click"})).is_none());
    assert_eq!(event_of(&json!({"type": "talk", "confirm": null})), Event::Talk(None));
    assert_eq!(event_of(&json!({"type": "talk", "confirm": browser_ask(4)})), Event::Talk(Some(card)));
    assert_eq!(event_of(&json!({"type": "confirm_done", "id": 4})), Event::TalkDone(Some(4)));
}

#[test]
fn answers_go_to_the_right_route_with_the_right_id() {
    assert_eq!(answer_body(&CardKey::Desk("a1b2".into()), true), (ANSWER_PATH, json!({"id": "a1b2", "answer": "yes"})));
    assert_eq!(answer_body(&CardKey::Desk("a1b2".into()), false), (ANSWER_PATH, json!({"id": "a1b2", "answer": "no"})));
    assert_eq!(answer_body(&CardKey::Talk(9), true), (TALK_ANSWER_PATH, json!({"id": 9, "approve": true})));
    let card = talk_card_of(&json!({"id": 9, "message": "Browser own: open", "asked_at": 500})).unwrap();
    assert_eq!(card_tag(&card), "talk:9@500");
    assert_eq!(card_tag(&desk_card_of(&held("d1")).unwrap()), "desk:d1@1");
}

#[test]
fn rows_are_newest_first_and_split_by_panel() {
    let rows = desk_rows(&recent());
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].head, "windows · refused · 2026-01-01T10:03:00Z");
    assert_eq!(rows[1].panel, Panel::Hands);
    assert_eq!(rows[1].head, "click · cancelled · 2026-01-01T10:02:00Z");
    let eyes = for_panel(&rows, Panel::Eyes);
    assert_eq!(eyes.len(), 3);
    assert_eq!(eyes[1].head, "windows · result · 2026-01-01T10:00:01Z");
    assert!(eyes[1].body.contains("\"result\": \"2 windows\""));
    let old = desk_rows(&json!([{"phase": "result", "step": {"mod": "nixa-eyes", "action": "read"}}, {"step": {"mod": "nixa-browser", "action": "open"}}]));
    assert_eq!(old.len(), 1);
    let calls = browser_rows(&json!([{"at": 0, "action": "open", "verdict": "done"}, {"verdict": "x"}]));
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].head, "open · done · 1970-01-01T00:00:00Z");
    assert!(desk_rows(&json!({"rows": []})).is_empty());
    let long = "x".repeat(MAX_BODY + 10);
    assert_eq!(clip(&long, MAX_BODY).chars().count(), MAX_BODY + 2);
    assert_eq!(clip("short", MAX_BODY), "short");
    let loaded = loaded_of(&json!({"card": held("z"), "recent": recent(), "allowReads": true}), Some(&json!({"talk": {"confirm": browser_ask(3)}})), None);
    assert_eq!(loaded.rows.len(), 4);
    assert!(loaded.allow_reads);
    assert_eq!(loaded.desk.map(|c| c.key), Some(CardKey::Desk("z".into())));
    assert_eq!(loaded.talk.map(|c| c.key), Some(CardKey::Talk(3)));
}

fn open(cx: &mut TestAppContext, panel: Panel, call: Call) -> (Entity<DeskView>, &mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let (view, cx) = cx.add_window_view(move |_, cx| {
        let mut v = build(panel, call, PathBuf::from("/data"));
        refresh(&mut v, cx);
        v
    });
    cx.run_until_parked();
    (view, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, app| window.draw(app).clear());
}

fn click(cx: &mut VisualTestContext, selector: &str) {
    draw(cx);
    let bounds = cx.debug_bounds(Box::leak(selector.to_string().into_boxed_str())).unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn shown(cx: &mut VisualTestContext, selector: &str) -> bool {
    draw(cx);
    cx.debug_bounds(Box::leak(selector.to_string().into_boxed_str())).is_some()
}

fn posts(sent: &Sent) -> Vec<(String, Option<Value>)> {
    sent.lock().unwrap().iter().filter(|(m, _, _)| m == "POST").map(|(_, p, b)| (p.clone(), b.clone())).collect()
}

#[gpui::test]
fn the_hands_yes_answers_the_held_desktop_step(cx: &mut TestAppContext) {
    let f = fake(desktop(held("a1b2c3d4-e5f6")));
    let (view, cx) = open(cx, Panel::Hands, f.call.clone());
    view.read_with(cx, |v, _| {
        assert!(v.linked);
        assert_eq!(v.desk.as_ref().map(|c| c.key.clone()), Some(CardKey::Desk("a1b2c3d4-e5f6".into())));
        assert_eq!(v.rows.len(), 5);
    });
    click(cx, "desk-yes-desk");
    assert_eq!(posts(&f.sent), vec![(ANSWER_PATH.to_string(), Some(json!({"id": "a1b2c3d4-e5f6", "answer": "yes"})))]);
    view.read_with(cx, |v, _| assert!(v.desk.is_none()));
    view.update(cx, refresh);
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.desk.is_none(), "an answered card does not come back"));
    *f.state.lock().unwrap() = desktop(held("n2"));
    view.update(cx, refresh);
    cx.run_until_parked();
    click(cx, "desk-no-desk");
    assert_eq!(posts(&f.sent).last().cloned(), Some((ANSWER_PATH.to_string(), Some(json!({"id": "n2", "answer": "no"})))));
    assert!(!posts(&f.sent).iter().any(|(p, _)| p == TALK_ANSWER_PATH));
}

#[gpui::test]
fn the_browser_card_answers_its_own_talk_id(cx: &mut TestAppContext) {
    let f = fake(desktop(Value::Null));
    let (view, cx) = open(cx, Panel::Browser, f.call.clone());
    view.update(cx, |v, cx| {
        assert!(take(v, &json!({"type": "talk", "confirm": browser_ask(7)})));
        assert!(!take(v, &json!({"type": "confirm_done", "id": 8})));
        cx.notify();
    });
    assert!(shown(cx, "desk-card-browser"));
    click(cx, "desk-yes-browser");
    assert_eq!(posts(&f.sent), vec![(TALK_ANSWER_PATH.to_string(), Some(json!({"id": 7, "approve": true})))]);
    view.update(cx, |v, _| {
        assert!(v.talk.is_none());
        assert!(take(v, &json!({"type": "talk", "confirm": {"id": 8, "message": "Push the branch?"}})));
        assert!(v.talk.is_none());
        assert!(take(v, &json!({"type": "talk", "confirm": browser_ask(7)})));
        assert!(v.talk.is_none(), "an answered card does not come back");
    });
}

#[gpui::test]
fn a_failed_answer_keeps_the_card_and_says_why(cx: &mut TestAppContext) {
    let f = fake(desktop(held("k1")));
    f.fail_post.store(true, Ordering::SeqCst);
    let (view, cx) = open(cx, Panel::Eyes, f.call.clone());
    click(cx, "desk-yes-desk");
    view.read_with(cx, |v, _| {
        assert!(v.desk.is_some());
        assert!(v.error.as_deref().is_some_and(|e| e.starts_with("Not sent to Nixa")));
        assert!(!v.answering);
    });
    assert!(shown(cx, "desk-card-desk"));
}

#[gpui::test]
fn switch_buttons_change_the_panel_and_its_audit(cx: &mut TestAppContext) {
    let f = fake(desktop(Value::Null));
    let (view, cx) = open(cx, Panel::Eyes, f.call.clone());
    assert!(shown(cx, "desk-row-2"));
    assert!(!shown(cx, "desk-row-3"));
    click(cx, "desk-switch-nixa-browser");
    view.read_with(cx, |v, _| {
        assert_eq!(v.panel, Panel::Browser);
        assert_eq!(for_panel(&v.rows, v.panel).len(), 1);
    });
    click(cx, "desk-switch-nixa-hands");
    view.read_with(cx, |v, _| assert_eq!(for_panel(&v.rows, v.panel).len(), 1));
    click(cx, "desk-switch-nixa-eyes");
    view.read_with(cx, |v, _| {
        assert_eq!(v.panel, Panel::Eyes);
        assert!(footer_text(v).contains(&audit_dir(&v.data).display().to_string()));
        assert!(footer_text(v).contains("Reads without a card: off"));
        assert!(footer_text(v).contains("Inputs run by themselves"));
        assert!(!footer_text(v).contains("always keep their card"));
    });
}

#[gpui::test]
fn the_reads_switch_saves_through_the_brain(cx: &mut TestAppContext) {
    let f = fake(desktop(Value::Null));
    let (view, cx) = open(cx, Panel::Hands, f.call.clone());
    assert!(shown(cx, "desk-reads"));
    view.read_with(cx, |v, _| assert!(!footer_text(v).contains("permission.json")));
    click(cx, "desk-reads");
    assert_eq!(posts(&f.sent), vec![(READS_PATH.to_string(), Some(json!({"allowReads": true})))]);
    view.read_with(cx, |v, _| assert!(v.allow_reads));
    assert!(!reads_of(&json!({"ok": true, "allowReads": false}), true));
    f.fail_post.store(true, Ordering::SeqCst);
    click(cx, "desk-reads");
    view.read_with(cx, |v, _| {
        assert!(v.allow_reads, "a failed save keeps the old switch");
        assert!(v.error.as_deref().is_some_and(|e| e.starts_with("Not sent to Nixa")));
    });
    click(cx, "desk-switch-nixa-browser");
    view.read_with(cx, |v, _| assert!(!shows_reads(v), "the browser has no reads switch"));
}

#[gpui::test]
fn a_lost_brain_hides_the_cards(cx: &mut TestAppContext) {
    let f = fake(desktop(held("q1")));
    let (view, cx) = open(cx, Panel::Hands, f.call.clone());
    view.read_with(cx, |v, _| assert!(v.linked && v.desk.is_some()));
    f.up.store(false, Ordering::SeqCst);
    view.update(cx, refresh);
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(!v.linked && v.desk.is_none() && v.talk.is_none()));
    f.up.store(true, Ordering::SeqCst);
    view.update(cx, refresh);
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.linked && v.desk.is_some()));
}

#[gpui::test]
fn apply_reaches_the_open_window_only(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    cx.update(|app| assert!(!apply(&json!({"type": "desktop_card", "card": held("w1")}), app)));
    let f = fake(desktop(Value::Null));
    let (view, cx) = open(cx, Panel::Hands, f.call.clone());
    let weak = view.downgrade();
    cx.update(|window, app| app.set_global(DeskWindow { handle: window.window_handle(), view: weak }));
    cx.update(|_, app| {
        assert!(apply(&json!({"type": "desktop_card", "card": held("w1")}), app));
        assert!(!apply(&json!({"type": "chat"}), app));
    });
    view.read_with(cx, |v, _| assert_eq!(v.desk.as_ref().map(|c| c.key.clone()), Some(CardKey::Desk("w1".into()))));
    cx.update(|_, app| assert!(apply(&json!({"type": "desktop_card", "card": null}), app)));
    view.read_with(cx, |v, _| assert!(v.desk.is_none()));
}

#[gpui::test]
fn a_reused_talk_id_after_a_brain_restart_still_shows(cx: &mut TestAppContext) {
    let f = fake(desktop(Value::Null));
    let (view, cx) = open(cx, Panel::Browser, f.call.clone());
    view.update(cx, |v, cx| {
        take(v, &json!({"type": "talk", "confirm": browser_ask_at(1, 100)}));
        cx.notify();
    });
    click(cx, "desk-yes-browser");
    view.update(cx, |v, _| {
        assert!(v.talk.is_none());
        take(v, &json!({"type": "talk", "confirm": browser_ask_at(1, 100)}));
        assert!(v.talk.is_none(), "the same answered card stays hidden");
        take(v, &json!({"type": "talk", "confirm": browser_ask_at(1, 900)}));
        assert_eq!(v.talk.as_ref().map(|c| c.key.clone()), Some(CardKey::Talk(1)), "a new card with a reused id shows");
        v.talk = None;
    });
    f.up.store(false, Ordering::SeqCst);
    view.update(cx, refresh);
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.answered.is_empty(), "a lost brain forgets old answers"));
    f.up.store(true, Ordering::SeqCst);
    view.update(cx, |v, _| {
        take(v, &json!({"type": "talk", "confirm": browser_ask_at(1, 100)}));
        assert!(v.talk.is_some());
    });
}
