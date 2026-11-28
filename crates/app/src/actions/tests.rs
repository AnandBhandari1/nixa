use super::*;
use crate::layout::Node;
use crate::tabs::Tab;
use crate::workspace::EngineState;
use gpui::{Entity, TestAppContext, VisualTestContext, px, size};
use nixa_protocol::{TermId, paths};
use std::path::PathBuf;
use std::sync::Once;

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn nth_counts_from_one() {
    let list = ids(&["a", "b", "c"]);
    assert_eq!(nth(&list, 1).as_deref(), Some("a"));
    assert_eq!(nth(&list, 3).as_deref(), Some("c"));
    assert_eq!(nth(&list, 4), None);
    assert_eq!(nth(&list, 0), None);
    assert_eq!(nth(&[], 1), None);
}

#[test]
fn stepped_wraps_both_ways() {
    let list = ids(&["a", "b", "c"]);
    assert_eq!(stepped(&list, Some("a"), 1).as_deref(), Some("b"));
    assert_eq!(stepped(&list, Some("c"), 1).as_deref(), Some("a"));
    assert_eq!(stepped(&list, Some("a"), -1).as_deref(), Some("c"));
    assert_eq!(stepped(&list, None, 1).as_deref(), Some("a"));
    assert_eq!(stepped(&list, None, -1).as_deref(), Some("c"));
    assert_eq!(stepped(&list, Some("x"), 1).as_deref(), Some("a"));
    assert_eq!(stepped(&[], Some("a"), 1), None);
}

#[test]
fn key_list_reads_a_comma_list() {
    let keys = key_list("cmd-2, ctrl-1,,shift-f2 ,f2");
    let shown: Vec<String> = keys.iter().map(|k| k.unparse()).collect();
    assert_eq!(shown, ["cmd-2", "ctrl-1", "shift-f2", "f2"]);
}

fn bound(cx: &mut TestAppContext, key: &str, want: &dyn Action) -> bool {
    let stroke = Keystroke::parse(key).expect("key");
    cx.update(|cx| {
        cx.all_bindings_for_input(&[stroke])
            .iter()
            .any(|b| b.action().partial_eq(want))
    })
}

#[gpui::test]
fn the_table_has_todays_keys(cx: &mut TestAppContext) {
    cx.update(|cx| cx.bind_keys(crate::keys::key_bindings(&crate::keys::table::defaults())));
    assert!(bound(cx, "f2", &RenameTab));
    assert!(bound(cx, "shift-f2", &RenameSpace));
    assert!(bound(cx, "cmd-alt-]", &NextSpace));
    assert!(bound(cx, "cmd-alt-[", &PrevSpace));
    assert!(bound(cx, "alt-down", &NextSpace));
    assert!(bound(cx, "alt-up", &PrevSpace));
    for number in 1..=9 {
        assert!(bound(cx, &format!("cmd-{number}"), &PickTab { number }));
        assert!(bound(cx, &format!("ctrl-{number}"), &PickSpace { number }));
    }
    assert!(bound(cx, "cmd-]", &NextAgent));
    assert!(bound(cx, "ctrl-tab", &NextTab));
    assert!(bound(cx, "cmd-t", &NewTab));
    assert!(bound(cx, "f1", &OpenPicker));
    assert!(bound(cx, "cmd-w", &CloseTab));
    assert!(bound(cx, "cmd-shift-w", &ClosePane));
    assert!(bound(cx, "cmd-n", &NewSpace));
    assert!(bound(cx, "cmd-/", &OpenHelp));
    assert!(bound(cx, "cmd-h", &Hide));
    assert!(bound(cx, "cmd-0", &FontReset));
    assert!(bound(cx, "cmd-alt-up", &GoPane { dir: Dir::Up }));
    assert!(bound(cx, "cmd-alt-shift-up", &SwapPane { dir: Dir::Up }));
    assert!(bound(cx, "alt-w", &CloseTab));
    assert!(bound(cx, "cmd-}", &NextTab));
    assert!(bound(cx, "cmd-{", &PrevTab));
    assert!(bound(cx, "cmd-shift-]", &NextTab));
    assert!(bound(cx, "cmd-shift-[", &PrevTab));
    assert!(bound(cx, "alt-right", &NextTab));
    assert!(bound(cx, "alt-left", &PrevTab));
    assert!(bound(cx, "alt-v", &Listen));
    assert!(bound(cx, "alt-shift-v", &ToggleNixa));
}

