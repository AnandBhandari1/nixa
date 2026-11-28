use super::debate::{self, DebateAgent, Debater, Debates, Seen, Step, Tick, Verdict, Why};
use super::goals::{self, Entry, Goals};
use super::model_pick::{self, CustomRef, Hold, Pick};
use super::opinions::{self, Opinions};
use super::usage::{self, Counts, Usage};
use super::world::{WorldAgent, WorldInput, WorldJob, describe_world};
use super::{gaps, peer, shadow};
use crate::app::{App, lock};
use crate::deep::Ask;
use crate::review_flow::checks::{CheckResult, ChecksOutcome};
use nixa_store::agent_map::AgentMapRow;
use serde_json::{Map, json};

fn app() -> (tempfile::TempDir, std::sync::Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn failed_outcome(script: &str, tail: &str) -> ChecksOutcome {
    ChecksOutcome {
        folder: "/tmp/shop".into(),
        results: vec![CheckResult { package: ".".into(), script: script.into(), ok: false, ms: 10, summary: "failed".into(), tail: tail.into(), timed_out: false }],
        failed: true,
        none: String::new(),
    }
}

fn ask(state: &str, pick: &str, why: &str) -> Ask {
    serde_json::from_value(json!({"id": "d7", "question": "q", "at": 1000, "state": state, "pick": pick, "why": why, "doneAt": 4000, "kind": "model"})).unwrap()
}

fn row(job: &str, agent: &str, model: &str, member: &str) -> AgentMapRow {
    AgentMapRow { id: format!("r-{job}"), job: job.into(), agent: agent.into(), model: model.into(), member: member.into(), ..AgentMapRow::default() }
}

#[test]
fn tokens_and_turns_read_short() {
    assert_eq!(usage::format_tokens(950), "950");
    assert_eq!(usage::format_tokens(1500), "1.5k");
    assert_eq!(usage::format_tokens(12_345), "12k");
    assert_eq!(usage::format_tokens(2_000_000), "2M");
    let turn = usage::TurnUsage { ms: 2500, tokens: 1500, input: 1000, output: 400, cached: 100 };
    assert_eq!(usage::format_turn(&turn), "2.5 s, 1.5k");
}

#[test]
fn usage_samples_turns_by_day_and_keeps_30_days() {
    let dir = tempfile::tempdir().unwrap();
    let day = 86_400_000u64;
    let now = 100 * day;
    let mut usage = Usage::open(dir.path());
    assert!(usage.sample(false, Counts { input: 5, output: 0, cached: 0 }, now).is_none());
    usage.sample(true, Counts { input: 5, output: 0, cached: 0 }, now);
    usage.sample(true, Counts { input: 105, output: 20, cached: 3 }, now + 500);
    let turn = usage.sample(false, Counts { input: 105, output: 20, cached: 3 }, now + 1500).unwrap();
    assert_eq!((turn.input, turn.output, turn.cached, turn.tokens, turn.ms), (100, 20, 3, 123, 1500));
    let today = usage.today(now + 1500);
    assert_eq!((today.turns, today.tokens), (1, 123));
    usage.sample(false, Counts { input: 106, output: 20, cached: 3 }, now + 2000);
    assert_eq!(usage.today(now + 2000).tokens, 124);
    let again = Usage::open(dir.path());
    assert_eq!(again.today(now + 2000).turns, 1);
    let mut later = Usage::open(dir.path());
    later.sample(true, Counts::default(), now + 40 * day);
    later.sample(false, Counts { input: 1, output: 1, cached: 0 }, now + 40 * day);
    assert_eq!(later.by_day(now + 40 * day).len(), 1);
    assert!(later.text(now + 40 * day).contains("1 turns, 2 tokens"));
}

#[test]
fn usage_counts_survive_a_counter_reset() {
    let mut usage = Usage::memory();
    usage.sample(false, Counts { input: 50, output: 0, cached: 0 }, 1000);
    usage.sample(true, Counts { input: 50, output: 0, cached: 0 }, 1000);
    let turn = usage.sample(false, Counts { input: 7, output: 2, cached: 0 }, 2000).unwrap();
    assert_eq!(turn.tokens, 9);
}

#[test]
fn goal_report_passes_and_fails() {
    let mut goals = Goals::memory();
    goals.note(Entry::Route { kind: "check".into(), to: "deep".into(), id: Some("d1".into()) }, 10);
    goals.note(Entry::Deep { kind: "check".into(), id: "d1".into(), state: "answered".into(), ms: 30_000, row: Some(4) }, 20);
    goals.note(Entry::Down { brain: "b1".into() }, 30);
    goals.note(Entry::Back { brain: "b1".into(), ms: 20_000 }, 50);
    goals.note(Entry::Said { words: 12 }, 60);
    goals.note(Entry::Said { words: 40 }, 70);
    goals.note(Entry::Held, 80);
    let rows = goals.since(0);
    let lines = goals::report(&rows, &|row| row == 4, 1_000_000);
    assert_eq!(lines[0].value, "1 send held for the same work");
    assert!(lines[0].user);
    assert_eq!((lines[1].value.as_str(), lines[1].pass), ("1 of 1", Some(true)));
    assert_eq!((lines[2].value.as_str(), lines[2].pass), ("1 of 1", Some(false)));
    assert_eq!(lines[3].pass, Some(true));
    assert_eq!(lines[4].pass, Some(true));
    assert_eq!((lines[5].value.as_str(), lines[5].pass), ("26 words on average over 2 replies", Some(false)));
    let text = goals::text(&lines, 7);
    assert!(text.starts_with("Goals, last 7 days:"));
    assert!(text.contains("The user checks this one."));
    assert!(text.contains("FAIL."));
    let empty = goals::report(&[], &|_| false, 0);
    assert!(goals::text(&empty, 7).contains("No data yet."));
}

#[test]
fn goal_days_are_checked() {
    assert_eq!(goals::days_of(None), Ok(goals::GOAL_DAYS));
    assert_eq!(goals::days_of(Some(" 3 ")), Ok(3));
    assert_eq!(goals::days_of(Some("30")), Ok(30));
    assert!(goals::days_of(Some("0")).is_err());
    assert!(goals::days_of(Some("31")).is_err());
    assert!(goals::days_of(Some("x")).is_err());
}

#[test]
fn goals_file_keeps_30_days() {
    let dir = tempfile::tempdir().unwrap();
    let mut goals = Goals::open(dir.path(), 0);
    goals.note(Entry::Held, 1000);
    goals.note(Entry::Said { words: 3 }, 40 * 86_400_000);
    let reopened = Goals::open(dir.path(), 41 * 86_400_000);
    assert_eq!(reopened.since(0).len(), 1);
    let text = std::fs::read_to_string(dir.path().join(goals::FILE)).unwrap();
    assert!(text.contains("\"what\":\"said\""));
    assert!(!text.contains("\"held\""));
}

#[test]
fn world_line_lists_agents_jobs_and_waits() {
    let input = WorldInput {
        at: 10 * 60_000,
        mode: "Nixa decides".into(),
        clock: "10:05".into(),
        agents: vec![
            WorldAgent { terminal: "m1".into(), number: Some(1), agent: "codex".into(), place: "shop".into(), state: "working".into(), since: Some(0), ..WorldAgent::default() },
            WorldAgent { terminal: "m2".into(), number: None, agent: String::new(), folder: "/Users/someone/site".into(), state: "blocked".into(), since: None, ..WorldAgent::default() },
        ],
        jobs: vec![WorldJob { terminal: "m1".into(), number: Some(1), work: "fix the \"cart\"\nbutton".into(), rounds: 2, waits: true }],
        prompts: vec!["terminal 2".into()],
        presence: "away".into(),
        waiting: vec!["#1 a".into(), "#2 b".into(), "#3 c".into(), "#4 d".into()],
        work: String::new(),
    };
    let line = describe_world(&input);
    assert!(line.starts_with("Context: 10:05, Nixa decides. Agents: terminal 1 (m1) codex in shop: working 10 min; terminal ? (m2) an agent in site: waiting on a question."));
    assert!(line.contains("Jobs: terminal 1 (\"fix the cart button\", round 2, waits for the user)."));
    assert!(line.contains("Prompts: terminal 2. User: away."));
    assert!(line.ends_with("Tasks waiting for the user: #1 a; #2 b; #3 c; and 1 more."));
}

#[test]
fn world_line_shows_the_status_note_an_agent_reported() {
    let input = WorldInput {
        at: 60_000,
        mode: "Nixa decides".into(),
        clock: "10:06".into(),
        agents: vec![WorldAgent {
            terminal: "m3".into(),
            number: Some(3),
            agent: "codex".into(),
            place: "shop".into(),
            state: "blocked".into(),
            note: "needs the\nlogin \"key\"".into(),
            since: Some(0),
            ..WorldAgent::default()
        }],
        ..WorldInput::default()
    };
    let line = describe_world(&input);
    assert!(line.contains("terminal 3 (m3) codex in shop: waiting on a question 1 min (needs the login key)."), "{line}");
}

#[test]
fn tool_problems_and_reply_gaps() {
    assert_eq!(gaps::tool_problem("{\"status\":\"held\",\"reason\":\"needs your yes\"}").unwrap(), gaps::Problem { reason: "needs your yes".into(), refused: true });
    assert_eq!(gaps::tool_problem("{\"ok\":false,\"error\":\"ECONNREFUSED\"}").unwrap().reason, "ECONNREFUSED");
    assert!(gaps::tool_problem("{\"ok\":true}").is_none());
    assert!(gaps::tool_problem("Refused: plan mode").unwrap().refused);
    assert!(!gaps::tool_problem("Could not reach the engine").unwrap().refused);
    assert!(gaps::tool_problem("All fine, sent.").is_none());
    let found = gaps::reply_gap("Done with that. I cannot see your screen. Screen recording is not set up.").unwrap();
    assert!(found.sure);
    assert_eq!(found.reason, "I cannot see your screen. Screen recording is not set up.");
    assert!(gaps::reply_gap("I cannot push: only you can say yes to a push.").is_none());
    assert!(!gaps::reply_gap("I could not open the file.").unwrap().sure);
    assert!(gaps::reply_gap("I have no tool for the calendar.").unwrap().sure);
    let mut args = Map::new();
    args.insert("action".into(), json!("list"));
    args.insert("q".into(), json!("x"));
    assert_eq!(gaps::call_shape(&args), "action list; arguments: action, q");
    assert_eq!(gaps::call_shape(&Map::new()), "no arguments");
}

#[test]
fn tool_gaps_are_saved_per_turn() {
    let (_dir, app) = app();
    let mut args = Map::new();
    args.insert("terminal".into(), json!("m4"));
    super::after_tool(&app, "nixa_term_read", &args, "Tool error: connection refused", true);
    super::after_tool(&app, "nixa_term_send", &args, "Sent.", false);
    let rows = gaps::list(&app, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tool, "nixa_term_read");
    assert_eq!(rows[0].asked, "arguments: terminal");
    super::after_reply(&app, "I cannot read terminal m4 with nixa_term_read. The engine is not running.", "read m4", &["nixa_term_read".to_string()]);
    assert_eq!(gaps::list(&app, Some(nixa_store::gaps::GapKind::Gap)).unwrap().len(), 2);
    super::end_turn(&app);
    super::after_reply(&app, "I could not open it.", "", &[]);
    assert_eq!(gaps::list(&app, None).unwrap().len(), 2);
    assert!(gaps::list_text(&gaps::list(&app, None).unwrap()).contains("nixa_term_read"));
}

#[tokio::test]
async fn confirmed_gaps_become_one_linked_todo() {
    use nixa_store::gaps::{get_gap, set_gap_task};
    use nixa_store::tasks::{TaskFilter, TaskStatus, list_tasks};
    let (_dir, app) = app();
    let conn = nixa_store::open(&app.db_file).unwrap();
    let patch = nixa_store::spaces::SpacePatch { name: Some("nixa".into()), folder: Some("/tmp".into()), ..Default::default() };
    nixa_store::spaces::set_space(&conn, "nixa", &patch).unwrap();
    let mut args = Map::new();
    args.insert("q".into(), json!("x"));
    super::after_tool(&app, "nixa_calendar", &args, "Tool error: There is no tool named nixa_calendar. token=abcd1234efgh", true);
    super::after_tool(&app, "nixa_git_push", &Map::new(), "{\"status\":\"held\",\"reason\":\"needs your yes\"}", false);
    super::after_tool(&app, "nixa_term_read", &Map::new(), "Tool error: ECONNREFUSED", true);
    super::after_tool(&app, "web_read", &Map::new(), "Tool error: request timed out", true);
    super::after_tool(&app, "nixa_helper", &Map::new(), "Tool error: the helper exited with code 2", true);
    let rows = gaps::list(&app, None).unwrap();
    assert_eq!(rows.len(), 5);
    let id = |tool: &str| rows.iter().find(|r| r.tool == tool).unwrap().id;
    let file = |gap: i64, confirm: bool| {
        let mut call = Map::new();
        call.insert("id".into(), json!(gap));
        call.insert("confirm".into(), json!(confirm));
        let app = app.clone();
        async move { super::call(&app, super::GAP_TASK_TOOL, &call).await.unwrap() }
    };
    let first = file(id("nixa_calendar"), false).await.unwrap();
    assert!(first.starts_with(&format!("Filed tool gap #{} as task #", id("nixa_calendar"))) && first.contains("It is not started."));
    let again = file(id("nixa_calendar"), false).await.unwrap();
    assert!(again.contains("already filed") && again.contains("No new task was made."));
    let all = list_tasks(&conn, &TaskFilter::default()).unwrap();
    assert_eq!(all.len(), 1);
    let task = &all[0];
    assert_eq!((task.status, task.space_id.as_str(), task.link_source.as_str(), task.link_status.as_str()), (TaskStatus::Todo, "nixa", "tool_gap", "capability"));
    assert_eq!(task.link_id, id("nixa_calendar").to_string());
    assert!(task.terminal_id.is_empty() && task.claimed_by.is_empty());
    assert!(task.title.starts_with("Tool gap in nixa_calendar: There is no tool named nixa_calendar."));
    assert!(task.description.contains("Evidence:") && task.description.contains("Expected behavior:") && task.description.contains("arguments: q"));
    assert!(!task.description.contains("abcd1234efgh") && !task.title.contains("abcd1234efgh"));
    assert_eq!(get_gap(&conn, id("nixa_calendar")).unwrap().unwrap().task_id, task.id);
    assert!(gaps::list_text(&gaps::list(&app, None).unwrap()).contains(&format!("(capability) from tool nixa_calendar, 1 times: There is no tool named nixa_calendar. token=<redacted> (asked: arguments: q), task #{}", task.id)));
    set_gap_task(&conn, id("nixa_calendar"), 0).unwrap();
    assert!(file(id("nixa_calendar"), false).await.unwrap().contains(&format!("task #{}", task.id)));
    assert_eq!(get_gap(&conn, id("nixa_calendar")).unwrap().unwrap().task_id, task.id);
    let held = gaps::list(&app, Some(nixa_store::gaps::GapKind::Refusal)).unwrap()[0].id;
    assert!(file(held, true).await.unwrap_err().contains("approval hold"));
    assert!(file(id("nixa_term_read"), true).await.unwrap_err().contains("setup problem"));
    assert!(file(id("web_read"), true).await.unwrap_err().contains("passing failure"));
    assert!(file(id("nixa_helper"), false).await.unwrap_err().contains("confirm true"));
    assert_eq!(list_tasks(&conn, &TaskFilter::default()).unwrap().len(), 1);
    assert!(file(id("nixa_helper"), true).await.unwrap().contains("Filed tool gap"));
    assert_eq!(list_tasks(&conn, &TaskFilter::default()).unwrap().len(), 2);
    assert!(file(9999, true).await.unwrap_err().contains("There is no tool gap #9999"));
    assert!(super::call(&app, super::GAP_TASK_TOOL, &Map::new()).await.unwrap().unwrap_err().contains("id"));
}

#[tokio::test]
async fn gap_tasks_go_to_the_space_the_gap_came_from() {
    use nixa_store::spaces::{SpacePatch, set_active_space, set_space};
    let (_dir, app) = app();
    let conn = nixa_store::open(&app.db_file).unwrap();
    for (id, folder) in [("nixa", "/tmp/nixa-home"), ("shop", "/tmp/shop")] {
        set_space(&conn, id, &SpacePatch { name: Some(id.into()), folder: Some(folder.into()), ..Default::default() }).unwrap();
    }
    let mut shop = Map::new();
    shop.insert("space".into(), json!("shop"));
    let missing = "Tool error: There is no tool named nixa_calendar.";
    super::after_tool(&app, "nixa_calendar", &shop, missing, true);
    super::after_tool(&app, "nixa_calendar", &Map::new(), missing, true);
    super::end_turn(&app);
    set_active_space(&conn, Some("shop")).unwrap();
    super::after_reply(&app, "I have no tool for invoices.", "make an invoice", &[]);
    let gone = nixa_store::gaps::GapInput { kind: nixa_store::gaps::GapKind::Gap, source: nixa_store::gaps::GapSource::Tool, tool: "nixa_old".into(), asked: String::new(), reason: "not supported".into(), space_id: "gone".into() };
    nixa_store::gaps::note_gap(&conn, &gone, &[]).unwrap();
    let rows = gaps::list(&app, None).unwrap();
    assert_eq!(rows.len(), 4);
    let gap = |tool: &str, space: &str| rows.iter().find(|r| r.tool == tool && r.space_id == space).unwrap().id;
    let space_of = |id: i64| {
        gaps::file_task(&app, id, false).unwrap();
        let gap = nixa_store::gaps::get_gap(&conn, id).unwrap().unwrap();
        nixa_store::tasks::get_task(&conn, gap.task_id).unwrap().unwrap().space_id
    };
    assert_eq!(space_of(gap("nixa_calendar", "shop")), "shop");
    assert_eq!(space_of(gap("nixa_calendar", "")), "nixa");
    assert_eq!(space_of(gap("", "shop")), "shop");
    assert_eq!(space_of(gap("nixa_old", "gone")), "nixa");
    let task = nixa_store::tasks::get_task(&conn, nixa_store::gaps::get_gap(&conn, gap("nixa_calendar", "shop")).unwrap().unwrap().task_id).unwrap().unwrap();
    assert_eq!(task.space_name, "shop");
    assert!(task.description.contains("- Space: shop"));
}

#[test]
fn gap_task_text_hides_private_words_and_needs_a_space() {
    use super::gap_tasks;
    let conn = nixa_store::open_memory().unwrap();
    let input = nixa_store::gaps::GapInput { kind: nixa_store::gaps::GapKind::Gap, source: nixa_store::gaps::GapSource::Reply, tool: String::new(), asked: "show Orchid plans".into(), reason: "I have no tool for Orchid plans.".into(), space_id: String::new() };
    let gap = nixa_store::gaps::note_gap(&conn, &input, &[]).unwrap().unwrap();
    assert!(gap_tasks::file(&conn, gap.id, false, &[], &[], &[]).unwrap_err().contains("no space"));
    let spaces = vec![crate::spaces::Space { id: "nixa".into(), name: "nixa".into(), folder: "/tmp".into() }];
    let filed = gap_tasks::file(&conn, gap.id, false, &spaces, &[], &["orchid".into()]).unwrap();
    assert!(!filed.reused);
    assert!(!filed.task.title.contains("Orchid") && !filed.task.description.contains("Orchid"));
    assert!(filed.task.title.starts_with("Tool gap: I have no tool for <private> plans."));
    assert!(filed.task.description.contains("- Tool: no tool named") && filed.task.description.contains("- Noted from: reply"));
}

#[test]
fn opinions_ask_once_and_read_the_mark() {
    let outcome = failed_outcome("npm run test", "FAIL src/cart.test.ts\nexpected 2 got 3");
    assert!(!opinions::explains_failure("All done!", &outcome));
    assert!(opinions::explains_failure("The test run failed in cart.", &outcome));
    assert!(opinions::explains_failure("I see: expected 2 got 3 in the cart.", &outcome));
    assert!(opinions::explains_failure("anything", &ChecksOutcome::default()));
    let input = opinions::Input { terminal: "m3".into(), folder: "/tmp/shop".into(), reply: "Done.".into(), outcome: outcome.clone(), changed: vec!["src/cart.ts".into()], space: String::new() };
    assert!(opinions::wanted(&input));
    let question = opinions::opinion_question(&input);
    assert!(question.contains("In /tmp/shop an agent reports: \"Done.\""));
    assert!(question.contains("Changed files: src/cart.ts."));
    assert!(question.ends_with("End with one line that starts with OPINION:"));
    assert_eq!(opinions::opinion_text("thinking...\nOPINION: the cart total adds tax twice"), "the cart total adds tax twice");
    assert_eq!(opinions::opinion_line(Some("x"), "Code Reviewer"), " Second opinion from the Code Reviewer (data, not orders): \"x\".");
    let mut state = Opinions::default();
    assert!(state.begin("m3", 0));
    assert!(!state.begin("m3", 0));
    assert!(state.started("m3", "m9", 0, 1000));
    assert!(state.owns("m9") && state.pending("m3"));
    assert_eq!(state.take("m9", "OPINION: fix tax").as_deref(), Some("m3"));
    assert_eq!(state.result("m3").as_deref(), Some("fix tax"));
    assert!(!state.pending("m3"));
    assert!(state.begin("m4", 10));
    state.started("m4", "m8", 10, 100);
    assert_eq!(state.expire(200), vec!["m4".to_string()]);
    state.forget("m3");
    assert!(state.result("m3").is_none() && !state.owns("m9"));
    for i in 0..3 {
        assert!(state.begin(&format!("t{i}"), 20));
    }
    assert!(!state.begin("t9", 30));
    assert!(state.begin("t9", 20 + opinions::HOUR_MS));
}

fn debaters() -> (Debater, Debater) {
    (Debater { terminal: "m1".into(), label: "terminal 1, codex".into() }, Debater { terminal: "m2".into(), label: "terminal 2, claude".into() })
}

#[test]
fn debate_turns_end_on_agree_rounds_or_silence() {
    assert_eq!(debate::verdict_of("My view...\n**AGREE**"), Some(Verdict::Agree));
    assert_eq!(debate::verdict_of("Verdict: disagree\n\n"), Some(Verdict::Disagree));
    assert_eq!(debate::verdict_of("I agree with most of it but this is a long line that is no verdict at all, really"), None);
    assert_eq!(debate::rounds_of(Some(9.0)), 5);
    assert_eq!(debate::rounds_of(None), 3);
    let (a, b) = debaters();
    let mut state = debate::new_debate("Use SQLite?", a.clone(), b.clone(), Some(2.0));
    assert!(debate::first_prompt(&state).contains("You debate with terminal 2, claude."));
    match debate::next_turn(&mut state, Some("Yes, it is simple.\nAGREE")) {
        debate::Next::Turn { side, round, text } => {
            assert_eq!((side, round), (debate::Side::B, 1));
            assert!(text.contains("The other side (terminal 1, codex) replied."));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(debate::next_turn(&mut state, Some("Agreed.\nAGREE")), debate::Next::Conclude(debate::End { why: Why::Agree, .. })));
    let mut state = debate::new_debate("p", a.clone(), b.clone(), Some(1.0));
    debate::next_turn(&mut state, Some("x\nDISAGREE"));
    let end = match debate::next_turn(&mut state, Some("y\nDISAGREE")) {
        debate::Next::Conclude(end) => end,
        other => panic!("{other:?}"),
    };
    assert_eq!(end.why, Why::Rounds);
    let event = debate::debate_event(&state, &end);
    assert!(event.starts_with("Event: Debate: The debate the user asked for has ended: all 1 round were used."));
    assert!(event.contains("- Round 1, side B: \"y DISAGREE\" (DISAGREE)"));
    let mut state = debate::new_debate("p", a, b, None);
    let end = match debate::next_turn(&mut state, Some("  ")) {
        debate::Next::Conclude(end) => end,
        other => panic!("{other:?}"),
    };
    assert_eq!(debate::end_text(&state, &end), "terminal 1, codex sent no reply, so the debate stopped in round 1");
    assert!(debate::fallback_conclusion(&state, &end).contains("No replies came."));
}

#[test]
fn debates_pass_turns_and_watch_the_waiting_side() {
    let (a, b) = debaters();
    let mut debates = Debates::default();
    assert!(debates.start("p", a.clone(), a.clone(), None, 0).is_err());
    let (step, missed) = debates.start("Is it ready?", a.clone(), b.clone(), Some(1.0), 0).unwrap();
    assert!(missed.is_none());
    assert!(matches!(&step, Step::Send { terminal, .. } if terminal == "m1"));
    assert!(debates.start("again", a.clone(), b.clone(), None, 0).is_err());
    debates.sent("m1", Some("old".into()), 0);
    assert_eq!(debates.take("m2", "not your turn"), Step::Nothing);
    assert_eq!(debates.tick(1000, debate::TURN_MS), Tick::Nothing);
    assert_eq!(debates.tick(debate::CHECK_MS + 1, debate::TURN_MS), Tick::Status("m1".into()));
    assert_eq!(debates.status_seen("m1", "idle", 4000, 100), Seen::Nothing);
    assert_eq!(debates.status_seen("m1", "working", 5000, 100), Seen::Nothing);
    assert_eq!(debates.status_seen("m1", "idle", 6000, 100), Seen::Nothing);
    let read = debates.status_seen("m1", "idle", 6200, 100);
    assert_eq!(read, Seen::Read { terminal: "m1".into(), count: 0 });
    let step = debates.read_done("m1", 0, "Ready.\nAGREE");
    assert!(matches!(&step, Step::Send { terminal, line: Some(line), .. } if terminal == "m2" && line.contains("terminal 1, codex answered (AGREE)")));
    debates.sent("m2", None, 7000);
    assert_eq!(debates.read_done("m1", 0, "late"), Step::Nothing);
    let end = debates.take("m2", "Not ready.\nDISAGREE");
    assert!(matches!(&end, Step::End { event, .. } if event.contains("Event: Debate:")));
    assert!(debates.status().is_none());
    assert!(debates.conclude("  ").is_err());
    assert_eq!(debates.conclude("Wait one day.").unwrap(), "Wait one day.");
    assert!(debates.conclude("again").is_err());
    let (_, _) = debates.start("p2", a.clone(), b.clone(), None, 0).unwrap();
    assert!(matches!(debates.status_seen("m1", "gone", 10, 100), Seen::Step(Step::End { .. })));
    assert!(debates.missed().is_some());
    let (_, _) = debates.start("p3", a, b, None, 0).unwrap();
    assert!(matches!(debates.tick(debate::TURN_MS + 1, debate::TURN_MS), Tick::Step(Step::End { line, .. }) if line.contains("gave no reply in 10 minutes")));
}

#[test]
fn debaters_are_found_by_number_member_or_agent() {
    let agents = vec![
        DebateAgent { terminal: "m1".into(), number: Some(1), agent: "codex".into(), place: "shop / main".into(), state: "idle".into(), ..DebateAgent::default() },
        DebateAgent { terminal: "m2".into(), number: Some(2), agent: "claude".into(), member: Some(("Code Reviewer".into(), "code-reviewer".into())), folder: "/tmp/site".into(), state: "done".into(), ..DebateAgent::default() },
        DebateAgent { terminal: "m3".into(), number: Some(3), agent: "agy".into(), state: "idle".into(), ..DebateAgent::default() },
    ];
    assert_eq!(debate::find_debater("terminal 2", &agents).unwrap().terminal, "m2");
    assert_eq!(debate::find_debater("#1", &agents).unwrap().terminal, "m1");
    assert_eq!(debate::find_debater("the code reviewer", &agents).unwrap().terminal, "m2");
    assert_eq!(debate::find_debater("Codex", &agents).unwrap().terminal, "m1");
    assert_eq!(debate::find_debater("antigravity", &agents).unwrap().terminal, "m3");
    assert_eq!(debate::find_debater("m3", &agents).unwrap().terminal, "m3");
    assert!(debate::find_debater("terminal 9", &agents).unwrap_err().contains("No open agent in terminal 9"));
    assert!(debate::find_debater("gemini", &agents).is_err());
    assert_eq!(debate::debater_label(&agents[0]), "terminal 1, codex in shop");
    assert_eq!(debate::debater_label(&agents[1]), "terminal 2, the Code Reviewer (claude) in site");
}

#[tokio::test]
async fn debate_tool_picks_the_second_agent_itself() {
    let (_dir, app) = app();
    let mut args = Map::new();
    args.insert("action".into(), json!("start"));
    args.insert("point".into(), json!("tabs or spaces"));
    args.insert("a".into(), json!("nixa"));
    let first = debate::call(&app, &args).await.unwrap_err();
    assert!(first.contains("No idle open agent is free"), "{first}");
    assert!(lock(&app.extras.debates).pending.is_none());
    let mut reply = Map::new();
    reply.insert("action".into(), json!("reply"));
    reply.insert("text".into(), json!("AGREE"));
    assert!(debate::call(&app, &reply).await.is_err());
}
#[test]
fn peer_output_is_parsed() {
    assert_eq!(peer::parse_lsof_pids("p123\nf5\np456\nx\n"), vec![123, 456]);
    assert_eq!(peer::parse_executable("p1\nfcwd\nn/tmp\nftxt\nn/Applications/Nixa.app/Contents/MacOS/nixa-app\n").as_deref(), Some("/Applications/Nixa.app/Contents/MacOS/nixa-app"));
    assert_eq!(peer::parse_executable("p1\n"), None);
    let parents = peer::parse_parents("  10   1\n 20 10\n 30 20\n bad line\n 40 40\n");
    assert_eq!(peer::chain_of(&parents, 30), vec![30, 20, 10, 1]);
    assert_eq!(peer::chain_of(&parents, 40), vec![40]);
    assert_eq!(peer::program_name("/opt/homebrew/bin/codex"), "codex");
}

#[tokio::test]
async fn a_call_counts_only_from_inside_its_terminal() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut child = tokio::process::Command::new("/usr/bin/nc")
        .arg0("NIXA_TERM_ID=m9")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .env("NIXA_TERM_ID", "m9")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut other = tokio::process::Command::new("/bin/sleep").arg("30").kill_on_drop(true).spawn().unwrap();
    let (_stream, addr) = tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept()).await.unwrap().unwrap();
    let pid = child.id();
    assert_eq!(peer::from_terminal(Some(addr), "m9", None).await, Err(peer::outside_text("m9")), "the terminal name in argv or env is not proof");
    assert_eq!(peer::from_terminal(Some(addr), "m9", other.id()).await, Err(peer::outside_text("m9")));
    assert_eq!(peer::from_terminal(Some(addr), "m3", pid).await, Ok(()));
    assert_eq!(peer::from_terminal(None, "m9", pid).await, Err(peer::unseen_text("m9")));
    let far: std::net::SocketAddr = format!("10.1.2.3:{}", addr.port()).parse().unwrap();
    assert_eq!(peer::from_terminal(Some(far), "m9", pid).await, Err(peer::unseen_text("m9")));
    let _own = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (_kept, mine) = listener.accept().await.unwrap();
    assert_eq!(peer::from_terminal(Some(mine), "m9", pid).await, Err(peer::outside_text("m9")));
    child.kill().await.ok();
    other.kill().await.ok();
}

