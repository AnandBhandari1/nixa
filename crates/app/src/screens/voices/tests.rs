use super::tree::{shown_rows, voice_name};
use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};

fn state() -> Value {
    json!({
        "voices": ["Zoey", "Harper", "Narrator"],
        "normal": ["Zoey", "Harper"],
        "source": "speak",
        "sample": "Hello, this is how I sound.",
        "humor": "light",
        "map": {"default": "Harper", "agents": {"codex": "Zoey"}, "spaces": {"old": "Zoey"}, "terminals": {"m9": "Harper"}, "models": {"gpt-x": "Zoey"}},
        "kinds": [{"id": "chat", "label": "Nixa chat replies", "hint": "Nixa's own spoken answers"}, {"id": "finished", "label": "Agent finished", "hint": "done"}],
        "agents": ["claude", "codex"],
        "models": [{"id": "Opus", "label": "Opus (claude)"}, {"id": "gpt-x", "label": "gpt-x (codex)"}],
        "rows": [
            {"id": "default", "voice": "Harper", "rule": {"tier": "default", "level": 5, "text": "Level 5: default voice"}},
            {"id": "agent:codex", "voice": "Zoey", "rule": {"tier": "agent", "level": 2, "text": "Level 2: agent codex"}}
        ],
        "live": {"spaces": ["Shop", "shop", "~"], "terminals": [{"address": "m2", "agent": "claude", "space": "Shop", "label": "claude · Shop · Fix login"}]}
    })
}

fn loaded() -> VoicesView {
    let mut v = build(String::new());
    assert!(take(&mut v, &json!({"ok": true, "data": state()})));
    v
}

#[test]
fn rows_follow_the_levels_and_the_saved_rules() {
    let v = loaded();
    let ids = row_ids(&rows(&v));
    assert_eq!(
        ids,
        vec!["default", "kind:chat", "kind:finished", "space:shop", "space:old", "agent:claude", "agent:codex", "model:opus", "model:gpt-x", "terminal:m2", "terminal:m9"]
    );
    let list = rows(&v);
    let chat = list.iter().find(|r| r.id == "kind:chat").unwrap();
    assert_eq!(chat.ctx, json!({"agent": "nixa", "kind": "chat"}));
    let saved = list.iter().find(|r| r.id == "terminal:m9").unwrap();
    assert_eq!(saved.label, "m9 (not open)");
    let live = list.iter().find(|r| r.id == "terminal:m2").unwrap();
    assert_eq!(live.ctx, json!({"terminal": "m2", "agent": "claude", "space": "Shop", "kind": "finished"}));
    assert_eq!(build_rows(None).len(), 1);
    assert_eq!(row_specs(&list[..1]), json!([{"id": "default", "ctx": {}}]));
}

#[test]
fn own_rules_and_unused_models() {
    let v = loaded();
    let list = rows(&v);
    let map = &v.state.as_ref().unwrap().map;
    let find = |id: &str| list.iter().find(|r| r.id == id).unwrap().clone();
    assert_eq!(explicit_voice(map, &find("default")).as_deref(), Some("Harper"));
    assert_eq!(explicit_voice(map, &find("agent:codex")).as_deref(), Some("Zoey"));
    assert_eq!(explicit_voice(map, &find("agent:claude")), None);
    assert_eq!(explicit_voice(map, &find("terminal:m9")).as_deref(), Some("Harper"));
    let hidden: Vec<String> = shown_rows(&list, v.state.as_ref(), "agent", false).into_iter().map(|r| r.id).collect();
    assert_eq!(hidden, vec!["agent:claude", "agent:codex", "model:gpt-x"]);
    assert_eq!(shown_rows(&list, v.state.as_ref(), "agent", true).len(), 4);
    assert_eq!(voice_name("cycle"), "random voice");
    assert!(parse(&json!({"ok": true})).is_none());
}

fn open(cx: &mut TestAppContext) -> (Entity<VoicesView>, &mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let (view, cx) = cx.add_window_view(|_, _| loaded());
    cx.run_until_parked();
    (view, cx)
}