#[test]
fn listen_starts_then_sends() {
    assert_eq!(hold_kind(false), "hold_start");
    assert_eq!(hold_kind(true), "hold_end");
}

fn data_dir() -> PathBuf {
    PathBuf::from(format!("/tmp/mo-p1-{}", std::process::id()))
}

fn t(n: u64) -> TermId {
    TermId::from_number(n)
}

fn tab(id: &str, space: &str, term: u64) -> Tab {
    Tab {
        id: id.into(),
        space: space.into(),
        root: Node::Leaf(t(term)),
        focus: t(term),
        zoom: None,
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    static ENV: Once = Once::new();
    ENV.call_once(|| unsafe {
        std::env::set_var(paths::ENV_DATA_DIR, data_dir());
        std::env::set_var(paths::ENV_ENGINE_SOCKET, data_dir().join("e.sock"));
    });
    cx.update(nixa_ui::init);
    cx.update(bind);
    let (ws, cx) = cx.add_window_view(Workspace::new);
    cx.simulate_resize(size(px(1280.), px(800.)));
    ws.update(cx, |ws, cx| {
        ws.engine = EngineState::Up;
        ws.tabs_path = data_dir().join("keys-tabs.json");
        ws.spaces = Default::default();
        for folder in ["/tmp/p1-a", "/tmp/p1-b", "/tmp/p1-c"] {
            crate::spaces::add(&mut ws.spaces, folder);
        }
        let first = ws.spaces.spaces[0].id.clone();
        let second = ws.spaces.spaces[1].id.clone();
        ws.spaces.active = Some(first.clone());
        ws.tabs.list = vec![
            tab("t1", &first, 1),
            tab("t2", &first, 2),
            tab("t3", &first, 3),
            tab("t4", &second, 4),
        ];
        ws.tabs.active.clear();
        ws.tabs.active.insert(first, "t1".into());
        cx.notify();
    });
    cx.run_until_parked();
    (ws, cx)
}

fn press(cx: &mut VisualTestContext, keys: &str) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

fn now(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> (usize, String) {
    ws.read_with(cx, |ws, _| {
        let space = ws.spaces.active.clone().unwrap_or_default();
        let index = ws.spaces.spaces.iter().position(|s| s.id == space).map_or(0, |i| i + 1);
        (index, ws.active_tab().map(|t| t.id.clone()).unwrap_or_default())
    })
}

#[gpui::test]
fn cmd_numbers_pick_tabs_of_the_space(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    press(cx, "cmd-2");
    assert_eq!(now(&ws, cx), (1, "t2".into()));
    press(cx, "cmd-3");
    assert_eq!(now(&ws, cx), (1, "t3".into()));
    press(cx, "cmd-9");
    assert_eq!(now(&ws, cx), (1, "t3".into()));
    press(cx, "cmd-1");
    assert_eq!(now(&ws, cx), (1, "t1".into()));
}

#[gpui::test]
fn ctrl_numbers_pick_spaces(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    press(cx, "ctrl-2");
    assert_eq!(now(&ws, cx), (2, "t4".into()));
    press(cx, "ctrl-3");
    assert_eq!(now(&ws, cx).0, 3);
    press(cx, "ctrl-9");
    assert_eq!(now(&ws, cx).0, 3);
    press(cx, "ctrl-1");
    assert_eq!(now(&ws, cx), (1, "t1".into()));
    let asked = ws.update(cx, |ws, cx| {
        ws.refocus = false;
        pick_space(ws, 2, cx);
        ws.refocus
    });
    assert!(asked, "a space switch must ask for the pane focus");
}

#[gpui::test]
fn next_and_previous_space_wrap(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    press(cx, "cmd-alt-]");
    assert_eq!(now(&ws, cx).0, 2);
    press(cx, "alt-down");
    assert_eq!(now(&ws, cx).0, 3);
    press(cx, "cmd-alt-]");
    assert_eq!(now(&ws, cx).0, 1);
    press(cx, "cmd-alt-[");
    assert_eq!(now(&ws, cx).0, 3);
    press(cx, "alt-up");
    assert_eq!(now(&ws, cx).0, 2);
}

#[gpui::test]
fn shift_f2_renames_the_active_space(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    press(cx, "ctrl-2");
    ws.update(cx, |ws, cx| {
        ws.sidebar_w = nixa_ui::metrics::SIDEBAR_MIN;
        cx.notify();
    });
    cx.run_until_parked();
    press(cx, "shift-f2");
    let (open, wide) = ws.read_with(cx, |ws, cx| {
        let id = ws.spaces.active.clone().unwrap_or_default();
        let open = matches!(&ws.spaces_view.read(cx).form, Form::Rename { id: r, .. } if *r == id);
        (open, ws.sidebar_w >= SIDEBAR_NARROW)
    });
    assert!(open, "no rename box for the active space");
    assert!(wide, "the sidebar did not open");
    cx.update(|_, cx| {
        let input = match &ws.read(cx).spaces_view.read(cx).form {
            Form::Rename { input, .. } => input.clone(),
            _ => panic!("no rename box"),
        };
        input.update(cx, |i, cx| i.set_value("second home", cx));
    });
    press(cx, "enter");
    let name = ws.read_with(cx, |ws, _| ws.active_space().map(|s| s.name.clone()));
    assert_eq!(name.as_deref(), Some("second home"));
}

#[gpui::test]
fn f2_asks_to_rename_the_active_tab(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    ws.update(cx, |ws, _| ws.notice = None);
    press(cx, "cmd-2");
    press(cx, "f2");
    let asked = ws.read_with(cx, |ws, cx| {
        ws.hub
            .agents
            .read(cx)
            .renaming
            .as_ref()
            .map(|r| (r.tab.clone(), r.term.clone(), r.start.clone()))
    });
    assert_eq!(asked, Some(("t2".into(), t(2), "shell".into())));
    assert!(cx.debug_bounds("agents-rename-inline").is_some());
    assert!(cx.debug_bounds("agents-rename-input").is_none());
    press(cx, "escape");
    assert!(ws.read_with(cx, |ws, cx| ws.hub.agents.read(cx).renaming.is_none()));
}

#[gpui::test]
fn a_kept_files_preview_stops_being_a_preview(cx: &mut TestAppContext) {
    use crate::screens::files::preview::Preview;
    let (ws, cx) = setup(cx);
    ws.update(cx, |ws, cx| {
        ws.files.update(cx, |v, _| {
            v.previews.preview = Some(Preview {
                file: "/tmp/p1-a/notes.md".into(),
                tab: "t3".into(),
            })
        });
        let event = crate::screens::agents::AgentsEvent::KeepPreview { tab: "t3".into() };
        crate::screens::agents::on_event(ws, &event, cx);
    });
    ws.read_with(cx, |ws, cx| {
        let p = &ws.files.read(cx).previews;
        assert!(p.preview.is_none());
        assert_eq!(p.kept.get("/tmp/p1-a/notes.md").map(String::as_str), Some("t3"));
    });
}

fn child(n: u64, parent: u64) -> nixa_protocol::TermInfo {
    nixa_protocol::TermInfo {
        id: t(n),
        name: format!("helper {n}"),
        labels: Default::default(),
        parent: Some(t(parent)),
        children: Vec::new(),
        cmd: vec!["codex".into()],
        cwd: "/tmp/p1-a".into(),
        pid: None,
        running: true,
        exit_code: None,
        status: None,
        status_note: None,
        keep: false,
        draft: false,
        title: None,
        rows: 24,
        cols: 80,
        created_ms: 0,
        seq: 0,
    }
}

#[gpui::test]
fn a_sub_agent_opens_beside_its_parent_and_goes_back(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    ws.update(cx, |ws, cx| {
        ws.terms.insert(t(9), child(9, 2));
        crate::screens::agents::apply::pop_out(ws, &t(9), cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        let tab = ws.active_tab().expect("a tab");
        assert_eq!(tab.id, "t2");
        assert_eq!(crate::layout::leaves(&tab.root), vec![t(2), t(9)]);
        assert_eq!(tab.focus, t(9));
    });
    ws.update(cx, |ws, cx| crate::screens::agents::apply::drop_child(ws, &t(9), cx));
    ws.read_with(cx, |ws, _| {
        let tab = ws.active_tab().expect("a tab");
        assert_eq!(crate::layout::leaves(&tab.root), vec![t(2)]);
        assert_eq!(tab.focus, t(2));
    });
}

fn window_sees_labels(ws: &mut Workspace, term: &TermId, labels: &[(&str, &str)]) {
    if let Some(info) = ws.terms.get_mut(term) {
        info.labels = labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    }
    let spaces = ws.spaces.clone();
    crate::tabs::sync(&mut ws.tabs, &ws.terms, |s| crate::spaces::find(&spaces, s).is_some());
}

fn leaves_of(ws: &Entity<Workspace>, cx: &mut VisualTestContext, tab: &str) -> Vec<TermId> {
    ws.read_with(cx, |ws, _| {
        crate::tabs::find(&ws.tabs, tab).map(|t| crate::layout::leaves(&t.root)).unwrap_or_default()
    })
}

#[gpui::test]
fn a_late_pop_out_label_does_not_bring_a_sent_back_pane_back(cx: &mut TestAppContext) {
    use crate::screens::agents::apply::{LabelJob, labels_done};
    use crate::tabs::{LABEL_SPACE, LABEL_TAB};
    let (ws, cx) = setup(cx);
    let space = ws.read_with(cx, |ws, _| ws.spaces.spaces[0].id.clone());
    let placed = [(LABEL_SPACE, space.as_str()), (LABEL_TAB, "t2")];
    ws.update(cx, |ws, cx| {
        let leads: Vec<(u64, String, String)> = ws
            .tabs
            .list
            .iter()
            .map(|tab| (crate::layout::leaves(&tab.root)[0].clone(), tab.space.clone(), tab.id.clone()))
            .map(|(id, space, tab)| (id.to_string().trim_start_matches('m').parse().unwrap_or(0), space, tab))
            .collect();
        for (n, space, tab) in leads {
            let mut lead = child(n, 0);
            lead.parent = None;
            lead.labels.insert(LABEL_SPACE.into(), space);
            lead.labels.insert(LABEL_TAB.into(), tab);
            ws.terms.insert(t(n), lead);
        }
        ws.terms.insert(t(9), child(9, 2));
        crate::screens::agents::apply::pop_out(ws, &t(9), cx);
    });
    cx.run_until_parked();
    assert_eq!(leaves_of(&ws, cx, "t2"), vec![t(2), t(9)]);
    ws.update(cx, |ws, cx| labels_done(ws, LabelJob::Unplace { term: t(9) }, Ok(()), cx));
    assert_eq!(leaves_of(&ws, cx, "t2"), vec![t(2)]);
    ws.update(cx, |ws, _| window_sees_labels(ws, &t(9), &placed));
    assert_eq!(leaves_of(&ws, cx, "t2"), vec![t(2)], "a late pop out label brought the pane back");
    ws.update(cx, |ws, _| window_sees_labels(ws, &t(9), &[(LABEL_SPACE, space.as_str())]));
    ws.update(cx, |ws, _| window_sees_labels(ws, &t(9), &[]));
    assert_eq!(leaves_of(&ws, cx, "t2"), vec![t(2)]);
    ws.update(cx, |ws, cx| {
        crate::screens::agents::apply::pop_out(ws, &t(9), cx);
        window_sees_labels(ws, &t(9), &placed);
    });
    assert_eq!(leaves_of(&ws, cx, "t2"), vec![t(2), t(9)], "a new pop out must work again");
}

#[gpui::test]
fn keys_do_nothing_without_spaces(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    ws.update(cx, |ws, cx| {
        ws.spaces = Default::default();
        ws.tabs.list.clear();
        ws.tabs.active.clear();
        ws.notice = None;
        cx.notify();
    });
    cx.run_until_parked();
    for key in [
        "cmd-1", "ctrl-1", "cmd-alt-]", "alt-up", "f2", "shift-f2", "alt-w", "alt-right", "cmd-}",
        "cmd-w", "cmd-alt-left", "cmd-alt-shift-right",
    ] {
        press(cx, key);
    }
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.spaces.active, None);
        assert_eq!(ws.notice, None);
        assert!(matches!(ws.spaces_view.read(cx).form, Form::None));
    });
}

