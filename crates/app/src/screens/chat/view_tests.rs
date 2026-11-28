use super::*;
use crate::screens::nixa_screen::NixaScreen;
use gpui::{TestAppContext, VisualTestContext};
use serde_json::json;

fn open(cx: &mut TestAppContext) -> (Entity<ChatView>, &mut VisualTestContext, Entity<NixaScreen>) {
    cx.update(nixa_ui::init);
    let screen = cx.update(|cx| {
        let conn = nixa_store::open_memory().ok();
        let screen = cx.new(|cx| NixaScreen::build(None, conn, cx));
        cx.set_global(crate::screens::nixa_screen::Shared(screen.clone()));
        screen
    });
    let (view, vcx) = cx.add_window_view(|_, cx| build(cx));
    vcx.update(|_, cx| {
        apply(
            &view,
            &json!({"type": "hello", "name": "Nixa", "busy": false, "items": [
                {"id": 1, "who": "user", "text": "hi", "at": 1, "typed": true}
            ]}),
            cx,
        );
        set_link(&view, model::Link::Up, cx);
    });
    vcx.run_until_parked();
    (view, vcx, screen)
}

fn drawn(vcx: &mut VisualTestContext, selector: &'static str) -> bool {
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear());
    vcx.debug_bounds(selector).is_some()
}

#[gpui::test]
fn the_panel_shows_only_the_chat(cx: &mut TestAppContext) {
    let (_, vcx, _) = open(cx);
    assert!(drawn(vcx, "chat-zone"));
    assert!(drawn(vcx, "chat-send"));
    assert!(!drawn(vcx, "nixa-sidebar-toggle"));
    assert!(!drawn(vcx, "nixa-tab-universe"));
}

#[gpui::test]
fn an_empty_chat_keeps_the_composer_full_width(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    vcx.update(|_, cx| apply(&view, &json!({"type": "hello", "name": "Nixa", "items": []}), cx));
    assert!(drawn(vcx, "chat-send"));
    let send = vcx.debug_bounds("chat-send").unwrap();
    let hide = vcx.debug_bounds("chat-hide").unwrap();
    assert!(f32::from(send.right()) + 40.0 > f32::from(hide.right()), "{send:?} {hide:?}");
}

#[gpui::test]
fn brain_event_lines_stay_out_of_the_chat(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    vcx.update(|_, cx| {
        apply(
            &view,
            &json!({"type": "chat", "item": {"id": 7, "who": "info", "text": "Event: Prompt p1: An agent asks. Answer with nixa_answer_prompt.", "at": 7}}),
            cx,
        );
        apply(
            &view,
            &json!({"type": "chat", "item": {"id": 8, "who": "info", "text": "The mic did not start.", "at": 8}}),
            cx,
        );
    });
    let ids: Vec<u64> = vcx.update(|_, cx| list::visible(view.read(cx)).iter().map(|i| i.id).collect());
    assert_eq!(ids, vec![1, 8]);
}

#[gpui::test]
fn chat_text_follows_the_terminal_font(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    let family = |vcx: &mut VisualTestContext| {
        vcx.update(|_, cx| list::chat_font(view.read(cx)).family.to_string())
    };
    assert_eq!(family(vcx), nixa_termview::DEFAULT_FONT);
    vcx.update(|_, cx| set_font_family(&view, "Fira Code".into(), cx));
    assert_eq!(family(vcx), "Fira Code");
    vcx.update(|_, cx| set_font_family(&view, "  ".into(), cx));
    assert_eq!(family(vcx), nixa_termview::DEFAULT_FONT);
    assert!(drawn(vcx, "chat-zone"));
}

#[gpui::test]
fn the_question_card_shows_in_the_tray(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    let question = vcx.update(|_, cx| {
        let q = crate::screens::question::new(cx);
        set_question(&view, q.clone(), cx);
        q
    });
    assert!(!drawn(vcx, "chat-tray"));
    vcx.update(|_, cx| {
        question.update(cx, |q, cx| {
            q.apply(
                &json!({"type": "prompts", "waiting": 2, "card": {
                    "id": "k1", "kind": "question", "agent": "codex", "terminal": "t1",
                    "title": "Pick one", "options": [{"label": "A"}]
                }}),
                cx,
            );
        })
    });
    assert!(drawn(vcx, "chat-tray"));
    assert!(drawn(vcx, "question-go"));
    let tray = vcx.debug_bounds("chat-tray").unwrap();
    let send = vcx.debug_bounds("chat-send").unwrap();
    assert!(tray.bottom() <= send.top());
}

