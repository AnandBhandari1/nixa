use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use nixa_protocol::{
    Client, EngineError, ErrorCode, Event, Key, Reply, Request, Snapshot, TermId, connect,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Input(String),
    Keys(Vec<Key>),
    Resize(u16, u16),
    Snapshot { scroll: usize, since: Option<u64> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Linked,
    Snapshot(Snapshot),
    Event(Event),
    Failed(String),
    ResizeFailed(u16, u16),
    Unlinked,
}

pub struct Link {
    jobs: Sender<Job>,
}

impl Link {
    pub fn send(&self, job: Job) {
        self.jobs.send(job).ok();
    }
}

pub fn start(socket: PathBuf, id: TermId) -> (Link, UnboundedReceiver<Msg>) {
    let (job_tx, job_rx) = channel();
    let (msg_tx, msg_rx) = unbounded();
    let events_tx = msg_tx.clone();
    let events_socket = socket.clone();
    let events_id = id.clone();
    thread::Builder::new()
        .name(format!("nixa-termview-jobs-{id}"))
        .spawn(move || run_jobs(&socket, &id, job_rx, msg_tx))
        .ok();
    thread::Builder::new()
        .name(format!("nixa-termview-events-{events_id}"))
        .spawn(move || run_events(&events_socket, &events_id, events_tx))
        .ok();
    (Link { jobs: job_tx }, msg_rx)
}

fn request_for(id: &TermId, job: Job) -> Request {
    let id = id.clone();
    match job {
        Job::Input(text) => Request::Input { id, text },
        Job::Keys(keys) => Request::Keys { id, keys },
        Job::Resize(rows, cols) => Request::Resize { id, rows, cols },
        Job::Snapshot { scroll, since } => Request::Snapshot {
            id,
            scroll,
            since_seq: since,
        },
    }
}

fn call(client: &mut Option<Client>, socket: &Path, req: &Request) -> Result<Reply, String> {
    if client.is_none() {
        let c = connect(socket).map_err(|e| e.to_string())?;
        c.set_timeout(Some(CALL_TIMEOUT))
            .map_err(|e| e.to_string())?;
        *client = Some(c);
    }
    let Some(c) = client.as_mut() else {
        return Err("no engine connection".to_string());
    };
    match c.call(req) {
        Ok(reply) => Ok(reply),
        Err(e) => {
            if e.downcast_ref::<EngineError>().is_none() {
                *client = None;
            }
            Err(e.to_string())
        }
    }
}

fn run_jobs(socket: &Path, id: &TermId, jobs: Receiver<Job>, out: UnboundedSender<Msg>) {
    let mut client: Option<Client> = None;
    while let Ok(first) = jobs.recv() {
        let mut batch = vec![first];
        batch.extend(jobs.try_iter());
        let mut snap = None;
        for job in batch {
            if let Job::Snapshot { .. } = job {
                snap = Some(job);
                continue;
            }
            let resize = match job {
                Job::Resize(rows, cols) => Some((rows, cols)),
                _ => None,
            };
            if let Err(e) = call(&mut client, socket, &request_for(id, job)) {
                out.unbounded_send(Msg::Failed(e)).ok();
                if let Some((rows, cols)) = resize {
                    out.unbounded_send(Msg::ResizeFailed(rows, cols)).ok();
                }
            }
        }
        let Some(job) = snap else {
            continue;
        };
        match call(&mut client, socket, &request_for(id, job)) {
            Ok(Reply::Snapshot { snapshot }) => {
                if out.unbounded_send(Msg::Snapshot(snapshot)).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(e) => {
                out.unbounded_send(Msg::Failed(e)).ok();
            }
        }
        if out.is_closed() {
            return;
        }
    }
}

fn not_found(e: &anyhow::Error) -> bool {
    e.downcast_ref::<EngineError>()
        .is_some_and(|e| e.code == ErrorCode::NotFound)
}

fn run_events(socket: &Path, id: &TermId, out: UnboundedSender<Msg>) {
    loop {
        if let Ok(client) = connect(socket) {
            if out.unbounded_send(Msg::Linked).is_err() {
                return;
            }
            let mut gone = false;
            let result = client.subscribe(vec![id.clone()], true, |event| {
                gone |= matches!(&event, Event::Removed { id: r } if r == id);
                out.unbounded_send(Msg::Event(event)).is_ok() && !gone
            });
            if gone || out.is_closed() {
                return;
            }
            if let Err(e) = &result
                && not_found(e)
            {
                out.unbounded_send(Msg::Event(Event::Removed { id: id.clone() }))
                    .ok();
                return;
            }
            let note = match result {
                Ok(()) => "engine closed the event stream".to_string(),
                Err(e) => e.to_string(),
            };
            out.unbounded_send(Msg::Failed(note)).ok();
            out.unbounded_send(Msg::Unlinked).ok();
        }
        if out.is_closed() {
            return;
        }
        thread::sleep(RETRY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    fn temp_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mtv-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("e.sock");
        std::fs::remove_file(&path).ok();
        path
    }

    fn drain(rx: &mut UnboundedReceiver<Msg>) -> Vec<Msg> {
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m);
        }
        out
    }

    #[test]
    fn failed_resize_is_reported_with_its_size() {
        let socket = temp_socket("down");
        let (job_tx, job_rx) = channel();
        let (out, mut rx) = unbounded();
        job_tx.send(Job::Resize(30, 100)).ok();
        drop(job_tx);
        run_jobs(&socket, &TermId::from_number(1), job_rx, out);
        let msgs = drain(&mut rx);
        assert!(msgs.iter().any(|m| matches!(m, Msg::Failed(_))));
        assert!(msgs.contains(&Msg::ResizeFailed(30, 100)));
        std::fs::remove_dir_all(socket.parent().unwrap()).ok();
    }

    #[test]
    fn accepted_resize_reports_nothing() {
        let socket = temp_socket("up");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("\"op\":\"resize\""));
            writer.write_all(b"{\"reply\":\"ok\"}\n").unwrap();
        });
        let (job_tx, job_rx) = channel();
        let (out, mut rx) = unbounded();
        job_tx.send(Job::Resize(30, 100)).ok();
        drop(job_tx);
        run_jobs(&socket, &TermId::from_number(1), job_rx, out);
        server.join().unwrap();
        assert!(drain(&mut rx).is_empty());
        std::fs::remove_dir_all(socket.parent().unwrap()).ok();
    }
    fn fake_engine(socket: &Path, lines: Vec<String>) -> thread::JoinHandle<()> {
        let listener = UnixListener::bind(socket).unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("\"op\":\"subscribe\""));
            for l in lines {
                writer.write_all(l.as_bytes()).unwrap();
            }
            let mut rest = String::new();
            reader.read_line(&mut rest).ok();
        })
    }

    fn run_with_limit(socket: PathBuf) -> Vec<Msg> {
        let (out, mut rx) = unbounded();
        let (done_tx, done_rx) = channel();
        thread::spawn(move || {
            run_events(&socket, &TermId::from_number(1), out);
            done_tx.send(()).ok();
        });
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the event thread did not end");
        drain(&mut rx)
    }

    #[test]
    fn the_event_thread_ends_when_its_terminal_is_removed() {
        let socket = temp_socket("removed");
        let id = TermId::from_number(1);
        let server = fake_engine(
            &socket,
            vec![
                nixa_protocol::wire::reply_line(&Reply::Subscribed),
                nixa_protocol::wire::reply_line(&Reply::Event {
                    event: Event::Removed { id: id.clone() },
                }),
            ],
        );
        let msgs = run_with_limit(socket.clone());
        server.join().unwrap();
        assert!(msgs.contains(&Msg::Linked));
        assert!(msgs.contains(&Msg::Event(Event::Removed { id })));
        assert!(!msgs.iter().any(|m| matches!(m, Msg::Failed(_))));
        std::fs::remove_dir_all(socket.parent().unwrap()).ok();
    }

    #[test]
    fn an_unknown_terminal_counts_as_removed() {
        let socket = temp_socket("unknown");
        let id = TermId::from_number(1);
        let server = fake_engine(
            &socket,
            vec![nixa_protocol::wire::reply_line(&Reply::not_found(&id))],
        );
        let msgs = run_with_limit(socket.clone());
        server.join().unwrap();
        assert!(msgs.contains(&Msg::Event(Event::Removed { id })));
        assert!(!msgs.iter().any(|m| matches!(m, Msg::Failed(_))));
        std::fs::remove_dir_all(socket.parent().unwrap()).ok();
    }
}
