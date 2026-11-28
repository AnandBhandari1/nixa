use super::{MenuEvent, NewMenu, Setup, build, customs_of, height, load, profile_text};
use crate::programs;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use serde_json::json;
use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<MenuEvent>>>;

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("nixa-menu-{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(dir.join("bin")).unwrap();
    dir
}

fn fake(dir: &std::path::Path, name: &str) {
    let bin = dir.join("bin").join(name);
    fs::write(&bin, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    dir: &std::path::Path,
) -> (Entity<NewMenu>, &'a mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let setup = Setup {
        file: programs::path_in(dir),
        dirs: vec![dir.join("bin")],
        link: None,
        space: Some("s1".into()),
        on_event: Rc::new(move |event, _| sink.borrow_mut().push(event)),
    };
    let (view, cx) = cx.add_window_view(move |window, cx| build(setup, window, cx));
    cx.update(|window, app| {
        let handle = view.read(app).focus.clone();
        window.focus(&handle);
        load(&view, app);
    });
    cx.run_until_parked();
    (view, cx, seen)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, app| window.draw(app).clear());
}

fn shown(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    redraw(cx);
    cx.debug_bounds(selector).is_some()
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    redraw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn type_in(view: &Entity<NewMenu>, cx: &mut VisualTestContext, name: &str, line: &str) {
    view.update(cx, |menu, cx| {
        menu.name_input.update(cx, |i, cx| i.set_value(name, cx));
        menu.cmd_input.update(cx, |i, cx| i.set_value(line, cx));
    });
}

#[gpui::test]
fn installed_agents_get_numbers_and_start(cx: &mut TestAppContext) {
    let dir = scratch("start");
    fake(&dir, "codex");
    fake(&dir, "omp");
    let (view, cx, seen) = open(cx, &dir);
    view.read_with(cx, |menu, _| {
        let keys: Vec<&str> = super::installed(menu)
            .iter()
            .map(|a| a.program.key.as_str())
            .collect();
        assert_eq!(keys, vec!["codex", "codex-continue", "omp"]);
        assert_eq!(super::choices(menu).len(), 5);
        assert_eq!(
            menu.team_error.as_deref(),
            Some(crate::links::brain::NOT_CONNECTED)
        );
    });
    assert!(shown(cx, "go-codex"));
    assert!(shown(cx, "team-error"));
    cx.simulate_keystrokes("3");
    cx.run_until_parked();
    assert_eq!(
        seen.borrow().last(),
        Some(&MenuEvent::Start {
            name: "Oh My Pi".into(),
            run: vec!["omp".into(), "--auto-approve".into()]
        })
    );
    click(cx, "go-codex");
    assert_eq!(
        seen.borrow().last(),
        Some(&MenuEvent::Start {
            name: "Codex".into(),
            run: vec!["codex".into(), "--dangerously-bypass-approvals-and-sandbox".into()]
        })
    );
    let file = view.read_with(cx, |menu, _| menu.file.clone());
    std::fs::write(file, r#"{"checksOff":["codex"],"checksOn":["codex"]}"#).unwrap();
    click(cx, "go-codex");
    assert_eq!(
        seen.borrow().last(),
        Some(&MenuEvent::Start {
            name: "Codex".into(),
            run: vec!["codex".into()]
        })
    );
    click(cx, "terminal");
    assert_eq!(seen.borrow().last(), Some(&MenuEvent::Terminal));
    click(cx, "settings");
    assert_eq!(seen.borrow().last(), Some(&MenuEvent::Settings("General")));
    assert!(shown(cx, "leaderboard"));
    let event_count = seen.borrow().len();
    cx.simulate_keystrokes("9");
    cx.run_until_parked();
    assert_eq!(seen.borrow().len(), event_count);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(seen.borrow().last(), Some(&MenuEvent::Close));
    click(cx, "new-menu-close");
    assert_eq!(seen.borrow().len(), event_count + 2);
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn add_edit_and_remove_an_agent(cx: &mut TestAppContext) {
    let dir = scratch("add");
    fake(&dir, "mine");
    let (view, cx, seen) = open(cx, &dir);
    click(cx, "add-agent");
    assert!(view.read_with(cx, |m, _| m.form.is_some()));
    click(cx, "form-save");
    assert_eq!(view.read_with(cx, |m, _| m.error.clone()), "Type a name.");
    type_in(&view, cx, "My Tool", "mine 'a b");
    click(cx, "form-save");
    assert_eq!(
        view.read_with(cx, |m, _| m.error.clone()),
        "A single quote is not closed."
    );
    type_in(&view, cx, "My Tool", "mine --fast");
    click(cx, "icon-pi");
    click(cx, "form-save");
    cx.run_until_parked();
    let text = fs::read_to_string(programs::path_in(&dir)).unwrap();
    let data: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        data["programs"]["my-tool"],
        json!({"name": "My Tool", "icon": "icons/pi.svg", "run": ["mine", "--fast"]})
    );
    view.read_with(cx, |m, _| {
        assert!(m.form.is_none());
        assert_eq!(m.mine, vec!["my-tool".to_string()]);
        assert!(
            m.agents
                .iter()
                .any(|a| a.program.key == "my-tool" && a.installed)
        );
    });
    assert!(shown(cx, "edit-my-tool"));
    click(cx, "edit-my-tool");
    view.read_with(cx, |m, cx| {
        assert_eq!(m.form.as_ref().unwrap().title, "Edit My Tool");
        assert_eq!(m.cmd_input.read(cx).value(), "mine --fast");
    });
    type_in(&view, cx, "My Tool", "mine");
    click(cx, "form-save");
    cx.run_until_parked();
    let data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(programs::path_in(&dir)).unwrap()).unwrap();
    assert_eq!(data["programs"]["my-tool"]["run"], json!(["mine"]));
    click(cx, "remove-my-tool");
    assert!(shown(cx, "remove-yes"));
    click(cx, "remove-cancel");
    assert!(view.read_with(cx, |m, _| m.confirm.is_none()));
    click(cx, "remove-my-tool");
    click(cx, "remove-yes");
    cx.run_until_parked();
    let data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(programs::path_in(&dir)).unwrap()).unwrap();
    assert_eq!(data["programs"], json!({}));
    view.read_with(cx, |m, _| {
        assert!(m.mine.is_empty());
        assert!(!m.agents.iter().any(|a| a.program.key == "my-tool"));
    });
    assert!(seen.borrow().is_empty());
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn escape_closes_the_form_first(cx: &mut TestAppContext) {
    let dir = scratch("esc");
    let (view, cx, seen) = open(cx, &dir);
    click(cx, "add-agent");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(view.read_with(cx, |m, _| m.form.is_none()));
    assert!(seen.borrow().is_empty());
    view.read_with(cx, |m, _| assert_eq!(height(m), 260.0 + 2.0 * 31.0 + 28.0));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn built_in_names_are_taken_before_the_list_loads(cx: &mut TestAppContext) {
    let dir = scratch("early");
    cx.update(nixa_ui::init);
    let setup = Setup {
        file: programs::path_in(&dir),
        dirs: vec![dir.join("bin")],
        link: None,
        space: None,
        on_event: Rc::new(|_, _| {}),
    };
    let (view, cx) = cx.add_window_view(move |window, cx| build(setup, window, cx));
    assert!(view.read_with(cx, |m, _| !m.loaded && m.agents.is_empty()));
    view.update_in(cx, |menu, window, cx| {
        super::open_form(menu, None, window, cx)
    });
    type_in(&view, cx, "Codex", "echo hi");
    view.update(cx, super::save);
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |m, _| m.error.clone()),
        "\"codex\" is already in the list. Pick another name."
    );
    assert!(!programs::path_in(&dir).exists());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn taken_keys_hold_every_built_in() {
    let keys = super::taken_keys(&[]);
    for (key, ..) in programs::BUILT_IN {
        assert!(keys.iter().any(|k| k == key));
    }
}

#[test]
fn custom_agents_are_read_from_the_brain_reply() {
    let data = json!({"agents": [
        {"id": "a1", "title": "Tester", "purpose": "Runs tests", "state": "open", "start": {"agent": "codex", "model": "gpt-5", "effort": "high"}},
        {"id": "a2", "title": "Off", "off": true, "start": {"agent": "claude"}},
        {"id": "a3", "title": "No start"},
        {"id": "a4", "start": {"agent": "omp"}}
    ]});
    let list = customs_of(&data);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].title, "Tester");
    assert_eq!(profile_text(&list[0]), "codex · gpt-5 · high");
    assert_eq!(list[1].title, "a4");
    assert_eq!(
        profile_text(&list[1]),
        "omp · default model · default effort"
    );
    assert!(customs_of(&json!({})).is_empty());
}
