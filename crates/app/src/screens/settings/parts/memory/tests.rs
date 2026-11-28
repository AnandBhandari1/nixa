use super::doc::{self, Ask, Busy, DocEditor, Document};
use super::text_area::TextArea;
use gpui::{Entity, TestAppContext, VisualTestContext};
use serde_json::json;

const NO_BRAIN: &str = "127.0.0.1:1";

fn open(cx: &mut TestAppContext, which: Document) -> (Entity<DocEditor>, &mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let (view, vcx) = cx.add_window_view(|_, cx| doc::build(which, NO_BRAIN.into(), false, cx));
    vcx.run_until_parked();
    (view, vcx)
}

fn text(view: &Entity<DocEditor>, vcx: &mut VisualTestContext) -> String {
    vcx.update(|_, cx| view.read(cx).area.read(cx).value().to_string())
}

fn type_in(view: &Entity<DocEditor>, vcx: &mut VisualTestContext, words: &str) {
    let area = vcx.update(|_, cx| view.read(cx).area.clone());
    vcx.update(|window, cx| window.focus(&area.read(cx).focus_handle(cx)));
    vcx.run_until_parked();
    vcx.simulate_input(words);
    vcx.run_until_parked();
}

use gpui::Focusable;

#[gpui::test]
fn a_read_fills_the_editor(cx: &mut TestAppContext) {
    let (view, vcx) = open(cx, Document::Memory);
    view.update_in(vcx, |v, _, cx| {
        doc::on_read(
            v,
            Ok(json!({"text": "- one\n- two", "hash": "h1", "held": ["two"]})),
            false,
            "",
            cx,
        )
    });
    assert_eq!(text(&view, vcx), "- one\n- two");
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.hash.as_deref(), Some("h1"));
        assert_eq!(v.held, vec!["two"]);
        assert_eq!(v.notice, "Loaded. Edit the text, then Save.");
        assert!(!doc::dirty(v, cx));
    });
}

#[gpui::test]
fn a_draft_survives_a_change_elsewhere(cx: &mut TestAppContext) {
    let (view, vcx) = open(cx, Document::Memory);
    view.update_in(vcx, |v, _, cx| {
        doc::on_read(v, Ok(json!({"text": "- one", "hash": "h1"})), false, "", cx)
    });
    type_in(&view, vcx, "x");
    assert!(vcx.update(|_, cx| doc::dirty(view.read(cx), cx)));
    let used = view.update_in(vcx, |v, _, cx| {
        doc::on_event(
            v,
            &json!({"type": "content_changed", "document": "memory", "hash": "h2"}),
            cx,
        )
    });
    assert!(used);
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert!(v.error);
        assert!(v.notice.starts_with("Changed elsewhere."));
    });
    let draft = text(&view, vcx);
    view.update_in(vcx, |v, _, cx| {
        doc::on_read(
            v,
            Ok(json!({"text": "- other", "hash": "h2"})),
            false,
            &draft,
            cx,
        )
    });
    assert_eq!(text(&view, vcx), draft);
    vcx.update(|_, cx| assert_eq!(view.read(cx).hash.as_deref(), Some("h1")));
    view.update_in(vcx, |v, _, cx| {
        doc::on_read(
            v,
            Ok(json!({"text": "- other", "hash": "h2"})),
            true,
            &draft,
            cx,
        )
    });
    assert_eq!(text(&view, vcx), "- other");
    vcx.update(|_, cx| assert_eq!(view.read(cx).hash.as_deref(), Some("h2")));
    let other = view.update_in(vcx, |v, _, cx| {
        doc::on_event(
            v,
            &json!({"type": "content_changed", "document": "rules", "hash": "z"}),
            cx,
        )
    });
    assert!(!other);
}

#[gpui::test]
fn the_same_hash_changes_nothing(cx: &mut TestAppContext) {
    let (view, vcx) = open(cx, Document::Rules);
    view.update_in(vcx, |v, _, cx| doc::on_read(v, Ok(json!({"text": "{}", "hash": "h1", "problem": "rules.json kept the last good rules: x", "file": "/d/rules.json"})), false, "", cx));
    view.update_in(vcx, |v, _, cx| {
        doc::on_event(
            v,
            &json!({"type": "content_changed", "document": "rules", "hash": "h1"}),
            cx,
        )
    });
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.busy, None);
        assert_eq!(
            v.problem.as_deref(),
            Some("rules.json kept the last good rules: x")
        );
        assert_eq!(v.file.as_deref(), Some("/d/rules.json"));
    });
}

#[gpui::test]
fn saving_empty_memory_asks_first(cx: &mut TestAppContext) {
    let (view, vcx) = open(cx, Document::Memory);
    view.update_in(vcx, |v, _, cx| {
        doc::on_read(v, Ok(json!({"text": "- one", "hash": "h1"})), false, "", cx)
    });
    let area = vcx.update(|_, cx| view.read(cx).area.clone());
    area.update_in(vcx, |a, _, cx| a.set_value("  ", cx));
    view.update_in(vcx, |v, _, cx| doc::press_save(v, cx));
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.ask, Some(Ask::Clear));
        assert_eq!(v.busy, None);
    });
    view.update_in(vcx, |v, _, cx| doc::save(v, cx));
    vcx.update(|_, cx| assert_eq!(view.read(cx).busy, Some(Busy::Saving)));
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.busy, None);
        assert!(v.error);
        assert!(v.notice.ends_with("Your draft is kept."), "{}", v.notice);
    });
    assert_eq!(text(&view, vcx), "  ");
}

