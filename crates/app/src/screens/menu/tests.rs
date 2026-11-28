use super::switches::{self, SwitchEvent, SwitchesView};
use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use serde_json::json;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<MenuEvent>>>;

fn open_menu(
    cx: &mut TestAppContext,
    tab: Tab,
) -> (Entity<MenuView>, &mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let (view, vcx) = cx.add_window_view(|_, cx| build(tab, cx));
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    vcx.update(|window, cx| {
        cx.subscribe(&view, move |_, e: &MenuEvent, _| {
            log.borrow_mut().push(e.clone())
        })
        .detach();
        window.focus(&view.focus_handle(cx));
    });
    vcx.run_until_parked();
    (view, vcx, seen)
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.run_until_parked();
    let bounds = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn events(seen: &Seen) -> Vec<MenuEvent> {
    std::mem::take(&mut *seen.borrow_mut())
}

fn labels(view: &Entity<MenuView>, vcx: &mut VisualTestContext) -> Vec<String> {
    vcx.update(|_, cx| rows(view.read(cx)).into_iter().map(|i| i.label).collect())
}

#[gpui::test]
fn the_menu_opens_on_quick_actions_with_its_rows(cx: &mut TestAppContext) {
    let (view, vcx, _) = open_menu(cx, Tab::Quick);
    let list = labels(&view, vcx);
    assert_eq!(list[0], "Tasks");
    assert_eq!(list.len(), 16);
    assert!(vcx.debug_bounds("nixa-menu").is_some());
    let menu = vcx.debug_bounds("nixa-menu").unwrap();
    assert_eq!(
        (f32::from(menu.size.width), f32::from(menu.size.height)),
        (WIDTH, HEIGHT)
    );
    for i in 0..5 {
        assert!(
            vcx.debug_bounds(
                [
                    "menu-tab-0",
                    "menu-tab-1",
                    "menu-tab-2",
                    "menu-tab-3",
                    "menu-tab-4"
                ][i]
            )
            .is_some()
        );
    }
}

#[gpui::test]
fn clicking_a_tab_shows_its_rows(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open_menu(cx, Tab::Quick);
    click(vcx, "menu-tab-1");
    assert_eq!(vcx.update(|_, cx| view.read(cx).nav.tab), Tab::Create);
    assert_eq!(labels(&view, vcx)[0], "New tab");
    click(vcx, "menu-tab-2");
    assert_eq!(
        labels(&view, vcx),
        ["Zoom pane", "Use my layout", "Layouts…", "Toggle sidebar"]
    );
    assert!(events(&seen).is_empty());
    click(vcx, "menu-tab-4");
    assert_eq!(events(&seen), vec![MenuEvent::OpenSettings("General")]);
}

#[gpui::test]
fn clicking_a_row_runs_its_action(cx: &mut TestAppContext) {
    let (_, vcx, seen) = open_menu(cx, Tab::Layout);
    click(vcx, "menu-row-0");
    assert_eq!(events(&seen), vec![MenuEvent::Run(Action::ZoomPane)]);
    click(vcx, "menu-close");
    assert_eq!(events(&seen), vec![MenuEvent::Close]);
}

#[gpui::test]
fn keys_move_switch_tabs_run_and_close(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open_menu(cx, Tab::Quick);
    vcx.simulate_keystrokes("down down");
    assert_eq!(vcx.update(|_, cx| view.read(cx).nav.at), 2);
    vcx.simulate_keystrokes("up");
    assert_eq!(vcx.update(|_, cx| view.read(cx).nav.at), 1);
    vcx.simulate_keystrokes("enter");
    assert_eq!(events(&seen), vec![MenuEvent::Run(Action::QuickCapture)]);
    vcx.simulate_keystrokes("tab");
    assert_eq!(
        vcx.update(|_, cx| (view.read(cx).nav.tab, view.read(cx).nav.at)),
        (Tab::Create, 0)
    );
    vcx.simulate_keystrokes("right right");
    assert_eq!(vcx.update(|_, cx| view.read(cx).nav.tab), Tab::Workspace);
    vcx.simulate_keystrokes("shift-tab");
    assert_eq!(vcx.update(|_, cx| view.read(cx).nav.tab), Tab::Layout);
    vcx.simulate_keystrokes("up enter");
    assert_eq!(events(&seen), vec![MenuEvent::Run(Action::ToggleSidebar)]);
    vcx.simulate_keystrokes("right right");
    assert_eq!(events(&seen), vec![MenuEvent::OpenSettings("General")]);
    vcx.simulate_keystrokes("escape");
    assert_eq!(events(&seen), vec![MenuEvent::Close]);
}

#[gpui::test]
fn projects_open_in_the_menu_and_escape_goes_back(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open_menu(cx, Tab::Quick);
    let at = vcx.update(|_, cx| {
        rows(view.read(cx))
            .iter()
            .position(|i| i.action == Action::Projects)
            .unwrap()
    });
    vcx.update(|_, cx| view.update(cx, |v, cx| choose(v, at, cx)));
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).nav.view),
        nav::View::Projects
    );
    vcx.update(|_, cx| {
        view.update(cx, |v, _| {
            v.projects = Some(vec![projects::Project {
                name: "nixa".into(),
                path: "/p/nixa".into(),
            }])
        })
    });
    assert_eq!(labels(&view, vcx), ["← Back", "Choose folder…", "nixa"]);
    click(vcx, "menu-row-2");
    assert_eq!(
        events(&seen),
        vec![MenuEvent::Run(Action::OpenProject {
            name: "nixa".into(),
            path: "/p/nixa".into()
        })]
    );
    vcx.simulate_keystrokes("escape");
    assert_eq!(vcx.update(|_, cx| view.read(cx).nav.view), nav::View::Menu);
    assert!(events(&seen).is_empty());
    vcx.simulate_keystrokes("up up up up");
    assert_eq!(vcx.update(|_, cx| view.read(cx).nav.at), 12);
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).nav.view),
        nav::View::Projects
    );
    click(vcx, "menu-row-0");
    assert_eq!(vcx.update(|_, cx| view.read(cx).nav.view), nav::View::Menu);
}

