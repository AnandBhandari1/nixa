use anyhow::{Result, bail};
use nixa_protocol::{
    Client, CreateSpec, EngineError, Event, Reply, Request, TermId, TermInfo, connect, paths, wire,
};
use std::io::BufReader;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY: Duration = Duration::from_secs(2);
const PING_EVERY: Duration = Duration::from_secs(5);
const MAX_IDLE: usize = 4;

static POOL: LazyLock<Mutex<Vec<Client>>> = LazyLock::new(|| Mutex::new(Vec::new()));

pub enum EngineMsg {
    Up(Vec<TermInfo>),
    Down(String),
    Event(Box<Event>),
}

fn take_client(path: &Path) -> Option<Client> {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    while let Some(pos) = pool.iter().rposition(|c| c.path() == path) {
        let client = pool.swap_remove(pos);
        if client.is_idle_alive() {
            return Some(client);
        }
    }
    None
}

fn put_client(client: Client) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    if pool.len() < MAX_IDLE {
        pool.push(client);
    }
}

fn call(req: &Request) -> Result<Reply> {
    call_at(&paths::socket_path(), req, CALL_TIMEOUT)
}

pub fn create(spec: CreateSpec) -> Result<TermInfo> {
    match call(&Request::Create { spec })? {
        Reply::Term { term } => Ok(term),
        other => bail!("unexpected reply to create: {other:?}"),
    }
}

pub fn remove(id: TermId) -> Result<()> {
    call(&Request::Remove { id })?;
    Ok(())
}

pub fn label(id: TermId, key: &str, value: &str) -> Result<()> {
    call(&Request::LabelSet {
        id,
        key: key.to_string(),
        value: Some(value.to_string()),
    })?;
    Ok(())
}

pub fn unlabel(id: TermId, key: &str) -> Result<()> {
    call(&Request::LabelSet {
        id,
        key: key.to_string(),
        value: None,
    })?;
    Ok(())
}

pub fn rename(id: TermId, name: &str) -> Result<()> {
    call(&Request::Rename {
        id,
        name: name.to_string(),
    })?;
    Ok(())
}

pub fn watch_wake(wake: futures::channel::mpsc::UnboundedSender<()>) -> Receiver<EngineMsg> {
    let (tx, rx) = channel();
    thread::Builder::new()
        .name("nixa-engine-watch".into())
        .spawn(move || watch_loop(tx, Some(wake)))
        .ok();
    rx
}

fn push(
    tx: &Sender<EngineMsg>,
    wake: Option<&futures::channel::mpsc::UnboundedSender<()>>,
    msg: EngineMsg,
) -> bool {
    let ok = tx.send(msg).is_ok();
    if ok && let Some(w) = wake {
        let _ = w.unbounded_send(());
    }
    ok
}

fn watch_loop(
    tx: Sender<EngineMsg>,
    wake: Option<futures::channel::mpsc::UnboundedSender<()>>,
) {
    loop {
        let why = match follow(&paths::socket_path(), &tx, wake.as_ref(), CALL_TIMEOUT, PING_EVERY)
        {
            Ok(true) => return,
            Ok(false) => "nixa-engine closed the connection".to_string(),
            Err(e) => e.to_string(),
        };
        if !push(&tx, wake.as_ref(), EngineMsg::Down(why)) {
            return;
        }
        thread::sleep(RETRY);
    }
}

fn call_at(path: &Path, req: &Request, timeout: Duration) -> Result<Reply> {
    if let Some(mut client) = take_client(path)
        && client.set_timeout(Some(timeout)).is_ok()
    {
        match client.call(req) {
            Ok(reply) => {
                put_client(client);
                return Ok(reply);
            }
            Err(e) if e.downcast_ref::<EngineError>().is_some() => {
                put_client(client);
                return Err(e);
            }
            Err(_) => {}
        }
    }
    let mut client = connect(path)?;
    client.set_timeout(Some(timeout))?;
    match client.call(req) {
        Ok(reply) => {
            put_client(client);
            Ok(reply)
        }
        Err(e) => {
            if e.downcast_ref::<EngineError>().is_some() {
                put_client(client);
            }
            Err(e)
        }
    }
}

fn list_at(path: &Path, timeout: Duration) -> Result<Vec<TermInfo>> {
    match call_at(
        path,
        &Request::List {
            parent: None,
            label: None,
        },
        timeout,
    )? {
        Reply::Terms { terms } => Ok(terms),
        other => bail!("unexpected reply to list: {other:?}"),
    }
}

fn watchdog(
    path: PathBuf,
    stream: UnixStream,
    done: Arc<AtomicBool>,
    timeout: Duration,
    every: Duration,
) {
    thread::spawn(move || {
        loop {
            thread::sleep(every);
            if done.load(Ordering::Relaxed) {
                return;
            }
            if call_at(&path, &Request::Ping, timeout).is_err() {
                stream.shutdown(Shutdown::Both).ok();
                return;
            }
        }
    });
}

