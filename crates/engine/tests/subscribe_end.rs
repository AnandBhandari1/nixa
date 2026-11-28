use nixa_protocol::{CreateSpec, ErrorCode, EngineError, Event, Reply, Request, TermId, connect};
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::thread;
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: PathBuf,
    socket: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn start(tag: &str) -> Daemon {
    let dir = std::env::temp_dir().join(format!("mu-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("e.sock");
    let child = Command::new(env!("CARGO_BIN_EXE_nixa-engine"))
        .arg("--socket")
        .arg(&socket)
        .env("SHELL", "/bin/sh")
        .env_remove("NIXA_TERM_ID")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let d = Daemon { child, dir, socket };
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if connect(&d.socket).is_ok_and(|mut c| matches!(c.call(&Request::Ping), Ok(Reply::Pong { .. }))) {
            return d;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("engine did not start");
}

fn create(d: &Daemon) -> TermId {
    let reply = connect(&d.socket)
        .unwrap()
        .call(&Request::Create {
            spec: CreateSpec {
                cmd: vec!["/bin/sh".to_string()],
                cwd: Some(d.dir.display().to_string()),
                ..CreateSpec::default()
            },
        })
        .unwrap();
    match reply {
        Reply::Term { term } => term.id,
        other => panic!("create failed: {other:?}"),
    }
}

fn remove(d: &Daemon, id: &TermId) {
    connect(&d.socket).unwrap().call(&Request::Remove { id: id.clone() }).unwrap();
}

#[cfg(target_os = "macos")]
fn threads(pid: u32) -> usize {
    let out = Command::new("ps").args(["-M", "-p", &pid.to_string()]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().count().saturating_sub(1)
}

#[cfg(not(target_os = "macos"))]
fn threads(pid: u32) -> usize {
    fs::read_dir(format!("/proc/{pid}/task")).map(|d| d.count()).unwrap_or(0)
}

fn wait_threads(pid: u32, want: usize) -> usize {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let n = threads(pid);
        if n <= want || Instant::now() >= deadline {
            return n;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_stream_ends_when_its_terminals_are_removed() {
    let d = start("end");
    let pid = d.child.id();
    let base = threads(pid);
    let id = create(&d);
    let with_term = threads(pid);
    let (tx, rx) = channel();
    let client = connect(&d.socket).unwrap();
    let watched = id.clone();
    let streamer = thread::spawn(move || {
        let mut seen = Vec::new();
        let result = client.subscribe(vec![watched], false, |e| {
            seen.push(e);
            true
        });
        tx.send((result.is_ok(), seen)).ok();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while threads(pid) < with_term + 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    remove(&d, &id);
    let (ok, seen) = rx.recv_timeout(Duration::from_secs(5)).expect("the event stream did not end");
    streamer.join().unwrap();
    assert!(ok);
    assert!(seen.iter().any(|e| matches!(e, Event::Removed { id: r } if *r == id)));
    assert_eq!(wait_threads(pid, base), base, "engine threads go back to the start count");
}

#[test]
fn a_stream_with_live_terminals_left_goes_on() {
    let d = start("left");
    let a = create(&d);
    let b = create(&d);
    let (tx, rx) = channel();
    let client = connect(&d.socket).unwrap();
    let ids = vec![a.clone(), b.clone()];
    thread::spawn(move || {
        let result = client.subscribe(ids, false, |e| tx.send(e).is_ok());
        drop(result);
    });
    thread::sleep(Duration::from_millis(200));
    remove(&d, &a);
    connect(&d.socket)
        .unwrap()
        .call(&Request::Rename { id: b.clone(), name: "still here".to_string() })
        .unwrap();
    let mut got_rename = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !got_rename {
        if let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) {
            got_rename |= matches!(e, Event::Renamed { ref id, .. } if *id == b);
        }
    }
    assert!(got_rename, "events of the terminal still alive keep coming");
}

#[test]
fn subscribing_to_unknown_terminals_is_refused() {
    let d = start("unknown");
    let pid = d.child.id();
    let base = threads(pid);
    let gone = create(&d);
    remove(&d, &gone);
    let err = connect(&d.socket).unwrap().subscribe(vec![gone], false, |_| true).unwrap_err();
    let code = err.downcast_ref::<EngineError>().map(|e| e.code);
    assert_eq!(code, Some(ErrorCode::NotFound));
    assert_eq!(wait_threads(pid, base), base);
}

#[test]
fn a_stream_of_all_terminals_stays_after_a_remove() {
    let d = start("all");
    let a = create(&d);
    let (tx, rx) = channel();
    let client = connect(&d.socket).unwrap();
    thread::spawn(move || {
        let result = client.subscribe(Vec::new(), false, |e| tx.send(e).is_ok());
        drop(result);
    });
    thread::sleep(Duration::from_millis(200));
    remove(&d, &a);
    let b = create(&d);
    let mut created = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !created {
        if let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) {
            created |= matches!(e, Event::Created { ref term } if term.id == b);
        }
    }
    assert!(created);
}
