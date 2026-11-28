use super::answer::{Answer, AskWord, GateWord, read_answer};
use super::driver::{self, Ctx, Effect, Settle};
use super::model::{self, check_workflow, lint, next_node_id, problems};
use super::routes::store_request;
use super::run::*;
use super::who::{Custom, Pick, WhoOptions, WhoPick, who_of};
use crate::tools::Args;
use nixa_store::Connection;
use nixa_store::agent_map::AgentMapRow;
use nixa_store::tasks::{self, Actor, AuthorKind, BlockedOn, CommentInput, CommentKind, TaskInput, TaskStatus};
use serde_json::{Value, json};

fn flow_json() -> Value {
    json!({
        "name": "build-review",
        "title": "Build then review",
        "when": "code work that needs a review",
        "nodes": {
            "n1": {"type": "start", "title": "Start"},
            "n2": {"type": "stage", "title": "Build", "who": {"kind": "agent", "id": "codex"}, "prompt": "Build it.", "retry": 1, "limit": 3, "skills": ["tdd", "tdd", " "]},
            "n3": {"type": "stage", "title": "Review", "who": {"kind": "agent", "id": "omp"}, "prompt": "Review it.", "limit": 3},
            "n4": {"type": "stage", "title": "Sum up", "who": {"kind": "agent", "id": "codex"}, "prompt": "Sum up.", "gate": "user"},
            "n5": {"type": "end", "title": "End"}
        },
        "links": [
            {"from": "n1", "to": "n2", "kind": "pass"},
            {"from": "n2", "to": "n3", "kind": "pass"},
            {"from": "n3", "to": "n4", "kind": "pass"},
            {"from": "n3", "to": "n2", "kind": "revise"},
            {"from": "n4", "to": "n5", "kind": "pass"}
        ],
        "layout": {"n1": {"x": 10, "y": 20}, "zz": {"x": 1, "y": 1}}
    })
}

fn flow() -> model::Workflow {
    check_workflow(&flow_json(), None).unwrap()
}

fn with(mut raw: Value, path: &[&str], value: Value) -> Value {
    let mut at = &mut raw;
    for key in &path[..path.len() - 1] {
        at = match key.parse::<usize>() {
            Ok(index) => &mut at[index],
            Err(_) => &mut at[*key],
        };
    }
    let last = path[path.len() - 1];
    match last.parse::<usize>() {
        Ok(index) => at[index] = value,
        Err(_) => at[last] = value,
    }
    raw
}

fn args(value: Value) -> Args {
    value.as_object().cloned().unwrap_or_default()
}

#[test]
fn checks_a_good_workflow() {
    let f = flow();
    assert!(lint(&f).is_empty(), "{:?}", lint(&f));
    assert_eq!(f.limits.max_steps, 50);
    assert_eq!(f.links[0].id, "l1");
    assert_eq!(f.nodes["n2"].skills, vec!["tdd".to_string()]);
    assert_eq!(f.layout.len(), 1);
    assert_eq!(next_node_id(&f), "n6");
    let alias = check_workflow(&with(flow_json(), &["limits"], json!({"maxSteps": 7})), None).unwrap();
    assert_eq!(alias.limits.max_steps, 7);
    let saved = model::to_json(&f);
    assert_eq!(saved["nodes"]["n2"]["type"], "stage");
    assert_eq!(saved["nodes"]["n1"]["who"]["kind"], "");
    assert_eq!(saved["nodes"]["n4"]["gate"], "user");
    assert_eq!(saved["limits"]["max_steps"], 50);
    let again = check_workflow(&saved, Some("build-review")).unwrap();
    assert_eq!(again, f);
    assert_eq!(model::flow_hash(&f).len(), 16);
}

#[test]
fn refuses_bad_workflows() {
    let cases: Vec<(Value, &str)> = vec![
        (json!([]), "A workflow is"),
        (with(flow_json(), &["version"], json!(2)), "version must be 1"),
        (with(flow_json(), &["name"], json!("Bad Name")), "small letters"),
        (with(flow_json(), &["when"], json!(" ")), "needs when"),
        (with(flow_json(), &["nodes"], json!([])), "needs nodes"),
        (with(flow_json(), &["nodes", "a b"], json!({"title": "x"})), "Node id a b"),
        (with(flow_json(), &["nodes", "n2", "title"], json!("")), "Node n2: A node needs a title."),
        (with(flow_json(), &["nodes", "n2", "gate"], json!("nobody")), "gate must be one of: none, checks, deep-brain, user."),
        (with(flow_json(), &["nodes", "n2", "retry"], json!(9)), "retry must be a whole number from 0 to 5."),
        (with(flow_json(), &["nodes", "n2", "limit"], json!(1.5)), "limit must be a whole number"),
        (with(flow_json(), &["nodes", "n2", "skills"], json!("tdd")), "skills is a list of names."),
        (with(flow_json(), &["nodes", "n2", "fallback"], json!({"who": {"kind": "map-row"}})), "fallback.who.kind must be one of: agent, custom-agent."),
        (with(flow_json(), &["nodes", "n2", "who"], json!({"kind": "robot"})), "who.kind must be one of"),
        (with(flow_json(), &["links"], json!(null)), "links is a list"),
        (with(flow_json(), &["links", "0", "kind"], json!("jump")), "Link 1 kind must be one of"),
    ];
    for (raw, want) in cases {
        let got = problems(&raw, None);
        assert!(got.len() == 1 && got[0].contains(want), "want {want}, got {got:?}");
    }
    let other = problems(&flow_json(), Some("other-name"));
    assert!(other[0].contains("must match"), "{other:?}");
}

