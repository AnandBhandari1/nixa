use super::check_tool::{changed_in, given_paths, parse_status, plan_checks, seconds_of, tool_cache};
use super::checks::{ChecksOutcome, RunOptions, describe_checks, packages_for, plan, run_checks, safe_package, short_checks, summarize, tail_of};
use super::done::{DoneEvent, DoneQueue, Source, describe_done, done_lines};
use super::handoff::{FIX_ROUNDS, Person, after_step, escalation, fixer_lines, member_for, reviewer_lines};
use super::review_job::{Check, Outcome as JobOutcome, outcome_refused, record};
use super::settle::{Effect, Facts, CheckLine, Outcome, Team, settle, seat_gone, escalate_blocked, same_file};
use nixa_store::agents::AgentState;
use nixa_store::review::{HandoffMove, HandoffState, MAX_HANDOFFS, ReviewSeat, handoff_state};
use nixa_store::tasks::{
    Actor, AuthorKind, BlockedOn, CommentInput, CommentKind, TaskInput, TaskPatch, TaskStatus, add_comment, add_task, claim_task, must_task, set_status,
    task_comments, update_task,
};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::Path;

fn person(id: &str, role: &str) -> Person {
    Person { id: id.into(), name: format!("{id} name"), role: role.into(), ..Person::default() }
}

fn people() -> Vec<Person> {
    vec![person("d1", "developer"), person("r1", "code-reviewer"), person("s1", "senior-developer"), person("j1", "junior-developer")]
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

#[test]
fn review_finds_problems_then_fix_then_recheck() {
    let reviewer = person("r1", "code-reviewer");
    let developer = person("d1", "developer");
    let start = HandoffState::default();
    let HandoffMove::Pass { to, state, why } = after_step(&reviewer, "", Some(2), &start) else { panic!("pass expected") };
    assert_eq!(to, "developer");
    assert_eq!(state.reviewer, "r1");
    assert_eq!(state.steps, 1);
    assert!(why.contains("found 2 problems"));
    let HandoffMove::Pass { to, state, .. } = after_step(&developer, "", None, &state) else { panic!("pass expected") };
    assert_eq!(to, "r1");
    assert!(state.recheck);
    let HandoffMove::Pass { to, state, why } = after_step(&reviewer, "", None, &state) else { panic!("pass expected") };
    assert_eq!(to, "senior-developer");
    assert!(state.escalated);
    assert_eq!(state.rounds, 2);
    assert!(why.contains("one level up"));
    assert!(why.contains("no FINDINGS line"));
    let senior = person("s1", "senior-developer");
    let HandoffMove::Pass { to, state, .. } = after_step(&senior, "", None, &state) else { panic!("pass expected") };
    assert_eq!(to, "r1");
    let HandoffMove::User { why } = after_step(&reviewer, "", Some(1), &state) else { panic!("user expected") };
    assert!(why.contains(&format!("after {FIX_ROUNDS} fix rounds")));
}

#[test]
fn no_findings_is_done_and_too_many_steps_go_to_the_user() {
    let reviewer = person("r1", "code-reviewer");
    assert_eq!(after_step(&reviewer, "", Some(0), &HandoffState::default()), HandoffMove::Done);
    let full = HandoffState { steps: MAX_HANDOFFS, ..HandoffState::default() };
    assert!(matches!(after_step(&reviewer, "", Some(3), &full), HandoffMove::User { .. }));
    let dev = person("d1", "developer");
    assert_eq!(after_step(&dev, "", None, &HandoffState::default()), HandoffMove::Done);
    let HandoffMove::Pass { to, .. } = after_step(&dev, "r1", None, &HandoffState::default()) else { panic!("pass expected") };
    assert_eq!(to, "r1");
    assert!(matches!(after_step(&dev, "r1", None, &full), HandoffMove::User { .. }));
}

#[test]
fn escalation_and_lines() {
    let junior = person("j1", "junior-developer");
    assert!(escalation(&junior, &HandoffState::default(), "is stuck").is_none());
    let chain = HandoffState { steps: 1, ..HandoffState::default() };
    let Some(HandoffMove::Pass { to, state, .. }) = escalation(&junior, &chain, "is stuck") else { panic!("pass expected") };
    assert_eq!(to, "developer");
    assert!(state.escalated);
    assert!(escalation(&junior, &state, "is stuck").is_none());
    assert!(escalation(&person("s1", "senior-developer"), &chain, "is stuck").is_none());
    let list = people();
    assert_eq!(member_for(&list, "Developer").unwrap().id, "d1");
    assert_eq!(member_for(&list, "r1 name").unwrap().id, "r1");
    let benched = vec![Person { status: "bench".into(), ..person("b1", "developer") }];
    assert!(member_for(&benched, "developer").is_none());
    let reviewer = person("r1", "code-reviewer");
    assert_eq!(reviewer_lines(Some(&reviewer), &HandoffState::default()).len(), 2);
    assert!(reviewer_lines(Some(&reviewer), &HandoffState { recheck: true, ..HandoffState::default() })[0].contains("re-check"));
    assert!(reviewer_lines(Some(&junior), &chain).is_empty());
    let lines = fixer_lines(Some(&junior), &chain, 7);
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains("$NIXA_BIN task status 7 blocked"));
    assert!(lines[1].contains("the developer"));
    assert!(fixer_lines(Some(&junior), &HandoffState::default(), 7).is_empty());
}

#[test]
fn summaries_read_test_tools() {
    assert_eq!(summarize("test", "# pass 3\n# fail 1\n", false, false), "test failed (1 of 4)");
    assert_eq!(summarize("test", "Tests:       2 failed, 5 passed, 7 total", false, false), "test failed (2 of 7)");
    assert_eq!(summarize("test", "Tests:       5 passed, 5 total", true, false), "test passed (5 of 5)");
    assert_eq!(summarize("test", "      Tests  1 failed | 4 passed (5)", false, false), "test failed (1 of 5)");
    let cargo = "test result: ok. 3 passed; 0 failed; 0 ignored\ntest result: FAILED. 1 passed; 2 failed; 0 ignored";
    assert_eq!(summarize("cargo test", cargo, false, false), "cargo test failed (2 of 6)");
    assert_eq!(summarize("typecheck", "a.ts(1,1): error TS2322: x\nb.ts: error TS1005: y", false, false), "typecheck failed (2 type errors)");
    let rust = "error[E0308]: mismatched types\nerror: unused\nerror: could not compile `x`";
    assert_eq!(summarize("cargo check", rust, false, false), "cargo check failed (2 errors)");
    assert_eq!(summarize("lint", "\u{1b}[31mbad\u{1b}[0m", false, false), "lint failed");
    assert_eq!(summarize("lint", "", true, true), "lint ran out of time");
    let long: String = (1..=20).map(|n| format!("line {n}\n\n")).collect();
    let tail = tail_of(&long);
    assert!(tail.starts_with("line 13"));
    assert!(tail.ends_with("line 20"));
}