#[test]
fn model_pick_question_and_answers() {
    let rows = vec![row("frontend fixes", "claude", "sonnet", ""), row("code review", "codex", "gpt-5", "Reviewer"), row("docs", "", "", "")];
    let found = model_pick::rows_for_task(&rows, "review the cart code", 2);
    assert_eq!(found[0].1, 2);
    let task = model_pick::TaskRef { id: 7, title: "Review the cart".into(), description: "code review please".into(), ..model_pick::TaskRef::default() };
    let input = model_pick::ask_for(&task, &rows, &["Cheap first".to_string()], &["Custom agents in shop: none.".to_string()]);
    assert_eq!(input.kind.as_deref(), Some(model_pick::ASK_KIND));
    assert_eq!(input.options.last().map(String::as_str), Some(model_pick::OWN));
    assert!(input.notes.contains("Criteria: 1) Cheap first"));
    assert!(input.question.starts_with("Which row of the agent map fits task #7"));
    let customs = vec![CustomRef { id: "a9".into(), title: "Reviewer".into(), space: "s1".into(), state: String::new() }];
    let installed = |agent: &str| agent == "claude";
    let by_row = model_pick::pick_from_answer(&ask("answered", "row 1", "fast"), &rows, &customs, &installed, "s1");
    assert_eq!((by_row.assigned.as_str(), by_row.model.as_str(), by_row.number), ("claude", "claude::sonnet", Some(1)));
    let member = model_pick::pick_from_answer(&ask("answered", "row 2", ""), &rows, &customs, &installed, "s1");
    assert_eq!(member.assigned, "custom:a9");
    let custom = model_pick::pick_from_answer(&ask("answered", "a9 please", ""), &rows, &customs, &installed, "s1");
    assert_eq!(custom.assigned, "custom:a9");
    assert!(model_pick::pick_from_answer(&ask("answered", "a9", ""), &rows, &customs, &installed, "s2").why.contains("belongs to another space"));
    let missing = model_pick::pick_from_answer(&ask("answered", "codex::gpt-5.", ""), &rows, &customs, &installed, "s1");
    assert!(missing.none && missing.why.contains("codex is not installed"));
    let hold = model_pick::pick_from_answer(&ask("answered", "do not start: cancel it", "dup"), &rows, &customs, &installed, "s1");
    assert_eq!(hold.hold, Some(Hold::Cancel));
    assert!(model_pick::pick_line(&hold).contains("Nixa cancelled the task"));
    assert!(model_pick::pick_from_answer(&ask("timeout", "", ""), &rows, &customs, &installed, "s1").why.contains("no answer in time"));
    assert!(model_pick::pick_from_answer(&ask("answered", "row 3", ""), &rows, &customs, &installed, "s1").why.contains("not an agent start"));
    assert!(model_pick::pick_from_answer(&ask("answered", "row 9", ""), &rows, &customs, &installed, "s1").why.contains("no row 9"));
    let raw = serde_json::to_string(&by_row).unwrap();
    assert_eq!(model_pick::read_pick(&raw), Some(by_row.clone()));
    assert!(model_pick::read_pick("{}").is_none());
    assert!(model_pick::pick_line(&by_row).starts_with("Deep brain d7 picked row 1 (frontend fixes): claude::sonnet for this task. Why: fast."));
    assert!(!Pick::default().none);
    assert!(model_pick::map_text(&rows, &[], &[], "").contains("row 3: docs, no pick"));
}