#[test]
fn lint_finds_every_problem() {
    let raw = json!({
        "name": "bad",
        "when": "never",
        "nodes": {
            "s": {"type": "stage", "title": "Lonely", "prompt": ""},
            "t": {"type": "stage", "title": "Loop", "who": {"kind": "agent", "id": "codex"}, "prompt": "x"},
            "e": {"type": "end", "title": "End"}
        },
        "links": [
            {"id": "a", "from": "t", "to": "t", "kind": "pass"},
            {"id": "a", "from": "t", "to": "x", "kind": "pass"},
            {"id": "b", "from": "e", "to": "t", "kind": "pass"},
            {"id": "c", "from": "s", "to": "t", "kind": "pass"},
            {"id": "d", "from": "s", "to": "e", "kind": "pass"}
        ]
    });
    let found = problems(&raw, None).join("\n");
    for want in [
        "There is no start box.",
        "\"Loop\" joins itself.",
        "Two links share the id a.",
        "Link a joins a box that does not exist.",
        "A link leaves the end box \"End\".",
        "\"Lonely\" has two \"pass\" links. Keep one.",
        "\"Lonely\" does not say who does it.",
        "\"Lonely\" has no job text.",
        "\"Loop\" has no \"pass\" link to a next box.",
        "\"Loop\" cannot reach an end box.",
    ] {
        assert!(found.contains(want), "missing {want} in\n{found}");
    }
    let mut f = flow();
    f.nodes.get_mut("n2").unwrap().limit = 1;
    assert!(lint(&f).iter().any(|p| p == "\"Build\" is inside a loop, so its limit must be 2 or more."));
    let mut lost = flow();
    lost.links.retain(|l| l.to != "n4");
    let text = lint(&lost).join("\n");
    assert!(text.contains("\"Sum up\" cannot be reached from the start box."), "{text}");
    assert!(text.contains("\"Review\" has no \"pass\" link"), "{text}");
}

fn start(f: &model::Workflow) -> RunState {
    new_run(f, "h", "", 5)
}

#[test]
fn moves_through_the_boxes() {
    let f = flow();
    let first = first_stage(&f, &start(&f));
    assert_eq!(first.moved, Move::Stage { node: "n2".into(), skipped: vec![], link: Some(model::LinkKind::Pass) });
    assert_eq!((first.run.steps, first.run.visits["n2"]), (1, 1));
    let review = next_stage(&f, &first.run, Outcome::Pass, true);
    assert!(matches!(&review.moved, Move::Stage { node, .. } if node == "n3"));
    let back = next_stage(&f, &review.run, Outcome::Revise, true);
    assert!(matches!(&back.moved, Move::Stage { node, .. } if node == "n2"));
    assert_eq!(back.run.visits["n2"], 2);
    let retry = next_stage(&f, &back.run, Outcome::Fail, true);
    assert_eq!(retry.moved, Move::Retry { node: "n2".into() });
    assert_eq!(retry.run.tries, 1);
    let stuck = next_stage(&f, &retry.run, Outcome::Fail, true);
    assert!(matches!(&stuck.moved, Move::Ask { cause: AskCause::NoLink, why, .. } if why == "the box Build has no fail link"));
    let mut full = back.run.clone();
    full.node = "n3".into();
    full.visits.insert("n2".into(), 3);
    let limit = next_stage(&f, &full, Outcome::Revise, true);
    assert!(matches!(&limit.moved, Move::Ask { cause: AskCause::Limit, box_id, .. } if box_id == "n2"));
    let mut waiting = limit.run.clone();
    if let Move::Ask { cause, box_id, why } = &limit.moved {
        waiting.ask = Some(RunAsk { box_id: box_id.clone(), cause: *cause, why: why.clone(), gate: false, review: false, deep: String::new() });
    }
    let on = go_on(&f, &waiting);
    assert!(matches!(&on.moved, Move::Stage { node, .. } if node == "n2"));
    assert_eq!(on.run.visits["n2"], 4);
    let next = go_next(&f, &waiting);
    assert!(matches!(&next.moved, Move::Stage { node, .. } if node == "n4"));
    let end = next_stage(&f, &RunState { node: "n4".into(), ..waiting.clone() }, Outcome::Pass, true);
    assert!(matches!(&end.moved, Move::End { node, .. } if node == "n5"));
}

#[test]
fn skips_off_boxes_and_counts_steps() {
    let mut f = flow();
    f.nodes.get_mut("n3").unwrap().off = true;
    let first = first_stage(&f, &start(&f));
    let next = next_stage(&f, &first.run, Outcome::Pass, true);
    assert_eq!(next.moved, Move::Stage { node: "n4".into(), skipped: vec!["n3".into()], link: Some(model::LinkKind::Pass) });
    let mut short = flow();
    short.limits.max_steps = 1;
    let first = first_stage(&short, &start(&short));
    let over = next_stage(&short, &first.run, Outcome::Pass, true);
    assert!(matches!(&over.moved, Move::Ask { cause: AskCause::MaxSteps, .. }));
}

#[test]
fn reads_stage_outcomes() {
    let f = flow();
    let facts = StageFacts::default();
    let got = |id: &str, text: &str| outcome_of(&f, id, text, &facts);
    assert!(matches!(got("n2", "done\n**RESULT: PASS**"), OutcomeRead::Got { outcome: Outcome::Pass, .. }));
    assert!(matches!(got("n2", "RESULT: PASS\n- RESULT: fail"), OutcomeRead::Got { outcome: Outcome::Fail, .. }));
    assert!(matches!(got("n2", "all good"), OutcomeRead::NoResult { .. }));
    assert!(matches!(got("n3", "FINDINGS: 0"), OutcomeRead::Got { outcome: Outcome::Pass, .. }));
    assert!(matches!(got("n3", "looks fine to me"), OutcomeRead::Got { outcome: Outcome::Revise, .. }));
    assert!(matches!(got("n3", "FINDINGS: 0 then FINDINGS: 2"), OutcomeRead::Got { outcome: Outcome::Revise, .. }));
    assert!(matches!(got("n3", "NO FINDINGS"), OutcomeRead::Got { outcome: Outcome::Pass, .. }));
    let failed = StageFacts { checks_failed: true, checks_text: "cargo test failed".into(), changed_code: false };
    assert!(matches!(outcome_of(&f, "n3", "FINDINGS: 0", &failed), OutcomeRead::Got { outcome: Outcome::Fail, why } if why.contains("cargo test failed")));
    let mut with_review = flow();
    with_review.links.push(model::Link { id: "r".into(), from: "n4".into(), to: "n3".into(), kind: model::LinkKind::Review });
    let changed = StageFacts { changed_code: true, ..StageFacts::default() };
    assert!(matches!(outcome_of(&with_review, "n4", "RESULT: PASS", &changed), OutcomeRead::Got { outcome: Outcome::Review, .. }));
    assert!(needs_gate(&f, "n4", Outcome::Pass));
    assert!(!needs_gate(&f, "n4", Outcome::Fail));
    assert!(!needs_gate(&f, "n2", Outcome::Pass));
}

