use super::api::{Role, action};
use super::git::{DIR_FILES, MAX_ENTRIES, changed_paths, fingerprint_now, in_print, parse_status, read_base, short_list};
use super::launch::{Seat, seat_of};
use super::lock::{auto_space, folder_mismatch, locked_space, task_locked};
use super::prompt::{FinalWord, final_word, go_on_text, named_task};
use super::scope::{Launch, Scope, Wait, start_wait};
use super::sweep::{check_inbox, same_title};
use super::*;
use nixa_store::spaces::SpacePatch;
use nixa_store::tasks::{TaskInput, TaskStatus};
use serde_json::json;

fn space(id: &str, folder: &str) -> Space {
    Space {
        id: id.into(),
        name: id.into(),
        folder: folder.into(),
    }
}

fn scope(repo: &str, files: &[&str]) -> Scope {
    Scope {
        repo: repo.into(),
        files: files.iter().map(|f| f.to_string()).collect(),
    }
}

fn app_with_space(auto: bool) -> (tempfile::TempDir, Arc<App>, String) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let folder = dir.path().join("work").display().to_string();
    std::fs::create_dir_all(&folder).unwrap();
    let conn = db(&app).unwrap();
    let patch = SpacePatch {
        name: Some("nixa".into()),
        folder: Some(folder.clone()),
        auto_start: Some(auto),
        ..SpacePatch::default()
    };
    nixa_store::spaces::set_space(&conn, "nixa", &patch).unwrap();
    (dir, app, folder)
}

fn add(app: &App, title: &str, status: TaskStatus, assigned: &str) -> Task {
    let conn = db(app).unwrap();
    let input = TaskInput {
        title: title.into(),
        status: Some(status),
        space_id: "nixa".into(),
        space_name: "nixa".into(),
        assigned_kind: crate::tasks::KIND_TERMINAL.into(),
        assigned: assigned.into(),
        ..TaskInput::default()
    };
    tasks::add_task(&conn, &input, &nixa()).unwrap()
}

#[test]
fn the_start_text_carries_the_project_rules() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().display().to_string();
    assert_eq!(super::start::with_rules("Task #1".into(), &folder), "Task #1");
    assert_eq!(super::start::with_rules("Task #1".into(), ""), "Task #1");
    std::fs::create_dir_all(dir.path().join("docs")).unwrap();
    std::fs::write(dir.path().join("docs/nixa.md"), "## Never\n- secrets/\n\n## Branch\n- main\n").unwrap();
    assert_eq!(super::start::with_rules("Task #1".into(), &folder), "Task #1\nProject rules from docs/nixa.md: never touch secrets/. Branch: main.");
}

#[test]
fn a_program_start_records_why_no_deep_brain_can_pick() {
    use crate::extras::model_pick::{ASK_KEY, Pick};
    use super::pick;
    let (_dir, app, folder) = app_with_space(false);
    let task = add(&app, "Fix the cart", TaskStatus::Todo, "claude");
    let sp = space("nixa", &folder);
    assert!(!pick::hold(&app, &task, &sp, &Seat::Custom("a1".into()), How::Now, true));
    assert!(!pick::has_pick(&db(&app).unwrap(), task.id));
    assert!(!pick::hold(&app, &task, &sp, &Seat::Program("claude".into()), How::Now, true));
    let conn = db(&app).unwrap();
    assert!(pick::has_pick(&conn, task.id));
    assert!(pick::usable(&app, &task).is_none());
    assert!(!pick::hold(&app, &task, &sp, &Seat::Program("claude".into()), How::Now, true));
    let other = add(&app, "Fix the menu", TaskStatus::Todo, "claude");
    let input = crate::deep::AskInput { question: "Which row?".into(), options: Vec::new(), notes: String::new(), kind: Some("model".into()), wait: None, brain: None };
    let made = lock(&app.deep).ask(input, &sp, true, true, crate::util::now_ms()).unwrap().0;
    tasks::set_task_meta(&conn, &format!("{ASK_KEY}{}", made.id), &other.id.to_string()).unwrap();
    assert_eq!(pick::ask_of(&conn, other.id), Some(made.id.clone()));
    assert!(pick::waits(&app, &conn, other.id));
    assert!(pick::hold(&app, &other, &sp, &Seat::Program("claude".into()), How::Now, true));
    assert_eq!(tasks::task_meta(&conn, &format!("{}{}", pick::START_KEY, other.id)).unwrap().as_deref(), Some("user"));
    assert_eq!(pick::start_mark(How::Auto, true), None);
    let chosen = Pick { assigned_kind: "agent".into(), assigned: "codex".into(), model: "codex::gpt-5".into(), effort: "high".into(), label: "row 2 (review): codex::gpt-5".into(), ask: made.id.clone(), ..Pick::default() };
    assert_eq!(pick::model_args(&chosen), vec!["--model", "gpt-5", "-c", "model_reasoning_effort=high"]);
    let moved = pick::apply(&app, &conn, other.id, &chosen).unwrap();
    assert_eq!(moved.assigned, "codex");
    tasks::set_task_meta(&conn, &format!("{}{}", crate::extras::model_pick::PICK_KEY, other.id), &serde_json::to_string(&chosen).unwrap()).unwrap();
    assert_eq!(pick::usable(&app, &moved).map(|p| p.model), Some("codex::gpt-5".into()));
    assert!(pick::usable(&app, &other).is_none(), "a pick for another agent is not used");
}

