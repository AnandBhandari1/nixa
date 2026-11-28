use crate::engine::Engine;
use crate::terminal::{self, NewTerm, Terminal, lock};
use crate::wait;
use crate::writer::{self, Done};
use nixa_protocol::paths::term_env;
use nixa_protocol::request::EndedSnapshot;
use nixa_protocol::{
    CreateSpec, DEFAULT_COLS, DEFAULT_ROWS, DEFAULT_SEND_TIMEOUT_MS, ErrorCode, Event, InterruptReport, Key,
    PROTOCOL_VERSION, ReadSource, Reply, Request, Status, TermId,
};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PASTE_ECHO_MAX: Duration = Duration::from_millis(250);
const PASTE_NO_ECHO: Duration = Duration::from_millis(60);
const PASTE_SETTLE: Duration = Duration::from_millis(45);
const PASTE_TOTAL_MAX: Duration = Duration::from_millis(350);
const SUBMIT_CONFIRM_SETTLE: Duration = Duration::from_millis(35);
const SUBMIT_RETRY_WAITS: [Duration; 3] = [
    Duration::from_millis(180),
    Duration::from_millis(280),
    Duration::from_millis(380),
];
const START_WATCH: Duration = Duration::from_millis(1200);
const INTERRUPT_WATCH: Duration = Duration::from_millis(1200);
const TAIL_LINES: usize = 10;
pub(crate) const PROMPT_TAIL_LINES: usize = 10;

fn bad(msg: impl Into<String>) -> Reply {
    Reply::error(ErrorCode::BadRequest, msg)
}

fn exited(id: &TermId) -> Reply {
    Reply::error(ErrorCode::Exited, format!("the program in {id} has exited"))
}

fn internal(e: impl std::fmt::Display) -> Reply {
    Reply::error(ErrorCode::Internal, e.to_string())
}

fn cannot_write(id: &TermId, e: std::io::Error) -> Reply {
    internal(format!("cannot write to {id}: {e}"))
}

fn write_failed(term: &Terminal, done: Done) -> Option<Reply> {
    writer::finish(done).err().map(|e| cannot_write(&term.id, e))
}

pub fn dispatch(engine: &Arc<Engine>, req: Request) -> Reply {
    match req {
        Request::Ping => Reply::Pong {
            version: PROTOCOL_VERSION,
            pid: std::process::id(),
        },
        Request::List { parent, label } => list(engine, parent, label),
        Request::Procs => crate::procs::list(engine),
        Request::Create { spec } => create(engine, spec),
        Request::Subscribe { .. } | Request::Shutdown => bad("this request is handled by the connection"),
        other => {
            let Some(id) = other.term_id().cloned() else {
                return bad("missing terminal id");
            };
            match engine.get(&id) {
                Some(term) => on_term(engine, &term, other),
                None => Reply::not_found(&id),
            }
        }
    }
}

fn on_term(engine: &Arc<Engine>, term: &Arc<Terminal>, req: Request) -> Reply {
    match req {
        Request::Get { .. } => Reply::Term {
            term: terminal::info(term),
        },
        Request::ChildReports { reader, .. } => Reply::ChildReports {
            reports: terminal::reports::list(term, reader.as_ref()),
        },
        Request::ChildReport { parent, text, .. } => {
            terminal::reports::submit(engine, term, parent.as_ref(), &text)
        }
        Request::ChildOutput { child, .. } => terminal::reports::output(term, &child),
        Request::ClearParent { .. } => {
            terminal::clear_parent(engine, term);
            Reply::Ok
        }
        Request::Kill { .. } => {
            terminal::kill(engine, term);
            Reply::Ok
        }
        Request::Remove { .. } => remove(engine, term, None),
        Request::RemoveEnded { expected, .. } => remove(engine, term, Some(&expected)),
        Request::Read { source, lines, .. } => read(term, source, lines),
        Request::Snapshot { scroll, since_seq, .. } => snapshot(term, scroll, since_seq),
        Request::Send {
            text,
            submit,
            timeout_ms,
            force,
            ..
        } => send(engine, term, &text, submit, timeout_ms, force),
        Request::Input { text, .. } => input(engine, term, &text),
        Request::Keys { keys, .. } => press(engine, term, &keys),
        Request::Interrupt { .. } => interrupt(engine, term),
        Request::Resize { rows, cols, .. } => resize(engine, term, rows, cols),
        Request::LabelSet { key, value, .. } => label_set(engine, term, key, value),
        Request::Rename { name, .. } => rename(engine, term, name),
        Request::StatusSet { status, note, .. } => {
            terminal::set_status(engine, term, status, note);
            Reply::Ok
        }
        Request::Wait { until, timeout_ms, .. } => wait::run(engine, term, until, timeout_ms),
        Request::Keep { on, .. } => keep(engine, term, on),
        _ => bad("unexpected request"),
    }
}

