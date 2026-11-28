use super::branch_ops;
use super::branches::{AskKind, BranchesView, Place};
use super::commit_menu::{self, CommitMenu, Form, Props};
use super::commit_ops::{self, Expect, ResetMode};
use super::graph::{self, GraphView};
use super::log::{self, LogQuery};
use super::repo::{self, TempDir};
use super::run;
use super::stash::StashView;
use super::stash_ops::{self, DiffKind};
use super::words;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use std::fs;
use std::path::{Path, PathBuf};

fn git(root: &Path, args: &[&str]) -> String {
    let out = run::git(root, args);
    assert!(out.ok(), "git {args:?} failed: {}", out.said());
    out.stdout
}

fn new_repo() -> (TempDir, PathBuf) {
    let dir = repo::temp_dir().unwrap();
    let root = dir.0.clone();
    git(&root, &["init", "-q", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Test User"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("core.hooksPath", "no-hooks"),
    ] {
        git(&root, &["config", k, v]);
    }
    (dir, root)
}

fn commit(root: &Path, file: &str, text: &str, message: &str) -> String {
    fs::write(root.join(file), text).unwrap();
    git(root, &["add", "--", file]);
    git(root, &["commit", "-q", "-m", message]);
    git(root, &["rev-parse", "HEAD"]).trim().to_string()
}

fn base() -> (TempDir, PathBuf) {
    let (dir, root) = new_repo();
    commit(&root, "a.txt", "one\n", "first");
    commit(&root, "a.txt", "one\ntwo\n", "second");
    (dir, root)
}

#[test]
fn words_and_names() {
    assert_eq!(words::branch_name_problem("good/name"), None);
    assert!(words::branch_name_problem("bad name").is_some());
    assert!(words::branch_name_problem("-x").is_some());
    assert_eq!(words::ago_at(100, 130), "just now");
    assert_eq!(words::ago_at(1, 1 + 604800), "1 week ago");
    assert_eq!(words::ago_at(1, 1 + 3 * 86400), "3 days ago");
    assert_eq!(words::short_ago_at(0, 300), "5m ago");
    assert!(words::is_sha_like("abc1234"));
    assert!(!words::is_sha_like("xyz"));
    assert_eq!(words::plural(1, "file"), "1 file");
    assert_eq!(words::plural(3, "file"), "3 files");
}

#[test]
fn log_reads_lanes_and_pages() {
    let (_d, root) = base();
    git(&root, &["checkout", "-q", "-b", "side", "HEAD~1"]);
    commit(&root, "b.txt", "b\n", "side work");
    git(&root, &["checkout", "-q", "main"]);
    git(&root, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    let q = LogQuery { limit: 50, all: true, ..LogQuery::default() };
    let mut list = log::read_log(&root, &log::log_args(&q)).unwrap();
    assert_eq!(list.len(), 4);
    assert_eq!(list[0].subject, "merge side");
    assert_eq!(list[0].parents.len(), 2);
    assert!(list[0].refs.iter().any(|r| r.head && r.name == "main"));
    let lanes = log::set_rows(&mut list, false);
    assert_eq!(lanes, 2);
    assert!(list.iter().any(|c| c.row.lane == 1));
    let page = log::read_log(&root, &log::log_args(&LogQuery { limit: 2, skip: 2, all: true, ..LogQuery::default() })).unwrap();
    assert_eq!(page.len(), 2);
    let plan = log::search_plan("side").unwrap();
    let grep = log::read_log(&root, &log::log_args(&LogQuery { limit: 50, grep: Some(plan.query.clone()), all: true, ..LogQuery::default() })).unwrap();
    let found = log::merge_search(Vec::new(), vec![grep], 10);
    assert_eq!(found.len(), 2);
    let short = list[3].short.clone();
    assert_eq!(log::find_by_id(&root, &short).len(), 1);
    assert_eq!(log::file_history(&root, "b.txt", 10).unwrap().len(), 1);
    assert_eq!(log::visible(&list, "second", false).len(), 1);
    assert!(graph::rail_width(1) >= 24.0);
}

#[test]
fn branch_ops_work() {
    let (_d, root) = base();
    assert!(branch_ops::create_branch(&root, "feature", None, false).ok);
    assert_eq!(branch_ops::create_branch(&root, "feature", None, false).kind, "exists");
    let list = branch_ops::list_branches(&root).unwrap();
    assert!(list.iter().any(|b| b.name == "main" && b.current));
    assert!(list.iter().any(|b| b.name == "feature" && !b.current));
    assert!(branch_ops::checkout_branch(&root, "feature").ok);
    commit(&root, "c.txt", "c\n", "feature work");
    assert!(branch_ops::checkout_branch(&root, "main").ok);
    let plan = branch_ops::merge_plan(&root, "feature", false).unwrap();
    assert!(plan.fast_forward);
    assert_eq!(plan.incoming, 1);
    let cmp = branch_ops::compare_counts(&root, "feature", "main").unwrap();
    assert_eq!((cmp.ahead, cmp.behind), (1, 0));
    assert!(branch_ops::create_branch(&root, "old", Some("feature"), false).ok);
    let merged = branch_ops::merge_into(&root, "feature", false, Some(&plan.head), Some(&plan.theirs));
    assert!(merged.ok, "{merged:?}");
    assert!(root.join("c.txt").exists());
    assert!(branch_ops::rename_branch(&root, "old", "older").ok);
    git(&root, &["checkout", "-q", "-b", "lost"]);
    commit(&root, "d.txt", "d\n", "lost work");
    git(&root, &["checkout", "-q", "main"]);
    assert_eq!(branch_ops::delete_branch(&root, "lost", false).kind, "unmerged");
    assert!(branch_ops::delete_branch(&root, "lost", true).ok);
    assert_eq!(branch_ops::delete_branch(&root, "main", true).kind, "inUse");
}

#[test]
fn switch_with_changes_stashes_them() {
    let (_d, root) = base();
    assert!(branch_ops::create_branch(&root, "other", None, false).ok);
    fs::write(root.join("a.txt"), "changed\n").unwrap();
    assert_eq!(branch_ops::switch_changes(&root).unwrap(), 1);
    let r = branch_ops::stash_then_switch(&root, "main", "other", false);
    assert!(r.ok, "{r:?}");
    assert!(r.stashed);
    assert_eq!(repo::current_branch(&root).as_deref(), Some("other"));
    let stashes = stash_ops::list(&root).unwrap();
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.starts_with(branch_ops::SWITCH_TAG));
}

#[test]
fn stash_ops_work() {
    let (_d, root) = base();
    assert_eq!(stash_ops::push(&root, "", true, false).kind, "nothing");
    fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    fs::write(root.join("new.txt"), "fresh\n").unwrap();
    assert!(stash_ops::push(&root, "my\nwork", true, false).ok);
    let list = stash_ops::list(&root).unwrap();
    assert_eq!(list.len(), 1);
    let s = &list[0];
    assert_eq!(s.message, "my work");
    assert!(s.untracked);
    let files = stash_ops::files(&root, &s.reference).unwrap();
    assert!(files.iter().any(|f| f.path == "a.txt" && f.status == 'M'));
    assert!(files.iter().any(|f| f.path == "new.txt" && f.untracked));
    let rows = stash_ops::diff_rows(&stash_ops::diff(&root, &s.reference, "a.txt", Some(false)).unwrap());
    assert!(rows.iter().any(|r| r.kind == DiffKind::Add && r.text == "three" && r.number == Some(3)));
    let rows = stash_ops::diff_rows(&stash_ops::diff(&root, &s.reference, "new.txt", Some(true)).unwrap());
    assert!(rows.iter().any(|r| r.kind == DiffKind::Add && r.text == "fresh"));
    assert!(stash_ops::diff(&root, &s.reference, "../x", None).is_err());
    assert_eq!(stash_ops::apply(&root, &s.reference, false, false, Some("0000000")).kind, "stashMoved");
    assert!(stash_ops::apply(&root, &s.reference, true, false, Some(&s.sha)).ok);
    assert!(stash_ops::list(&root).unwrap().is_empty());
    assert!(root.join("new.txt").exists());
    assert!(stash_ops::push(&root, "again", true, false).ok);
    let s = stash_ops::list(&root).unwrap().remove(0);
    assert!(stash_ops::drop(&root, &s.reference, Some(&s.sha)).ok);
    assert!(stash_ops::list(&root).unwrap().is_empty());
}

#[test]
fn commit_ops_work() {
    let (_d, root) = base();
    let head = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    let first = git(&root, &["rev-parse", "HEAD~1"]).trim().to_string();
    let info = commit_ops::commit_info(&root, &head).unwrap();
    assert!(info.is_head && info.on_head && !info.pushed && !info.is_merge);
    assert_eq!(info.subject, "second");
    assert!(commit_ops::create_tag(&root, "v1", &first, "first tag").ok);
    assert_eq!(git(&root, &["cat-file", "-t", "v1"]).trim(), "tag");
    assert!(commit_ops::create_branch_at(&root, "from-first", &first, false).ok);
    let p = commit_ops::reset_preview(&root, &first).unwrap();
    assert_eq!(p.leaving, 1);
    let expect = Expect { head: Some(p.head.clone()), branch: Some(p.branch.clone()) };
    assert!(commit_ops::reset_to(&root, &first, ResetMode::Mixed, &expect, &[]).ok);
    assert_eq!(git(&root, &["rev-parse", "HEAD"]).trim(), first);
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "one\ntwo\n");
    git(&root, &["commit", "-q", "-am", "second again"]);
    let info = commit_ops::commit_info(&root, "HEAD").unwrap();
    assert!(commit_ops::undo_last_commit(&root, &info.expect()).ok);
    assert_eq!(git(&root, &["rev-parse", "HEAD"]).trim(), first);
    git(&root, &["commit", "-q", "-m", "third"]);
    commit(&root, "b.txt", "b\n", "fourth");
    let third = git(&root, &["rev-parse", "HEAD~1"]).trim().to_string();
    let info = commit_ops::commit_info(&root, &third).unwrap();
    let r = commit_ops::reword_commit(&root, &third, "third, better", "with a body", &info.expect());
    assert!(r.ok, "{r:?}");
    assert_eq!(git(&root, &["log", "-1", "--format=%s", "HEAD~1"]).trim(), "third, better");
    assert_eq!(git(&root, &["log", "-1", "--format=%s", "HEAD"]).trim(), "fourth");
    let info = commit_ops::commit_info(&root, "HEAD").unwrap();
    let r = commit_ops::revert_commit(&root, &info.sha, 1, &info.expect());
    assert!(r.ok, "{r:?}");
    assert!(!root.join("b.txt").exists());
    git(&root, &["checkout", "-q", "from-first"]);
    let info = commit_ops::commit_info(&root, "main~1").unwrap();
    let r = commit_ops::cherry_pick(&root, &info.sha, true, 1, &info.expect());
    assert!(r.ok, "{r:?}");
    assert!(git(&root, &["log", "-1", "--format=%B"]).contains("cherry picked from commit"));
    assert_eq!(commit_ops::compose_message("  sum ", "\n\n body \n"), "sum\n\n body\n");
}