fn temp() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("rf").tempdir().expect("temp dir")
}

#[test]
fn finds_packages_for_changed_files() {
    let dir = temp();
    let root = dir.path();
    std::fs::create_dir_all(root.join("web/src")).unwrap();
    std::fs::create_dir_all(root.join("core/src")).unwrap();
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(root.join("web/package.json"), r#"{"scripts":{"test":"node t.js","lint":" ","build":"x"}}"#).unwrap();
    std::fs::write(root.join("core/Cargo.toml"), "[package]\nname = \"core\"\n").unwrap();
    let changed = vec!["web/src/a.ts".to_string(), "core/src/lib.rs".to_string(), "../outside.rs".to_string()];
    let found = packages_for(root, &changed, 3);
    assert_eq!(found, vec![root.join("web"), root.join("core")]);
    let planned = plan(root, &changed, &RunOptions::default()).unwrap();
    let names: Vec<&str> = planned.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["test", "cargo check", "cargo clippy", "cargo test"]);
    assert_eq!(planned[0].args, vec!["run".to_string(), "test".to_string()]);
    assert!(plan(&root.join("notes"), &[], &RunOptions::default()).is_err());
    std::fs::create_dir(root.join(".git")).unwrap();
    assert_eq!(packages_for(root, &[], 3), vec![root.join("core"), root.join("web")]);
    let check = plan_checks(root, &["notes/a.md".to_string()]);
    assert_eq!(check.packages.len(), 2);
}

#[tokio::test]
async fn runs_commands_with_time_limits() {
    let dir = temp();
    std::fs::write(dir.path().join("fail.sh"), "#!/bin/sh\necho broken; echo detail >&2; exit 3\n").unwrap();
    std::fs::set_permissions(dir.path().join("fail.sh"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let options = RunOptions { commands: vec!["echo fine".into(), "./fail.sh".into()], ..RunOptions::default() };
    let outcome = run_checks(dir.path(), &[], &options).await;
    assert_eq!(outcome.results.len(), 2);
    assert!(outcome.results[0].ok);
    assert!(!outcome.results[1].ok);
    assert!(outcome.failed);
    assert!(outcome.results[1].tail.contains("broken"));
    assert!(outcome.results[1].tail.contains("detail"));
    let slow = RunOptions { commands: vec!["sleep 5".into(), "echo late".into()], total_ms: Some(300), check_ms: Some(300), ..RunOptions::default() };
    let started = std::time::Instant::now();
    let outcome = run_checks(dir.path(), &[], &slow).await;
    assert!(started.elapsed().as_millis() < 3000);
    assert!(outcome.results[0].timed_out);
    assert_eq!(outcome.results[0].summary, "sleep 5 ran out of time");
    assert!(!outcome.results[1].ok);
    assert_eq!(outcome.results[1].summary, "echo late not run, out of time");
    let text = describe_checks(&outcome);
    assert!(text.starts_with("Checks run by Nixa in "));
    assert!(text.ends_with("The job is not done while a check fails."));
    let none = ChecksOutcome { none: "no package.json or Cargo.toml found".into(), ..ChecksOutcome::default() };
    assert_eq!(short_checks(&none), "none (no package.json or Cargo.toml found)");
    let missing = run_checks(Path::new("/"), &[], &RunOptions { commands: vec!["/nonexistent/x".into()], ..RunOptions::default() }).await;
    assert!(missing.failed);
    let shell = run_checks(dir.path(), &[], &RunOptions { commands: vec!["echo a; id".into()], ..RunOptions::default() }).await;
    assert!(shell.results.is_empty());
    assert!(plan(dir.path(), &[], &RunOptions { commands: vec!["sh -c id".into()], ..RunOptions::default() }).is_err());
    let typed = plan(dir.path(), &[], &RunOptions { commands: vec!["A=1 echo hi".into()], ..RunOptions::default() }).unwrap();
    assert_eq!((typed[0].program.as_str(), typed[0].args.len()), ("/usr/bin/env", 3));
}

#[tokio::test]
async fn git_status_gives_changed_files() {
    assert_eq!(parse_status(" M a.rs\0R  new.rs\0old.rs\0?? b/c.ts\0"), vec!["a.rs", "new.rs", "old.rs", "b/c.ts"]);
    assert_eq!(parse_status("C  copy.rs\0base.rs\0"), vec!["copy.rs"]);
    assert!(tool_cache("target/debug/x"));
    assert!(tool_cache("web/node_modules/a/b.js"));
    assert!(tool_cache(".trail/cache/x"));
    assert!(!tool_cache("src/targets.rs"));
    let dir = temp();
    let root = dir.path();
    let ok = std::process::Command::new("git").arg("init").arg("-q").arg(root).status().map(|s| s.success()).unwrap_or(false);
    if !ok {
        return;
    }
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    let changed = changed_in(root).await.unwrap();
    assert_eq!(changed, vec!["src/main.rs".to_string()]);
    let inner = changed_in(&root.join("src")).await.unwrap();
    assert_eq!(inner, vec!["main.rs".to_string()]);
    assert!(given_paths(root, &["../x".to_string()]).is_err());
    assert_eq!(given_paths(root, &["src/main.rs".to_string()]).unwrap(), vec!["src/main.rs".to_string()]);
    assert_eq!(seconds_of(Some(&json!(30))), 30_000);
    assert_eq!(seconds_of(Some(&json!(9999))), 600_000);
    assert_eq!(seconds_of(None), 120_000);
}

#[cfg(unix)]
#[test]
fn given_paths_refuse_a_link_out_of_the_project() {
    let outside = temp();
    std::fs::write(outside.path().join("package.json"), "{\"scripts\":{\"test\":\"true\"}}").unwrap();
    let dir = temp();
    let root = dir.path();
    std::os::unix::fs::symlink(outside.path(), root.join("away")).unwrap();
    assert!(given_paths(root, &["away".to_string()]).unwrap_err().contains("outside"));
    assert!(given_paths(root, &["away/package.json".to_string()]).is_err());
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/package.json"), "{}").unwrap();
    assert_eq!(given_paths(root, &["web".to_string()]).unwrap(), vec!["web/package.json".to_string()]);
    let missing = given_paths(root, &["away/deleted.rs".to_string()]).unwrap();
    let plan = plan_checks(root, &missing);
    assert!(plan.packages.iter().all(|p| !p.ends_with("away")));
    let derived = packages_for(root, &["away/deleted.rs".to_string(), "away/src/old.ts".to_string()], 3);
    assert!(derived.iter().all(|p| !p.ends_with("away")));
    assert!(!safe_package(root, &root.join("away")));
    assert!(safe_package(root, &root.join("web")));
    std::fs::create_dir_all(root.join("linked")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("package.json"), root.join("linked/package.json")).unwrap();
    assert!(!safe_package(root, &root.join("linked")));
    assert!(packages_for(root, &["linked/a.ts".to_string()], 3).iter().all(|p| !p.ends_with("linked")));
}

fn event(terminal: &str, source: Source, at: u64, text: &str) -> DoneEvent {
    DoneEvent { terminal: terminal.into(), agent: "codex".into(), cwd: "/p/app".into(), text: text.into(), screen: String::new(), source, at, fallback: None }
}

#[test]
fn done_queue_keeps_one_event_per_terminal() {
    let mut queue = DoneQueue::new(5000);
    assert!(queue.add(event("m1", Source::Watch, 1000, ""), 1000));
    assert!(!queue.add(event("m1", Source::Watch, 1500, ""), 1500));
    assert!(queue.add(event("m1", Source::Hook, 2000, "built it"), 2000));
    assert_eq!(queue.size(), 1);
    assert!(!queue.add(event("m1", Source::Hook, 3000, "built it"), 3000));
    assert!(!queue.add(event("m1", Source::Watch, 4000, ""), 4000));
    assert!(queue.add(event("m2", Source::Watch, 4000, ""), 4000));
    let due = queue.due(4000, usize::MAX, |_| true);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].text, "built it");
    assert!(queue.due(8000, usize::MAX, |_| false).is_empty());
    assert_eq!(queue.due(9000, usize::MAX, |_| true).len(), 1);
    let mut waiting = event("m3", Source::Hook, 1, "");
    waiting.screen = "● Waiting for 2 background agents to finish".into();
    assert!(!queue.add(waiting.clone(), 1));
    assert!(queue.is_waiting_for_agents("m3"));
    assert!(queue.again(event("m3", Source::Hook, 2, "done now"), 2));
    assert!(!queue.is_waiting_for_agents("m3"));
}

