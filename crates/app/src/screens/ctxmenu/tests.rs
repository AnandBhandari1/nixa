use super::model::*;
use super::run;
use super::view::{CtxEvent, CtxMenu, build, commands};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
use nixa_protocol::TermId;
use nixa_ui::Icon;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

fn id(n: u64) -> TermId {
    TermId::from_number(n)
}

fn labels(list: &[Entry]) -> Vec<&str> {
    list.iter().map(|e| e.label.as_str()).collect()
}

fn separated(list: &[Entry]) -> Vec<&str> {
    list.iter()
        .filter(|e| e.separated)
        .map(|e| e.label.as_str())
        .collect()
}

fn tab(id: &str, settled: bool) -> TabFact {
    TabFact {
        id: id.into(),
        name: format!("tab {id}"),
        settled,
        ..TabFact::default()
    }
}

fn facts() -> Facts {
    Facts {
        spaces: vec![SpaceFact {
            id: "s1".into(),
            name: "nixa".into(),
            folder: "/work/nixa".into(),
            place: "/work/nixa".into(),
            tabs: vec![tab("t1", false), tab("t2", true)],
        }],
        terms: vec![
            TermFact {
                id: id(1),
                name: "lead".into(),
                to_program: false,
                parent: None,
            },
            TermFact {
                id: id(2),
                name: "helper".into(),
                to_program: false,
                parent: Some(id(1)),
            },
            TermFact {
                id: id(3),
                name: String::new(),
                to_program: true,
                parent: Some(id(9)),
            },
        ],
        themes: Vec::new(),
        theme: Some("ink".into()),
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mo-w35-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp folder");
    dir
}

#[test]
fn link_menu_opens_and_shows_in_finder() {
    let target = Target::Link {
        path: "/work/nixa/src/main.rs".into(),
    };
    let list = entries(&target, &facts(), None);
    assert_eq!(labels(&list), ["Open in here", "Show in Finder"]);
    assert_eq!(
        list[1].action,
        Action::LinkReveal {
            path: "/work/nixa/src/main.rs".into()
        }
    );
    assert_eq!(list[0].icon, Icon::Notebook);
    assert_eq!(list[1].icon, Icon::Folder);
    assert!(separated(&list).is_empty());
    assert_eq!(title(&target), "main.rs");
}

#[test]
fn an_empty_link_has_no_items() {
    let empty = Target::Link {
        path: String::new(),
    };
    assert!(entries(&empty, &facts(), None).is_empty());
}

#[test]
fn pane_menu_without_a_known_terminal() {
    let target = Target::Pane {
        tab: "t1".into(),
        terminal: id(7),
    };
    let list = entries(&target, &facts(), None);
    assert_eq!(
        labels(&list),
        [
            "Split right",
            "Split down",
            "Zoom",
            "Close pane",
            "Quick actions"
        ]
    );
    assert_eq!(separated(&list), ["Close pane", "Quick actions"]);
    assert_eq!(
        list[0].action,
        Action::Split {
            tab: "t1".into(),
            terminal: id(7),
            side: Side::Right
        }
    );
    assert_eq!(list[1].icon, Icon::PanelRight);
    assert_eq!(list[2].icon, Icon::Square);
    assert_eq!(list[3].icon, Icon::X);
}

#[test]
fn pane_menu_sends_right_clicks_and_goes_back_to_the_parent() {
    let child = Target::Pane {
        tab: "t1".into(),
        terminal: id(2),
    };
    let list = entries(&child, &facts(), None);
    assert_eq!(
        labels(&list),
        [
            "Split right",
            "Split down",
            "Zoom",
            "Send right-clicks to pane",
            "Send back to lead",
            "Close pane",
            "Quick actions"
        ]
    );
    assert_eq!(
        list[3].action,
        Action::RightClicks {
            terminal: id(2),
            to_program: true
        }
    );
    assert_eq!(list[4].action, Action::SendBack { terminal: id(2) });
    let lead = Target::Pane {
        tab: "t1".into(),
        terminal: id(1),
    };
    assert!(
        !labels(&entries(&lead, &facts(), None))
            .iter()
            .any(|l| l.starts_with("Send back"))
    );
    let program = Target::Pane {
        tab: "t1".into(),
        terminal: id(3),
    };
    let list = entries(&program, &facts(), None);
    assert_eq!(list[3].label, "Use the built-in right-click menu");
    assert_eq!(
        list[3].action,
        Action::RightClicks {
            terminal: id(3),
            to_program: false
        }
    );
    assert!(!labels(&list).iter().any(|l| l.starts_with("Send back")));
}

#[test]
fn parent_without_a_name_uses_its_id() {
    let mut f = facts();
    f.terms[0].name.clear();
    let list = entries(
        &Target::Pane {
            tab: "t1".into(),
            terminal: id(2),
        },
        &f,
        None,
    );
    assert!(labels(&list).contains(&"Send back to m1"));
}

#[test]
fn tab_menu_has_rename_save_put_away_and_close() {
    let target = Target::Tab {
        space: "s1".into(),
        id: Some("t1".into()),
        terminal: Some(id(1)),
    };
    let list = entries(&target, &facts(), None);
    assert_eq!(
        labels(&list),
        [
            "Rename",
            "New tab",
            "Save as custom agent",
            "Put tab away",
            "Close",
            "Quick actions"
        ]
    );
    assert_eq!(
        list[2].action,
        Action::SaveAgent {
            terminal: id(1),
            title: "tab t1".into(),
            space: "s1".into(),
            space_name: "nixa".into(),
            folder: "/work/nixa".into()
        }
    );
    assert_eq!(list[3].icon, Icon::Archive);
    assert_eq!(separated(&list), ["Close", "Quick actions"]);
    let settled = Target::Tab {
        space: "s1".into(),
        id: Some("t2".into()),
        terminal: None,
    };
    let list = entries(&settled, &facts(), None);
    assert_eq!(
        labels(&list),
        ["Rename", "New tab", "Bring back", "Close", "Quick actions"]
    );
    assert_eq!(list[2].icon, Icon::Pin);
}

#[test]
fn tab_menu_on_the_bar_without_a_tab() {
    let target = Target::Tab {
        space: "s1".into(),
        id: None,
        terminal: None,
    };
    let list = entries(&target, &facts(), None);
    assert_eq!(labels(&list), ["New tab", "Quick actions"]);
    let unknown = Target::Tab {
        space: "gone".into(),
        id: Some("t9".into()),
        terminal: Some(id(1)),
    };
    assert_eq!(
        labels(&entries(&unknown, &facts(), None)),
        ["Rename", "New tab", "Close", "Quick actions"]
    );
}

#[test]
fn settled_menu_lists_put_away_tabs_by_name() {
    let mut f = facts();
    f.spaces[0].tabs = vec![
        TabFact {
            id: "a".into(),
            name: "auto".into(),
            named: true,
            program: Some("claude".into()),
            settled: true,
            ..TabFact::default()
        },
        TabFact {
            id: "b".into(),
            name: "b".into(),
            program: Some("codex".into()),
            settled: true,
            ..TabFact::default()
        },
        TabFact {
            id: "c".into(),
            name: "c".into(),
            title: Some("vim notes".into()),
            settled: true,
            ..TabFact::default()
        },
        TabFact {
            id: "d".into(),
            name: "plain".into(),
            settled: true,
            ..TabFact::default()
        },
        TabFact {
            id: "e".into(),
            name: "live".into(),
            ..TabFact::default()
        },
    ];
    let list = entries(&Target::Settled { space: "s1".into() }, &f, None);
    assert_eq!(
        labels(&list),
        ["auto", "codex", "vim notes", "plain", "Quick actions"]
    );
    assert_eq!(list[1].action, Action::UnsettleTab { id: "b".into() });
}

#[test]
fn space_menu_follows_the_checkout() {
    let target = Target::Space { id: "s1".into() };
    assert_eq!(
        labels(&entries(&target, &facts(), None)),
        ["Rename", "Close", "Quick actions"]
    );
    let main = entries(&target, &facts(), Some(Checkout::Main));
    assert_eq!(
        labels(&main),
        [
            "Rename",
            "New worktree",
            "Open worktree",
            "Close",
            "Quick actions"
        ]
    );
    assert_eq!(
        main[1].action,
        Action::Worktree {
            space: "s1".into(),
            step: Worktree::New
        }
    );
    assert_eq!(main[1].icon, Icon::GitBranch);
    let linked = entries(&target, &facts(), Some(Checkout::Linked));
    assert_eq!(
        labels(&linked),
        [
            "Rename",
            "New worktree",
            "Open worktree",
            "Remove worktree",
            "Close",
            "Quick actions"
        ]
    );
    assert_eq!(
        linked[3].action,
        Action::Worktree {
            space: "s1".into(),
            step: Worktree::Remove
        }
    );
}

#[test]
fn default_menu_has_new_space_keybinds_and_themes() {
    let list = entries(&Target::Default, &facts(), None);
    assert_eq!(
        labels(&list),
        [
            "New space",
            "Keybinds",
            "Theme: graphite",
            "Theme: ink",
            "Theme: moss",
            "Theme: atom one dark",
            "Quick actions"
        ]
    );
    assert_eq!(
        separated(&list),
        ["Keybinds", "Theme: graphite", "Quick actions"]
    );
    assert_eq!(
        list[5].action,
        Action::Theme {
            name: "atom-one-dark".into()
        }
    );
    assert!(list[3].checked);
    assert!(!list[2].checked);
    assert_eq!(list[0].icon, Icon::Plus);
    assert_eq!(list[2].icon, Icon::Settings);
    let mut f = facts();
    f.themes = vec!["paper".into()];
    assert_eq!(
        labels(&entries(&Target::Default, &f, None)),
        ["New space", "Keybinds", "Theme: paper", "Quick actions"]
    );
}

#[test]
fn key_text_uses_mac_order_and_signs() {
    assert_eq!(super::keys::key_text(false, false, true, true, "d"), "⇧⌘D");
    assert_eq!(
        super::keys::key_text(true, true, false, false, "tab"),
        "⌃⌥⇥"
    );
    assert_eq!(super::keys::key_text(false, false, false, true, ","), "⌘,");
    assert_eq!(
        super::keys::key_text(false, false, true, true, "enter"),
        "⇧⌘↩"
    );
    assert_eq!(
        super::keys::key_text(false, false, false, false, "f5"),
        "F5"
    );
}

#[test]
fn small_helpers() {
    assert_eq!(base_name("/a/b/"), "b");
    assert_eq!(base_name("file"), "file");
    assert_eq!(base_name("/"), "/");
    assert_eq!(space_place("/f", ["", "/f/sub"]), "/f/sub");
    assert_eq!(space_place("/f", [""]), "/f");
    assert_eq!(title(&Target::Default), "Actions");
    assert_eq!(run::right_click_value(true), "program");
    assert_eq!(run::right_click_value(false), "menu");
    assert_eq!(RIGHT_CLICK_LABEL, "nixa.right-click");
    assert_eq!(commands().len(), 1);
}

#[test]
fn save_body_names_the_terminal_title_and_purpose() {
    let body = run::save_body(&id(12), " lead ", " checks the docs ");
    assert_eq!(body["terminal"], "mc");
    assert_eq!(body["title"], "lead");
    assert_eq!(body["purpose"], "checks the docs");
    assert_eq!(run::save_missing("", "x"), Some(run::NEED_TITLE));
    assert_eq!(run::save_missing("x", " "), Some(run::NEED_PURPOSE));
    assert_eq!(run::save_missing("x", "y"), None);
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git")
        .status
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn checkout_tells_main_from_linked() {
    let plain = temp("plain");
    assert_eq!(run::checkout(&plain), None);
    let repo = temp("repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "one"]);
    assert_eq!(run::checkout(&repo), Some(Checkout::Main));
    std::fs::create_dir_all(repo.join("sub")).ok();
    assert_eq!(run::checkout(&repo.join("sub")), Some(Checkout::Main));
    let tree = repo.with_extension("tree");
    std::fs::remove_dir_all(&tree).ok();
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            tree.to_str().unwrap(),
            "-b",
            "side",
        ],
    );
    assert_eq!(run::checkout(&tree), Some(Checkout::Linked));
    std::fs::remove_dir_all(&tree).ok();
    std::fs::remove_dir_all(&repo).ok();
    std::fs::remove_dir_all(&plain).ok();
}

type Events = Rc<RefCell<Vec<CtxEvent>>>;

fn open(
    cx: &mut TestAppContext,
    target: Target,
    f: Facts,
) -> (Entity<CtxMenu>, Events, &mut VisualTestContext) {
    cx.update(|cx| {
        nixa_ui::init(cx);
        crate::actions::bind(cx);
    });
    let (menu, vcx) = cx
        .add_window_view(|window, cx| build(target, f, point(px(40.), px(40.)), None, window, cx));
    let events: Events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&menu, move |_, event: &CtxEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
    });
    vcx.run_until_parked();
    (menu, events, vcx)
}

