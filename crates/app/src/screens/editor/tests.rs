use super::*;

#[test]
fn titles_and_show_words() {
    assert_eq!(title_of("/tmp/a/notes.md"), "notes.md");
    assert_eq!(title_of(""), "Editor");
    assert_eq!(split_show("/tmp/a.txt"), ("/tmp/a.txt".to_string(), None));
    assert_eq!(split_show("/tmp/a.txt|find:x"), ("/tmp/a.txt".to_string(), Some("find:x".to_string())));
    assert_eq!(split_show("/tmp/a.txt|"), ("/tmp/a.txt".to_string(), None));
}

#[gpui::test]
fn following_a_local_link_moves_the_window_key(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("nixa-app-editor-link-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.md").display().to_string();
    let b = dir.join("b.md").display().to_string();
    std::fs::write(&a, "[next](b.md)\n").unwrap();
    std::fs::write(&b, "# B\n").unwrap();
    cx.update(|cx| {
        nixa_ui::init(cx);
        init(cx);
        open(&a, cx);
    });
    cx.run_until_parked();
    let handle = cx.update(|cx| first(&a, cx)).expect("a.md has a window");
    handle.update(cx, |ed, _, cx| ed.follow_link("b.md", cx)).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let open = &cx.global::<OpenEditors>().0;
        assert!(!open.contains_key(&normalized(&a)));
        assert_eq!(ids(open.get(&normalized(&b))), vec![handle.window_id()]);
    });
    std::fs::remove_dir_all(&dir).ok();
}

fn first(path: &str, cx: &mut App) -> Option<WindowHandle<Editor>> {
    cx.global::<OpenEditors>().0.get(&normalized(path)).and_then(|l| l.first().copied())
}

fn ids(list: Option<&Vec<WindowHandle<Editor>>>) -> Vec<gpui::WindowId> {
    list.map(|l| l.iter().map(|h| h.window_id()).collect()).unwrap_or_default()
}

#[gpui::test]
fn a_link_to_a_file_open_in_another_window_keeps_both_windows(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("nixa-app-editor-two-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.md").display().to_string();
    let b = dir.join("b.md").display().to_string();
    let c = dir.join("c.md").display().to_string();
    std::fs::write(&a, "[next](b.md)\n").unwrap();
    std::fs::write(&b, "[next](c.md)\n").unwrap();
    std::fs::write(&c, "# C\n").unwrap();
    cx.update(|cx| {
        nixa_ui::init(cx);
        init(cx);
        open(&a, cx);
        open(&b, cx);
    });
    cx.run_until_parked();
    let wa = cx.update(|cx| first(&a, cx)).expect("a.md has a window");
    let wb = cx.update(|cx| first(&b, cx)).expect("b.md has a window");
    wa.update(cx, |ed, _, cx| ed.follow_link("b.md", cx)).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let open = &cx.global::<OpenEditors>().0;
        assert!(!open.contains_key(&normalized(&a)));
        assert_eq!(ids(open.get(&normalized(&b))), vec![wb.window_id(), wa.window_id()]);
    });
    wb.update(cx, |ed, _, cx| ed.follow_link("c.md", cx)).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let open = &cx.global::<OpenEditors>().0;
        assert_eq!(ids(open.get(&normalized(&b))), vec![wa.window_id()]);
        assert_eq!(ids(open.get(&normalized(&c))), vec![wb.window_id()]);
    });
    let windows = cx.update(|cx| cx.windows().len());
    cx.update(|cx| open(&c, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), windows);
    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn show_needs_the_editor_word_and_a_path(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        assert!(!show("files", cx));
        assert!(!show("editor:", cx));
    });
}
