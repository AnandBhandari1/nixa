use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use crate::{engine, help, http};
use anyhow::{Context, Result, anyhow, bail};
use nixa_protocol::paths;
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const BRAIN_BIN: &str = "nixa-brain";
const LOG_FILE: &str = "brain.log";
const PID_FILE: &str = "brain.pid";
const POLL: Duration = Duration::from_millis(100);
const PING: Duration = Duration::from_secs(2);

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::BRAIN.as_bytes())?;
        return Ok(2);
    };
    match sub.as_str() {
        "start" => start(ctx, rest, out),
        "stop" => stop(ctx, rest, out),
        "status" => status(ctx, rest, out),
        "help" | "-h" | "--help" => {
            out.write_all(help::BRAIN.as_bytes())?;
            Ok(0)
        }
        other => Err(usage(format!("unknown command nixa brain {other}"))),
    }
}

fn health() -> Option<Value> {
    http::get_within("/health", PING).ok().filter(Value::is_object)
}

fn brain_bin() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot find the nixa command path")?;
    let dir = exe.parent().ok_or_else(|| anyhow!("nixa has no folder"))?;
    let bin = dir.join(BRAIN_BIN);
    if bin.is_file() {
        return Ok(bin);
    }
    bail!(
        "cannot find {BRAIN_BIN} next to {}. Build it with: CARGO_TARGET_DIR=target/gpui cargo build -p nixa-brain, or pass --bin PATH",
        exe.display()
    )
}

fn pid_file() -> PathBuf {
    paths::data_dir().join(PID_FILE)
}

fn read_pid() -> Option<(u32, String)> {
    let text = fs::read_to_string(pid_file()).ok()?;
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    Some((pid, lines.next().unwrap_or("").to_string()))
}

fn command_of(pid: u32) -> Option<String> {
    let out = Command::new("/bin/ps").args(["-o", "command=", "-p", &pid.to_string()]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn listener_pid(addr: &str) -> Option<u32> {
    let port = addr.rsplit(':').next()?;
    let out = Command::new("/usr/sbin/lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<u32>().ok())
        .find(|pid| command_of(*pid).is_some_and(|c| c.contains(BRAIN_BIN)))
}

fn known_pid(addr: &str) -> Option<u32> {
    let from_file = read_pid().filter(|(pid, bin)| !bin.is_empty() && command_of(*pid).is_some_and(|c| c.contains(bin.as_str())));
    from_file.map(|(pid, _)| pid).or_else(|| listener_pid(addr))
}

fn start(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--bin", "--wait"], false)?;
    let bin = p.value(&["--bin"]).map(PathBuf::from);
    let wait = p.value(&["--wait"]).map(|v| args::duration_ms(&v)).transpose()?;
    p.finish(0, false)?;
    let addr = http::brain_addr()?;
    if let Some(h) = health() {
        return report(&ctx, out, "already running", &addr, known_pid(&addr), &h);
    }
    let bin = match bin {
        Some(b) => b,
        None => brain_bin()?,
    };
    let dir = paths::ensure_data_dir().context("cannot make the data folder")?;
    let log = dir.join(LOG_FILE);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log)
        .with_context(|| format!("cannot open {}", log.display()))?;
    fs::set_permissions(&log, fs::Permissions::from_mode(0o600))?;
    let mut cmd = Command::new(&bin);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(file.try_clone()?))
        .stderr(Stdio::from(file))
        .env(paths::ENV_BRAIN_ADDR, &addr)
        .process_group(0);
    if std::env::var_os(paths::ENV_NIXA_BIN).is_none()
        && let Ok(exe) = std::env::current_exe()
    {
        cmd.env(paths::ENV_NIXA_BIN, exe);
    }
    let mut child = cmd.spawn().with_context(|| format!("cannot start {}", bin.display()))?;
    let pid = child.id();
    let pid_path = dir.join(PID_FILE);
    OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&pid_path)?.write_all(format!("{pid}\n{}\n", bin.display()).as_bytes())?;
    fs::set_permissions(&pid_path, fs::Permissions::from_mode(0o600))?;
    let deadline = Instant::now() + Duration::from_millis(wait.unwrap_or(15_000));
    while Instant::now() < deadline {
        if let Some(code) = child.try_wait()? {
            let _ = fs::remove_file(&pid_path);
            bail!(
                "{} exited early ({code}). Last lines of {}:\n{}",
                bin.display(),
                log.display(),
                engine::log_tail(&log, 20)
            );
        }
        if let Some(h) = health() {
            return report(&ctx, out, "started", &addr, Some(pid), &h);
        }
        sleep(POLL);
    }
    bail!(
        "nixa-brain pid {pid} did not answer at {addr} in time. Stop it with: nixa brain stop. Log: {}",
        log.display()
    )
}

