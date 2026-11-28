use super::model::*;
use super::plan::{self, Kind};
use crate::screens::git::tools::state::{Op, read_state};
use crate::screens::git::tools::testrepo::{Repo, repo};

fn commit(sha: &str, subject: &str, body: &str) -> Commit {
    Commit {
        sha: sha.into(),
        short: short(sha),
        subject: subject.into(),
        body: body.into(),
        ..Commit::default()
    }
}

fn step(sha: &str, action: Action) -> Step {
    Step {
        sha: sha.into(),
        action,
        message: None,
    }
}

const A: &str = "aaaaaaa1";
const B: &str = "bbbbbbb2";
const C: &str = "ccccccc3";

fn three() -> Vec<Commit> {
    vec![
        commit(A, "one", ""),
        commit(B, "two", "more about two"),
        commit(C, "three", ""),
    ]
}

#[test]
fn messages_are_built_and_split() {
    let m = Message {
        summary: " a\n b ".into(),
        body: "\n\nline\r\nnext  \n".into(),
    };
    assert_eq!(compose(&m), "a b\n\nline\nnext\n");
    assert_eq!(compose(&Message::default()), "");
    let s = split_message("Head\n\n\nBody one\nBody two\n");
    assert_eq!(s.summary, "Head");
    assert_eq!(s.body, "Body one\nBody two");
    assert_eq!(shell_quote("it's"), "'it'\\''s'");
    let line = amend_line(&Message {
        summary: "Fix it's".into(),
        body: "x".into(),
    });
    assert_eq!(
        line,
        format!("exec printf '%s\\n' 'Fix it'\\''s' '' 'x' | {AMEND_COMMAND}")
    );
    assert_eq!(one_line("a\nb\tc"), "a b c");
}

#[test]
fn steps_are_checked() {
    let commits = three();
    let ok = vec![
        step(A, Action::Pick),
        step(B, Action::Squash),
        step(C, Action::Drop),
    ];
    assert!(validate(&ok, Some(&commits)).ok());
    let first_join = vec![
        step(A, Action::Fixup),
        step(B, Action::Pick),
        step(C, Action::Pick),
    ];
    let check = validate(&first_join, Some(&commits));
    assert_eq!(check.problems[0].index, Some(0));
    let all_dropped = vec![
        step(A, Action::Drop),
        step(B, Action::Drop),
        step(C, Action::Drop),
    ];
    assert_eq!(
        validate(&all_dropped, None).problem().unwrap(),
        "Keep at least one commit."
    );
    let mut reword = ok.clone();
    reword[0] = Step {
        sha: A.into(),
        action: Action::Reword,
        message: Some(Message::default()),
    };
    assert_eq!(
        validate(&reword, None).problem().unwrap(),
        "Write a summary for aaaaaaa."
    );
    let short_list = vec![step(A, Action::Pick)];
    assert!(
        validate(&short_list, Some(&commits))
            .problem()
            .unwrap()
            .contains("does not match")
    );
    let twice = vec![
        step(A, Action::Pick),
        step(A, Action::Pick),
        step(C, Action::Pick),
    ];
    assert!(validate(&twice, None).problem().unwrap().contains("twice"));
}

#[test]
fn joined_messages_and_todo() {
    let commits = three();
    let steps = vec![
        step(A, Action::Pick),
        step(B, Action::Squash),
        step(C, Action::Fixup),
    ];
    let joined = joined_message(&steps, &commits, 1);
    assert_eq!(joined.summary, "one");
    assert_eq!(joined.body, "two\n\nmore about two");
    let todo = build_todo(&steps, &commits).unwrap();
    let lines: Vec<&str> = todo.lines().collect();
    assert_eq!(lines[0], "pick aaaaaaa1 one");
    assert_eq!(lines[1], "squash bbbbbbb2 two");
    assert!(lines[2].starts_with("exec printf '%s\\n' 'one' '' 'two' '' 'more about two' |"));
    assert_eq!(lines[3], "fixup ccccccc3 three");
    let reword = vec![
        Step {
            sha: A.into(),
            action: Action::Reword,
            message: Some(Message {
                summary: "new".into(),
                body: String::new(),
            }),
        },
        step(B, Action::Pick),
        step(C, Action::Pick),
    ];
    let todo = build_todo(&reword, &commits).unwrap();
    assert!(todo.starts_with("pick aaaaaaa1 one\nexec printf '%s\\n' 'new' |"));
    assert_eq!(
        join_target(&[Action::Pick, Action::Drop, Action::Squash], 2),
        Some(0)
    );
    assert_eq!(join_target(&[Action::Drop, Action::Squash], 1), None);
}