#[test]
fn writes_stage_lines() {
    let f = flow();
    let mut run = first_stage(&f, &start(&f)).run;
    run.fails = "tests failed".into();
    let lines = stage_lines(&f, &run, 7, Some(("old report", "/tmp/r.md"))).join("\n");
    assert!(lines.contains("You do stage 1: Build. Round 1 of 3."), "{lines}");
    assert!(lines.contains("Skills for this stage: tdd."));
    assert!(lines.contains("The last try failed"));
    assert!(lines.contains("full text in /tmp/r.md"));
    assert!(lines.contains("RESULT: PASS or RESULT: FAIL"));
    assert!(lines.contains("task status 7 review"));
    run.node = "n3".into();
    assert!(stage_lines(&f, &run, 7, None).join("\n").contains("FINDINGS: <number>"));
    assert_eq!(progress_line(&f, &run), "Workflow Build then review: stage 1, Review, round 0 of 3, 1 of 50 steps (running).");
    let text = write_run(&run);
    assert_eq!(read_run(&text), Some(run));
    assert!(!keeps_checks_line(&f, "n2", &|_| true));
    assert!(keeps_checks_line(&f, "n4", &|_| true));
}

#[test]
fn reads_answers() {
    assert_eq!(read_answer("pass", true, false), Ok(Answer::Gate(GateWord::Pass)));
    assert_eq!(read_answer("2", true, false), Ok(Answer::Gate(GateWord::Revise)));
    assert_eq!(read_answer("1. pass", true, false), Ok(Answer::Gate(GateWord::Pass)));
    assert_eq!(read_answer("I would not pass it, revise please", true, false), Ok(Answer::Gate(GateWord::Revise)));
    assert_eq!(read_answer("Cancel the run.", true, false), Ok(Answer::Cancel));
    assert_eq!(read_answer("4", true, false), Ok(Answer::Cancel));
    assert!(read_answer("maybe", true, false).is_err());
    assert_eq!(read_answer("Go to the next box", false, false), Ok(Answer::Ask(AskWord::Next)));
    assert_eq!(read_answer("\"go on from this box\"", false, false), Ok(Answer::Ask(AskWord::GoOn)));
    assert_eq!(read_answer("please retry", false, false), Ok(Answer::Ask(AskWord::GoOn)));
    assert_eq!(read_answer("skip this box", false, true), Ok(Answer::Ask(AskWord::Next)));
    assert_eq!(read_answer("use the second file", false, true), Ok(Answer::Ask(AskWord::GoOn)));
    assert!(read_answer("hmm", false, false).is_err());
}

fn options() -> WhoOptions {
    WhoOptions {
        map_rows: vec![
            AgentMapRow { id: "r1".into(), job: "build".into(), member: "builder".into(), agent: "codex".into(), model: "gpt".into(), ..AgentMapRow::default() },
            AgentMapRow { id: "r2".into(), job: "test".into(), agent: "omp".into(), model: "m2".into(), effort: "high".into(), ..AgentMapRow::default() },
            AgentMapRow { id: "r3".into(), job: "lint tool".into(), ..AgentMapRow::default() },
            AgentMapRow { id: "r4".into(), job: "old".into(), off: true, ..AgentMapRow::default() },
        ],
        programs: vec!["codex".into(), "omp".into(), "claude".into()],
        customs: vec![
            Custom { id: "a1".into(), title: "Reviewer".into(), space: "nixa".into(), off: false, claude: false },
            Custom { id: "a2".into(), title: "Old".into(), space: String::new(), off: true, claude: false },
        ],
        models: vec![("codex".into(), "gpt-5".into())],
        space: "nixa".into(),
        fallback: false,
    }
}

fn box_with(who: Value) -> model::Node {
    model::check_node(&json!({"title": "Box", "who": who, "prompt": "x", "fallback": {"who": {"kind": "agent", "id": "claude"}, "model": "opus"}})).unwrap()
}

fn assigned(pick: WhoPick) -> Pick {
    match pick {
        WhoPick::Assign(p) => p,
        other => panic!("not assigned: {other:?}"),
    }
}

