use super::*;
use gpui::{Context, IntoElement, Modifiers, Render, TestAppContext, VisualTestContext};
use std::cell::RefCell;

struct Host {
    state: RailState,
    seen: Rc<RefCell<Vec<RailAction>>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = nixa_ui::theme(cx).clone();
        let seen = self.seen.clone();
        let on: OnRail = Rc::new(move |a, _, _| seen.borrow_mut().push(a));
        div()
            .size_full()
            .flex()
            .child(div().flex_1())
            .child(render(&self.state, &t, on))
    }
}

fn open(
    cx: &mut TestAppContext,
    state: RailState,
) -> (&mut VisualTestContext, Rc<RefCell<Vec<RailAction>>>) {
    cx.update(nixa_ui::init);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let (_, cx) = cx.add_window_view(move |_, _| Host { state, seen: sink });
    (cx, seen)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, app| window.draw(app).clear());
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    redraw(cx);
    let b = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(b.center(), Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn every_button_sends_its_action(cx: &mut TestAppContext) {
    let (cx, seen) = open(cx, state(Some(PanelKind::Files), 0, Some("/r".into())));
    for id in [
        "nixa-rail-commands",
        "nixa-rail-tasks",
        "nixa-rail-panel-files",
        "nixa-rail-panel-changes",
        "nixa-rail-repeats",
        "nixa-rail-deep",
        "nixa-rail-agents",
        "nixa-rail-workflows",
        "nixa-rail-automate",
        "nixa-rail-rules",
        "nixa-rail-memory",
        "nixa-rail-settings",
    ] {
        click(cx, id);
    }
    assert_eq!(
        *seen.borrow(),
        vec![
            RailAction::Commands,
            RailAction::Tasks,
            RailAction::Panel(PanelKind::Files),
            RailAction::Panel(PanelKind::Git),
            RailAction::Repeats,
            RailAction::DeepBrain,
            RailAction::Agents,
            RailAction::Workflows,
            RailAction::Automate,
            RailAction::Rules,
            RailAction::Memory,
            RailAction::Settings,
        ]
    );
    redraw(cx);
    assert!(cx.debug_bounds("nixa-rail-tasks-badge").is_none());
}

#[gpui::test]
fn the_tasks_badge_shows_the_count(cx: &mut TestAppContext) {
    let (cx, _) = open(cx, state(None, 3, None));
    redraw(cx);
    let badge = cx.debug_bounds("nixa-rail-tasks-badge").expect("badge");
    let button = cx.debug_bounds("nixa-rail-tasks").expect("tasks");
    assert!(button.contains(&badge.center()));
}

#[test]
fn tips_and_panels() {
    assert_eq!(
        tasks_tip(0),
        "Tasks. Opens in its own window. Drag it to another screen, or make it full screen."
    );
    assert!(tasks_tip(2).starts_with("Tasks: 2 waiting for you."));
    assert!(commands_tip(None).contains("select a project"));
    assert!(commands_tip(Some("/r")).contains("/r"));
    let mut s = state(None, 0, None);
    s.panels.push(PanelKind::Nixa);
    assert_eq!(visible_panels(&s), vec![PanelKind::Files, PanelKind::Git]);
    assert_eq!(command_menu_argv()[0], "sh");
    assert!(COMMAND_MENU.contains("cmdmenu --root"));
    assert_eq!(commands().len(), 12);
    assert_eq!(RULES_PART, crate::screens::settings::RULES);
    assert_eq!(SCHEDULES_PART, crate::screens::settings::parts::schedules::PART);
    assert_eq!(MEMORY_PART, crate::screens::settings::MEMORY);
}

#[test]
fn waiting_tasks_skip_locked_spaces() {
    let conn = nixa_store::open_memory().unwrap();
    assert_eq!(tasks_waiting(&conn), 0);
    let by = nixa_store::tasks::Actor::new(nixa_store::tasks::AuthorKind::User, "sam");
    for space in ["open", "shut", ""] {
        let input = nixa_store::tasks::TaskInput {
            title: format!("t {space}"),
            space_id: space.into(),
            ..Default::default()
        };
        let task = nixa_store::tasks::add_task(&conn, &input, &by).unwrap();
        nixa_store::tasks::set_status(
            &conn,
            task.id,
            nixa_store::tasks::TaskStatus::Blocked,
            Some(nixa_store::tasks::BlockedOn::User),
            true,
        )
        .unwrap();
    }
    assert_eq!(tasks_waiting(&conn), 3);
    let lock = nixa_store::spaces::SpacePatch {
        locked: Some(true),
        ..Default::default()
    };
    nixa_store::spaces::set_space(&conn, "shut", &lock).unwrap();
    assert_eq!(tasks_waiting(&conn), 2);
}

#[test]
fn the_deep_brain_button_asks_once_per_space() {
    assert_eq!(super::host::deep_body("shop"), serde_json::json!({"space": "shop", "ensure": true}));
    assert_eq!(super::host::deep_note(&serde_json::json!({"ok": true, "data": {"space": "shop", "started": true}})), "Starting the deep brain of space shop.");
    assert!(super::host::deep_note(&serde_json::json!({"data": {"space": "shop", "started": false}})).contains("already up"));
    assert!(super::deep_tip(None).contains("open a space"));
}