fn label_matches(labels: &BTreeMap<String, String>, filter: &str) -> bool {
    match filter.split_once('=') {
        Some((k, v)) => labels.get(k).is_some_and(|have| have == v),
        None => labels.contains_key(filter),
    }
}

fn list(engine: &Engine, parent: Option<TermId>, label: Option<String>) -> Reply {
    let terms = engine
        .terms()
        .iter()
        .map(|t| terminal::info(t))
        .filter(|t| parent.is_none() || t.parent == parent)
        .filter(|t| label.as_deref().is_none_or(|l| label_matches(&t.labels, l)))
        .collect();
    Reply::Terms { terms }
}

fn home() -> String {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "/".to_string())
}

fn default_name(cmd: &[String]) -> String {
    cmd.first()
        .and_then(|c| Path::new(c).file_name())
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "shell".to_string())
}

fn create(engine: &Arc<Engine>, spec: CreateSpec) -> Reply {
    create_as(engine, spec, None)
}

pub fn create_as(engine: &Arc<Engine>, mut spec: CreateSpec, fixed: Option<TermId>) -> Reply {
    use crate::terminal_ids::{agent_for, space_for};
    use nixa_protocol::id::SPACE_SLUG_LABEL;
    let cwd = spec.cwd.filter(|c| !c.is_empty()).unwrap_or_else(home);
    if !Path::new(&cwd).is_dir() {
        return bad(format!("cwd {cwd} is not a folder"));
    }
    if spec.cmd.first().is_some_and(|c| c.is_empty()) {
        return bad("empty program name");
    }
    let rows = spec.rows.unwrap_or(DEFAULT_ROWS).max(1);
    let cols = spec.cols.unwrap_or(DEFAULT_COLS).max(2);
    let parent = spec.parent.and_then(|id| engine.get(&id));
    let inherited = if let Some(parent) = &parent {
        let state = lock(parent);
        if let Err(error) = nixa_protocol::term::check_start_parent(&parent.id, state.parent.as_ref()) {
            return bad(error);
        }
        state.labels.get(SPACE_SLUG_LABEL).cloned()
    } else {
        None
    };
    let space = space_for(spec.labels.get(SPACE_SLUG_LABEL).map(String::as_str), inherited.as_deref(), Path::new(&cwd), Path::new(&home()));
    let agent = agent_for(&default_name(&spec.cmd));
    let id = match fixed.map(Ok).unwrap_or_else(|| engine.next_id(&space, &agent)) {
        Ok(id) => id,
        Err(e) => return internal(format!("cannot reserve a terminal id: {e}")),
    };
    spec.labels.insert(SPACE_SLUG_LABEL.into(), space);
    let mut env = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];
    env.extend(spec.env);
    env.extend(term_env(&id, &engine.socket));
    tool_path(&mut env);
    gate_path(&mut env);
    heavy_path(&mut env);
    let term = Arc::new(terminal::new_terminal(NewTerm {
        id: id.clone(),
        name: spec.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| default_name(&spec.cmd)),
        labels: spec.labels,
        parent: parent.map(|term| term.id.clone()),
        cmd: spec.cmd.clone(),
        cwd,
        env,
        rows,
        cols,
        keep: spec.keep,
    }));
    engine.insert(term.clone());
    if let Err(e) = terminal::spawn(engine, &term, &spec.cmd) {
        engine.take(&id);
        return bad(format!("cannot start {:?}: {e:#}", spec.cmd));
    }
    Reply::Term { term: terminal::info(&term) }
}