#[test]
fn summary_and_changes() {
    let mut commits = three();
    commits[2].pushed = true;
    let same = vec![
        step(A, Action::Pick),
        step(B, Action::Pick),
        step(C, Action::Pick),
    ];
    assert!(is_unchanged(&same, &commits));
    let moved = move_step(&same, 2, true);
    assert_eq!(moved[1].sha, C);
    assert_eq!(move_step(&same, 0, true), same);
    let sum = summarize(&moved, &commits);
    assert!(sum.moved);
    assert_eq!(sum.first_change, Some(1));
    assert_eq!(sum.pushed_changed, 1);
    assert_eq!(sum.text, "3 kept, 0 dropped, 0 squashed. New order.");
    let dropped = vec![
        step(A, Action::Pick),
        step(B, Action::Drop),
        step(C, Action::Pick),
    ];
    assert_eq!(summarize(&dropped, &commits).dropped, 1);
}

#[test]
fn log_status_and_ignored_files() {
    let log = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0aaaaaaa\0pppppppp\0tttt\0Ann\0a@example.com\u{0}1700000000\0Subj\0Body\n\n\u{1e}\n";
    let c = parse_log(log);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].subject, "Subj");
    assert_eq!(c[0].body, "Body");
    assert!(!c[0].is_merge);
    let entries = parse_status_z("M  a.txt\0MM b.txt\0UU c.txt\0R  new.txt\0old.txt\0");
    assert_eq!(entries.len(), 4);
    let local = local_changes(&entries);
    assert!(local.staged && local.dirty);
    assert_eq!(local.mixed, vec!["b.txt".to_string()]);
    assert_eq!(local.unmerged, vec!["c.txt".to_string()]);
    let ignored = vec![
        "build/".to_string(),
        "x.log".to_string(),
        "src/".to_string(),
    ];
    let touched = vec!["x.log".to_string(), "src/a.rs".to_string()];
    let (risky, deeper) = ignored_at_risk(&ignored, &touched);
    assert_eq!(risky, vec!["x.log".to_string()]);
    assert_eq!(deeper, vec!["src/".to_string()]);
    assert_eq!(
        conflict_files("tree\na.txt\nb.txt\n\nmsg"),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );
    assert_eq!(force_hint(0), "");
}

fn line_repo() -> (Repo, String, Vec<String>) {
    let r = repo();
    let base = r.commit("a.txt", "zero\n", "base");
    let shas = vec![
        r.commit("b.txt", "b\n", "add b"),
        r.commit("c.txt", "c\n", "add c"),
        r.commit("a.txt", "zero\none\n", "change a"),
    ];
    (r, base, shas)
}

#[test]
fn a_real_plan_is_read() {
    let (r, base, shas) = line_repo();
    let p = plan::rebase_plan(r.path(), Some(&base));
    assert!(p.ok(), "{:?}", p.note);
    assert_eq!(
        p.commits.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(),
        shas
    );
    assert_eq!(p.note.title, "3 commits to change");
    assert_eq!(p.branch.as_deref(), Some("main"));
    assert!(!p.detached && !p.dirty);
    let none = plan::rebase_plan(r.path(), None);
    assert_eq!(none.note.kind, Some(Kind::NoUpstream));
    let bad = plan::rebase_plan(r.path(), Some("-x"));
    assert_eq!(bad.note.kind, Some(Kind::Invalid));
    r.write("b.txt", "dirty\n");
    r.git(&["add", "b.txt"]);
    r.write("b.txt", "dirtier\n");
    let mixed = plan::rebase_plan(r.path(), Some(&base));
    assert_eq!(mixed.note.kind, Some(Kind::MixedChanges));
    assert_eq!(mixed.note.files, vec!["b.txt".to_string()]);
}

