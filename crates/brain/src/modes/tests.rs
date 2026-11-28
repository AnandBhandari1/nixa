use super::autonomy::{self, Autonomy, StepInput};
use super::journal::{self, DecisionInput};
use super::keywords::{MatchState, match_line, strip_tail};
use super::scores::{self, Checks, JobClose, Mark, SettleInput};
use super::work::{self, GateInput};
use super::{complaints, index};
use crate::app::{App, lock};
use crate::settings_api::keywords::defaults;
use crate::settings_api::work::{Hold, Topic, TopicKind, Work};
use serde_json::json;
use std::sync::Arc;

fn hit(line: &str, state: MatchState) -> Option<(String, String)> {
    match_line(line, &defaults(), state).map(|m| (m.row.id, m.rest))
}

const FREE: MatchState = MatchState { held: false, hold_on: false };
const HELD: MatchState = MatchState { held: true, hold_on: false };
const HOLD_ON: MatchState = MatchState { held: true, hold_on: true };

#[test]
fn phrase_index_finds_every_phrase() {
    let built = index::build(&[("plan mode".into(), 0), ("mode".into(), 1), ("switch to plan mode".into(), 2), ("".into(), 3)]);
    let hits = index::scan(&built, "OK, switch to plan mode now!");
    let items: Vec<usize> = hits.iter().map(|h| h.item).collect();
    assert!(items.contains(&0) && items.contains(&1) && items.contains(&2));
    assert!(!items.contains(&3));
    let whole = hits.iter().find(|h| h.item == 2).unwrap();
    assert_eq!((whole.start, whole.end), (1, 5));
    assert!(index::scan(&built, "planning modes").is_empty());
    assert!(index::states(&built) > 1);
}

#[test]
fn keywords_match_like_spoken_words() {
    assert_eq!(hit("Let's discuss the login page", FREE), Some(("discuss".into(), "the login page".into())));
    assert_eq!(hit("plan mode", FREE), None);
    assert_eq!(hit("ok switch to review mode please", FREE), None);
    assert_eq!(hit("assign", FREE), None);
    assert_eq!(hit("assign", HELD).map(|h| h.0), Some("assign".into()));
    assert_eq!(hit("go ahead?", HELD), None);
    assert_eq!(hit("hold everything", FREE).map(|h| h.0), Some("hold".into()));
    assert_eq!(hit("hold the door for me", FREE), None);
    assert_eq!(hit("pause for a moment", FREE).map(|h| h.0), Some("hold".into()));
    assert_eq!(hit("go on", HELD), None);
    assert_eq!(hit("go on", HOLD_ON).map(|h| h.0), Some("go-on".into()));
    assert_eq!(hit("review terminal 3", FREE), Some(("review".into(), "3".into())));
    assert_eq!(hit("review m4", FREE), Some(("review".into(), "m4".into())));
    assert_eq!(hit("review the code in auth", FREE), None);
    assert_eq!(hit("explain why", FREE), None);
    assert_eq!(strip_tail("thanks for now"), "");
    assert_eq!(strip_tail("for now thanks"), "for");
    assert_eq!(strip_tail("the door please"), "the door");
}

#[test]
fn off_keywords_are_not_matched() {
    let mut rows = defaults();
    rows.iter_mut().filter(|r| r.id == "hold").for_each(|r| r.on = false);
    assert!(match_line("hold", &rows, FREE).is_none());
}

fn topic() -> Topic {
    Topic { kind: TopicKind::Discuss, text: "login".into(), at: 1, alias: "lets discuss".into() }
}