#[test]
fn done_lines_name_agent_job_and_place() {
    let numbers = HashMap::from([("m1".to_string(), 3u32)]);
    let jobs = HashMap::from([("m1".to_string(), "Add the \"numbers\" feature".to_string())]);
    let places = HashMap::new();
    let mut e = event("m1", Source::Hook, 1, "Built it.\n```rust\nfn x() {}\n```\nAll tests pass.");
    let lines = done_lines(std::slice::from_ref(&e), &numbers, &jobs, &places);
    assert_eq!(lines[0], "- Agent 3 (m1), job \"Add the 'numbers' feature\" in app finished its task. Its last reply: \"Built it. (code) All tests pass.\"");
    e.text.clear();
    e.terminal = "m9".into();
    let lines = done_lines(&[e], &numbers, &jobs, &places);
    assert!(lines[0].starts_with("- Agent m9 (m9) in app finished its task. No reply text"));
    assert!(describe_done(&[], &numbers, &jobs, &places).starts_with("Event: Done:"));
}

#[test]
fn review_job_records_and_refuses() {
    let numbers = HashMap::from([("m4".to_string(), 11u32)]);
    let mut check = Check::default();
    check.begin("m4", false);
    let wrong = record(&mut check, &numbers, "Nixa", &args(json!({"terminal": "12", "outcome": "done", "reason": "ok"})));
    assert!(wrong.unwrap_err().contains("terminal 11 (m4)"));
    check.outcomes.insert("m4".into(), ChecksOutcome { failed: true, ..ChecksOutcome::default() });
    let failed = record(&mut check, &numbers, "Nixa", &args(json!({"terminal": 11, "outcome": "done", "reason": "tests pass"})));
    assert!(failed.unwrap_err().contains("they failed"));
    check.outcomes.clear();
    assert!(record(&mut check, &numbers, "Nixa", &args(json!({"terminal": "terminal 11", "outcome": "done", "reason": " "}))).is_err());
    let ok = record(&mut check, &numbers, "Nixa", &args(json!({"terminal": "m4", "outcome": "done", "reason": "tests pass"})));
    assert!(ok.unwrap().contains("Review recorded for m4: done"));
    check.begin("m4", true);
    let deep = record(&mut check, &numbers, "Nixa", &args(json!({"terminal": "m4", "outcome": "needs-user", "reason": "pick a name"})));
    assert!(deep.unwrap_err().contains("not the user's"));
    assert!(record(&mut check, &numbers, "Nixa", &args(json!({"terminal": "m4", "outcome": "needs-user", "reason": "it wants to git push"}))).is_err());
    assert!(record(&mut check, &numbers, "Nixa", &args(json!({"terminal": "m4", "outcome": "needs-user", "reason": "deep brain d14 says ask the user"}))).is_err());
    let sign_in = record(&mut check, &numbers, "Nixa", &args(json!({"terminal": "m4", "outcome": "needs-user", "reason": "the agent needs the user to sign in to the store"})));
    assert!(sign_in.unwrap_err().contains("task note"));
    check.begin("m4", false);
    assert!(record(&mut check, &numbers, "Nixa", &args(json!({"terminal": "m4", "outcome": "needs-user", "reason": "the agent needs the user to sign in to the store"}))).is_ok());
    check.picks.insert("m4".into(), "2) send the fix".into());
    assert!(record(&mut check, &numbers, "Nixa", &args(json!({"terminal": "m4", "outcome": "needs-user", "reason": "it needs a test on the user's own phone"}))).is_err());
    assert!(outcome_refused(Some("1"), JobOutcome::NeedsUser).is_none());
    assert!(outcome_refused(Some("fix the test"), JobOutcome::Done).is_none());
    assert!(check.end("m4").is_some());
    assert!(check.checking.is_empty());
}