#[test]
fn shadow_text_helpers() {
    assert_eq!(shadow::next_step("Fixed it. Want me to run the tests?").as_deref(), Some("Want me to run the tests?"));
    assert!(shadow::next_step("All done.").is_none());
    assert!(shadow::step_note("Done. Should I push to main?", "").contains("Nixa or the deep brain decides it"));
    assert!(shadow::plain_yes("Yes") && shadow::plain_yes("ok please"));
    assert!(!shadow::plain_yes("yes?") && !shadow::plain_yes("no") && !shadow::plain_yes("yes but not now wait"));
    assert!(shadow::is_work_request("Can you please fix the login page"));
    assert!(!shadow::is_work_request("yes"));
    assert!(!shadow::is_work_request("what time is it"));
    let gate = |t: &str| t.contains("push");
    let offer = shadow::offered_work("Tests pass. Want me to update the docs or push the branch?", &gate).unwrap();
    assert_eq!(offer.safe, "Want me to update the docs");
    assert_eq!(offer.gated.as_deref(), Some("push the branch?"));
    assert!(shadow::offered_work("Do you want me to dance?", &gate).is_none());
    assert_eq!(shadow::asked_user("Done. Which name do you like? Thanks.").as_deref(), Some("Which name do you like?"));
    assert_eq!(shadow::change_summary(&["a.rs".to_string()]), "1 file: a.rs");
    let many: Vec<String> = ["src/a/x.rs", "src/a/y.rs", "src/b/z.rs", "docs/r.md", "lib.rs"].iter().map(|s| s.to_string()).collect();
    assert_eq!(shadow::change_summary(&many), "5 files, 2 in src/a, 1 in src/b, 1 in docs and more");
}