#[gpui::test]
fn a_save_reply_keeps_newer_edits(cx: &mut TestAppContext) {
    let (view, vcx) = open(cx, Document::Memory);
    view.update_in(vcx, |v, _, cx| {
        doc::on_read(v, Ok(json!({"text": "- one", "hash": "h1"})), false, "", cx)
    });
    type_in(&view, vcx, "Z");
    let typed = text(&view, vcx);
    view.update_in(vcx, |v, _, cx| {
        doc::on_saved(
            v,
            Ok(json!({"text": "- one", "hash": "h2", "notesSaved": ["0 added, 0 removed."]})),
            "- one",
            cx,
        )
    });
    assert_eq!(text(&view, vcx), typed);
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.hash.as_deref(), Some("h2"));
        assert!(v.notice.starts_with("Saved the submitted text."));
        assert!(doc::dirty(v, cx));
    });
    view.update_in(vcx, |v, _, cx| {
        doc::on_saved(
            v,
            Ok(json!({"text": typed.clone(), "hash": "h3", "notesSaved": ["1 added, 0 removed."]})),
            &typed,
            cx,
        )
    });
    vcx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(
            v.notice,
            "Saved. Nixa reads the changes on its next brain turn. 1 added, 0 removed."
        );
        assert!(!doc::dirty(v, cx));
    });
    view.update_in(vcx, |v, _, cx| doc::on_saved(v, Err("Memory changed elsewhere. Your draft is kept. Reload the latest text before saving.".into()), "x", cx));
    vcx.update(|_, cx| {
        assert_eq!(
            view.read(cx).notice,
            "Memory changed elsewhere. Your draft is kept. Reload the latest text before saving."
        )
    });
}

#[gpui::test]
fn drafts_come_back_after_reopen(cx: &mut TestAppContext) {
    let (view, vcx) = open(cx, Document::Memory);
    view.update_in(vcx, |v, _, cx| {
        doc::on_read(v, Ok(json!({"text": "- one", "hash": "h1"})), false, "", cx)
    });
    type_in(&view, vcx, "Q");
    let typed = text(&view, vcx);
    let (again, vcx2) =
        cx.add_window_view(|_, cx| doc::build(Document::Memory, NO_BRAIN.into(), false, cx));
    vcx2.run_until_parked();
    let restored = vcx2.update(|_, cx| {
        (
            again.read(cx).area.read(cx).value().to_string(),
            again.read(cx).hash.clone(),
            again.read(cx).saved.clone(),
        )
    });
    assert_eq!(
        restored,
        (typed, Some("h1".to_string()), "- one".to_string())
    );
}

#[gpui::test]
fn the_text_area_takes_lines_and_arrows(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let (area, vcx) = cx.add_window_view(|_, cx| TextArea::new(cx, "type"));
    vcx.update(|window, cx| window.focus(&area.read(cx).focus_handle(cx)));
    vcx.run_until_parked();
    vcx.simulate_input("ab");
    vcx.simulate_keystrokes("enter");
    vcx.simulate_input("cd");
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| area.read(cx).value().to_string()),
        "ab\ncd"
    );
    vcx.simulate_keystrokes("up");
    vcx.run_until_parked();
    vcx.simulate_input("X");
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| area.read(cx).value().to_string()),
        "abX\ncd"
    );
    vcx.simulate_keystrokes("cmd-down backspace");
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| area.read(cx).value().to_string()),
        "abX\nc"
    );
    vcx.simulate_keystrokes("cmd-left");
    vcx.simulate_input("- ");
    vcx.simulate_keystrokes("shift-right cmd-x");
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| area.read(cx).value().to_string()),
        "abX\n- "
    );
    vcx.simulate_keystrokes("cmd-a backspace");
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| area.read(cx).value().to_string()), "");
}

#[gpui::test]
fn down_walks_every_wrapped_row(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let (area, vcx) = cx.add_window_view(|_, cx| TextArea::new(cx, "type"));
    vcx.update(|window, cx| window.focus(&area.read(cx).focus_handle(cx)));
    vcx.simulate_input(&"words that wrap onto more rows ".repeat(30));
    vcx.simulate_keystrokes("cmd-up");
    vcx.run_until_parked();
    let mut seen = vec![vcx.update(|_, cx| area.read(cx).cursor())];
    for _ in 0..3 {
        vcx.simulate_keystrokes("down");
        vcx.run_until_parked();
        seen.push(vcx.update(|_, cx| area.read(cx).cursor()));
    }
    assert!(seen.windows(2).all(|w| w[1] > w[0]), "{seen:?}");
}

#[gpui::test]
fn the_text_area_keeps_its_limit(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let (area, vcx) = cx.add_window_view(|_, cx| TextArea::new(cx, "type").limit(5));
    vcx.update(|window, cx| window.focus(&area.read(cx).focus_handle(cx)));
    vcx.simulate_input("abcdefgh");
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| area.read(cx).value().to_string()),
        "abcde"
    );
}
