use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use crate::help;
use anyhow::{Context, Result, anyhow, bail};
use nixa_protocol::{PROTOCOL_VERSION, Reply, Request, connect, paths};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const ENGINE_BIN: &str = "nixa-engine";
const LOG_FILE: &str = "engine.log";
const POLL: Duration = Duration::from_millis(50);

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::ENGINE.as_bytes())?;
        return Ok(2);
    };
    match sub.as_str() {
        "start" => start(ctx, rest, out),
        "stop" => stop(ctx, rest, out),
        "status" => status(ctx, rest, out),
        "restart" => restart(ctx, rest, out),
        "help" | "-h" | "--help" => {
            out.write_all(help::ENGINE.as_bytes())?;
            Ok(0)
        }
        other => Err(usage(format!("unknown command nixa engine {other}"))),
    }
}

fn ping(socket: &Path) -> Option<(u32, u32)> {
    let mut client = connect(socket).ok()?;
    client.set_timeout(Some(Duration::from_secs(2))).ok()?;
    match client.call(&Request::Ping).ok()? {
        Reply::Pong { version, pid } => Some((version, pid)),
        _ => None,
    }
}

fn engine_bin() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot find the nixa command path")?;
    let dir = exe.parent().ok_or_else(|| anyhow!("nixa has no folder"))?;
    let bin = dir.join(ENGINE_BIN);
    if bin.is_file() {
        return Ok(bin);
    }
    bail!(
        "cannot find {ENGINE_BIN} next to {}. Build it with: CARGO_TARGET_DIR=target/gpui cargo build -p nixa-engine, or pass --bin PATH",
        exe.display()
    )
}

fn log_path(socket: &Path) -> Result<PathBuf> {
    let dir = socket
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(paths::data_dir);
    paths::ensure_private_dir(&dir).with_context(|| format!("cannot make {}", dir.display()))?;
    Ok(dir.join(LOG_FILE))
}

pub fn log_tail(log: &Path, lines: usize) -> String {
    let text = fs::read_to_string(log).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn start(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--bin", "--wait"], false)?;
    let bin = p.value(&["--bin"]).map(PathBuf::from);
    let wait = p.value(&["--wait"]).map(|v| args::duration_ms(&v)).transpose()?;
    p.finish(0, false)?;
    if let Some((_, pid)) = ping(&ctx.socket) {
        return report(&ctx, out, "already running", pid);
    }
    let bin = match bin {
        Some(b) => b,
        None => engine_bin()?,
    };
    let log = log_path(&ctx.socket)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log)
        .with_context(|| format!("cannot open {}", log.display()))?;
    fs::set_permissions(&log, fs::Permissions::from_mode(0o600))?;
    let mut child = Command::new(&bin)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file.try_clone()?))
        .stderr(Stdio::from(file))
        .env(paths::ENV_ENGINE_SOCKET, &ctx.socket)
        .process_group(0)
        .spawn()
        .with_context(|| format!("cannot start {}", bin.display()))?;
    let deadline = Instant::now() + Duration::from_millis(wait.unwrap_or(10_000));
    while Instant::now() < deadline {
        if let Some((_, pid)) = ping(&ctx.socket) {
            return report(&ctx, out, "started", pid);
        }
        if let Some(code) = child.try_wait()? {
            bail!(
                "{} exited early ({code}). Last lines of {}:\n{}",
                bin.display(),
                log.display(),
                log_tail(&log, 20)
            );
        }
        sleep(POLL);
    }
    bail!(
        "nixa-engine did not open {} in time. Log: {}",
        ctx.socket.display(),
        log.display()
    )
}

fn report(ctx: &Ctx, out: &mut dyn Write, what: &str, pid: u32) -> Result<i32> {
    if ctx.json {
        let v = serde_json::json!({"running": true, "pid": pid, "socket": ctx.socket});
        writeln!(out, "{v}")?;
    } else {
        writeln!(out, "nixa-engine {what}, pid {pid}, socket {}", ctx.socket.display())?;
    }
    Ok(0)
}

fn stop(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    match shut_down(&ctx)? {
        Some(pid) => writeln!(out, "nixa-engine stopped, pid {pid}")?,
        None => writeln!(out, "nixa-engine is not running")?,
    }
    Ok(0)
}

fn shut_down(ctx: &Ctx) -> Result<Option<u32>> {
    let Some((_, pid)) = ping(&ctx.socket) else {
        return Ok(None);
    };
    let mut client = ctx::connect(ctx, Some(Duration::from_secs(5)))?;
    let _ = client.request(&Request::Shutdown);
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ping(&ctx.socket).is_none() {
            return Ok(Some(pid));
        }
        sleep(POLL);
    }
    bail!("nixa-engine pid {pid} still answers after shutdown")
}

fn restart(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--bin", "--wait"], false)?;
    let mut start_args = Vec::new();
    for name in ["--bin", "--wait"] {
        if let Some(v) = p.value(&[name]) {
            start_args.push(name.to_string());
            start_args.push(v);
        }
    }
    p.finish(0, false)?;
    let quiet = Ctx { json: false, ..ctx.clone() };
    match shut_down(&ctx)? {
        Some(pid) if !ctx.json => writeln!(out, "nixa-engine stopped, pid {pid}")?,
        _ => {}
    }
    let mut sink = Vec::new();
    start(&quiet, &start_args, &mut sink)?;
    status(&ctx, &[], out)
}

fn status(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let Some((version, pid)) = ping(&ctx.socket) else {
        if ctx.json {
            let v = serde_json::json!({"running": false, "socket": ctx.socket});
            writeln!(out, "{v}")?;
        } else {
            writeln!(out, "nixa-engine is not running (socket {})", ctx.socket.display())?;
        }
        return Ok(1);
    };
    let terms = match ctx::call(&ctx, &Request::List { parent: None, label: None }) {
        Ok(Reply::Terms { terms }) => Some(terms.len()),
        _ => None,
    };
    if ctx.json {
        let v = serde_json::json!({
            "running": true,
            "pid": pid,
            "version": version,
            "socket": ctx.socket,
            "terms": terms,
        });
        writeln!(out, "{v}")?;
    } else {
        let count = terms.map(|n| format!(", {n} terminals")).unwrap_or_default();
        writeln!(
            out,
            "nixa-engine running, pid {pid}, protocol {version}{count}, socket {}",
            ctx.socket.display()
        )?;
    }
    if version != PROTOCOL_VERSION {
        eprintln!("nixa: engine protocol {version}, this nixa speaks {PROTOCOL_VERSION}");
    }
    Ok(0)
}
