use super::{Inner, Terminal, lock};
use crate::engine::Engine;
use nixa_protocol::{ChildReport, ErrorCode, Event, Reply, Status, TermId, paths::ENV_PARENT_ID};
use portable_pty::CommandBuilder;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const NOTES: usize = 64;
const NOTE_CHARS: usize = 800;
const RESULT_CHARS: usize = 8_000;
const ARCHIVES: usize = 16;
const ARCHIVE_AGE: Duration = Duration::from_secs(30 * 60);
const OUTPUT_LINES: usize = 60;
const OUTPUT_BYTES: usize = 8 * 1024;

#[derive(Default)]
pub(super) struct Inbox {
    notes: VecDeque<ChildReport>,
    archives: VecDeque<Archive>,
}

struct Archive {
    child: TermId,
    at: Instant,
    output: String,
}

impl Inbox {
    fn prune(&mut self, now: Instant) {
        self.archives
            .retain(|entry| now.duration_since(entry.at) < ARCHIVE_AGE);
    }

    fn save_output(&mut self, child: &TermId, output: String) {
        let at = Instant::now();
        self.prune(at);
        if self.archives.len() == ARCHIVES {
            self.archives.pop_front();
        }
        let start = output.ceil_char_boundary(output.len().saturating_sub(OUTPUT_BYTES));
        self.archives.push_back(Archive {
            child: child.clone(),
            at,
            output: output[start..].to_string(),
        });
    }
}

pub(super) fn configure_env(builder: &mut CommandBuilder, parent: Option<&TermId>) {
    if let Some(parent) = parent {
        builder.env(ENV_PARENT_ID, parent.as_str());
    } else {
        builder.env_remove(ENV_PARENT_ID);
    }
}

pub(super) fn startup(inner: &mut Inner) {
    if let Some(parent) = &inner.parent {
        let message = format!(
            "Nixa child of {parent}. Report your result with \"$NIXA_BIN\" term report \"TEXT\".\r\n"
        );
        super::reflow::feed(inner, message.as_bytes());
        inner.output.feed(message.as_bytes());
    }
}

pub fn list(parent: &Terminal, reader: Option<&TermId>) -> Vec<ChildReport> {
    let mut i = lock(parent);
    let shown: Vec<ChildReport> = i.child_reports.notes.iter().cloned().collect();
    if reader == Some(&parent.id) {
        for note in i.child_reports.notes.iter_mut().filter(|note| note.current_job_result) {
            note.read = true;
        }
    }
    shown
}

pub fn output(parent: &Terminal, child: &TermId) -> Reply {
    let mut i = lock(parent);
    i.child_reports.prune(Instant::now());
    match i
        .child_reports
        .archives
        .iter()
        .find(|entry| &entry.child == child)
    {
        Some(entry) => Reply::Text {
            id: child.clone(),
            text: entry.output.clone(),
        },
        None => Reply::not_found(child),
    }
}

pub fn last_exit(parent: &Terminal) -> Option<TermId> {
    let mut i = lock(parent);
    i.child_reports.prune(Instant::now());
    i.child_reports
        .archives
        .back()
        .map(|entry| entry.child.clone())
}

fn parent_of(engine: &Engine, child: &Inner) -> Option<std::sync::Arc<Terminal>> {
    if child.removed {
        return None;
    }
    child.parent.as_ref().and_then(|id| engine.get(id))
}

fn linked(parent: &Inner, child: &TermId) -> bool {
    !parent.removed && parent.children.contains(child)
}

fn record(parent: &Terminal, state: &mut Inner, report: ChildReport) -> Event {
    let event = Event::ChildNote {
        id: parent.id.clone(),
        child: report.child.clone(),
        status: report.status,
        exited: report.exited,
        note: report.note.clone(),
    };
    let notes = &mut state.child_reports.notes;
    if notes.len() == NOTES {
        let oldest = notes
            .iter()
            .position(|note| !note.current_job_result)
            .unwrap_or(0);
        notes.remove(oldest);
    }
    notes.push_back(report);
    event
}

