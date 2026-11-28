use nixa_protocol::{CreateSpec, Event, Reply, Request, TermId, connect, wire};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const PANES: usize = 8;
const ROWS: u16 = 50;
const COLS: u16 = 200;
const FLOOD: &str = "seq 1 60000; i=0; while [ $i -lt 400 ]; do printf '\\033[31mred\\033[0m \\033[1;32mbold green\\033[0m \\033[48;5;236m shaded \\033[0m plain text to fill the line up to the edge %s\\n' $i $i $i $i $i $i $i $i $i $i; i=$((i+1)); done";

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

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ms-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ping_ok(socket: &Path) -> bool {
    connect(socket).is_ok_and(|mut c| matches!(c.call(&Request::Ping), Ok(Reply::Pong { .. })))
}

fn start(tag: &str) -> (Daemon, Duration) {
    let dir = test_dir(tag);
    let socket = dir.join("e.sock");
    let began = Instant::now();
    let child = Command::new(env!("CARGO_BIN_EXE_nixa-engine"))
        .arg("--socket")
        .arg(&socket)
        .env("SHELL", "/bin/sh")
        .env_remove("NIXA_TERM_ID")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let daemon = Daemon { child, dir, socket };
    let deadline = began + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ping_ok(&daemon.socket) {
            return (daemon, began.elapsed());
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("engine did not start");
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn spread(mut v: Vec<Duration>) -> String {
    if v.is_empty() {
        return "none".to_string();
    }
    v.sort();
    let at = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    format!(
        "n={} p50={:.1}ms p95={:.1}ms max={:.1}ms",
        v.len(),
        ms(at(0.5)),
        ms(at(0.95)),
        ms(at(1.0))
    )
}

fn p95(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v.get(((v.len().max(1) - 1) as f64 * 0.95).round() as usize).copied().unwrap_or_default()
}

#[test]
fn start_time_is_short() {
    let mut times = Vec::new();
    for n in 0..5 {
        let (_d, took) = start(&format!("st{n}"));
        times.push(took);
    }
    println!("speed: engine start to first answer {}", spread(times.clone()));
    assert!(p95(times) < Duration::from_secs(2));
}

struct Seen {
    snaps: Vec<Duration>,
    bytes: usize,
    events: usize,
}

fn view_loop(socket: PathBuf, id: TermId, done: Arc<AtomicBool>, seen: Arc<Mutex<Seen>>) {
    let (tx, rx) = channel::<Event>();
    let ev_socket = socket.clone();
    let ev_id = id.clone();
    thread::spawn(move || {
        if let Ok(c) = connect(&ev_socket) {
            let _ = c.subscribe(vec![ev_id], true, |e| tx.send(e).is_ok());
        }
    });
    let mut client = connect(&socket).unwrap();
    let mut have: Option<u64> = None;
    loop {
        let Ok(first) = rx.recv_timeout(Duration::from_millis(200)) else {
            if done.load(Ordering::SeqCst) {
                return;
            }
            continue;
        };
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut ended = false;
        let mut want = false;
        for e in &batch {
            match e {
                Event::OutputChanged { seq, .. } => want |= have != Some(*seq),
                Event::Exit { .. } => ended = true,
                _ => {}
            }
        }
        seen.lock().unwrap().events += batch.len();
        if want {
            let began = Instant::now();
            let reply = client
                .call(&Request::Snapshot { id: id.clone(), scroll: 0, since_seq: have })
                .unwrap();
            let took = began.elapsed();
            let mut s = seen.lock().unwrap();
            s.snaps.push(took);
            if let Reply::Snapshot { snapshot } = &reply {
                have = Some(snapshot.seq);
                s.bytes += wire::reply_line(&reply).len();
            }
        }
        if ended {
            return;
        }
    }
}

fn big_file(dir: &Path) -> (PathBuf, usize) {
    let line = "\x1b[31mred\x1b[0m \x1b[1;32mbold green\x1b[0m \x1b[48;5;236m shaded \x1b[0m plain text that fills most of a wide pane line ";
    let mut text = String::new();
    for n in 0..60_000 {
        text.push_str(line);
        text.push_str(&n.to_string());
        text.push_str("\r\n");
    }
    text.push_str("big file end\r\n");
    let path = dir.join("big.txt");
    fs::write(&path, &text).unwrap();
    (path, text.len())
}

#[test]
fn big_output_is_read_fast_and_snapshots_stay_quick() {
    let (d, _) = start("big");
    let (path, size) = big_file(&d.dir);
    let socket = d.socket.clone();
    let done = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Seen { snaps: Vec::new(), bytes: 0, events: 0 }));
    let began = Instant::now();
    let mut viewers = Vec::new();
    for n in 0..4 {
        let reply = connect(&socket)
            .unwrap()
            .call(&Request::Create {
                spec: CreateSpec {
                    name: Some(format!("cat{n}")),
                    cmd: vec!["/bin/sh".to_string(), "-c".to_string(), format!("sleep 0.3; cat '{}'", path.display())],
                    cwd: Some(d.dir.display().to_string()),
                    rows: Some(ROWS),
                    cols: Some(COLS),
                    ..CreateSpec::default()
                },
            })
            .unwrap();
        let Reply::Term { term } = reply else {
            panic!("create failed: {reply:?}");
        };
        let (socket, done, seen) = (socket.clone(), done.clone(), seen.clone());
        viewers.push(thread::spawn(move || view_loop(socket, term.id, done, seen)));
    }
    for v in viewers {
        v.join().unwrap();
    }
    let wall = began.elapsed().saturating_sub(Duration::from_millis(300));
    done.store(true, Ordering::SeqCst);
    let s = seen.lock().unwrap();
    println!(
        "speed: 4 x cat {:.1} MB in {:.0}ms, {:.1} MB/s",
        size as f64 / 1e6,
        ms(wall),
        4.0 * size as f64 / 1e6 / wall.as_secs_f64()
    );
    println!("speed: snapshot calls while reading {}", spread(s.snaps.clone()));
    assert!(p95(s.snaps.clone()) < Duration::from_millis(500));
}