fn remove(engine: &Engine, term: &Arc<Terminal>, expected: Option<&EndedSnapshot>) -> Reply {
    let (parent, children) = {
        let mut i = lock(term);
        if let Some(expected) = expected {
            if i.labels.get("nixa.role").is_some_and(|role| role == "deep-brain") {
                return bad(format!("{} is a deep brain terminal. Nothing was closed. End its deep session first (leave or stop the seat through the deep service), then close it again, or close it with remove, which ends the program and then closes it.", term.id));
            }
            if i.status == Some(Status::Working) {
                return bad(format!("{} is still working. Nothing was closed. End it first with remove, which stops the program and then closes it.", term.id));
            }
            if i.running || i.exit_code.is_none() {
                return bad(format!("{} has not ended. Nothing was closed. End it first with remove, which stops the program and then closes it.", term.id));
            }
            if i.keep {
                return bad(format!("{} is set to keep open. Nothing was closed. Clear keep with the keep request and close it again, or close it with remove, which ends the program and then closes it.", term.id));
            }
            if i.labels.get("nixa.space") != expected.space_label.as_ref()
                || i.labels.get("space") != expected.window_space_label.as_ref()
                || i.cwd != expected.cwd
                || i.seq != expected.seq
                || i.exit_code != Some(expected.exit_code)
            {
                return bad(format!("{} changed while closing. Nothing was closed.", term.id));
            }
        }
        if engine.take(&term.id).is_none() {
            return Reply::not_found(&term.id);
        }
        i.removed = true;
        (i.parent.clone(), i.children.clone())
    };
    if expected.is_none() {
        terminal::kill(engine, term);
    }
    if let Some(p) = parent.and_then(|p| engine.get(&p)) {
        lock(&p).children.retain(|c| *c != term.id);
    }
    for c in children.iter().filter_map(|c| engine.get(c)) {
        lock(&c).parent = None;
    }
    term.bump();
    engine.emit(Event::Removed { id: term.id.clone() });
    Reply::Ok
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn read(term: &Terminal, source: ReadSource, lines: Option<usize>) -> Reply {
    let text = {
        let i = lock(term);
        match source {
            ReadSource::Screen => i.screen.screen_text(),
            ReadSource::History => i.screen.history_text(),
        }
    };
    let text = match lines {
        Some(n) => last_lines(&text, n),
        None => text,
    };
    Reply::Text {
        id: term.id.clone(),
        text,
    }
}

fn snapshot(term: &Terminal, scroll: usize, since_seq: Option<u64>) -> Reply {
    let i = lock(term);
    if since_seq == Some(i.seq) {
        return Reply::Unchanged {
            id: term.id.clone(),
            seq: i.seq,
        };
    }
    Reply::Snapshot {
        snapshot: i.screen.snapshot(&term.id, i.seq, scroll, i.title.clone()),
    }
}

fn clean_paste_text(text: &str, bracketed: bool) -> String {
    let text = text.replace("\r\n", "\n");
    text
        .chars()
        .filter(|c| {
            let control = c.is_control();
            !control || (bracketed && matches!(c, '\t' | '\n'))
        })
        .collect()
}

fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let clean = clean_paste_text(text, bracketed);
    if bracketed && !clean.is_empty() {
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        clean.into_bytes()
    }
}

pub(crate) fn enter_bytes(report_all_keys: bool) -> &'static [u8] {
    if report_all_keys { b"\x1b[13u" } else { b"\r" }
}

fn prompt_needle(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    let needle: String = line.chars().take(32).collect();
    (needle.chars().count() >= 3).then_some(needle)
}

fn non_empty_tail_lines(text: &str, max: usize) -> Vec<&str> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(max)..].to_vec()
}

fn is_empty_prompt_line(line: &str) -> bool {
    let t = line.trim();
    matches!(t, ">" | "❯" | "›" | "╰─" | "╰─>" | "$" | "%" | "#")
}