#[gpui::test]
fn a_closed_menu_draws_nothing(cx: &mut TestAppContext) {
    let (view, vcx, _) = open_menu(cx, Tab::Quick);
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.open = false;
            cx.notify();
        })
    });
    vcx.run_until_parked();
    let shown = vcx.update(|_, cx| view.read(cx).open);
    assert!(!shown);
    assert_eq!(vcx.update(|_, cx| rows(view.read(cx)).len()), 16);
}

#[test]
fn commands_and_keys_point_at_menu_parts() {
    assert_eq!(
        commands()[0],
        ("Quick actions menu", Command::Open(Tab::Quick))
    );
    assert!(
        commands()
            .iter()
            .any(|(_, c)| *c == Command::Run(Action::ProjectCommands))
    );
    let found: Vec<Option<Command>> = ["", "tasks", "projectcommands", "favourites"]
        .iter()
        .map(|id| key_command(id))
        .collect();
    assert_eq!(
        found,
        vec![
            Some(Command::Open(Tab::Quick)),
            Some(Command::Run(Action::Tasks)),
            Some(Command::Run(Action::ProjectCommands)),
            Some(Command::Run(Action::Favourites)),
        ]
    );
}

type SwitchSeen = Rc<RefCell<Vec<SwitchEvent>>>;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mmt-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn open_switches(
    cx: &mut TestAppContext,
    data: PathBuf,
) -> (Entity<SwitchesView>, &mut VisualTestContext, SwitchSeen) {
    cx.update(nixa_ui::init);
    let store = nixa_store::open_memory().unwrap();
    let (view, vcx) = cx.add_window_view(|_, _| switches::build(data, Some(store)));
    let seen: SwitchSeen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &SwitchEvent, _| {
            log.borrow_mut().push(e.clone())
        })
        .detach();
    });
    vcx.run_until_parked();
    (view, vcx, seen)
}