fn clean_note(note: &str, limit: usize, multiline: bool) -> String {
    note.chars()
        .map(|c| {
            if c.is_control() && !(multiline && c == '\n') {
                ' '
            } else {
                c
            }
        })
        .take(limit)
        .collect()
}

pub fn submit(engine: &Engine, child: &Terminal, expected: Option<&TermId>, text: &str) -> Reply {
    if text.chars().take(RESULT_CHARS + 1).count() > RESULT_CHARS {
        return Reply::error(
            ErrorCode::BadRequest,
            format!("A report can contain up to {RESULT_CHARS} characters"),
        );
    }
    let text = clean_note(text, RESULT_CHARS, true);
    if text.trim().is_empty() {
        return Reply::error(ErrorCode::BadRequest, "A report needs text");
    }
    let i = lock(child);
    let Some(parent) =
        parent_of(engine, &i).filter(|parent| expected.is_none_or(|id| id == &parent.id))
    else {
        return Reply::error(
            ErrorCode::BadRequest,
            "This terminal has no matching parent",
        );
    };
    let mut state = lock(&parent);
    if !linked(&state, &child.id) {
        return Reply::error(
            ErrorCode::BadRequest,
            "This terminal has no matching parent",
        );
    }
    let event = record(
        &parent,
        &mut state,
        ChildReport {
            child: child.id.clone(),
            status: i.status,
            exited: false,
            note: format!("{}: {}", child.id, text.trim()),
            current_job_result: true, read: false,
        },
    );
    engine.emit(event);
    Reply::Ok
}

pub(super) fn status(
    engine: &Engine,
    child: &Terminal,
    i: &mut Inner,
    note: Option<&str>,
) -> Option<Event> {
    let status = i.status?;
    let starting = status == Status::Working && !i.job_active;
    let current_job_result =
        note.is_some() && (status == Status::Done || (status == Status::Idle && i.job_active));
    match status {
        Status::Working => i.job_active = true,
        Status::Idle | Status::Done | Status::Error => i.job_active = false,
        Status::Blocked => {}
    }
    if !starting && !current_job_result && !status.needs_attention() {
        return None;
    }
    let parent = parent_of(engine, i)?;
    let mut state = lock(&parent);
    if !linked(&state, &child.id) {
        return None;
    }
    if starting {
        for report in &mut state.child_reports.notes {
            if report.child == child.id {
                report.current_job_result = false;
            }
        }
        return None;
    }
    let mut text = format!("{} ({}) is {status}", child.id, i.name);
    if let Some(note) = note {
        text.push_str(": ");
        text.push_str(note);
    }
    Some(record(
        &parent,
        &mut state,
        ChildReport {
            child: child.id.clone(),
            status: Some(status),
            exited: false,
            note: clean_note(&text, NOTE_CHARS, false),
            current_job_result,
            read: false,
        },
    ))
}

