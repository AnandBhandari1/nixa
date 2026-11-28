use crate::engine::{Engine, new_engine, start_flusher};
use crate::handlers;
use crate::terminal;
use anyhow::{Context, Result, bail};
use nixa_protocol::{ErrorCode, Event, Reply, Request, TermId, paths, wire};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;

const CLOSE_CHECK: Duration = Duration::from_millis(500);

fn prepare_dir(socket: &Path) -> Result<()> {
    let Some(dir) = socket.parent() else {
        return Ok(());
    };
    if !dir.exists() || dir == paths::data_dir() {
        paths::ensure_private_dir(dir).with_context(|| format!("cannot make folder {}", dir.display()))?;
    }
    Ok(())
}

fn bind_private(socket: &Path) -> Result<UnixListener> {
    match fs::symlink_metadata(socket) {
        Ok(meta) if !meta.file_type().is_socket() => {
            bail!("{} is not a socket, so nixa-engine leaves it alone", socket.display());
        }
        Ok(_) if UnixStream::connect(socket).is_ok() => {
            bail!("another nixa-engine already runs on {}", socket.display());
        }
        Ok(_) => fs::remove_file(socket).with_context(|| format!("cannot remove old socket {}", socket.display()))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("cannot check {}", socket.display())),
    }
    let parent = socket.parent().unwrap_or(Path::new("."));
    let hold = parent.join(format!(".e{}", std::process::id()));
    let _ = fs::remove_dir_all(&hold);
    paths::ensure_private_dir(&hold).with_context(|| format!("cannot make folder {}", hold.display()))?;
    let inner = hold.join("s");
    let bound = UnixListener::bind(&inner).map_err(anyhow::Error::from).and_then(|listener| {
        fs::set_permissions(&inner, fs::Permissions::from_mode(0o600))?;
        fs::rename(&inner, socket)?;
        Ok(listener)
    });
    let _ = fs::remove_dir_all(&hold);
    bound.with_context(|| format!("cannot listen on {}", socket.display()))
}

fn same_user(stream: &UnixStream) -> bool {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    let ok = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0;
    ok && uid == unsafe { libc::geteuid() }
}

pub struct Running {
    engine: Arc<Engine>,
}

pub fn start(socket: std::path::PathBuf) -> Result<Running> {
    prepare_dir(&socket)?;
    let listener = bind_private(&socket)?;
    let engine = Arc::new(new_engine(socket));
    start_flusher(engine.clone());
    let back = crate::recover::restore(&engine);
    if back > 0 {
        eprintln!("nixa-engine: brought back {back} terminals after an unplanned stop");
    }
    crate::recover::start_saver(engine.clone());
    let accepting = engine.clone();
    thread::Builder::new()
        .name("accept".to_string())
        .spawn(move || accept(listener, accepting))
        .context("cannot start the accept thread")?;
    Ok(Running { engine })
}

pub fn stop(running: &Running) {
    crate::recover::forget(&running.engine.socket);
    terminal::hangup_all(&running.engine);
    let _ = fs::remove_file(&running.engine.socket);
}

pub fn run(socket: std::path::PathBuf) -> Result<()> {
    let running = start(socket)?;
    eprintln!("nixa-engine {} listening on {}", std::process::id(), running.engine.socket.display());
    loop {
        thread::park();
    }
}

fn accept(listener: UnixListener, engine: Arc<Engine>) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let engine = engine.clone();
                let started = thread::Builder::new()
                    .name("conn".to_string())
                    .spawn(move || serve(engine, stream));
                if let Err(e) = started {
                    eprintln!("nixa-engine: cannot start a connection thread: {e}");
                }
            }
            Err(e) => eprintln!("nixa-engine: accept failed: {e}"),
        }
    }
}

