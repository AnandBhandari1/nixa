use crate::app::{App, lock};
use crate::deep::SPACE_LABEL;
use crate::followup::close::{CloseState, keep_open};
use crate::privacy::taint::hide_secret_shapes;
use crate::tools::{Args, ToolResult};
use nixa_protocol::request::EndedSnapshot;
use nixa_protocol::{ReadSource, Reply, Request, Status, TermId, TermInfo};
use nixa_store::spaces;
use nixa_store::tasks::{self, TaskFilter, TaskStatus};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub(super) const NAME: &str = "nixa_term_close";
pub(super) const REMOVE: &str = "nixa_term_remove";
pub(super) const DESCRIPTION: &str = "Close one terminal and its tab by exact ID. Saves its final output and keeps task history. It first turns keep off, ends a program that still runs, ends the open job, clears open prompts and frees a task it holds, in its own space; a terminal in a space the user locked is never closed. A team run, debate or second opinion it is part of is stopped first. A deep brain seat is ended first (its seat is left and journaled) and then closed.";
pub const ARCHIVE: &str = "closed-terminal-reports";
const WINDOW_SPACE_LABEL: &str = "space";

fn own_space(conn: &nixa_store::Connection, term: &TermInfo) -> Option<String> {
    term.labels
        .get(SPACE_LABEL)
        .or_else(|| term.labels.get(WINDOW_SPACE_LABEL))
        .cloned()
        .or_else(|| crate::spaces::for_folder(&crate::spaces::from_store(conn), &term.cwd).map(|space| space.id.clone()))
}

fn locked_own(app: &App, term: &TermInfo) -> Result<(), String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| e.to_string())?;
    let Some(space) = own_space(&conn, term) else { return Ok(()) };
    if !spaces::get_space(&conn, &space).map_err(|e| e.to_string())?.is_some_and(|saved| saved.locked) {
        return Ok(());
    }
    let id = term.id.as_str();
    Err(format!("Terminal {id} is in space {space}, which the user locked. Nixa does not close terminals in a locked space. The user closes it, or opens the space first. Nothing was closed."))
}

fn busy_now(app: &App, id: &str) -> bool {
    let reviewing = {
        let opinions = lock(&app.extras.opinions);
        opinions.pending(id) || opinions.watched().iter().any(|(_, reviewer, _)| reviewer == id)
    };
    reviewing || lock(&app.extras.debates).owns(id)
}

fn leave_busy(app: &Arc<App>, id: &str) -> Vec<String> {
    let mut cleared = Vec::new();
    let runs: Vec<String> = lock(&app.teams.runs)
        .iter()
        .filter(|r| r.state == crate::teams::model::RunState::Running && r.steps.iter().any(|s| s.terminal == id))
        .map(|r| r.id.clone())
        .collect();
    for run in runs {
        crate::teams::runs::end_run(app, &run, crate::teams::model::RunState::Stopped, &format!("Stopped because Nixa closed {id}. The other tabs stay open."));
        cleared.push(format!("stopped team run {run}"));
    }
    {
        let mut debates = lock(&app.extras.debates);
        if debates.owns(id) && debates.stop().is_ok() {
            cleared.push("stopped its debate".to_string());
        }
    }
    let mut opinions = lock(&app.extras.opinions);
    let owners: Vec<String> = opinions.watched().into_iter().filter(|(_, reviewer, _)| reviewer == id).map(|(terminal, _, _)| terminal).collect();
    for owner in &owners {
        opinions.forget(owner);
    }
    let pending = opinions.pending(id);
    if pending {
        opinions.forget(id);
    }
    if pending || !owners.is_empty() {
        cleared.push("ended its second opinion".to_string());
    }
    cleared
}

fn held_tasks(conn: &nixa_store::Connection, id: &str) -> Result<Vec<tasks::Task>, String> {
    let filter = TaskFilter {
        terminal: id.into(),
        status: vec![TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review],
        ..TaskFilter::default()
    };
    tasks::list_tasks(conn, &filter).map_err(|e| e.to_string())
}

