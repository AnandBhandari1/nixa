use super::*;
use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext,
};
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<FilesEvent>>>;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nixa-files-ui-{name}-{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample(name: &str) -> PathBuf {
    let dir = temp(name);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn helper() {}\n").unwrap();
    fs::write(dir.join("notes.md"), "a helper note\n").unwrap();
    fs::write(dir.join("run.sh"), "echo hi\n").unwrap();
    dir
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    root: &Path,
) -> (Entity<FilesView>, &'a mut VisualTestContext, Seen, PathBuf) {
    cx.update(nixa_ui::init);
    let home = temp(&format!(
        "home-{}",
        root.file_name().unwrap().to_string_lossy()
    ));
    let home_for = home.clone();
    let (view, cx) = cx.add_window_view(move |_, cx| {
        let mut v = build(cx);
        v.home = home_for;
        v
    });
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|_, app| {
        app.subscribe(&view, move |_, event: &FilesEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
    });
    let root = root.display().to_string();
    view.update(cx, |v, cx| change_root(v, Some(root), cx));
    cx.run_until_parked();
    (view, cx, seen, home)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, app| window.draw(app).clear());
}

fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> gpui::Bounds<gpui::Pixels> {
    redraw(cx);
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"))
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let b = bounds(cx, selector);
    cx.simulate_click(b.center(), Modifiers::none());
    cx.run_until_parked();
}

fn double_click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = bounds(cx, selector).center();
    for count in 1..=2 {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: count,
        });
    }
    cx.run_until_parked();
}

fn right_click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = bounds(cx, selector).center();
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
}

fn paths(view: &Entity<FilesView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| v.rows.iter().map(|r| r.path.clone()).collect())
}

fn menu_index(view: &Entity<FilesView>, cx: &mut VisualTestContext, action: FileAction) -> usize {
    view.read_with(cx, |v, _| {
        let target = &v.menu.as_ref().expect("menu is open").target;
        actions::menu_items(target)
            .iter()
            .position(|a| *a == action)
            .unwrap()
    })
}

fn choose(view: &Entity<FilesView>, cx: &mut VisualTestContext, action: FileAction) {
    let i = menu_index(view, cx, action);
    cx.update(|window, app| view.update(app, |v, cx| pick(v, i, window, cx)));
    cx.run_until_parked();
}