fn serve(engine: Arc<Engine>, stream: UnixStream) {
    if !same_user(&stream) {
        return;
    }
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    let mut line = String::new();
    let mut buf = Vec::with_capacity(256);
    loop {
        let req = match wire::read_request_into(&mut reader, &mut line) {
            Ok(Some(req)) => req,
            Ok(None) => return,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                let reply = Reply::error(ErrorCode::BadRequest, format!("cannot read request: {e}"));
                if wire::write_reply_buf(&mut writer, &reply, &mut buf).is_err() {
                    return;
                }
                continue;
            }
            Err(_) => return,
        };
        match req {
            Request::Subscribe { ids, output } => {
                stream_events(&engine, writer, reader, ids, output);
                return;
            }
            Request::Shutdown => {
                let _ = wire::write_reply_buf(&mut writer, &Reply::Ok, &mut buf);
                shutdown(&engine);
            }
            other => {
                let reply = handlers::dispatch(&engine, other);
                if wire::write_reply_buf(&mut writer, &reply, &mut buf).is_err() {
                    return;
                }
            }
        }
    }
}

fn stream_events(
    engine: &Engine,
    mut writer: UnixStream,
    mut reader: BufReader<UnixStream>,
    ids: Vec<TermId>,
    output: bool,
) {
    let rx = engine.subscribe(ids.clone(), output);
    let mut left: Vec<TermId> = ids.iter().filter(|id| engine.get(id).is_some()).cloned().collect();
    let mut buf = Vec::with_capacity(512);
    if !ids.is_empty() && left.is_empty() {
        let _ = wire::write_reply_buf(&mut writer, &Reply::not_found(&ids[0]), &mut buf);
        return;
    }
    if wire::write_reply_buf(&mut writer, &Reply::Subscribed, &mut buf).is_err() {
        return;
    }
    let closed = Arc::new(AtomicBool::new(false));
    let flag = closed.clone();
    let watcher = thread::Builder::new().name("sub-watch".to_string()).spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        flag.store(true, Ordering::SeqCst);
    });
    if watcher.is_err() {
        return;
    }
    loop {
        match rx.recv_timeout(CLOSE_CHECK) {
            Ok(first) => {
                buf.clear();
                let mut done = false;
                for event in std::iter::once(first).chain(rx.try_iter()) {
                    let gone = match &event {
                        Event::Removed { id } => left.contains(id),
                        _ => false,
                    };
                    wire::append_reply(&Reply::Event { event: event.clone() }, &mut buf);
                    if gone {
                        left.retain(|id| id != event.term_id());
                        if left.is_empty() {
                            done = true;
                            break;
                        }
                    }
                }
                if writer.write_all(&buf).is_err() || writer.flush().is_err() {
                    break;
                }
                if done {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if closed.load(Ordering::SeqCst) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = writer.shutdown(Shutdown::Both);
}

fn shutdown(engine: &Engine) -> ! {
    crate::recover::forget(&engine.socket);
    terminal::hangup_all(engine);
    let _ = fs::remove_file(&engine.socket);
    thread::sleep(Duration::from_millis(150));
    std::process::exit(0);
}

#[cfg(test)]
mod bind_tests {
    use super::*;

    #[test]
    fn bind_private_leaves_a_plain_file_alone_and_binds_a_private_socket() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let file = dir.path().join("e.sock");
        fs::write(&file, "keep me").unwrap();
        let err = bind_private(&file).err().map(|e| format!("{e:#}")).unwrap_or_default();
        assert!(err.contains("not a socket"), "{err}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep me");
        let socket = dir.path().join("s.sock");
        let listener = bind_private(&socket).unwrap();
        let meta = fs::symlink_metadata(&socket).unwrap();
        assert!(meta.file_type().is_socket());
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert!(bind_private(&socket).err().is_some_and(|e| e.to_string().contains("another nixa-engine")));
        drop(listener);
        assert!(bind_private(&socket).is_ok());
        assert!(fs::read_dir(dir.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(&format!(".e{}", std::process::id()))));
    }
}