fn clear_holds(app: &App, id: &str) -> Result<Vec<String>, String> {
    let mut cleared = Vec::new();
    let followed = lock(&app.followup.jobs).remove(id).is_some();
    let watched = lock(&app.watch).jobs.remove(id).is_some();
    if followed || watched {
        cleared.push("ended its open job".to_string());
    }
    let prompts = lock(&app.prompts.book).by_terminal(id);
    if !prompts.is_empty() {
        let mut book = lock(&app.prompts.book);
        for prompt in &prompts {
            book.remove(&prompt.id);
        }
        cleared.push("cleared its open prompts".to_string());
    }
    let conn = nixa_store::open(&app.db_file).map_err(|e| e.to_string())?;
    for task in held_tasks(&conn, id)? {
        tasks::unclaim_task(&conn, task.id).map_err(|e| e.to_string())?;
        cleared.push(format!("freed task #{}", task.id));
    }
    Ok(cleared)
}

async fn clear_for_close(app: &Arc<App>, term: TermInfo) -> Result<TermInfo, String> {
    let id = term.id.clone();
    let mut cleared = clear_holds(app, id.as_str())?;
    if super::end_deep_seat(app, id.as_str()).is_some() {
        cleared.push("ended its deep brain seat".to_string());
    }
    cleared.extend(leave_busy(app, id.as_str()));
    let link = app.announce.link();
    let timeout = Duration::from_secs(crate::engine::CALL_SECS);
    if term.keep {
        link.call(Request::Keep { id: id.clone(), on: false }, timeout)
            .await
            .map_err(|e| format!("Could not turn keep off for {id}: {e}. Nothing was closed."))?;
        cleared.push("turned keep off".to_string());
    }
    if term.running {
        link.call(Request::Kill { id: id.clone() }, timeout)
            .await
            .map_err(|e| format!("Could not end the program in {id}: {e}. Nothing was closed."))?;
        cleared.push("ended its program".to_string());
    }
    let mut fresh = term;
    if fresh.keep || fresh.running {
        fresh = link.get(id.as_str()).await.map_err(|e| e.to_string())?;
        for _ in 0..END_POLLS {
            if !fresh.running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(END_POLL_MS)).await;
            fresh = link.get(id.as_str()).await.map_err(|e| e.to_string())?;
        }
    }
    if fresh.running {
        link.call(Request::Kill { id: id.clone() }, timeout)
            .await
            .map_err(|e| format!("Could not end the program in {id}: {e}. {}", still_ending(id.as_str())))?;
        for _ in 0..END_POLLS * 4 {
            tokio::time::sleep(Duration::from_millis(END_POLL_MS)).await;
            fresh = link.get(id.as_str()).await.map_err(|e| e.to_string())?;
            if !fresh.running {
                break;
            }
        }
        if fresh.running {
            return Err(still_ending(id.as_str()));
        }
    }
    if !cleared.is_empty() {
        let what = format!("made {id} ready to close: {}", cleared.join(", "));
        crate::modes::note(app, "auto-yes", &what, Some(id.as_str()), Some("Nixa closes instead of refusing"));
    }
    Ok(fresh)
}

pub fn still_ending(id: &str) -> String {
    format!("The program in {id} is still ending. This is not a refusal: wait a few seconds with nixa_term_wait until exit, then call nixa_term_close again.")
}

const END_POLLS: usize = 10;
const END_POLL_MS: u64 = 200;

