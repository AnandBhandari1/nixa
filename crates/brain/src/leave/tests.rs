use super::book::{self, Book, Leave};
use super::lines::{self, Moved};
use super::*;
use crate::schedules::zone;
use nixa_store::tasks::{Actor, AuthorKind, TaskInput};

fn leave(terminal: &str, engine: &str, until: u64, line: &str) -> Leave {
    Leave { terminal: terminal.into(), engine: engine.into(), until, known: true, line: line.into(), ..Leave::default() }
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn leave_is_saved_kept_after_restart_and_expires() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let report = Report { terminal: "nixa-codex-2".into(), engine: "codex".into(), line: "■ You've hit your usage limit.".into(), until: Some(5_000), by: BY_SCREEN.into(), ..Report::default() };
    let made = add(&app, &report, 1_000).unwrap();
    assert!(made.known && made.until == 5_000 && made.engine == "codex");
    assert!(add(&app, &report, 1_100).is_none(), "the same terminal is not put on leave twice");
    let again = App::open(dir.path()).unwrap();
    let saved = lock(&again.leave.book).clone();
    assert_eq!(saved.items.len(), 1);
    assert_eq!(saved.handled.get("nixa-codex-2").map(String::as_str), Some("■ You've hit your usage limit."));
    assert!(book::engine_away(&saved, "codex", 4_999).is_some());
    assert!(book::engine_away(&saved, "codex", 5_000).is_none());
    assert_eq!(book::due(&saved, 5_000).len(), 1);
    let unknown = Report { terminal: "m4".into(), engine: "claude".into(), until: None, ..Report::default() };
    let made = add(&again, &unknown, 1_000).unwrap();
    assert!(!made.known && made.until == 1_000 + book::UNKNOWN_MS);
}

#[test]
fn a_handled_line_does_not_come_back_until_the_agent_works_again() {
    let mut b = Book::default();
    assert!(book::add(&mut b, leave("m1", "codex", 10, "limit line")));
    book::remove(&mut b, "m1");
    assert!(!book::add(&mut b, leave("m1", "codex", 20, "limit line")));
    assert!(book::add(&mut b, leave("m1", "codex", 20, "another limit line")));
    book::remove(&mut b, "m1");
    b.handled.remove("m1");
    assert!(book::add(&mut b, leave("m1", "codex", 30, "limit line")));
    assert!(book::forget(&mut b, &[]));
    assert!(b.handled.is_empty());
}

#[test]
fn engine_names_match_across_spellings() {
    assert!(book::same_engine("agy", "antigravity"));
    assert!(book::same_engine("cursor-agent", "cursor"));
    assert!(book::same_engine("member:codex", "codex"));
    assert!(book::same_engine("codex::gpt-5", "Codex"));
    assert!(!book::same_engine("", ""));
    assert!(known_engine("claude") && known_engine("agy") && known_engine("crush") && known_engine("opencode"));
    assert!(!known_engine("zsh") && !known_engine("python3"));
}

#[test]
fn hand_over_picks_a_free_engine_of_another_kind() {
    let installed = strings(&["claude", "codex", "antigravity", "opencode"]);
    let order = strings(&["claude", "codex", "agy", "opencode"]);
    assert_eq!(book::pick_engine(&installed, &order, &[], "claude").as_deref(), Some("codex"));
    assert_eq!(book::pick_engine(&installed, &order, &strings(&["codex"]), "claude").as_deref(), Some("antigravity"));
    assert_eq!(book::pick_engine(&installed, &order, &strings(&["codex", "agy"]), "claude").as_deref(), Some("opencode"));
    assert_eq!(book::pick_engine(&installed, &[], &[], "codex").as_deref(), Some("claude"));
    assert_eq!(book::pick_engine(&strings(&["claude"]), &order, &[], "claude"), None, "no other engine is free");
    assert_eq!(book::pick_engine(&installed, &order, &strings(&["codex", "agy", "opencode"]), "claude"), None);
}

#[test]
fn hand_over_starts_after_the_engine_that_left_not_at_claude() {
    let installed = strings(&["claude", "codex", "antigravity", "opencode"]);
    let order = strings(&["claude", "codex", "agy", "opencode"]);
    assert_eq!(book::pick_engine(&installed, &order, &[], "codex").as_deref(), Some("antigravity"));
    assert_eq!(book::pick_engine(&installed, &order, &[], "antigravity").as_deref(), Some("opencode"));
    assert_eq!(book::pick_engine(&installed, &order, &[], "opencode").as_deref(), Some("claude"));
    assert_eq!(book::pick_engine(&installed, &order, &strings(&["claude"]), "opencode").as_deref(), Some("codex"));
}

