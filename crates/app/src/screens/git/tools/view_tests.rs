use super::blame::{BlameView, Open as BlameOpen};
use super::conflict::{ConflictView, Open as ConflictOpen};
use super::host::{self, Host, Notice};
use super::rebase::{Open as RebaseOpen, RebaseView};
use super::testrepo::{Repo, repo};
use gpui::{App, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use std::sync::atomic::{AtomicUsize, Ordering};

static NOTICES: AtomicUsize = AtomicUsize::new(0);

fn count_notice(_: &Notice, _: &mut App) -> bool {
    NOTICES.fetch_add(1, Ordering::SeqCst);
    false
}

fn test_host() -> Host {
    Host {
        notify: count_notice,
        ..host::host()
    }
}

fn click(vcx: &mut VisualTestContext, selector: &str) {
    vcx.run_until_parked();
    let name: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let bounds = vcx
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn blame_repo() -> (Repo, String) {
    let r = repo();
    let first = r.commit("a.txt", "one\ntwo\n", "First");
    r.commit("a.txt", "one\nTWO\n", "Second");
    (r, first)
}

fn open_blame<'a>(
    cx: &'a mut TestAppContext,
    r: &Repo,
) -> (Entity<BlameView>, &'a mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let open = BlameOpen {
        repo: Some(r.path().to_path_buf()),
        file: Some("a.txt".into()),
        rev: String::new(),
        line: Some(2),
    };
    let (view, vcx) =
        cx.add_window_view(move |window, cx| BlameView::new(open, test_host(), window, cx));
    vcx.run_until_parked();
    (view, vcx)
}

#[gpui::test]
fn blame_window_walks_back_in_time(cx: &mut TestAppContext) {
    let (r, first) = blame_repo();
    let (view, vcx) = open_blame(cx, &r);
    view.read_with(vcx, |v, _| {
        assert_eq!(v.parsed.lines.len(), 2);
        assert_eq!(v.selected, Some(1));
        assert_eq!(v.stats(), "2 lines · 2 commits");
        assert_eq!(v.heading(), "Who changed each line: a.txt");
    });
    click(vcx, "blame-before");
    view.read_with(vcx, |v, _| {
        assert_eq!(v.rev, first);
        assert_eq!(v.stack.len(), 1);
        assert_eq!(v.parsed.lines[1].text, "two");
    });
    click(vcx, "blame-back");
    view.read_with(vcx, |v, _| {
        assert!(v.rev.is_empty());
        assert!(v.stack.is_empty());
        assert_eq!(v.selected, Some(1));
    });
    click(vcx, "blame-line-0");
    view.read_with(vcx, |v, _| assert_eq!(v.selected, Some(0)));
    click(vcx, "blame-show");
    view.read_with(vcx, |v, _| {
        assert_eq!(v.note.as_deref(), Some(host::NO_HISTORY))
    });
    vcx.simulate_keystrokes("down");
    view.read_with(vcx, |v, _| assert_eq!(v.selected, Some(1)));
}

#[gpui::test]
fn blame_window_finds_text(cx: &mut TestAppContext) {
    let (r, _) = blame_repo();
    let (view, vcx) = open_blame(cx, &r);
    view.update(vcx, |v, cx| {
        v.find.update(cx, |i, cx| i.set_value(":9", cx));
        v.run_find(cx);
    });
    view.read_with(vcx, |v, _| {
        assert_eq!(v.selected, Some(1));
        assert_eq!(v.note.as_deref(), Some("This file has 2 lines."));
    });
}

fn conflicted() -> Repo {
    let r = repo();
    r.commit("a.txt", "one\n", "base");
    r.commit("b.txt", "bee\n", "base b");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.commit("a.txt", "feature\n", "feature a");
    r.commit("b.txt", "feature b\n", "feature b");
    r.git(&["checkout", "-q", "main"]);
    r.commit("a.txt", "main\n", "main a");
    r.commit("b.txt", "main b\n", "main b");
    assert!(!r.try_git(&["merge", "-q", "feature"]));
    r
}