#[test]
fn a_task_wakes_a_deep_brain_and_reuses_its_pending_pick() {
    use super::pick;
    use nixa_store::agent_map::{AgentMapRow, add_row};
    let (_dir, app, folder) = app_with_space(false);
    let conn = db(&app).unwrap();
    add_row(&conn, &AgentMapRow { job: "Build".into(), agent: "claude".into(), model: "claude::sonnet".into(), ..AgentMapRow::default() }).unwrap();
    let task = add(&app, "Fix the cart", TaskStatus::Todo, "claude");
    let sp = space("nixa", &folder);
    assert!(!pick::hold(&app, &task, &sp, &Seat::Custom("a1".into()), How::Now, true));
    assert!(pick::hold(&app, &task, &sp, &Seat::Program("claude".into()), How::Now, true));
    assert!(!crate::followup::flow::deep_ready(&app, &sp.id));
    assert_eq!(lock(&app.deep).open_asks().len(), 1);
    assert_eq!(lock(&app.deep).seats.len(), 1);
    let ask = pick::ask_of(&conn, task.id).unwrap();
    assert!(pick::hold(&app, &task, &sp, &Seat::Program("claude".into()), How::Now, true));
    assert_eq!(pick::ask_of(&conn, task.id).as_deref(), Some(ask.as_str()));
    assert_eq!(lock(&app.deep).open_asks().len(), 1, "a waiting start cannot flood the deep queue");
    let next = add(&app, "Fix another thing", TaskStatus::Todo, "claude");
    app.save_settings(|s| s.deep_auto_start = Some(false)).unwrap();
    assert!(!pick::hold(&app, &next, &sp, &Seat::Program("claude".into()), How::Now, true));
    assert!(pick::has_pick(&conn, next.id));
    assert!(tasks::task_comments(&conn, next.id).unwrap().iter().any(|c| c.text.contains("uses its own agent and model pick")));
}

#[tokio::test]
async fn a_pending_model_pick_counts_against_the_space_limit() {
    use super::pick;
    let (_dir, app, folder) = app_with_space(false);
    let conn = db(&app).unwrap();
    nixa_store::spaces::set_space(&conn, "nixa", &SpacePatch { max_doing: Some(1), ..SpacePatch::default() }).unwrap();
    let first = add(&app, "First task", TaskStatus::Todo, "claude");
    let second = add(&app, "Second task", TaskStatus::Todo, "claude");
    let sp = space("nixa", &folder);
    let (ask, _) = crate::deep_service::ask(&app, crate::deep::AskInput { question: "Who?".into(), ..Default::default() }, &sp).unwrap();
    tasks::set_task_meta(&conn, &format!("{}{}", crate::extras::model_pick::ASK_KEY, ask.id), &first.id.to_string()).unwrap();
    assert!(pick::waits(&app, &conn, first.id));
    super::start::start_task(&app, second.clone(), &sp, How::Now, true).await.unwrap();
    assert_eq!(lock(&app.deep).open_asks().len(), 1);
    let comments = tasks::task_comments(&conn, second.id).unwrap();
    assert!(comments.iter().any(|c| c.text.contains("parallel task limit")), "{comments:?}");
}

#[tokio::test]
async fn a_held_manual_start_stays_until_it_launches() {
    use crate::extras::model_pick::{ASK_KEY, PICK_KEY, Pick};
    use super::pick;
    let (_dir, app, folder) = app_with_space(false);
    let conn = db(&app).unwrap();
    let task = add(&app, "Fix the cart", TaskStatus::Todo, "claude");
    let sp = space("nixa", &folder);
    let input = crate::deep::AskInput { question: "Which row?".into(), options: Vec::new(), notes: String::new(), kind: Some("model".into()), wait: None, brain: None };
    let made = lock(&app.deep).ask(input, &sp, true, true, crate::util::now_ms()).unwrap().0;
    tasks::set_task_meta(&conn, &format!("{ASK_KEY}{}", made.id), &task.id.to_string()).unwrap();
    assert!(pick::hold(&app, &task, &sp, &Seat::Program("claude".into()), How::Now, true));
    assert!(pick::due_starts(&app, &conn).is_empty(), "the start waits for the deep brain");
    lock(&app.deep).asks.retain(|a| a.id != made.id);
    tasks::remove_task_meta(&conn, &format!("{ASK_KEY}{}", made.id)).unwrap();
    let chosen = Pick { assigned_kind: "agent".into(), assigned: "claude".into(), model: "claude::sonnet".into(), label: "row 1".into(), ask: made.id.clone(), ..Pick::default() };
    tasks::set_task_meta(&conn, &format!("{PICK_KEY}{}", task.id), &serde_json::to_string(&chosen).unwrap()).unwrap();
    std::fs::write(app.dir.join(crate::settings_api::OFF_FILE), "").unwrap();
    pick::picked(&app, task.id, &chosen);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(pick::marks(&conn), vec![(task.id, "user".to_string())], "a pick that lands while Nixa is off keeps the start");
    std::fs::remove_file(app.dir.join(crate::settings_api::OFF_FILE)).unwrap();
    assert_eq!(pick::due_starts(&app, &conn), vec![(task.id, true)], "the queue tick starts it when Nixa is on again");
    tasks::set_status(&conn, task.id, TaskStatus::Cancelled, None, false).unwrap();
    assert!(pick::due_starts(&app, &conn).is_empty());
    assert!(pick::marks(&conn).is_empty(), "a closed task drops its start");
}

