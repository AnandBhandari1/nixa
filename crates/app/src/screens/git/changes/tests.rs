use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<ChangesEvent>>>;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nixa-changes-{name}-{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn setup(dir: &Path, who: bool) {
    let hooks = dir.join(".git").join("nixa-test-hooks");
    fs::create_dir_all(&hooks).unwrap();
    sh(dir, &["config", "commit.gpgsign", "false"]);
    sh(dir, &["config", "tag.gpgsign", "false"]);
    sh(dir, &["config", "core.hooksPath", &hooks.display().to_string()]);
    if who {
        sh(dir, &["config", "user.name", "Test Person"]);
        sh(dir, &["config", "user.email", "test@example.com"]);
    }
}

fn repo(name: &str, who: bool) -> PathBuf {
    let dir = temp(name);
    sh(&dir, &["init", "-q", "-b", "main"]);
    setup(&dir, who);
    fs::write(dir.join("a.txt"), "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n").unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/b.txt"), "bee\n").unwrap();
    if !who {
        sh(&dir, &["-c", "user.name=Seed", "-c", "user.email=seed@example.com", "add", "-A"]);
        sh(&dir, &["-c", "user.name=Seed", "-c", "user.email=seed@example.com", "commit", "-q", "-m", "first"]);
    } else {
        sh(&dir, &["add", "-A"]);
        sh(&dir, &["commit", "-q", "-m", "first"]);
    }
    dir
}

fn with_remote(name: &str) -> (PathBuf, PathBuf) {
    let dir = repo(name, true);
    let bare = temp(&format!("{name}-remote"));
    sh(&bare, &["init", "-q", "--bare", "-b", "main"]);
    sh(&dir, &["remote", "add", "origin", &bare.display().to_string()]);
    sh(&dir, &["push", "-q", "-u", "origin", "main"]);
    (dir, bare)
}

fn clone_of(bare: &Path, name: &str) -> PathBuf {
    let dir = temp(name);
    fs::remove_dir_all(&dir).ok();
    let parent = dir.parent().unwrap().to_path_buf();
    sh(&parent, &["clone", "-q", &bare.display().to_string(), &dir.display().to_string()]);
    setup(&dir, true);
    dir
}

fn open<'a>(cx: &'a mut TestAppContext, dir: &Path) -> (Entity<ChangesView>, &'a mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let home = temp(&format!("home-{}", dir.file_name().unwrap().to_string_lossy()));
    let home = home.display().to_string();
    let (view, cx) = cx.add_window_view(move |_, cx| {
        let mut v = build(cx);
        v.home = home;
        v
    });
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|_, app| {
        app.subscribe(&view, move |_, e: &ChangesEvent, _| sink.borrow_mut().push(e.clone())).detach();
    });
    let place = dir.display().to_string();
    view.update(cx, |v, cx| change_place(v, Some(place.clone()), Some(place), cx));
    cx.run_until_parked();
    (view, cx, seen)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, app| window.draw(app).clear());
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    redraw(cx);
    let b = cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(b.center(), Modifiers::none());
    cx.run_until_parked();
}

fn shown(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    redraw(cx);
    cx.debug_bounds(selector).is_some()
}

fn paths(view: &Entity<ChangesView>, cx: &mut VisualTestContext) -> Vec<(String, bool)> {
    view.read_with(cx, |v, _| v.files.iter().map(|f| (f.path.clone(), f.staged)).collect())
}

fn type_message(view: &Entity<ChangesView>, cx: &mut VisualTestContext, text: &str) {
    let text = text.to_string();
    view.update(cx, |v, cx| v.message.update(cx, |i, cx| i.set_value(text, cx)));
    cx.run_until_parked();
}

fn head_subject(dir: &Path) -> String {
    sh(dir, &["log", "-1", "--format=%s"]).trim().to_string()
}

fn notes(seen: &Seen) -> Vec<String> {
    seen.borrow()
        .iter()
        .filter_map(|e| match e {
            ChangesEvent::Notify(n) => Some(n.title.clone()),
            _ => None,
        })
        .collect()
}

fn done(dirs: &[&Path]) {
    for d in dirs {
        fs::remove_dir_all(d).ok();
    }
}