#[test]
fn picks_who_does_a_box() {
    let o = options();
    let p = assigned(who_of(&box_with(json!({"kind": "agent", "id": "codex"})), &o, ""));
    assert_eq!((p.assigned_kind.as_str(), p.assigned.as_str(), p.fallback), ("agent", "codex", false));
    let p = assigned(who_of(&box_with(json!({"kind": "agent", "id": "kimi"})), &o, ""));
    assert_eq!((p.assigned.as_str(), p.model.as_str(), p.fallback), ("claude", "claude::opus", true));
    let p = assigned(who_of(&box_with(json!({"kind": "model", "id": "gpt-5"})), &o, ""));
    assert_eq!((p.assigned.as_str(), p.model.as_str()), ("codex", "codex::gpt-5"));
    let p = assigned(who_of(&box_with(json!({"kind": "custom-agent", "id": "a1"})), &o, ""));
    assert_eq!((p.assigned_kind.as_str(), p.assigned.as_str()), ("member", "a1"));
    let p = assigned(who_of(&box_with(json!({"kind": "map-row", "id": "r1"})), &o, ""));
    assert_eq!((p.assigned_kind.as_str(), p.assigned.as_str(), p.model.as_str()), ("member", "builder", "codex::gpt"));
    let p = assigned(who_of(&box_with(json!({"kind": "map-row", "id": "r2"})), &o, ""));
    assert_eq!((p.assigned.as_str(), p.model.as_str(), p.effort.as_str()), ("omp", "omp::m2", "high"));
    assert_eq!(who_of(&box_with(json!({"kind": "deep-brain"})), &o, ""), WhoPick::Deep { label: "Box".into() });
    assert_eq!(who_of(&box_with(json!({"kind": "user"})), &o, ""), WhoPick::User { label: "Box".into() });
    let lonely = model::check_node(&json!({"title": "Box", "who": {"kind": "custom-agent", "id": "a2"}, "prompt": "x"})).unwrap();
    assert!(matches!(who_of(&lonely, &o, ""), WhoPick::Missing { why } if why.contains("turned off")));
    let mut far = o.clone();
    far.space = "other".into();
    let reviewer = model::check_node(&json!({"title": "Box", "who": {"kind": "custom-agent", "id": "a1"}, "prompt": "x", "fallback": {"who": {"kind": "agent", "id": "kimi"}}})).unwrap();
    assert!(matches!(who_of(&reviewer, &far, ""), WhoPick::Missing { why } if why.contains("another space") && why.contains("fallback failed") && why.contains("kimi is not installed")));
    let tool = model::check_node(&json!({"title": "Box", "who": {"kind": "map-row", "id": "r3"}, "prompt": "x"})).unwrap();
    assert!(matches!(who_of(&tool, &o, ""), WhoPick::Missing { why } if why.contains("not an agent start")));
    let off = model::check_node(&json!({"title": "Box", "who": {"kind": "map-row", "id": "r4"}, "prompt": "x"})).unwrap();
    assert!(matches!(who_of(&off, &o, ""), WhoPick::Missing { why } if why.contains("row 4 is off")));
    let mut again = o.clone();
    again.fallback = true;
    let p = assigned(who_of(&box_with(json!({"kind": "agent", "id": "codex"})), &again, ""));
    assert_eq!(p.assigned, "claude");
}

struct Db {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    conn: Connection,
}

fn db() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let conn = nixa_store::open(&path.join("nixa.db")).unwrap();
    conn.execute_batch(nixa_store::workflows::SQL).unwrap();
    Db { _dir: dir, path, conn }
}

fn ctx(db: &Db) -> Ctx<'_> {
    Ctx { conn: &db.conn, dir: &db.path, now: 1000, options: options(), deep_first: false }
}

fn put(db: &Db, raw: Value) {
    let name = raw["name"].as_str().unwrap().to_string();
    store_request(&db.conn, "put", &args(json!({"name": name, "workflow": raw})), &Value::Null).unwrap();
}

fn new_task(db: &Db, workflow: &str) -> i64 {
    let input = TaskInput { title: "Make the thing".into(), space_id: "nixa".into(), workflow: workflow.into(), ..TaskInput::default() };
    tasks::add_task(&db.conn, &input, &Actor::new(AuthorKind::User, "Sam")).unwrap().id
}

fn work(db: &Db, id: i64, terminal: &str, report: &str) {
    let by = Actor::in_terminal(AuthorKind::Agent, "agent", terminal);
    let claim = tasks::claim_task(&db.conn, id, &by, "").unwrap();
    assert!(claim.ok, "{}", claim.reason);
    tasks::add_comment(&db.conn, id, &CommentInput::new(CommentKind::Result, report), &by).unwrap();
    tasks::set_status(&db.conn, id, TaskStatus::Review, None, false).unwrap();
}

fn run_of(db: &Db, id: i64) -> RunState {
    driver::loaded(&db.conn, id).unwrap().unwrap().0
}

fn task(db: &Db, id: i64) -> tasks::Task {
    tasks::must_task(&db.conn, id).unwrap()
}

