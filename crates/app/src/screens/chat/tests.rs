use super::model::Link;
use super::wire::Command;
use super::*;
use gpui::{
    Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, point,
    size,
};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<ChatEvent>>>;

fn open(cx: &mut TestAppContext) -> (Entity<ChatView>, &mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let (view, vcx) = cx.add_window_view(|_, cx| build(cx));
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &ChatEvent, _| {
            log.borrow_mut().push(e.clone())
        })
        .detach();
    });
    vcx.run_until_parked();
    (view, vcx, seen)
}

fn feed(view: &Entity<ChatView>, vcx: &mut VisualTestContext, msg: serde_json::Value) {
    vcx.update(|_, cx| apply(view, &msg, cx));
    vcx.run_until_parked();
}

fn hello(view: &Entity<ChatView>, vcx: &mut VisualTestContext) {
    feed(
        view,
        vcx,
        json!({"type": "hello", "name": "Nixa", "busy": false, "items": [
            {"id": 1, "who": "user", "text": "hi", "at": 1, "typed": true},
            {"id": 2, "who": "nixa", "text": "hello", "at": 2},
            {"id": 3, "who": "info", "text": "a note", "at": 3}
        ]}),
    );
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.run_until_parked();
    let bounds = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn drawn(vcx: &mut VisualTestContext, selector: &'static str) -> bool {
    vcx.run_until_parked();
    vcx.debug_bounds(selector).is_some()
}

fn events(seen: &Seen) -> Vec<ChatEvent> {
    std::mem::take(&mut *seen.borrow_mut())
}

#[gpui::test]
fn typing_and_enter_sends(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    hello(&view, vcx);
    vcx.update(|window, cx| window.focus(&view.read(cx).input.focus_handle(cx)));
    vcx.simulate_input("what is claude doing");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        events(&seen),
        vec![ChatEvent::Send(Command::Typed(
            "what is claude doing".into()
        ))]
    );
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).input.read(cx).value().to_string()),
        ""
    );
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(events(&seen).is_empty());
}

#[gpui::test]
fn the_composer_wraps_long_text_and_shift_enter_starts_a_line(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    hello(&view, vcx);
    vcx.update(|window, cx| window.focus(&view.read(cx).input.focus_handle(cx)));
    vcx.run_until_parked();
    let empty = vcx.debug_bounds("chat-type").unwrap();
    let long = "a long sentence that keeps going ".repeat(12);
    vcx.simulate_input("first line");
    vcx.simulate_keystrokes("shift-enter");
    vcx.simulate_input(&long);
    vcx.run_until_parked();
    assert!(events(&seen).is_empty());
    let grown = vcx.debug_bounds("chat-type").unwrap();
    let most = px(19. * f32::from(COMPOSER_ROWS) + 12.);
    assert!(grown.size.height > empty.size.height * 2., "{grown:?} {empty:?}");
    assert!(grown.size.height <= most, "{grown:?}");
    assert!(grown.size.width == empty.size.width);
    let send = vcx.debug_bounds("chat-send").unwrap();
    assert!((send.bottom() - grown.bottom()).abs() <= px(1.), "{send:?} {grown:?}");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        events(&seen),
        vec![ChatEvent::Send(Command::Typed(format!("first line\n{}", long.trim())))]
    );
    assert_eq!(vcx.update(|_, cx| view.read(cx).input.read(cx).value().to_string()), "");
    vcx.run_until_parked();
    let after = vcx.debug_bounds("chat-type").unwrap();
    assert_eq!(after.size.height, empty.size.height);
    vcx.simulate_input(&long.repeat(4));
    vcx.run_until_parked();
    let full = vcx.debug_bounds("chat-type").unwrap();
    assert!((full.size.height - most).abs() <= px(2.), "{full:?}");
}

#[gpui::test]
fn the_send_button_sends(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    hello(&view, vcx);
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.input.update(cx, |i, cx| i.set_value("  ok  ", cx))
        })
    });
    click(vcx, "chat-send");
    assert_eq!(
        events(&seen),
        vec![ChatEvent::Send(Command::Typed("ok".into()))]
    );
}

#[gpui::test]
fn clean_asks_first(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    hello(&view, vcx);
    assert!(!drawn(vcx, "chat-clean-yes"));
    click(vcx, "chat-clean");
    assert!(events(&seen).is_empty());
    assert!(vcx.update(|_, cx| view.read(cx).clean_ask));
    click(vcx, "chat-clean-no");
    assert!(!vcx.update(|_, cx| view.read(cx).clean_ask));
    click(vcx, "chat-clean");
    click(vcx, "chat-clean-yes");
    assert_eq!(events(&seen), vec![ChatEvent::Send(Command::Clean)]);
    assert!(!vcx.update(|_, cx| view.read(cx).clean_ask));
}

