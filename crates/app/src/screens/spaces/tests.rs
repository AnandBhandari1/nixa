use super::*;
use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<SpacesEvent>>>;

fn open(cx: &mut TestAppContext) -> (Entity<SpacesView>, &mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let (view, vcx) = cx.add_window_view(|_, cx| build(cx));
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &SpacesEvent, _| {
            if *e != SpacesEvent::Reload {
                log.borrow_mut().push(e.clone())
            }
        })
        .detach();
    });
    vcx.run_until_parked();
    (view, vcx, seen)
}

fn space(id: &str, name: &str, folder: &str) -> Space {
    Space {
        id: id.into(),
        name: name.into(),
        folder: folder.into(),
        ..Space::default()
    }
}

fn three() -> Input {
    Input {
        spaces: vec![
            space("s1", "alpha", "/no/such/a"),
            space("s2", "beta", "/no/such/b"),
            space("s3", "gamma", "/no/such/c"),
        ],
        active: Some("s1".into()),
        ..Input::default()
    }
}

fn feed(view: &Entity<SpacesView>, vcx: &mut VisualTestContext, input: Input) {
    vcx.update(|_, cx| sync(view, input, cx));
    vcx.run_until_parked();
}

fn center(vcx: &mut VisualTestContext, selector: &str) -> gpui::Point<gpui::Pixels> {
    vcx.run_until_parked();
    let name: &'static str = Box::leak(selector.to_string().into_boxed_str());
    vcx.debug_bounds(name)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

fn click(vcx: &mut VisualTestContext, selector: &str) {
    let at = center(vcx, selector);
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

fn drawn(vcx: &mut VisualTestContext, selector: &str) -> bool {
    vcx.run_until_parked();
    let name: &'static str = Box::leak(selector.to_string().into_boxed_str());
    vcx.debug_bounds(name).is_some()
}

fn events(seen: &Seen) -> Vec<SpacesEvent> {
    std::mem::take(&mut *seen.borrow_mut())
}

fn act(view: &Entity<SpacesView>, vcx: &mut VisualTestContext, action: Action, id: &str) {
    vcx.update(|window, cx| run(view, action, Some(id), window, cx));
    vcx.run_until_parked();
}

#[gpui::test]
fn rows_show_and_a_click_selects(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    feed(&view, vcx, three());
    for id in ["s1", "s2", "s3"] {
        assert!(drawn(vcx, &format!("space-row-{id}")));
    }
    click(vcx, "space-row-s2");
    assert_eq!(events(&seen), vec![SpacesEvent::Select("s2".into())]);
    click(vcx, "space-new");
    assert_eq!(events(&seen), vec![SpacesEvent::New(None)]);
}

#[gpui::test]
fn the_lock_button_locks_without_selecting(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    feed(&view, vcx, three());
    click(vcx, "space-lock-s2");
    assert_eq!(
        events(&seen),
        vec![SpacesEvent::Lock { id: "s2".into(), locked: true }]
    );
    assert!(vcx.update(|_, cx| view.read(cx).input.spaces[1].locked));
    click(vcx, "space-lock-s2");
    assert_eq!(
        events(&seen),
        vec![SpacesEvent::Lock { id: "s2".into(), locked: false }]
    );
}

#[gpui::test]
fn dragging_a_row_moves_the_space(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    feed(&view, vcx, three());
    let from = center(vcx, "space-row-s1");
    let to = center(vcx, "space-row-s3");
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| view.read(cx).drag.as_ref().and_then(|d| d.over)), Some(2));
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(events(&seen), vec![SpacesEvent::Move { id: "s1".into(), to: 2 }]);
    let ids: Vec<String> = vcx.update(|_, cx| view.read(cx).input.spaces.iter().map(|s| s.id.clone()).collect());
    assert_eq!(ids, ["s2", "s3", "s1"]);
    assert!(vcx.update(|_, cx| view.read(cx).drag.is_none()));
}

#[gpui::test]
fn a_double_click_renames(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    feed(&view, vcx, three());
    let at = center(vcx, "space-row-s2");
    for count in 1..=2 {
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: count,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: count,
        });
    }
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| matches!(&view.read(cx).form, Form::Rename { id, .. } if id == "s2")));
    events(&seen);
    vcx.update(|_, cx| {
        let Form::Rename { input, .. } = &view.read(cx).form else {
            panic!("no rename box");
        };
        input.clone().update(cx, |i, cx| i.set_value("delta", cx));
    });
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        events(&seen),
        vec![SpacesEvent::Rename { id: "s2".into(), name: "delta".into() }]
    );
    assert!(vcx.update(|_, cx| matches!(view.read(cx).form, Form::None)));
    act(&view, vcx, Action::Rename, "s3");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| matches!(view.read(cx).form, Form::None)));
    assert!(events(&seen).is_empty());
}