#[test]
fn the_gate_holds_only_new_work_while_hold_is_on() {
    let none = Work::default();
    let plain = GateInput::default();
    assert!(work::gate(&none, "send-work", &plain).is_none());
    assert!(work::passed_by(&none, "start-agent", &plain).is_none());
    let hold = Work { hold: Some(Hold { at: 1, alias: "hold".into() }), ..Work::default() };
    let held = work::gate(&hold, "send-work", &plain).unwrap();
    assert_eq!(held.reason, "keyword: hold");
    assert_eq!(held.item, "keyword:hold:hold");
    assert!(held.why.contains("ends by itself"));
    assert!(work::gate(&hold, "send-work", &GateInput { pass: true, ..GateInput::default() }).is_none());
    assert!(work::gate(&hold, "safe", &plain).is_none());
    assert!(work::gate(&hold, "fix", &plain).is_none());
    assert!(work::gate(&hold, "check", &plain).is_none());
    assert!(work::gate(&hold, "answer-prompt", &plain).is_none());
    assert_eq!(work::passed_by(&hold, "fix", &plain).unwrap().reason, "keyword: hold");
    assert!(work::gate(&hold, "send-work", &GateInput { hold_over: true, ..GateInput::default() }).is_none());
    let talk = Work { topic: Some(topic()), ..Work::default() };
    assert!(work::gate(&talk, "start-agent", &plain).is_none());
    assert!(work::gate(&talk, "fix", &plain).is_none());
    assert_eq!(work::passed_by(&talk, "start-agent", &plain).unwrap().reason, "keyword: discuss");
    assert!(work::passed_by(&talk, "fix", &plain).is_none());
    assert!(work::hold_over(1, work::HOLD_MS + 2));
    assert!(!work::hold_over(1, work::HOLD_MS));
    assert!(work::topic_over(&talk, work::TOPIC_MS + 2));
    assert!(!work::topic_over(&talk, 2));
    assert!(work::HELD_NEXT.contains("nixa_deep_ask") && !work::HELD_NEXT.contains("tell the user in one sentence"));
}

#[test]
fn work_words() {
    let hold = Work { hold: Some(Hold { at: 1, alias: "hold".into() }), ..Work::default() };
    let held = work::gate(&hold, "start-agent", &GateInput::default()).unwrap();
    assert_eq!(work::held_text(&held, " start the tests?! "), "Not started yet (keyword: hold): start the tests. Nixa goes on with the other work.");
    assert!(!work::held_text(&held, "start the tests").contains("Held for you"));
    assert_eq!(work::action_of_text("Needs your yes: Start codex with the first message: hi", false), "start-agent");
    assert_eq!(work::action_of_text("Send \"yes\" to answer \"go?\"", false), "answer-prompt");
    assert_eq!(work::action_of_text("Send \"fix it\"", true), "fix");
    assert_eq!(work::action_of_text("Send \"fix it\"", false), "send-work");
    assert_eq!(work::action_of_text("Press enter", false), "answer-prompt");
    assert_eq!(work::action_of_text("Interrupt m3", false), "app");
    assert!(work::asks_work("tell him to start the tests"));
    assert!(!work::asks_work("should we start?"));
    assert!(!work::asks_work("what is it"));
    assert!(work::hands_to_deep("let the deep brain decide"));
    assert!(work::hands_to_deep("ask him to decide"));
    assert!(!work::hands_to_deep("decide it yourself"));
    let line = work::work_line(&Work { topic: Some(topic()), hold: Some(Hold { at: 2, alias: "hold".into() }) }, |_| "10:00".into());
    assert_eq!(line, "Keyword discuss since 10:00: login. Hold on since 10:00.");
    assert_eq!(work::work_line(&Work::default(), |_| "10:00".into()), "");
    assert!(work::ACTIONS.iter().all(|a| work::is_action(a)));
}