#[test]
fn runs_a_task_from_start_to_end() {
    let db = db();
    put(&db, flow_json());
    let id = new_task(&db, "build-review");
    let c = ctx(&db);
    let begun = driver::begin(&c, id).unwrap().unwrap();
    assert!(begun.go);
    assert!(!begun.effects.contains(&Effect::Start { task: id }));
    assert!(driver::begin(&c, id).unwrap().is_none());
    let t = task(&db, id);
    assert_eq!((t.assigned_kind.as_str(), t.assigned.as_str(), t.status), ("agent", "codex", TaskStatus::Todo));
    assert!(tasks::task_meta(&db.conn, &driver::pick_key(id)).unwrap().unwrap().contains("codex"));
    let info = driver::stage_info(&db.conn, id).unwrap();
    assert!(info.lines[0].contains("You do stage 1: Build"));
    assert!(!info.checks);

    work(&db, id, "m5", "Built it.\nRESULT: PASS");
    let done = driver::settle(&c, id, &Settle::default()).unwrap();
    assert!(done.effects.contains(&Effect::Close { terminal: "m5".into() }), "{:?}", done.effects);
    assert!(done.effects.contains(&Effect::Start { task: id }));
    let run = run_of(&db, id);
    assert_eq!((run.node.as_str(), run.history.len()), ("n3", 1));
    assert!(std::path::Path::new(&run.history[0].file).exists());
    assert_eq!(task(&db, id).assigned, "omp");
    assert!(driver::stage_info(&db.conn, id).unwrap().lines.join("\n").contains("Built it."));

    work(&db, id, "m6", "Two problems.\nFINDINGS: 2");
    driver::settle(&c, id, &Settle::default()).unwrap();
    let run = run_of(&db, id);
    assert_eq!((run.node.as_str(), run.visits["n2"]), ("n2", 2));

    work(&db, id, "m7", "RESULT: FAIL");
    let retry = driver::settle(&c, id, &Settle::default()).unwrap();
    assert!(retry.effects.iter().any(|e| matches!(e, Effect::Retry { terminal, .. } if terminal == "m7")));
    assert_eq!((run_of(&db, id).tries, task(&db, id).status), (1, TaskStatus::Doing));
    tasks::set_status(&db.conn, id, TaskStatus::Review, None, false).unwrap();
    tasks::add_comment(&db.conn, id, &CommentInput::new(CommentKind::Result, "Fixed.\nRESULT: PASS"), &Actor::in_terminal(AuthorKind::Agent, "agent", "m7")).unwrap();
    driver::settle(&c, id, &Settle::default()).unwrap();
    assert_eq!(run_of(&db, id).node, "n3");

    work(&db, id, "m8", "FINDINGS: 0");
    driver::settle(&c, id, &Settle::default()).unwrap();
    assert_eq!(run_of(&db, id).node, "n4");

    work(&db, id, "m9", "Summary.\nRESULT: PASS");
    let gate = driver::settle(&c, id, &Settle::default()).unwrap();
    assert!(gate.effects.iter().any(|e| matches!(e, Effect::Quiet { terminal, .. } if terminal == "m9")));
    assert!(!gate.effects.iter().any(|e| matches!(e, Effect::AskDeep { .. })));
    let run = run_of(&db, id);
    assert_eq!(run.state, RunWord::Waiting);
    assert!(run.ask.as_ref().unwrap().gate);
    let t = task(&db, id);
    assert_eq!((t.status, t.blocked_on, t.terminal_id.as_str()), (TaskStatus::Blocked, Some(BlockedOn::Other), ""));
    assert!(gate.effects.iter().any(|e| matches!(e, Effect::Pick { .. })));
    assert!(driver::held(&db.conn, id).is_some());
    assert!(driver::act(&c, id, "answer", "Sam", "perhaps").is_err());
    assert!(driver::act(&c, id, "pause", "Sam", "").is_err());

    let end = driver::act(&c, id, "answer", "Sam", "pass").unwrap();
    assert!(end.text.contains("reached its end"), "{}", end.text);
    assert_eq!(run_of(&db, id).state, RunWord::Done);
    assert_eq!(task(&db, id).status, TaskStatus::Done);
    assert!(driver::act(&c, id, "cancel", "Sam", "").is_err());
    let shown = driver::run_show(&db.conn, id).unwrap();
    assert_eq!(shown["run"]["state"], "done");
    assert_eq!(shown["reports"].as_array().unwrap().len(), 6);
    assert!(driver::run_list(&db.conn).is_empty());
}

#[test]
fn pause_resume_cancel_and_agent_blocks() {
    let db = db();
    put(&db, flow_json());
    let id = new_task(&db, "build-review");
    let c = ctx(&db);
    driver::begin(&c, id).unwrap().unwrap();
    assert!(driver::act(&c, id, "resume", "Sam", "").is_err());
    driver::act(&c, id, "pause", "Sam", "").unwrap();
    assert!(driver::held(&db.conn, id).unwrap().contains("paused"));
    work(&db, id, "m5", "RESULT: PASS");
    let paused = driver::settle(&c, id, &Settle::default()).unwrap();
    assert!(!paused.effects.contains(&Effect::Start { task: id }));
    assert!(paused.text.contains("is set and paused"), "{}", paused.text);
    let resumed = driver::act(&c, id, "resume", "Sam", "").unwrap();
    assert!(resumed.effects.contains(&Effect::Start { task: id }));
    assert_eq!(driver::run_list(&db.conn).len(), 1);

    let by = Actor::in_terminal(AuthorKind::Agent, "agent", "m6");
    tasks::claim_task(&db.conn, id, &by, "").unwrap();
    tasks::add_comment(&db.conn, id, &CommentInput::new(CommentKind::Question, "Which file is the real one?"), &by).unwrap();
    tasks::set_status(&db.conn, id, TaskStatus::Blocked, Some(BlockedOn::User), true).unwrap();
    assert!(driver::on_change(&c, id, "status:blocked", "user").unwrap().is_none());
    let blocked = driver::on_change(&c, id, "status:blocked", "agent").unwrap().unwrap();
    assert!(blocked.text.contains("Which file is the real one?"), "{}", blocked.text);
    let run = run_of(&db, id);
    assert_eq!(run.ask.as_ref().unwrap().cause, AskCause::AgentBlock);
    let on = driver::act(&c, id, "answer", "Sam", "use the second file").unwrap();
    assert!(on.text.contains("goes on with the box Review"), "{}", on.text);
    let run = run_of(&db, id);
    assert!(run.answered.contains("use the second file"));
    assert_eq!(run.state, RunWord::Running);

    let by = Actor::in_terminal(AuthorKind::Agent, "agent", "m7");
    tasks::claim_task(&db.conn, id, &by, "").unwrap();
    let cancel = driver::act(&c, id, "cancel", "Sam", "").unwrap();
    assert!(cancel.text.contains("Workflow run cancelled: Sam cancelled it."));
    assert!(cancel.effects.iter().any(|e| matches!(e, Effect::Quiet { terminal, .. } if terminal == "m7")));
    assert_eq!(task(&db, id).status, TaskStatus::Cancelled);
    assert_eq!(run_of(&db, id).state, RunWord::Cancelled);
}

#[test]
fn closing_the_task_ends_the_run() {
    let db = db();
    put(&db, flow_json());
    let id = new_task(&db, "build-review");
    let c = ctx(&db);
    driver::begin(&c, id).unwrap();
    assert!(driver::on_change(&c, id, "status:done", "user").unwrap().is_none());
    tasks::set_status(&db.conn, id, TaskStatus::Done, None, false).unwrap();
    let done = driver::on_change(&c, id, "status:done", "user").unwrap().unwrap();
    assert!(done.text.contains("the user set the task to done"), "{}", done.text);
    assert_eq!(run_of(&db, id).state, RunWord::Done);
    assert_eq!(task(&db, id).status, TaskStatus::Done);
}