pub(super) fn exit(
    engine: &Engine,
    child: &Terminal,
    i: &mut Inner,
    code: i32,
    respawning: bool,
) -> Option<(Event, bool)> {
    let parent = parent_of(engine, i)?;
    let mut state = lock(&parent);
    if !linked(&state, &child.id) {
        return None;
    }
    let event = record(
        &parent,
        &mut state,
        ChildReport {
            child: child.id.clone(),
            status: i.status,
            exited: true,
            note: clean_note(
                &format!("{} ({}) ended with code {code}", child.id, i.name),
                NOTE_CHARS,
                false,
            ),
            current_job_result: false, read: false,
        },
    );
    if respawning || !i.children.is_empty() || engine.take(&child.id).is_none() {
        return Some((event, false));
    }
    state
        .child_reports
        .save_output(&child.id, i.screen.tail_text(OUTPUT_LINES));
    state.children.retain(|id| id != &child.id);
    i.removed = true;
    Some((event, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::new_engine;
    use crate::handlers::dispatch;
    use crate::terminal::{
        NewTerm, clear_parent, link_parent, new_terminal, on_exit, on_output, set_status,
    };
    use nixa_protocol::{Request, WaitFor, WaitOutcome};
    use std::sync::Arc;

    fn fixture() -> (Arc<Engine>, Arc<Terminal>, Arc<Terminal>) {
        let engine = Arc::new(new_engine("/dev/null/engine.sock".into()));
        let term = |n, parent| {
            Arc::new(new_terminal(NewTerm {
                id: TermId::from_number(n),
                name: format!("terminal {n}"),
                parent,
                labels: Default::default(),
                cmd: Vec::new(),
                cwd: "/".into(),
                env: Vec::new(),
                rows: 24,
                cols: 80,
                keep: false,
            }))
        };
        let parent = term(1, None);
        let child = term(2, Some(parent.id.clone()));
        engine.insert(parent.clone());
        engine.insert(child.clone());
        let mut i = lock(&child);
        i.running = true;
        i.generation = 1;
        link_parent(&engine, &child.id, &mut i);
        drop(i);
        (engine, parent, child)
    }

    fn reports(engine: &Arc<Engine>, parent: &Terminal) -> Vec<ChildReport> {
        let Reply::ChildReports { reports } = dispatch(
            engine,
            Request::ChildReports {
                id: parent.id.clone(),
                reader: None,
            },
        ) else {
            panic!("missing parent inbox");
        };
        reports
    }

    #[test]
    fn child_report_startup_sets_the_parent_and_names_the_command() {
        let (_, parent, child) = fixture();
        let mut builder = CommandBuilder::new("sh");
        builder.env(ENV_PARENT_ID, "m9");
        configure_env(&mut builder, Some(&parent.id));
        assert_eq!(
            builder.get_env(ENV_PARENT_ID).and_then(|v| v.to_str()),
            Some("m1")
        );
        configure_env(&mut builder, None);
        assert!(builder.get_env(ENV_PARENT_ID).is_none());
        let mut i = lock(&child);
        startup(&mut i);
        assert!(i.screen.history_text().contains("term report"));
        assert!(
            i.output
                .since(0)
                .contains("\"$NIXA_BIN\" term report \"TEXT\"")
        );
    }

    #[test]
    fn child_report_uses_the_actual_parent_and_clears_at_the_next_job() {
        let (engine, parent, child) = fixture();
        let events = engine.subscribe(vec![parent.id.clone()], false);
        set_status(&engine, &child, Some(Status::Working), None);
        let post = |parent| {
            dispatch(
                &engine,
                Request::ChildReport {
                    id: child.id.clone(),
                    parent,
                    text: "Run passed\nVerified output".into(),
                },
            )
        };
        assert!(matches!(
            post(Some(TermId::from_number(9))),
            Reply::Error { .. }
        ));
        assert_eq!(post(Some(parent.id.clone())), Reply::Ok);
        assert!(
            matches!(events.try_recv(), Ok(Event::ChildNote { id, child: source, .. }) if id == parent.id && source == child.id)
        );
        let first = reports(&engine, &parent);
        assert_eq!(first.len(), 1);
        assert!(first[0].current_job_result);
        assert!(first[0].note.contains("Run passed\nVerified output"));
        assert_eq!(reports(&engine, &parent), first);
        assert!(lock(&parent).screen.history_text().is_empty());
        set_status(
            &engine,
            &child,
            Some(Status::Blocked),
            Some("waiting".into()),
        );
        set_status(&engine, &child, Some(Status::Working), None);
        assert!(reports(&engine, &parent)[0].current_job_result);
        set_status(&engine, &child, Some(Status::Idle), None);
        assert_eq!(reports(&engine, &parent).len(), 2);
        set_status(&engine, &child, Some(Status::Working), None);
        assert!(
            reports(&engine, &parent)
                .iter()
                .all(|report| !report.current_job_result)
        );
        clear_parent(&engine, &child);
        assert!(matches!(post(None), Reply::Error { .. }));
    }

    #[test]
    fn only_the_parent_reading_its_reports_marks_the_result_read() {
        let (engine, parent, child) = fixture();
        set_status(&engine, &child, Some(Status::Working), None);
        set_status(&engine, &child, Some(Status::Done), Some("tests pass".into()));
        let read_by = |reader: Option<TermId>| {
            let Reply::ChildReports { reports } = dispatch(
                &engine,
                Request::ChildReports { id: parent.id.clone(), reader },
            ) else {
                panic!("missing parent inbox");
            };
            reports
        };
        assert!(!reports(&engine, &parent)[0].read);
        read_by(Some(child.id.clone()));
        assert!(!reports(&engine, &parent)[0].read);
        read_by(Some(parent.id.clone()));
        let after = reports(&engine, &parent);
        assert!(after[0].read && after[0].current_job_result);
    }

    #[test]
    fn child_report_on_idle_requires_a_job_and_a_result_note() {
        let (engine, parent, child) = fixture();
        set_status(&engine, &child, Some(Status::Idle), Some("ready".into()));
        set_status(&engine, &child, Some(Status::Working), None);
        set_status(&engine, &child, Some(Status::Idle), None);
        assert!(reports(&engine, &parent).is_empty());
        set_status(&engine, &child, Some(Status::Working), None);
        set_status(
            &engine,
            &child,
            Some(Status::Idle),
            Some("verified result".into()),
        );
        let first = reports(&engine, &parent);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].status, Some(Status::Idle));
        assert!(first[0].current_job_result && first[0].note.contains("verified result"));
        set_status(
            &engine,
            &child,
            Some(Status::Idle),
            Some("verified result".into()),
        );
        assert_eq!(reports(&engine, &parent), first);
    }

    #[test]
    fn child_report_exit_saves_output_before_removal_and_can_still_be_waited_for() {
        let (engine, parent, child) = fixture();
        on_output(&engine, &child, b"verified last output\r\n");
        let events = engine.subscribe(Vec::new(), false);
        on_exit(&engine, &child, 1, 4);
        assert!(matches!(
            events.try_recv(),
            Ok(Event::Exit { code: Some(4), .. })
        ));
        assert!(matches!(
            events.try_recv(),
            Ok(Event::ChildNote { exited: true, .. })
        ));
        assert!(matches!(events.try_recv(), Ok(Event::Removed { id }) if id == child.id));
        assert!(engine.get(&child.id).is_none());
        assert!(lock(&parent).children.is_empty());
        assert_eq!(
            dispatch(
                &engine,
                Request::ChildOutput {
                    id: parent.id.clone(),
                    child: child.id.clone(),
                }
            ),
            Reply::Text {
                id: child.id.clone(),
                text: "verified last output".into()
            }
        );
        let waited = dispatch(
            &engine,
            Request::Wait {
                id: parent.id.clone(),
                until: WaitFor::ChildAny,
                timeout_ms: Some(0),
            },
        );
        assert!(
            matches!(waited, Reply::Waited { result } if result.outcome == WaitOutcome::Met
            && result.matched.as_deref() == Some(child.id.as_str()))
        );
    }

    #[test]
    fn child_report_keeps_detached_and_respawning_terminals() {
        let (engine, parent, child) = fixture();
        {
            let mut i = lock(&child);
            assert!(matches!(
                exit(&engine, &child, &mut i, 0, true),
                Some((_, false))
            ));
        }
        assert!(matches!(output(&parent, &child.id), Reply::Error { .. }));
        assert!(engine.get(&child.id).is_some());
        clear_parent(&engine, &child);
        on_exit(&engine, &child, 1, 0);
        assert!(engine.get(&child.id).is_some());
    }

    #[test]
    fn child_report_output_is_bounded_and_expires() {
        let mut inbox = Inbox::default();
        let text = "é".repeat(OUTPUT_BYTES);
        for n in 1..=ARCHIVES + 1 {
            inbox.save_output(&TermId::from_number(n as u64), text.clone());
        }
        assert_eq!(inbox.archives.len(), ARCHIVES);
        assert_eq!(
            inbox.archives.front().unwrap().child,
            TermId::from_number(2)
        );
        assert_eq!(inbox.archives.back().unwrap().output.len(), OUTPUT_BYTES);
        inbox.prune(Instant::now() + ARCHIVE_AGE);
        assert!(inbox.archives.is_empty());
    }
}
