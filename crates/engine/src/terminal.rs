mod reflow;
pub mod reports;

pub use reflow::resize;

use crate::cwd;
use crate::engine::Engine;
use crate::output::{Output, new_output};
use crate::screen::{Screen, new_screen};
use crate::draft::Draft;
use crate::writer::{self, Done, PtyWriter};
use alacritty_terminal::event::{Event as TermEvent, WindowSize};
use anyhow::Result;
use nixa_protocol::{Event, Status, TermId, TermInfo};
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const EXIT_DRAIN: Duration = Duration::from_millis(500);
const KILL_GRACE: Duration = Duration::from_millis(1500);
const CWD_EVERY: Duration = Duration::from_millis(100);
pub const OUTPUT_EVERY: Duration = Duration::from_millis(16);
pub const FAST_EXIT: Duration = Duration::from_secs(2);
pub const FAST_EXIT_LIMIT: u32 = 3;
const PENDING_SUBMIT_MAX: Duration = Duration::from_secs(60);
const PENDING_NUDGE_DELAY: Duration = Duration::from_millis(120);

#[derive(Clone, Debug)]
pub struct PendingSubmit {
    pub needle: Option<String>,
    pub tail_after_text: String,
    pub generation: u64,
    pub armed_at: Instant,
    pub retries_left: u8,
}

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: PtyWriter,
    killer: Box<dyn ChildKiller + Send + Sync>,
    zsh: bool,
}

pub struct Inner {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub parent: Option<TermId>,
    pub children: Vec<TermId>,
    child_reports: reports::Inbox,
    job_active: bool,
    pub cmd: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    pub pid: Option<u32>,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub status: Option<Status>,
    pub status_note: Option<String>,
    pub keep: bool,
    pub title: Option<String>,
    pub seq: u64,
    pub input_mark: usize,
    pub screen: Screen,
    pub output: Output,
    pub draft: Draft,
    pub generation: u64,
    pub exits: u64,
    pub killed: bool,
    pub removed: bool,
    pub live_cwd: Option<String>,
    cwd_checked: Option<Instant>,
    pub fast_exits: u32,
    spawned_at: Instant,
    out_sent: Option<Instant>,
    out_sent_seq: u64,
    out_due: bool,
    reader_done: u64,
    pty: Option<Pty>,
    resize_pending: Option<reflow::Pending>,
    pub pending_submit: Option<PendingSubmit>,
}

pub struct Terminal {
    pub id: TermId,
    pub created_ms: u64,
    pub inner: Mutex<Inner>,
    resize_serial: Mutex<()>,
    tick: Mutex<u64>,
    changed: Condvar,
}

