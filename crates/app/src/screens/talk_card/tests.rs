use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use serde_json::json;

fn talk(confirm: Value) -> Value {
    json!({"type": "talk", "presence": {"here": true}, "confirm": confirm, "speaking": false})
}

#[test]
fn spoken_confirms_are_read_from_the_talk_state() {
    let card = card_of(&talk(json!({"id": 4, "message": " Push the branch? ", "why": "checks passed", "askedAt": 1, "deadline": 2}))).unwrap().unwrap();
    assert_eq!(card, Card { id: 4, message: "Push the branch?".into(), why: "checks passed".into() });
    assert_eq!(card_of(&talk(Value::Null)), Some(None));
    assert_eq!(card_of(&talk(json!({"id": 5, "message": "  "}))), Some(None));
    assert_eq!(card_of(&json!({"type": "talk", "presence": {}})), None);
    assert_eq!(card_of(&json!({"type": "confirm", "confirm": {"id": 1, "message": "x"}})), None);
    assert_eq!(answer(4, true), json!({"type": "confirm", "approve": true, "id": 4}));
    assert_eq!(answer(4, false)["approve"], json!(false));
    assert_eq!(clip("abcdef", 4), "abc…");
    assert_eq!(clip("abc", 4), "abc");
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.run_until_parked();
    let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

struct Host;

impl gpui::Render for Host {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        let t = nixa_ui::theme(cx).clone();
        div().size_full().children(render(&t, "Nixa", cx))
    }
}

#[gpui::test]
fn yes_and_no_answer_the_card(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(feed(&talk(json!({"id": 7, "message": "Close terminal 3?", "why": ""})), cx));
        assert_eq!(current(cx).map(|c| c.id), Some(7));
        assert!(!feed(&json!({"type": "busy", "busy": true}), cx));
    });
    cx.update(nixa_ui::init);
    let (_host, vcx) = cx.add_window_view(|_, cx| {
        let h = hub(cx);
        cx.observe(&h, |_, _, cx| cx.notify()).detach();
        Host
    });
    vcx.run_until_parked();
    click(vcx, YES_ID);
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        assert_eq!(current(cx), None);
        assert_eq!(hub(cx).read(cx).sent.clone(), vec![answer(7, true)]);
        feed(&talk(json!({"id": 8, "message": "Delete the branch?", "why": "it is merged"})), cx);
    });
    vcx.run_until_parked();
    click(vcx, NO_ID);
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        assert_eq!(hub(cx).read(cx).sent.clone().last(), Some(&answer(8, false)));
        assert_eq!(current(cx), None);
        feed(&talk(json!({"id": 9, "message": "Send it?"})), cx);
        feed(&talk(Value::Null), cx);
        assert_eq!(current(cx), None);
    });
}
