use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

struct Place {
    dir: PathBuf,
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn place(tag: &str) -> Place {
    let dir = std::env::temp_dir().join(format!("nixa-brain-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    Place { dir }
}

fn fake_health(mark: PathBuf) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { break };
            let mut buf = [0u8; 2048];
            let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = conn.read(&mut buf);
            if !mark.exists() {
                continue;
            }
            let body = r#"{"ok":true,"name":"Nixa","version":"9.9.9","busy":false,"engine":false,"brain":null}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = conn.write_all(reply.as_bytes());
        }
    });
    port
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn nixa(addr: &str, data: &Path, mark: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nixa"))
        .args(args)
        .env("NIXA_BRAIN_ADDR", addr)
        .env("NIXA_DATA_DIR", data)
        .env("MARK", mark)
        .env_remove("NIXA_BIN")
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

#[test]
fn start_status_stop_with_a_fake_brain() {
    let p = place("cycle");
    let mark = p.dir.join("up");
    let data = p.dir.join("data");
    let port = fake_health(mark.clone());
    let addr = format!("127.0.0.1:{port}");
    let bin = script(
        &p.dir,
        "nixa-brain",
        "trap 'rm -f \"$MARK\"; exit 0' TERM\necho \"addr $NIXA_BRAIN_ADDR bin $NIXA_BIN\"\ntouch \"$MARK\"\nwhile :; do sleep 0.1; done",
    );
    let bin_arg = bin.display().to_string();

    let st = nixa(&addr, &data, &mark, &["brain", "status"]);
    assert_eq!(st.status.code(), Some(1));
    assert!(text(&st.stdout).contains("not running"));

    let up = nixa(&addr, &data, &mark, &["brain", "start", "--bin", &bin_arg]);
    assert_eq!(up.status.code(), Some(0), "{}{}", text(&up.stdout), text(&up.stderr));
    assert!(text(&up.stdout).contains("nixa-brain started at"), "{}", text(&up.stdout));
    let pid_text = fs::read_to_string(data.join("brain.pid")).unwrap();
    let pid = pid_text.lines().next().unwrap().to_string();
    assert!(pid_text.contains(&bin_arg));
    assert_eq!(fs::metadata(data.join("brain.pid")).unwrap().permissions().mode() & 0o777, 0o600);
    std::thread::sleep(Duration::from_millis(200));
    let log = fs::read_to_string(data.join("brain.log")).unwrap();
    assert!(log.contains(&format!("addr {addr} bin ")), "{log}");
    assert!(log.contains("/nixa"), "{log}");

    let st = nixa(&addr, &data, &mark, &["brain", "status"]);
    assert_eq!(st.status.code(), Some(0));
    assert!(text(&st.stdout).contains(&format!("pid {pid}, version 9.9.9, engine down")), "{}", text(&st.stdout));

    let again = nixa(&addr, &data, &mark, &["brain", "start", "--bin", &bin_arg]);
    assert!(text(&again.stdout).contains("already running"));

    let json = nixa(&addr, &data, &mark, &["--json", "brain", "status"]);
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v["running"], true);
    assert_eq!(v["health"]["version"], "9.9.9");

    let down = nixa(&addr, &data, &mark, &["brain", "stop"]);
    assert_eq!(down.status.code(), Some(0), "{}{}", text(&down.stdout), text(&down.stderr));
    assert!(text(&down.stdout).contains(&format!("stopped, pid {pid}")));
    assert!(!mark.exists());
    assert!(!data.join("brain.pid").exists());
    let ps = Command::new("/bin/ps").args(["-p", &pid]).output().unwrap();
    assert!(!ps.status.success());

    let down = nixa(&addr, &data, &mark, &["brain", "stop"]);
    assert!(text(&down.stdout).contains("not running"));
}

#[test]
fn early_exit_shows_the_log() {
    let p = place("early");
    let mark = p.dir.join("up");
    let data = p.dir.join("data");
    let port = fake_health(mark.clone());
    let bin = script(&p.dir, "nixa-brain", "echo boom here\nexit 3");
    let out = nixa(&format!("127.0.0.1:{port}"), &data, &mark, &["brain", "start", "--bin", &bin.display().to_string()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("boom here"), "{}", text(&out.stderr));
    assert!(!data.join("brain.pid").exists());
}

#[test]
fn live_port_is_refused() {
    let p = place("live");
    let mark = p.dir.join("up");
    let bin = script(&p.dir, "nixa-brain", &format!("touch '{}'", p.dir.join("ran").display()));
    for addr in ["127.0.0.1:7321", "127.0.0.1:7330", "127.0.0.1:7331", " 127.0.0.1:7321 ", "192.168.1.20:7421"] {
        for args in [vec!["brain", "start", "--bin", bin.to_str().unwrap()], vec!["brain", "stop"], vec!["brain", "status"], vec!["deep", "ls"]] {
            let out = nixa(addr, &p.dir.join("data"), &mark, &args);
            assert_eq!(out.status.code(), Some(1), "{addr} {args:?}");
            assert!(text(&out.stderr).contains("is not the nixa-brain address of Nixa"), "{addr}: {}", text(&out.stderr));
        }
    }
    assert!(!p.dir.join("ran").exists());
}