#[gpui::test]
fn general_switches_flip_and_save(cx: &mut TestAppContext) {
    let data = temp("flip");
    let (view, vcx, seen) = open_switches(cx, data.clone());
    click(vcx, "switch-nixa");
    assert!(data.join(switches::OFF_FILE).exists());
    assert!(seen.borrow().is_empty());
    assert!(
        vcx.update(|_, cx| view.read(cx).error.clone())
            .unwrap()
            .starts_with("Saved here.")
    );
    assert_eq!(vcx.update(|_, cx| view.read(cx).state.nixa), Some(false));
    click(vcx, "switch-nixa");
    assert!(!data.join(switches::OFF_FILE).exists());
    click(vcx, "switch-phones");
    assert!(data.join(switches::HEADPHONES_FILE).exists());
    click(vcx, "switch-agent-voice");
    click(vcx, "switch-git-voice");
    let state = vcx.update(|_, cx| view.read(cx).state.clone());
    assert_eq!(
        (state.agent_voice, state.git_voice),
        (Some(false), Some(false))
    );
    let saved = vcx.update(|_, cx| {
        let v = view.read(cx);
        switches::load(&v.data, v.store.as_ref())
    });
    assert_eq!(saved, state);
    click(vcx, "switch-nixa-screen");
    assert_eq!(
        std::mem::take(&mut *seen.borrow_mut()),
        vec![SwitchEvent::NixaScreen]
    );
    vcx.update(|_, cx| switches::set_linked(&view, true, cx));
    vcx.run_until_parked();
    click(vcx, "switch-agent-voice");
    click(vcx, "switch-nixa");
    assert_eq!(
        std::mem::take(&mut *seen.borrow_mut()),
        vec![
            SwitchEvent::Send(json!({"type": "set_agent_voice", "on": true})),
            SwitchEvent::Send(json!({"type": "set_nixa", "on": false})),
        ]
    );
    vcx.update(|_, cx| switches::feed(&view, &json!({"type": "agent_voice", "on": false}), cx));
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).state.agent_voice),
        Some(false)
    );
    std::fs::remove_dir_all(&data).ok();
}

#[gpui::test]
fn privacy_needs_the_brain(cx: &mut TestAppContext) {
    let data = temp("brain");
    let (view, vcx, seen) = open_switches(cx, data.clone());
    assert!(vcx.debug_bounds("switch-privacy").is_none());
    vcx.update(|_, cx| {
        switches::feed(&view, &json!({"type": "hello", "name": "Nixa"}), cx);
        switches::feed(&view, &json!({"type": "prompts", "waiting": 2}), cx);
    });
    vcx.run_until_parked();
    click(vcx, "switch-privacy");
    assert!(seen.borrow().is_empty());
    vcx.update(|_, cx| {
        switches::feed(&view, &json!({"type": "state", "privacy": true}), cx);
    });
    vcx.run_until_parked();
    click(vcx, "switch-privacy");
    assert_eq!(
        std::mem::take(&mut *seen.borrow_mut()),
        vec![
            SwitchEvent::Send(json!({"type": "set_privacy", "on": false})),
        ]
    );
    assert_eq!(vcx.update(|_, cx| view.read(cx).snap.waiting), 2);
    assert_eq!(vcx.update(|_, cx| view.read(cx).name.clone()), "Nixa");
    assert!(vcx.debug_bounds("switch-work-build").is_none());
    click(vcx, "switch-card");
    assert_eq!(
        std::mem::take(&mut *seen.borrow_mut()),
        vec![SwitchEvent::ShowCard]
    );
    vcx.update(|_, cx| {
        switches::set_error(
            &view,
            Some("The brain does not know the message set_privacy.".into()),
            cx,
        )
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("switches-error").is_some());
    vcx.update(|_, cx| switches::set_linked(&view, false, cx));
    vcx.run_until_parked();
    assert!(!vcx.update(|_, cx| view.read(cx).snap.linked));
    std::fs::remove_dir_all(&data).ok();
}