#[test]
fn saved_work_comes_back() {
    let saved = work::Saved { topic: Some(topic()), hold: Some(Hold { at: 5, alias: "pause".into() }) };
    assert_eq!(work::parse_saved(Some(&work::saved_text(&saved))), saved);
    assert_eq!(work::parse_saved(Some(r#"{"topic": null, "hold": null, "build": ["m3"]}"#)), work::Saved::default());
    assert_eq!(work::parse_saved(Some("not json")), work::Saved::default());
    assert_eq!(work::parse_saved(Some(r#"{"topic": {"kind": "other"}, "hold": {"at": "x"}}"#)), work::Saved::default());
    assert_eq!(work::parse_saved(None), work::Saved::default());
}

#[test]
fn autonomy_rule_keeps_routine_verdicts_out_of_replies() {
    let rule = autonomy::AUTONOMY_RULE;
    assert!(rule.contains("Call nixa_autonomy"));
    assert!(!rule.contains("say its one-line verdict"));
    assert!(rule.contains("never add a routine autonomy line"));
    assert!(rule.contains("only when it refuses a step"));
    assert!(rule.contains("No step waits for the user"));
    assert!(rule.contains("Steps in other spaces run without a wait"));
    assert!(!rule.contains("Decision"));
    assert!(autonomy::scope_gate(false, true, Some("/work/shop"), "/work/nixa").allowed);
    assert!(!rule.contains("still wait for the user"));
    assert!(autonomy::LIMIT_LINE.contains("runs without a wait"));
    assert!(!autonomy::LIMIT_LINE.contains("Manual"));
}

#[test]
fn autonomy_is_judged() {
    assert_eq!(autonomy::judge("restart the brain by itself").verdict, Autonomy::Increase);
    assert_eq!(autonomy::judge("always ask for a card before this").verdict, Autonomy::Decrease);
    assert_eq!(autonomy::judge("git push origin main").verdict, Autonomy::Limit);
    assert_eq!(autonomy::judge("rename the file").verdict, Autonomy::Neutral);
    assert_eq!(autonomy::judge("do it automatically but always ask for a card").verdict, Autonomy::Neutral);
    assert!(autonomy::in_nixa_space(Some("/work/nixa/src/../docs"), "/work/nixa"));
    assert!(!autonomy::in_nixa_space(Some("/work/nixa-other"), "/work/nixa"));
    assert!(!autonomy::in_nixa_space(Some("/work/nixa/../other"), "/work/nixa"));
    assert!(autonomy::in_nixa_space(Some("Nixa"), "/work/nixa"));
    assert!(!autonomy::in_nixa_space(None, "/work/nixa"));
    let outside = autonomy::weigh(&StepInput {
        kind: "next step".into(),
        text: "fix the tool gap".into(),
        source_space: Some("/work/shop".into()),
        action_space: Some("/work/shop".into()),
        changes_files: true,
        from_user: false,
        root: "/work/nixa".into(),
    });
    assert!(outside.step.allowed);
    assert_eq!(outside.step.verdict, Autonomy::Increase);
    assert_eq!((outside.step.source_space.as_str(), outside.step.action_space.as_str()), ("shop", "shop"));
    let learning = autonomy::weigh(&StepInput { kind: "learning".into(), text: "they use pnpm".into(), source_space: Some("/work/shop".into()), root: "/work/nixa".into(), ..StepInput::default() });
    assert!(learning.step.allowed);
    assert_eq!(learning.step.action_space, "none");
    let limit = autonomy::weigh(&StepInput { kind: "next step".into(), text: "deploy it".into(), action_space: Some("/work/nixa".into()), changes_files: true, root: "/work/nixa".into(), ..StepInput::default() });
    assert!(limit.step.allowed);
    let mine = autonomy::weigh(&StepInput { kind: "next step".into(), text: "deploy it".into(), from_user: true, changes_files: true, root: "/work/nixa".into(), ..StepInput::default() });
    assert!(mine.step.allowed);
    let pushed = autonomy::weigh(&StepInput { kind: "next step".into(), text: "git push and deploy it".into(), action_space: Some("/work/nixa".into()), changes_files: true, root: "/work/nixa".into(), ..StepInput::default() });
    assert_eq!(pushed.step.verdict, Autonomy::Limit);
    assert!(pushed.step.allowed, "{}", pushed.reason);
    assert!(pushed.reason.contains("decision journal"));
}

fn temp_db() -> (tempfile::TempDir, nixa_store::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = nixa_store::open(&dir.path().join("nixa.db")).unwrap();
    journal::ensure(&conn, 1_000).unwrap();
    scores::ensure(&conn).unwrap();
    (dir, conn)
}

fn input(kind: &str, choice: &str, terminal: Option<&str>, key: Option<&str>) -> DecisionInput {
    DecisionInput { kind: kind.into(), choice: choice.into(), terminal: terminal.map(str::to_string), key: key.map(str::to_string), ..DecisionInput::default() }
}

#[test]
fn journal_records_searches_and_corrects() {
    let (_dir, conn) = temp_db();
    let first = journal::record(&conn, &DecisionInput { reason: Some("tests pass".into()), member: Some("dev".into()), ..input("job", "fix the login page", Some("m3"), Some("job|dev|/w/app")) }, 10_000).unwrap();
    let second = journal::record(&conn, &DecisionInput { confidence: Some("not sure".into()), autonomy: Some("increase".into()), ..input("prompt", "yes to\n  the  read", Some("m3"), None) }, 20_000).unwrap();
    let odd = journal::record(&conn, &input("strange", "", None, None), 30_000).unwrap();
    let got = journal::get(&conn, odd).unwrap().unwrap();
    assert_eq!((got.kind.as_str(), got.choice.as_str()), ("job", "(no words)"));
    assert_eq!(journal::get(&conn, second).unwrap().unwrap().choice, "yes to the read");
    assert_eq!(journal::count(&conn).unwrap(), 3);
    assert_eq!(journal::search(&conn, "login tests", 10).unwrap().iter().map(|d| d.id).collect::<Vec<_>>(), vec![first]);
    assert_eq!(journal::search(&conn, "%_ x", 10).unwrap().len(), 3);
    assert_eq!(journal::last_for(&conn, Some("m3"), 1).unwrap()[0].id, second);
    let corrected = journal::correct(&conn, 15_000, 31_000).unwrap().unwrap();
    assert_eq!(corrected.id, odd);
    assert!(journal::get(&conn, odd).unwrap().unwrap().corrected);
    assert!(journal::correct(&conn, 1_000, 99_000).unwrap().is_none());
    let text = journal::describe(&journal::get(&conn, second).unwrap().unwrap());
    assert!(text.contains("decided an agent prompt: \"yes to the read\"."));
    assert!(text.contains("Autonomy: increase.") && text.contains("Nixa was not sure."));
    assert_eq!(journal::describe_all(&[]), "No decisions are recorded for that.");
    journal::set_meta(&conn, "lastReview", "5").unwrap();
    journal::set_meta(&conn, "lastReview", "6").unwrap();
    assert_eq!(journal::meta(&conn, "lastReview").unwrap().as_deref(), Some("6"));
    assert_eq!(journal::meta(&conn, "none").unwrap(), None);
    assert!(journal::is_correction("No, not that one"));
    assert!(journal::is_correction("don't"));
    assert!(!journal::is_correction("now do it"));
}

#[test]
fn journal_settles_once_and_keeps_bad() {
    let (_dir, conn) = temp_db();
    let id = journal::record(&conn, &input("fix", "send fix", Some("m2"), Some("fix|dev|none")), 1_000).unwrap();
    assert_eq!(journal::open_rows(&conn, "m2", "fix").unwrap().len(), 1);
    let done = journal::settle(&conn, id, "good", Some("checks passed"), 2_000).unwrap().unwrap();
    assert_eq!((done.key.as_str(), done.before), ("fix|dev|none", None));
    assert!(journal::settle(&conn, id, "neutral", None, 3_000).unwrap().is_none());
    assert_eq!(journal::settle(&conn, id, "bad", None, 3_000).unwrap().unwrap().before.as_deref(), Some("good"));
    assert!(journal::settle(&conn, id, "good", None, 4_000).unwrap().is_none());
    assert!(journal::open_rows(&conn, "m2", "fix").unwrap().is_empty());
    let row = journal::get(&conn, id).unwrap().unwrap();
    assert_eq!((row.result.as_deref(), row.outcome.as_deref()), (Some("bad"), Some("bad")));
    let keyless = journal::record(&conn, &input("fix", "x", Some("m2"), None), 1_000).unwrap();
    assert!(journal::settle(&conn, keyless, "good", None, 2_000).unwrap().is_none());
}

#[test]
fn journal_changes_and_records() {
    let (_dir, conn) = temp_db();
    let files: Vec<String> = (0..5).map(|n| format!("src/f{n}.rs")).chain([" ".to_string()]).collect();
    journal::record_changes(&conn, &journal::ChangeInput { terminal: "m4".into(), project: Some("/w/app".into()), files, outcome: "done".into(), report: Some("ok".into()) }, 5_000).unwrap();
    let rows = journal::changes_since(&conn, 0, Some("app")).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].files.len(), rows[0].count), (5, 6));
    assert!(journal::changes_since(&conn, 0, Some("shop")).unwrap().is_empty());
    assert!(journal::describe_changes(&rows).contains("6 files (src/f0.rs, src/f1.rs, src/f2.rs and 3 more), done"));
    assert_eq!(journal::last_change_for(&conn, "m4").unwrap().unwrap().report.as_deref(), Some("ok"));
    for (kind, choice) in [("job", "a"), ("close", "done"), ("fix", "b"), ("hold", "c")] {
        journal::record(&conn, &DecisionInput { member: Some("dev".into()), project: Some("/w/app".into()), ..input(kind, choice, Some("m4"), None) }, 6_000).unwrap();
    }
    journal::record(&conn, &DecisionInput { member: Some("qa".into()), ..input("close", "needs-user", Some("m5"), None) }, 6_000).unwrap();
    let records = journal::records(&conn).unwrap();
    assert_eq!(journal::describe_records(&records), vec!["dev: 1 job, 1 done, 1 held for the user, 1 fix", "qa: 0 jobs, 0 done, 1 held for the user, 0 fixes"]);
    let ranked = journal::rank_members(&records, Some("/w/app"), &["qa".into(), "dev".into(), "new".into()]);
    assert_eq!(ranked, vec!["new", "dev", "qa"]);
}

#[test]
fn review_text_names_corrections() {
    let rows = vec![
        journal::Decision { id: 1, at: 0, kind: "confirm".into(), choice: "push".into(), outcome: Some("no".into()), ..journal::Decision::default() },
        journal::Decision { id: 2, at: 0, kind: "fix".into(), choice: "fix it".into(), corrected: true, ..journal::Decision::default() },
    ];
    let text = journal::review_text(&rows, 0);
    assert!(text.contains("Apply the best one to three lines yourself"), "{text}");
    assert!(!text.contains("The user gives their yes to each one."));
    assert!(text.starts_with("Event: Weekly review: since "));
    assert!(text.contains("made 2 decisions (1 sent a fix, 1 asked the user)"));
    assert!(text.contains("The user corrected these:") && text.contains("The user said no to these:"));
    assert!(super::review_due(1, 1 + journal::REVIEW_MS, &rows, 0));
    assert!(!super::review_due(1, 2, &rows, 0));
    assert!(!super::review_due(1, 1 + journal::REVIEW_MS, &[], 0));
    assert!(super::review_due(1, 1 + journal::REVIEW_MS, &[], 1));
}

#[test]
fn score_keys_and_levels() {
    assert_eq!(scores::kind_key("job", Some("Dev Team|x"), Some("/w/app/")), "job|dev-team-x|/w/app");
    assert_eq!(scores::kind_key("hold", None, Some("(no folder)")), "hold|any|none");
    let parts = scores::key_parts("job|dev|/w/a|b");
    assert_eq!((parts.kind.as_str(), parts.action.as_str(), parts.project.as_str()), ("job", "dev", "/w/a|b"));
    assert_eq!(scores::key_parts("job").project, "none");
    assert_eq!(scores::message_action("Needs your yes: send the tests"), "message");
    assert_eq!(scores::message_action("apply Fix.rs now"), "fix.rs");
    assert_eq!(scores::message_action("Restart m3"), "restart");
    assert_eq!(scores::prompt_action("permission", Some("Bash")), "permission:Bash");
    assert_eq!(scores::prompt_action("plan", None), "plan");
    let verbs = vec!["push".to_string()];
    assert_eq!(scores::action_of(&scores::ActionInput { kind: "hold", choice: "x", verbs: &verbs, ..Default::default() }), "git:push");
    assert_eq!(scores::action_of(&scores::ActionInput { kind: "rule", choice: "x", reason: Some("rule r12: yes"), ..Default::default() }), "rule:r12");
    assert_eq!(scores::action_of(&scores::ActionInput { kind: "close", choice: "x", agent: Some("codex"), ..Default::default() }), "codex");
    assert_eq!(scores::level_of(5, 0).0, scores::Level::High);
    assert_eq!(scores::level_of(3, 2).0, scores::Level::Middle);
    assert_eq!(scores::level_of(1, 0).0, scores::Level::Low);
    let row = scores::ScoreRow { key: "k".into(), good: 35, bad: 10 };
    assert_eq!(scores::next_counts(&row, "bad", Some("good")), (17, 6));
    assert_eq!(scores::next_counts(&row, "good", None), (18, 5));
    assert_eq!(scores::score_line(&scores::ScoreRow { key: "job|dev|/w/app".into(), good: 2, bad: 1 }), "job dev in app: 2 of 3 good, middle");
}

#[test]
fn scores_count_and_move() {
    let (_dir, conn) = temp_db();
    scores::add(&conn, "job|dev|/w/app", "good", None, 1).unwrap();
    scores::add(&conn, "job|dev|/w/app", "neutral", None, 2).unwrap();
    let moved = scores::add(&conn, "job|dev|/w/app", "bad", Some("good"), 3).unwrap();
    assert_eq!((moved.good, moved.bad), (0, 1));
    scores::add(&conn, "fix|qa|none", "good", None, 4).unwrap();
    assert_eq!(scores::lines(&conn, Some("app"), 10).unwrap(), vec!["job dev in app: 0 of 1 good, low"]);
    assert_eq!(scores::top(&conn, None, 10).unwrap().len(), 2);
    assert_eq!(scores::level(&conn, "none").unwrap(), scores::Level::Low);
}

fn decision(id: i64, at: u64, kind: &str, terminal: &str, choice: &str) -> journal::Decision {
    journal::Decision { id, at, kind: kind.into(), terminal: Some(terminal.into()), choice: choice.into(), key: Some(format!("{kind}|x|/w/app")), ..journal::Decision::default() }
}

#[test]
fn results_settle_by_the_rules() {
    let now = 10 * 60 * 60 * 1000;
    let mut corrected = decision(1, now - 1000, "fix", "m1", "a");
    corrected.corrected = true;
    let mut yes = decision(2, now - 1000, "confirm", "m1", "push?");
    yes.outcome = Some("yes".into());
    let quiet = decision(3, now - scores::QUIET_MS - 1, "prompt", "m2", "allow read");
    let errored = decision(4, now - scores::QUIET_MS - 1, "auto-yes", "m3", "send");
    let complained = decision(5, now - 5000, "rule", "m4", "rule");
    let hold = decision(6, now - 1000, "hold", "m5", "push");
    let old_hold = decision(7, now - scores::HOLD_YES_MS - 1, "hold", "m6", "push");
    let job = decision(8, now - scores::SAME_JOB_MS - 10_000, "job", "m7", "Run the unit tests now");
    let close = decision(9, now - scores::SAME_JOB_MS - 5_000, "close", "m7", "done");
    let back = decision(10, now - scores::SAME_JOB_MS, "job", "m7", "Run the unit tests now");
    let rows = vec![corrected, yes, quiet, errored, complained, hold, old_hold, job, close, back];
    let errors = vec![Mark { terminal: "m3".into(), at: now - scores::QUIET_MS + 1000 }];
    let approvals = vec![Mark { terminal: "m5".into(), at: now }];
    let complaints = vec![now - 4000];
    let out = scores::settle(&SettleInput { decisions: &rows, now, approvals: &approvals, complaints: &complaints, errors: &errors });
    let of = |id: i64| out.iter().find(|s| s.id == id).map(|s| s.outcome.clone());
    assert_eq!(of(1).as_deref(), Some("bad"));
    assert_eq!(of(2).as_deref(), Some("good"));
    assert_eq!(of(3).as_deref(), Some("good"));
    assert_eq!(of(4).as_deref(), Some("neutral"));
    assert_eq!(of(5).as_deref(), Some("bad"));
    assert_eq!(of(6).as_deref(), Some("good"));
    assert_eq!(of(7).as_deref(), Some("neutral"));
    assert_eq!(of(8).as_deref(), Some("bad"));
    assert_eq!(of(9).as_deref(), Some("bad"));
    assert_eq!(of(10), None);
}

#[test]
fn job_and_fix_results() {
    let close = |outcome: &str, checks: Option<Checks>| JobClose { outcome: outcome.into(), checks, spent: false, short: Some("cargo".into()) };
    assert_eq!(scores::job_result(&close("done", Some(Checks::Passed))).0, "good");
    assert_eq!(scores::job_result(&close("done", Some(Checks::Failed))).0, "neutral");
    assert_eq!(scores::job_result(&close("done", None)).1, "closed done with no checks");
    assert_eq!(scores::job_result(&close("used all rounds", None)).0, "bad");
    assert_eq!(scores::job_result(&close("checks failed", None)).1, "closed with failed checks: cargo");
    assert_eq!(scores::job_result(&close("stopped", None)), ("neutral".into(), "closed: stopped".into()));
    assert_eq!(scores::fix_result(Checks::None, None), None);
    assert_eq!(scores::fix_result(Checks::Failed, None).unwrap().0, "bad");
    assert_eq!(scores::check_state(None), Checks::None);
    assert_eq!(scores::check_state(Some((true, false))), Checks::Failed);
    assert_eq!(scores::check_state(Some((false, false))), Checks::Passed);
    let (_dir, conn) = temp_db();
    journal::record(&conn, &input("job", "build it", Some("m9"), Some("job|dev|none")), 1).unwrap();
    journal::record(&conn, &input("fix", "fix it", Some("m9"), Some("fix|dev|none")), 2).unwrap();
    assert_eq!(scores::checked(&conn, "m9", Checks::None, None, 3).unwrap(), 0);
    assert_eq!(scores::job_closed(&conn, "m9", &close("done", Some(Checks::Passed)), 4).unwrap(), 2);
    assert_eq!(scores::get(&conn, "job|dev|none").unwrap().good, 1);
    assert_eq!(scores::get(&conn, "fix|dev|none").unwrap(), scores::ScoreRow { key: "fix|dev|none".into(), good: 0, bad: 0 });
}

#[test]
fn complaints_are_spotted_and_kept() {
    assert!(complaints::is_complaint("this is so confusing"));
    assert!(complaints::is_complaint("You keep asking me the same thing"));
    assert!(complaints::is_complaint("I can't hear you at all"));
    assert!(!complaints::is_complaint("stupid"));
    assert!(!complaints::is_complaint("from now on do not be so annoying"));
    assert!(!complaints::is_complaint("please run the tests"));
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("deep").join(complaints::FILE);
    complaints::add(&file, " you are not listening ", false, Some("Done."), 100).unwrap();
    complaints::add(&file, "how many times must I say it", true, None, 200).unwrap();
    std::fs::write(dir.path().join("x"), "").unwrap();
    let recent = complaints::recent(&file, 1);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].at, 200);
    assert_eq!(complaints::since(&file, 150).len(), 1);
    let text = complaints::text_of(&complaints::recent(&file, 5));
    assert!(text.contains("typed: \"how many times must I say it\""));
    assert!(text.contains("said: \"you are not listening\". Nixa had just said: \"Done.\""));
    assert_eq!(complaints::text_of(&[]), "No complaints saved.");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
}

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

#[tokio::test]
async fn keyword_lines_change_the_work_state() {
    let (_dir, app) = app();
    assert!(!super::heard(&app, "Nixa, plan mode", true));
    assert!(super::gate(&app, "start-agent", None).is_none());
    assert!(super::heard(&app, "hold everything", true));
    assert!(lock(&app.switches.work).hold.is_some());
    assert!(lock(&app.brain).carry.contains("ends by itself"));
    let held = super::hold_work(&app, "send-work", "Needs your yes: send the fix?", None).unwrap();
    assert!(held.starts_with("Not started yet (keyword: hold): send the fix."), "{held}");
    assert!(super::gate(&app, "fix", None).is_none());
    assert!(super::heard(&app, "go on", true));
    assert!(lock(&app.switches.work).hold.is_none());
    assert!(!super::heard(&app, "let us discuss the menu", false));
    assert_eq!(lock(&app.switches.work).topic.as_ref().map(|t| t.text.clone()).as_deref(), Some("the menu"));
    assert!(super::gate(&app, "send-work", None).is_none());
    assert!(!super::heard(&app, "what is the weather", true));
    super::save(&app);
    let saved = work::parse_saved(std::fs::read_to_string(&app.modes.work_file).ok().as_deref());
    assert_eq!(saved.topic.map(|t| t.alias).as_deref(), Some("discuss"));
    let conn = super::store(&app).unwrap();
    let rows = journal::since(&conn, 0).unwrap();
    assert!(rows.iter().filter(|r| r.kind == "keyword").count() >= 3);
    assert!(rows.iter().any(|r| r.kind == "hold" && r.item.as_deref() == Some("keyword:hold:hold everything")));
    assert!(rows.iter().any(|r| r.kind == "auto-yes"));
}

#[tokio::test]
async fn saved_state_comes_back_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let app = App::open(dir.path()).unwrap();
        assert!(super::heard(&app, "pause", true));
        super::save(&app);
    }
    let app = App::open(dir.path()).unwrap();
    assert!(lock(&app.switches.work).hold.is_none());
    super::tick(&app);
    assert_eq!(lock(&app.switches.work).hold.as_ref().map(|h| h.alias.clone()).as_deref(), Some("pause"));
}