#[test]
fn chat_lines_are_plain_and_name_the_move() {
    let moved = Moved::Moved { title: "#4 Fix login".into(), to_title: "Claude Code".into() };
    assert_eq!(lines::chat_line("Codex", "back at 17:00", &moved), "Codex hit its usage limit (back at 17:00). Task #4 Fix login moved to Claude Code.");
    let waits = lines::chat_line("Codex", "back at 17:00", &Moved::Waiting { title: "#4 Fix login".into() });
    assert!(waits.contains("No other agent is free") && waits.contains("#4 Fix login waits"));
    assert_eq!(lines::chat_line("Codex", "back at 17:00", &Moved::Nothing), "Codex hit its usage limit (back at 17:00).");
    let z = zone::utc();
    let day = 86_400_000;
    assert_eq!(lines::back_part(&z, day + 17 * 3_600_000, true, day + 1), "back at 17:00");
    assert_eq!(lines::back_part(&z, 4 * day + 9 * 3_600_000, true, day), "back at Mon 09:00");
    assert!(lines::back_part(&z, 0, false, 0).contains("1 hour"));
    assert_eq!(lines::when_word(&z, 4 * day + 9 * 3_600_000, true, day), "Monday");
    let note = lines::handoff_note("Nova", "Codex", "m2", "back at 17:00", "tests half done", "fix-login", "/tmp/shop");
    assert!(note.starts_with(crate::queue::prompt::HANDOFF_PREFIX));
    assert!(note.contains("tests half done") && note.contains("Branch: fix-login") && note.contains("Folder: /tmp/shop"));
}

#[test]
fn office_lines_are_light_and_keep_names() {
    for seed in 0..12 {
        let line = lines::fallback_leave("Nova", "Orbit", "Monday", seed);
        assert!(line.contains("Nova") && line.contains("Orbit") && line.contains("Monday"), "{line}");
        let alone = lines::fallback_leave("Nova", "", "17:00", seed);
        assert!(alone.contains("Nova") && !alone.contains("{to}"), "{alone}");
        assert!(lines::fallback_back("Nova", seed).contains("Nova"));
    }
    assert_eq!(lines::good_line(" Nova  is on the beach,  Orbit works ", &["Nova", "Orbit"]).as_deref(), Some("Nova is on the beach, Orbit works"));
    assert!(lines::good_line("Someone is on the beach", &["Nova"]).is_none());
    assert!(lines::good_line(&"x".repeat(200), &[]).is_none());
}