fn db() -> (tempfile::TempDir, nixa_store::Connection) {
    let dir = temp();
    let conn = nixa_store::open(&dir.path().join("nixa.db")).expect("db");
    (dir, conn)
}

fn agent(name: &str, terminal: &str) -> Actor {
    Actor::in_terminal(AuthorKind::Agent, name, terminal)
}

fn new_task(conn: &nixa_store::Connection, assigned: &str, then: &str) -> i64 {
    let input = TaskInput { title: "Build the thing".into(), status: Some(TaskStatus::Todo), space_id: "s1".into(), space_name: "Space one".into(), assigned_kind: "member".into(), assigned: assigned.into(), ..TaskInput::default() };
    let task = add_task(conn, &input, &Actor::new(AuthorKind::User, "Sam")).unwrap();
    if !then.is_empty() {
        update_task(conn, task.id, &TaskPatch { then_member: Some(then.into()), ..TaskPatch::default() }).unwrap();
    }
    task.id
}

fn work(conn: &nixa_store::Connection, id: i64, who: &str, terminal: &str, report: &str) {
    let claimed = claim_task(conn, id, &agent(who, terminal), "").unwrap();
    assert!(claimed.ok, "{}", claimed.reason);
    if !report.is_empty() {
        add_comment(conn, id, &CommentInput::new(CommentKind::Result, report), &agent(who, terminal)).unwrap();
    }
    set_status(conn, id, TaskStatus::Review, None, false).unwrap();
}

fn team() -> Team {
    Team { people: people(), folders: HashMap::from([("s1".to_string(), "/p/app".to_string())]), ..Team::default() }
}

fn facts(files: &[&str]) -> Facts {
    Facts { changed: files.iter().map(|f| f.to_string()).collect(), ..Facts::default() }
}

#[test]
fn chain_runs_two_fix_rounds_then_the_user() {
    let (_dir, conn) = db();
    let team = team();
    let id = new_task(&conn, "d1", "r1");
    work(&conn, id, "d1", "m1", "built it");
    let out = settle(&conn, id, "/p/app", &facts(&["src/a.rs"]), &team).unwrap();
    assert_eq!(out.outcome, Outcome::Moved);
    let task = must_task(&conn, id).unwrap();
    assert_eq!((task.status, task.assigned.as_str(), task.then_member.as_str()), (TaskStatus::Todo, "r1", ""));
    assert_eq!(task.uncommitted, vec!["src/a.rs".to_string()]);
    let steps = |conn: &nixa_store::Connection| handoff_state(conn, id).unwrap();
    assert_eq!(steps(&conn).steps, 1);
    work(&conn, id, "r1", "m2", "1. src/a.rs is wrong\n2. no test\nFINDINGS: 2");
    settle(&conn, id, "/p/app", &facts(&[]), &team).unwrap();
    assert_eq!(must_task(&conn, id).unwrap().assigned, "d1");
    assert_eq!(steps(&conn).reviewer, "r1");
    let comments = task_comments(&conn, id).unwrap();
    let handoff = comments.iter().rev().find(|c| c.kind == CommentKind::Handoff).unwrap();
    assert!(handoff.text.starts_with("Hand-off from r1 name to d1 name: Checked by Nixa: no code change. No checks to run. The review by r1 name found 2 problems."));
    assert!(handoff.text.contains("Report from r1 name: \"\"\"1. src/a.rs is wrong"));
    work(&conn, id, "d1", "m1", "fixed both");
    settle(&conn, id, "/p/app", &facts(&["src/a.rs"]), &team).unwrap();
    assert_eq!(must_task(&conn, id).unwrap().assigned, "r1");
    assert!(steps(&conn).recheck);
    work(&conn, id, "r1", "m2", "still one problem FINDINGS: 1");
    settle(&conn, id, "/p/app", &facts(&[]), &team).unwrap();
    assert_eq!(must_task(&conn, id).unwrap().assigned, "s1");
    assert_eq!(steps(&conn).rounds, 2);
    assert_eq!(steps(&conn).steps, MAX_HANDOFFS);
    work(&conn, id, "s1", "m3", "fixed");
    let out = settle(&conn, id, "/p/app", &facts(&[]), &team).unwrap();
    assert_eq!(out.outcome, Outcome::User);
    let question = task_comments(&conn, id).unwrap().into_iter().rev().find(|c| c.kind == CommentKind::Question).unwrap();
    assert!(question.text.contains("s1 name finished the fix, and this task has already passed between members 4 times. Waiting for the user."));
    let task = must_task(&conn, id).unwrap();
    assert_eq!((task.status, task.blocked_on), (TaskStatus::Blocked, Some(BlockedOn::User)));
    assert!(task.needs_user);
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Speak { text } if text.contains("waits for you"))));
}

#[test]
fn clean_review_is_done_with_uncommitted_files() {
    let (_dir, conn) = db();
    let team = team();
    let id = new_task(&conn, "d1", "r1");
    work(&conn, id, "d1", "m1", "built");
    settle(&conn, id, "/p/app", &facts(&["src/a.rs"]), &team).unwrap();
    work(&conn, id, "r1", "m2", "all good. NO FINDINGS");
    let out = settle(&conn, id, "/p/app", &facts(&["src/b.rs"]), &team).unwrap();
    assert_eq!(out.outcome, Outcome::Done);
    let task = must_task(&conn, id).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.uncommitted, vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
    let last = task_comments(&conn, id).unwrap().into_iter().last().unwrap();
    assert!(last.text.contains("Earlier steps of this task changed 1 more file (src/a.rs). Done. Not committed"));
    assert_eq!(handoff_state(&conn, id).unwrap(), HandoffState::default());
}