#[gpui::test]
fn tab_aliases_step_tabs_and_wrap(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    press(cx, "alt-right");
    assert_eq!(now(&ws, cx), (1, "t2".into()));
    press(cx, "cmd-}");
    assert_eq!(now(&ws, cx), (1, "t3".into()));
    press(cx, "alt-right");
    assert_eq!(now(&ws, cx), (1, "t1".into()));
    press(cx, "alt-left");
    assert_eq!(now(&ws, cx), (1, "t3".into()));
    press(cx, "cmd-{");
    assert_eq!(now(&ws, cx), (1, "t2".into()));
    press(cx, "cmd-shift-]");
    assert_eq!(now(&ws, cx), (1, "t3".into()));
    press(cx, "cmd-shift-[");
    assert_eq!(now(&ws, cx), (1, "t2".into()));
}

fn split_second_tab(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update(cx, |ws, cx| {
        ws.engine = EngineState::Starting;
        if let Some(tab) = ws.tabs.list.iter_mut().find(|t| t.id == "t2") {
            tab.root = crate::layout::row(vec![Node::Leaf(t(2)), Node::Leaf(t(5))]);
            tab.focus = t(5);
        }
        cx.notify();
    });
    cx.run_until_parked();
}

fn tab_ids(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    ws.read_with(cx, |ws, _| ws.tabs.list.iter().map(|t| t.id.clone()).collect())
}