#[gpui::test]
fn keys_move_and_choose(cx: &mut TestAppContext) {
    let (menu, events, vcx) = open(
        cx,
        Target::Pane {
            tab: "t1".into(),
            terminal: id(7),
        },
        facts(),
    );
    assert_eq!(menu.read_with(vcx, |m, _| m.at), Some(0));
    vcx.simulate_keystrokes("down down");
    assert_eq!(menu.read_with(vcx, |m, _| m.at), Some(2));
    vcx.simulate_keystrokes("up up up");
    assert_eq!(menu.read_with(vcx, |m, _| m.at), Some(4));
    vcx.simulate_keystrokes("home enter");
    assert_eq!(
        events.borrow().as_slice(),
        [
            CtxEvent::Run(Action::Split {
                tab: "t1".into(),
                terminal: id(7),
                side: Side::Right
            }),
            CtxEvent::Close
        ]
    );
    vcx.simulate_keystrokes("escape");
    assert_eq!(events.borrow().last(), Some(&CtxEvent::Close));
}

#[gpui::test]
fn rows_show_the_bound_keys(cx: &mut TestAppContext) {
    let (menu, _, vcx) = open(
        cx,
        Target::Pane {
            tab: "t1".into(),
            terminal: id(7),
        },
        facts(),
    );
    let items = menu.update_in(vcx, |m, window, _| m.menu_items(Some(window)));
    let keys: Vec<Option<String>> = items
        .iter()
        .map(|i| i.shortcut.as_ref().map(|k| k.to_string()))
        .collect();
    assert_eq!(
        keys,
        [
            Some("⌘D".into()),
            Some("⇧⌘D".into()),
            Some("⇧⌘↩".into()),
            Some("⇧⌘W".into()),
            None
        ]
    );
}