#[test]
fn a_deep_brain_hold_closes_or_parks_the_task() {
    use crate::extras::model_pick::{Hold, PICK_KEY, Pick};
    use super::pick;
    let (_dir, app, _folder) = app_with_space(false);
    let conn = db(&app).unwrap();
    let cancel = add(&app, "Old idea", TaskStatus::Todo, "claude");
    let hold = |h| Pick { none: true, ask: "d4".into(), why: "not needed".into(), hold: Some(h), ..Pick::default() };
    tasks::set_task_meta(&conn, &format!("{PICK_KEY}{}", cancel.id), "x").unwrap();
    assert_eq!(pick::apply(&app, &conn, cancel.id, &hold(Hold::Cancel)).unwrap().status, TaskStatus::Cancelled);
    assert!(!pick::has_pick(&conn, cancel.id));
    let done = add(&app, "Done already", TaskStatus::Todo, "claude");
    assert_eq!(pick::apply(&app, &conn, done.id, &hold(Hold::Done)).unwrap().status, TaskStatus::Done);
    let wait = add(&app, "Later", TaskStatus::Todo, "claude");
    assert_eq!(pick::apply(&app, &conn, wait.id, &hold(Hold::Wait)).unwrap().status, TaskStatus::Todo);
    assert!(inner(&app).start_failed.contains_key(&wait.id));
    let notes = tasks::task_comments(&conn, cancel.id).unwrap();
    assert!(notes.iter().any(|c| c.text.contains("Nixa cancelled the task and started nothing")));
}

fn args(value: serde_json::Value) -> crate::tools::Args {
    value.as_object().cloned().unwrap()
}

#[test]
fn locked_space_holds_its_tasks() {
    let rows = vec![nixa_store::spaces::Space {
        space_id: "web".into(),
        name: "Web".into(),
        folder: "/work/web".into(),
        auto_start: true,
        max_doing: 8,
        locked: true,
        repos: Vec::new(),
        color: 0,
        position: 0,
        created_at: 0,
        updated_at: 0,
    }];
    assert_eq!(locked_space(&rows, "web", "", "").unwrap().name, "Web");
    assert!(locked_space(&rows, "", "web", "").is_some());
    assert!(locked_space(&rows, "", "", "/work/web/src").is_some());
    assert!(locked_space(&rows, "other", "other", "/work/other").is_none());
    let mut task = blank_task();
    task.repo = "/work/web/app".into();
    assert!(task_locked(&task, &rows).is_some());
    assert!(locked_text("Web").contains("the user unlocks it"));
}

fn blank_task() -> Task {
    let conn = nixa_store::open_memory().unwrap();
    tasks::add_task(
        &conn,
        &TaskInput {
            title: "x".into(),
            ..TaskInput::default()
        },
        &nixa(),
    )
    .unwrap()
}

#[test]
fn folder_outside_the_space_is_held() {
    let known = vec![space("nixa", "/work/nixa"), space("web", "/work/web")];
    assert!(folder_mismatch(&known, &known[0], "/work/nixa/crates").is_none());
    let found = folder_mismatch(&known, &known[0], "/work/web/src").unwrap();
    assert!(found.contains("belongs to space \"web\""));
    assert!(!found.contains("the user"), "the mode decides who fixes the folder");
    assert!(folder_mismatch(&known, &known[0], "relative/path").is_none());
    assert_eq!(super::lock::folder_owner(&known, &known[0], "/work/web/src").map(|s| s.id.as_str()), Some("web"));
    assert!(super::lock::folder_owner(&known, &known[0], "/elsewhere").is_none());
    assert!(auto_space(&space("x", "/work/x/")));
    assert!(auto_space(&space("web", "/work/web")));
}

#[test]
fn start_waits_for_files_and_the_limit() {
    let mine = scope("/r", &["src/a.rs"]);
    let other = scope("/r", &["src"]);
    let free = scope("/r", &["docs/x.md"]);
    let launches = HashMap::new();
    let wait = Wait {
        task: 2,
        space: "nixa",
        scope: &mine,
        active: &[(1, other)],
        launches: &launches,
        doing: &[1],
        max_doing: 8,
    };
    assert_eq!(
        start_wait(&wait).as_deref(),
        Some("files overlap with task #1")
    );
    let wait = Wait {
        task: 2,
        space: "nixa",
        scope: &mine,
        active: &[(1, free.clone())],
        launches: &launches,
        doing: &[1],
        max_doing: 1,
    };
    assert_eq!(
        start_wait(&wait).as_deref(),
        Some("the space has reached its parallel task limit")
    );
    let mut starting = HashMap::new();
    starting.insert(
        3,
        Launch {
            space: "nixa".into(),
            scope: scope("/r", &["src/a.rs"]),
        },
    );
    let wait = Wait {
        task: 2,
        space: "nixa",
        scope: &mine,
        active: &[],
        launches: &starting,
        doing: &[],
        max_doing: 8,
    };
    assert_eq!(
        start_wait(&wait).as_deref(),
        Some("files overlap with task #3")
    );
    let wait = Wait {
        task: 2,
        space: "nixa",
        scope: &mine,
        active: &[(1, free)],
        launches: &launches,
        doing: &[1],
        max_doing: 2,
    };
    assert!(start_wait(&wait).is_none());
}