#[tokio::test]
async fn complaints_and_corrections_from_the_chat() {
    let (_dir, app) = app();
    super::record(&app, input("auto-yes", "Send \"go\"", Some("m2"), None)).unwrap();
    assert!(!super::heard(&app, "no, wrong one", true));
    assert!(lock(&app.chat).list().iter().any(|i| i.text == "Noted: you corrected my last decision."));
    assert!(!super::heard(&app, "you keep asking me about it", false));
    assert_eq!(complaints::read(&app.modes.complaints_file).len(), 1);
    let conn = super::store(&app).unwrap();
    let row = journal::last_for(&conn, Some("m2"), 1).unwrap().remove(0);
    assert!(row.corrected);
    assert_eq!(row.key.as_deref(), Some("auto-yes|message|none"));
}

#[tokio::test]
async fn tools_answer() {
    let (_dir, app) = app();
    let args = |v: serde_json::Value| v.as_object().unwrap().clone();
    let out = super::call(&app, super::tools::AUTONOMY, &args(json!({"kind": "next step", "text": "fix the missing tool", "runsIn": "/elsewhere/shop", "changesFiles": true}))).await.unwrap().unwrap();
    assert!(out.contains("This increases my autonomy") && out.contains("Allowed"));
    assert!(lock(&app.modes.live).step.as_ref().is_some_and(|s| s.allowed));
    let bad = super::call(&app, super::tools::AUTONOMY, &args(json!({"kind": "dream", "text": "x", "changesFiles": true}))).await.unwrap();
    assert!(bad.is_err());
    let explain = super::call(&app, super::tools::EXPLAIN, &args(json!({}))).await.unwrap().unwrap();
    assert!(explain.contains("weighed an autonomy step"));
    let past = super::call(&app, super::tools::PAST, &args(json!({"query": "missing tool"}))).await.unwrap().unwrap();
    assert!(past.contains("next step: fix the missing tool"));
    assert_eq!(super::call(&app, super::tools::CHANGED, &args(json!({}))).await.unwrap().unwrap(), "No changed files are recorded for that.");
    assert_eq!(super::call(&app, super::tools::COMPLAINTS, &args(json!({"limit": 3}))).await.unwrap().unwrap(), "No complaints saved.");
    assert_eq!(super::call(&app, super::tools::SCORES, &args(json!({}))).await.unwrap().unwrap(), "No scores yet.");
    assert!(super::call(&app, "other", &args(json!({}))).await.is_none());
    assert_eq!(super::specs("Nixa").len(), super::NAMES.len());
}