#[gpui::test]
fn click_a_row_and_the_close_button(cx: &mut TestAppContext) {
    let (_, events, vcx) = open(cx, Target::Space { id: "s1".into() }, facts());
    let row = vcx.debug_bounds("ctx-row-0").expect("first row");
    vcx.simulate_click(row.center(), Modifiers::none());
    assert_eq!(
        events.borrow().as_slice(),
        [
            CtxEvent::Run(Action::RenameSpace { id: "s1".into() }),
            CtxEvent::Close
        ]
    );
    let close = vcx.debug_bounds("ctx-close").expect("close button");
    vcx.simulate_click(close.center(), Modifiers::none());
    assert_eq!(events.borrow().last(), Some(&CtxEvent::Close));
}

#[gpui::test]
fn link_open_tells_a_folder_from_a_file(cx: &mut TestAppContext) {
    let dir = temp("link");
    let path = dir.to_string_lossy().to_string();
    let (menu, events, vcx) = open(
        cx,
        Target::Link {
            path: path.clone(),
        },
        facts(),
    );
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        events.borrow().as_slice(),
        [
            CtxEvent::Open {
                path: path.clone(),
                folder: true
            },
            CtxEvent::Close
        ]
    );
    let file = dir.join("a.txt");
    std::fs::write(&file, "x").unwrap();
    let file = file.to_string_lossy().to_string();
    menu.update(vcx, |m, cx| {
        m.target = Target::Link {
            path: file.clone(),
        };
        m.choose(0, cx);
    });
    {
        let got = events.borrow();
        assert_eq!(
            got[got.len() - 2],
            CtxEvent::Open {
                path: file,
                folder: false
            }
        );
        assert_eq!(got.last(), Some(&CtxEvent::Close));
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn save_without_a_brain_says_so_and_stays_open(cx: &mut TestAppContext) {
    let target = Target::Tab {
        space: "s1".into(),
        id: Some("t1".into()),
        terminal: Some(id(1)),
    };
    let (menu, events, vcx) = open(cx, target, facts());
    vcx.simulate_keystrokes("down down enter");
    assert_eq!(
        events.borrow().as_slice(),
        [CtxEvent::Notice {
            title: run::SAVE_TITLE.into(),
            body: run::NO_BRAIN.into()
        }]
    );
    assert!(!menu.read_with(vcx, |m, _| m.sending));
}

#[gpui::test]
fn space_menu_finds_worktree_rows_from_git(cx: &mut TestAppContext) {
    let repo = temp("space");
    git(&repo, &["init", "-q"]);
    let mut f = facts();
    f.spaces[0].place = repo.to_string_lossy().to_string();
    let (menu, _, vcx) = open(cx, Target::Space { id: "s1".into() }, f);
    for _ in 0..200 {
        if menu.read_with(vcx, |m, _| m.git.is_some()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        vcx.run_until_parked();
    }
    let names = menu.read_with(vcx, |m, _| {
        m.entries().into_iter().map(|e| e.label).collect::<Vec<_>>()
    });
    assert_eq!(
        names,
        [
            "Rename",
            "New worktree",
            "Open worktree",
            "Close",
            "Quick actions"
        ]
    );
    std::fs::remove_dir_all(&repo).ok();
}

#[gpui::test]
fn every_row_waits_while_saving(cx: &mut TestAppContext) {
    let target = Target::Tab {
        space: "s1".into(),
        id: Some("t1".into()),
        terminal: Some(id(1)),
    };
    let (menu, events, vcx) = open(cx, target, facts());
    menu.update(vcx, |m, cx| {
        m.sending = true;
        cx.notify();
    });
    vcx.run_until_parked();
    let items = menu.read_with(vcx, |m, _| m.menu_items(None));
    assert!(items.iter().all(|i| i.disabled));
    assert_eq!(items[2].label.as_ref(), run::SAVING);
    vcx.simulate_keystrokes("down enter");
    let row = vcx.debug_bounds("ctx-row-4").expect("close row");
    vcx.simulate_click(row.center(), Modifiers::none());
    assert!(events.borrow().is_empty());
    vcx.simulate_keystrokes("escape");
    assert_eq!(events.borrow().as_slice(), [CtxEvent::Close]);
}