#[test]
fn git_status_and_changes() {
    let out = b" M src/a.rs\0R  new.rs\0old.rs\0?? notes.txt\0";
    assert_eq!(
        parse_status(out),
        vec!["src/a.rs", "new.rs", "old.rs", "notes.txt"]
    );
    let base = read_base(r#"{"a":"1","b":"2"}"#).unwrap();
    let now = read_base(r#"{"a":"1","b":"3","c":"4"}"#).unwrap();
    assert_eq!(changed_paths(&base, &now), vec!["b", "c"]);
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().display().to_string();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&folder)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    let first = fingerprint_now(&folder).unwrap();
    assert!(first.contains_key("a.txt"));
    std::fs::write(dir.path().join("a.txt"), "two").unwrap();
    let second = fingerprint_now(&folder).unwrap();
    assert_eq!(changed_paths(&first, &second), vec!["a.txt"]);
}

#[test]
fn a_big_untracked_folder_still_gives_a_start_point() {
    let strs = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut all = strs(&["a.rs", "small/x.txt", "small/y.txt"]);
    all.extend((0..=DIR_FILES).map(|n| format!("big/{n}.o")));
    all.sort();
    let normal = strs(&["a.rs", "big/", "small/"]);
    let names = |list: Vec<(String, Option<usize>)>| list.into_iter().map(|(p, _)| p).collect::<Vec<_>>();
    assert_eq!(names(short_list(&all, &normal, DIR_FILES)), strs(&["a.rs", "big/", "small/x.txt", "small/y.txt"]));
    assert_eq!(names(short_list(&all, &normal, 0)), normal);
    let base = read_base(r#"{"a.rs":"1","big/":"folder of 9 files","small/x.txt":"2"}"#).unwrap();
    let now = read_base(r#"{"a.rs":"1","big/":"folder of 9 files","small/":"folder of 300 files"}"#).unwrap();
    assert_eq!(changed_paths(&base, &now), strs(&["small/"]));
    assert!(in_print(&now, "small/x.txt") && in_print(&base, "small/"));
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().display().to_string();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&folder).status().unwrap().success());
    git(&["init", "-q"]);
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    git(&["add", "a.txt"]);
    std::fs::create_dir_all(dir.path().join("target")).unwrap();
    for n in 0..=MAX_ENTRIES {
        std::fs::write(dir.path().join(format!("target/{n}.o")), "x").unwrap();
    }
    std::fs::create_dir_all(dir.path().join("notes")).unwrap();
    std::fs::write(dir.path().join("notes/plan.md"), "plan").unwrap();
    let first = fingerprint_now(&folder).unwrap();
    assert!(first.contains_key("a.txt") && first.contains_key("target/") && first.contains_key("notes/plan.md"), "{first:?}");
    assert!(in_print(&first, "target/7.o") && !in_print(&first, "src/b.rs"));
    std::fs::write(dir.path().join("a.txt"), "two").unwrap();
    std::fs::write(dir.path().join("notes/todo.md"), "todo").unwrap();
    let second = fingerprint_now(&folder).unwrap();
    assert_eq!(changed_paths(&first, &second), vec!["a.txt", "notes/todo.md"]);
    std::fs::write(dir.path().join("target/new.o"), "y").unwrap();
    let third = fingerprint_now(&folder).unwrap();
    assert_eq!(changed_paths(&second, &third), vec!["target/"]);
}

#[test]
fn words_from_reports_and_jobs() {
    assert_eq!(final_word("work\nall good\nDONE"), Some(FinalWord::Done));
    assert_eq!(
        final_word("stuck\nBLOCKED: no key"),
        Some(FinalWord::Blocked)
    );
    assert_eq!(
        final_word("NEEDS USER: which port?"),
        Some(FinalWord::NeedsUser)
    );
    assert_eq!(final_word("still going"), None);
    assert_eq!(named_task("Task #42 fix the chat"), Some(42));
    assert_eq!(named_task("Please work on task #7 now"), Some(7));
    assert_eq!(named_task("fix 7 things"), None);
    assert_eq!(same_title("Fix the Chat!"), same_title("fix  the chat"));
}

#[test]
fn seats_and_skip_flags() {
    let mut task = blank_task();
    task.assigned = "m3".into();
    task.assigned_kind = crate::tasks::KIND_TERMINAL.into();
    assert_eq!(seat_of(&task), Some(Seat::Terminal("m3".into())));
    task.assigned = "claude".into();
    assert_eq!(seat_of(&task), Some(Seat::Program("claude".into())));
    task.assigned = "a1".into();
    task.assigned_kind = crate::tasks::KIND_CUSTOM.into();
    assert_eq!(seat_of(&task), Some(Seat::Custom("a1".into())));
    task.assigned = String::new();
    assert_eq!(seat_of(&task), None);
    assert!(nixa_store::agents::skips_agent_checks(&[
        "claude".into(),
        "--dangerously-skip-permissions".into()
    ]));
    assert!(nixa_store::agents::skips_agent_checks(&[
        "sh".into(),
        "-c".into(),
        "codex --yolo".into()
    ]));
    assert!(!nixa_store::agents::skips_agent_checks(&[
        "claude".into(),
        "--model".into(),
        "x".into()
    ]));
}

#[test]
fn go_on_text_names_the_task() {
    let mut task = blank_task();
    task.title = "Fix the chat".into();
    let text = go_on_text(&task, &[]);
    assert!(text.contains(&format!("task #{}", task.id)));
    assert!(text.contains("$NIXA_BIN task status"));
}

#[test]
fn inbox_moves_ready_tasks_and_drops_twins() {
    let (_dir, app, _) = app_with_space(false);
    let one = add(&app, "Fix the chat", TaskStatus::Inbox, "claude");
    let two = add(&app, "fix the chat!", TaskStatus::Inbox, "claude");
    check_inbox(&app);
    let conn = db(&app).unwrap();
    assert_eq!(
        tasks::get_task(&conn, one.id).unwrap().unwrap().status,
        TaskStatus::Todo
    );
    assert_eq!(
        tasks::get_task(&conn, two.id).unwrap().unwrap().status,
        TaskStatus::Cancelled
    );
    let said = tasks::task_comments(&conn, two.id).unwrap();
    assert!(
        said.iter()
            .any(|c| c.text.contains(&format!("Same as #{}", one.id)))
    );
}

#[test]
fn the_inbox_event_names_the_agent_options() {
    let (_dir, app, _) = app_with_space(false);
    let conn = db(&app).unwrap();
    let add = |title: &str, off: bool| nixa_store::agents::add_agent(&conn, &nixa_store::agents::AddAgent { title: title.into(), purpose: "Normal fixes".into(), space: "nixa".into(), space_name: "nixa".into(), off, ..Default::default() }, "Nixa").unwrap();
    let on = add("Developer", false);
    add("Retired", true);
    let task = blank_task();
    let text = super::sweep::inbox_event(std::slice::from_ref(&task), "nixa", &super::sweep::agent_options(&conn, &super::places(&conn)));
    assert!(text.contains(&format!("Agents: {} \"Developer\", space nixa: Normal fixes.", on.id)), "{text}");
    assert!(!text.contains("Retired"), "{text}");
    let auto = super::sweep::inbox_event(std::slice::from_ref(&task), "nixa", "none");
    assert!(auto.contains(super::sweep::INBOX_AUTO), "{auto}");
    assert!(!auto.contains("The user picks it"), "{auto}");
}

#[tokio::test]
async fn start_now_queues_at_the_space_limit() {
    let (_dir, app, _) = app_with_space(false);
    let conn = db(&app).unwrap();
    nixa_store::spaces::set_space(
        &conn,
        "nixa",
        &SpacePatch {
            max_doing: Some(1),
            ..SpacePatch::default()
        },
    )
    .unwrap();
    let held = add(&app, "First job", TaskStatus::Todo, "claude");
    let worker = Actor::in_terminal(AuthorKind::Agent, "worker", "m9");
    assert!(tasks::claim_task(&conn, held.id, &worker, "").unwrap().ok);
    let next = add(&app, "Second job", TaskStatus::Todo, "claude");
    let task = start_now(&app, next.id, true).await.unwrap();
    assert!(task.terminal_id.is_empty());
    let said = tasks::task_comments(&conn, next.id).unwrap();
    assert!(
        said.iter().any(|c| c.text.starts_with("Start now queued:")),
        "{said:?}"
    );
    assert!(inner(&app).queued.contains_key(&next.id));
    assert!(inner(&app).starting.is_empty());
}

#[tokio::test]
async fn start_now_refuses_locked_and_unready_tasks() {
    let (_dir, app, _) = app_with_space(false);
    let none = add(&app, "Nobody assigned", TaskStatus::Todo, "");
    assert!(
        start_now(&app, none.id, true)
            .await
            .unwrap_err()
            .contains("no one assigned")
    );
    let task = add(&app, "Locked work", TaskStatus::Todo, "claude");
    let conn = db(&app).unwrap();
    nixa_store::spaces::set_space(
        &conn,
        "nixa",
        &SpacePatch {
            locked: Some(true),
            ..SpacePatch::default()
        },
    )
    .unwrap();
    assert!(
        start_now(&app, task.id, true)
            .await
            .unwrap_err()
            .contains("is locked")
    );
    assert!(
        start_now(&app, 999, true)
            .await
            .unwrap_err()
            .contains("No task #999")
    );
}

#[tokio::test]
async fn space_lock_accepts_the_brain_but_refuses_terminal_agents() {
    let (_dir, app, _) = app_with_space(false);
    let brain = Role::Brain;
    let saved = action(
        &app,
        "space",
        &args(json!({"space": "nixa", "autoStart": true, "maxDoing": 3})),
        &brain,
    )
    .await
    .unwrap();
    assert_eq!(saved["space"]["max_doing"], 3);
    assert!(
        action(
            &app,
            "space",
            &args(json!({"space": "nixa", "locked": true})),
            &brain
        )
        .await
        .is_ok()
    );
    assert!(action(&app, "space", &args(json!({"space": "nixa", "locked": false})), &Role::Terminal("m1".into())).await.unwrap_err().contains("Only the user"));
    assert!(
        action(
            &app,
            "space",
            &args(json!({"space": "nixa", "maxDoing": 50})),
            &Role::User
        )
        .await
        .is_err()
    );
    assert!(
        action(
            &app,
            "space",
            &args(json!({"space": "nixa", "locked": true})),
            &Role::User
        )
        .await
        .is_ok()
    );
    let list = action(&app, "spaces", &args(json!({})), &brain)
        .await
        .unwrap();
    assert_eq!(list["spaces"][0]["locked"], true);
    assert!(
        action(&app, "pull", &args(json!({"dry": true})), &Role::User)
            .await
            .unwrap_err()
            .contains("does not know the action pull")
    );
    assert!(
        action(
            &app,
            "start",
            &args(json!({"id": 1})),
            &Role::Terminal("m1".into())
        )
        .await
        .is_err()
    );
    assert!(
        action(&app, "nope", &args(json!({})), &brain)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn next_skips_claimed_and_locked() {
    let (_dir, app, _) = app_with_space(false);
    let first = add(&app, "First", TaskStatus::Todo, "claude");
    add(&app, "Second", TaskStatus::Todo, "claude");
    let got = action(&app, "next", &args(json!({"space": "nixa"})), &Role::User)
        .await
        .unwrap();
    assert_eq!(got["task"]["id"], first.id);
    let conn = db(&app).unwrap();
    nixa_store::spaces::set_space(
        &conn,
        "nixa",
        &SpacePatch {
            locked: Some(true),
            ..SpacePatch::default()
        },
    )
    .unwrap();
    assert!(
        action(&app, "next", &args(json!({"space": "nixa"})), &Role::User)
            .await
            .unwrap_err()
            .contains("locked")
    );
}

#[tokio::test]
async fn checkpoint_needs_a_claim() {
    let (_dir, app, _) = app_with_space(false);
    let task = add(&app, "Job", TaskStatus::Todo, "claude");
    assert!(
        action(
            &app,
            "checkpoint",
            &args(json!({"id": task.id})),
            &Role::User
        )
        .await
        .unwrap_err()
        .contains("no claim")
    );
    let conn = db(&app).unwrap();
    let worker = Actor::in_terminal(AuthorKind::Agent, "worker", "m9");
    assert!(tasks::claim_task(&conn, task.id, &worker, "").unwrap().ok);
    assert!(
        action(
            &app,
            "checkpoint",
            &args(json!({"id": task.id})),
            &Role::Terminal("m2".into())
        )
        .await
        .unwrap_err()
        .contains("not held by terminal m2")
    );
    let saved = action(
        &app,
        "checkpoint",
        &args(json!({"id": task.id, "why": "test"})),
        &Role::Terminal("m9".into()),
    )
    .await
    .unwrap();
    assert_eq!(saved["task"]["id"], task.id);
    let said = tasks::task_comments(&conn, task.id).unwrap();
    assert!(
        said.iter().any(|c| c
            .text
            .starts_with("Work so far from worker in terminal m9 (test).")),
        "{said:?}"
    );
    let last = tasks::task_meta(&conn, &format!("{LAST_KEY}{}", task.id))
        .unwrap()
        .unwrap();
    assert!(last.contains("m9"));
}


#[tokio::test]
async fn tasks_window_actions_change_tasks() {
    let (_dir, app, _folder) = app_with_space(false);
    let user = Role::User;
    let added = action(&app, "add", &args(json!({"title": "Window task", "space": "nixa", "description": "from the window"})), &user).await.unwrap();
    let id = added["task"]["id"].as_i64().unwrap();
    let options = action(&app, "options", &args(json!({})), &user).await.unwrap();
    assert_eq!(options["spaces"][0]["id"], "nixa");
    assert!(options.get("pulls").is_none());
    let listed = action(&app, "list", &args(json!({"status": "todo", "space": "nixa"})), &user).await.unwrap();
    assert_eq!(listed["tasks"][0]["id"].as_i64(), Some(id));
    assert_eq!(listed["tasks"][0]["comments"].as_i64(), Some(0));
    action(&app, "comment", &args(json!({"id": id, "text": "first note"})), &user).await.unwrap();
    let shown = action(&app, "show", &args(json!({"id": id})), &user).await.unwrap();
    assert_eq!(shown["comments"].as_array().map(Vec::len), Some(1));
    let moved = action(&app, "update", &args(json!({"id": id, "status": "blocked", "on": "other", "title": "Renamed"})), &user).await.unwrap();
    assert_eq!((moved["task"]["status"].as_str(), moved["task"]["title"].as_str()), (Some("blocked"), Some("Renamed")));
    action(&app, "archive", &args(json!({"id": id})), &user).await.unwrap();
    let archived = action(&app, "list", &args(json!({"archived": true})), &user).await.unwrap();
    assert_eq!(archived["tasks"].as_array().map(Vec::len), Some(1));
    assert!(action(&app, "delete", &args(json!({"id": id})), &user).await.unwrap_err().contains("confirm"));
    action(&app, "delete", &args(json!({"id": id, "confirm": true})), &user).await.unwrap();
    assert!(action(&app, "show", &args(json!({"id": id})), &user).await.is_err());
}

#[tokio::test]
async fn window_message_reads_the_top_level_action() {
    let (_dir, app, _folder) = app_with_space(false);
    let reply = super::api::ws_request(&app, &json!({"type": "tasks", "request": "r1", "action": "add", "args": {"title": "Top level"}})).await.unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["data"]["task"]["title"], "Top level");
}


#[test]
fn start_kinds_pick_the_work_mode_action() {
    assert_eq!(super::start::start_action(super::start::How::HandOff), "hand-off");
    assert_eq!(super::start::start_action(super::start::How::HandOver), "hand-off");
    assert_eq!(super::start::start_action(super::start::How::Auto), "auto-start");
    assert_eq!(super::start::start_action(super::start::How::Now), "start-task");
    assert_eq!(super::start::start_action(super::start::How::Continue), "start-task");
}

#[tokio::test]
async fn tasks_stop_reading_stops_the_read() {
    let (_dir, app, _space) = app_with_space(false);
    let before = lock(&app.talk.speaker).generation;
    let out = super::window::action(&app, "speak_stop", &args(json!({}))).await.unwrap();
    assert_eq!(out["stopped"], true);
    assert_eq!(lock(&app.talk.speaker).generation, before);
    assert!(lock(&app.talk.speaker).line.is_none());
}

#[tokio::test]
async fn tasks_stop_reading_drops_a_held_read_only() {
    let (_dir, app, _space) = app_with_space(false);
    let added = super::window::action(&app, "add", &args(json!({"title": "Held read", "space": "nixa"}))).await.unwrap();
    let id = added["task"]["id"].as_i64().unwrap();
    crate::talk::hold::start(&app, "p1");
    assert!(crate::talk::hold::catch(&app, "Build finished in shop.", &crate::talk::speech::SayOptions::nixa("chat")));
    super::window::action(&app, "speak", &args(json!({"id": id}))).await.unwrap();
    assert_eq!(lock(&app.talk.hold).held(), 2);
    let out = super::window::action(&app, "speak_stop", &args(json!({}))).await.unwrap();
    assert_eq!(out["dropped"], 1);
    assert_eq!(lock(&app.talk.hold).texts(), vec!["Build finished in shop.".to_string()]);
}

async fn held_read_then(change: &str) {
    let (_dir, app, _space) = app_with_space(false);
    let added = super::window::action(&app, "add", &args(json!({"title": "Held read", "space": "nixa", "description": "old words"}))).await.unwrap();
    let id = added["task"]["id"].as_i64().unwrap();
    crate::talk::hold::start(&app, "p1");
    let chat = format!("Task {id}: Held read. old words");
    assert!(crate::talk::hold::catch(&app, &chat, &crate::talk::speech::SayOptions::nixa("chat")));
    assert!(crate::talk::hold::catch(&app, "Build finished in shop.", &crate::talk::speech::SayOptions::nixa("chat")));
    let read = super::window::action(&app, "speak", &args(json!({"id": id}))).await.unwrap();
    assert_eq!(read["held"], true);
    assert_eq!(lock(&app.talk.hold).held(), 3);
    match change {
        "edit" => {
            super::window::action(&app, "update", &args(json!({"id": id, "title": "New title", "description": "new words"}))).await.unwrap();
        }
        _ => {
            super::window::action(&app, "delete", &args(json!({"id": id, "confirm": true}))).await.unwrap();
        }
    }
    let out = super::window::action(&app, "speak_stop", &args(json!({}))).await.unwrap();
    assert_eq!(out["dropped"], 1, "{change}");
    assert_eq!(lock(&app.talk.hold).texts(), vec![chat, "Build finished in shop.".to_string()], "{change}");
    assert!(lock(&app.talk.hold).reads().is_empty(), "{change}");
}

#[tokio::test]
async fn tasks_stop_reading_drops_a_held_read_after_an_edit() {
    held_read_then("edit").await;
}

#[tokio::test]
async fn tasks_stop_reading_drops_a_held_read_after_a_delete() {
    held_read_then("delete").await;
}

#[tokio::test]
async fn task_window_finds_prefixed_custom_assignments() {
    let (_dir, app, _space) = app_with_space(false);
    let action = super::window::action;
    let custom = action(&app, "add", &args(json!({"title": "Custom assignment", "assign": "custom:scout", "then": "custom:reviewer"}))).await.unwrap();
    action(&app, "add", &args(json!({"title": "Plain assignment", "assign": "scout"}))).await.unwrap();

    let list = action(&app, "list", &args(json!({"assigned": "custom:scout"}))).await.unwrap();
    assert_eq!(list["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(list["tasks"][0]["id"], custom["task"]["id"]);
    assert_eq!(list["tasks"][0]["then_member"], "custom:reviewer");
}

#[tokio::test]
async fn a_held_read_waits_until_stopped_and_preserves_other_reads() {
    use crate::talk::hold;
    let (_dir, app, _space) = app_with_space(false);
    let action = super::window::action;
    let start_read = |title| {
        let app = &app;
        async move {
            let task = action(app, "add", &args(json!({"title": title}))).await.unwrap();
            action(app, "speak", &args(json!({"id": task["task"]["id"]}))).await.unwrap()
        }
    };
    hold::start(&app, "p1");
    let first = start_read("First task read").await;
    let second = start_read("Second task read").await;
    assert_eq!(first["held"], true);
    assert_eq!(first["read"], first["read_id"]);

    let wanted = args(json!({"read": first["read_id"]}));
    let wait = action(&app, "speak_wait", &wanted);
    tokio::pin!(wait);
    tokio::select! {
        biased;
        _ = &mut wait => panic!("A held read completed before it was released"),
        _ = tokio::task::yield_now() => {}
    }
    assert_eq!(action(&app, "speak_stop", &wanted).await.unwrap()["dropped"], 1);
    assert_eq!(wait.await.unwrap()["read"], first["read"]);
    assert_eq!(hold::stop_reads(&app), 1);
    assert_eq!(action(&app, "speak_wait", &args(json!({"read": second["read_id"]}))).await.unwrap()["read"], second["read_id"]);
}

#[test]
fn checks_line_names_the_shipped_skill_and_never_waits() {
    let line = super::prompt::CHECKS_LINE;
    assert!(line.contains("nixa-delegate") && !line.contains("milo"), "{line}");
    assert!(line.contains("Checked by: self") && line.contains("Never wait"), "{line}");
    assert_eq!(super::prompt::DELEGATE_SKILL, "nixa-delegate");
}

#[test]
fn ask_lines_never_block_the_task() {
    let auto = super::prompt::ask_lines(5).join("\n");
    assert!(!auto.contains("blocked --on"), "{auto}");
    assert!(auto.contains("recommended option") && auto.contains("task comment 5"));
}

#[test]
fn a_task_on_a_closed_terminal_goes_to_its_agent_profile_or_the_inbox() {
    let conn = nixa_store::open_memory().unwrap();
    let by = nixa_store::tasks::Actor::new(nixa_store::tasks::AuthorKind::Nixa, "Nixa");
    for title in ["one", "two"] {
        nixa_store::tasks::add_task(&conn, &TaskInput { title: title.into(), ..Default::default() }, &by).unwrap();
        let id = if title == "one" { 1 } else { 2 };
        nixa_store::tasks::assign_task(&conn, id, "terminal", if id == 1 { "m7" } else { "m8" }).unwrap();
        nixa_store::tasks::set_status(&conn, id, TaskStatus::Todo, None, false).unwrap();
    }
    let input = nixa_store::agents::AddAgent { title: "Developer".into(), purpose: "Normal fixes".into(), space: "nixa".into(), space_name: "nixa".into(), ..Default::default() };
    let agent = nixa_store::agents::add_agent(&conn, &input, "Nixa").unwrap();
    conn.execute(&format!("UPDATE custom_agents SET terminal = 'm7' WHERE id = '{}'", agent.id), []).unwrap();
    let one = super::launch::replace_gone(&conn, &nixa_store::tasks::get_task(&conn, 1).unwrap().unwrap(), "m7").unwrap();
    assert_eq!((one.status, one.assigned.as_str(), one.assigned_kind.as_str()), (TaskStatus::Todo, agent.id.as_str(), crate::tasks::KIND_CUSTOM));
    assert_eq!(seat_of(&one), Some(Seat::Custom(agent.id.clone())));
    let two = super::launch::replace_gone(&conn, &nixa_store::tasks::get_task(&conn, 2).unwrap().unwrap(), "m8").unwrap();
    assert_eq!((two.status, two.assigned.as_str()), (TaskStatus::Inbox, ""));
}

#[test]
fn dead_claims_in_a_locked_space_are_found() {
    let conn = nixa_store::open_memory().unwrap();
    let by = nixa_store::tasks::Actor::new(nixa_store::tasks::AuthorKind::Nixa, "Nixa");
    for (id, terminal) in [(1, "m3"), (2, "m4")] {
        nixa_store::tasks::add_task(&conn, &TaskInput { title: format!("Task {id}"), ..Default::default() }, &by).unwrap();
        nixa_store::tasks::set_status(&conn, id, TaskStatus::Todo, None, false).unwrap();
        assert!(nixa_store::tasks::claim_task(&conn, id, &nixa_store::tasks::Actor::in_terminal(nixa_store::tasks::AuthorKind::Agent, "dev", terminal), "").unwrap().ok);
    }
    let held = nixa_store::tasks::held_tasks(&conn).unwrap();
    let alive: std::collections::HashSet<String> = ["m4".to_string()].into_iter().collect();
    let dead = super::sweep::dead_locked_claims(&held, &alive);
    assert_eq!(dead.iter().map(|t| t.id).collect::<Vec<_>>(), vec![1]);
}