#[test]
fn the_commit_note_follows_the_decision_mode() {
    assert!(super::settle::commit_tail(false).contains("the user commits"));
    let auto = super::settle::commit_tail(true);
    assert!(auto.contains("Nixa decides the git step") && !auto.contains("the user commits"));
}

#[test]
fn failed_checks_go_back_then_up_a_level() {
    let (_dir, conn) = db();
    let team = team();
    let id = new_task(&conn, "j1", "");
    nixa_store::review::set_handoff_state(&conn, id, &HandoffState { steps: 1, ..HandoffState::default() }).unwrap();
    work(&conn, id, "j1", "m5", "done");
    let failing = Facts { changed: vec!["src/a.rs".into()], checks: Some(CheckLine { failed: true, text: "in app: test failed (1 of 4)".into() }), ..Facts::default() };
    let out = settle(&conn, id, "/p/app", &failing, &team).unwrap();
    assert_eq!(out.outcome, Outcome::Back);
    assert!(matches!(&out.effects[0], Effect::Send { terminal, text } if terminal == "m5" && text.contains("$NIXA_BIN task status")));
    assert_eq!(must_task(&conn, id).unwrap().status, TaskStatus::Doing);
    set_status(&conn, id, TaskStatus::Review, None, false).unwrap();
    let out = settle(&conn, id, "/p/app", &failing, &team).unwrap();
    assert_eq!(out.outcome, Outcome::Moved);
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Stop { terminal } if terminal == "m5")));
    assert_eq!(must_task(&conn, id).unwrap().assigned, "d1");
    assert!(handoff_state(&conn, id).unwrap().escalated);
}

#[test]
fn plain_task_checks_stop_after_three_fails() {
    let (_dir, conn) = db();
    let team = team();
    let id = new_task(&conn, "", "");
    work(&conn, id, "x", "m6", "");
    let failing = Facts { checks: Some(CheckLine { failed: true, text: "in app: lint failed".into() }), ..Facts::default() };
    for _ in 0..2 {
        assert_eq!(settle(&conn, id, "/p/app", &failing, &team).unwrap().outcome, Outcome::Back);
        set_status(&conn, id, TaskStatus::Review, None, false).unwrap();
    }
    assert_eq!(settle(&conn, id, "/p/app", &failing, &team).unwrap().outcome, Outcome::User);
    let auto = Team { deep_first: true, ..team.clone() };
    let other = new_task(&conn, "", "");
    work(&conn, other, "x", "m6", "");
    for _ in 0..2 {
        assert_eq!(settle(&conn, other, "/p/app", &failing, &auto).unwrap().outcome, Outcome::Back);
        set_status(&conn, other, TaskStatus::Review, None, false).unwrap();
    }
    let out = settle(&conn, other, "/p/app", &failing, &auto).unwrap();
    assert_eq!(out.outcome, Outcome::Held);
    assert!(matches!(&out.effects[0], Effect::AskDeep { question, .. } if !question.contains("ask the user")));
    let task = must_task(&conn, other).unwrap();
    assert_eq!((task.status, task.needs_user), (TaskStatus::Review, false));
}

#[test]
fn pickup_sends_changed_code_to_the_space_reviewer() {
    let (_dir, conn) = db();
    let reviewer = nixa_store::agents::add_agent(
        &conn,
        &nixa_store::agents::AddAgent { title: "Reviewer".into(), purpose: "review code".into(), provider: "codex".into(), space: "s1".into(), role: "reviewer".into(), ..Default::default() },
        "test",
    )
    .unwrap();
    let mut team = team();
    team.people.clear();
    team.deep_first = true;
    team.seats = vec![ReviewSeat { id: reviewer.id.clone(), title: "Reviewer".into(), role: "reviewer".into(), space: "s1".into(), state: AgentState::Free, terminal: "m8".into(), can_resume: true }];
    let id = new_task(&conn, "", "");
    work(&conn, id, "w", "m7", "changed src/lib.rs and docs/a.md");
    let out = settle(&conn, id, "/p/app", &facts(&["src/lib.rs", "docs/a.md"]), &team).unwrap();
    assert_eq!(out.outcome, Outcome::Moved);
    let task = must_task(&conn, id).unwrap();
    assert_eq!(task.assigned, format!("custom:{}", reviewer.id));
    let state = handoff_state(&conn, id).unwrap();
    assert!(state.auto);
    let seat_gone_out = seat_gone(&conn, id, "The reviewer could not start.", &Team { seats: Vec::new(), ..team.clone() }).unwrap().unwrap();
    assert_eq!(seat_gone_out.outcome, Outcome::Held);
    assert!(matches!(&seat_gone_out.effects[0], Effect::AskDeep { question, .. } if question.contains("no agent reviewed the code")));
    assert_eq!(must_task(&conn, id).unwrap().status, TaskStatus::Review);
    let manual = new_task(&conn, "", "");
    work(&conn, manual, "w", "m9", "changed src/lib.rs");
    let out = settle(&conn, manual, "/p/app", &facts(&["src/lib.rs"]), &Team { deep_first: false, ..team }).unwrap();
    assert_eq!(out.outcome, Outcome::Done);
}

fn named(conn: &nixa_store::Connection, id: i64, files: &[&str]) {
    let mut input = CommentInput::new(CommentKind::Progress, "changed these");
    input.links.files = files.iter().map(|f| f.to_string()).collect();
    add_comment(conn, id, &input, &agent("w", "m7")).unwrap();
}