#[test]
fn a_real_preview_finds_conflicts() {
    let r = repo();
    let base = r.commit("a.txt", "zero\n", "base");
    let one = r.commit("a.txt", "one\n", "one");
    let two = r.commit("a.txt", "two\n", "two");
    let p = plan::rebase_plan(r.path(), Some(&base));
    let swapped = vec![step(&two, Action::Pick), step(&one, Action::Pick)];
    let pre = plan::preview(r.path(), Some(&base), &swapped, &p.head);
    assert_eq!(pre.note.kind, Some(Kind::Conflict));
    assert_eq!(pre.note.files, vec!["a.txt".to_string()]);
    let keep = vec![step(&one, Action::Pick), step(&two, Action::Squash)];
    let pre = plan::preview(r.path(), Some(&base), &keep, &p.head);
    assert!(pre.clean, "{:?}", pre.note);
    assert!(pre.note.hint.contains("stay the same"));
    let stale = plan::preview(r.path(), Some(&base), &keep, "0000000");
    assert_eq!(stale.note.kind, Some(Kind::HeadChanged));
}

#[test]
fn a_real_rebase_changes_commits() {
    let (r, base, shas) = line_repo();
    let p = plan::rebase_plan(r.path(), Some(&base));
    let steps = vec![
        Step {
            sha: shas[0].clone(),
            action: Action::Reword,
            message: Some(Message {
                summary: "add bee".into(),
                body: "with words".into(),
            }),
        },
        step(&shas[2], Action::Pick),
        step(&shas[1], Action::Drop),
    ];
    let same = vec![
        step(&shas[0], Action::Pick),
        step(&shas[1], Action::Pick),
        step(&shas[2], Action::Pick),
    ];
    assert_eq!(
        plan::run_rebase(r.path(), Some(&base), &same, &p.head, None)
            .note
            .kind,
        Some(Kind::Nothing)
    );
    let out = plan::run_rebase(r.path(), Some(&base), &steps, &p.head, None);
    assert!(out.note.ok, "{:?}", out.note);
    let log = r.git(&["log", "--format=%s", &format!("{base}..HEAD")]);
    assert_eq!(log, "change a\nadd bee\n");
    assert_eq!(
        r.git(&["log", "-1", "--format=%b", "HEAD~1"]).trim(),
        "with words"
    );
    assert!(!r.path().join("c.txt").exists());
    assert_eq!(read_state(r.path()).unwrap().op, None);
}

#[test]
fn a_real_rebase_stops_on_a_conflict() {
    let r = repo();
    let base = r.commit("a.txt", "zero\n", "base");
    let one = r.commit("a.txt", "one\n", "one");
    let two = r.commit("a.txt", "two\n", "two");
    let p = plan::rebase_plan(r.path(), Some(&base));
    let swapped = vec![step(&two, Action::Pick), step(&one, Action::Pick)];
    let out = plan::run_rebase(r.path(), Some(&base), &swapped, &p.head, None);
    assert_eq!(out.note.kind, Some(Kind::Conflict), "{:?}", out.note);
    assert!(out.locked());
    let state = read_state(r.path()).unwrap();
    assert_eq!(state.op, Some(Op::Rebase));
    assert_eq!(state.step, Some((1, 2)));
    let busy = plan::rebase_plan(r.path(), Some(&base));
    assert_eq!(busy.note.kind, Some(Kind::Busy));
    assert_eq!(busy.note.title, "A rebase is in progress");
    r.git(&["rebase", "--abort"]);
}

#[test]
fn ignored_files_in_the_way_are_refused() {
    let r = repo();
    let base = r.commit("a.txt", "zero\n", "base");
    r.commit("keep.log", "tracked\n", "add log");
    r.git(&["rm", "-q", "--cached", "keep.log"]);
    r.write(".git/info/exclude", "*.log\n");
    r.git(&["commit", "-q", "-m", "untrack log"]);
    let p = plan::rebase_plan(r.path(), Some(&base));
    assert_eq!(p.note.kind, Some(Kind::Ignored), "{:?}", p.note);
    assert_eq!(p.note.files, vec!["keep.log".to_string()]);
}
