use crate::terminal_ids;
use crate::terminal::{self, Terminal};
use nixa_protocol::{Event, TermId};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Instant;

struct State {
    terms: BTreeMap<TermId, Arc<Terminal>>,
}

struct Sub {
    ids: Vec<TermId>,
    output: bool,
    tx: Sender<Event>,
}

pub struct Engine {
    pub socket: PathBuf,
    ids_dir: PathBuf,
    counter_file: PathBuf,
    state: Mutex<State>,
    subs: Mutex<Vec<Sub>>,
    tick: Mutex<u64>,
    changed: Condvar,
    due: Mutex<Vec<(Instant, TermId)>>,
    due_changed: Condvar,
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn new_engine(socket: PathBuf) -> Engine {
    let dir = socket.parent().unwrap_or(Path::new(".")).to_path_buf();
    Engine {
        socket,
        ids_dir: dir.join("terminal-ids"),
        counter_file: dir.join(crate::counter::COUNTER_FILE),
        state: Mutex::new(State {
            terms: BTreeMap::new(),
        }),
        subs: Mutex::new(Vec::new()),
        tick: Mutex::new(0),
        changed: Condvar::new(),
        due: Mutex::new(Vec::new()),
        due_changed: Condvar::new(),
    }
}

pub fn start_flusher(engine: Arc<Engine>) {
    let started = thread::Builder::new().name("output-flush".to_string()).spawn(move || {
        loop {
            for id in engine.next_due() {
                if let Some(term) = engine.get(&id) {
                    terminal::flush_output(&engine, &term);
                }
            }
        }
    });
    if let Err(e) = started {
        eprintln!("nixa-engine: cannot start the output flush thread: {e}");
    }
}

fn wants(sub: &Sub, event: &Event) -> bool {
    (sub.output || !event.is_output()) && (sub.ids.is_empty() || sub.ids.contains(event.term_id()))
}

impl Engine {
    pub fn next_id(&self, space: &str, agent: &str) -> io::Result<TermId> {
        terminal_ids::reserve(&self.ids_dir, &self.counter_file, space, agent)
    }

    pub fn get(&self, id: &TermId) -> Option<Arc<Terminal>> {
        guard(&self.state).terms.get(id).cloned()
    }

    pub fn terms(&self) -> Vec<Arc<Terminal>> {
        guard(&self.state).terms.values().cloned().collect()
    }

    pub fn insert(&self, term: Arc<Terminal>) {
        guard(&self.state).terms.insert(term.id.clone(), term);
    }

    pub fn take(&self, id: &TermId) -> Option<Arc<Terminal>> {
        guard(&self.state).terms.remove(id)
    }

    pub fn subscribe(&self, ids: Vec<TermId>, output: bool) -> Receiver<Event> {
        let (tx, rx) = channel();
        guard(&self.subs).push(Sub { ids, output, tx });
        rx
    }

    pub fn emit(&self, event: Event) {
        self.emit_all(vec![event]);
    }

    pub fn emit_all(&self, events: Vec<Event>) {
        if events.is_empty() {
            return;
        }
        let notify_global = events.iter().any(|e| !e.is_output());
        guard(&self.subs).retain(|sub| {
            events
                .iter()
                .filter(|e| wants(sub, e))
                .all(|e| sub.tx.send(e.clone()).is_ok())
        });
        if notify_global {
            self.bump();
        }
    }

    pub fn bump(&self) {
        let mut t = guard(&self.tick);
        *t = t.wrapping_add(1);
        self.changed.notify_all();
    }

    pub fn tick(&self) -> u64 {
        *guard(&self.tick)
    }

    pub fn flush_later(&self, id: TermId, at: Instant) {
        guard(&self.due).push((at, id));
        self.due_changed.notify_one();
    }

    fn next_due(&self) -> Vec<TermId> {
        let mut due = guard(&self.due);
        loop {
            let now = Instant::now();
            if due.iter().any(|(at, _)| *at <= now) {
                let (ready, later): (Vec<_>, Vec<_>) = due.drain(..).partition(|(at, _)| *at <= now);
                *due = later;
                return ready.into_iter().map(|(_, id)| id).collect();
            }
            due = match due.iter().map(|(at, _)| *at - now).min() {
                Some(left) => match self.due_changed.wait_timeout(due, left) {
                    Ok((g, _)) => g,
                    Err(e) => e.into_inner().0,
                },
                None => self.due_changed.wait(due).unwrap_or_else(|e| e.into_inner()),
            };
        }
    }

    pub fn wait_change(&self, seen: u64, deadline: Instant) {
        let mut t = guard(&self.tick);
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