pub(crate) fn still_in_prompt(tail_after_text: &str, tail_now: &str, needle: Option<&str>) -> bool {
    if tail_now == tail_after_text {
        return true;
    }
    let Some(needle) = needle else {
        return false;
    };
    let before = non_empty_tail_lines(tail_after_text, 6);
    let after = non_empty_tail_lines(tail_now, 6);
    let Some(before_pos) = before.iter().rposition(|l| l.contains(needle)) else {
        return false;
    };
    let Some(after_pos) = after.iter().rposition(|l| l.contains(needle)) else {
        return false;
    };
    // If an empty prompt box appeared below the line with our text, the message was submitted and scrolled up.
    if after[after_pos + 1..].iter().any(|l| is_empty_prompt_line(l)) {
        return false;
    }
    // If the prompt line itself is still at the same bottom offset (e.g., only a status line ticked), it is still stuck.
    let before_from_bottom = before.len() - before_pos;
    let after_from_bottom = after.len() - after_pos;
    after_from_bottom <= before_from_bottom && after[after_pos].trim() == before[before_pos].trim()
}

fn wait_for_paste_settle(term: &Terminal, generation: u64, seq_before: u64, interactive: bool) -> u64 {
    let started = Instant::now();
    let total_deadline = started + PASTE_TOTAL_MAX;
    let first_echo_deadline = started + if interactive { PASTE_ECHO_MAX } else { PASTE_NO_ECHO };
    let mut last_seq = seq_before;
    let mut last_change = None;
    loop {
        let seen = term.tick();
        let now = Instant::now();
        let (running, same_gen, seq, draft) = {
            let i = lock(term);
            (i.running, i.generation == generation, i.seq, i.draft.active())
        };
        if !running || !same_gen || draft {
            return seq;
        }
        if seq != last_seq {
            last_seq = seq;
            last_change = Some(now);
        }
        if now >= total_deadline {
            return last_seq;
        }
        let next_deadline = match last_change {
            Some(at) => {
                let settled = at + PASTE_SETTLE;
                if now >= settled {
                    return last_seq;
                }
                settled.min(total_deadline)
            }
            None => {
                if now >= first_echo_deadline {
                    return last_seq;
                }
                first_echo_deadline.min(total_deadline)
            }
        };
        term.wait_change(seen, next_deadline);
    }
}

fn wait_enter_confirmed(
    term: &Terminal,
    generation: u64,
    status_before: Option<Status>,
    seq_before_enter: u64,
    tail_after_text: &str,
    needle: Option<&str>,
    wait: Duration,
) -> bool {
    let deadline = Instant::now() + wait;
    let mut changed_at = None;
    loop {
        let seen = term.tick();
        let now = Instant::now();
        let (running, same_gen, status, seq, tail_now) = {
            let i = lock(term);
            (
                i.running,
                i.generation == generation,
                i.status,
                i.seq,
                if i.seq > seq_before_enter {
                    Some(i.screen.tail_text(PROMPT_TAIL_LINES))
                } else {
                    None
                },
            )
        };
        if !running || !same_gen {
            return true;
        }
        if matches!(status, Some(Status::Working) | Some(Status::Blocked)) && status != status_before {
            return true;
        }
        if let Some(tail_now) = tail_now
            && !still_in_prompt(tail_after_text, &tail_now, needle)
        {
            let at = *changed_at.get_or_insert(now);
            let settle = at + SUBMIT_CONFIRM_SETTLE;
            if now >= settle {
                return true;
            }
            term.wait_change(seen, settle.min(deadline));
            continue;
        }
        if now >= deadline {
            if seq == seq_before_enter {
                return false;
            }
            let tail_now = lock(term).screen.tail_text(PROMPT_TAIL_LINES);
            return !still_in_prompt(tail_after_text, &tail_now, needle);
        }
        term.wait_change(seen, deadline);
    }
}

fn draft_event(engine: &Engine, term: &Terminal, before: bool, after: bool) {
    if before != after {
        term.bump();
        engine.emit(Event::Draft {
            id: term.id.clone(),
            active: after,
        });
    }
}

