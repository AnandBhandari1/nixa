use super::*;

#[test]
fn start_switch_reads_its_parts() {
    let b = blame_open("/r|src/a.rs|HEAD~1|12");
    assert_eq!(b.repo, Some(PathBuf::from("/r")));
    assert_eq!(b.file.as_deref(), Some("src/a.rs"));
    assert_eq!(b.rev, "HEAD~1");
    assert_eq!(b.line, Some(12));
    let c = conflict_open("/r|x.txt");
    assert_eq!(c.file.as_deref(), Some("x.txt"));
    let r = rebase_open("/r");
    assert_eq!(r.base, None);
    assert_eq!(rebase_open("/r|abc1234").base.as_deref(), Some("abc1234"));
}
