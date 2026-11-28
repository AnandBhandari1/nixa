use super::*;
use gpui::{Modifiers, TestAppContext, div};
use serde_json::json;

const NOW: u64 = 1_000 * MINUTE_MS;

fn talk(presence: Value) -> Value {
    json!({"type": "talk", "presence": presence, "confirm": null})
}

#[test]
fn presence_reads_the_talk_state() {
    let p = presence_of(&talk(json!({"here": false, "away": true, "awayMinutes": 12, "held": 2, "lastSeen": NOW - 20 * MINUTE_MS}))).unwrap();
    assert!(p.known && p.away && !p.here);
    assert_eq!((p.away_minutes, p.held, p.quiet_until), (12, 2, None));
    assert_eq!(away_minutes(&p, NOW), 20);
    assert_eq!(chip_of(&p, NOW), Some(Chip::Away { minutes: 20 }));
    let quiet = presence_of(&talk(json!({"here": true, "away": true, "quietUntil": NOW + 30 * MINUTE_MS}))).unwrap();
    assert_eq!(chip_of(&quiet, NOW), Some(Chip::Quiet { until: NOW + 30 * MINUTE_MS }));
    assert_eq!(chip_of(&quiet, NOW + 31 * MINUTE_MS), Some(Chip::Away { minutes: 0 }));
    let here = presence_of(&talk(json!({"here": true}))).unwrap();
    assert_eq!(chip_of(&here, NOW), None);
    assert_eq!(presence_of(&json!({"type": "talk", "confirm": null})), None);
    assert_eq!(presence_of(&json!({"type": "voice", "presence": {}})), None);
    assert_eq!(end_quiet(), json!({"type": "set_quiet", "minutes": 0}));
    assert_eq!(ask_message(), json!({"type": "talk_state"}));
}

#[test]
fn chips_say_until_when() {
    let t = nixa_ui::theme::default_theme();
    let quiet = spec(&Chip::Quiet { until: 9 * 60 * MINUTE_MS + 5 * MINUTE_MS }, "Nixa", 0, &t);
    assert_eq!(quiet.label, "Quiet until 09:05");
    assert!(quiet.tip.ends_with("Click to let Nixa speak again."));
    assert_eq!(quiet.id.as_ref(), QUIET_ID);
    let away = spec(&Chip::Away { minutes: 7 }, "Nixa", 0, &t);
    assert_eq!(away.label, "Away");
    assert!(away.tip.starts_with("No sign of you for 7 minutes."));
}

struct Host;

impl gpui::Render for Host {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        let t = nixa_ui::theme(cx).clone();
        div().size_full().children(render_at(&t, "Nixa", NOW, cx))
    }
}

#[gpui::test]
fn the_quiet_chip_lets_nixa_speak_again(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    cx.update(|cx| assert!(feed(&talk(json!({"here": true, "quietUntil": NOW + 10 * MINUTE_MS})), cx)));
    let (_host, vcx) = cx.add_window_view(|_, cx| {
        let h = hub(cx);
        cx.observe(&h, |_, _, cx| cx.notify()).detach();
        Host
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds(AWAY_ID).is_none());
    let bounds = vcx.debug_bounds(QUIET_ID).expect("quiet chip is drawn");
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        assert_eq!(hub(cx).read(cx).sent.clone(), vec![end_quiet()]);
        feed(&talk(json!({"here": false, "away": true, "awayMinutes": 3})), cx);
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds(AWAY_ID).is_some());
    vcx.update(|_, cx| assert_eq!(chip_of(&current(cx), NOW), Some(Chip::Away { minutes: 3 })));
}
