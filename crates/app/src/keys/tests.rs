use super::table::{self, Entry, entry};
use super::{file, names};
use crate::actions::{
    ClosePane, CloseTab, FontBigger, GoPane, Hide, NewSpace, NewTab, OpenHelp, PickTab, SwapPane,
};
use crate::pane_moves::Dir;
use gpui::{Action, KeyBinding, Keystroke, NoAction, TestAppContext};

#[test]
fn every_default_name_is_a_known_action() {
    for e in table::defaults() {
        assert!(names::action(&e.name).is_some(), "{}", e.name);
        assert!(super::binding(&e).is_some(), "{} {}", e.key, e.name);
        assert_ne!(names::label(&e.name), e.name, "{} has no label", e.name);
    }
}

#[test]
fn default_keys_are_unique() {
    let list = table::defaults();
    for (i, a) in list.iter().enumerate() {
        assert!(
            !list[i + 1..].iter().any(|b| b.key == a.key),
            "{} is bound twice",
            a.key
        );
    }
}

#[test]
fn keys_are_written_one_way() {
    assert_eq!(table::clean_key("shift-cmd-d"), table::clean_key("cmd-shift-d"));
    assert_eq!(table::clean_key("cmd--").as_deref(), Some("cmd--"));
    assert_eq!(table::clean_key("  "), None);
    assert_eq!(entry("shift-cmd-w", "pane.close"), entry("cmd-shift-w", "pane.close"));
}

#[test]
fn names_with_a_value_build_the_right_action() {
    let same = |name: &str, want: &dyn Action| {
        names::action(name).is_some_and(|a| a.partial_eq(want))
    };
    assert!(same("tab.select:3", &PickTab { number: 3 }));
    assert!(same("pane.go:left", &GoPane { dir: Dir::Left }));
    assert!(same("pane.swap:down", &SwapPane { dir: Dir::Down }));
    assert!(same("none", &NoAction));
    assert!(same("", &NoAction));
    assert!(names::action("tab.select:0").is_none());
    assert!(names::action("tab.select:x").is_none());
    assert!(names::action("pane.go:sideways").is_none());
    assert!(names::action("pane.split:left").is_none());
    assert!(names::action("rocket.launch").is_none());
}

#[test]
fn the_user_file_changes_and_turns_off_keys() {
    let text = r#"{
        "global": {
            "cmd-shift-w": "tab.close",
            "cmd-n": "none",
            "cmd-y": "app.help",
            "cmd-u": "rocket.launch",
            "not a key--x-y": "app.help",
            "cmd-i": 4
        },
        "terminal": {}
    }"#;
    let (user, problems) = file::parse(text);
    assert_eq!(problems.len(), 4, "{problems:?}");
    let merged = table::merged(table::defaults(), &user);
    let find = |key: &str| {
        let key = table::clean_key(key).unwrap_or_default();
        merged.iter().find(|e| e.key == key).map(|e| e.name.clone())
    };
    assert_eq!(find("cmd-shift-w").as_deref(), Some("tab.close"));
    assert_eq!(find("cmd-n").as_deref(), Some("none"));
    assert_eq!(find("cmd-y").as_deref(), Some("app.help"));
    assert_eq!(find("cmd-t").as_deref(), Some("tab.new"));
    assert_eq!(find("cmd-u"), None);
    let count = |key: &str| merged.iter().filter(|e| e.key == key).count();
    assert_eq!(count(&table::clean_key("cmd-shift-w").unwrap_or_default()), 1);
}

#[test]
fn a_broken_file_keeps_the_defaults() {
    let (user, problems) = file::parse("{ not json");
    assert!(user.is_empty());
    assert_eq!(problems.len(), 1);
    let (user, problems) = file::parse("[]");
    assert!(user.is_empty());
    assert_eq!(problems.len(), 1);
    assert_eq!(table::merged(table::defaults(), &user), table::defaults());
}

#[test]
fn a_missing_file_is_not_a_problem() {
    let path = std::env::temp_dir().join(format!("nixa-keys-missing-{}.json", std::process::id()));
    let (user, problems) = file::read(&path);
    assert!(user.is_empty());
    assert!(problems.is_empty());
}

