use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use crate::help;
use crate::term::term_id;
use anyhow::Result;
use nixa_protocol::{Reply, Request};
use serde_json::json;
use std::io::{self, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

pub const EXIT_TIMEOUT: i32 = 124;
const TICK: Duration = Duration::from_millis(20);
const GRACE: Duration = Duration::from_millis(300);

#[derive(Debug, Default, PartialEq)]
pub struct Spec {
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub stdin: bool,
    pub timeout_ms: Option<u64>,
    pub term: Option<String>,
}

enum Chunk {
    Out(Vec<u8>),
    Err(Vec<u8>),
}

#[derive(Default)]
struct Outcome {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    if argv.first().is_some_and(|a| matches!(a.as_str(), "help" | "-h" | "--help")) {
        out.write_all(help::EXEC.as_bytes())?;
        return Ok(0);
    }
    let (spec, ctx) = parse(ctx, argv)?;
    let base = match &spec.term {
        Some(id) => Some(term_cwd(&ctx, id)?),
        None => None,
    };
    let dir = folder(base.as_deref(), spec.cwd.as_deref())?;
    let started = Instant::now();
    let outcome = execute(&spec, dir.as_deref(), !ctx.json, out)?;
    if ctx.json {
        let line = json!({
            "code": outcome.code,
            "stdout": String::from_utf8_lossy(&outcome.stdout),
            "stderr": String::from_utf8_lossy(&outcome.stderr),
            "ms": started.elapsed().as_millis() as u64,
            "timed_out": outcome.timed_out,
        });
        writeln!(out, "{line}")?;
    } else if outcome.timed_out {
        let ms = spec.timeout_ms.unwrap_or_default();
        let time = if ms % 1000 == 0 { format!("{}s", ms / 1000) } else { format!("{ms}ms") };
        eprintln!("nixa exec: stopped {:?} after {time} (timeout), exit {EXIT_TIMEOUT}", spec.cmd[0]);
    }
    Ok(outcome.code)
}

pub fn parse(ctx: &Ctx, argv: &[String]) -> Result<(Spec, Ctx)> {
    let with_value = ["--cwd", "-C", "--env", "-e", "--in", "--timeout"];
    let (mut p, ctx) = ctx::command(ctx, argv, &with_value, true)?;
    let stdin = p.flag(&["-i", "--stdin"]);
    let cwd = p.value(&["--cwd", "-C"]);
    let env = p
        .values(&["--env", "-e"])
        .into_iter()
        .map(|item| match item.split_once('=') {
            Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
            _ => Err(usage(format!("--env needs KEY=VALUE, got {item:?}"))),
        })
        .collect::<Result<Vec<_>>>()?;
    let term = p.value(&["--in"]);
    let timeout_ms = p.value(&["--timeout"]).map(|v| args::duration_ms(&v)).transpose()?;
    p.finish(0, true)?;
    if p.rest.is_empty() {
        return Err(usage("nixa exec needs a command, for example: nixa exec -- ls -la"));
    }
    let spec = Spec {
        cmd: p.rest,
        cwd,
        env,
        stdin,
        timeout_ms,
        term,
    };
    Ok((spec, ctx))
}

pub fn term_cwd(ctx: &Ctx, id: &str) -> Result<PathBuf> {
    let id = term_id(id)?;
    match ctx::call(ctx, &Request::Get { id })? {
        Reply::Term { term } => Ok(PathBuf::from(term.cwd)),
        other => Err(ctx::unexpected(&other)),
    }
}

pub fn folder(base: Option<&Path>, cwd: Option<&str>) -> Result<Option<PathBuf>> {
    let dir = match (base, cwd) {
        (Some(b), Some(c)) => Some(b.join(c)),
        (Some(b), None) => Some(b.to_path_buf()),
        (None, Some(c)) => Some(PathBuf::from(c)),
        (None, None) => None,
    };
    if let Some(d) = &dir
        && !d.is_dir()
    {
        return Err(anyhow::anyhow!("folder {} does not exist", d.display()));
    }
    Ok(dir)
}

pub fn exit_code(status: ExitStatus) -> i32 {
    match (status.code(), status.signal()) {
        (Some(code), _) => code,
        (None, Some(sig)) => 128 + sig,
        (None, None) => 1,
    }
}

fn start_code(err: &io::Error) -> i32 {
    match err.kind() {
        io::ErrorKind::NotFound => 127,
        _ => 126,
    }
}

fn execute(spec: &Spec, dir: Option<&Path>, stream: bool, out: &mut dyn Write) -> Result<Outcome> {
    let mut cmd = Command::new(&spec.cmd[0]);
    cmd.args(&spec.cmd[1..])
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(if spec.stdin { Stdio::inherit() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let code = start_code(&e);
            let msg = format!("nixa exec: cannot start {:?}: {e}\n", spec.cmd[0]);
            if !stream {
                return Ok(Outcome { code, stderr: msg.into_bytes(), ..Outcome::default() });
            }
            eprint!("{msg}");
            return Ok(Outcome { code, ..Outcome::default() });
        }
    };
    let (tx, rx) = mpsc::channel();
    pipe(child.stdout.take(), tx.clone(), Chunk::Out);
    pipe(child.stderr.take(), tx, Chunk::Err);
    let deadline = spec.timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    collect(&mut child, &rx, deadline, stream, out)
}

fn pipe<R: Read + Send + 'static>(src: Option<R>, tx: Sender<Chunk>, wrap: fn(Vec<u8>) -> Chunk) {
    let Some(mut src) = src else { return };
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match src.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(wrap(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
}

fn collect(
    child: &mut Child,
    rx: &Receiver<Chunk>,
    deadline: Option<Instant>,
    stream: bool,
    out: &mut dyn Write,
) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    let mut status = None;
    let mut pipes_open = true;
    let mut stop_at = None;
    loop {
        if pipes_open {
            match rx.recv_timeout(TICK) {
                Ok(chunk) => take(chunk, &mut outcome, stream, out)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => pipes_open = false,
            }
        } else if status.is_none() {
            thread::sleep(TICK);
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
        if status.is_none() && deadline.is_some_and(|d| Instant::now() >= d) {
            let _ = child.kill();
            status = Some(child.wait()?);
            outcome.timed_out = true;
            stop_at = Some(Instant::now() + GRACE);
        }
        let grace_over = stop_at.is_some_and(|t| Instant::now() >= t);
        if status.is_some() && (!pipes_open || grace_over) {
            break;
        }
    }
    while let Ok(chunk) = rx.try_recv() {
        take(chunk, &mut outcome, stream, out)?;
    }
    outcome.code = match (outcome.timed_out, status) {
        (true, _) => EXIT_TIMEOUT,
        (false, Some(s)) => exit_code(s),
        (false, None) => 1,
    };
    Ok(outcome)
}

fn take(chunk: Chunk, outcome: &mut Outcome, stream: bool, out: &mut dyn Write) -> Result<()> {
    match (chunk, stream) {
        (Chunk::Out(b), true) => {
            out.write_all(&b)?;
            out.flush()?;
        }
        (Chunk::Err(b), true) => {
            let mut err = io::stderr();
            err.write_all(&b)?;
            err.flush()?;
        }
        (Chunk::Out(b), false) => outcome.stdout.extend_from_slice(&b),
        (Chunk::Err(b), false) => outcome.stderr.extend_from_slice(&b),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn ctx() -> Ctx {
        Ctx { socket: PathBuf::from("/nonexistent.sock"), json: false }
    }

    fn spec(cmd: &[&str], timeout_ms: Option<u64>) -> Spec {
        Spec { cmd: strs(cmd), timeout_ms, ..Spec::default() }
    }

    #[test]
    fn parse_options_and_command() {
        let argv = strs(&["-i", "--cwd", "/tmp", "-e", "A=1", "--timeout", "2s", "--json", "ls", "--json", "-l"]);
        let (s, c) = parse(&ctx(), &argv).unwrap();
        assert!(s.stdin && c.json);
        assert_eq!(s.cwd.as_deref(), Some("/tmp"));
        assert_eq!(s.env, vec![("A".to_string(), "1".to_string())]);
        assert_eq!(s.timeout_ms, Some(2000));
        assert_eq!(s.cmd, strs(&["ls", "--json", "-l"]));
        let (s, _) = parse(&ctx(), &strs(&["--in", ".", "--", "-x"])).unwrap();
        assert_eq!(s.term.as_deref(), Some("."));
        assert_eq!(s.cmd, strs(&["-x"]));
        assert!(parse(&ctx(), &strs(&["--json"])).is_err());
        assert!(parse(&ctx(), &strs(&["-e", "NOVALUE", "ls"])).is_err());
        assert!(parse(&ctx(), &strs(&["--zap", "ls"])).is_err());
    }

    #[test]
    fn exit_codes_and_output() {
        let mut out = Vec::new();
        let o = execute(&spec(&["sh", "-c", "printf out; printf err >&2; exit 5"], None), None, false, &mut out).unwrap();
        assert_eq!((o.code, o.stdout.as_slice(), o.stderr.as_slice()), (5, &b"out"[..], &b"err"[..]));
        let o = execute(&spec(&["sh", "-c", "kill -9 $$"], None), None, false, &mut out).unwrap();
        assert_eq!(o.code, 137);
        let o = execute(&spec(&["no-such-program-xyz"], None), None, false, &mut out).unwrap();
        assert_eq!(o.code, 127);
        let o = execute(&spec(&["sh", "-c", "printf a; exit 0"], None), None, true, &mut out).unwrap();
        assert_eq!((o.code, out.as_slice()), (0, &b"a"[..]));
    }

    #[test]
    fn timeout_and_null_stdin() {
        let mut out = Vec::new();
        let start = Instant::now();
        let o = execute(&spec(&["sleep", "5"], Some(200)), None, false, &mut out).unwrap();
        assert!(o.timed_out && o.code == EXIT_TIMEOUT);
        assert!(start.elapsed() < Duration::from_secs(3));
        let o = execute(&spec(&["cat"], Some(3000)), None, false, &mut out).unwrap();
        assert!(!o.timed_out && o.code == 0);
    }

    #[test]
    fn working_folder() {
        let tmp = std::env::temp_dir().join(format!("nixa-exec-test-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        let dir = folder(Some(&tmp), Some("sub")).unwrap().unwrap();
        let mut out = Vec::new();
        let o = execute(&spec(&["pwd"], None), Some(&dir), false, &mut out).unwrap();
        let got = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        assert_eq!(got.canonicalize().unwrap(), dir.canonicalize().unwrap());
        assert!(folder(Some(&tmp), Some("missing")).is_err());
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