#[test]
fn rows_group_by_folder_and_fold() {
    let file = |path: &str, folder: &str| FileEntry {
        path: path.into(),
        folder: folder.into(),
        status: 'M',
        fresh: false,
        conflict: false,
        x: ' ',
        y: 'M',
        staged: false,
        add: 1,
        del: 0,
        binary: false,
    };
    let files = vec![file("src/b.rs", "src"), file("a.rs", ""), file("src/c.rs", "src")];
    let mut closed = HashSet::new();
    let rows = view::list_rows(&files, &closed);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0], view::ListRow::Folder { name: "".into(), count: 1, closed: false });
    assert_eq!(rows[2], view::ListRow::Folder { name: "src".into(), count: 2, closed: false });
    closed.insert("src".to_string());
    let rows = view::list_rows(&files, &closed);
    assert_eq!(rows.len(), 3);
    let only = vec![file("a.rs", "")];
    assert_eq!(view::list_rows(&only, &HashSet::new()), vec![view::ListRow::File(0)]);
}

#[test]
fn messages_and_errors_read_well() {
    assert_eq!(git::compose_message("  fix   the bug ", ""), "fix the bug\n");
    assert_eq!(git::compose_message("fix", "\n\nmore words\n"), "fix\n\nmore words\n");
    assert_eq!(git::classify("! [rejected] main -> main (fetch first)"), git::Kind::Rejected);
    assert_eq!(git::classify("fatal: Authentication failed for 'x'"), git::Kind::Auth);
    let text = "stash@{0}\0abc\0On main: nixa-switch: left main for dev\nstash@{1}\0def\0On x: other";
    let left = git::left_stash_for(text, "main").unwrap();
    assert_eq!((left.reference.as_str(), left.to.as_str()), ("stash@{0}", "dev"));
    assert!(git::left_stash_for(text, "dev").is_none());
    assert!(matches!(
        words::remote_plan(RemoteKind::Push, RemoteOpts::default(), true, None, true),
        Plan::Refuse(_)
    ));
    assert!(matches!(
        words::remote_plan(RemoteKind::Push, RemoteOpts::default(), false, Some("main"), false),
        Plan::Refuse(_)
    ));
}

#[test]
fn menus_turn_off_what_cannot_run() {
    let m = menus::push_menu("/r".into(), Some("main".into()), None, 0, 0, false, PullMode::Rebase);
    let lines = menus::push_lines(&m);
    let items: Vec<_> = lines
        .iter()
        .filter_map(|l| match l {
            menus::Line::Item(i) => Some(i),
            _ => None,
        })
        .collect();
    assert!(items.iter().all(|i| i.off.is_some()));
    let m = menus::push_menu("/r".into(), Some("main".into()), Some("origin/main".into()), 1, 0, true, PullMode::Merge);
    let lines = menus::push_lines(&m);
    let force = lines.iter().find_map(|l| match l {
        menus::Line::Item(i) if i.id == "push-force" => Some(i.clone()),
        _ => None,
    });
    let force = force.unwrap();
    assert!(force.danger && force.off.is_none());
    let f = FileEntry {
        path: "x/y.txt".into(),
        folder: "x".into(),
        status: 'M',
        fresh: false,
        conflict: false,
        x: ' ',
        y: 'M',
        staged: false,
        add: 0,
        del: 0,
        binary: false,
    };
    let mut fm = menus::file_menu("/r".into(), &f);
    fm.facts = Some(git::Facts { tracked: true, in_head: true });
    let ignore = menus::file_lines(&fm).into_iter().find_map(|l| match l {
        menus::Line::Item(i) if i.id == "ignore" => Some(i),
        _ => None,
    });
    assert!(ignore.unwrap().off.is_some());
    let r = menus::Rejected { remote: Some("origin".into()), branch: Some("main".into()), detail: String::new(), stale: false, confirm: false };
    assert!(menus::rejected_why(&r).starts_with("origin/main has commits"));
}