#[tokio::test]
async fn tools_and_view_answer() {
    let (_dir, app) = app();
    let names: Vec<String> = super::specs().into_iter().map(|s| s.name).collect();
    assert_eq!(names, super::NAMES.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    let goals = super::call(&app, super::GOALS_TOOL, &Map::new()).await.unwrap().unwrap();
    assert!(goals.contains("Goals, last 7 days:") && goals.contains("No brain usage in the last 30 days."));
    let gaps = super::call(&app, super::GAPS_TOOL, &Map::new()).await.unwrap().unwrap();
    assert_eq!(gaps, "No tool gaps or refusals are noted.");
    let map = super::call(&app, model_pick::MAP_TOOL, &Map::new()).await.unwrap().unwrap();
    assert!(map.starts_with("Agent map: 0 rows."));
    assert!(super::call(&app, "nixa_nothing", &Map::new()).await.is_none());
    super::said(&app, "one two three");
    super::held(&app);
    super::routed(&app, "check", true, Some("d1"));
    let lines = super::goal_lines(&app, crate::util::now_ms());
    assert_eq!(lines[0].value, "1 send held for the same work");
    assert!(super::view(&app)["usage"]["today"].is_object());
    assert!(!super::busy(&app, "m1"));
    assert_eq!(super::prompt_note(&app, "m1"), "");
    assert!(super::world::context(&app).starts_with("Context: "));
}

#[test]
fn usage_sampler_reads_the_brain_counters() {
    let (_dir, app) = app();
    lock(&app.brain).busy = true;
    super::sample_usage(&app, 1000);
    lock(&app.brain).usage.input += 40;
    lock(&app.brain).busy = false;
    super::sample_usage(&app, 3000);
    assert_eq!(lock(&app.extras.usage).today(3000).tokens, 40);
}

#[test]
fn automatic_debate_picks_an_idle_partner_in_the_same_space_first() {
    let agents = vec![
        DebateAgent { terminal: "m1".into(), agent: "codex".into(), place: "shop".into(), state: "idle".into(), ..DebateAgent::default() },
        DebateAgent { terminal: "m2".into(), agent: "claude".into(), place: "site".into(), state: "idle".into(), ..DebateAgent::default() },
        DebateAgent { terminal: "m3".into(), agent: "agy".into(), place: "shop".into(), state: "working".into(), ..DebateAgent::default() },
        DebateAgent { terminal: "m4".into(), agent: "omp".into(), place: "shop".into(), state: "done".into(), ..DebateAgent::default() },
    ];
    let none = |_: &str| false;
    assert_eq!(debate::partner_of(Some("m1"), &agents, &none).unwrap().terminal, "m4");
    let m4_busy = |t: &str| t == "m4";
    assert_eq!(debate::partner_of(Some("m1"), &agents, &m4_busy).unwrap().terminal, "m2");
    assert_eq!(debate::partner_of(None, &agents, &none).unwrap().terminal, "m1");
    let all_busy = |_: &str| true;
    assert!(debate::partner_of(Some("m1"), &agents, &all_busy).is_none());
}