#[gpui::test]
fn copy_puts_the_message_on_the_clipboard(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    hello(&view, vcx);
    click(vcx, "chat-copy-2");
    let text = vcx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(text.as_deref(), Some("hello"));
}

#[gpui::test]
fn long_multiline_messages_grow_and_rewrap_with_the_panel(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    let long = format!(
        "First line with enough words to wrap in a narrow panel.\n\nSecond paragraph.\n{}\nFinal line.",
        "x".repeat(180)
    );
    let paragraphs = concat!(
        "Project status\n\n",
        "The release checklist is ready. Start with the build, review the test results, ",
        "and confirm that the sample workspace opens as expected.\n\n",
        "Next, check the notes and file links. Record any follow-up in the task board, ",
        "including the steps for reproducing the issue.\n\n",
        "Final sentence is visible.",
    );
    feed(
        &view,
        vcx,
        json!({"type": "hello", "items": [
            {"id": 5, "who": "nixa", "text": paragraphs},
            {"id": 6, "who": "user", "text": long},
            {"id": 7, "who": "info", "text": long}
        ]}),
    );

    let measure = |vcx: &mut VisualTestContext, width| {
        vcx.simulate_resize(size(px(width), px(1400.)));
        vcx.update(|_, cx| set_width(&view, width, cx));
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear());
        let zone = vcx.debug_bounds("chat-zone").unwrap();
        ["chat-message-5", "chat-message-6", "chat-message-7"].map(|selector| {
            let body = vcx.debug_bounds(selector).unwrap();
            assert!(body.left() >= zone.left() && body.right() <= zone.right());
            assert!(body.size.width > px(width * 0.65), "{width}: {body:?}");
            body.size
        })
    };
    let narrow = measure(vcx, 280.);
    let wide = measure(vcx, 650.);
    for (narrow, wide) in narrow.into_iter().zip(wide) {
        assert!(narrow.width < wide.width, "{narrow:?} vs {wide:?}");
        assert!(narrow.height > wide.height, "{narrow:?} vs {wide:?}");
        assert!(wide.height > px(4. * 18.), "{wide:?}");
    }
    assert!(!drawn(vcx, "chat-expand-5"));
    click(vcx, "chat-copy-5");
    assert_eq!(
        vcx.read_from_clipboard().and_then(|item| item.text()),
        Some(paragraphs.to_string())
    );
}

#[gpui::test]
fn skipped_lines_hide_until_asked(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    hello(&view, vcx);
    feed(
        &view,
        vcx,
        json!({"type": "chat", "item": {"id": 7, "who": "user", "text": "mm", "at": 7, "quiet": true}}),
    );
    assert!(!drawn(vcx, "chat-copy-7"));
    let shows = |vcx: &mut VisualTestContext| {
        vcx.update(|_, cx| list::visible(view.read(cx)).iter().any(|i| i.id == 7))
    };
    assert!(!shows(vcx));
    click(vcx, "chat-skipped");
    assert!(drawn(vcx, "chat-copy-7"));
    assert!(shows(vcx));
    click(vcx, "chat-skipped");
    assert!(!shows(vcx));
}

#[gpui::test]
fn chips_send_their_commands(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    hello(&view, vcx);
    feed(
        &view,
        vcx,
        json!({"type": "state", "listen": "hotkey", "mic_ready": true, "mic_off": false}),
    );
    click(vcx, "chat-model");
    assert!(!drawn(vcx, "chat-listen"));
    assert!(!drawn(vcx, "chat-mic"));
    click(vcx, "chat-voice-place");
    click(vcx, "chat-voice-place-menu-1");
    assert_eq!(
        events(&seen),
        vec![
            ChatEvent::OpenSettings("Chat brain"),
            ChatEvent::Send(Command::PhoneVoice(true)),
        ]
    );
}

#[gpui::test]
fn phone_chips_open_the_part_and_move_the_voice(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    hello(&view, vcx);
    assert!(drawn(vcx, "chat-phone"));
    assert!(drawn(vcx, "chat-voice-place"));
    assert!(drawn(vcx, "chat-voice"));
    vcx.update(|_, cx| {
        crate::screens::phone_chip::feed(
            &json!({"type": "phone", "phone": {"on": true, "phones": [{"id": "p1", "name": "Pixel", "connected": true}]}}),
            cx,
        )
    });
    click(vcx, "chat-phone");
    click(vcx, "chat-voice-place");
    click(vcx, "chat-voice-place-menu-1");
    assert_eq!(
        events(&seen),
        vec![
            ChatEvent::OpenSettings("Phone"),
            ChatEvent::Send(Command::PhoneVoice(true)),
        ]
    );
}

