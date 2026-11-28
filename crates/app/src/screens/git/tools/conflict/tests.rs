use super::io;
use super::model::*;
use crate::screens::git::tools::state::{Op, read_state};
use crate::screens::git::tools::testrepo::repo;

const TEXT: &str = "top\n<<<<<<< HEAD\nmine\n||||||| base\nold\n=======\ntheirs\n>>>>>>> feature\nmid\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> feature\nend";

#[test]
fn blocks_are_read() {
    let p = parse_conflicts(TEXT);
    let c = conflicts_of(&p);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].ours(), "mine");
    assert_eq!(c[0].theirs(), "theirs");
    assert_eq!(c[0].base_lines, Some(vec!["old".to_string()]));
    assert_eq!(c[0].ours_label, "HEAD");
    assert_eq!(c[0].theirs_label, "feature");
    assert_eq!(c[0].line, 2);
    assert_eq!(c[1].choices().len(), 3);
    assert_eq!(c[0].choices().len(), 4);
    assert_eq!(side_starts(&p), vec![(2, 2), (4, 4)]);
}

#[test]
fn choices_build_the_result() {
    let p = parse_conflicts(TEXT);
    assert_eq!(resolve_text(&p, &[]), TEXT);
    let text = resolve_text(&p, &[Some(Choice::Ours), Some(Choice::Both)]);
    assert_eq!(text, "top\nmine\nmid\na\nb\nend");
    assert!(!has_markers(&text));
    let base = resolve_text(&p, &[Some(Choice::Base), Some(Choice::Theirs)]);
    assert_eq!(base, "top\nold\nmid\nb\nend");
    let c = conflicts_of(&p);
    assert!(!is_chosen(&c[1], Some(Choice::Base)));
    assert!(is_chosen(&c[1], Some(Choice::Theirs)));
    let mut picks = vec![None, None];
    toggle(&mut picks, 0, Choice::Ours);
    assert_eq!(picks[0], Some(Choice::Ours));
    toggle(&mut picks, 0, Choice::Ours);
    assert_eq!(picks[0], None);
}

#[test]
fn broken_markers_stay_text() {
    let p = parse_conflicts("a\n<<<<<<< HEAD\nb\n>>>>>>> x\nc");
    assert!(conflicts_of(&p).is_empty());
    assert!(has_markers("a\n<<<<<<< HEAD\nb"));
    assert!(!has_markers("<<<<<<<< eight"));
    assert_eq!(resolve_text(&p, &[]), "a\n<<<<<<< HEAD\nb\n>>>>>>> x\nc");
}

#[test]
fn crlf_files_keep_their_line_ends() {
    let p = parse_conflicts("x\r\n<<<<<<< HEAD\r\nm\r\n=======\r\nt\r\n>>>>>>> o\r\n");
    let c = conflicts_of(&p);
    assert_eq!(c.len(), 1);
    assert!(c[0].crlf);
    assert_eq!(c[0].ours(), "m");
    let out = resolve_text(&p, &[Some(Choice::Theirs)]);
    assert_eq!(out, "x\r\nt\r\n");
    assert_eq!(for_area(&out), "x\nt\n");
    assert_eq!(to_crlf("a\nb"), "a\r\nb");
}

#[test]
fn words_follow_the_state() {
    assert_eq!(finish_hint(None), "Commit to finish.");
    assert_eq!(
        finish_hint(Some(Op::CherryPick)),
        "Press Continue in the Changes panel to finish the cherry-pick."
    );
    let status = Status {
        loading: false,
        done: false,
        conflicts: 2,
        chosen: 1,
        original: TEXT,
        hand_edited: false,
        result: "",
    };
    assert_eq!(
        result_status(&status),
        "1 of 2 conflicts chosen. Pick a side for each one, or edit the result."
    );
    let joined = join_sides(&conflicts_of(&parse_conflicts(TEXT)), true);
    assert!(joined.starts_with("--- conflict 1 (line 2) ---\nmine"));
    assert_eq!(cut("abcdef", 3), "abc");
}

fn conflicted() -> crate::screens::git::tools::testrepo::Repo {
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

#[test]
fn a_real_merge_conflict_is_saved() {
    let r = conflicted();
    let state = read_state(r.path()).unwrap();
    assert_eq!(state.op, Some(Op::Merge));
    assert_eq!(
        state.conflicts,
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );
    assert_eq!(state.branch.as_deref(), Some("main"));
    let text = io::read_file(r.path(), "a.txt").unwrap();
    let p = parse_conflicts(&text);
    let out = resolve_text(&p, &[Some(Choice::Both)]);
    io::write_resolved(r.path(), "a.txt", &out).unwrap();
    assert_eq!(r.read("a.txt"), "main\nfeature\n");
    let after = read_state(r.path()).unwrap();
    assert_eq!(after.conflicts, vec!["b.txt".to_string()]);
    assert_eq!(io::read_file(r.path(), "gone.txt").unwrap_err(), io::GONE);
    assert!(io::read_file(r.path(), "../x").is_err());
}