#[gpui::test]
fn conflict_window_saves_and_moves_on(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let r = conflicted();
    let open = ConflictOpen {
        repo: Some(r.path().to_path_buf()),
        file: Some("a.txt".into()),
    };
    let (view, vcx) =
        cx.add_window_view(move |window, cx| ConflictView::new(open, test_host(), window, cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, cx| {
        assert_eq!(v.conflicts.len(), 1);
        assert!(!v.can_save(cx));
        assert_eq!(v.heading(), "Resolve conflict: a.txt");
        assert_eq!(v.op(), Some(super::state::Op::Merge));
    });
    click(vcx, "cf-use-0-theirs");
    view.read_with(vcx, |v, cx| {
        assert!(v.can_save(cx));
        assert!(v.has_progress());
        assert_eq!(v.result.read(cx).value(), "feature\n");
        assert_eq!(v.status(cx), "All conflicts chosen. Ready to save.");
    });
    click(vcx, "cf-save");
    assert_eq!(r.read("a.txt"), "feature\n");
    view.read_with(vcx, |v, _| {
        assert_eq!(v.file.as_deref(), Some("b.txt"));
        assert_eq!(
            v.info.as_deref(),
            Some("Saved a.txt. 1 more file to fix. Now showing b.txt.")
        );
        assert!(!v.has_progress());
    });
    view.update(vcx, |v, cx| {
        v.result.update(cx, |a, cx| a.set_value("by hand\n", cx));
        v.hand_edited = true;
    });
    click(vcx, "cf-use-0-ours");
    view.read_with(vcx, |v, _| {
        assert_eq!(
            v.pending,
            Some(super::conflict::view::Pending::Choice(
                0,
                super::conflict::model::Choice::Ours
            ))
        );
    });
    click(vcx, "cf-ask-no");
    click(vcx, "cf-save");
    assert_eq!(r.read("b.txt"), "by hand\n");
    view.read_with(vcx, |v, _| {
        assert!(v.done);
        assert_eq!(
            v.info.as_deref(),
            Some("Saved b.txt. All conflicts are fixed. Press Continue in the Changes panel to finish the merge.")
        );
    });
}

#[gpui::test]
fn conflict_window_asks_before_closing(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let r = conflicted();
    let open = ConflictOpen {
        repo: Some(r.path().to_path_buf()),
        file: Some("a.txt".into()),
    };
    let (view, vcx) =
        cx.add_window_view(move |window, cx| ConflictView::new(open, test_host(), window, cx));
    vcx.run_until_parked();
    click(vcx, "cf-use-0-both");
    click(vcx, "cf-ai-explain");
    view.read_with(vcx, |v, _| {
        assert_eq!(
            v.ai.error.as_deref(),
            Some(format!("AI failed: {}", host::NO_AI).as_str())
        );
    });
    click(vcx, "cf-close");
    view.read_with(vcx, |v, _| {
        assert_eq!(v.pending, Some(super::conflict::view::Pending::Close))
    });
    view.update_in(vcx, |v, window, cx| window.focus(&v.focus_handle(cx)));
    vcx.simulate_keystrokes("escape");
    view.read_with(vcx, |v, _| assert_eq!(v.pending, None));
}

#[gpui::test]
fn rebase_window_drops_a_commit(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let r = repo();
    let base = r.commit("a.txt", "zero\n", "base");
    let b = r.commit("b.txt", "b\n", "add b");
    let c = r.commit("c.txt", "c\n", "add c");
    let open = RebaseOpen {
        repo: Some(r.path().to_path_buf()),
        base: Some(base.clone()),
    };
    let (view, vcx) =
        cx.add_window_view(move |window, cx| RebaseView::new(open, test_host(), window, cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| {
        assert_eq!(v.rows.len(), 2);
        assert!(v.editable());
    });
    click(vcx, &format!("rebase-act-{b}-drop"));
    click(vcx, &format!("rebase-act-{c}-reword"));
    view.update(vcx, |v, cx| {
        let e = &v.editors[&(c.clone(), super::rebase::view::EditorKind::Reword)];
        e.summary.update(cx, |i, cx| i.set_value("add sea", cx));
    });
    click(vcx, "rebase-check");
    view.read_with(vcx, |v, _| {
        let (_, pre) = v.preview.as_ref().expect("a check");
        assert!(pre.clean);
    });
    click(vcx, "rebase-apply");
    view.read_with(vcx, |v, _| assert!(v.confirm));
    let before = NOTICES.load(Ordering::SeqCst);
    click(vcx, "rebase-confirm-yes");
    view.read_with(vcx, |v, _| {
        let done = v.finished.as_ref().expect("finished");
        assert_eq!(done.note.title, "Commits changed");
        assert!(!v.editable());
    });
    assert!(NOTICES.load(Ordering::SeqCst) > before);
    assert_eq!(
        r.git(&["log", "--format=%s", &format!("{base}..HEAD")]),
        "add sea\n"
    );
}

#[gpui::test]
fn rebase_window_explains_a_missing_upstream(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let r = repo();
    r.commit("a.txt", "zero\n", "base");
    let open = RebaseOpen {
        repo: Some(r.path().to_path_buf()),
        base: None,
    };
    let (view, vcx) =
        cx.add_window_view(move |window, cx| RebaseView::new(open, test_host(), window, cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| {
        let plan = v.plan.as_ref().unwrap();
        assert_eq!(plan.note.kind, Some(super::rebase::plan::Kind::NoUpstream));
        assert!(!v.editable());
    });
}
