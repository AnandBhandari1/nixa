use crate::check::PAGE_ROUTES;
use crate::run;
use std::fs::{self, File};
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const DOOR_PORT: u16 = 7430;
pub const SETUP_PORT: u16 = 7431;
pub const LOOPBACK: &str = "127.0.0.1";
const DOOR_FILE: &str = "phone-door.json";
const START_WAIT: Duration = Duration::from_secs(40);
const DOOR_WAIT: Duration = Duration::from_secs(20);
const STOP_WAIT: Duration = Duration::from_secs(3);
const STEP: Duration = Duration::from_millis(250);

pub struct Target {
    pub name: &'static str,
    pub base: String,
    pub tls: bool,
}

pub fn targets(brain_addr: &str) -> Vec<Target> {
    vec![
        Target { name: "phone door", base: format!("https://{LOOPBACK}:{DOOR_PORT}"), tls: true },
        Target { name: "phone door setup", base: format!("http://{LOOPBACK}:{SETUP_PORT}"), tls: false },
        Target { name: "brain", base: format!("http://{brain_addr}"), tls: false },
    ]
}

pub fn port_free(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok() && TcpListener::bind((LOOPBACK, port)).is_ok()
}

pub fn free_port() -> Result<u16, String> {
    TcpListener::bind((LOOPBACK, 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("cannot find a free port: {e}"))
}

pub fn code(url: &str, tls: bool) -> u16 {
    let mut cmd = Command::new("/usr/bin/curl");
    cmd.args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "--max-time", "3", "--noproxy", "*"]);
    if tls {
        cmd.arg("-k");
    }
    cmd.arg(url);
    run::output(&mut cmd)
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out.stdout).trim().parse().ok())
        .unwrap_or(0)
}

pub fn verdict(name: &str, path: &str, code: u16) -> Option<String> {
    match code {
        0 => Some(format!("the {name} did not answer {path}")),
        200..=299 => Some(format!("the {name} serves the page route {path} (HTTP {code})")),
        _ => None,
    }
}

fn tail(log: &Path) -> String {
    let text = fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(20)..].join("\n")
}

fn wait_for(child: &mut Child, limit: Duration, mut ready: impl FnMut() -> bool) -> Result<(), String> {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("it ended ({status})"));
        }
        if ready() {
            return Ok(());
        }
        thread::sleep(STEP);
    }
    Err(format!("no answer in {} s", limit.as_secs()))
}

fn signal(group: u32, name: &str) {
    Command::new("/bin/kill")
        .args(["-s", name, "--", &format!("-{group}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
}

fn stop(mut child: Child) {
    let group = child.id();
    signal(group, "TERM");
    let until = Instant::now() + STOP_WAIT;
    while Instant::now() < until && matches!(child.try_wait(), Ok(None)) {
        thread::sleep(Duration::from_millis(50));
    }
    signal(group, "KILL");
    child.kill().ok();
    child.wait().ok();
}

fn start(app: &Path, base: &Path, addr: &str) -> Result<(Child, PathBuf), String> {
    let data = base.join("data");
    let home = base.join("home");
    for dir in [&data, &home] {
        fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    }
    fs::write(data.join(DOOR_FILE), "{\"on\":true,\"mobile\":false}\n").map_err(|e| format!("cannot write {DOOR_FILE}: {e}"))?;
    let log = base.join("brain.log");
    let out = File::create(&log).map_err(|e| format!("cannot make {}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| format!("cannot use {}: {e}", log.display()))?;
    let bin = crate::layout::bin_dir(app).join("nixa-brain");
    let child = Command::new(&bin)
        .current_dir(&data)
        .env("HOME", &home)
        .env("NIXA_DATA_DIR", &data)
        .env("NIXA_BRAIN_ADDR", addr)
        .env("NIXA_ENGINE_SOCKET", base.join("e.sock"))
        .env("NIXA_PHONE_ADDRESS", LOOPBACK)
        .env_remove("NIXA_TERM_ID")
        .env_remove("NIXA_BIN")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", bin.display()))?;
    Ok((child, log))
}

fn probe(child: &mut Child, addr: &str, log: &Path) -> Result<Vec<String>, String> {
    let health = format!("http://{addr}/health");
    wait_for(child, START_WAIT, || code(&health, false) == 200)
        .map_err(|why| format!("the bundled nixa-brain did not start: {why}. Log:\n{}", tail(log)))?;
    let door = format!("https://{LOOPBACK}:{DOOR_PORT}/");
    wait_for(child, DOOR_WAIT, || code(&door, true) != 0)
        .map_err(|why| format!("the bundled phone door did not start: {why}. Log:\n{}", tail(log)))?;
    let mut problems = Vec::new();
    for target in targets(addr) {
        for path in PAGE_ROUTES {
            let got = code(&format!("{}{path}", target.base), target.tls);
            problems.extend(verdict(target.name, path, got));
        }
    }
    Ok(problems)
}

pub fn check(app: &Path) -> Result<Vec<String>, String> {
    for port in [DOOR_PORT, SETUP_PORT] {
        if !port_free(port) {
            return Err(format!(
                "port {port} is in use, so the bundled phone door cannot start for the page route check. Turn off the phone door that uses it, or add --no-live-check"
            ));
        }
    }
    let base = PathBuf::from(format!("/tmp/mxb-{}", std::process::id()));
    fs::remove_dir_all(&base).ok();
    let addr = format!("{LOOPBACK}:{}", free_port()?);
    let result = start(app, &base, &addr).and_then(|(mut child, log)| {
        let found = probe(&mut child, &addr, &log);
        stop(child);
        found
    });
    fs::remove_dir_all(&base).ok();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_routes_must_not_answer_ok() {
        assert_eq!(verdict("phone door", "/app.js", 404), None);
        assert_eq!(verdict("brain", "/", 401), None);
        assert_eq!(verdict("phone door", "/", 200).unwrap(), "the phone door serves the page route / (HTTP 200)");
        assert_eq!(verdict("brain", "/wake.js", 0).unwrap(), "the brain did not answer /wake.js");
        let all = targets("127.0.0.1:9");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].base, "https://127.0.0.1:7430");
        assert!(all[0].tls && !all[1].tls);
        assert_eq!(all[2].base, "http://127.0.0.1:9");
    }

    #[test]
    fn a_closed_port_gives_no_code() {
        let port = free_port().unwrap();
        assert_eq!(code(&format!("http://127.0.0.1:{port}/"), false), 0);
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(!port_free(port));
        thread::spawn(move || {
            use std::io::{BufRead, BufReader, Write};
            if let Ok((mut conn, _)) = listener.accept() {
                let mut line = String::new();
                BufReader::new(&conn).read_line(&mut line).ok();
                conn.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").ok();
            }
        });
        assert_eq!(code(&format!("http://127.0.0.1:{port}/app.js"), false), 404);
    }

    #[test]
    fn a_missing_brain_is_reported() {
        let base = crate::testkit::temp("live");
        let err = start(&base.join("none.app"), &base.join("run"), "127.0.0.1:9").unwrap_err();
        assert!(err.contains("cannot start") && err.contains("nixa-brain"), "{err}");
        fs::remove_dir_all(base).ok();
    }
}
