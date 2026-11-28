use super::load::blame_file;
use super::model::*;
use crate::screens::git::tools::testrepo::repo;

const PORCELAIN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1 1 2\nauthor Ann\nauthor-mail <ann@example.com>\nauthor-time 1700000000\nauthor-tz +0530\nsummary First\nprevious bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \"old name.txt\"\nfilename \"a\\303\\251.txt\"\n\tone\naaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 2 2\n\ttwo\r\n0000000000000000000000000000000000000000 3 3 1\nauthor Not Committed Yet\nsummary Version of a.txt\nfilename a.txt\n\tthree\tx\n";

#[test]
fn porcelain_is_read() {
    let p = parse_blame(PORCELAIN, MAX_LINES);
    assert_eq!(p.lines.len(), 3);
    assert_eq!(p.lines[0].author, "Ann");
    assert_eq!(p.lines[0].filename, "aé.txt");
    assert_eq!(p.lines[1].text, "two");
    assert_eq!(p.lines[1].short, "aaaaaaa");
    assert!(p.lines[2].uncommitted);
    let c = &p.commits["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"];
    assert_eq!(c.email, "ann@example.com");
    assert_eq!(c.previous.as_ref().unwrap().file, "old name.txt");
    assert_eq!(commit_count(&p), 1);
    let cut = parse_blame(PORCELAIN, 2);
    assert_eq!(cut.lines.len(), 2);
    assert!(cut.truncated);
}

#[test]
fn bands_tabs_and_times() {
    let p = parse_blame(PORCELAIN, MAX_LINES);
    let b = bands(&p.lines);
    assert_eq!(
        b[0],
        Band {
            first: true,
            band: 0
        }
    );
    assert_eq!(
        b[1],
        Band {
            first: false,
            band: 0
        }
    );
    assert_eq!(
        b[2],
        Band {
            first: true,
            band: 1
        }
    );
    assert_eq!(expand_tabs("a\tb", 4), "a   b");
    assert_eq!(age_text(0, 10), "");
    assert_eq!(age_text(100, 130), "now");
    assert_eq!(age_text(1, 1 + 7_200), "2h ago");
    assert_eq!(age_text(1, 1 + 40_000_000), "1y ago");
    assert_eq!(format_date(1_700_000_000, "+0530"), "15 Nov 2023, 03:43");
    assert_eq!(format_date(1_700_000_000, ""), "14 Nov 2023, 22:13");
    assert_eq!(commas(20000), "20,000");
    assert_eq!(plural(1, "line", "lines"), "1 line");
    assert_eq!(
        short_rev("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        "aaaaaaa"
    );
    assert_eq!(short_rev("HEAD~2"), "HEAD~2");
}

#[test]
fn find_goes_to_numbers_and_text() {
    let p = parse_blame(PORCELAIN, MAX_LINES);
    assert_eq!(find_line(&p.lines, ":2", None), Some(1));
    assert_eq!(find_line(&p.lines, "99", None), Some(2));
    assert_eq!(find_line(&p.lines, "T", None), Some(1));
    assert_eq!(find_line(&p.lines, "t", Some(1)), Some(2));
    assert_eq!(find_line(&p.lines, "zzz", None), None);
}

#[test]
fn paths_and_revs_are_checked() {
    assert!(clean_path("../x").is_err());
    assert!(clean_path("/etc/x").is_err());
    assert!(clean_path("a/b.rs").is_ok());
    assert!(clean_rev("-x").is_err());
    assert!(clean_rev("a..b").is_err());
    assert_eq!(clean_rev(" HEAD ").unwrap(), "HEAD");
}

#[test]
fn failures_get_plain_words() {
    assert_eq!(
        blame_failure("fatal: file a.txt has only 0 lines", ""),
        None
    );
    assert_eq!(
        blame_failure("fatal: no such path 'x' in HEAD", "")
            .unwrap()
            .kind,
        Kind::New
    );
    assert_eq!(
        blame_failure("fatal: no such path 'x' in abc", "abc")
            .unwrap()
            .kind,
        Kind::Missing
    );
    assert_eq!(
        blame_failure("fatal: bad revision 'zz'", "zz")
            .unwrap()
            .kind,
        Kind::BadRev
    );
    assert_eq!(
        blame_failure("fatal: could not open object name list: x", "")
            .unwrap()
            .kind,
        Kind::IgnoreRevs
    );
}

#[test]
fn a_real_repo_is_blamed() {
    let r = repo();
    let first = r.commit("a.txt", "one\ntwo\n", "First");
    r.commit("a.txt", "one\nTWO\nthree\n", "Second");
    r.write("a.txt", "one\nTWO\nthree\nfour\n");
    let p = blame_file(r.path(), "a.txt", "", false, None).unwrap();
    assert_eq!(p.lines.len(), 4);
    assert_eq!(p.lines[0].sha, first);
    assert_eq!(p.lines[1].summary, "Second");
    assert!(p.lines[3].uncommitted);
    let second = &p.commits[&p.lines[1].sha];
    assert_eq!(second.previous.as_ref().unwrap().sha, first);
    let old = blame_file(r.path(), "a.txt", &first, false, None).unwrap();
    assert_eq!(old.lines.len(), 2);
    let missing = blame_file(r.path(), "b.txt", &first, false, None).unwrap_err();
    assert_eq!(missing.kind, Kind::Missing);
    r.write("new.txt", "x\n");
    assert_eq!(
        blame_file(r.path(), "new.txt", "", false, None)
            .unwrap_err()
            .kind,
        Kind::New
    );
    r.commit("empty.txt", "", "Empty");
    assert!(
        blame_file(r.path(), "empty.txt", "", false, None)
            .unwrap()
            .lines
            .is_empty()
    );
    r.commit("bin.dat", "a\0b\n", "Binary");
    assert_eq!(
        blame_file(r.path(), "bin.dat", "", false, None)
            .unwrap_err()
            .kind,
        Kind::Binary
    );
}

#[test]
fn a_cancelled_blame_stops() {
    let r = repo();
    r.commit("a.txt", "one\n", "First");
    let cancel = crate::screens::git::tools::run::new_cancel();
    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    let e = blame_file(r.path(), "a.txt", "", true, Some(&cancel)).unwrap_err();
    assert_eq!(e.kind, Kind::Cancelled);
}