#[test]
fn small_helpers_hold() {
    assert_eq!(diff::next_change(&[3, 9, 20], Some(9), true), Some(20));
    assert_eq!(diff::next_change(&[3, 9, 20], Some(9), false), Some(3));
    assert!(git::has_markers("a
<<<<<<< HEAD
b
=======
c
>>>>>>> side
"));
    assert!(!git::has_markers("plain text
"));
    assert_eq!(git::exact_pattern("dir/a*b.txt").unwrap(), r"/dir/a\*b.txt");
    assert!(!git::masked_url("https://user:secret@example.com/x.git").contains("secret"));
    assert_eq!(git::base_name("a/b/c.rs"), "c.rs");
    assert_eq!(git::folder_of("a/b/c.rs"), "a/b");
    let (p, _) = patch::parse_head("@@ -1,3 +1,4 @@ fn main").unwrap();
    assert_eq!((p.old_start, p.old_count, p.new_start, p.new_count), (1, 3, 1, 4));
}

#[gpui::test]
fn files_stage_unstage_and_discard(cx: &mut TestAppContext) {
    let dir = repo("stage", true);
    fs::write(dir.join("a.txt"), "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n").unwrap();
    fs::write(dir.join("new.txt"), "new\n").unwrap();
    let (view, cx, _) = open(cx, &dir);
    assert_eq!(paths(&view, cx), vec![("a.txt".into(), false), ("new.txt".into(), false)]);
    assert!(shown(cx, "changes-file-0"));
    click(cx, "changes-stage-all");
    assert_eq!(paths(&view, cx), vec![("a.txt".into(), true), ("new.txt".into(), true)]);
    click(cx, "changes-unstage-0");
    assert_eq!(paths(&view, cx), vec![("a.txt".into(), false), ("new.txt".into(), true)]);
    click(cx, "changes-discard-0");
    assert!(shown(cx, "changes-ask"));
    click(cx, "changes-ask-yes");
    assert_eq!(paths(&view, cx), vec![("new.txt".into(), true)]);
    assert!(fs::read_to_string(dir.join("a.txt")).unwrap().starts_with("one\n"));
    click(cx, "changes-discard-all");
    click(cx, "changes-ask-yes");
    assert!(paths(&view, cx).is_empty());
    assert!(!dir.join("new.txt").exists());
    assert!(shown(cx, "changes-empty"));
    done(&[&dir]);
}

#[gpui::test]
fn commit_amend_and_no_hooks(cx: &mut TestAppContext) {
    let dir = repo("commit", true);
    fs::write(dir.join("src/b.txt"), "bee\nmore\n").unwrap();
    let (view, cx, seen) = open(cx, &dir);
    click(cx, "changes-stage-0");
    type_message(&view, cx, "add more");
    click(cx, "changes-commit");
    assert_eq!(head_subject(&dir), "add more");
    view.read_with(cx, |v, cx| {
        assert!(message_text(v, cx).is_empty());
        assert!(v.files.is_empty());
    });
    click(cx, "changes-amend");
    view.read_with(cx, |v, cx| {
        assert!(v.amend);
        assert_eq!(message_text(v, cx), "add more");
    });
    type_message(&view, cx, "add more words");
    click(cx, "changes-commit");
    assert_eq!(head_subject(&dir), "add more words");
    assert_eq!(sh(&dir, &["rev-list", "--count", "HEAD"]).trim(), "2");
    let hooks = dir.join(".git").join("nixa-test-hooks").join("pre-commit");
    fs::write(&hooks, "#!/bin/sh\necho lint failed >&2\nexit 1\n").unwrap();
    std::process::Command::new("chmod").args(["+x", &hooks.display().to_string()]).status().unwrap();
    fs::write(dir.join("a.txt"), "changed\n").unwrap();
    view.update(cx, refresh);
    cx.run_until_parked();
    type_message(&view, cx, "blocked");
    click(cx, "changes-commit-all");
    assert_eq!(head_subject(&dir), "add more words");
    assert!(view.read_with(cx, |v, _| v.hook_fail.is_some()));
    click(cx, "changes-no-hooks");
    assert_eq!(head_subject(&dir), "blocked");
    assert!(view.read_with(cx, |v, _| v.hook_fail.is_none()));
    assert!(notes(&seen).iter().any(|n| n.contains("ommit")));
    done(&[&dir]);
}

#[gpui::test]
fn commit_asks_who_first(cx: &mut TestAppContext) {
    let dir = repo("identity", false);
    fs::write(dir.join("a.txt"), "x\n").unwrap();
    let (view, cx, _) = open(cx, &dir);
    type_message(&view, cx, "who am i");
    click(cx, "changes-commit-all");
    assert!(view.read_with(cx, |v, _| matches!(v.popup, Some(Popup::Identity(_)))));
    assert_eq!(head_subject(&dir), "first");
    view.update(cx, |v, cx| {
        if let Some(Popup::Identity(b)) = v.popup.as_ref() {
            b.name.update(cx, |i, cx| i.set_value("New Person", cx));
            b.email.update(cx, |i, cx| i.set_value("new@example.com", cx));
        }
    });
    click(cx, "identity-use");
    assert_eq!(head_subject(&dir), "who am i");
    assert_eq!(sh(&dir, &["log", "-1", "--format=%an <%ae>"]).trim(), "New Person <new@example.com>");
    assert!(view.read_with(cx, |v, _| v.popup.is_none()));
    done(&[&dir]);
}

#[gpui::test]
fn diff_stages_a_hunk_and_single_lines(cx: &mut TestAppContext) {
    let dir = repo("diff", true);
    fs::write(dir.join("a.txt"), "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n").unwrap();
    let (view, cx, _) = open(cx, &dir);
    click(cx, "changes-file-0");
    assert!(shown(cx, "diff-view"));
    let diff = view.read_with(cx, |v, _| v.diff.clone()).unwrap();
    assert_eq!(diff.read_with(cx, |d, _| d.hunks.len()), 2);
    click(cx, "hunkbar-0");
    click(cx, "hunk-stage");
    let cached = sh(&dir, &["diff", "--cached"]);
    assert!(cached.contains("+ONE") && !cached.contains("+TEN"));
    let rows = diff.read_with(cx, |d, _| {
        d.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == diff::RowKind::Add && r.text == "TEN" && d.hunks[r.hunk.unwrap()].side == diff::Side::Unstaged)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    });
    let at = rows[0];
    diff.update(cx, |d, cx| diff_view::click_row(d, at, false, cx));
    cx.run_until_parked();
    click(cx, "hunk-lines-stage");
    let cached = sh(&dir, &["diff", "--cached"]);
    assert!(cached.contains("+TEN") && !cached.contains("-ten"), "cached: {cached}");
    let left = sh(&dir, &["diff"]);
    assert!(left.contains("-ten") && !left.contains("+TEN"), "left: {left}");
    click(cx, "diff-ignore-space");
    assert!(diff.read_with(cx, |d, _| d.ignore_space));
    click(cx, "diff-close");
    assert!(view.read_with(cx, |v, _| v.diff.is_none()));
    done(&[&dir]);
}

#[gpui::test]
fn push_pull_and_the_pull_check(cx: &mut TestAppContext) {
    let (dir, bare) = with_remote("remote");
    let other = clone_of(&bare, "remote-other");
    fs::write(other.join("a.txt"), "THEIRS\n").unwrap();
    sh(&other, &["commit", "-q", "-am", "theirs"]);
    sh(&other, &["push", "-q"]);
    fs::write(dir.join("a.txt"), "MINE\n").unwrap();
    sh(&dir, &["commit", "-q", "-am", "mine"]);
    let (view, cx, seen) = open(cx, &dir);
    assert!(view.read_with(cx, |v, _| v.has_remote));
    click(cx, "changes-push");
    assert!(view.read_with(cx, |v, _| matches!(v.popup, Some(Popup::Rejected(_)))), "notes: {:?}", notes(&seen));
    assert!(shown(cx, "push-rejected-view"));
    click(cx, "push-rejected-cancel");
    view.update(cx, |v, cx| open_push_menu(v, gpui::point(gpui::px(300.), gpui::px(80.)), cx));
    cx.run_until_parked();
    click(cx, "push-menu-check-pull");
    let check = view.read_with(cx, |v, _| match &v.popup {
        Some(Popup::Push { menu, .. }) => menu.check.clone().and_then(|c| c.check),
        _ => None,
    });
    let check = check.expect("the pull check ran");
    assert_eq!(check.behind, 1);
    assert!(!check.clean);
    assert_eq!(check.conflicts, vec!["a.txt".to_string()]);
    assert!(shown(cx, "push-menu-check"));
    done(&[&dir, &bare, &other]);
}

#[gpui::test]
fn pull_then_push_and_fetch(cx: &mut TestAppContext) {
    let (dir, bare) = with_remote("sync");
    let other = clone_of(&bare, "sync-other");
    fs::write(other.join("src/b.txt"), "theirs\n").unwrap();
    sh(&other, &["commit", "-q", "-am", "theirs"]);
    sh(&other, &["push", "-q"]);
    fs::write(dir.join("a.txt"), "mine\n").unwrap();
    sh(&dir, &["commit", "-q", "-am", "mine"]);
    let (view, cx, seen) = open(cx, &dir);
    click(cx, "changes-fetch");
    assert_eq!(view.read_with(cx, |v, _| v.header.behind), 1, "notes: {:?}", notes(&seen));
    click(cx, "changes-push");
    click(cx, "push-rejected-pull");
    let remote_head = sh(&bare, &["log", "-1", "--format=%s", "main"]);
    assert_eq!(remote_head.trim(), "mine", "notes: {:?}", notes(&seen));
    assert_eq!(view.read_with(cx, |v, _| (v.header.ahead, v.header.behind)), (0, 0));
    done(&[&dir, &bare, &other]);
}

#[gpui::test]
fn conflict_banner_and_sides(cx: &mut TestAppContext) {
    let dir = repo("conflict", true);
    sh(&dir, &["checkout", "-q", "-b", "side"]);
    fs::write(dir.join("a.txt"), "side\n").unwrap();
    sh(&dir, &["commit", "-q", "-am", "side"]);
    sh(&dir, &["checkout", "-q", "main"]);
    fs::write(dir.join("a.txt"), "main\n").unwrap();
    sh(&dir, &["commit", "-q", "-am", "main"]);
    let merged = std::process::Command::new("git").args(["merge", "side"]).current_dir(&dir).output().unwrap();
    assert!(!merged.status.success());
    let (view, cx, _) = open(cx, &dir);
    assert!(shown(cx, "changes-op"));
    assert_eq!(view.read_with(cx, |v, _| conflict_count(v)), 1);
    click(cx, "changes-theirs-0");
    assert!(shown(cx, "changes-ask"));
    click(cx, "changes-ask-yes");
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "side\n");
    assert_eq!(view.read_with(cx, |v, _| conflict_count(v)), 0);
    click(cx, "changes-op-continue");
    assert!(view.read_with(cx, |v, _| v.state.as_ref().and_then(|s| s.op).is_none()));
    assert_eq!(sh(&dir, &["rev-list", "--count", "HEAD"]).trim(), "4");
    done(&[&dir]);
}

#[gpui::test]
fn repo_picker_and_file_menu(cx: &mut TestAppContext) {
    let top = temp("picker");
    let one = top.join("one");
    let two = top.join("two");
    for d in [&one, &two] {
        fs::create_dir_all(d).unwrap();
        sh(d, &["init", "-q", "-b", "main"]);
        setup(d, true);
        fs::write(d.join("f.txt"), "x\n").unwrap();
    }
    fs::write(two.join("g.txt"), "y\n").unwrap();
    let (view, cx, seen) = open(cx, &top);
    assert_eq!(view.read_with(cx, |v, _| v.repos.len()), 2);
    view.update(cx, |v, cx| open_repo_menu(v, gpui::point(gpui::px(10.), gpui::px(30.)), cx));
    click(cx, "repo-menu-1");
    let root = view.read_with(cx, |v, _| v.root.clone()).unwrap();
    assert!(root.ends_with("/two"));
    assert_eq!(paths(&view, cx).len(), 2);
    view.update(cx, |v, cx| open_file_menu(v, "g.txt", gpui::point(gpui::px(40.), gpui::px(120.)), cx));
    cx.run_until_parked();
    assert!(shown(cx, "file-menu"));
    click(cx, "file-menu-ignore");
    click(cx, "file-menu-ask-yes");
    let exclude = fs::read_to_string(two.join(".git/info/exclude")).unwrap();
    assert!(exclude.lines().any(|l| l == "/g.txt"), "{exclude}");
    assert_eq!(paths(&view, cx), vec![("f.txt".into(), false)]);
    view.update(cx, |v, cx| open_file_menu(v, "f.txt", gpui::point(gpui::px(40.), gpui::px(120.)), cx));
    cx.run_until_parked();
    click(cx, "file-menu-open");
    assert!(seen.borrow().iter().any(|e| matches!(e, ChangesEvent::OpenFile { path, .. } if path == "f.txt")));
    done(&[&top]);
}

#[gpui::test]
fn left_changes_come_back(cx: &mut TestAppContext) {
    let dir = repo("left", true);
    fs::write(dir.join("a.txt"), "kept aside\n").unwrap();
    sh(&dir, &["stash", "push", "-q", "-m", "nixa-switch: left main for dev"]);
    let (view, cx, _) = open(cx, &dir);
    assert!(shown(cx, "changes-left-stash"));
    click(cx, "changes-left-back");
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "kept aside\n");
    assert!(view.read_with(cx, |v, _| v.left.is_none()));
    assert!(sh(&dir, &["stash", "list"]).trim().is_empty());
    done(&[&dir]);
}

#[gpui::test]
fn the_first_diff_opens_new_files_and_busy_confirmations_wait(cx: &mut TestAppContext) {
    let dir = repo("first-diff", true);
    let (view, cx, _) = open(cx, &dir);
    cx.update(|_, cx| show_first_diff(&view, cx));
    assert!(view.read_with(cx, |v, _| !v.show_diff && v.diff.is_none()));
    fs::write(dir.join("new.txt"), "new\n").unwrap();
    cx.update(|_, cx| refresh_view(&view, cx));
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.diff.is_none()));
    cx.update(|_, cx| show_first_diff(&view, cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.open.clone()).as_deref(), Some("new.txt"));

    view.update(cx, |v, cx| {
        ask_discard(v, vec!["new.txt".into()], "new.txt", cx);
        v.busy = true;
        cx.notify();
    });
    click(cx, "changes-ask-yes");
    assert!(view.read_with(cx, |v, _| v.ask.is_some()));
    assert!(dir.join("new.txt").is_file());
    view.update(cx, |v, cx| {
        v.busy = false;
        cx.notify();
    });
    click(cx, "changes-ask-yes");
    assert!(!dir.join("new.txt").exists());
    done(&[&dir]);
}