#[test]
fn no_checks_and_unnamed_files_hold_the_task() {
    let (_dir, conn) = db();
    let mut team = team();
    team.people.clear();
    let id = new_task(&conn, "", "");
    work(&conn, id, "w", "m7", "built it");
    let out = settle(&conn, id, "/p/app", &facts(&["src/a.rs", "src/b.rs"]), &team).unwrap();
    assert_eq!(out.outcome, Outcome::Held);
    let task = must_task(&conn, id).unwrap();
    assert_eq!((task.status, task.needs_user), (TaskStatus::Review, true));
    assert!(task.uncommitted.is_empty());
    assert!(task_comments(&conn, id).unwrap().last().unwrap().text.contains("did not name the changed files"));

    team.deep_first = true;
    let asked = new_task(&conn, "", "");
    work(&conn, asked, "w", "m7", "built it");
    let out = settle(&conn, asked, "/p/app", &facts(&["src/a.rs"]), &team).unwrap();
    assert_eq!(out.outcome, Outcome::Held);
    assert!(matches!(&out.effects[0], Effect::AskDeep { question, .. } if question.contains("no proof")));
    assert_eq!(must_task(&conn, asked).unwrap().status, TaskStatus::Review);

    let shown = new_task(&conn, "", "");
    work(&conn, shown, "w", "m7", "");
    named(&conn, shown, &["src/a.rs"]);
    add_comment(&conn, shown, &CommentInput::new(CommentKind::Result, "built it, 12 passed, 0 failed"), &agent("w", "m7")).unwrap();
    assert_eq!(settle(&conn, shown, "/p/app", &facts(&["src/a.rs"]), &team).unwrap().outcome, Outcome::Done);
    let written = new_task(&conn, "", "");
    work(&conn, written, "w", "m7", "Changed src/a.rs. Failed: 0.");
    assert_eq!(settle(&conn, written, "/p/app", &facts(&["src/a.rs"]), &team).unwrap().outcome, Outcome::Done);
    let backup = new_task(&conn, "", "");
    work(&conn, backup, "w", "m7", "Changed src/a.rs.bak only.");
    assert_eq!(settle(&conn, backup, "/p/app", &facts(&["src/a.rs"]), &team).unwrap().outcome, Outcome::Held);
    let passed = Facts { checks: Some(CheckLine { failed: false, text: "cargo test passed".into() }), ..facts(&["src/a.rs"]) };
    let checked = new_task(&conn, "", "");
    work(&conn, checked, "w", "m7", "built it");
    assert_eq!(settle(&conn, checked, "/p/app", &passed, &team).unwrap().outcome, Outcome::Done);
}

#[test]
fn a_failure_in_the_agents_report_holds_the_task() {
    let (_dir, conn) = db();
    let mut team = team();
    team.people.clear();
    let id = new_task(&conn, "", "");
    work(&conn, id, "w", "m7", "Fixed the prompt in src/a.rs.\nBrain lib: 942 passed, 1 failed.");
    let passed = Facts { checks: Some(CheckLine { failed: false, text: "cargo test passed".into() }), ..facts(&["src/a.rs"]) };
    let out = settle(&conn, id, "/p/app", &passed, &team).unwrap();
    assert_eq!(out.outcome, Outcome::Held);
    assert_eq!(must_task(&conn, id).unwrap().status, TaskStatus::Review);
    assert!(task_comments(&conn, id).unwrap().last().unwrap().text.contains("names a failing check: \"Brain lib: 942 passed, 1 failed.\""));
}

#[tokio::test]
async fn a_missing_start_point_holds_the_task_and_a_start_point_scopes_the_files() {
    let (_dir, conn) = db();
    let mut team = team();
    team.people.clear();
    let id = new_task(&conn, "", "");
    work(&conn, id, "w", "m7", "built it");
    named(&conn, id, &["src/a.rs"]);
    let unknown = Facts { no_base: true, checks: Some(CheckLine { failed: true, text: "never touch: .config/x.json".into() }), ..facts(&["src/a.rs", ".config/x.json"]) };
    assert_eq!(settle(&conn, id, "/p/app", &unknown, &team).unwrap().outcome, Outcome::Held);
    assert!(task_comments(&conn, id).unwrap().last().unwrap().text.contains("Nixa has no start point for this claim, so it cannot tell which of these files the task changed."));
    assert!(must_task(&conn, id).unwrap().uncommitted.is_empty());

    let repo = temp();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap().status.success());
    git(&["init", "-q"]);
    let app = repo.path().join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("old.txt"), "other work").unwrap();
    std::fs::write(app.join("still.txt"), "other work").unwrap();
    std::fs::write(repo.path().join("outside.txt"), "other work").unwrap();
    let folder = app.display().to_string();
    let base = crate::queue::git::base_of(&folder).await;
    std::fs::write(app.join("mine.txt"), "this task").unwrap();
    std::fs::write(app.join("old.txt"), "this task too").unwrap();
    let off = temp();
    let (scoped, _) = super::facts_with(off.path(), &folder, false, Some((&base, &folder))).await;
    let mut files = scoped.changed.clone();
    files.sort();
    assert_eq!((files, scoped.no_base), (vec!["mine.txt".to_string(), "old.txt".to_string()], false));
    let (blind, _) = super::facts_with(off.path(), &folder, false, Some(("", &folder))).await;
    assert!(blind.no_base);
    assert_eq!(blind.changed.len(), 3);
}

#[test]
fn blocked_developer_goes_up_and_helpers() {
    let (_dir, conn) = db();
    let team = team();
    let id = new_task(&conn, "d1", "");
    nixa_store::review::set_handoff_state(&conn, id, &HandoffState { steps: 1, ..HandoffState::default() }).unwrap();
    let claimed = claim_task(&conn, id, &agent("d1", "m1"), "").unwrap();
    assert!(claimed.ok);
    add_comment(&conn, id, &CommentInput::new(CommentKind::Question, "the API is missing"), &agent("d1", "m1")).unwrap();
    set_status(&conn, id, TaskStatus::Blocked, Some(BlockedOn::Other), false).unwrap();
    let out = escalate_blocked(&conn, id, &[], &team).unwrap().unwrap();
    assert_eq!(out.outcome, Outcome::Moved);
    assert_eq!(must_task(&conn, id).unwrap().assigned, "s1");
    assert!(same_file("/p/app", "/p/app/src/a.rs", "src/a.rs"));
    assert!(same_file("/p/app", "./src/a.rs", "src/a.rs"));
    assert!(!same_file("/p/app", "src/b.rs", "src/a.rs"));
}

fn joined_app() -> (tempfile::TempDir, std::sync::Arc<crate::app::App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(dir.path()).unwrap();
    (dir, app)
}