#[gpui::test]
fn urgency_badges_and_numbers(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    let mut input = three();
    input.counts.insert("s1".into(), Counts { blocked: 2, done: 0 });
    input.counts.insert("s3".into(), Counts { blocked: 0, done: 1 });
    feed(&view, vcx, input);
    assert!(drawn(vcx, "space-blocked-s1"));
    assert!(drawn(vcx, "space-done-s3"));
    assert!(!drawn(vcx, "space-blocked-s2"));
}

#[gpui::test]
fn a_narrow_sidebar_shows_tiles(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    let mut input = three();
    input.narrow = true;
    feed(&view, vcx, input);
    assert!(drawn(vcx, "space-tile-s1"));
    assert!(!drawn(vcx, "space-row-s1"));
    click(vcx, "space-tile-s3");
    assert_eq!(events(&seen), vec![SpacesEvent::Select("s3".into())]);
}

#[gpui::test]
fn close_asks_first_when_tabs_are_open(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    let mut input = three();
    input.tabs.insert("s1".into(), 2);
    feed(&view, vcx, input);
    act(&view, vcx, Action::Close, "s1");
    assert!(events(&seen).is_empty());
    click(vcx, "space-close-yes");
    assert_eq!(events(&seen), vec![SpacesEvent::Close("s1".into())]);
    act(&view, vcx, Action::Close, "s2");
    assert_eq!(events(&seen), vec![SpacesEvent::Close("s2".into())]);
    act(&view, vcx, Action::NewWorktree, "s3");
    assert!(matches!(events(&seen).as_slice(), [SpacesEvent::Notice(_)]));
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nixa-spaces-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn sh(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

#[gpui::test]
fn git_tails_nesting_and_worktree_steps(cx: &mut TestAppContext) {
    let root = temp("wt");
    let main = root.join("proj");
    std::fs::create_dir_all(&main).unwrap();
    sh(&main, &["init", "-q"]);
    std::fs::write(main.join("a"), "a").unwrap();
    sh(&main, &["add", "a"]);
    sh(&main, &["commit", "-qm", "a"]);
    sh(&main, &["worktree", "add", "-q", "-b", "spike", root.join("proj-spike").to_str().unwrap()]);
    let (view, vcx, seen) = open(cx);
    let s = |p: &Path| p.display().to_string();
    let input = Input {
        spaces: vec![
            space("s1", "spike", &s(&root.join("proj-spike"))),
            space("s2", "plain", &s(&root)),
            space("s3", "proj", &s(&main)),
        ],
        active: Some("s3".into()),
        ..Input::default()
    };
    feed(&view, vcx, input);
    let rows: Vec<(String, bool)> = vcx.update(|_, cx| {
        let v = view.read(cx);
        order::ordered(&v.input.spaces, |sp| place(v, sp), &v.checks.git, &v.shown)
            .into_iter()
            .map(|r| (r.id, r.tree.is_some()))
            .collect()
    });
    assert_eq!(
        rows,
        vec![("s2".into(), false), ("s3".into(), false), ("s1".into(), true)]
    );
    assert_eq!(vcx.update(|_, cx| linked(&view, "s1", cx)), Some(true));
    assert_eq!(vcx.update(|_, cx| linked(&view, "s3", cx)), Some(false));
    assert_eq!(vcx.update(|_, cx| linked(&view, "s2", cx)), None);

    act(&view, vcx, Action::NewWorktree, "s3");
    vcx.simulate_input("later");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let made = s(&root.join("proj-later"));
    assert_eq!(
        events(&seen),
        vec![SpacesEvent::Add { folder: made.clone(), name: "later".into() }]
    );
    assert!(Path::new(&made).is_dir());

    act(&view, vcx, Action::OpenWorktree, "s3");
    assert!(vcx.update(|_, cx| matches!(&view.read(cx).form, Form::Open { list, .. } if list.len() == 1)));
    click(vcx, "space-open-0");
    assert_eq!(
        events(&seen),
        vec![SpacesEvent::Add { folder: made.clone(), name: "later".into() }]
    );

    act(&view, vcx, Action::RemoveWorktree, "s3");
    assert!(matches!(events(&seen).as_slice(), [SpacesEvent::Notice(_)]));
    act(&view, vcx, Action::RemoveWorktree, "s1");
    click(vcx, "space-remove-yes");
    assert_eq!(events(&seen), vec![SpacesEvent::Close("s1".into())]);
    assert!(!root.join("proj-spike").exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn commands_cover_every_action() {
    let all = commands();
    assert_eq!(all.len(), 8);
    assert!(all.iter().any(|(_, a)| *a == Action::RemoveWorktree));
}