#[gpui::test]
fn the_tree_lists_and_opens_folders(cx: &mut TestAppContext) {
    let dir = sample("tree");
    let (view, cx, _, _) = open(cx, &dir);
    assert_eq!(paths(&view, cx), vec!["src", "notes.md", "run.sh"]);
    click(cx, "file-src");
    assert_eq!(
        paths(&view, cx),
        vec!["src", "src/lib.rs", "src/main.rs", "notes.md", "run.sh"]
    );
    bounds(cx, "file-src/main.rs");
    click(cx, "file-src");
    assert_eq!(paths(&view, cx), vec!["src", "notes.md", "run.sh"]);
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn terminal_open_keeps_existing_tabs(cx: &mut TestAppContext) {
    let dir = sample("preview");
    let (view, cx, seen, _) = open(cx, &dir);
    let notes = dir.join("notes.md").display().to_string();
    let run = dir.join("run.sh").display().to_string();
    right_click(cx, "file-notes.md");
    choose(&view, cx, FileAction::Terminal);
    assert_eq!(
        seen.borrow().last(),
        Some(&FilesEvent::Terminal {
            file: notes.clone(),
            keep: true,
            replace: None
        })
    );
    cx.update(|_, app| opened(&view, &notes, true, "t1", app));
    right_click(cx, "file-run.sh");
    choose(&view, cx, FileAction::Terminal);
    assert_eq!(
        seen.borrow().last(),
        Some(&FilesEvent::Terminal {
            file: run.clone(),
            keep: true,
            replace: None
        })
    );
    cx.update(|_, app| opened(&view, &run, true, "t2", app));
    right_click(cx, "file-run.sh");
    choose(&view, cx, FileAction::Terminal);
    assert_eq!(seen.borrow().last(), Some(&FilesEvent::Select("t2".into())));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn a_normal_click_opens_the_graphical_preview_after_a_terminal_open(cx: &mut TestAppContext) {
    let dir = sample("graphical");
    let (view, cx, seen, _) = open(cx, &dir);
    let notes = dir.join("notes.md").display().to_string();
    right_click(cx, "file-notes.md");
    choose(&view, cx, FileAction::Terminal);
    cx.update(|_, app| opened(&view, &notes, false, "t1", app));
    click(cx, "file-notes.md");
    assert_eq!(
        seen.borrow().last(),
        Some(&FilesEvent::Open {
            file: notes.clone(),
            keep: false,
            replace: None
        })
    );
    double_click(cx, "file-notes.md");
    assert_eq!(
        seen.borrow().last(),
        Some(&FilesEvent::Open {
            file: notes.clone(),
            keep: true,
            replace: None
        })
    );
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn git_marks_show_on_rows(cx: &mut TestAppContext) {
    let dir = sample("marks");
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return;
    }
    let (view, cx, _, _) = open(cx, &dir);
    view.read_with(cx, |v, _| {
        assert_eq!(git::mark_for(&v.marks, "notes.md"), Some(git::Mark::Added));
        assert_eq!(
            git::mark_for(&v.marks, "src/main.rs"),
            Some(git::Mark::Added)
        );
        assert!(v.index.is_some());
    });
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn right_click_new_file_through_the_prompt(cx: &mut TestAppContext) {
    let dir = sample("newfile");
    let (view, cx, seen, _) = open(cx, &dir);
    right_click(cx, "file-src");
    bounds(cx, "files-menu");
    choose(&view, cx, FileAction::NewFile);
    bounds(cx, "files-prompt");
    cx.simulate_input("fresh.rs");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(dir.join("src/fresh.rs").is_file());
    view.read_with(cx, |v, _| assert!(v.prompt.is_none()));
    assert!(seen.borrow().iter().any(
        |e| matches!(e, FilesEvent::Notify { title, error: false, .. } if title == "File created")
    ));
    click(cx, "file-src");
    assert!(paths(&view, cx).contains(&"src/fresh.rs".to_string()));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn rename_shows_the_error_when_the_name_is_taken(cx: &mut TestAppContext) {
    let dir = sample("rename");
    let (view, cx, _, _) = open(cx, &dir);
    right_click(cx, "file-notes.md");
    choose(&view, cx, FileAction::Rename);
    let value = view.read_with(cx, |v, cx| {
        v.prompt
            .as_ref()
            .unwrap()
            .input
            .read(cx)
            .value()
            .to_string()
    });
    assert_eq!(value, "notes.md");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("run.sh");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let error = view.read_with(cx, |v, _| v.prompt.as_ref().and_then(|p| p.error.clone()));
    assert!(error.unwrap_or_default().contains("already exists"));
    bounds(cx, "files-prompt-error");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("todo.md");
    click(cx, "files-prompt-save");
    assert!(dir.join("todo.md").is_file());
    assert!(!dir.join("notes.md").exists());
    assert!(paths(&view, cx).contains(&"todo.md".to_string()));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn delete_asks_then_moves_to_the_trash(cx: &mut TestAppContext) {
    let dir = sample("delete");
    let (view, cx, _, home) = open(cx, &dir);
    right_click(cx, "file-run.sh");
    choose(&view, cx, FileAction::Delete);
    bounds(cx, "files-confirm");
    assert!(dir.join("run.sh").exists());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.confirm.is_none()));
    assert!(dir.join("run.sh").exists());
    right_click(cx, "file-run.sh");
    choose(&view, cx, FileAction::Delete);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!dir.join("run.sh").exists());
    assert!(home.join(".Trash/run.sh").exists());
    assert!(!paths(&view, cx).contains(&"run.sh".to_string()));
    fs::remove_dir_all(&dir).ok();
    fs::remove_dir_all(&home).ok();
}

#[gpui::test]
fn menu_keys_move_and_choose(cx: &mut TestAppContext) {
    let dir = sample("keys");
    let (view, cx, seen, _) = open(cx, &dir);
    right_click(cx, "file-run.sh");
    cx.simulate_keystrokes("down");
    view.read_with(cx, |v, _| assert_eq!(v.menu.as_ref().unwrap().hi, Some(0)));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let run = dir.join("run.sh").display().to_string();
    let folder = dir.display().to_string();
    assert!(seen.borrow().contains(&FilesEvent::Run {
        name: "run.sh".into(),
        folder,
        argv: vec!["bash".into(), run]
    }));
    view.read_with(cx, |v, _| assert!(v.menu.is_none()));
    right_click(cx, "file-notes.md");
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert!(v.menu.is_none()));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn search_finds_names_and_lines(cx: &mut TestAppContext) {
    let dir = sample("search");
    let (view, cx, seen, _) = open(cx, &dir);
    click(cx, "files-search");
    cx.simulate_input("helper");
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.found.lines.is_empty()));
    cx.executor().advance_clock(SEARCH_WAIT);
    cx.run_until_parked();
    let (names, lines) = view.read_with(cx, |v, _| {
        (
            v.found
                .names
                .iter()
                .map(|h| h.path.clone())
                .collect::<Vec<_>>(),
            v.found
                .lines
                .iter()
                .map(|h| (h.path.clone(), h.line))
                .collect::<Vec<_>>(),
        )
    });
    assert!(names.is_empty());
    assert!(
        lines.contains(&("notes.md".to_string(), Some(1))),
        "{lines:?}"
    );
    assert!(
        lines.contains(&("src/lib.rs".to_string(), Some(1))),
        "{lines:?}"
    );
    click(cx, "hit-0");
    assert!(matches!(
        seen.borrow().last(),
        Some(FilesEvent::Open { keep: false, .. })
    ));
    click(cx, "files-search-clear");
    view.read_with(cx, |v, _| {
        assert!(v.query.is_empty());
        assert!(v.found.lines.is_empty());
    });
    click(cx, "files-search");
    cx.simulate_input("main");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let names = view.read_with(cx, |v, _| {
        v.found
            .names
            .iter()
            .map(|h| h.path.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(names, vec!["src/main.rs".to_string()]);
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn the_watch_sees_new_files(cx: &mut TestAppContext) {
    let dir = sample("watch");
    let (view, cx, _, _) = open(cx, &dir);
    view.update(cx, tick);
    cx.run_until_parked();
    let before = view.read_with(cx, |v, _| v.load_seq);
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(dir.join("added.txt"), "x").unwrap();
    view.update(cx, tick);
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.load_seq) > before);
    assert!(paths(&view, cx).contains(&"added.txt".to_string()));
    fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn root_change_keeps_open_folders_per_root(cx: &mut TestAppContext) {
    let a = sample("root-a");
    let b = sample("root-b");
    let (view, cx, seen, _) = open(cx, &a);
    click(cx, "file-src");
    view.update(cx, |v, cx| {
        change_root(v, Some(b.display().to_string()), cx)
    });
    cx.run_until_parked();
    assert_eq!(paths(&view, cx), vec!["src", "notes.md", "run.sh"]);
    view.update(cx, |v, cx| {
        change_root(v, Some(a.display().to_string()), cx)
    });
    cx.run_until_parked();
    assert!(paths(&view, cx).contains(&"src/main.rs".to_string()));
    click(cx, "files-close");
    assert_eq!(seen.borrow().last(), Some(&FilesEvent::Close));
    view.update(cx, |v, cx| change_root(v, None, cx));
    cx.run_until_parked();
    assert!(paths(&view, cx).is_empty());
    bounds(cx, "files-empty");
    fs::remove_dir_all(&a).ok();
    fs::remove_dir_all(&b).ok();
}

#[gpui::test]
fn copy_path_goes_to_the_clipboard(cx: &mut TestAppContext) {
    let dir = sample("copy");
    let (view, cx, _, _) = open(cx, &dir);
    right_click(cx, "file-notes.md");
    choose(&view, cx, FileAction::Copy);
    let text = cx.update(|_, app| app.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(text, Some(dir.join("notes.md").display().to_string()));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn host_commands_run_in_the_folder() {
    let argv = host::run_argv("/r/bin", &["bash".to_string(), "/r/bin/go.sh".to_string()]);
    assert_eq!(
        argv[..3],
        ["sh".to_string(), "-c".to_string(), host::RUN_IN.to_string()]
    );
    assert_eq!(
        argv[3..],
        ["run", "/r/bin", "bash", "/r/bin/go.sh"].map(String::from)
    );
    let out = std::process::Command::new(&argv[0])
        .args(&argv[1..4])
        .arg("/")
        .args(["pwd"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "/");
}


#[gpui::test]
fn a_failed_create_shows_the_reason_and_no_success(cx: &mut TestAppContext) {
    use crate::workspace::{EngineState, Workspace};
    use nixa_ui::toast::ToastKind;
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| unsafe {
        let dir = std::env::temp_dir().join(format!("nixa-files-no-engine-{}", std::process::id()));
        std::env::set_var(nixa_protocol::paths::ENV_ENGINE_SOCKET, dir.join("e.sock"));
    });
    cx.update(nixa_ui::init);
    let (ws, cx) = cx.add_window_view(Workspace::new);
    let dir = temp("failed-create");
    let file = dir.join("notes.md");
    fs::write(&file, "a note\n").unwrap();
    let folder = dir.display().to_string();
    ws.update(cx, |ws, cx| {
        ws.engine = EngineState::Up;
        ws.tabs_path = dir.join("tabs.json");
        ws.spaces = Default::default();
        let space = crate::spaces::add(&mut ws.spaces, &folder);
        ws.spaces.active = Some(space);
        ws.tabs.list.clear();
        ws.hub.events.toasts.update(cx, |t, _| t.items.clear());
        host::on_event(ws, &FilesEvent::Run { name: "go.sh".into(), folder: folder.clone(), argv: vec!["sh".into(), "go.sh".into()] }, cx);
        host::on_event(ws, &FilesEvent::Terminal { file: file.display().to_string(), keep: false, replace: None }, cx);
        assert_eq!(ws.tabs.pending.len(), 2);
        assert!(ws.hub.events.toasts.read(cx).items.is_empty());
    });
    cx.run_until_parked();
    ws.update(cx, |ws, cx| {
        let items = ws.hub.events.toasts.read(cx).items.clone();
        let mut titles: Vec<&str> = items.iter().map(|t| t.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, vec!["Could not open notes.md in a terminal", "Could not start go.sh"]);
        assert!(items.iter().all(|t| t.kind == ToastKind::Error && !t.body.trim().is_empty()));
        assert!(!items.iter().any(|t| t.kind == ToastKind::Success));
        assert!(ws.tabs.pending.is_empty() && ws.tabs.list.is_empty());
    });
}