#[gpui::test]
fn voice_button_turns_voice_on_and_off_and_the_place_is_a_menu(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    hello(&view, vcx);
    feed(
        &view,
        vcx,
        json!({"type": "voice", "voice": {"settings": {"micOn": true, "voiceOn": false, "listen": "always"}, "live": {"listening": true}}}),
    );
    assert!(vcx.update(|_, cx| view.read(cx).snap.voice_off));
    click(vcx, "chat-voice");
    assert_eq!(events(&seen), vec![ChatEvent::Send(Command::Voice(true))]);
    feed(
        &view,
        vcx,
        json!({"type": "voice", "voice": {"settings": {"micOn": true, "voiceOn": true, "listen": "always"}, "live": {"listening": true}}}),
    );
    click(vcx, "chat-voice");
    assert_eq!(events(&seen), vec![ChatEvent::Send(Command::Voice(false))]);
    click(vcx, "chat-voice-place");
    assert!(drawn(vcx, "chat-voice-place-menu-0"));
    click(vcx, "chat-voice-place-menu-1");
    assert!(vcx.update(|_, cx| view.read(cx).menu.is_none()));
    assert_eq!(events(&seen), vec![ChatEvent::Send(Command::PhoneVoice(true))]);
    vcx.update(|_, cx| {
        crate::screens::phone_chip::feed(&json!({"type": "phone", "mobile": true}), cx)
    });
    click(vcx, "chat-voice-place");
    click(vcx, "chat-voice-place-menu-0");
    assert_eq!(events(&seen), vec![ChatEvent::Send(Command::PhoneVoice(false))]);
}

#[gpui::test]
fn stop_shows_only_while_busy(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    hello(&view, vcx);
    assert!(!drawn(vcx, "chat-stop"));
    feed(&view, vcx, json!({"type": "busy", "busy": true}));
    feed(
        &view,
        vcx,
        json!({"type": "delta", "text": "Thinking about it"}),
    );
    click(vcx, "chat-stop");
    assert_eq!(events(&seen), vec![ChatEvent::Send(Command::Stop)]);
    feed(&view, vcx, json!({"type": "busy", "busy": false}));
    assert!(!vcx.update(|_, cx| format::busy(&view.read(cx).snap)));
}

#[gpui::test]
fn the_header_has_no_settings_page_chips(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    hello(&view, vcx);
    for id in ["chat-automate", "chat-rules", "chat-memory", "chat-standing-instructions"] {
        assert!(!drawn(vcx, id), "{id}");
    }
}

#[gpui::test]
fn scrolling_up_shows_jump_to_newest(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    hello(&view, vcx);
    for id in 10..90u64 {
        feed(
            &view,
            vcx,
            json!({"type": "chat", "item": {"id": id, "who": "nixa", "text": format!("line {id}"), "at": id}}),
        );
    }
    assert!(!drawn(vcx, "chat-jump"));
    let zone = vcx.debug_bounds("chat-zone").unwrap();
    vcx.simulate_event(ScrollWheelEvent {
        position: zone.center(),
        delta: ScrollDelta::Pixels(point(gpui::px(0.), gpui::px(600.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| view.read(cx).user_moved));
    feed(
        &view,
        vcx,
        json!({"type": "chat", "item": {"id": 95, "who": "nixa", "text": "new", "at": 95}}),
    );
    assert!(drawn(vcx, "chat-jump"));
    click(vcx, "chat-jump");
    assert!(!vcx.update(|_, cx| view.read(cx).user_moved));
}

#[gpui::test]
fn starting_shows_no_tools(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    assert!(!drawn(vcx, "chat-model"));
    assert!(!drawn(vcx, "chat-send"));
    vcx.update(|_, cx| set_link(&view, Link::Off, cx));
    assert!(!drawn(vcx, "chat-model"));
    hello(&view, vcx);
    assert!(drawn(vcx, "chat-model"));
    assert!(drawn(vcx, "chat-send"));
    assert!(!drawn(vcx, "chat-work-mode"));
    assert!(!drawn(vcx, "chat-decision"));
    feed(&view, vcx, json!({"type": "state", "state": "off"}));
    assert!(vcx.update(|_, cx| view.read(cx).snap.off()));
}

#[gpui::test]
fn confirm_box_answers(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    hello(&view, vcx);
    feed(
        &view,
        vcx,
        json!({"type": "confirm", "id": "c9", "text": "Push the branch?", "why": "it was asked"}),
    );
    click(vcx, "chat-confirm-yes");
    assert_eq!(
        events(&seen),
        vec![ChatEvent::Send(Command::Confirm {
            id: "c9".into(),
            approve: true
        })]
    );
}

#[test]
fn commands_cover_the_card() {
    let list = commands();
    assert_eq!(list.len(), 6);
    assert!(list.iter().any(|(_, a)| *a == Action::ChatBrain));
}