#[test]
fn the_starter_file_is_valid() {
    let (user, problems) = file::parse(file::STARTER);
    assert!(user.is_empty());
    assert!(problems.is_empty());
}

#[test]
fn keys_show_as_mac_symbols() {
    assert_eq!(table::shown("cmd-shift-d"), "⇧⌘D");
    assert_eq!(table::shown("ctrl-alt-left"), "⌃⌥←");
    assert_eq!(table::shown("f2"), "F2");
    assert_eq!(table::shown("cmd-/"), "⌘/");
}

#[test]
fn groups_keep_every_entry_once() {
    let list = vec![
        Entry {
            key: "cmd-t".into(),
            name: "tab.new".into(),
        },
        Entry {
            key: "cmd-q".into(),
            name: "app.quit".into(),
        },
        Entry {
            key: "cmd-n".into(),
            name: "none".into(),
        },
    ];
    let groups = table::groups(&list);
    let titles: Vec<&str> = groups.iter().map(|(t, _)| *t).collect();
    assert_eq!(titles, ["App", "Tabs", "Turned off"]);
    assert_eq!(groups.iter().map(|(_, l)| l.len()).sum::<usize>(), 3);
}

fn first(cx: &mut TestAppContext, key: &str) -> Option<Box<dyn Action>> {
    let stroke = Keystroke::parse(key).expect("key");
    cx.update(|cx| {
        cx.all_bindings_for_input(&[stroke])
            .first()
            .map(|b| b.action().boxed_clone())
    })
}

fn wins(cx: &mut TestAppContext, key: &str, want: &dyn Action) -> bool {
    first(cx, key).is_some_and(|a| a.partial_eq(want))
}

#[gpui::test]
fn the_new_keys_are_bound(cx: &mut TestAppContext) {
    cx.update(|cx| cx.bind_keys(super::key_bindings(&table::defaults())));
    assert!(wins(cx, "cmd-w", &CloseTab));
    assert!(wins(cx, "cmd-shift-w", &ClosePane));
    assert!(wins(cx, "cmd-n", &NewSpace));
    assert!(wins(cx, "cmd-/", &OpenHelp));
    assert!(wins(cx, "cmd-=", &FontBigger));
    assert!(wins(cx, "cmd-h", &Hide));
    assert!(wins(cx, "cmd-alt-left", &GoPane { dir: Dir::Left }));
    assert!(wins(cx, "cmd-alt-shift-down", &SwapPane { dir: Dir::Down }));
    let all = cx.update(|cx| {
        cx.all_bindings_for_input(&[Keystroke::parse("cmd-w").expect("key")])
            .iter()
            .any(|b| b.action().partial_eq(&ClosePane))
    });
    assert!(all, "other windows still close with cmd-w");
}

#[test]
fn a_reload_keeps_other_keys_in_their_place() {
    let old = vec![
        KeyBinding::new("enter", NewTab, Some("Box")),
        KeyBinding::new("cmd-w", ClosePane, None),
        KeyBinding::new("cmd-t", NewTab, None),
        KeyBinding::new("alt-left", Hide, Some("Text")),
    ];
    let app = vec![
        KeyBinding::new("cmd-n", NewSpace, None),
        KeyBinding::new("cmd-w", CloseTab, None),
    ];
    let out = super::rebuilt(&old, app);
    let keys: Vec<String> = out
        .iter()
        .map(|b| b.keystrokes().iter().map(|k| k.unparse()).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(keys, ["enter", "cmd-n", "cmd-w", "alt-left"]);
    assert!(out[2].action().partial_eq(&CloseTab));
    let fresh = super::rebuilt(&[], vec![KeyBinding::new("cmd-n", NewSpace, None)]);
    assert_eq!(fresh.len(), 1);
}

#[gpui::test]
fn a_turned_off_key_binds_no_action(cx: &mut TestAppContext) {
    let user = vec![entry("cmd-w", "none")];
    let merged = table::merged(table::defaults(), &user);
    cx.update(|cx| cx.bind_keys(super::key_bindings(&merged)));
    assert!(wins(cx, "cmd-w", &NoAction));
}