fn click(cx: &mut VisualTestContext, selector: &str) {
    cx.update(|window, app| window.draw(app).clear());
    let bounds = cx.debug_bounds(Box::leak(selector.to_string().into_boxed_str())).unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn shown(cx: &mut VisualTestContext, selector: &str) -> bool {
    cx.update(|window, app| window.draw(app).clear());
    cx.debug_bounds(Box::leak(selector.to_string().into_boxed_str())).is_some()
}

fn last(view: &Entity<VoicesView>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |v, _| v.last.clone().unwrap_or(Value::Null))
}

#[gpui::test]
fn pick_a_row_then_use_a_voice(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "voices-pick-agent:claude");
    view.read_with(cx, |v, _| {
        assert_eq!(v.target.id, "agent:claude");
        assert_eq!(v.target.label, "claude");
    });
    click(cx, "voices-use-Narrator");
    let sent = last(&view, cx);
    assert_eq!((sent["action"].as_str(), sent["level"].as_str(), sent["key"].as_str(), sent["voice"].as_str()), (Some("set"), Some("agent"), Some("claude"), Some("Narrator")));
    assert_eq!(sent["rows"].as_array().map(Vec::len), Some(11));
    click(cx, "voices-clear-agent:codex");
    let sent = last(&view, cx);
    assert_eq!((sent["action"].as_str(), sent["level"].as_str(), sent["key"].as_str()), (Some("clear"), Some("agent"), Some("codex")));
}

#[gpui::test]
fn humor_reset_and_models(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "voices-humor");
    assert_eq!(last(&view, cx)["value"], "off");
    click(cx, "voices-reset");
    view.read_with(cx, |v, _| assert!(v.confirm_reset));
    click(cx, "voices-reset-no");
    view.read_with(cx, |v, _| assert!(!v.confirm_reset));
    click(cx, "voices-reset");
    click(cx, "voices-reset-yes");
    assert_eq!(last(&view, cx)["action"], "reset");
    view.read_with(cx, |v, _| assert!(!v.confirm_reset));
    assert!(!shown(cx, "voices-row-model:opus"));
    assert!(shown(cx, "voices-row-model:gpt-x"));
    click(cx, "voices-models");
    view.read_with(cx, |v, _| assert!(v.show_models));
    assert!(shown(cx, "voices-row-model:opus"));
}

#[gpui::test]
fn play_toggles_and_the_idle_event_ends_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "voices-play-Harper");
    view.read_with(cx, |v, _| assert!(is_playing(v, "voice:Harper")));
    assert_eq!(last(&view, cx), json!({"action": "play", "voice": "Harper"}));
    click(cx, "voices-play-Harper");
    view.read_with(cx, |v, _| assert!(v.playing.is_none()));
    assert_eq!(last(&view, cx), json!({"action": "stop"}));
    click(cx, "voices-rowplay-agent:codex");
    assert_eq!(last(&view, cx), json!({"action": "play", "ctx": {"agent": "codex", "kind": "finished"}}));
    view.update(cx, |v, _| v.playing.as_mut().unwrap().id = Some(7));
    cx.update(|_, app| {
        assert!(apply(&view, &json!({"type": "voices_idle", "id": 7}), app));
        assert!(!apply(&view, &json!({"type": "voice", "id": 7}), app));
    });
    view.read_with(cx, |v, _| {
        assert!(v.playing.is_none());
        assert!(v.idled.contains(&7));
    });
    click(cx, "voices-play-Zoey");
    view.update(cx, stop_sample);
    view.read_with(cx, |v, _| assert!(v.playing.is_none()));
    assert_eq!(last(&view, cx), json!({"action": "stop"}));
}


#[test]
fn state_parses_the_voice_off_note() {
    let state = parse(&json!({"voices": ["Harper"], "voiceOn": false, "note": "Voice is off."})).unwrap();
    assert_eq!(state.voice_on, Some(false));
    assert_eq!(state.note, "Voice is off.");
}