#[tokio::test]
async fn a_limit_without_a_task_sends_the_leave_event_once() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let mut rx = app.out.subscribe();
    let report = Report { terminal: "m9".into(), name: "Nova".into(), engine: "codex".into(), until: Some(now_ms() + 3_600_000), by: BY_AGENT.into(), ..Report::default() };
    let text = super::report(&app, report.clone(), None).await.unwrap();
    assert!(text.starts_with("Codex hit its usage limit (back at "), "{text}");
    let mut leave_events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if event["type"] == LEAVE_EVENT {
            leave_events.push(event);
        }
    }
    assert_eq!(leave_events.len(), 1);
    let event = &leave_events[0];
    assert_eq!(event["id"], "m9");
    assert_eq!(event["name"], "Nova");
    assert!(event["until"].as_u64().unwrap() > 0);
    assert!(event["line"].as_str().unwrap().contains("Nova"));
    assert_eq!(event["to"], "");
    assert_eq!(event["to_name"], "");
    assert!(super::report(&app, report, None).await.unwrap_err().contains("already on leave"));
    assert!(away_text(&app, "codex").is_some_and(|t| t.contains("Codex is on leave")));
    assert!(away_text(&app, "claude").is_none());
    assert_eq!(away_in(dir.path()), vec!["codex".to_string()]);
    assert_eq!(view(&app)["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_back_event_comes_when_the_time_passes() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let mut rx = app.out.subscribe();
    book::add(&mut lock(&app.leave.book), leave("m3", "claude", 1_000, ""));
    lock(&app.leave.book).items[0].name = "Orbit".into();
    let due = book::due(&lock(&app.leave.book), 2_000);
    come_back(&app, &due[0], true).await;
    save(&app);
    assert!(lock(&app.leave.book).items.is_empty());
    let mut back = None;
    while let Ok(event) = rx.try_recv() {
        if event["type"] == BACK_EVENT {
            back = Some(event);
        }
    }
    let back = back.expect("agent_back sent");
    assert_eq!(back, back_event("m3", "Orbit", back["line"].as_str().unwrap()));
    assert!(back["line"].as_str().unwrap().contains("Orbit"));
    assert!(book::load(&dir.path().join(book::FILE)).items.is_empty());
}

#[tokio::test]
async fn the_open_task_of_the_terminal_is_found_and_moved_only_once() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let conn = crate::queue::db(&app).unwrap();
    let by = Actor::new(AuthorKind::Nixa, "Nixa");
    let task = tasks::add_task(&conn, &TaskInput { title: "Fix login".into(), ..Default::default() }, &by).unwrap();
    tasks::set_status(&conn, task.id, TaskStatus::Todo, None, false).unwrap();
    assert!(tasks::claim_task(&conn, task.id, &Actor::in_terminal(AuthorKind::Agent, "dev", "m5"), "").unwrap().ok);
    assert_eq!(open_task(&conn, "m5").map(|t| t.id), Some(task.id));
    assert!(open_task(&conn, "m6").is_none());
    lock(&app.leave.moving).insert(task.id);
    let l = leave("m5", "codex", now_ms() + 10_000, "");
    assert!(matches!(hand(&app, &l, None).await, Moved::Nothing), "a task that is already moving is not moved again");
    lock(&app.leave.moving).remove(&task.id);
}

#[test]
fn leave_and_back_events_follow_the_shared_shape() {
    assert_eq!(
        leave_event("m1", "Nova", 5, "gone to the beach", "m2", "Orbit"),
        json!({"type": "agent_leave", "id": "m1", "name": "Nova", "until": 5, "line": "gone to the beach", "to": "m2", "to_name": "Orbit"})
    );
    assert_eq!(back_event("m1", "", ""), json!({"type": "agent_back", "id": "m1", "name": "", "line": ""}));
}

#[tokio::test]
async fn a_closed_terminal_leaves_quietly_with_no_back_event() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let mut rx = app.out.subscribe();
    book::add(&mut lock(&app.leave.book), leave("m7", "codex", 1_000, ""));
    returns(&app, 2_000, &zone::utc()).await;
    assert!(lock(&app.leave.book).items.is_empty());
    while let Ok(event) = rx.try_recv() {
        assert_ne!(event["type"], BACK_EVENT);
    }
}

#[test]
fn an_unknown_leave_is_kept_only_for_a_new_hit_or_under_the_cap() {
    let mut l = leave("m1", "codex", 0, "old limit line");
    l.known = false;
    l.since = 0;
    let hit = |line: &str| limits::Hit { line: line.into(), until: None, before: String::new() };
    assert_eq!(keep_away(&l, Some(&hit("old limit line")), EXTEND_CAP_MS - 1).as_deref(), Some("old limit line"));
    assert_eq!(keep_away(&l, Some(&hit("old limit line")), EXTEND_CAP_MS), None, "the same old line does not keep it away for ever");
    assert_eq!(keep_away(&l, Some(&hit("a new limit line")), EXTEND_CAP_MS * 3).as_deref(), Some("a new limit line"));
    assert_eq!(keep_away(&l, None, 10), None);
    l.known = true;
    assert_eq!(keep_away(&l, Some(&hit("a new limit line")), 10), None);
}

#[test]
fn an_asked_engine_that_cannot_take_the_task_says_why() {
    let installed = strings(&["claude", "codex"]);
    assert!(book::why_not(&installed, &[], "claude", "claude").is_some_and(|w| w.contains("same engine")));
    assert!(book::why_not(&installed, &strings(&["codex"]), "claude", "codex").is_some_and(|w| w.contains("on leave")));
    assert!(book::why_not(&installed, &[], "claude", "crush").is_some_and(|w| w.contains("not installed")));
    assert_eq!(book::why_not(&installed, &[], "claude", "codex"), None);
}