#[test]
fn many_fast_terminals_stay_responsive() {
    let (d, _) = start("flood");
    let socket = d.socket.clone();
    let done = Arc::new(AtomicBool::new(false));
    let pings = Arc::new(Mutex::new(Vec::new()));
    let probe = {
        let (socket, done, pings) = (socket.clone(), done.clone(), pings.clone());
        thread::spawn(move || {
            let mut c = connect(&socket).unwrap();
            while !done.load(Ordering::SeqCst) {
                let began = Instant::now();
                c.call(&Request::Ping).unwrap();
                pings.lock().unwrap().push(began.elapsed());
                thread::sleep(Duration::from_millis(10));
            }
        })
    };
    let seen = Arc::new(Mutex::new(Seen { snaps: Vec::new(), bytes: 0, events: 0 }));
    let began = Instant::now();
    let mut viewers = Vec::new();
    let mut ids = Vec::new();
    for n in 0..PANES {
        let reply = connect(&socket)
            .unwrap()
            .call(&Request::Create {
                spec: CreateSpec {
                    name: Some(format!("flood{n}")),
                    cmd: vec!["/bin/sh".to_string(), "-c".to_string(), format!("sleep 0.3; {FLOOD}")],
                    cwd: Some(d.dir.display().to_string()),
                    rows: Some(ROWS),
                    cols: Some(COLS),
                    ..CreateSpec::default()
                },
            })
            .unwrap();
        let Reply::Term { term } = reply else {
            panic!("create failed: {reply:?}");
        };
        ids.push(term.id.clone());
        let (socket, done, seen) = (socket.clone(), done.clone(), seen.clone());
        viewers.push(thread::spawn(move || view_loop(socket, term.id, done, seen)));
    }
    let limit = Instant::now() + Duration::from_secs(120);
    for v in viewers {
        while !v.is_finished() && Instant::now() < limit {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(v.is_finished(), "a flood terminal did not end in time");
        v.join().unwrap();
    }
    let wall = began.elapsed();
    done.store(true, Ordering::SeqCst);
    probe.join().unwrap();
    let mut tail_ok = 0;
    for id in &ids {
        if let Ok(Reply::Text { text, .. }) = connect(&socket).unwrap().call(&Request::Read {
            id: id.clone(),
            source: nixa_protocol::ReadSource::Screen,
            lines: None,
        }) && text.contains("edge 399")
        {
            tail_ok += 1;
        }
    }
    let s = seen.lock().unwrap();
    let pings = pings.lock().unwrap().clone();
    println!("speed: {PANES} terminals {ROWS}x{COLS} flood done in {:.0}ms", ms(wall));
    println!("speed: snapshot calls {}", spread(s.snaps.clone()));
    println!(
        "speed: snapshot data {:.1} MB, {:.0} KB each, {} events",
        s.bytes as f64 / 1e6,
        s.bytes as f64 / 1e3 / s.snaps.len().max(1) as f64,
        s.events
    );
    println!("speed: ping during flood {}", spread(pings.clone()));
    assert_eq!(tail_ok, PANES, "every terminal shows its last line");
    assert!(p95(pings) < Duration::from_millis(250));
}

#[test]
fn message_passing_between_terminals_is_fast() {
    let (d, _) = start("msgpass");
    let mut client = connect(&d.socket).unwrap();
    let make = |c: &mut nixa_protocol::Client, name: &str, parent: Option<TermId>| -> TermId {
        let reply = c
            .call(&Request::Create {
                spec: CreateSpec {
                    name: Some(name.to_string()),
                    cmd: vec!["/bin/cat".to_string()],
                    cwd: Some(d.dir.display().to_string()),
                    parent,
                    ..CreateSpec::default()
                },
            })
            .unwrap();
        let Reply::Term { term } = reply else {
            panic!("create failed: {reply:?}");
        };
        term.id
    };

    let parent = make(&mut client, "agent-a", None);
    let child = make(&mut client, "agent-b", Some(parent.clone()));

    let mut send_times = Vec::new();
    for i in 0..30 {
        let tag = format!("ping-{i:03}");
        let began = Instant::now();
        let sent = client
            .call(&Request::Send {
                id: child.clone(),
                text: tag.clone(),
                submit: true,
                timeout_ms: Some(2000),
                force: false,
            })
            .unwrap();
        assert!(matches!(sent, Reply::Sent { .. }));
        let waited = client
            .call(&Request::Wait {
                id: child.clone(),
                until: nixa_protocol::WaitFor::Text {
                    pattern: tag,
                    regex: false,
                },
                timeout_ms: Some(2000),
            })
            .unwrap();
        assert!(matches!(
            waited,
            Reply::Waited { result } if result.outcome == nixa_protocol::WaitOutcome::Met
        ));
        send_times.push(began.elapsed());
    }
    println!("speed: terminal send+receive {}", spread(send_times.clone()));
    assert!(p95(send_times) < Duration::from_millis(50));

    let (tx, rx) = channel::<Event>();
    let ev_socket = d.socket.clone();
    let ev_parent = parent.clone();
    let sub = thread::spawn(move || {
        if let Ok(c) = connect(&ev_socket) {
            let _ = c.subscribe(vec![ev_parent], false, |e| tx.send(e).is_ok());
        }
    });
    thread::sleep(Duration::from_millis(20));
    let mut report_times = Vec::new();
    for i in 0..30 {
        let msg = format!("done-{i:03}");
        let began = Instant::now();
        let reply = client
            .call(&Request::ChildReport {
                id: child.clone(),
                parent: Some(parent.clone()),
                text: msg.clone(),
            })
            .unwrap();
        assert_eq!(reply, Reply::Ok);
        let ev = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(ev, Event::ChildNote { note, .. } if note.contains(&msg)));
        report_times.push(began.elapsed());
    }
    drop(rx);
    let _ = client.call(&Request::Remove { id: child });
    let _ = client.call(&Request::Remove { id: parent });
    let _ = sub.join();
    println!("speed: child report to parent event {}", spread(report_times.clone()));
    assert!(p95(report_times) < Duration::from_millis(20));
}