#[test]
fn deep_brain_gates() {
    let db = db();
    let raw = with(flow_json(), &["nodes", "n4", "gate"], json!("deep-brain"));
    put(&db, raw);
    let id = new_task(&db, "build-review");
    let c = ctx(&db);
    driver::begin(&c, id).unwrap();
    for (terminal, report) in [("m1", "RESULT: PASS"), ("m2", "FINDINGS: 0"), ("m3", "RESULT: PASS")] {
        work(&db, id, terminal, report);
        driver::settle(&c, id, &Settle::default()).unwrap();
    }
    let run = run_of(&db, id);
    assert_eq!(run.state, RunWord::Waiting);
    assert_eq!(task(&db, id).blocked_on, Some(BlockedOn::Other));
    driver::set_deep(&db.conn, id, "q7").unwrap();
    assert_eq!(driver::waiting_task(&db.conn, "q7").unwrap(), Some(id));
    assert!(driver::deep_answered(&c, id, "q1", true, "pass").unwrap().is_none());
    let none = driver::deep_answered(&c, id, "q7", true, "").unwrap().unwrap();
    assert!(!none.text.contains("Waiting for the user"), "{}", none.text);
    assert_ne!(task(&db, id).blocked_on, Some(BlockedOn::User));
}

#[test]
fn deep_first_asks_the_deep_brain_also_for_hard_stops() {
    let db = db();
    put(&db, flow_json());
    let id = new_task(&db, "build-review");
    let mut c = ctx(&db);
    c.deep_first = true;
    driver::begin(&c, id).unwrap();
    work(&db, id, "m1", "I did it but wrote no result line.");
    let asked = driver::settle(&c, id, &Settle::default()).unwrap();
    assert!(asked.effects.iter().any(|e| matches!(e, Effect::AskDeep { options, .. } if options.len() == 3)));
    let other = new_task(&db, "build-review");
    driver::begin(&c, other).unwrap();
    work(&db, other, "m2", "Needs a git push to production first.");
    let hard = driver::settle(&c, other, &Settle::default()).unwrap();
    assert!(hard.effects.iter().any(|e| matches!(e, Effect::AskDeep { .. })));
    assert_eq!(task(&db, other).blocked_on, Some(BlockedOn::Other));
    let next = driver::act(&c, other, "answer", "Sam", "Go to the next box").unwrap();
    assert!(next.text.contains("goes on with the box Review"), "{}", next.text);
    let retry_waits = driver::act(&c, id, "retry", "Sam", "");
    assert!(retry_waits.is_ok(), "{retry_waits:?}");
    assert!(driver::act(&c, id, "retry", "Sam", "").is_err());
}

#[tokio::test]
async fn background_workflow_recovery_runs_in_every_space() {
    use nixa_store::spaces::{SpacePatch, set_space};

    let db = db();
    let app = crate::app::App::open(&db.path).unwrap();
    set_space(&db.conn, "nixa", &SpacePatch { name: Some("nixa".into()), ..Default::default() }).unwrap();
    set_space(&db.conn, "other", &SpacePatch { name: Some("other".into()), ..Default::default() }).unwrap();
    let mut c = ctx(&db);
    c.deep_first = true;
    put(&db, flow_json());
    let mut ids = Vec::new();
    for (space, ask) in [("nixa", "d8"), ("other", "d9")] {
        let id = new_task(&db, "build-review");
        tasks::update_task(&db.conn, id, &tasks::TaskPatch { space_id: Some(space.into()), ..Default::default() }).unwrap();
        driver::begin(&c, id).unwrap();
        for (terminal, report) in [("m1", "RESULT: PASS"), ("m2", "FINDINGS: 0"), ("m3", "RESULT: PASS")] {
            work(&db, id, terminal, report);
            driver::settle(&c, id, &Settle::default()).unwrap();
        }
        driver::set_deep(&db.conn, id, ask).unwrap();
        db.conn.execute("UPDATE workflow_runs SET updated_at = 1 WHERE task_id = ?1", [id]).unwrap();
        ids.push(id);
    }
    super::revisit_waiting(&app).await;
    assert_eq!(task(&db, ids[0]).status, TaskStatus::Done);
    assert_eq!(task(&db, ids[1]).status, TaskStatus::Done);
}

#[test]
fn automatic_hard_stop_goes_to_the_deep_brain_not_the_user() {
    let db = db();
    crate::modes::journal::ensure(&db.conn, 1000).unwrap();
    put(&db, flow_json());
    let id = new_task(&db, "build-review");
    let c = ctx(&db);
    driver::begin(&c, id).unwrap();
    work(&db, id, "m1", "Needs a git push to production first.");
    let hard = driver::settle(&c, id, &Settle::default()).unwrap();
    assert!(hard.effects.iter().any(|e| matches!(e, Effect::AskDeep { .. })), "{:?}", hard.effects);
    assert_ne!(task(&db, id).blocked_on, Some(BlockedOn::User));
    assert_eq!(task(&db, id).blocked_on, Some(BlockedOn::Other));
    let rows = crate::modes::journal::last_for(&db.conn, None, 10).unwrap();
    assert!(rows.iter().any(|row| row.kind == "deep" && row.choice.contains(&format!("task #{id}"))), "{rows:?}");
}

