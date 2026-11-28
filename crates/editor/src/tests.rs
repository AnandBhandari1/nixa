use crate::view::{Editor, EditorEvent, Mode, Options, State};
use gpui::{Entity, TestAppContext, VisualTestContext};
use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn temp(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("nixa-edv-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn open<'a>(cx: &'a mut TestAppContext, path: &str) -> (Entity<Editor>, &'a mut VisualTestContext, Rc<RefCell<Vec<EditorEvent>>>) {
    cx.update(|cx| {
        nixa_ui::init(cx);
        crate::bind_keys(cx);
    });
    let p = path.to_string();
    let (ed, vcx) = cx.add_window_view(move |window, cx| Editor::new(Some(&p), Options::default(), window, cx));
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&ed, move |_, ev: &EditorEvent, _| sink.borrow_mut().push(ev.clone())).detach();
    });
    vcx.run_until_parked();
    (ed, vcx, events)
}

fn focus(ed: &Entity<Editor>, vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| {
        let f = ed.read(cx).focus.clone();
        window.focus(&f);
    });
    vcx.run_until_parked();
}

#[gpui::test]
fn source_scrollbar_drags_without_selecting(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton, point, px};
    let dir = temp("scrollbar-drag");
    let path = dir.join("long.txt");
    fs::write(&path, "editor line\n".repeat(400)).unwrap();
    let (ed, vcx, _) = open(cx, &path.to_string_lossy());
    let bounds = ed.read_with(vcx, |ed, _| ed.layout.as_ref().unwrap().bounds);
    let x = bounds.right() - px(4.0);
    let start = point(x, bounds.top() + px(5.0));
    let end = point(x, bounds.bottom() - px(2.0));
    vcx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    ed.read_with(vcx, |ed, _| {
        assert!(ed.layout.as_ref().unwrap().lines.iter().any(|line| line.line == 400));
        assert_eq!(ed.buf.sel.range(), 0..0);
    });
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn preview_drag_highlights_and_copies_rendered_range(cx: &mut TestAppContext) {
    use gpui::{ClipboardItem, Modifiers, MouseButton};
    let dir = temp("preview-range");
    let path = dir.join("a.md");
    fs::write(&path, "Omit this.\n\nSkip **Café** and _tea_\n\n[Second](./elsewhere.md) stays partly outside.\n\nLeave this out.\n").unwrap();
    let (ed, vcx, _) = open(cx, &path.to_string_lossy());
    let (start, end) = ed.read_with(vcx, |ed, _| {
        let selection = &ed.md_selection;
        assert!(selection.highlighted().is_empty());
        (selection.fragment_point(1, 5).unwrap(), selection.fragment_point(2, 6).unwrap())
    });
    vcx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("untouched".into())));
    vcx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    ed.read_with(vcx, |ed, _| {
        assert_eq!(ed.md_selection.highlighted(), &["Café and tea", "Second"]);
    });
    vcx.simulate_keystrokes("cmd-c");
    assert_eq!(vcx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())).as_deref(), Some("Café and tea\n\nSecond"));
    vcx.simulate_click(start, Modifiers::none());
    vcx.run_until_parked();
    ed.read_with(vcx, |ed, _| assert!(ed.md_selection.highlighted().is_empty()));
    fs::remove_dir_all(&dir).ok();
}

fn wait_disk(vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(Duration::from_millis(crate::view::WATCH_MS * 2 + 100));
    vcx.run_until_parked();
}