fn send(
    engine: &Engine,
    term: &Terminal,
    text: &str,
    submit: bool,
    timeout_ms: Option<u64>,
    force: bool,
) -> Reply {
    let began = Instant::now();
    let limit = timeout_ms.unwrap_or(DEFAULT_SEND_TIMEOUT_MS);
    let deadline = wait::deadline_after(began, limit);
    let mut stale_cleared = false;
    let (forced, done, generation, status_before, submitted_inline, cleared_draft, interactive, seq_before_write, tail_before_write) = loop {
        let seen = term.tick();
        let wake = {
            let mut i = lock(term);
            if !i.running {
                return exited(&term.id);
            }
            if !force && i.draft.active() && i.draft.stale(&i.screen.tail_text(PROMPT_TAIL_LINES), Instant::now()) {
                i.draft.clear();
                stale_cleared = true;
            }
            let busy = i.draft.active();
            if !busy || force {
                i.pending_submit = None;
                let bracketed = i.screen.bracketed_paste();
                let alt_screen = i.screen.alt_screen();
                let focus_events = i.screen.focus_events();
                let interactive = bracketed || alt_screen;
                let inline_submit = submit && !text.is_empty() && !interactive && !text.contains('\n');
                let seq_before = i.seq;
                let tail_before = if submit && !inline_submit && !text.is_empty() {
                    i.screen.tail_text(PROMPT_TAIL_LINES)
                } else {
                    String::new()
                };
                let mut bytes = Vec::new();
                if focus_events && !text.is_empty() {
                    bytes.extend_from_slice(b"\x1b[I");
                }
                bytes.extend_from_slice(&paste_bytes(text, bracketed));
                if inline_submit {
                    bytes.push(b'\r');
                }
                match i.write(&bytes) {
                    Ok(done) => {
                        let cleared = if inline_submit {
                            let had = i.draft.active();
                            i.draft.clear();
                            had
                        } else {
                            false
                        };
                        break (busy, done, i.generation, i.status, inline_submit, cleared, interactive, seq_before, tail_before);
                    }
                    Err(e) => return cannot_write(&term.id, e),
                }
            }
            let now = Instant::now();
            i.draft.stale_at().filter(|at| *at > now).map_or(deadline, |at| at.min(deadline))
        };
        if Instant::now() >= deadline {
            return Reply::error(
                ErrorCode::DraftBusy,
                format!(
                    "a person is typing in {}; the draft was not sent or cleared within {limit} ms",
                    term.id
                ),
            );
        }
        term.wait_change(seen, wake);
    };
    if stale_cleared {
        draft_event(engine, term, true, false);
    }
    if cleared_draft {
        draft_event(engine, term, true, false);
    }
    if let Some(reply) = write_failed(term, done) {
        return reply;
    }
    let waited_ms = began.elapsed().as_millis() as u64;
    if submit && !submitted_inline {
        let (echoed, tail_after_text) = if !text.is_empty() {
            let seq_after = wait_for_paste_settle(term, generation, seq_before_write, interactive);
            let tail_after = lock(term).screen.tail_text(PROMPT_TAIL_LINES);
            let echoed = seq_after > seq_before_write && tail_after != tail_before_write;
            (echoed, tail_after)
        } else {
            (false, String::new())
        };
        let needle = prompt_needle(text);
        let attempts = if echoed { SUBMIT_RETRY_WAITS.len() } else { 1 };
        let mut confirmed = false;
        for (attempt, wait) in SUBMIT_RETRY_WAITS.iter().take(attempts).enumerate() {
            let (before, done, seq_before_enter) = {
                let mut i = lock(term);
                if !i.running || i.generation != generation {
                    return exited(&term.id);
                }
                if i.draft.active() && !forced {
                    return Reply::error(
                        ErrorCode::DraftBusy,
                        format!(
                            "the text was typed into {} but not submitted: a person started typing",
                            term.id
                        ),
                    );
                }
                let use_kitty = i.screen.report_all_keys()
                    || (attempt == 2 && (interactive || i.screen.kitty_keyboard()));
                let seq_now = i.seq;
                let done = match i.write(enter_bytes(use_kitty)) {
                    Ok(done) => done,
                    Err(e) => return cannot_write(&term.id, e),
                };
                let before = i.draft.active();
                i.draft.clear();
                (before, done, seq_now)
            };
            draft_event(engine, term, before, false);
            if let Some(reply) = write_failed(term, done) {
                return reply;
            }
            if !echoed {
                confirmed = true;
                break;
            }
            if wait_enter_confirmed(
                term,
                generation,
                status_before,
                seq_before_enter,
                &tail_after_text,
                needle.as_deref(),
                *wait,
            ) {
                confirmed = true;
                break;
            }
        }
        if echoed && !confirmed {
            let mut i = lock(term);
            if i.running && i.generation == generation && !i.draft.active() {
                i.pending_submit = Some(terminal::PendingSubmit {
                    needle,
                    tail_after_text,
                    generation,
                    armed_at: Instant::now(),
                    retries_left: 2,
                });
            }
        }
    }
    let (started, tail) = if submit && !text.is_empty() { watch_start(term, status_before) } else { (None, String::new()) };
    Reply::Sent {
        id: term.id.clone(),
        waited_ms,
        forced,
        started,
        tail,
    }
}