#[test]
fn user_gates_and_boxes_go_to_the_deep_brain_in_automatic() {
    let db = db();
    put(&db, flow_json());
    let mine = with(with(flow_json(), &["nodes", "n2", "who"], json!({"kind": "user"})), &["name"], json!("own-box"));
    let mine = with(mine, &["nodes", "n2", "prompt"], json!("Choose the button colour"));
    put(&db, mine);
    let mut c = ctx(&db);
    c.deep_first = true;
    let id = new_task(&db, "build-review");
    driver::begin(&c, id).unwrap();
    for (terminal, report) in [("m1", "RESULT: PASS"), ("m2", "FINDINGS: 0"), ("m3", "Choose the button colour.
RESULT: PASS")] {
        work(&db, id, terminal, report);
        driver::settle(&c, id, &Settle::default()).unwrap();
    }
    assert_eq!(run_of(&db, id).state, RunWord::Waiting);
    assert_eq!(task(&db, id).blocked_on, Some(BlockedOn::Other));
    assert!(!task(&db, id).needs_user);
    let other = new_task(&db, "own-box");
    let begun = driver::begin(&c, other).unwrap().unwrap();
    assert!(begun.effects.iter().any(|e| matches!(e, Effect::AskDeep { .. })), "{:?}", begun.effects);
    assert_eq!(task(&db, other).blocked_on, Some(BlockedOn::Other));
    assert!(run_of(&db, other).ask.unwrap().gate);
}

#[test]
fn nixa_decides_user_gates_itself_in_automatic() {
    let db = db();
    crate::modes::journal::ensure(&db.conn, 1000).unwrap();
    put(&db, flow_json());
    let c = ctx(&db);
    let id = new_task(&db, "build-review");
    driver::begin(&c, id).unwrap();
    let mut picks = Vec::new();
    for (terminal, report) in [("m1", "RESULT: PASS"), ("m2", "FINDINGS: 0"), ("m3", "RESULT: PASS")] {
        work(&db, id, terminal, report);
        let done = driver::settle(&c, id, &Settle::default()).unwrap();
        assert!(!done.effects.iter().any(|e| matches!(e, Effect::AskDeep { .. })), "{:?}", done.effects);
        picks.extend(done.effects.into_iter().filter(|e| matches!(e, Effect::Pick { .. })));
    }
    assert_eq!(picks, vec![Effect::Pick { task: id, pick: "pass".into() }]);
    assert_eq!(task(&db, id).blocked_on, Some(BlockedOn::Other));
    assert!(!task(&db, id).needs_user);
    let rows = crate::modes::journal::last_for(&db.conn, None, 10).unwrap();
    assert!(rows.iter().any(|row| row.kind == "workflow" && row.choice.contains(&format!("task #{id}"))), "{rows:?}");
    let done = driver::act(&c, id, "answer", driver::NIXA_BY, "pass").unwrap();
    assert!(done.text.contains("reached its end"), "{}", done.text);
    set_deep_and_fail(&db, &c);
}

fn set_deep_and_fail(db: &Db, c: &Ctx) {
    let raw = with(flow_json(), &["nodes", "n4", "gate"], json!("deep-brain"));
    let raw = with(raw, &["name"], json!("deep-gate"));
    put(db, raw);
    let id = new_task(db, "deep-gate");
    driver::begin(c, id).unwrap();
    for (terminal, report) in [("m4", "RESULT: PASS"), ("m5", "FINDINGS: 0"), ("m6", "RESULT: PASS")] {
        work(db, id, terminal, report);
        driver::settle(c, id, &Settle::default()).unwrap();
    }
    assert_eq!(run_of(db, id).state, RunWord::Waiting);
    driver::set_deep(&db.conn, id, "q9").unwrap();
    let none = driver::deep_answered(c, id, "q9", false, "").unwrap().unwrap();
    assert!(none.text.contains("reached its end"), "{}", none.text);
    assert_eq!(task(db, id).status, TaskStatus::Done);
    assert!(driver::deep_failed(c, id, "no deep brain").unwrap().is_none());
    assert!(!task(db, id).needs_user);
}

#[test]
fn begin_refuses_and_falls_back() {
    let db = db();
    let c = ctx(&db);
    let missing = new_task(&db, "gone");
    assert!(driver::begin(&c, missing).unwrap().is_none());
    assert!(tasks::task_comments(&db.conn, missing).unwrap().iter().any(|m| m.text.contains("no longer exists")));
    let plain = new_task(&db, "");
    assert!(driver::begin(&c, plain).unwrap().is_none());
    let nixa = with(flow_json(), &["nodes", "n2", "who"], json!({"kind": "nixa"}));
    let nixa = with(nixa, &["name"], json!("nixa-box"));
    put(&db, nixa);
    let given = new_task(&db, "nixa-box");
    assert!(driver::begin(&c, given).is_ok());
    assert_eq!(task(&db, given).assigned, "codex");
    nixa_store::workflows::put_workflow(&db.conn, "broken", "{\"name\": \"broken\"}").unwrap();
    let broken = new_task(&db, "broken");
    assert!(driver::begin(&c, broken).unwrap_err().contains("does not pass the checker"));
    let nobody = with(with(flow_json(), &["nodes", "n2", "who"], json!({"kind": "agent", "id": "kimi"})), &["name"], json!("no-agent"));
    put(&db, nobody);
    let stuck = new_task(&db, "no-agent");
    let done = driver::begin(&c, stuck).unwrap().unwrap();
    assert!(!done.go);
    assert!(run_of(&db, stuck).ask.unwrap().why.contains("kimi is not installed"));
}

#[test]
fn custom_agent_default_workflow() {
    let db = db();
    put(&db, flow_json());
    let input = nixa_store::agents::AddAgent { title: "Builder".into(), purpose: "builds".into(), provider: "codex".into(), ..Default::default() };
    let agent = nixa_store::agents::add_agent(&db.conn, &input, "Sam").unwrap();
    let change = nixa_store::agents::EditAgent { workflow: Some("build-review".into()), ..Default::default() };
    nixa_store::agents::edit_agent(&db.conn, &agent.id, &change).unwrap();
    let id = new_task(&db, "");
    tasks::assign_task(&db.conn, id, "member", &agent.id).unwrap();
    let c = ctx(&db);
    driver::begin(&c, id).unwrap().unwrap();
    assert_eq!(task(&db, id).workflow, "build-review");
    assert_eq!(run_of(&db, id).node, "n2");
}

#[test]
fn store_requests_for_the_window() {
    let db = db();
    let bad = store_request(&db.conn, "put", &args(json!({"name": "x", "workflow": {"name": "x", "when": "w", "nodes": {}}})), &Value::Null);
    assert!(bad.unwrap_err().starts_with("Not saved. There is no start box."));
    assert!(store_request(&db.conn, "read", &args(json!({})), &Value::Null).unwrap_err().contains("needs a workflow name"));
    assert!(store_request(&db.conn, "jump", &args(json!({})), &Value::Null).unwrap_err().contains("workflows takes"));
    put(&db, flow_json());
    let (saved, name) = store_request(&db.conn, "put", &args(json!({"name": "build-review", "workflow": with(flow_json(), &["title"], json!("Two"))})), &Value::Null).unwrap();
    assert_eq!((saved["can_undo"].clone(), name), (json!(true), Some("build-review".to_string())));
    let (list, _) = store_request(&db.conn, "list", &Args::new(), &Value::Null).unwrap();
    assert_eq!(list["workflows"][0]["title"], "Two");
    let (undone, _) = store_request(&db.conn, "undo", &args(json!({"name": "build-review"})), &Value::Null).unwrap();
    assert_eq!(undone["workflow"]["title"], "Build then review");
    let (checked, _) = store_request(&db.conn, "check", &args(json!({"workflow": flow_json()})), &Value::Null).unwrap();
    assert_eq!(checked["problems"], json!([]));
    store_request(&db.conn, "delete", &args(json!({"name": "build-review"})), &Value::Null).unwrap();
    assert!(store_request(&db.conn, "read", &args(json!({"name": "build-review"})), &Value::Null).unwrap_err().contains("There is no workflow named"));
    assert!(store_request(&db.conn, "delete", &args(json!({"name": "build-review"})), &Value::Null).is_err());
    nixa_store::workflows::put_workflow(&db.conn, "broken", "not json").unwrap();
    let (list, _) = store_request(&db.conn, "list", &Args::new(), &Value::Null).unwrap();
    assert!(list["workflows"].as_array().unwrap().iter().any(|w| w["broken"].is_string()));
    let (runs, _) = store_request(&db.conn, "runs", &Args::new(), &Value::Null).unwrap();
    assert_eq!(runs["runs"], json!([]));
    let skills = tempfile::tempdir().unwrap();
    std::fs::create_dir(skills.path().join("tdd")).unwrap();
    let opts = super::routes::options_json(&db.conn, skills.path(), &[("nixa_task".into(), "tasks".into())], &[]);
    assert_eq!(opts["skills"][0]["name"], "tdd");
    assert_eq!(opts["gates"], json!(["none", "checks", "deep-brain", "user"]));
    assert!(opts["who_kinds"].as_array().unwrap().iter().all(|w| w != ""));
}

#[test]
fn tool_text() {
    let list = json!({"workflows": [{"name": "a", "title": "A", "nodes": 3, "when": "w"}]});
    assert_eq!(super::tool::text_of("list", &list), "a: A, 3 boxes. When: w");
    assert_eq!(super::tool::text_of("check", &json!({"problems": []})), "The workflow passes the checker.");
    assert!(super::tool::text_of("check", &json!({"problems": ["x"]})).contains("1 problem:\n- x"));
    assert_eq!(super::tool::text_of("runs", &json!({"runs": []})), "No workflow run is active.");
}

#[test]
fn custom_agent_stage_model_reaches_the_custom_start() {
    let db = db();
    let raw = with(flow_json(), &["nodes", "n2", "who"], json!({"kind": "custom-agent", "id": "a1"}));
    let raw = with(raw, &["nodes", "n2", "model"], json!("claude::opus"));
    let raw = with(raw, &["nodes", "n2", "effort"], json!("high"));
    put(&db, raw);
    let id = new_task(&db, "build-review");
    assert!(driver::begin(&ctx(&db), id).unwrap().unwrap().go);
    let key = format!("{}{id}", crate::extras::model_pick::PICK_KEY);
    let stored = tasks::task_meta(&db.conn, &key).unwrap().unwrap();
    let pick = crate::extras::model_pick::read_pick(&stored).expect("the saved stage pick reads back");
    assert_eq!((pick.assigned_kind.as_str(), pick.assigned.as_str(), pick.model.as_str(), pick.effort.as_str()), ("member", "a1", "claude::opus", "high"));
    let want = crate::queue::pick::want(&pick).expect("the saved stage pick gives a launch choice");
    let agent = nixa_store::agents::CustomAgent {
        id: "a1".into(),
        title: "Reviewer".into(),
        purpose: String::new(),
        instructions: String::new(),
        role: String::new(),
        job: String::new(),
        workflow: String::new(),
        start: Some(nixa_store::agents::StartProfile { agent: "claude".into(), model: "sonnet".into(), ..nixa_store::agents::StartProfile::default() }),
        off: false,
        migrated_from: String::new(),
        provider: "claude".into(),
        session: String::new(),
        space: String::new(),
        space_name: String::new(),
        folder: String::new(),
        terminal: String::new(),
        created_at: 0,
        last_used: 0,
        by: String::new(),
        skills: Vec::new(),
    };
    let start = crate::custom_agents::with_model(agent, Some(&want)).start.unwrap();
    assert_eq!((start.model.as_str(), start.effort.as_str()), ("opus", "high"));
}

#[test]
fn a_lost_frozen_copy_starts_the_run_again_in_automatic() {
    let db = db();
    put(&db, flow_json());
    let id = new_task(&db, "build-review");
    let c = ctx(&db);
    driver::begin(&c, id).unwrap();
    db.conn.execute("UPDATE workflow_runs SET flow = 'lost' WHERE task_id = ?1", [id]).unwrap();
    let done = driver::settle(&c, id, &Settle::default()).unwrap();
    assert!(done.effects.contains(&Effect::Start { task: id }), "{:?}", done.effects);
    let t = task(&db, id);
    assert_eq!((t.status, t.needs_user), (TaskStatus::Todo, false));
    assert_eq!(run_of(&db, id).state, RunWord::Failed);
}

#[test]
fn the_end_commit_line_lets_nixa_decide() {
    assert_eq!(driver::commit_line(0), "");
    let auto = driver::commit_line(2);
    assert!(auto.contains("Nixa decides the commit") && !auto.contains("Nixa does not commit"), "{auto}");
}