fn hook_done(terminal: &str) -> DoneEvent {
    DoneEvent { terminal: terminal.into(), agent: "claude".into(), cwd: "/tmp".into(), text: "All done.".into(), screen: String::new(), source: Source::Hook, at: crate::util::now_ms(), fallback: None }
}

#[test]
fn settle_outcomes_give_journal_words() {
    assert_eq!(super::closed_word(&Outcome::Done, false), Some("done"));
    assert_eq!(super::closed_word(&Outcome::User, false), Some("needs-user"));
    assert_eq!(super::closed_word(&Outcome::Held, false), Some("needs-user"));
    assert_eq!(super::closed_word(&Outcome::Held, true), None);
    assert_eq!(super::closed_word(&Outcome::Back, false), None);
    assert_eq!(super::closed_word(&Outcome::Moved, false), None);
    assert!(super::decide_line(true, 4, "q").contains("nixa_deep_ask"));
    assert!(!super::decide_line(false, 4, "q").contains("nixa_deep_ask"));
    assert!(!super::decide_line(false, 4, "q").contains("tell the user"));
    let failed = CheckLine { failed: true, text: "cargo test failed".into() };
    let passed = CheckLine { failed: false, text: "cargo test passed".into() };
    assert_eq!(super::check_state(Some(&failed)), crate::modes::scores::Checks::Failed);
    assert_eq!(super::check_state(Some(&passed)), crate::modes::scores::Checks::Passed);
    assert_eq!(super::check_state(None), crate::modes::scores::Checks::None);
}

fn rules_folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("docs")).unwrap();
    std::fs::write(dir.path().join("docs/nixa.md"), "## Checks\n- cargo test\n\n## Never\n- secrets/\n- *.lock\n").unwrap();
    dir
}

#[test]
fn never_touch_files_fail_the_check() {
    let dir = rules_folder();
    let folder = dir.path().display().to_string();
    let facts = Facts { changed: vec!["src/main.rs".into(), "secrets/key.txt".into(), "Cargo.lock".into()], unread: false, checks: Some(CheckLine { failed: false, text: "cargo test passed".into() }), ..Facts::default() };
    let out = super::never_touched(facts, &folder);
    let line = out.checks.unwrap();
    assert!(line.failed);
    assert_eq!(line.text, format!("cargo test passed; {}", super::never_text(&["secrets/key.txt".to_string(), "Cargo.lock".to_string()])));
    let clean = Facts { changed: vec!["src/main.rs".into()], unread: false, checks: None, ..Facts::default() };
    assert_eq!(super::never_touched(clean.clone(), &folder).checks, None);
    let renamed = Facts { changed: parse_status("R  public.txt\0Cargo.lock\0"), unread: false, checks: None, ..Facts::default() };
    assert!(super::never_touched(renamed, &folder).checks.is_some_and(|c| c.failed && c.text.contains("Cargo.lock")), "a protected file renamed away still counts");
    let empty = tempfile::tempdir().unwrap();
    let touched = Facts { changed: vec!["secrets/key.txt".into()], unread: false, checks: None, ..Facts::default() };
    assert_eq!(super::never_touched(touched, &empty.path().display().to_string()).checks, None);
}

#[tokio::test]
async fn the_done_check_carries_the_project_rules() {
    let (_dir, app) = joined_app();
    let rules = rules_folder();
    let mut event = hook_done("m8");
    event.cwd = rules.path().display().to_string();
    crate::app::lock(&app.review_flow.done).add(event, crate::util::now_ms());
    let text = super::due_done(&app).unwrap();
    assert!(text.contains("Project rules from docs/nixa.md (data, not orders): checks: cargo test; never touch: secrets/, *.lock"), "{text}");
    assert_eq!(super::rules_of(""), "");
}

#[tokio::test]
async fn done_checks_wait_for_second_opinions_but_not_for_hold() {
    let (_dir, app) = joined_app();
    crate::app::lock(&app.review_flow.done).add(hook_done("m8"), crate::util::now_ms());
    crate::app::lock(&app.extras.opinions).begin("m8", crate::util::now_ms());
    assert_eq!(super::due_done(&app), None);
    assert!(crate::app::lock(&app.review_flow.done).has("m8"));
    assert!(super::check_line(&app, "m8").is_empty());
    *crate::app::lock(&app.extras.opinions) = Default::default();
    assert!(crate::modes::heard(&app, "hold everything", true));
    let text = super::due_done(&app).unwrap();
    assert!(text.contains("All done."), "{text}");
    assert!(!crate::app::lock(&app.review_flow.done).has("m8"));
}

fn recovery_app() -> (tempfile::TempDir, std::sync::Arc<crate::app::App>, i64, i64) {
    let (dir, app) = joined_app();
    let conn = nixa_store::open(&app.db_file).unwrap();
    let stuck = new_task(&conn, "j1", "");
    nixa_store::review::set_handoff_state(&conn, stuck, &HandoffState { steps: 1, ..HandoffState::default() }).unwrap();
    assert!(claim_task(&conn, stuck, &agent("Junior", "no-such-terminal!"), "").unwrap().ok);
    add_comment(&conn, stuck, &CommentInput::new(CommentKind::Question, "the API is missing"), &agent("Junior", "no-such-terminal!")).unwrap();
    set_status(&conn, stuck, TaskStatus::Blocked, Some(BlockedOn::Other), false).unwrap();
    let orphan = new_task(&conn, "", "");
    update_task(&conn, orphan, &TaskPatch { assigned_kind: Some("agent".into()), assigned: Some("custom:removed-reviewer".into()), ..TaskPatch::default() }).unwrap();
    let state = HandoffState { auto: true, reviewer: "custom:removed-reviewer".into(), steps: 1, ..HandoffState::default() };
    nixa_store::review::set_handoff_state(&conn, orphan, &state).unwrap();
    (dir, app, stuck, orphan)
}