fn graph(cx: &mut TestAppContext, root: PathBuf) -> (Entity<GraphView>, &mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let (view, vcx) = cx.add_window_view(move |_, cx| GraphView::new(Some(root), cx));
    vcx.run_until_parked();
    (view, vcx)
}

fn click(vcx: &mut VisualTestContext, name: &str) {
    vcx.run_until_parked();
    let key: &'static str = Box::leak(name.to_string().into_boxed_str());
    let bounds = vcx.debug_bounds(key).unwrap_or_else(|| panic!("no {name} on screen"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn graph_loads_searches_and_picks(cx: &mut TestAppContext) {
    let (_d, root) = base();
    let (view, vcx) = graph(cx, root.clone());
    view.read_with(vcx, |v, _| {
        assert_eq!(v.commits.len(), 2);
        assert!(!v.more);
        assert!(!v.busy);
    });
    let short = view.read_with(vcx, |v, _| v.commits[1].short.clone());
    click(vcx, &format!("commit-{short}"));
    view.read_with(vcx, |v, _| assert_eq!(v.selected.as_deref(), Some(v.commits[1].sha.as_str())));
    view.update(vcx, |v, cx| v.run_search("first", false, false, None, cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| {
        assert!(v.search.is_some());
        assert_eq!(v.commits.len(), 1);
        assert_eq!(v.commits[0].subject, "first");
    });
    click(vcx, "history-search-results");
    view.read_with(vcx, |v, _| {
        assert!(v.search.is_none());
        assert_eq!(v.commits.len(), 2);
    });
    view.update(vcx, |v, cx| v.show_file_history("a.txt", cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| assert_eq!(v.file.as_deref(), Some("a.txt")));
    click(vcx, "history-file-filter");
    view.read_with(vcx, |v, _| assert!(v.file.is_none()));
}

#[gpui::test]
fn graph_loads_more(cx: &mut TestAppContext) {
    let (_d, root) = new_repo();
    commit(&root, "a.txt", "0", "start");
    let tree = git(&root, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    let mut parent = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    for i in 0..(log::PAGE + 20) {
        let msg = format!("step {i}");
        parent = run::git(&root, &["commit-tree", &tree, "-p", &parent, "-m", &msg]).line();
    }
    git(&root, &["update-ref", "refs/heads/main", &parent]);
    let (view, vcx) = graph(cx, root.clone());
    view.read_with(vcx, |v, _| {
        assert_eq!(v.commits.len(), log::PAGE);
        assert!(v.more);
    });
    view.update(vcx, |v, cx| v.load_more(cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| {
        assert_eq!(v.commits.len(), log::PAGE + 21);
        assert!(!v.more);
    });
}

#[gpui::test]
fn branches_view_lists_and_opens_actions(cx: &mut TestAppContext) {
    let (_d, root) = base();
    assert!(branch_ops::create_branch(&root, "feature", None, false).ok);
    cx.update(nixa_ui::init);
    let r = root.clone();
    let (view, vcx) = cx.add_window_view(move |window, cx| BranchesView::new(Some(r), window, cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| {
        let shown = v.visible();
        assert_eq!(shown.local.len(), 2);
        assert!(shown.local[0].current);
    });
    click(vcx, "br-more-1");
    view.read_with(vcx, |v, _| assert_eq!(v.open.as_deref(), Some("feature")));
    click(vcx, "br-del-1");
    view.read_with(vcx, |v, _| assert_eq!(v.confirm, Some(("feature".to_string(), false))));
    click(vcx, "br-del-yes-1");
    view.read_with(vcx, |v, _| {
        assert!(v.note.as_ref().is_some_and(|n| n.ok));
        assert_eq!(v.visible().local.len(), 1);
    });
    assert!(branch_ops::create_branch(&root, "next", None, false).ok);
    view.update(vcx, |v, cx| v.load(cx));
    vcx.run_until_parked();
    let next = view.read_with(vcx, |v, _| v.visible().local.into_iter().find(|b| b.name == "next").unwrap());
    view.update_in(vcx, |v, window, cx| v.open_ask(AskKind::Rename, &next, Place::Local, None, window, cx));
    view.read_with(vcx, |v, _| assert_eq!(v.rename_text, "next"));
    view.update_in(vcx, |v, window, cx| {
        v.rename_text = "renamed".into();
        v.rename(window, cx)
    });
    vcx.run_until_parked();
    assert!(branch_ops::list_branches(&root).unwrap().iter().any(|b| b.name == "renamed"));
}

#[gpui::test]
fn stash_view_shows_files_and_diff(cx: &mut TestAppContext) {
    let (_d, root) = base();
    fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    assert!(stash_ops::push(&root, "saved", true, false).ok);
    cx.update(nixa_ui::init);
    let r = root.clone();
    let (view, vcx) = cx.add_window_view(move |window, cx| StashView::new(Some(r), window, cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| {
        assert_eq!(v.stashes.as_ref().map(Vec::len), Some(1));
        assert!(v.selected.is_some());
        assert_eq!(v.files.len(), 1);
        assert_eq!(v.file.as_deref(), Some("a.txt"));
        assert!(v.rows.iter().any(|r| r.kind == DiffKind::Add));
    });
    click(vcx, "stash-drop");
    click(vcx, "stash-drop-yes");
    view.read_with(vcx, |v, _| {
        assert!(v.note.as_ref().is_some_and(|n| n.ok));
        assert_eq!(v.stashes.as_ref().map(Vec::len), Some(0));
    });
}

#[gpui::test]
fn commit_menu_gates_and_forms(cx: &mut TestAppContext) {
    let (_d, root) = base();
    let head = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    let first = git(&root, &["rev-parse", "HEAD~1"]).trim().to_string();
    cx.update(nixa_ui::init);
    let props = Props { repo: root.clone(), sha: first.clone(), short: words::short_sha(&first), subject: "first".into(), author: "Test User".into(), form: None };
    let (view, vcx) = cx.add_window_view(move |window, cx| CommitMenu::new(props, window, cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| {
        assert!(v.info.is_some());
        let items: Vec<_> = v.items().into_iter().flatten().collect();
        let tip = |id: &str| items.iter().find(|i| i.id == id).unwrap().tip.clone();
        assert!(tip("copy").is_none());
        assert!(tip("undo").is_some());
        assert!(tip("reset").is_none());
        assert!(tip("web").is_some());
        assert!(tip("pick").is_some());
    });
    click(vcx, "cm-reset");
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| {
        assert_eq!(v.form, Some(Form::Reset));
        assert!(matches!(&v.preview, Some(Ok(p)) if p.leaving == 1));
        assert!(v.reset_ready());
    });
    click(vcx, "cm-reset-soft");
    view.read_with(vcx, |v, _| assert_eq!(v.mode, ResetMode::Soft));
    click(vcx, "cm-reset-run");
    assert_eq!(git(&root, &["rev-parse", "HEAD"]).trim(), first);
    let staged = git(&root, &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.trim(), "a.txt");
    let _ = head;
    let names: Vec<&str> = commit_menu::MODES.iter().map(|m| m.1).collect();
    assert_eq!(names, ["soft", "mixed", "hard"]);
}