#[gpui::test]
fn alt_w_closes_the_whole_tab(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    split_second_tab(&ws, cx);
    press(cx, "cmd-2");
    press(cx, "alt-w");
    assert_eq!(tab_ids(&ws, cx), ["t1", "t3", "t4"]);
}

#[gpui::test]
fn cmd_w_closes_the_whole_tab(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    split_second_tab(&ws, cx);
    press(cx, "cmd-2");
    press(cx, "cmd-w");
    assert_eq!(tab_ids(&ws, cx), ["t1", "t3", "t4"]);
}

#[gpui::test]
fn cmd_alt_arrows_move_focus_and_swap_panes(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    split_second_tab(&ws, cx);
    press(cx, "cmd-2");
    let state = |ws: &Entity<Workspace>, cx: &mut VisualTestContext| {
        ws.read_with(cx, |ws, _| {
            let tab = ws.tabs.list.iter().find(|t| t.id == "t2").expect("t2");
            (crate::layout::leaves(&tab.root), tab.focus.clone())
        })
    };
    press(cx, "cmd-alt-left");
    assert_eq!(state(&ws, cx), (vec![t(2), t(5)], t(2)));
    press(cx, "cmd-alt-left");
    assert_eq!(state(&ws, cx), (vec![t(2), t(5)], t(2)));
    press(cx, "cmd-alt-shift-right");
    assert_eq!(state(&ws, cx), (vec![t(5), t(2)], t(2)));
    press(cx, "cmd-alt-left");
    assert_eq!(state(&ws, cx), (vec![t(5), t(2)], t(5)));
}