#[gpui::test]
fn types_and_saves_keeping_line_endings(cx: &mut TestAppContext) {
    let dir = temp("save");
    let path = dir.join("a.txt");
    fs::write(&path, "a\r\nb\r\n").unwrap();
    let (ed, vcx, events) = open(cx, &path.to_string_lossy());
    assert_eq!(ed.read_with(vcx, |e, _| e.state().clone()), State::Text);
    focus(&ed, vcx);
    vcx.simulate_input("x");
    assert!(ed.read_with(vcx, |e, _| e.is_dirty()));
    assert!(events.borrow().contains(&EditorEvent::Edited));
    vcx.simulate_keystrokes("cmd-s");
    vcx.run_until_parked();
    assert_eq!(fs::read_to_string(&path).unwrap(), "xa\r\nb\r\n");
    assert!(!ed.read_with(vcx, |e, _| e.is_dirty()));
    assert_eq!(ed.read_with(vcx, |e, _| e.notice().map(str::to_string)), Some("Saved".into()));
    assert!(events.borrow().iter().any(|e| matches!(e, EditorEvent::Saved(_))));
    vcx.simulate_keystrokes("enter tab");
    assert_eq!(ed.read_with(vcx, |e, _| e.text().to_string()), "x\n  a\nb\n");
    vcx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(ed.read_with(vcx, |e, _| e.text().to_string()), "xa\nb\n");
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn close_asks_when_there_are_edits(cx: &mut TestAppContext) {
    let dir = temp("close");
    let path = dir.join("a.rs");
    fs::write(&path, "fn a() {}\n").unwrap();
    let (ed, vcx, events) = open(cx, &path.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_input("// ");
    let closed_now = ed.update(vcx, |e, cx| e.request_close(cx));
    assert!(!closed_now);
    assert!(ed.read_with(vcx, |e, _| e.closing()));
    assert!(!events.borrow().contains(&EditorEvent::Close));
    vcx.simulate_keystrokes("escape");
    assert!(!ed.read_with(vcx, |e, _| e.closing()));
    ed.update(vcx, |e, cx| e.save(true, cx));
    vcx.run_until_parked();
    assert!(events.borrow().contains(&EditorEvent::Close));
    assert_eq!(fs::read_to_string(&path).unwrap(), "// fn a() {}\n");
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn clean_file_reloads_when_disk_changes(cx: &mut TestAppContext) {
    let dir = temp("reload");
    let path = dir.join("a.txt");
    fs::write(&path, "one\n").unwrap();
    let (ed, vcx, _) = open(cx, &path.to_string_lossy());
    fs::write(&path, "two lines\nnow\n").unwrap();
    wait_disk(vcx);
    assert_eq!(ed.read_with(vcx, |e, _| e.text().to_string()), "two lines\nnow\n");
    assert_eq!(ed.read_with(vcx, |e, _| e.notice().map(str::to_string)), Some("Reloaded. The file changed on disk.".into()));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn edited_file_gets_a_conflict_and_keep_mine_saves(cx: &mut TestAppContext) {
    let dir = temp("conflict");
    let path = dir.join("a.txt");
    fs::write(&path, "one\n").unwrap();
    let (ed, vcx, _) = open(cx, &path.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_input("mine ");
    fs::write(&path, "theirs\n").unwrap();
    wait_disk(vcx);
    assert!(ed.read_with(vcx, |e, _| e.has_conflict()));
    ed.update(vcx, |e, cx| e.save(false, cx));
    assert_eq!(ed.read_with(vcx, |e, _| e.notice().map(str::to_string)), Some("Choose Reload or Keep mine first.".into()));
    ed.update(vcx, |e, cx| e.keep_mine(cx));
    vcx.run_until_parked();
    assert!(!ed.read_with(vcx, |e, _| e.has_conflict()));
    ed.update(vcx, |e, cx| e.save(false, cx));
    vcx.run_until_parked();
    assert_eq!(fs::read_to_string(&path).unwrap(), "mine one\n");
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn deleted_file_shows_a_conflict(cx: &mut TestAppContext) {
    let dir = temp("deleted");
    let path = dir.join("a.txt");
    fs::write(&path, "one\n").unwrap();
    let (ed, vcx, _) = open(cx, &path.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_input("x");
    fs::remove_file(&path).unwrap();
    wait_disk(vcx);
    assert!(ed.read_with(vcx, |e, _| e.has_conflict()));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn find_counts_and_steps(cx: &mut TestAppContext) {
    let dir = temp("find");
    let path = dir.join("a.txt");
    fs::write(&path, "Apple\nbanana apple\napple\n").unwrap();
    let (ed, vcx, _) = open(cx, &path.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_keystrokes("cmd-f");
    vcx.simulate_input("apple");
    vcx.run_until_parked();
    assert_eq!(ed.read_with(vcx, |e, _| e.find_label()), "1/3 · Ln 1");
    vcx.simulate_keystrokes("enter");
    assert_eq!(ed.read_with(vcx, |e, _| e.find_label()), "2/3 · Ln 2");
    vcx.simulate_keystrokes("cmd-shift-g cmd-shift-g");
    assert_eq!(ed.read_with(vcx, |e, _| e.find_label()), "3/3 · Ln 3");
    vcx.simulate_keystrokes("escape");
    assert!(!ed.read_with(vcx, |e, _| e.finding()));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn shapes_pick_their_start_mode(cx: &mut TestAppContext) {
    let dir = temp("modes");
    let md = dir.join("n.md");
    fs::write(&md, "# Hi\n\n[out](../x.md)\n").unwrap();
    let (ed, vcx, _) = open(cx, &md.to_string_lossy());
    assert_eq!(ed.read_with(vcx, |e, _| e.mode()), Mode::Preview);
    ed.update(vcx, |e, cx| e.follow_link("../x.md", cx));
    assert_eq!(ed.read_with(vcx, |e, _| e.notice().map(str::to_string)), Some("This link points outside the document folder.".into()));
    fs::write(dir.join("other.md"), "other\n").unwrap();
    ed.update(vcx, |e, cx| e.follow_link("other.md", cx));
    vcx.run_until_parked();
    assert_eq!(ed.read_with(vcx, |e, _| e.text().to_string()), "other\n");
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn odd_files_show_a_message(cx: &mut TestAppContext) {
    let dir = temp("odd");
    let bin = dir.join("b.dat");
    fs::write(&bin, [0u8, 1, 2]).unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend(4u32.to_be_bytes());
    png.extend(3u32.to_be_bytes());
    fs::write(dir.join("p.png"), &png).unwrap();
    let (ed, vcx, _) = open(cx, &bin.to_string_lossy());
    assert_eq!(ed.read_with(vcx, |e, _| e.state().clone()), State::Binary);
    for (name, want) in [("none.txt", State::Missing), ("", State::Folder), ("p.png", State::Image)] {
        let p = dir.join(name).to_string_lossy().trim_end_matches('/').to_string();
        ed.update(vcx, |e, cx| e.open(&p, cx));
        vcx.run_until_parked();
        assert_eq!(ed.read_with(vcx, |e, _| e.state().clone()), want, "{name}");
    }
    ed.update(vcx, |e, cx| e.open("relative/path.txt", cx));
    assert!(matches!(ed.read_with(vcx, |e, _| e.state().clone()), State::Error(m) if m.contains("full path")));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn dirty_editor_will_not_open_another_file(cx: &mut TestAppContext) {
    let dir = temp("other");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    fs::write(&a, "a\n").unwrap();
    fs::write(&b, "b\n").unwrap();
    let (ed, vcx, _) = open(cx, &a.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_input("x");
    let opened = ed.update(vcx, |e, cx| e.open(&b.to_string_lossy(), cx));
    assert!(!opened);
    assert_eq!(ed.read_with(vcx, |e, _| e.text().to_string()), "xa\n");
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn big_files_open_read_only(cx: &mut TestAppContext) {
    let dir = temp("big");
    let path = dir.join("big.log");
    fs::write(&path, "line of text\n".repeat(120_000)).unwrap();
    let (ed, vcx, _) = open(cx, &path.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_input("x");
    assert!(!ed.read_with(vcx, |e, _| e.is_dirty()));
    assert!(ed.read_with(vcx, |e, _| e.notice().unwrap_or("").starts_with("Read only.")));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn save_and_close_stays_open_when_typing_during_the_save(cx: &mut TestAppContext) {
    let dir = temp("saveclose");
    let path = dir.join("a.txt");
    fs::write(&path, "a\n").unwrap();
    let (ed, vcx, events) = open(cx, &path.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_input("x");
    ed.update(vcx, |e, cx| e.save(true, cx));
    vcx.simulate_input("y");
    vcx.run_until_parked();
    assert_eq!(fs::read_to_string(&path).unwrap(), "xa\n");
    assert!(ed.read_with(vcx, |e, _| e.is_dirty()));
    assert!(!events.borrow().contains(&EditorEvent::Close));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn close_and_links_wait_for_a_pending_save(cx: &mut TestAppContext) {
    let dir = temp("pending");
    let path = dir.join("a.txt");
    fs::write(&path, "a\n").unwrap();
    fs::write(dir.join("b.txt"), "b\n").unwrap();
    let (ed, vcx, events) = open(cx, &path.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_input("x");
    let (closed, moved) = ed.update(vcx, |e, cx| {
        e.save(true, cx);
        e.buf.undo();
        let closed = e.request_close(cx);
        e.follow_link("b.txt", cx);
        (closed, e.path().map(str::to_string))
    });
    assert!(!closed);
    assert_eq!(moved, Some(path.to_string_lossy().to_string()));
    vcx.run_until_parked();
    assert_eq!(fs::read_to_string(&path).unwrap(), "xa\n");
    assert_eq!(ed.read_with(vcx, |e, _| e.text().to_string()), "a\n");
    assert!(ed.read_with(vcx, |e, _| e.is_dirty()));
    assert!(!events.borrow().contains(&EditorEvent::Close));
    let closed = ed.update(vcx, |e, cx| {
        e.save(false, cx);
        e.request_close(cx)
    });
    assert!(!closed);
    assert!(!events.borrow().contains(&EditorEvent::Close));
    vcx.run_until_parked();
    assert_eq!(fs::read_to_string(&path).unwrap(), "a\n");
    assert!(events.borrow().contains(&EditorEvent::Close));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn a_cancelled_deferred_close_does_not_close_a_later_save(cx: &mut TestAppContext) {
    let dir = temp("deferred");
    let path = dir.join("a.txt");
    fs::write(&path, "a\n").unwrap();
    let (ed, vcx, events) = open(cx, &path.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_input("x");
    let closed = ed.update(vcx, |e, cx| {
        e.save(true, cx);
        e.request_close(cx)
    });
    assert!(!closed);
    vcx.simulate_input("y");
    vcx.run_until_parked();
    assert!(ed.read_with(vcx, |e, _| e.is_dirty()));
    assert!(!events.borrow().contains(&EditorEvent::Close));
    vcx.simulate_keystrokes("escape");
    ed.update(vcx, |e, cx| e.save(false, cx));
    vcx.run_until_parked();
    assert_eq!(fs::read_to_string(&path).unwrap(), "xya\n");
    assert!(!events.borrow().contains(&EditorEvent::Close));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn link_outside_its_folder_asks_before_saving(cx: &mut TestAppContext) {
    let dir = temp("link");
    fs::create_dir_all(dir.join("proj")).unwrap();
    fs::create_dir_all(dir.join("away")).unwrap();
    let real = dir.join("away/settings.txt");
    let link = dir.join("proj/notes.txt");
    fs::write(&real, "keep\n").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let (ed, vcx, events) = open(cx, &link.to_string_lossy());
    focus(&ed, vcx);
    vcx.simulate_input("x");
    ed.update(vcx, |e, cx| e.save(true, cx));
    vcx.run_until_parked();
    assert!(ed.read_with(vcx, |e, _| e.link_prompt().is_some_and(|p| p.ends_with("settings.txt"))));
    assert_eq!(fs::read_to_string(&real).unwrap(), "keep\n");
    vcx.simulate_keystrokes("escape");
    assert!(ed.read_with(vcx, |e, _| e.link_prompt().is_none()));
    ed.update(vcx, |e, cx| e.save(true, cx));
    ed.update(vcx, |e, cx| e.save_through_link(cx));
    vcx.run_until_parked();
    assert_eq!(fs::read_to_string(&real).unwrap(), "xkeep\n");
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert!(events.borrow().contains(&EditorEvent::Close));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn go_to_line_waits_for_the_file_to_load(cx: &mut TestAppContext) {
    let dir = temp("line");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    fs::write(&a, "one\ntwo\nthree\n").unwrap();
    fs::write(&b, "alpha\nbeta\ngamma\ndelta\n").unwrap();
    let (ed, vcx, _) = open(cx, &a.to_string_lossy());
    ed.update(vcx, |e, cx| e.go_to_line(3, cx));
    assert_eq!(ed.read_with(vcx, |e, _| e.buf.sel.head), 8);
    ed.update(vcx, |e, cx| {
        e.open(&b.to_string_lossy(), cx);
        e.go_to_line(4, cx);
    });
    vcx.run_until_parked();
    assert_eq!(ed.read_with(vcx, |e, _| e.buf.sel.head), 17);
    ed.update(vcx, |e, cx| e.go_to_line(99, cx));
    assert_eq!(ed.read_with(vcx, |e, _| e.buf.sel.head), 23);
    fs::remove_dir_all(&dir).ok();
}