fn follow(
    path: &Path,
    tx: &Sender<EngineMsg>,
    wake: Option<&futures::channel::mpsc::UnboundedSender<()>>,
    timeout: Duration,
    every: Duration,
) -> Result<bool> {
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream.try_clone()?);
    wire::write_request(
        &mut writer,
        &Request::Subscribe {
            ids: Vec::new(),
            output: false,
        },
    )?;
    match wire::read_reply(&mut reader)? {
        Some(Reply::Subscribed) => {}
        Some(Reply::Error { code, message }) => bail!("engine error {code}: {message}"),
        other => bail!("unexpected reply to subscribe: {other:?}"),
    }
    stream.set_read_timeout(None)?;
    if !push(tx, wake, EngineMsg::Up(list_at(path, timeout)?)) {
        return Ok(true);
    }
    let done = Arc::new(AtomicBool::new(false));
    watchdog(path.to_path_buf(), stream, done.clone(), timeout, every);
    let result = stream_events(&mut reader, tx, wake, path, timeout);
    done.store(true, Ordering::Relaxed);
    result
}

fn stream_events(
    reader: &mut BufReader<UnixStream>,
    tx: &Sender<EngineMsg>,
    wake: Option<&futures::channel::mpsc::UnboundedSender<()>>,
    path: &Path,
    timeout: Duration,
) -> Result<bool> {
    let mut line = String::new();
    while let Some(reply) = wire::read_reply_into(reader, &mut line)? {
        let Reply::Event { event } = reply else { continue };
        let message = match event {
            Event::ParentCleared { .. } => EngineMsg::Up(list_at(path, timeout)?),
            event => EngineMsg::Event(Box::new(event)),
        };
        if !push(tx, wake, message) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixListener;
    use std::time::Instant;

    fn socket(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ma-{name}-{}.sock", std::process::id()));
        std::fs::remove_file(&path).ok();
        path
    }

    #[test]
    fn clearing_a_parent_refreshes_the_terminal_tree() {
        let path = socket("parent-cleared");
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            assert_eq!(wire::read_request(&mut reader).unwrap(), Some(Request::List { parent: None, label: None }));
            wire::write_reply(&mut stream, &Reply::Terms { terms: Vec::new() }).unwrap();
        });
        let (read, mut write) = UnixStream::pair().unwrap();
        wire::write_reply(&mut write, &Reply::Event {
            event: Event::ParentCleared { id: TermId::from_number(2), parent: TermId::from_number(1) },
        }).unwrap();
        drop(write);
        let (tx, rx) = channel();
        assert!(!stream_events(&mut BufReader::new(read), &tx, None, &path, Duration::from_secs(1)).unwrap());
        assert!(matches!(rx.recv().unwrap(), EngineMsg::Up(terms) if terms.is_empty()));
        server.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_silent_engine_does_not_hang_the_watch() {
        let path = socket("silent");
        let listener = UnixListener::bind(&path).unwrap();
        let hold = thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_secs(2));
            drop(conn);
        });
        let (tx, _rx) = channel();
        let start = Instant::now();
        let result = follow(
            &path,
            &tx,
            None,
            Duration::from_millis(200),
            Duration::from_secs(60),
        );
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        hold.join().unwrap();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_hung_stream_is_cut_by_the_watchdog() {
        let path = socket("hung");
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (sub, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(sub.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let mut out = sub.try_clone().unwrap();
            out.write_all(wire::reply_line(&Reply::Subscribed).as_bytes())
                .unwrap();
            let (list, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(list.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let mut out = list.try_clone().unwrap();
            out.write_all(wire::reply_line(&Reply::Terms { terms: Vec::new() }).as_bytes())
                .unwrap();
            let event = Reply::Event {
                event: Event::Removed {
                    id: TermId::from_number(7),
                },
            };
            let mut out = sub.try_clone().unwrap();
            out.write_all(wire::reply_line(&event).as_bytes()).unwrap();
            drop(listener);
            thread::sleep(Duration::from_secs(2));
            drop(sub);
        });
        let (tx, rx) = channel();
        let start = Instant::now();
        let result = follow(
            &path,
            &tx,
            None,
            Duration::from_millis(200),
            Duration::from_millis(100),
        );
        assert!(start.elapsed() < Duration::from_millis(1500));
        assert!(matches!(result, Ok(false) | Err(_)));
        assert!(matches!(rx.recv().unwrap(), EngineMsg::Up(terms) if terms.is_empty()));
        assert!(
            matches!(rx.recv().unwrap(), EngineMsg::Event(e) if matches!(*e, Event::Removed { .. }))
        );
        server.join().unwrap();
        std::fs::remove_file(&path).ok();
    }
}