fn check_terminal(app: &App, term: &TermInfo) -> Result<i32, String> {
    let id = term.id.as_str();
    let conn = nixa_store::open(&app.db_file).map_err(|e| e.to_string())?;
    let filter = TaskFilter {
        terminal: id.into(),
        status: vec![TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review],
        limit: Some(1),
        ..TaskFilter::default()
    };
    let task = tasks::list_tasks(&conn, &filter).map_err(|e| e.to_string())?;
    let state = CloseState {
        live: true,
        deep: lock(&app.deep).owns(id),
        job: lock(&app.followup.jobs).contains_key(id) || lock(&app.watch).jobs.contains_key(id),
        prompts: lock(&app.prompts.book).by_terminal(id).len(),
        busy: crate::teams::owns(&app.teams, id) || busy_now(app, id),
        working: term.status == Some(Status::Working) && term.running,
        task: task.first().map(|task| format!("#{}", task.id)),
        ..CloseState::default()
    };
    if let Some(reason) = keep_open(&state) {
        return Err(format!("{id} stays open because {reason}."));
    }
    if term.keep {
        return Err(format!("{id} is set to keep open. Nothing was closed."));
    }
    term.exit_code
        .filter(|_| !term.running)
        .ok_or_else(|| format!("{id} has not ended. Nothing was closed."))
}

fn check_mode(app: &Arc<App>, id: &TermId) -> Result<(), String> {
    match super::work_hold(app, "close-job", &format!("close terminal {id}"), Some(id.as_str())) {
        Some(held) => Err(held),
        None => Ok(()),
    }
}

fn save_report(dir: &Path, filename: &str, report: &Value) -> anyhow::Result<()> {
    let dir = dir.join(ARCHIVE);
    nixa_protocol::paths::make_private_dirs(&dir)?;
    let file = tempfile::NamedTempFile::new_in(&dir)?;
    serde_json::to_writer_pretty(file.as_file(), report)?;
    file.as_file().sync_all()?;
    file.persist(dir.join(filename))?;
    Ok(())
}