#[gpui::test]
fn cmd_shift_w_closes_one_pane(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    split_second_tab(&ws, cx);
    press(cx, "cmd-2");
    press(cx, "cmd-shift-w");
    assert_eq!(tab_ids(&ws, cx), ["t1", "t2", "t3", "t4"]);
    let leaves = ws.read_with(cx, |ws, _| {
        ws.tabs.list.iter().find(|t| t.id == "t2").map(|t| crate::layout::leaves(&t.root))
    });
    assert_eq!(leaves, Some(vec![t(2)]));
}

#[gpui::test]
fn voice_keys_reach_the_brain_link(cx: &mut TestAppContext) {
    let (ws, cx) = setup(cx);
    ws.update(cx, |ws, _| {
        ws.hub.link = None;
        ws.notice = None;
    });
    press(cx, "alt-v");
    let notice = ws.read_with(cx, |ws, _| ws.notice.clone()).unwrap_or_default();
    assert!(notice.starts_with("Nixa cannot listen now"), "{notice}");
    ws.update(cx, |ws, _| ws.notice = None);
    press(cx, "alt-shift-v");
    let notice = ws.read_with(cx, |ws, _| ws.notice.clone()).unwrap_or_default();
    assert!(notice.starts_with("Nixa was not switched"), "{notice}");
}