fn line(what: &str, addr: &str, pid: Option<u32>, h: &Value) -> String {
    let mut text = format!("nixa-brain {what} at {addr}");
    if let Some(pid) = pid {
        text.push_str(&format!(", pid {pid}"));
    }
    if let Some(v) = h.get("version").and_then(Value::as_str) {
        text.push_str(&format!(", version {v}"));
    }
    if let Some(b) = h.get("brain").filter(|b| b.is_object()) {
        let name = b.get("name").and_then(Value::as_str).unwrap_or("?");
        let model = b.get("model").and_then(Value::as_str).filter(|m| !m.is_empty());
        match model {
            Some(m) => text.push_str(&format!(", brain {name} ({m})")),
            None => text.push_str(&format!(", brain {name}")),
        }
    }
    match h.get("engine").and_then(Value::as_bool) {
        Some(true) => text.push_str(", engine up"),
        Some(false) => text.push_str(", engine down"),
        None => {}
    }
    text
}

fn report(ctx: &Ctx, out: &mut dyn Write, what: &str, addr: &str, pid: Option<u32>, h: &Value) -> Result<i32> {
    if ctx.json {
        writeln!(out, "{}", json!({"running": true, "addr": addr, "pid": pid, "health": h}))?;
    } else {
        writeln!(out, "{}", line(what, addr, pid, h))?;
    }
    Ok(0)
}

fn stop(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, _ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let addr = http::brain_addr()?;
    if health().is_none() {
        let _ = fs::remove_file(pid_file());
        writeln!(out, "nixa-brain is not running at {addr}")?;
        return Ok(0);
    }
    let pid = known_pid(&addr).ok_or_else(|| anyhow!("nixa-brain answers at {addr}, but no nixa-brain process was found for it"))?;
    let sent = Command::new("/bin/kill").args(["-TERM", &pid.to_string()]).status()?;
    if !sent.success() {
        bail!("cannot stop nixa-brain pid {pid}");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if health().is_none() && command_of(pid).is_none() {
            let _ = fs::remove_file(pid_file());
            writeln!(out, "nixa-brain stopped, pid {pid}")?;
            return Ok(0);
        }
        sleep(POLL);
    }
    bail!("nixa-brain pid {pid} still runs after stop")
}

fn status(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let addr = http::brain_addr()?;
    let Some(h) = health() else {
        if ctx.json {
            writeln!(out, "{}", json!({"running": false, "addr": addr}))?;
        } else {
            writeln!(out, "nixa-brain is not running at {addr}")?;
        }
        return Ok(1);
    };
    report(&ctx, out, "running", &addr, known_pid(&addr), &h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_line_names_the_parts() {
        let h = json!({"version": "0.1.0", "brain": {"name": "claude", "model": "opus"}, "engine": true});
        assert_eq!(
            line("running", "127.0.0.1:7421", Some(7), &h),
            "nixa-brain running at 127.0.0.1:7421, pid 7, version 0.1.0, brain claude (opus), engine up"
        );
        assert_eq!(line("started", "a:1", None, &json!({"brain": null})), "nixa-brain started at a:1");
    }

    #[test]
    fn own_process_is_seen() {
        let me = std::process::id();
        assert!(command_of(me).is_some());
        assert!(command_of(u32::MAX - 1).is_none());
    }
}