pub(super) async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let raw = args
        .get("terminal")
        .and_then(Value::as_str)
        .ok_or("terminal is needed.")?;
    let id = raw.parse::<TermId>().map_err(|e| e.to_string())?;
    let link = app.announce.link();
    let term = link.get(id.as_str()).await.map_err(|e| e.to_string())?;
    if term.id != id {
        return Err("The engine returned a different terminal ID. Nothing was closed.".into());
    }
    locked_own(app, &term)?;
    let term = clear_for_close(app, term).await?;
    let exit_code = check_terminal(app, &term)?;
    check_mode(app, &id)?;

    let timeout = Duration::from_secs(crate::engine::CALL_SECS);
    let read = Request::Read {
        id: id.clone(),
        source: ReadSource::History,
        lines: Some(super::READ_LINES),
    };
    let reply = link
        .call(read, timeout)
        .await
        .map_err(|e| format!("Could not read the final report: {e}. Nothing was closed."))?;
    let Reply::Text { id: read_id, text } = reply else {
        return Err("Could not read the final report. Nothing was closed.".into());
    };
    if read_id != id {
        return Err(
            "The engine returned output for a different terminal. Nothing was closed.".into(),
        );
    }
    let redacted = hide_secret_shapes(text.trim_end());
    let tail = match redacted.char_indices().rev().nth(super::READ_CHARS - 1) {
        Some((start, _)) if start > 0 => format!("[Earlier output cut]\n{}", &redacted[start..]),
        _ => redacted,
    };
    let filename = format!("{id}.json");
    let report = json!({
        "terminal": id, "status": term.status, "exit_code": exit_code,
        "status_note": term.status_note.as_deref().map(hide_secret_shapes), "output_tail": tail,
    });
    save_report(&app.dir, &filename, &report)
        .map_err(|e| format!("Could not save the final report: {e}. Nothing was closed."))?;
    check_terminal(app, &term)?;
    check_mode(app, &id)?;

    let expected = EndedSnapshot {
        space_label: term.labels.get(SPACE_LABEL).cloned(),
        window_space_label: term.labels.get(WINDOW_SPACE_LABEL).cloned(),
        cwd: term.cwd,
        seq: term.seq,
        exit_code,
    };
    match link
        .call(
            Request::RemoveEnded {
                id: id.clone(),
                expected,
            },
            timeout,
        )
        .await
    {
        Ok(Reply::Ok) => {
            crate::modes::note(app, "close", &format!("closed terminal {id}"), Some(id.as_str()), None);
            Ok(format!("Closed ended terminal {id} (exit {exit_code}). Final report saved in the Nixa data folder at {ARCHIVE}/{filename}. Task history was not changed."))
        }
        Ok(_) => Err(format!("The engine did not confirm closure of {id}.")),
        Err(e) => Err(format!("Could not confirm closure of {id}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deep::{ROLE_LABEL, ROLE_VALUE};
    use crate::turns::ToolCall;
    use nixa_protocol::wire::{read_request, write_reply};
    use nixa_store::tasks::{self, Actor, AuthorKind, TaskInput, TaskStatus};
    use std::io::BufReader;
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    use std::sync::Arc;

    fn setup() -> (tempfile::TempDir, Arc<App>, UnixListener, TermInfo) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let socket = dir.path().join("engine.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        app.announce.set_socket(Some(socket));
        let conn = nixa_store::open(&app.db_file).unwrap();
        let patch = spaces::SpacePatch {
            folder: Some(dir.path().display().to_string()),
            ..Default::default()
        };
        spaces::set_space(&conn, "project", &patch).unwrap();
        spaces::set_active_space(&conn, Some("project")).unwrap();
        let term = serde_json::from_value(json!({
            "id": "project-agent-a1b2c3", "name": "agent", "rows": 24, "cols": 80,
            "cwd": dir.path(), "running": false, "created_ms": 0, "exit_code": 0, "seq": 12,
            "status": "done", "status_note": "Checks passed", "labels": {"nixa.space": "project"},
        }))
        .unwrap();
        (dir, app, listener, term)
    }

    fn exchange(listener: &UnixListener, req: Request, reply: Reply) {
        let (mut stream, _) = listener.accept().unwrap();
        let received = read_request(&mut BufReader::new(&stream)).unwrap();
        assert_eq!(received, Some(req));
        write_reply(&mut stream, &reply).unwrap();
    }

    #[tokio::test]
    async fn close_is_in_both_chat_surfaces_and_archives_before_remove() {
        let (dir, app, listener, term) = setup();
        let id = term.id.clone();
        let args = json!({"terminal": id});
        let spec = crate::tools::specs(&app)
            .into_iter()
            .find(|spec| spec.name == NAME)
            .unwrap();
        let cli = crate::cli_brains::door::tool_list(&app);
        for name in [NAME, REMOVE] {
            assert_eq!(
                cli.as_array()
                    .unwrap()
                    .iter()
                    .find(|tool| tool["name"] == name)
                    .unwrap()["inputSchema"],
                spec.schema
            );
        }
        let conn = nixa_store::open(&app.db_file).unwrap();
        let creator = Actor::new(AuthorKind::Nixa, "Nixa");
        let input = TaskInput {
            title: "Retain the task results".into(),
            status: Some(TaskStatus::Todo),
            ..Default::default()
        };
        let task = tasks::add_task(&conn, &input, &creator).unwrap();
        assert!(!dir.path().join(ARCHIVE).exists());

        let user = Actor::new(AuthorKind::User, "user");
        tasks::done_task(&conn, task.id, "Accepted", &user).unwrap();
        let before = serde_json::to_value(tasks::show_task(&conn, task.id).unwrap()).unwrap();
        let path = dir.path().join(ARCHIVE).join(format!("{id}.json"));
        let server = std::thread::spawn(move || {
            let id = term.id.clone();
            let cwd = term.cwd.clone();
            exchange(
                &listener,
                Request::Get { id: id.clone() },
                Reply::Term { term },
            );
            exchange(
                &listener,
                Request::Read {
                    id: id.clone(),
                    source: ReadSource::History,
                    lines: Some(60),
                },
                Reply::Text {
                    id: id.clone(),
                    text: format!("Final report\nghp_{}", "a".repeat(22)),
                },
            );
            let (mut stream, _) = listener.accept().unwrap();
            assert_eq!(
                read_request(&mut BufReader::new(&stream)).unwrap(),
                Some(Request::RemoveEnded {
                    id,
                    expected: EndedSnapshot {
                        space_label: Some("project".into()),
                        window_space_label: None,
                        cwd,
                        seq: 12,
                        exit_code: 0
                    },
                })
            );
            assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
            let report: Value = crate::util::read_json(&path).unwrap();
            assert_eq!(report["status_note"], "Checks passed");
            assert!(
                report["output_tail"]
                    .as_str()
                    .unwrap()
                    .starts_with("Final report\n")
            );
            assert!(!report["output_tail"].as_str().unwrap().contains("ghp_"));
            write_reply(&mut stream, &Reply::Ok).unwrap();
        });
        let (out, error) = crate::cli_brains::door::run_tool(&app, REMOVE, &args).await;
        assert!(
            !error && out.contains(&format!("Closed ended terminal {id}")),
            "{out}"
        );
        server.join().unwrap();
        assert_eq!(
            serde_json::to_value(tasks::show_task(&conn, task.id).unwrap()).unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn close_rejects_non_ids_before_engine_lookup() {
        let (_dir, app, _listener, _) = setup();
        for raw in [
            "agent",
            "1",
            "project-agent",
            " project-agent-a1b2c3",
            "PROJECT-AGENT-A1B2C3",
        ] {
            let call = ToolCall {
                id: "test".into(),
                name: NAME.into(),
                args: crate::turns::args_of(&json!({"terminal": raw})),
            };
            let (out, error) = crate::tools::call(&app, &call).await;
            assert!(error && out.contains("bad terminal id"), "{out}");
        }
    }

    #[tokio::test]
    async fn automatic_close_ends_a_kept_live_terminal_in_its_own_space() {
        let (dir, app, listener, mut term) = setup();
        term.labels.insert(SPACE_LABEL.into(), "other".into());
        term.running = true;
        term.keep = true;
        term.exit_code = None;
        let id = term.id.clone();
        let args = json!({"terminal": id});
        let server = std::thread::spawn(move || {
            let id = term.id.clone();
            let cwd = term.cwd.clone();
            let mut ended = term.clone();
            ended.running = false;
            ended.keep = false;
            ended.exit_code = Some(0);
            exchange(&listener, Request::Get { id: id.clone() }, Reply::Term { term });
            exchange(&listener, Request::Keep { id: id.clone(), on: false }, Reply::Ok);
            exchange(&listener, Request::Kill { id: id.clone() }, Reply::Ok);
            exchange(&listener, Request::Get { id: id.clone() }, Reply::Term { term: ended });
            exchange(
                &listener,
                Request::Read { id: id.clone(), source: ReadSource::History, lines: Some(super::super::READ_LINES) },
                Reply::Text { id: id.clone(), text: "Final report".into() },
            );
            exchange(
                &listener,
                Request::RemoveEnded {
                    id,
                    expected: EndedSnapshot { space_label: Some("other".into()), window_space_label: None, cwd, seq: 12, exit_code: 0 },
                },
                Reply::Ok,
            );
        });
        let out = call(&app, args.as_object().unwrap()).await.unwrap();
        assert!(out.contains("Closed ended terminal"), "{out}");
        server.join().unwrap();
        assert!(dir.path().join(ARCHIVE).join(format!("{}.json", args["terminal"].as_str().unwrap())).exists());
    }

    #[tokio::test]
    async fn automatic_close_ends_a_deep_brain_seat_then_closes_it() {
        let (dir, app, listener, mut term) = setup();
        term.labels.insert(ROLE_LABEL.into(), ROLE_VALUE.into());
        let space = crate::spaces::Space { id: "project".into(), name: "project".into(), folder: dir.path().display().to_string() };
        lock(&app.deep).join(term.id.as_str(), "Claude", &space, Some("claude"), 0).unwrap();
        assert!(lock(&app.deep).owns(term.id.as_str()));
        let args = json!({"terminal": term.id});
        let server = std::thread::spawn(move || {
            let id = term.id.clone();
            let cwd = term.cwd.clone();
            exchange(&listener, Request::Get { id: id.clone() }, Reply::Term { term });
            exchange(
                &listener,
                Request::Read { id: id.clone(), source: ReadSource::History, lines: Some(super::super::READ_LINES) },
                Reply::Text { id: id.clone(), text: "Final report".into() },
            );
            exchange(
                &listener,
                Request::RemoveEnded {
                    id,
                    expected: EndedSnapshot { space_label: Some("project".into()), window_space_label: None, cwd, seq: 12, exit_code: 0 },
                },
                Reply::Ok,
            );
        });
        let out = call(&app, args.as_object().unwrap()).await.unwrap();
        assert!(out.contains("Closed ended terminal"), "{out}");
        server.join().unwrap();
        assert!(!lock(&app.deep).owns(args["terminal"].as_str().unwrap()));
        let conn = crate::modes::store(&app).unwrap();
        let rows = crate::modes::journal::last_for(&conn, None, 20).unwrap();
        assert!(rows.iter().any(|row| row.choice.contains("ended deep brain seat")), "{rows:?}");
    }

    #[tokio::test]
    async fn a_terminal_in_a_locked_own_space_is_not_closed() {
        let (dir, app, listener, term) = setup();
        let conn = nixa_store::open(&app.db_file).unwrap();
        spaces::set_space(&conn, "project", &spaces::SpacePatch { locked: Some(true), ..Default::default() }).unwrap();
        let args = json!({"terminal": term.id});
        let server = std::thread::spawn(move || {
            let id = term.id.clone();
            exchange(&listener, Request::Get { id }, Reply::Term { term });
        });
        let refused = call(&app, args.as_object().unwrap()).await.unwrap_err();
        assert!(refused.contains("locked") && refused.contains("Nothing was closed"), "{refused}");
        server.join().unwrap();
        assert!(!dir.path().join(ARCHIVE).exists());
    }

    #[test]
    fn a_stale_reviewer_does_not_keep_a_terminal_open() {
        let (_dir, app, _listener, term) = setup();
        let id = term.id.as_str();
        {
            let mut opinions = lock(&app.extras.opinions);
            assert!(opinions.begin("owner-term", 0));
            assert!(opinions.started("owner-term", id, 0, 1000));
        }
        assert!(busy_now(&app, id));
        lock(&app.extras.opinions).expire(2000);
        assert!(lock(&app.extras.opinions).owns(id));
        assert!(!busy_now(&app, id));
    }

    #[tokio::test]
    async fn close_never_removes_if_the_report_cannot_be_saved() {
        let (dir, app, listener, mut term) = setup();
        term.labels.clear();
        term.labels
            .insert(WINDOW_SPACE_LABEL.into(), "project".into());
        std::fs::write(dir.path().join(ARCHIVE), "not a directory").unwrap();
        let id = term.id.clone();
        let args = json!({"terminal": id});
        let server = std::thread::spawn(move || {
            exchange(
                &listener,
                Request::Get { id: id.clone() },
                Reply::Term { term },
            );
            exchange(
                &listener,
                Request::Read {
                    id: id.clone(),
                    source: ReadSource::History,
                    lines: Some(super::super::READ_LINES),
                },
                Reply::Text {
                    id,
                    text: "Final report".into(),
                },
            );
            listener
        });
        let out = call(&app, args.as_object().unwrap()).await.unwrap_err();
        assert!(out.contains("Could not save the final report"), "{out}");
        let listener = server.join().unwrap();
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
