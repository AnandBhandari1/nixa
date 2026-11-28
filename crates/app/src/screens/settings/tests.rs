use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};

fn window<'a>(cx: &'a mut TestAppContext, part: &'static str) -> (Entity<SettingsView>, &'a mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("mss-{}-{part}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    cx.update(move |cx| {
        nixa_ui::init(cx);
        nixa_ui::theme::install_from(cx, dir);
        crate::actions::bind(cx);
    });
    cx.add_window_view(move |window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut view = SettingsView {
            part,
            kept: Kept::default(),
            focus,
            parts_scroll: ScrollHandle::new(),
            reveal_part: true,
            watched: Vec::new(),
            subs: Vec::new(),
        };
        prepare(&mut view, cx);
        view
    })
}

fn click(vcx: &mut VisualTestContext, id: &'static str) {
    let at = vcx.debug_bounds(id).unwrap_or_else(|| panic!("{id} is not on screen")).center();
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

fn open_windows(vcx: &mut VisualTestContext) -> usize {
    vcx.cx.update(|cx| cx.windows().len())
}

#[gpui::test]
fn a_part_click_shows_that_part(cx: &mut TestAppContext) {
    let (view, vcx) = window(cx, "Phone");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("settings-part-Rules").is_some());
    click(vcx, "settings-part-Look");
    assert_eq!(vcx.update(|_, cx| view.read(cx).part), "Look");
    assert!(vcx.update(|_, cx| view.read(cx).kept.look.is_some()));
    click(vcx, "settings-part-App");
    assert_eq!(vcx.update(|_, cx| view.read(cx).part), "App");
    assert!(vcx.update(|_, cx| view.read(cx).kept.app.is_some()));
    click(vcx, "settings-part-Look");
    assert!(vcx.update(|_, cx| view.read(cx).kept.look.is_some()));
}

#[gpui::test]
fn escape_closes_the_window(cx: &mut TestAppContext) {
    let (_view, vcx) = window(cx, "Look");
    vcx.run_until_parked();
    assert_eq!(open_windows(vcx), 1);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert_eq!(open_windows(vcx), 0);
}

#[gpui::test]
fn cmd_w_closes_the_window(cx: &mut TestAppContext) {
    let (_view, vcx) = window(cx, "App");
    vcx.run_until_parked();
    assert_eq!(open_windows(vcx), 1);
    vcx.simulate_keystrokes("cmd-w");
    vcx.run_until_parked();
    assert_eq!(open_windows(vcx), 0);
}

#[gpui::test]
fn schedules_start_no_brain_link_of_their_own(cx: &mut TestAppContext) {
    let (view, vcx) = window(cx, "Schedules");
    vcx.run_until_parked();
    let link = vcx.update(|_, cx| {
        view.read(cx)
            .kept
            .schedules
            .as_ref()
            .map(|part| part.read(cx).link.is_some())
    });
    assert_eq!(link, Some(false));
}

#[gpui::test]
fn repeats_opened_first_get_the_shared_brain_link(cx: &mut TestAppContext) {
    let link = crate::links::brain::start_at("127.0.0.1:9", crate::links::brain::Timing::default()).unwrap();
    let shared = link.clone();
    cx.update(move |cx| share_brain_link(shared, cx));
    let (view, vcx) = window(cx, repeats::PART);
    let has_link = vcx.update(|_, cx| {
        view.read(cx).kept.repeats.as_ref().map(|part| part.read(cx).link.is_some())
    });
    link.stop();
    assert_eq!(has_link, Some(true));
}

#[gpui::test]
fn opening_app_scrolls_the_selected_row_into_view(cx: &mut TestAppContext) {
    let (view, vcx) = window(cx, "App");
    vcx.simulate_resize(size(px(640.), px(420.)));
    view.update(vcx, |view, cx| {
        select(view, "App", cx);
        cx.notify();
    });
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        let scroll = &view.read(cx).parts_scroll;
        let row = scroll.bounds_for_item(part_row("App")).unwrap();
        let offset = scroll.offset().y;
        assert!(row.top() + offset >= scroll.bounds().top());
        assert!(row.bottom() + offset <= scroll.bounds().bottom());
    });
}