#[gpui::test]
fn graph_loads_and_clears_with_the_repository(cx: &mut TestAppContext) {
    let dir = repo("embedded-graph", true);
    let other = repo("embedded-graph-other", true);
    sh(&other, &["commit", "-q", "--allow-empty", "-m", "second repository"]);
    let plain = temp("embedded-graph-plain");
    let (view, cx, _) = open(cx, &dir);
    click(cx, "changes-tab-graph");
    let graph = view.read_with(cx, |v, _| v.graph.clone()).unwrap();
    graph.read_with(cx, |graph, _| {
        assert_eq!(graph.commits.len(), 1);
        assert_eq!(graph.commits[0].subject, "first");
    });
    assert!(shown(cx, "changes-history-graph"));
    let folder = plain.display().to_string();
    view.update(cx, |v, cx| change_place(v, Some(folder.clone()), Some(folder), cx));
    cx.run_until_parked();
    assert!(graph.read_with(cx, |graph, _| graph.repo.is_none() && graph.commits.is_empty()));
    assert!(shown(cx, "changes-no-repo"));
    assert!(shown(cx, "changes-init"));
    let folder = other.display().to_string();
    view.update(cx, |v, cx| change_place(v, Some(folder.clone()), Some(folder), cx));
    cx.run_until_parked();
    graph.read_with(cx, |graph, _| {
        assert_eq!(graph.repo.as_deref().and_then(|p| p.canonicalize().ok()), Some(other.canonicalize().unwrap()));
        assert_eq!(graph.commits[0].subject, "second repository");
    });
    assert!(shown(cx, "changes-history-graph"));
    done(&[&dir, &other, &plain]);
}

#[gpui::test]
fn a_disabled_button_with_a_click_handler_does_not_fire(cx: &mut TestAppContext) {
    use gpui::prelude::*;
    use nixa_ui::button::{ButtonKind, button, disabled};

    struct Probe {
        off: bool,
        clicked: bool,
    }

    impl gpui::Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let t = nixa_ui::theme(cx).clone();
            gpui::div().child(
                disabled(button(&t, "disabled-probe", "Run", ButtonKind::Primary), self.off)
                    .debug_selector(|| "disabled-probe".into())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.clicked = true;
                        cx.notify();
                    })),
            )
        }
    }

    cx.update(nixa_ui::init);
    let (view, cx) = cx.add_window_view(|_, _| Probe { off: true, clicked: false });
    click(cx, "disabled-probe");
    assert!(!view.read_with(cx, |view, _| view.clicked));
    view.update(cx, |view, cx| {
        view.off = false;
        cx.notify();
    });
    click(cx, "disabled-probe");
    assert!(view.read_with(cx, |view, _| view.clicked));
}