#[tokio::test]
async fn work_goes_on_with_either_decider() {
    let (_dir, app) = app();
    let topic = |app: &Arc<App>| lock(&app.switches.work).topic.is_some();
    assert!(!super::heard(&app, "let us discuss the menu", true));
    assert!(topic(&app));
    app.decision.set_choice("nixa").unwrap();
    assert!(!super::heard(&app, "let the deep brain decide", true));
    assert!(topic(&app));
    assert!(!super::heard(&app, "tell claude to make the menu page", true));
    assert!(!topic(&app));
    app.decision.set_choice("deep").unwrap();
    assert!(!super::heard(&app, "let us discuss the menu", true));
    assert!(topic(&app));
    assert!(!super::heard(&app, "let the deep brain decide", true));
    assert!(!topic(&app));
    let carried = lock(&app.brain).carry.clone();
    assert!(carried.contains("nixa_deep_ask"), "{carried}");
}

#[tokio::test]
async fn hold_holds_only_new_work_and_ends_by_itself() {
    let (_dir, app) = app();
    assert!(super::heard(&app, "hold everything", true));
    assert!(super::gate(&app, "fix", None).is_none());
    assert!(super::gate(&app, "check", None).is_none());
    assert!(super::gate(&app, "start-agent", None).is_some());
    let held = super::hold_work(&app, "send-work", "send the fix", None).unwrap();
    assert!(held.starts_with("Not started yet (keyword: hold): send the fix."), "{held}");
    assert!(!held.contains("Held for you"));
    assert_eq!(super::effects::waited_line(0), "");
    assert_eq!(super::effects::waited_line(1), " 1 agent question waited.");
    assert_eq!(super::effects::waited_line(3), " 3 agent questions waited.");
    let id = super::note(&app, "fix", "fix the menu", Some("m5"), Some("deep brain 4")).unwrap();
    let conn = super::store(&app).unwrap();
    let rows = journal::since(&conn, 0).unwrap();
    assert!(rows.iter().any(|r| r.id == id && r.kind == "fix" && r.terminal.as_deref() == Some("m5")));
    if let Some(hold) = lock(&app.switches.work).hold.as_mut() {
        hold.at = 1;
    }
    super::tick(&app);
    assert!(lock(&app.switches.work).hold.is_none());
}

#[test]
fn hold_and_discuss_texts() {
    assert!(super::effects::hold_note("hold").contains("still run"));
    assert!(!super::effects::hold_note("hold").contains("answer no agent question"));
    assert!(!super::effects::hold_reply(false).contains("until you say go on"));
    assert!(super::effects::discuss_note("").contains("Do not wait for assign"));
    assert!(!super::effects::DISCUSS_INFO.contains("until you say assign"));
}