fn watch_start(term: &Terminal, before: Option<Status>) -> (Option<bool>, String) {
    if before.is_none() || before == Some(Status::Working) {
        return (None, String::new());
    }
    let deadline = Instant::now() + START_WATCH;
    loop {
        let seen = term.tick();
        {
            let i = lock(term);
            if !i.running {
                return (Some(false), i.screen.tail_text(TAIL_LINES));
            }
            if i.status == Some(Status::Working) || i.status == Some(Status::Blocked) {
                return (Some(true), String::new());
            }
            if Instant::now() >= deadline {
                return (Some(false), i.screen.tail_text(TAIL_LINES));
            }
        }
        term.wait_change(seen, deadline);
    }
}

fn input(engine: &Engine, term: &Terminal, text: &str) -> Reply {
    let (before, after, done) = {
        let mut i = lock(term);
        if !i.running {
            return exited(&term.id);
        }
        let done = match i.write(text.as_bytes()) {
            Ok(done) => done,
            Err(e) => return cannot_write(&term.id, e),
        };
        let before = i.draft.active();
        i.draft.feed(text);
        if i.draft.active() {
            i.pending_submit = None;
        }
        (before, i.draft.active(), done)
    };
    draft_event(engine, term, before, after);
    if let Some(reply) = write_failed(term, done) {
        return reply;
    }
    Reply::Ok
}

fn press(engine: &Engine, term: &Terminal, keys: &[Key]) -> Reply {
    let (before, after, done) = {
        let mut i = lock(term);
        if !i.running {
            return exited(&term.id);
        }
        let app = i.screen.app_cursor();
        let bytes: Vec<u8> = keys.iter().flat_map(|k| k.to_bytes(app)).collect();
        let done = match i.write(&bytes) {
            Ok(done) => done,
            Err(e) => return cannot_write(&term.id, e),
        };
        let before = i.draft.active();
        i.draft.program_keys(&bytes);
        i.pending_submit = None;
        (before, i.draft.active(), done)
    };
    draft_event(engine, term, before, after);
    if let Some(reply) = write_failed(term, done) {
        return reply;
    }
    Reply::Ok
}