#[test]
fn the_queue_recovery_escalates_and_replaces_a_gone_reviewer_once() {
    let (_dir, app, stuck, orphan) = recovery_app();
    let first = super::recover_now(&app, people());
    let outcome = |id: i64| first.iter().filter(|(task, _)| task.id == id).map(|(_, s)| s.outcome).collect::<Vec<_>>();
    assert_eq!(outcome(stuck), vec![Outcome::Moved]);
    assert_eq!(outcome(orphan), vec![Outcome::Held]);
    let conn = nixa_store::open(&app.db_file).unwrap();
    let up = must_task(&conn, stuck).unwrap();
    assert_eq!((up.status, up.assigned.as_str()), (TaskStatus::Todo, "d1"));
    assert!(task_comments(&conn, stuck).unwrap().iter().any(|c| c.kind == CommentKind::Handoff && c.text.contains("Blocked on other: \"the API is missing\"")));
    let held = must_task(&conn, orphan).unwrap();
    assert_eq!(held.status, TaskStatus::Review);
    assert!(task_comments(&conn, orphan).unwrap().last().unwrap().text.contains("The saved agent removed-reviewer for this step is gone."));
    assert!(super::recover_now(&app, people()).is_empty());
}

#[test]
fn a_task_without_a_space_asks_the_deep_brain_first() {
    let (_dir, app) = joined_app();
    let conn = nixa_store::open(&app.db_file).unwrap();
    let input = TaskInput { title: "No space".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() };
    let task = add_task(&conn, &input, &Actor::new(AuthorKind::User, "Sam")).unwrap();
    assert!(super::team_of(&app, &conn, &task, people()).deep_first);
}

#[test]
fn a_review_with_no_change_is_raised_again_after_the_time_limit() {
    let limit = super::REVIEW_STALL_MS;
    assert!(!super::review_stalled(1_000, 0, 1_000 + limit - 1));
    assert!(super::review_stalled(1_000, 0, 1_000 + limit));
    assert!(!super::review_stalled(1_000, 1_000 + limit, 1_000 + limit + 5), "one ask per time limit");
    assert!(super::review_stalled(1_000, 1_000 + limit, 1_000 + limit * 2));
}

fn job_agent(conn: &nixa_store::Connection, title: &str, job: &str) -> String {
    let input = nixa_store::agents::AddAgent { title: title.into(), purpose: format!("{title} work"), job: job.into(), space: "s1".into(), ..Default::default() };
    nixa_store::agents::add_agent(conn, &input, "test").unwrap().id
}

fn blocked_junior(conn: &nixa_store::Connection, junior: &str) -> i64 {
    let id = new_task(conn, "", "");
    update_task(conn, id, &TaskPatch { assigned_kind: Some("agent".into()), assigned: Some(nixa_store::review::custom_ref(junior)), ..TaskPatch::default() }).unwrap();
    nixa_store::review::set_handoff_state(conn, id, &HandoffState { steps: 1, ..HandoffState::default() }).unwrap();
    assert!(claim_task(conn, id, &agent("Junior", "m1"), "").unwrap().ok);
    set_status(conn, id, TaskStatus::Blocked, Some(BlockedOn::Other), false).unwrap();
    id
}

#[test]
fn a_blocked_junior_custom_agent_moves_up_to_the_developer_job() {
    let (_dir, conn) = db();
    let junior = job_agent(&conn, "Junior", "junior-developer");
    let developer = job_agent(&conn, "Developer", "developer");
    job_agent(&conn, "Senior", "senior-developer");
    let id = blocked_junior(&conn, &junior);
    let out = escalate_blocked(&conn, id, &[], &Team::default()).unwrap().unwrap();
    assert_eq!(out.outcome, Outcome::Moved);
    let up = must_task(&conn, id).unwrap();
    assert_eq!((up.status, up.assigned_kind.as_str(), up.assigned), (TaskStatus::Todo, "agent", nixa_store::review::custom_ref(&developer)));
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Stop { terminal } if terminal == "m1")));
}

#[test]
fn escalation_skips_a_job_agent_of_another_space() {
    let (_dir, conn) = db();
    let junior = job_agent(&conn, "Junior", "junior-developer");
    let input = nixa_store::agents::AddAgent { title: "Far developer".into(), purpose: "Far work".into(), job: "developer".into(), space: "s9".into(), ..Default::default() };
    nixa_store::agents::add_agent(&conn, &input, "test").unwrap();
    let id = blocked_junior(&conn, &junior);
    let _ = escalate_blocked(&conn, id, &[], &Team::default()).unwrap();
    let after = must_task(&conn, id).unwrap();
    assert_eq!(after.assigned, nixa_store::review::custom_ref(&junior));
}

#[test]
fn escalation_keeps_a_terminal_that_holds_another_open_task() {
    let (_dir, conn) = db();
    let junior = job_agent(&conn, "Junior", "junior-developer");
    job_agent(&conn, "Developer", "developer");
    let other = new_task(&conn, "", "");
    assert!(claim_task(&conn, other, &agent("Junior", "m1"), "").unwrap().ok);
    let id = blocked_junior(&conn, &junior);
    let out = escalate_blocked(&conn, id, &[], &Team::default()).unwrap().unwrap();
    assert_eq!(out.outcome, Outcome::Moved);
    assert!(!out.effects.iter().any(|e| matches!(e, Effect::Stop { .. })));
}

#[test]
fn a_task_with_no_space_gets_the_home_space_for_its_check() {
    let conn = nixa_store::open_memory().unwrap();
    let by = nixa_store::tasks::Actor::new(nixa_store::tasks::AuthorKind::Nixa, "Nixa");
    nixa_store::tasks::add_task(&conn, &nixa_store::tasks::TaskInput { title: "one".into(), ..Default::default() }, &by).unwrap();
    let task = nixa_store::tasks::get_task(&conn, 1).unwrap().unwrap();
    let home = crate::spaces::Space { id: "nixa".into(), name: "nixa".into(), folder: "/tmp".into() };
    let moved = super::to_home_space(&conn, &task, &home).unwrap();
    assert_eq!((moved.space_id.as_str(), moved.space_name.as_str()), ("nixa", "nixa"));
    let team = Team { people: Vec::new(), seats: Vec::new(), worker: None, deep_first: true, folders: [("nixa".to_string(), "/tmp".to_string())].into_iter().collect() };
    assert_eq!(super::settle::folder_of(&moved, &team).as_deref(), Some("/tmp"));
}