#[gpui::test]
fn leaving_voices_stops_the_sample_and_returning_restarts_refresh(cx: &mut TestAppContext) {
    use crate::screens::voices as screen;

    let (view, vcx) = window(cx, "General");
    vcx.update(|_, cx| {
        let voice = cx.new(|_| screen::build("127.0.0.1:0".into()));
        cx.set_global(screen::Shared(voice.clone()));
        view.update(cx, |view, cx| select(view, voices::PART, cx));
        let started = voice.read(cx).seq;
        assert!(started > 0);
        voice.update(cx, |voice, _| {
            voice.playing = Some(screen::Playing { key: "sample".into(), id: None, turn: 1 });
        });
        view.update(cx, |view, cx| select(view, "App", cx));
        assert_eq!(screen::shared(cx).unwrap().entity_id(), voice.entity_id());
        assert!(voice.read(cx).playing.is_none());
        let stopped = voice.read(cx).seq;
        assert!(stopped > started);
        view.update(cx, |view, cx| select(view, voices::PART, cx));
        assert_eq!(screen::shared(cx).unwrap().entity_id(), voice.entity_id());
        assert!(voice.read(cx).seq > stopped);
    });
}

#[gpui::test]
fn drones_part_opens_and_renders_snapshot(cx: &mut TestAppContext) {
    let (view, vcx) = window(cx, "General");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("settings-part-Drones").is_some());
    click(vcx, "settings-part-Drones");
    assert_eq!(vcx.update(|_, cx| view.read(cx).part), drones::PART);
    let drones_view = vcx
        .update(|_, cx| view.read(cx).kept.drones.clone())
        .expect("drones view kept");
    assert!(vcx.debug_bounds("drones-page").is_some());
    let msg = serde_json::json!({
        "type": "drones_changed",
        "drones": {
            "on": true,
            "every": 5,
            "remindAfter": 15,
            "subjects": 3,
            "findings": [],
            "steps": {
                "subject": {"kind": "switch", "label": "Task or terminal", "fact": "subject"}
            }
        }
    });
    vcx.update(|_, cx| drones::feed(&drones_view, &msg, cx));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("drones-master-on").is_some());
    vcx.update(|_, cx| {
        let data = drones_view.read(cx).data.as_ref().expect("drones data");
        assert!(data.on);
        assert_eq!(data.every, 5);
        assert_eq!(data.subjects, 3);
        assert_eq!(data.steps.len(), 1);
    });
}

#[test]
fn parts_are_known() {
    assert_eq!(known("Chat brain"), "Chat brain");
    assert_eq!(known("Drones"), "Drones");
    assert_eq!(known("nothing"), "General");
    assert_eq!(group_of("Git accounts"), Some("Work"));
    assert_eq!(group_of("Memory"), Some("Nixa"));
    assert_eq!(group_of("Rules"), Some("Work"));
    assert_eq!(known("Rule trees"), "Rules");
    assert_eq!(known("Answer rules"), "Rules");
    assert_eq!(group_of("Drones"), Some("Work"));
    assert_eq!(group_of("Look"), Some("App"));
    assert_eq!(group_of("Privacy"), Some("Nixa"));
    assert_eq!(group_of("Knowledge"), Some("Nixa"));
    assert_eq!(group_of("Web search"), Some("Work"));
    assert_eq!(group_of("Connectors"), Some("Work"));
    assert_eq!(all().count(), 22);
}

#[test]
fn parts_are_in_the_live_order() {
    let nixa: Vec<_> = GROUPS[0].1.to_vec();
    assert_eq!(
        nixa,
        ["General", "Chat brain", "Repeat messages", "Memory", "Knowledge", "Mic and speaker", "Voices", "Phone", "Privacy", "Nixa screen"]
    );
    let work: Vec<_> = GROUPS[1].1.to_vec();
    assert_eq!(
        work,
        [
            "Spaces",
            "Agents and team",
            "Workflows",
            "Rules",
            "Decision model",
            "Drones",
            "Schedules",
            "Git accounts",
            "Web search",
            "Connectors"
        ]
    );
    assert_eq!(GROUPS[2].1.to_vec(), ["Look", "App"]);
}

#[test]
fn only_later_parts_have_a_later_note() {
    for part in all() {
        assert!(later(part).is_none(), "{part}");
    }
}