pub struct NewTerm {
    pub id: TermId,
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub parent: Option<TermId>,
    pub cmd: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    pub rows: u16,
    pub cols: u16,
    pub keep: bool,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn new_terminal(spec: NewTerm) -> Terminal {
    Terminal {
        id: spec.id,
        created_ms: now_ms(),
        resize_serial: Mutex::new(()),
        tick: Mutex::new(0),
        changed: Condvar::new(),
        inner: Mutex::new(Inner {
            name: spec.name,
            labels: spec.labels,
            parent: spec.parent,
            children: Vec::new(),
            child_reports: reports::Inbox::default(),
            job_active: false,
            cmd: spec.cmd,
            cwd: spec.cwd,
            env: spec.env,
            pid: None,
            running: false,
            exit_code: None,
            status: None,
            status_note: None,
            keep: spec.keep,
            title: None,
            seq: 0,
            input_mark: 0,
            screen: new_screen(spec.rows, spec.cols),
            output: new_output(),
            draft: Draft::default(),
            generation: 0,
            exits: 0,
            killed: false,
            removed: false,
            live_cwd: None,
            cwd_checked: None,
            fast_exits: 0,
            spawned_at: Instant::now(),
            out_sent: None,
            out_sent_seq: 0,
            out_due: false,
            reader_done: 0,
            pty: None,
            resize_pending: None,
            pending_submit: None,
        }),
    }
}

impl Terminal {
    pub fn tick(&self) -> u64 {
        *self.tick.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn bump(&self) {
        let mut t = self.tick.lock().unwrap_or_else(|e| e.into_inner());
        *t = t.wrapping_add(1);
        self.changed.notify_all();
    }

    pub fn wait_change(&self, seen: u64, deadline: Instant) {
        let mut t = self.tick.lock().unwrap_or_else(|e| e.into_inner());
        while *t == seen {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            t = match self.changed.wait_timeout(t, deadline - now) {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
        }
    }
}

pub fn lock(term: &Terminal) -> MutexGuard<'_, Inner> {
    term.inner.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<Done> {
        match &self.pty {
            Some(p) => {
                let done = writer::queue(&p.writer, bytes.to_vec());
                self.input_mark = self.output.end();
                Ok(done)
            }
            None => Err(writer::gone()),
        }
    }

    fn reply(&self, bytes: &[u8]) {
        if let Some(p) = &self.pty {
            writer::queue_reply(&p.writer, bytes.to_vec());
        }
    }

    fn signal_hangup(&mut self) {
        if let Some(p) = &mut self.pty {
            let _ = p.killer.kill();
        }
    }
}

pub fn info_of(term: &Terminal, i: &Inner) -> TermInfo {
    TermInfo {
        id: term.id.clone(),
        name: i.name.clone(),
        labels: i.labels.clone(),
        parent: i.parent.clone(),
        children: i.children.clone(),
        cmd: i.cmd.clone(),
        cwd: i.cwd.clone(),
        pid: i.pid,
        running: i.running,
        exit_code: i.exit_code,
        status: i.status,
        status_note: i.status_note.clone(),
        keep: i.keep,
        draft: i.draft.active(),
        title: i.title.clone(),
        rows: i.screen.rows(),
        cols: i.screen.cols(),
        created_ms: term.created_ms,
        seq: i.seq,
    }
}

pub fn info(term: &Terminal) -> TermInfo {
    let i = lock(term);
    info_of(term, &i)
}

pub fn spawn(engine: &Arc<Engine>, term: &Arc<Terminal>, cmd: &[String]) -> Result<Option<u32>> {
    let (rows, cols, cwd, env) = {
        let i = lock(term);
        (i.screen.rows(), i.screen.cols(), i.cwd.clone(), i.env.clone())
    };
    let pair = native_pty_system().openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut builder = match cmd.split_first() {
        Some((prog, args)) => {
            let mut b = CommandBuilder::new(prog);
            b.args(args);
            b.args(crate::agent_hooks::extra_args(cmd));
            b
        }
        None => CommandBuilder::new_default_prog(),
    };
    builder.cwd(&cwd);
    drop_host_session(&mut builder);
    for (k, v) in &env {
        builder.env(k, v);
    }
    let shell = cmd.first().cloned().unwrap_or_else(|| builder.get_shell());
    let zsh = Path::new(&shell).file_name().is_some_and(|name| name == "zsh");
    let reader = pair.master.try_clone_reader()?;
    let writer = writer::start_writer(format!("write-{}", term.id), pair.master.take_writer()?)?;
    let (child, pid, generation) = {
        let mut i = lock(term);
        anyhow::ensure!(!i.removed && !i.killed, "terminal {} was closed", term.id);
        reports::configure_env(&mut builder, i.parent.as_ref());
        let child = pair.slave.spawn_command(builder)?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let pid = child.process_id();
        i.generation += 1;
        i.pty = Some(Pty {
            master: pair.master,
            writer,
            killer,
            zsh,
        });
        i.pid = pid;
        i.running = true;
        i.exit_code = None;
        i.killed = false;
        i.job_active = false;
        i.spawned_at = Instant::now();
        if i.generation == 1 {
            link_parent(engine, &term.id, &mut i);
            reports::startup(&mut i);
            engine.emit(Event::Created { term: info_of(term, &i) });
        }
        (child, pid, i.generation)
    };
    start_reader(engine.clone(), term.clone(), reader, generation);
    start_waiter(engine.clone(), term.clone(), child, generation);
    Ok(pid)
}

fn link_parent(engine: &Engine, child: &TermId, inner: &mut Inner) {
    let parent = inner.parent.as_ref().and_then(|id| engine.get(id));
    if let Some(mut parent) = parent.as_deref().map(lock).filter(|parent| !parent.removed) {
        parent.children.push(child.clone());
    } else {
        inner.parent = None;
    }
}

pub fn clear_parent(engine: &Engine, term: &Terminal) {
    let mut child = lock(term);
    let Some(parent) = child.parent.take() else {
        return;
    };
    if let Some(old) = engine.get(&parent) {
        lock(&old).children.retain(|id| id != &term.id);
    }
    engine.emit(Event::ParentCleared { id: term.id.clone(), parent });
}

fn start_reader(engine: Arc<Engine>, term: Arc<Terminal>, mut reader: Box<dyn Read + Send>, generation: u64) {
    let name = format!("read-{}", term.id);
    let started = thread::Builder::new().name(name).spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => on_output(&engine, &term, &buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        {
            let mut i = lock(&term);
            i.reader_done = i.reader_done.max(generation);
        }
        term.bump();
    });
    if let Err(e) = started {
        eprintln!("nixa-engine: cannot start reader thread: {e}");
    }
}

fn start_waiter(
    engine: Arc<Engine>,
    term: Arc<Terminal>,
    mut child: Box<dyn Child + Send + Sync>,
    generation: u64,
) {
    let name = format!("wait-{}", term.id);
    let started = thread::Builder::new().name(name).spawn(move || {
        let code = match child.wait() {
            Ok(status) => status.exit_code() as i32,
            Err(_) => -1,
        };
        let deadline = Instant::now() + EXIT_DRAIN;
        loop {
            let seen = term.tick();
            if lock(&term).reader_done >= generation || Instant::now() >= deadline {
                break;
            }
            term.wait_change(seen, deadline);
        }
        on_exit(&engine, &term, generation, code);
    });
    if let Err(e) = started {
        eprintln!("nixa-engine: cannot start wait thread: {e}");
    }
}

fn throttle(i: &mut Inner, now: Instant) -> Result<bool, Instant> {
    if i.out_due {
        return Ok(false);
    }
    match i.out_sent {
        Some(sent) if now.duration_since(sent) < OUTPUT_EVERY => {
            i.out_due = true;
            Err(sent + OUTPUT_EVERY)
        }
        _ => {
            i.out_sent = Some(now);
            i.out_sent_seq = i.seq;
            Ok(true)
        }
    }
}

pub fn flush_output(engine: &Engine, term: &Terminal) {
    let seq = {
        let mut i = lock(term);
        i.out_due = false;
        if i.seq == i.out_sent_seq {
            return;
        }
        i.out_sent = Some(Instant::now());
        i.out_sent_seq = i.seq;
        i.seq
    };
    engine.emit(Event::OutputChanged {
        id: term.id.clone(),
        seq,
    });
    note_cwd(term);
}

fn note_cwd(term: &Terminal) {
    let (pid, generation) = {
        let mut i = lock(term);
        if !i.keep {
            return;
        }
        let now = Instant::now();
        if i.cwd_checked.is_some_and(|at| now.duration_since(at) < CWD_EVERY) {
            return;
        }
        i.cwd_checked = Some(now);
        (i.pid, i.generation)
    };
    let Some(dir) = pid.and_then(cwd::of_pid) else {
        return;
    };
    let mut i = lock(term);
    if i.generation == generation && i.running {
        i.live_cwd = Some(dir);
    }
}

fn on_output(engine: &Engine, term: &Terminal, bytes: &[u8]) {
    let mut events = Vec::new();
    let mut flush_at = None;
    let mut sent_now = false;
    let statuses = {
        let mut i = lock(term);
        reflow::feed(&mut i, bytes);
        i.output.feed(bytes);
        i.seq += 1;
        match throttle(&mut i, Instant::now()) {
            Ok(true) => {
                sent_now = true;
                events.push(Event::OutputChanged {
                    id: term.id.clone(),
                    seq: i.seq,
                });
            }
            Ok(false) => {}
            Err(at) => flush_at = Some(at),
        }
        for ev in i.screen.take_events() {
            match ev {
                TermEvent::PtyWrite(text) => i.reply(text.as_bytes()),
                TermEvent::TextAreaSizeRequest(format) => {
                    let size = WindowSize {
                        num_lines: i.screen.rows(),
                        num_cols: i.screen.cols(),
                        cell_width: 8,
                        cell_height: 16,
                    };
                    i.reply(format(size).as_bytes());
                }
                TermEvent::Title(title) => {
                    if i.title.as_deref() != Some(title.as_str()) {
                        i.title = Some(title.clone());
                        events.push(Event::Title {
                            id: term.id.clone(),
                            title: Some(title),
                        });
                    }
                }
                TermEvent::ResetTitle if i.title.take().is_some() => {
                    events.push(Event::Title {
                        id: term.id.clone(),
                        title: None,
                    });
                }
                _ => {}
            }
        }
        i.output.take_statuses()
    };
    term.bump();
    if !events.is_empty() {
        engine.emit_all(events);
    }
    if let Some(at) = flush_at {
        engine.flush_later(term.id.clone(), at);
    }
    if sent_now {
        note_cwd(term);
    }
    for (status, note) in statuses {
        set_status(engine, term, Some(status), note);
    }
}

pub fn set_status(engine: &Engine, term: &Terminal, status: Option<Status>, note: Option<String>) {
    let note = note.filter(|n| !n.trim().is_empty());
    let mut i = lock(term);
    if i.status == status && i.status_note == note {
        return;
    }
    let was_working = i.status == Some(Status::Working);
    i.status = status;
    i.status_note = note.clone();
    if matches!(status, Some(Status::Working) | Some(Status::Blocked)) {
        i.pending_submit = None;
    }
    let should_nudge = was_working
        && matches!(status, None | Some(Status::Idle) | Some(Status::Done))
        && i.pending_submit.as_ref().is_some_and(|p| {
            p.generation == i.generation
                && p.retries_left > 0
                && p.armed_at.elapsed() <= PENDING_SUBMIT_MAX
        });
    let report = reports::status(engine, term, &mut i, note.as_deref());
    let mut events = vec![Event::Status {
        id: term.id.clone(),
        status,
        note,
    }];
    events.extend(report);
    drop(i);
    term.bump();
    engine.emit_all(events);
    if should_nudge && let Some(arc) = engine.get(&term.id) {
        nudge_pending_submit(arc);
    }
}

fn nudge_pending_submit(term: Arc<Terminal>) {
    let _ = thread::Builder::new()
        .name(format!("submit-nudge-{}", term.id))
        .spawn(move || {
            thread::sleep(PENDING_NUDGE_DELAY);
            let done = {
                let mut i = lock(&term);
                if !i.running || i.draft.active() || i.status == Some(Status::Working) {
                    return;
                }
                let Some(mut pending) = i.pending_submit.take() else {
                    return;
                };
                if pending.generation != i.generation
                    || pending.retries_left == 0
                    || pending.armed_at.elapsed() > PENDING_SUBMIT_MAX
                {
                    return;
                }
                let tail_now = i.screen.tail_text(crate::handlers::PROMPT_TAIL_LINES);
                if !crate::handlers::still_in_prompt(
                    &pending.tail_after_text,
                    &tail_now,
                    pending.needle.as_deref(),
                ) {
                    return;
                }
                let enter = crate::handlers::enter_bytes(i.screen.report_all_keys());
                let Ok(done) = i.write(enter) else {
                    return;
                };
                pending.retries_left = pending.retries_left.saturating_sub(1);
                if pending.retries_left > 0 {
                    i.pending_submit = Some(pending);
                }
                done
            };
            let _ = writer::finish(done);
        });
}

fn on_exit(engine: &Arc<Engine>, term: &Arc<Terminal>, generation: u64, code: i32) {
    let (respawn, give_up, had_draft, report, ended) = {
        let mut i = lock(term);
        if i.generation != generation {
            return;
        }
        let was_busy = matches!(i.status, Some(Status::Working) | Some(Status::Blocked));
        i.running = false;
        i.exit_code = Some(code);
        i.pid = None;
        i.pty = None;
        i.exits += 1;
        let had_draft = i.draft.active();
        i.draft.clear();
        i.pending_submit = None;
        if i.spawned_at.elapsed() < FAST_EXIT {
            i.fast_exits += 1;
        } else {
            i.fast_exits = 0;
        }
        let wanted = i.keep && !i.killed && !i.removed;
        let give_up = wanted && i.fast_exits >= FAST_EXIT_LIMIT;
        let respawn = wanted && !give_up;
        let ended = (was_busy && !respawn).then(|| {
            i.status = Some(if code == 0 { Status::Done } else { Status::Error });
            i.status_note = Some(format!("the program ended with code {code}"));
            (i.status, i.status_note.clone())
        });
        let report = reports::exit(engine, term, &mut i, code, respawn);
        (respawn, give_up, had_draft, report, ended)
    };
    let mut events = Vec::new();
    if let Some((status, note)) = ended {
        events.push(Event::Status {
            id: term.id.clone(),
            status,
            note,
        });
    }
    events.push(Event::Exit {
        id: term.id.clone(),
        code: Some(code),
    });
    if had_draft {
        events.push(Event::Draft {
            id: term.id.clone(),
            active: false,
        });
    }
    if let Some((report, removed)) = report {
        events.push(report);
        if removed {
            events.push(Event::Removed { id: term.id.clone() });
        }
    }
    term.bump();
    engine.emit_all(events);
    if respawn {
        respawn_shell(engine, term);
    }
    if give_up {
        let note = format!("keep stopped: the program ended {FAST_EXIT_LIMIT} times in a row right after it started");
        eprintln!("nixa-engine: {}: {note}", term.id);
        set_status(engine, term, Some(Status::Error), Some(note));
    }
}

pub fn respawn_shell(engine: &Arc<Engine>, term: &Arc<Terminal>) {
    {
        let mut i = lock(term);
        if i.running || i.killed || i.removed {
            return;
        }
        i.cwd = cwd::pick(i.live_cwd.as_deref(), &i.cwd);
    }
    match spawn(engine, term, &[]) {
        Ok(pid) => {
            let had_status = {
                let mut i = lock(term);
                i.cmd.clear();
                let had = i.status.is_some() || i.status_note.is_some();
                i.status = None;
                i.status_note = None;
                had
            };
            let mut events = vec![Event::Respawned {
                id: term.id.clone(),
                pid,
            }];
            if had_status {
                events.push(Event::Status {
                    id: term.id.clone(),
                    status: None,
                    note: None,
                });
            }
            term.bump();
            engine.emit_all(events);
        }
        Err(e) => eprintln!("nixa-engine: cannot start a fresh shell in {}: {e:#}", term.id),
    }
}

fn wait_stopped(_engine: &Engine, term: &Terminal, generation: u64, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        let seen = term.tick();
        {
            let i = lock(term);
            if !i.running || i.generation != generation {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        term.wait_change(seen, deadline);
    }
}

pub fn kill(engine: &Engine, term: &Terminal) {
    let (generation, pid) = {
        let mut i = lock(term);
        i.killed = true;
        if !i.running {
            return;
        }
        i.signal_hangup();
        (i.generation, i.pid)
    };
    if wait_stopped(engine, term, generation, KILL_GRACE) {
        return;
    }
    if let Some(pid) = pid {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    wait_stopped(engine, term, generation, KILL_GRACE);
}

pub fn hangup_all(engine: &Engine) {
    for term in engine.terms() {
        let mut i = lock(&term);
        i.killed = true;
        i.removed = true;
        i.signal_hangup();
    }
}

fn drop_host_session(builder: &mut CommandBuilder) {
    for key in nixa_protocol::paths::HOST_SESSION_ENV {
        builder.env_remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_does_not_get_the_host_claude_session() {
        let mut builder = CommandBuilder::new("sh");
        builder.env("CLAUDE_CODE_ENTRYPOINT", "claude-desktop");
        builder.env("CLAUDECODE", "1");
        builder.env("CLAUDE_CODE_EXPERIMENTAL_CODE_MODE", "1");
        drop_host_session(&mut builder);
        assert_eq!(builder.get_env("CLAUDE_CODE_ENTRYPOINT"), None);
        assert_eq!(builder.get_env("CLAUDECODE"), None);
        assert!(builder.get_env("CLAUDE_CODE_EXPERIMENTAL_CODE_MODE").is_some());
    }

    fn term() -> Terminal {
        new_terminal(NewTerm {
            id: TermId::from_number(1),
            name: "t".to_string(),
            labels: BTreeMap::new(),
            parent: None,
            cmd: Vec::new(),
            cwd: "/".to_string(),
            env: Vec::new(),
            rows: 2,
            cols: 10,
            keep: false,
        })
    }

    #[test]
    fn output_events_are_throttled() {
        let t = term();
        let mut i = lock(&t);
        let start = Instant::now();
        i.seq = 1;
        assert_eq!(throttle(&mut i, start), Ok(true));
        i.seq = 2;
        assert_eq!(throttle(&mut i, start + Duration::from_millis(2)), Err(start + OUTPUT_EVERY));
        i.seq = 3;
        assert_eq!(throttle(&mut i, start + Duration::from_millis(4)), Ok(false));
        assert!(i.out_due);
        assert_eq!(i.out_sent_seq, 1);
        i.out_due = false;
        i.seq = 4;
        assert_eq!(throttle(&mut i, start + OUTPUT_EVERY), Ok(true));
        assert_eq!(i.out_sent_seq, 4);
    }
}