fn interrupt(engine: &Engine, term: &Terminal) -> Reply {
    let (status_before, exits, had_draft, done) = {
        let mut i = lock(term);
        let was_running = i.running;
        let done = match was_running.then(|| i.write(b"\x03")).transpose() {
            Ok(done) => done,
            Err(e) => return cannot_write(&term.id, e),
        };
        let had = i.draft.active();
        i.draft.clear();
        i.pending_submit = None;
        (i.status, if was_running { Some(i.exits) } else { None }, had, done)
    };
    draft_event(engine, term, had_draft, false);
    if let Some(done) = done
        && let Some(reply) = write_failed(term, done)
    {
        return reply;
    }
    if let Some(exits) = exits {
        let deadline = Instant::now() + INTERRUPT_WATCH;
        loop {
            let seen = term.tick();
            {
                let i = lock(term);
                if i.status != status_before || i.exits != exits {
                    break;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            term.wait_change(seen, deadline);
        }
    }
    let i = lock(term);
    Reply::Interrupted {
        report: InterruptReport {
            id: term.id.clone(),
            status_before,
            status_after: i.status,
            running: i.running,
            exit_code: i.exit_code,
            tail: i.screen.tail_text(TAIL_LINES),
        },
    }
}

fn resize(engine: &Engine, term: &Terminal, rows: u16, cols: u16) -> Reply {
    if rows == 0 || cols < 2 {
        return bad("rows must be at least 1 and cols at least 2");
    }
    let seq = match terminal::resize(engine, term, rows, cols) {
        Ok(Some(seq)) => seq,
        Ok(None) => return Reply::Ok,
        Err(e) => return internal(format!("cannot resize {}: {e:#}", term.id)),
    };
    engine.emit_all(vec![
        Event::Resized {
            id: term.id.clone(),
            rows,
            cols,
        },
        Event::OutputChanged {
            id: term.id.clone(),
            seq,
        },
    ]);
    Reply::Ok
}

fn label_set(engine: &Engine, term: &Terminal, key: String, value: Option<String>) -> Reply {
    if key.trim().is_empty() || key.contains('=') {
        return bad("label key must not be empty or contain =");
    }
    let labels = {
        let mut i = lock(term);
        match value {
            Some(v) => i.labels.insert(key, v),
            None => i.labels.remove(&key),
        };
        i.labels.clone()
    };
    engine.emit(Event::Label {
        id: term.id.clone(),
        labels,
    });
    Reply::Ok
}

fn rename(engine: &Engine, term: &Terminal, name: String) -> Reply {
    let name = name.trim().to_string();
    if name.is_empty() {
        return bad("name must not be empty");
    }
    lock(term).name = name.clone();
    engine.emit(Event::Renamed {
        id: term.id.clone(),
        name,
    });
    Reply::Ok
}

fn keep(engine: &Arc<Engine>, term: &Arc<Terminal>, on: bool) -> Reply {
    let respawn = {
        let mut i = lock(term);
        i.keep = on;
        i.fast_exits = 0;
        let respawn = on && !i.running && !i.removed;
        if respawn {
            i.killed = false;
        }
        respawn
    };
    if respawn {
        terminal::respawn_shell(engine, term);
    }
    Reply::Ok
}


fn tool_path(env: &mut Vec<(String, String)>) {
    let base = env.iter().rev().find(|(k, _)| k == "PATH").map(|(_, v)| v.clone()).or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    let home = nixa_protocol::paths::home_dir();
    let cli = std::env::current_exe().ok().and_then(|exe| nixa_protocol::paths::bundled_cli(&exe)).or_else(nixa_protocol::paths::nixa_bin);
    let bin_dir = cli.as_deref().and_then(Path::parent);
    env.push(("PATH".to_string(), nixa_protocol::paths::tool_path_with_bin(&base, home.as_deref(), bin_dir)));
}

fn heavy_path(env: &mut Vec<(String, String)>) {
    let data = nixa_protocol::paths::data_dir();
    let base = env.iter().rev().find(|(k, _)| k == "PATH").map(|(_, v)| v.clone()).or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    env.push(("PATH".to_string(), nixa_protocol::heavy::path_with(&data, &base)));
}

static GATE_NOTE: std::sync::Once = std::sync::Once::new();

fn gate_path(env: &mut Vec<(String, String)>) {
    let dir = nixa_protocol::paths::data_dir().join("git").join("bin");
    let ready = std::fs::metadata(&dir).is_ok_and(|m| m.is_dir() && m.permissions().mode() & 0o100 != 0)
        && std::fs::metadata(dir.join("gh")).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if !ready {
        GATE_NOTE.call_once(|| eprintln!("nixa-engine: the gh gate {} is missing or cannot run, terminals use the normal gh", dir.display()));
        return;
    }
    let base = env.iter().rev().find(|(k, _)| k == "PATH").map(|(_, v)| v.clone()).or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    let dir = dir.display().to_string();
    let parts: Vec<&str> = std::iter::once(dir.as_str()).chain(base.split(':').filter(|p| !p.is_empty() && *p != dir)).collect();
    env.push(("PATH".to_string(), parts.join(":")));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_terminal(id: &TermId, parent: Option<TermId>) -> Arc<Terminal> {
        Arc::new(terminal::new_terminal(NewTerm {
            id: id.clone(),
            name: "fixture".into(),
            labels: BTreeMap::new(),
            parent,
            cmd: Vec::new(),
            cwd: "/".into(),
            env: Vec::new(),
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            keep: false,
        }))
    }

    #[test]
    fn nested_start_is_refused_and_clearing_unlinks_only_the_parent() {
        let engine = Arc::new(crate::engine::new_engine("/dev/null/engine.sock".into()));
        let parent_id = TermId::from_number(1);
        let child_id = TermId::from_number(2);
        let existing_helper_id = TermId::from_number(3);
        let parent = fixture_terminal(&parent_id, None);
        let child = fixture_terminal(&child_id, Some(parent_id.clone()));
        let existing_helper = fixture_terminal(&existing_helper_id, Some(child_id.clone()));
        lock(&parent).children.push(child_id.clone());
        lock(&child).children.push(existing_helper_id.clone());
        engine.insert(parent.clone());
        engine.insert(child.clone());
        engine.insert(existing_helper.clone());

        let create = Request::Create { spec: CreateSpec {
            parent: Some(child_id.clone()),
            cwd: Some("/".into()),
            ..CreateSpec::default()
        }};
        assert!(matches!(
            dispatch(&engine, create),
            Reply::Error { code: ErrorCode::BadRequest, message }
            if message.contains("m2 as a parent: it already has a parent")
        ));
        assert_eq!(engine.terms().len(), 3);

        let events = engine.subscribe(Vec::new(), false);
        let clear = Request::ClearParent { id: child_id.clone() };
        assert_eq!(dispatch(&engine, clear.clone()), Reply::Ok);
        let detached = terminal::info(&child);
        assert_eq!(detached.parent, None);
        assert_eq!(detached.children, vec![existing_helper_id]);
        assert_eq!(terminal::info(&existing_helper).parent.as_ref(), Some(&child_id));
        assert!(terminal::info(&parent).children.is_empty());
        assert_eq!(engine.terms().len(), 3);
        assert_eq!(events.try_recv().unwrap(), Event::ParentCleared { id: child_id, parent: parent_id });
        assert_eq!(dispatch(&engine, clear), Reply::Ok);
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn labels_filter() {
        let mut labels = BTreeMap::new();
        labels.insert("role".to_string(), "tester".to_string());
        assert!(label_matches(&labels, "role"));
        assert!(label_matches(&labels, "role=tester"));
        assert!(!label_matches(&labels, "role=builder"));
        assert!(!label_matches(&labels, "space"));
    }

    #[test]
    fn helpers() {
        assert_eq!(last_lines("a\nb\nc", 2), "b\nc");
        assert_eq!(last_lines("a", 5), "a");
        assert_eq!(default_name(&["/usr/bin/claude".to_string()]), "claude");
        assert_eq!(default_name(&[]), "shell");
        assert_eq!(paste_bytes("a\nb", true), b"\x1b[200~a\nb\x1b[201~".to_vec());
        assert_eq!(paste_bytes("ab", true), b"\x1b[200~ab\x1b[201~".to_vec());
        assert_eq!(paste_bytes("ab", false), b"ab".to_vec());
        assert_eq!(paste_bytes("a\x1b[201~b", true), b"\x1b[200~a[201~b\x1b[201~".to_vec());
        let nested = paste_bytes("a\x1b[201\x1b[201~~b", true);
        let end = b"\x1b[201~";
        let ends = nested.windows(end.len()).filter(|w| *w == end).count();
        assert_eq!(ends, 1);
        assert!(nested.ends_with(end));
        assert_eq!(nested.iter().filter(|b| **b == 0x1b).count(), 2);
        let raw = paste_bytes("ls\r\nrm\rx\n\x1b[A\x03", false);
        assert_eq!(raw, b"lsrmx[A".to_vec());
        assert!(!raw.contains(&b'\r') && !raw.contains(&b'\n'));
        assert_eq!(paste_bytes("a\tb\rc", true), b"\x1b[200~a\tbc\x1b[201~".to_vec());
        assert_eq!(prompt_needle("  hello world  \nsecond"), Some("hello world".into()));
        assert_eq!(prompt_needle("hi"), None);
        let stuck = "───\n❯ move the s2 tasks to s6\n───\n bypass permissions";
        assert!(still_in_prompt(stuck, stuck, Some("move the s2 tasks to s6")));
        let clock_tick = "───\n❯ move the s2 tasks to s6\n───\n bypass permissions · 9:43 PM";
        assert!(still_in_prompt(stuck, clock_tick, Some("move the s2 tasks to s6")));
        let submitted = "───\n❯ move the s2 tasks to s6\n✻ Working...\n❯";
        assert!(!still_in_prompt(stuck, submitted, Some("move the s2 tasks to s6")));
        let cleared = "───\n❯\n───\n bypass permissions";
        assert!(!still_in_prompt(stuck, cleared, Some("move the s2 tasks to s6")));
    }
}
