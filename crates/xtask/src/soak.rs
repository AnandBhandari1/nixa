use nixa_protocol::TermId;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

const REFUSED_PORTS: [u16; 3] = [7321, 7330, 7331];
const DEFAULT_PORT: u16 = 7465;
const FRESH_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
const DATA_PREFIX: &str = "/tmp/nixa-soak-";
const PARTS: [&str; 3] = ["nixa-app", "nixa-engine", "nixa-brain"];
const APP: usize = 0;
const ENGINE: usize = 1;
const BRAIN: usize = 2;
const TICK: Duration = Duration::from_millis(500);
const ACT_EVERY: Duration = Duration::from_secs(5);
const REPORT_EVERY: Duration = Duration::from_secs(300);
const APP_RESTART_GAP: Duration = Duration::from_secs(30);
const WARM_UP_MAX_SECS: f64 = 300.0;
const TREND_MIN_SECS: f64 = 1800.0;
const KEPT_ENV: [&str; 7] = [
    "USER",
    "LOGNAME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "__CF_USER_TEXT_ENCODING",
];
const AGENT_BIN_ENV: [&str; 5] = [
    "NIXA_CODEX_BIN",
    "NIXA_OPENCODE_BIN",
    "NIXA_AGY_BIN",
    "NIXA_CURSOR_BIN",
    "NIXA_KIMI_BIN",
];
const NO_AGENT: &str = "#!/bin/sh\necho \"agent programs are off in a soak run\" >&2\nexit 1\n";
const MANIFEST: &str = "soak-run";
const SPACE_ID: &str = "soak";
const AGENTS_PER_TAB: u32 = 3;
const CALL_LIMIT: Duration = Duration::from_secs(10);
const START_LIMIT: Duration = Duration::from_secs(60);
const TOOL_LIMIT: Duration = Duration::from_secs(15);
const STOP_WAIT: Duration = Duration::from_secs(10);
const MIN_SAMPLES: usize = 4;
const DRAIN_LIMIT: Duration = Duration::from_secs(2);
const ENGINE_WAIT: Duration = Duration::from_secs(15);

const HELP: &str = "xtask soak: run Nixa with fake agents for hours and record memory, CPU and crashes

Usage:
  cargo run -p xtask -- soak [--for 4h] [--agents 6] [--sample 30s] [--churn 10m]
                             [--port 7465] [--bin-dir DIR] [--out DIR] [--no-window] [--keep-data]
  cargo run -p xtask -- soak stop --data /tmp/nixa-soak-PID [--bin-dir DIR]

  --for TIME      how long to run (default 4h). TIME: 90s, 30m, 4h
  --agents N      fake agent terminals (default 6, at most 64)
  --sample TIME   time between memory and CPU samples (default 30s)
  --churn TIME    replace the oldest fake agent this often (default 10m, 0 turns it off)
  --port N        brain port of the run (default 7465; 7321, 7330 and 7331 are refused)
  --bin-dir DIR   folder with nixa-app, nixa-engine, nixa-brain and nixa (default target/gpui/debug)
  --out DIR       report folder (default target/soak/<date>-<time>)
  --no-window     run the engine and the brain only, without nixa-app
  --keep-data     keep the run data folder in /tmp after the run

The run uses its own data folder in /tmp and its own brain port. It never uses the dev data folder.
Build first: CARGO_TARGET_DIR=target/gpui cargo build -p nixa-app -p nixa-engine -p nixa-brain -p nixa-cli
Stop early: create the file named stop in the report folder.
After a killed run: cargo run -p xtask -- soak stop --data <data folder named in the report>
The report folder holds report.md, samples.csv, events.log, engine.log, brain.log and app.log.
";

const FAKE_AGENT: &str = r#"#!/bin/bash
id="${1:-0}"
long="0123456789abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyz0123456789"
bar="===================="
osc() { printf '\033]7421;status=%s;note=%s\007' "$1" "$2"; }
n=0
while :; do
  n=$((n + 1))
  osc working "fake agent $id step $n"
  lines=$((20 + RANDOM % 180))
  for ((i = 1; i <= lines; i++)); do
    case $((RANDOM % 6)) in
      0) printf '\033[32mok\033[0m step %d.%d done in %dms\n' "$n" "$i" $((RANDOM % 900)) ;;
      1) printf '\033[33mwarn\033[0m src/part_%d.rs:%d: value is never read\n' $((RANDOM % 50)) $((RANDOM % 400)) ;;
      2) printf '%s\n' "${long:0:$((RANDOM % 200))}" ;;
      3) for p in 10 40 70 100; do printf '\r[%-20s] %3d%%' "${bar:0:$((p / 5))}" "$p"; sleep 0.02; done; printf '\n' ;;
      4) printf '\033[1;34mreading\033[0m src/part_%d.rs (%d lines)\n' $((RANDOM % 50)) $((RANDOM % 2000)) ;;
      *) printf 'thinking about line %d of %d\n' "$i" "$lines" ;;
    esac
    if ((i % 5 == 0)); then sleep 0.05; fi
  done
  if ((RANDOM % 8 == 0)); then
    osc working "fake agent $id big output"
    for ((i = 1; i <= 3000; i++)); do printf 'burst line %d %s\n' "$i" "${long:0:80}"; done
  fi
  if ((RANDOM % 4 == 0)); then
    osc blocked "fake agent $id asks a question"
    printf 'Fake agent %s: go on with step %d? (y/n) ' "$id" "$n"
    answer=""
    read -r -t 30 answer
    printf '\nanswer: %s\n' "${answer:-none}"
  fi
  osc done "fake agent $id finished step $n"
  sleep $((2 + RANDOM % 8))
  osc idle "fake agent $id waits"
done
"#;

#[derive(Debug, Clone, PartialEq)]
struct Opts {
    run_for: Duration,
    agents: usize,
    sample: Duration,
    churn: Option<Duration>,
    port: u16,
    bin_dir: Option<PathBuf>,
    out: Option<PathBuf>,
    window: bool,
    keep_data: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            run_for: Duration::from_secs(4 * 3600),
            agents: 6,
            sample: Duration::from_secs(30),
            churn: Some(Duration::from_secs(600)),
            port: DEFAULT_PORT,
            bin_dir: None,
            out: None,
            window: true,
            keep_data: false,
        }
    }
}

#[derive(Debug, Clone)]
struct Bins {
    app: PathBuf,
    engine: PathBuf,
    brain: PathBuf,
    nixa: PathBuf,
}

#[derive(Debug, Clone)]
struct Proc {
    pid: u32,
    ppid: u32,
    start: String,
    exe: String,
    args: String,
}

struct Run {
    bins: Bins,
    data: PathBuf,
    out: PathBuf,
    port: u16,
    window: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Row {
    t: f64,
    part: usize,
    pid: u32,
    rss_kb: u64,
    cpu_pct: Option<f64>,
    threads: u32,
    fds: u32,
}

#[derive(Debug, Clone, Default)]
struct Watch {
    pid: Option<u32>,
    seen: bool,
    cpu: Option<(f64, Instant)>,
    stops: u32,
    hangs: u32,
    hung: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Calls {
    count: u32,
    errors: u32,
    ok_ms: f64,
    max_ms: f64,
}

#[derive(Debug, Default)]
struct State {
    started: Option<Instant>,
    started_text: String,
    rows: Vec<Row>,
    events: Vec<(f64, String)>,
    watch: [Watch; 3],
    agents: Vec<String>,
    made: u32,
    churned: u32,
    turn: usize,
    app: Option<Child>,
    app_started: Option<Instant>,
    app_exits: Vec<String>,
    app_log_at: u64,
    closed_outside: Vec<String>,
    engine_calls: Calls,
    brain_calls: Calls,
    lost_agents: Vec<String>,
    other_terms: Vec<String>,
    other_clients: Vec<String>,
    window_calls: Calls,
    agent_fails: u32,
}

#[derive(Debug, Clone, PartialEq)]
struct Stats {
    samples: usize,
    pid: u32,
    processes: usize,
    after: usize,
    first_mb: f64,
    warm_mb: f64,
    end_mb: f64,
    last_mb: f64,
    peak_mb: f64,
    slope_mb_h: Option<f64>,
    avg_cpu: Option<f64>,
    peak_cpu: Option<f64>,
    threads: (u32, u32),
    fds: (u32, u32),
    fds_warm: u32,
}

#[derive(Debug, Clone, Default)]
struct Ending {
    ran: Duration,
    early: bool,
    stopped_by: String,
    error: Option<String>,
    leftovers: Vec<String>,
    alive: Vec<String>,
    panics: Vec<String>,
    agent_starts: Vec<String>,
    crash_reports: Vec<String>,
    data_note: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Verdict {
    Pass,
    Check,
    Early,
}

pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("help" | "-h" | "--help") => {
            print!("{HELP}");
            Ok(())
        }
        Some("stop") => stop_command(&args[1..]),
        _ => soak(parse_opts(args)?),
    }
}

fn parse_duration(text: &str) -> Result<Duration, String> {
    let t = text.trim();
    let (num, mult) = if let Some(n) = t.strip_suffix("ms") {
        (n, 1)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1_000)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 60_000)
    } else if let Some(n) = t.strip_suffix('h') {
        (n, 3_600_000)
    } else {
        (t, 1_000)
    };
    let n: u64 = num
        .parse()
        .map_err(|_| format!("bad time {text:?}, use for example 90s, 30m or 4h"))?;
    n.checked_mul(mult)
        .map(Duration::from_millis)
        .ok_or_else(|| format!("time {text:?} is too large"))
}

fn check_port(port: u16) -> Result<(), String> {
    if REFUSED_PORTS.contains(&port) {
        return Err(format!(
            "port {port} is reserved for another local app and is refused, pick another --port"
        ));
    }
    if port < 1024 {
        return Err(format!("port {port} is too low, pick a port from 1024"));
    }
    Ok(())
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = || {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--for" => o.run_for = parse_duration(&value()?)?,
            "--sample" => o.sample = parse_duration(&value()?)?,
            "--churn" => {
                let d = parse_duration(&value()?)?;
                o.churn = (!d.is_zero()).then_some(d);
            }
            "--agents" => {
                o.agents = value()?
                    .parse()
                    .map_err(|_| "--agents needs a number".to_string())?
            }
            "--port" => {
                o.port = value()?
                    .parse()
                    .map_err(|_| "--port needs a port number".to_string())?
            }
            "--bin-dir" => o.bin_dir = Some(PathBuf::from(value()?)),
            "--out" => o.out = Some(PathBuf::from(value()?)),
            "--no-window" => o.window = false,
            "--keep-data" => o.keep_data = true,
            other => {
                return Err(format!(
                    "unknown soak option {other}, see: cargo run -p xtask -- soak help"
                ));
            }
        }
        if !matches!(flag, "--no-window" | "--keep-data") {
            i += 1;
        }
        i += 1;
    }
    if o.agents == 0 || o.agents > 64 {
        return Err("--agents must be from 1 to 64".into());
    }
    if o.sample < Duration::from_secs(1) {
        return Err("--sample must be at least 1s".into());
    }
    if o.run_for < Duration::from_secs(10) {
        return Err("--for must be at least 10s".into());
    }
    check_port(o.port)?;
    Ok(o)
}

fn find_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|d| fs::read_to_string(d.join("Cargo.toml")).is_ok_and(|t| t.contains("[workspace]")))
        .map(Path::to_path_buf)
}

fn find_bins(dir: &Path, window: bool) -> Result<Bins, String> {
    let dir = &fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let bins = Bins {
        app: dir.join("nixa-app"),
        engine: dir.join("nixa-engine"),
        brain: dir.join("nixa-brain"),
        nixa: dir.join("nixa"),
    };
    let mut need = vec![&bins.engine, &bins.brain, &bins.nixa];
    if window {
        need.push(&bins.app);
    }
    let missing: Vec<String> = need
        .iter()
        .filter(|p| !p.is_file())
        .map(|p| p.display().to_string())
        .collect();
    if missing.is_empty() {
        return Ok(bins);
    }
    Err(format!(
        "missing programs: {}. Build them with: CARGO_TARGET_DIR=target/gpui cargo build -p nixa-app -p nixa-engine -p nixa-brain -p nixa-cli, or pass --bin-dir",
        missing.join(", ")
    ))
}

fn is_soak_data(path: &Path) -> bool {
    let text = path.display().to_string();
    text.strip_prefix(DATA_PREFIX).is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

fn clock(cmd: &[&str]) -> String {
    bounded(Command::new(cmd[0]).args(&cmd[1..]), TOOL_LIMIT)
        .map(|(_, out, _)| out.trim().to_string())
        .unwrap_or_default()
}

fn kill_group(pgid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn bounded(cmd: &mut Command, limit: Duration) -> Result<(ExitStatus, String, String), String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel::<(usize, Option<Vec<u8>>)>();
    let pipes: [Option<Box<dyn Read + Send>>; 2] = [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    ];
    for (i, pipe) in pipes.into_iter().enumerate() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            if let Some(mut p) = pipe {
                let mut buf = [0u8; 8192];
                while let Ok(n) = p.read(&mut buf) {
                    if n == 0 || tx.send((i, Some(buf[..n].to_vec()))).is_err() {
                        break;
                    }
                }
            }
            let _ = tx.send((i, None));
        });
    }
    drop(tx);
    let deadline = Instant::now() + limit;
    let mut out = [Vec::new(), Vec::new()];
    let mut open = 2;
    let mut status = None;
    let mut timed_out = false;
    let mut drain_until: Option<Instant> = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(s)) => status = Some(s),
                Ok(None) if Instant::now() < deadline => {}
                _ => {
                    kill_group(child.id());
                    let _ = child.kill();
                    status = child.wait().ok();
                    timed_out = true;
                }
            }
            if status.is_some() {
                drain_until = Some(Instant::now() + DRAIN_LIMIT);
            }
        }
        if status.is_some() && (open == 0 || drain_until.is_some_and(|d| Instant::now() >= d)) {
            break;
        }
        if status.is_none() && timed_out {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok((i, Some(bytes))) => out[i].extend(bytes),
            Ok((_, None)) => open -= 1,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                open = 0;
                sleep(Duration::from_millis(10));
            }
        }
    }
    if timed_out {
        return Err(format!("timed out after {} s", limit.as_secs_f64().ceil()));
    }
    status
        .map(|s| (s, lossy(&out[0]), lossy(&out[1])))
        .ok_or_else(|| "lost the program's exit status".to_string())
}

fn tool(program: &str, args: &[&str]) -> Option<String> {
    match bounded(Command::new(program).args(args), TOOL_LIMIT) {
        Ok((status, out, _)) if status.success() => Some(out),
        _ => None,
    }
}

fn with_env(cmd: &mut Command, run: &Run) {
    cmd.env_clear();
    for key in KEPT_ENV {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    cmd.env("NIXA_DATA_DIR", &run.data)
        .env("NIXA_BRAIN_ADDR", format!("127.0.0.1:{}", run.port))
        .env("NIXA_ENGINE_SOCKET", run.data.join("engine.sock"))
        .env("NIXA_BIN", &run.bins.nixa)
        .env("HOME", run.data.join("home"))
        .env("PATH", FRESH_PATH)
        .env("SHELL", "/bin/zsh")
        .current_dir(&run.data);
    for key in AGENT_BIN_ENV {
        cmd.env(key, run.data.join("no-agent"));
    }
}

fn nixa(run: &Run, args: &[&str]) -> (bool, String) {
    let mut cmd = Command::new(&run.bins.nixa);
    with_env(&mut cmd, run);
    let limit = if args.get(1) == Some(&"start") {
        START_LIMIT
    } else {
        CALL_LIMIT
    };
    match bounded(cmd.args(args), limit) {
        Ok((status, out, err)) => (status.success(), format!("{out}{err}").trim().to_string()),
        Err(e) => (false, format!("nixa {}: {e}", args.join(" "))),
    }
}

fn json_u32(text: &str, key: &str) -> Option<u32> {
    let at = text.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = text[at..].trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn part_pid(run: &Run, part: usize) -> Option<u32> {
    let what = if part == ENGINE { "engine" } else { "brain" };
    let (ok, text) = nixa(run, &["--json", what, "status"]);
    if ok { json_u32(&text, "pid") } else { None }
}

fn silent_pid(owned: &[(u32, String)], part: usize) -> Option<u32> {
    owned
        .iter()
        .find(|(_, p)| p == PARTS[part])
        .map(|(pid, _)| *pid)
}

fn parse_cpu_time(text: &str) -> Option<f64> {
    let (days, rest) = match text.split_once('-') {
        Some((d, r)) => (d.parse::<f64>().ok()?, r),
        None => (0.0, text),
    };
    let mut total = 0.0;
    for part in rest.split(':') {
        total = total * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(days * 86_400.0 + total)
}

fn ps_stat(pid: u32) -> Option<(u64, f64)> {
    let text = tool("/bin/ps", &["-o", "rss=,time=", "-p", &pid.to_string()])?;
    let mut parts = text.split_whitespace();
    let rss = parts.next()?.parse().ok()?;
    let cpu = parse_cpu_time(parts.next()?)?;
    Some((rss, cpu))
}

fn is_alive(pid: u32) -> Option<bool> {
    let pid = pid.to_string();
    match bounded(
        Command::new("/bin/ps").args(["-o", "stat=", "-p", &pid]),
        TOOL_LIMIT,
    ) {
        Ok((_, out, _)) if !out.trim().is_empty() => Some(!out.trim().starts_with('Z')),
        Ok((status, _, err)) if status.code() == Some(1) && err.trim().is_empty() => Some(false),
        _ => None,
    }
}

fn parse_table(comm: &str, args: &str, starts: &str) -> Vec<Proc> {
    let split = |line: &str| -> Option<(u32, String)> {
        let line = line.trim_start();
        let end = line.find(char::is_whitespace)?;
        Some((line[..end].parse().ok()?, line[end..].trim().to_string()))
    };
    let mut table: Vec<Proc> = comm
        .lines()
        .filter_map(|line| {
            let (pid, rest) = split(line)?;
            let (ppid, exe) = split(&rest)?;
            Some(Proc {
                pid,
                ppid,
                start: String::new(),
                exe,
                args: String::new(),
            })
        })
        .collect();
    for (pid, rest) in args.lines().filter_map(split) {
        if let Some(p) = table.iter_mut().find(|p| p.pid == pid) {
            p.args = rest;
        }
    }
    for (pid, rest) in starts.lines().filter_map(split) {
        if let Some(p) = table.iter_mut().find(|p| p.pid == pid) {
            p.start = rest;
        }
    }
    table
}

fn owned_from(
    table: &[Proc],
    roots: &[(u32, String)],
    bins: &[String],
    script: &str,
    me: u32,
) -> Vec<(u32, String, String)> {
    let agent = format!("/bin/bash {script} ");
    let started_by_us = |p: &Proc| {
        !p.start.is_empty()
            && roots
                .iter()
                .any(|(pid, start)| *pid == p.pid && *start == p.start)
    };
    let mut owned: Vec<u32> = table
        .iter()
        .filter(|p| p.pid != me)
        .filter(|p| {
            (started_by_us(p) && bins.contains(&p.exe))
                || format!("{} ", p.args).starts_with(&agent)
        })
        .map(|p| p.pid)
        .collect();
    let mut i = 0;
    while i < owned.len() {
        let parent = owned[i];
        for p in table {
            if p.ppid == parent && p.pid != me && !owned.contains(&p.pid) {
                owned.push(p.pid);
            }
        }
        i += 1;
    }
    owned
        .into_iter()
        .filter_map(|pid| table.iter().find(|p| p.pid == pid))
        .map(|p| {
            (
                p.pid,
                p.exe.rsplit('/').next().unwrap_or(&p.exe).to_string(),
                p.start.clone(),
            )
        })
        .collect()
}

fn owned_pids(run: &Run) -> Result<Vec<(u32, String)>, String> {
    owned_procs(run).map(|list| list.into_iter().map(|(pid, program, _)| (pid, program)).collect())
}

fn owned_procs(run: &Run) -> Result<Vec<(u32, String, String)>, String> {
    let ps = |field: &str| -> Result<String, String> {
        match bounded(
            Command::new("/bin/ps").args(["-A", "-o", field]),
            TOOL_LIMIT,
        )? {
            (status, out, _) if status.success() => Ok(out),
            (_, _, err) => Err(format!("ps failed: {}", err.trim())),
        }
    };
    let mut table = parse_table(
        &ps("pid=,ppid=,comm=")?,
        &ps("pid=,command=")?,
        &ps("pid=,lstart=")?,
    );
    let roots = manifest_pids(&run.data);
    for p in table
        .iter_mut()
        .filter(|p| roots.iter().any(|(pid, _)| *pid == p.pid))
    {
        if let Ok(real) = fs::canonicalize(&p.exe) {
            p.exe = real.display().to_string();
        }
    }
    let mut bins = Vec::new();
    for bin in [&run.bins.app, &run.bins.engine, &run.bins.brain] {
        bins.push(bin.display().to_string());
        if let Ok(real) = fs::canonicalize(bin) {
            bins.push(real.display().to_string());
        }
    }
    let script = run.data.join("fake-agent.sh").display().to_string();
    Ok(owned_from(
        &table,
        &roots,
        &bins,
        &script,
        std::process::id(),
    ))
}

fn same_process(pid: u32, start: &str) -> Option<bool> {
    match is_alive(pid)? {
        false => Some(false),
        true => start_time(pid).map(|now| now == start),
    }
}

fn stop_pids(pids: &[(u32, String)]) -> Vec<u32> {
    let signal = |sig: &str, pids: &[(u32, String)]| {
        for (pid, start) in pids {
            if same_process(*pid, start) == Some(true) {
                let _ = tool("/bin/kill", &[sig, &pid.to_string()]);
            }
        }
    };
    let wait = |pids: &[(u32, String)], limit: Duration| {
        let deadline = Instant::now() + limit;
        loop {
            let left: Vec<(u32, String)> = pids
                .iter()
                .filter(|(pid, start)| same_process(*pid, start) != Some(false))
                .cloned()
                .collect();
            if left.is_empty() || Instant::now() >= deadline {
                return left;
            }
            sleep(Duration::from_millis(200));
        }
    };
    signal("-TERM", pids);
    let left = wait(pids, STOP_WAIT);
    if left.is_empty() {
        return Vec::new();
    }
    signal("-KILL", &left);
    wait(&left, Duration::from_secs(3))
        .into_iter()
        .map(|(p, _)| p)
        .collect()
}

fn thread_count(pid: u32) -> u32 {
    tool("/bin/ps", &["-M", "-p", &pid.to_string()])
        .map(|t| t.lines().count().saturating_sub(1) as u32)
        .unwrap_or(0)
}

fn fd_count(pid: u32) -> u32 {
    tool("/usr/sbin/lsof", &["-nP", "-p", &pid.to_string(), "-Ff"])
        .map(|t| t.lines().filter(|l| l.starts_with('f')).count() as u32)
        .unwrap_or(0)
}

fn http_get(port: u16, path: &str) -> Option<(u16, Duration)> {
    let start = Instant::now();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    let head = lossy(&buf[..buf.len().min(64)]);
    let code = head.split_whitespace().nth(1)?.parse().ok()?;
    Some((code, start.elapsed()))
}

fn elapsed(st: &State) -> f64 {
    st.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0)
}

fn note(run: &Run, st: &mut State, text: String) {
    let t = elapsed(st);
    println!("soak {} {text}", clock_text(t));
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run.out.join("events.log"))
    {
        let _ = writeln!(f, "{} {text}", clock_text(t));
    }
    st.events.push((t, text));
}

fn clock_text(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("+{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn span_text(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}h {:02}m {:02}s", s / 3600, s / 60 % 60, s % 60)
}

fn exit_text(status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit code {code}"),
        (None, Some(sig)) => format!("killed by signal {sig}"),
        _ => "ended".into(),
    }
}

fn track(w: &mut Watch, part: &str, pid: Option<u32>) -> Option<String> {
    let text = match (w.pid, pid) {
        (Some(a), Some(b)) if a != b => {
            w.stops += 1;
            w.cpu = None;
            Some(format!(
                "{part} has a new pid {b} (was {a}): it stopped and started again"
            ))
        }
        (Some(a), None) => {
            w.stops += 1;
            w.cpu = None;
            Some(format!("{part} pid {a} is gone"))
        }
        (None, Some(b)) if w.seen => Some(format!("{part} is back, pid {b}")),
        _ => None,
    };
    if pid.is_some() {
        w.seen = true;
    }
    w.pid = pid;
    text
}

fn write_script(data: &Path) -> Result<PathBuf, String> {
    let path = data.join("fake-agent.sh");
    fs::write(&path, FAKE_AGENT).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    Ok(path)
}

fn start_app(run: &Run) -> Result<Child, String> {
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run.out.join("app.log"))
        .map_err(|e| format!("cannot open app.log: {e}"))?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&run.bins.app);
    with_env(&mut cmd, run);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", run.bins.app.display()))
}

fn add_agent(run: &Run, st: &mut State) {
    st.made += 1;
    let n = st.made.to_string();
    let name = format!("fake-{n}");
    let script = run.data.join("fake-agent.sh").display().to_string();
    let data = run.data.display().to_string();
    let space = format!("space={SPACE_ID}");
    let tab = format!("tab=soak-{}", (st.made - 1) / AGENTS_PER_TAB + 1);
    let mut args = vec![
        "term",
        "new",
        "--no-parent",
        "--name",
        &name,
        "--label",
        "soak=fake",
        "--cwd",
        &data,
    ];
    if run.window {
        args.extend(["--label", &space, "--label", &tab]);
    }
    args.extend(["--", "/bin/bash", &script, &n]);
    let (ok, text) = nixa(run, &args);
    match text.lines().last().map(str::trim) {
        Some(id) if ok && id.parse::<TermId>().is_ok() => {
            st.agents.push(id.to_string());
            note(
                run,
                st,
                format!("fake agent {name} started in terminal {id}"),
            );
        }
        _ => {
            st.agent_fails += 1;
            note(run, st, format!("fake agent {name} did not start: {text}"));
        }
    }
}

fn count_call(calls: &mut Calls, ok: bool, took: Duration) {
    calls.count += 1;
    if !ok {
        calls.errors += 1;
        return;
    }
    let ms = took.as_secs_f64() * 1000.0;
    calls.ok_ms += ms;
    calls.max_ms = calls.max_ms.max(ms);
}

fn timed(run: &Run, calls: &mut Calls, args: &[&str]) {
    let start = Instant::now();
    let (ok, _) = nixa(run, args);
    count_call(calls, ok, start.elapsed());
}

fn window_focus(run: &Run, terminal: &str) -> bool {
    let Ok(mut s) = UnixStream::connect(run.data.join("app.sock")) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
    if writeln!(s, "{{\"op\":\"focus\",\"terminal\":\"{terminal}\"}}").is_err() {
        return false;
    }
    let mut line = String::new();
    let _ = BufReader::new(s).read_line(&mut line);
    line.contains("\"reply\":\"ran\"")
}

fn act(run: &Run, st: &mut State) {
    if !st.agents.is_empty() {
        let id = st.agents[st.turn % st.agents.len()].clone();
        st.turn += 1;
        let mut calls = std::mem::take(&mut st.engine_calls);
        timed(
            run,
            &mut calls,
            &["term", "send", &id, "y", "--submit", "--timeout", "5s"],
        );
        timed(run, &mut calls, &["term", "read", &id, "--lines", "40"]);
        timed(run, &mut calls, &["term", "ls"]);
        st.engine_calls = calls;
        if run.window && st.app.is_some() {
            let start = Instant::now();
            let ok = window_focus(run, &id);
            count_call(&mut st.window_calls, ok, start.elapsed());
        }
    }
    match http_get(run.port, "/health") {
        Some((200, took)) => count_call(&mut st.brain_calls, true, took),
        _ => count_call(&mut st.brain_calls, false, Duration::ZERO),
    }
}

fn churn(run: &Run, st: &mut State) {
    if st.agents.is_empty() {
        return;
    }
    let old = st.agents.remove(0);
    let (ok, text) = nixa(run, &["term", "rm", &old]);
    let said = if ok {
        format!("fake agent terminal {old} removed")
    } else {
        format!("removing fake agent terminal {old} failed: {text}")
    };
    note(run, st, said);
    st.churned += 1;
    add_agent(run, st);
}

fn log_len(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn quit_reason(log_tail: &str, status: ExitStatus) -> Option<String> {
    if log_tail.contains("quit was asked from outside the app") {
        return Some(
            "a quit came from outside the app (Dock menu, a quit Apple Event or log out)".into(),
        );
    }
    if log_tail.contains("the main window was closed") {
        return Some("the window was closed with its close button or a close key".into());
    }
    match status.signal() {
        Some(15) => Some("another program sent TERM (for example Force Quit or kill)".into()),
        Some(9) => Some("another program sent KILL (for example Force Quit or kill -9)".into()),
        _ => None,
    }
}

fn app_log_since(run: &Run, at: u64) -> String {
    let bytes = fs::read(run.out.join("app.log")).unwrap_or_default();
    let from = (at as usize).min(bytes.len());
    lossy(&bytes[from..])
}

fn launch_app(run: &Run, st: &mut State) -> Result<u32, String> {
    st.app_log_at = log_len(&run.out.join("app.log"));
    let child = start_app(run)?;
    let pid = child.id();
    record_pid(run, Some(pid));
    st.app = Some(child);
    st.app_started = Some(Instant::now());
    Ok(pid)
}

fn check_app(run: &Run, st: &mut State) {
    let ended = st.app.as_mut().and_then(|c| c.try_wait().ok().flatten());
    if let Some(status) = ended {
        let text = exit_text(status);
        st.app = None;
        match quit_reason(&app_log_since(run, st.app_log_at), status) {
            Some(why) => {
                st.closed_outside.push(format!("{} {text}: {why}", clock_text(elapsed(st))));
                note(
                    run,
                    st,
                    format!("nixa-app was closed from outside ({text}): {why}. The soak does not start it again, so the window does not come to the front again"),
                );
            }
            None => {
                st.app_exits.push(text.clone());
                note(run, st, format!("nixa-app ended by itself: {text}"));
            }
        }
    }
    let gap_over = st
        .app_started
        .is_some_and(|at| at.elapsed() >= APP_RESTART_GAP);
    if st.app.is_none() && gap_over && st.closed_outside.is_empty() {
        match launch_app(run, st) {
            Ok(pid) => note(run, st, format!("nixa-app started again, pid {pid}")),
            Err(e) => {
                st.app_started = Some(Instant::now());
                note(run, st, e)
            }
        }
    }
}

fn window_text(st: &State) -> String {
    match (&st.app, st.app_started) {
        (Some(c), Some(at)) => format!(
            "nixa-app pid {} had run for {} s",
            c.id(),
            at.elapsed().as_secs()
        ),
        _ => "no nixa-app ran".into(),
    }
}

fn engine_changed(before: Option<u32>, now: Option<u32>) -> bool {
    now.is_some() && before != now
}

fn term_ids(json: &str) -> Vec<String> {
    json.split("{\"id\":\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect()
}

fn check_terms(run: &Run, st: &mut State) {
    let (ok, text) = nixa(run, &["--json", "term", "ls"]);
    if !ok {
        return;
    }
    let ids = term_ids(&text);
    let lost: Vec<String> = st
        .agents
        .iter()
        .filter(|a| !ids.contains(a))
        .cloned()
        .collect();
    for id in lost {
        st.agents.retain(|a| a != &id);
        st.lost_agents.push(id.clone());
        let window = window_text(st);
        note(
            run,
            st,
            format!("fake agent terminal {id} is gone, something removed it ({window})"),
        );
        add_agent(run, st);
    }
    let made: Vec<String> = ids
        .into_iter()
        .filter(|id| {
            !st.agents.contains(id) && !st.other_terms.contains(id) && !st.lost_agents.contains(id)
        })
        .collect();
    for id in made {
        st.other_terms.push(id.clone());
        note(
            run,
            st,
            format!("terminal {id} was made by something other than the soak"),
        );
    }
}

fn client_pids(lsof: &str) -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = Vec::new();
    for line in lsof.lines() {
        if let Some(pid) = line.strip_prefix('p').and_then(|p| p.parse().ok()) {
            out.push((pid, String::new()));
        } else if let (Some(name), Some(last)) = (line.strip_prefix('c'), out.last_mut()) {
            last.1 = name.to_string();
        }
    }
    out
}

fn check_clients(run: &Run, st: &mut State) {
    let port = format!("-iTCP:{}", run.port);
    let Some(text) = tool(
        "/usr/sbin/lsof",
        &["-nP", &port, "-sTCP:ESTABLISHED", "-Fpc"],
    ) else {
        return;
    };
    let ours = [
        st.watch[APP].pid,
        st.watch[BRAIN].pid,
        Some(std::process::id()),
    ];
    for (pid, name) in client_pids(&text) {
        let key = format!("pid {pid} ({name})");
        if ours.contains(&Some(pid)) || st.other_clients.contains(&key) {
            continue;
        }
        st.other_clients.push(key.clone());
        note(
            run,
            st,
            format!("another program is connected to the soak brain: {key}"),
        );
    }
}

fn sample(run: &Run, st: &mut State, agents: usize) {
    let owned = owned_pids(run).unwrap_or_default();
    let mut pids = [
        st.app.as_ref().map(Child::id),
        part_pid(run, ENGINE),
        part_pid(run, BRAIN),
    ];
    for part in [ENGINE, BRAIN] {
        if pids[part].is_some() {
            st.watch[part].hung = false;
            continue;
        }
        let Some(pid) = silent_pid(&owned, part) else {
            continue;
        };
        pids[part] = Some(pid);
        if !st.watch[part].hung {
            st.watch[part].hung = true;
            st.watch[part].hangs += 1;
            note(
                run,
                st,
                format!("{} pid {pid} runs but does not answer", PARTS[part]),
            );
        }
    }
    for pid in pids {
        record_pid(run, pid);
    }
    let t = elapsed(st);
    for (part, pid) in pids.iter().enumerate() {
        let before = st.watch[part].pid;
        if let Some(text) = track(&mut st.watch[part], PARTS[part], *pid) {
            note(run, st, text);
            if part == ENGINE && engine_changed(before, *pid) {
                st.agents.clear();
                for _ in 0..agents {
                    add_agent(run, st);
                }
            }
        }
        let Some(pid) = *pid else { continue };
        let Some((rss_kb, cpu)) = ps_stat(pid) else {
            continue;
        };
        let now = Instant::now();
        let w = &mut st.watch[part];
        let cpu_pct = w.cpu.map(|(c, at)| {
            ((cpu - c) / now.duration_since(at).as_secs_f64().max(0.001) * 100.0).max(0.0)
        });
        w.cpu = Some((cpu, now));
        let row = Row {
            t,
            part,
            pid,
            rss_kb,
            cpu_pct,
            threads: thread_count(pid),
            fds: fd_count(pid),
        };
        append_csv(&run.out, &row);
        st.rows.push(row);
    }
    check_terms(run, st);
    check_clients(run, st);
}

fn csv_line(row: &Row) -> String {
    let cpu = row.cpu_pct.map(|c| format!("{c:.1}")).unwrap_or_default();
    format!(
        "{:.0},{},{},{:.1},{cpu},{},{}",
        row.t,
        PARTS[row.part],
        row.pid,
        row.rss_kb as f64 / 1024.0,
        row.threads,
        row.fds
    )
}

fn append_csv(out: &Path, row: &Row) {
    let path = out.join("samples.csv");
    let fresh = !path.exists();
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        if fresh {
            let _ = writeln!(f, "t_secs,part,pid,rss_mb,cpu_pct,threads,open_files");
        }
        let _ = writeln!(f, "{}", csv_line(row));
    }
}

fn warm_up(total_secs: f64) -> f64 {
    (total_secs * 0.1).min(WARM_UP_MAX_SECS)
}

fn slope_per_hour(points: &[(f64, f64)]) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let num: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let den: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    if den == 0.0 { 0.0 } else { num / den * 3600.0 }
}

fn trend(points: &[(f64, f64)]) -> Option<f64> {
    let span = points.last()?.0 - points.first()?.0;
    (span >= TREND_MIN_SECS).then(|| slope_per_hour(points))
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    match n {
        0 => 0.0,
        _ if n % 2 == 1 => values[n / 2],
        _ => (values[n / 2 - 1] + values[n / 2]) / 2.0,
    }
}

fn part_stats(st: &State, part: usize) -> Option<Stats> {
    let rows: Vec<&Row> = st.rows.iter().filter(|r| r.part == part).collect();
    let mut pids: Vec<u32> = rows.iter().map(|r| r.pid).collect();
    pids.dedup();
    let pid = rows.last()?.pid;
    let seg: Vec<&Row> = rows.into_iter().filter(|r| r.pid == pid).collect();
    let span = seg.last()?.t - seg.first()?.t;
    let mut s = stats(&seg, warm_up(span))?;
    s.processes = pids.len();
    Some(s)
}

fn stats(rows: &[&Row], warm_secs: f64) -> Option<Stats> {
    let first = rows.first()?;
    let last = rows.last()?;
    let warm_t = first.t + warm_secs;
    let warm = rows.iter().find(|r| r.t >= warm_t).unwrap_or(last);
    let mb = |r: &Row| r.rss_kb as f64 / 1024.0;
    let after: Vec<(f64, f64)> = rows
        .iter()
        .filter(|r| r.t >= warm.t)
        .map(|r| (r.t, mb(r)))
        .collect();
    let cpus: Vec<f64> = rows.iter().filter_map(|r| r.cpu_pct).collect();
    let q = (after.len() / 4).max(1);
    let mut head: Vec<f64> = after[..q].iter().map(|p| p.1).collect();
    let mut tail: Vec<f64> = after[after.len() - q..].iter().map(|p| p.1).collect();
    Some(Stats {
        samples: rows.len(),
        pid: last.pid,
        processes: 1,
        after: after.len(),
        first_mb: mb(first),
        warm_mb: median(&mut head),
        end_mb: median(&mut tail),
        last_mb: mb(last),
        peak_mb: rows.iter().map(|r| mb(r)).fold(0.0, f64::max),
        slope_mb_h: trend(&after),
        avg_cpu: (!cpus.is_empty()).then(|| cpus.iter().sum::<f64>() / cpus.len() as f64),
        peak_cpu: cpus.iter().copied().reduce(f64::max),
        threads: (first.threads, last.threads),
        fds: (first.fds, last.fds),
        fds_warm: warm.fds,
    })
}

fn growth(s: &Stats) -> f64 {
    s.end_mb - s.warm_mb
}

fn memory_flat(s: &Stats) -> bool {
    growth(s) <= (s.warm_mb * 0.15).max(20.0)
}

fn files_flat(s: &Stats) -> bool {
    s.fds.1 <= s.fds_warm + 20
}

fn problems(st: &State, end: &Ending, window: bool) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(e) = &end.error {
        out.push(format!("the run failed: {e}"));
    }
    for (part, name) in PARTS.iter().enumerate() {
        if part == APP && !window {
            continue;
        }
        match part_stats(st, part) {
            None => out.push(format!("{name} has no samples")),
            Some(s) if s.after < MIN_SAMPLES => out.push(format!(
                "{name} has only {} samples after warm-up, at least {MIN_SAMPLES} are needed for a memory result",
                s.after
            )),
            Some(s) => {
                if !memory_flat(&s) {
                    out.push(format!("{name} memory grew {:.1} MB after warm-up", growth(&s)));
                }
                if !files_flat(&s) {
                    out.push(format!("{name} open files grew from {} to {}", s.fds_warm, s.fds.1));
                }
            }
        }
        let stops = if part == APP {
            st.app_exits.len()
        } else {
            st.watch[part].stops as usize
        };
        if stops > 0 {
            out.push(format!(
                "{name} stopped {stops} {}",
                if stops == 1 { "time" } else { "times" }
            ));
        }
        if st.watch[part].hangs > 0 {
            let n = st.watch[part].hangs;
            let word = if n == 1 { "time" } else { "times" };
            out.push(format!("{name} did not answer {n} {word} while it ran"));
        }
    }
    let calls = [
        ("engine calls", &st.engine_calls),
        ("brain calls", &st.brain_calls),
        ("window focus calls", &st.window_calls),
    ];
    for (what, c) in calls {
        if c.errors > 0 {
            out.push(format!("{} of {} {what} failed", c.errors, c.count));
        }
    }
    if window && st.window_calls.count > 0 && st.window_calls.count == st.window_calls.errors {
        out.push("the window never showed a fake agent pane".into());
    }
    if st.agent_fails > 0 {
        out.push(format!("{} fake agent starts failed", st.agent_fails));
    }
    if window && !st.closed_outside.is_empty() {
        out.push(format!(
            "nixa-app was closed from outside the run ({}), so the window part is not a full result",
            st.closed_outside.join("; ")
        ));
    }
    let lists = [
        ("panic lines in the logs", &end.panics),
        ("crash reports from macOS", &end.crash_reports),
        ("processes were left after the stop", &end.leftovers),
        ("processes still run after TERM and KILL", &end.alive),
        ("agent program starts in brain.log", &end.agent_starts),
        (
            "fake agent terminals were removed by something else",
            &st.lost_agents,
        ),
        (
            "terminals were made by something other than the soak",
            &st.other_terms,
        ),
        (
            "other programs connected to the soak brain",
            &st.other_clients,
        ),
    ];
    for (what, items) in lists {
        if !items.is_empty() {
            out.push(format!("{} {what}", items.len()));
        }
    }
    out
}

fn verdict(found: &[String], end: &Ending) -> Verdict {
    if !found.is_empty() {
        Verdict::Check
    } else if end.early {
        Verdict::Early
    } else {
        Verdict::Pass
    }
}

fn calls_line(name: &str, c: &Calls) -> String {
    let ok = c.count.saturating_sub(c.errors);
    if ok == 0 {
        return format!(
            "- {name}: {} calls, {} errors, no successful calls",
            c.count, c.errors
        );
    }
    format!(
        "- {name}: {} calls, {} errors, slowest {:.0} ms and average {:.0} ms of the successful ones",
        c.count,
        c.errors,
        c.max_ms,
        c.ok_ms / ok as f64
    )
}

fn part_block(text: &mut String, name: &str, s: &Stats, stops: u32) {
    let growth = growth(s);
    let mem = if memory_flat(s) { "flat" } else { "GROWS" };
    let files = if files_flat(s) { "flat" } else { "GROWS" };
    let cpu = |v: Option<f64>| {
        v.map(|c| format!("{c:.1}%"))
            .unwrap_or_else(|| "not measured".into())
    };
    let _ = writeln!(text, "\n## {name}\n");
    let _ = writeln!(
        text,
        "- Memory: {:.1} MB at start, {:.1} MB after warm-up, {:.1} MB near the end, {:.1} MB at the last sample, peak {:.1} MB",
        s.first_mb, s.warm_mb, s.end_mb, s.last_mb, s.peak_mb
    );
    if s.processes > 1 {
        let _ = writeln!(
            text,
            "- This part ran as {} processes; the numbers are for the last one, pid {}",
            s.processes, s.pid
        );
    }
    let trend = s
        .slope_mb_h
        .map(|v| format!("trend {v:+.1} MB per hour"))
        .unwrap_or_else(|| "no trend under 30 minutes after warm-up".into());
    let _ = writeln!(
        text,
        "- Growth after warm-up: {growth:+.1} MB, {trend}: {mem}"
    );
    let _ = writeln!(
        text,
        "- CPU: average {}, peak {}",
        cpu(s.avg_cpu),
        cpu(s.peak_cpu)
    );
    let _ = writeln!(
        text,
        "- Threads: {} at start, {} at the end",
        s.threads.0, s.threads.1
    );
    let _ = writeln!(
        text,
        "- Open files: {} at start, {} after warm-up, {} at the end: {files}",
        s.fds.0, s.fds_warm, s.fds.1
    );
    let _ = writeln!(text, "- Stops and restarts: {stops}");
    let _ = writeln!(text, "- Samples: {}", s.samples);
}

fn list_block(text: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        let _ = writeln!(text, "- {title}: none");
        return;
    }
    let _ = writeln!(text, "- {title}:");
    for item in items.iter().take(20) {
        let _ = writeln!(text, "  - {item}");
    }
    if items.len() > 20 {
        let _ = writeln!(text, "  - and {} more", items.len() - 20);
    }
}

fn render(o: &Opts, run: &Run, st: &State, end: &Ending) -> String {
    let found = problems(st, end, o.window);
    let mut text = String::from("# Nixa soak report\n\n");
    match verdict(&found, end) {
        Verdict::Pass => text.push_str("Result: PASS. No crashes, no failed calls, memory and open files stay flat.\n"),
        Verdict::Check => text.push_str("Result: CHECK.\n"),
        Verdict::Early => text.push_str("Result: STOPPED EARLY. The run did not reach its planned time, so this is not a final result.\n"),
    }
    if !found.is_empty() {
        text.push('\n');
        for p in &found {
            let _ = writeln!(text, "- {p}");
        }
    }
    let churn = o.churn.map(|d| {
        format!(
            "one replaced every {} ({} replaced)",
            span_text(d),
            st.churned
        )
    });
    let _ = writeln!(text, "\n## Run\n");
    let _ = writeln!(text, "- Started: {}", st.started_text);
    let _ = writeln!(
        text,
        "- Planned: {}. Ran: {}. Ended by: {}",
        span_text(o.run_for),
        span_text(end.ran),
        end.stopped_by
    );
    let _ = writeln!(
        text,
        "- Fake agents: {}, {}",
        o.agents,
        churn.unwrap_or_else(|| "no churn".into())
    );
    let _ = writeln!(
        text,
        "- Window: {}. Brain port: {}. Sample every {}",
        if o.window {
            "on, fake agents in tabs of the test space soak, a different pane focused every 5 s"
        } else {
            "off"
        },
        run.port,
        span_text(o.sample)
    );
    let _ = writeln!(
        text,
        "- Data folder: {} ({})",
        run.data.display(),
        end.data_note
    );
    let _ = writeln!(
        text,
        "- Warm-up skipped for growth: the first 10% of each process's time, at most {WARM_UP_MAX_SECS:.0} s"
    );
    let _ = writeln!(
        text,
        "- Agent programs: the environment is cleared except a short list, HOME is <data folder>/home, PATH is {FRESH_PATH}, and NIXA_CODEX_BIN, NIXA_OPENCODE_BIN, NIXA_AGY_BIN, NIXA_CURSOR_BIN and NIXA_KIMI_BIN point at a stub that exits at once. brain.log is checked for any agent program start."
    );
    let _ = writeln!(
        text,
        "- Files: samples.csv, events.log, engine.log, brain.log, app.log"
    );
    let _ = writeln!(
        text,
        "- Growth compares the middle value of the first quarter of the samples after warm-up with the middle value of the last quarter. Memory counts as flat when it grows at most 15% (at least 20 MB). Open files count as flat when they grow at most 20."
    );
    for (part, name) in PARTS.iter().enumerate() {
        if let Some(s) = part_stats(st, part) {
            part_block(&mut text, name, &s, st.watch[part].stops);
        }
    }
    let _ = writeln!(text, "\n## Calls\n");
    let _ = writeln!(
        text,
        "{}",
        calls_line("Engine (nixa term send, read, ls)", &st.engine_calls)
    );
    let _ = writeln!(text, "{}", calls_line("Brain GET /health", &st.brain_calls));
    if o.window {
        let _ = writeln!(
            text,
            "{}",
            calls_line("Window focus of a fake agent pane", &st.window_calls)
        );
    }
    let _ = writeln!(text, "\n## Crashes\n");
    list_block(&mut text, "nixa-app exits by itself", &st.app_exits);
    list_block(
        &mut text,
        "nixa-app closed from outside the run",
        &st.closed_outside,
    );
    list_block(
        &mut text,
        "Crash reports in ~/Library/Logs/DiagnosticReports",
        &end.crash_reports,
    );
    list_block(&mut text, "Panic lines in the logs", &end.panics);
    list_block(&mut text, "Processes left after the stop", &end.leftovers);
    list_block(
        &mut text,
        "Processes still running after TERM and KILL",
        &end.alive,
    );
    list_block(
        &mut text,
        "Agent program starts in brain.log",
        &end.agent_starts,
    );
    list_block(
        &mut text,
        "Fake agent terminals removed by something else",
        &st.lost_agents,
    );
    list_block(
        &mut text,
        "Other programs connected to the soak brain",
        &st.other_clients,
    );
    list_block(
        &mut text,
        "Terminals made by something other than the soak",
        &st.other_terms,
    );
    let _ = writeln!(text, "\n## Events\n");
    for (t, e) in st.events.iter().take(400) {
        let _ = writeln!(text, "- {} {e}", clock_text(*t));
    }
    if st.events.len() > 400 {
        let _ = writeln!(text, "- and {} more in events.log", st.events.len() - 400);
    }
    text
}

fn panic_lines(out: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for name in ["engine.log", "brain.log", "app.log"] {
        let text = fs::read_to_string(out.join(name)).unwrap_or_default();
        for line in text.lines().filter(|l| l.contains("panicked at")) {
            found.push(format!(
                "{name}: {}",
                line.chars().take(200).collect::<String>()
            ));
        }
    }
    found
}

fn crash_reports(since: SystemTime) -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let dir = Path::new(&home).join("Library/Logs/DiagnosticReports");
    let Ok(list) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<String> = list
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            PARTS.iter().any(|p| name.starts_with(&format!("{p}-"))) || name.starts_with("nixa-20")
        })
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|m| m >= since)
        })
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    found.sort();
    found
}

fn stop_app(st: &mut State) {
    let Some(mut child) = st.app.take() else {
        return;
    };
    let _ = tool("/bin/kill", &["-TERM", &child.id().to_string()]);
    let deadline = Instant::now() + STOP_WAIT;
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn stop_run(run: &Run) -> (Vec<String>, Vec<String>) {
    let mut alive: Vec<String> = Vec::new();
    let failed = |e: String, alive: &mut Vec<String>| {
        let line = format!("the process check failed ({e}), so the data folder is kept");
        if !alive.contains(&line) {
            alive.push(line);
        }
    };
    let named = |program: &str, alive: &mut Vec<String>| -> Vec<(u32, String)> {
        match owned_procs(run) {
            Ok(owned) => owned
                .into_iter()
                .filter(|(_, p, _)| p == program)
                .map(|(pid, _, start)| (pid, start))
                .collect(),
            Err(e) => {
                failed(e, alive);
                Vec::new()
            }
        }
    };
    let mut stuck: Vec<u32> = Vec::new();
    let app = named("nixa-app", &mut alive);
    stuck.extend(stop_pids(&app));
    let _ = nixa(run, &["engine", "stop"]);
    let brain = named("nixa-brain", &mut alive);
    stuck.extend(stop_pids(&brain));
    let engine = named("nixa-engine", &mut alive);
    stuck.extend(stop_pids(&engine));
    let mut left = Vec::new();
    for _ in 0..4 {
        sleep(Duration::from_millis(500));
        let rest = match owned_procs(run) {
            Ok(rest) => rest,
            Err(e) => {
                failed(e, &mut alive);
                break;
            }
        };
        if rest.is_empty() {
            break;
        }
        for (pid, program, _) in &rest {
            left.push(format!("{program} pid {pid}"));
        }
        let pids: Vec<(u32, String)> = rest.into_iter().map(|(pid, _, start)| (pid, start)).collect();
        stuck.extend(stop_pids(&pids));
    }
    match owned_pids(run) {
        Ok(rest) => stuck.extend(rest.iter().map(|(p, _)| *p)),
        Err(e) => failed(e, &mut alive),
    }
    stuck.sort();
    stuck.dedup();
    alive.extend(stuck.iter().map(|p| format!("pid {p}")));
    (left, alive)
}

fn shut_down(run: &Run, st: &mut State) -> (Vec<String>, Vec<String>) {
    stop_app(st);
    let (left, alive) = stop_run(run);
    for line in &left {
        note(run, st, format!("left after the stop, now stopped: {line}"));
    }
    for line in &alive {
        note(
            run,
            st,
            format!("still running after TERM and KILL: {line}"),
        );
    }
    (left, alive)
}

fn drive(run: &Run, o: &Opts, st: &mut State) -> Result<String, String> {
    let text = start_engine(run)?;
    note(run, st, text);
    let (ok, text) = nixa(run, &["brain", "start", "--wait", "30s"]);
    record_pid(run, brain_pid_file(&run.data));
    record_pid(run, part_pid(run, BRAIN));
    if !ok {
        return Err(format!("nixa-brain did not start: {text}"));
    }
    note(run, st, text);
    if o.window {
        make_space(run)?;
        note(
            run,
            st,
            format!("test space {SPACE_ID} made, fake agents go into its tabs"),
        );
        let pid = launch_app(run, st)?;
        note(run, st, format!("nixa-app started, pid {pid}"));
    }
    sleep(Duration::from_secs(3));
    sample(run, st, o.agents);
    if st.agents.is_empty() {
        for _ in 0..o.agents {
            add_agent(run, st);
        }
    }
    if st.agents.is_empty() {
        return Err("no fake agent could start".into());
    }
    let start = st.started.unwrap_or_else(Instant::now);
    let deadline = start + o.run_for;
    let mut next_sample = Instant::now() + o.sample;
    let mut next_act = Instant::now();
    let mut next_churn = o.churn.map(|d| Instant::now() + d);
    let mut next_report = Instant::now() + REPORT_EVERY;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Ok("time is up".into());
        }
        if run.out.join("stop").exists() {
            return Ok("the stop file".into());
        }
        check_app(run, st);
        if now >= next_sample {
            sample(run, st, o.agents);
            next_sample += o.sample;
        }
        if now >= next_act {
            act(run, st);
            next_act = now + ACT_EVERY;
        }
        if let (Some(at), Some(every)) = (next_churn, o.churn)
            && now >= at
        {
            churn(run, st);
            next_churn = Some(now + every);
        }
        if now >= next_report {
            let end = Ending {
                ran: start.elapsed(),
                early: true,
                stopped_by: "still running".into(),
                data_note: "in use".into(),
                ..Ending::default()
            };
            let _ = fs::write(run.out.join("report.md"), render(o, run, st, &end));
            next_report = now + REPORT_EVERY;
        }
        sleep(TICK);
    }
}

fn copy_logs(run: &Run) {
    for name in ["engine.log", "brain.log"] {
        let _ = fs::copy(run.data.join(name), run.out.join(name));
    }
}

fn manifest_text(port: u16, soak_pid: u32) -> String {
    format!("port={port}\nsoak={soak_pid}\n")
}

fn read_manifest(data: &Path) -> Option<(u16, u32)> {
    let text = fs::read_to_string(data.join(MANIFEST)).ok()?;
    let value = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .map(str::to_string)
    };
    Some((value("port")?.parse().ok()?, value("soak")?.parse().ok()?))
}

fn manifest_pids(data: &Path) -> Vec<(u32, String)> {
    let text = fs::read_to_string(data.join(MANIFEST)).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let (pid, start) = l.strip_prefix("pid=")?.split_once(" start=")?;
            Some((pid.trim().parse().ok()?, start.trim().to_string()))
        })
        .filter(|(_, start)| !start.is_empty())
        .collect()
}

fn start_time(pid: u32) -> Option<String> {
    let text = tool("/bin/ps", &["-o", "lstart=", "-p", &pid.to_string()])?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn record_pid(run: &Run, pid: Option<u32>) {
    let Some(pid) = pid else { return };
    let Some(start) = start_time(pid) else { return };
    if manifest_pids(&run.data).contains(&(pid, start.clone())) {
        return;
    }
    if let Ok(mut f) = fs::OpenOptions::new()
        .append(true)
        .open(run.data.join(MANIFEST))
    {
        let _ = writeln!(f, "pid={pid} start={start}");
    }
}

fn brain_pid_file(data: &Path) -> Option<u32> {
    fs::read_to_string(data.join("brain.pid"))
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

fn start_engine(run: &Run) -> Result<String, String> {
    let log_path = run.data.join("engine.log");
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("cannot open engine.log: {e}"))?;
    let _ = fs::set_permissions(&log_path, fs::Permissions::from_mode(0o600));
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&run.bins.engine);
    with_env(&mut cmd, run);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", run.bins.engine.display()))?;
    let pid = child.id();
    record_pid(run, Some(pid));
    let deadline = Instant::now() + ENGINE_WAIT;
    let result = loop {
        if part_pid(run, ENGINE).is_some() {
            break Ok(format!(
                "nixa-engine started, pid {pid}, socket {}",
                run.data.join("engine.sock").display()
            ));
        }
        if let Ok(Some(status)) = child.try_wait() {
            break Err(format!("nixa-engine exited early: {}", exit_text(status)));
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "nixa-engine pid {pid} did not answer in {} s",
                ENGINE_WAIT.as_secs()
            ));
        }
        sleep(Duration::from_millis(200));
    };
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    result
}

fn prepare(run: &Run) -> Result<(), String> {
    fs::create_dir_all(run.data.join("home"))
        .map_err(|e| format!("cannot make the run home folder: {e}"))?;
    fs::write(
        run.data.join(MANIFEST),
        manifest_text(run.port, std::process::id()),
    )
    .map_err(|e| e.to_string())?;
    let stub = run.data.join("no-agent");
    fs::write(&stub, NO_AGENT).map_err(|e| e.to_string())?;
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    write_script(&run.data)?;
    Ok(())
}

fn space_sql(home: &Path, now_ms: u128) -> String {
    format!(
        "INSERT OR IGNORE INTO spaces (space_id, name, folder, created_at, updated_at) VALUES ('{SPACE_ID}', 'soak', '{}', {now_ms}, {now_ms}); INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('app.active_space', '\"{SPACE_ID}\"', {now_ms});",
        home.display()
    )
}

fn make_space(run: &Run) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let db = run.data.join("nixa.db").display().to_string();
    let sql = space_sql(&run.data.join("home"), now);
    match bounded(
        Command::new("/usr/bin/sqlite3").args(["-cmd", ".timeout 5000", &db, &sql]),
        TOOL_LIMIT,
    ) {
        Ok((status, _, _)) if status.success() => Ok(()),
        Ok((_, out, err)) => Err(format!(
            "cannot make the soak space: {}{}",
            out.trim(),
            err.trim()
        )),
        Err(e) => Err(format!("cannot make the soak space: {e}")),
    }
}

fn agent_starts(brain_log: &str) -> Vec<String> {
    brain_log
        .lines()
        .filter(|l| l.contains("cli-brain: starting"))
        .map(|l| l.chars().take(200).collect())
        .collect()
}

fn finish_data(data: &Path, alive: &[String], keep: bool) -> String {
    if !alive.is_empty() {
        "not removed: processes may still run or could not be checked".to_string()
    } else if keep {
        "kept".to_string()
    } else if is_soak_data(data) && fs::remove_dir_all(data).is_ok() {
        "removed".to_string()
    } else {
        "not removed".to_string()
    }
}

fn soak(o: Opts) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = find_root(&cwd)
        .or_else(|| find_root(Path::new(env!("CARGO_MANIFEST_DIR"))))
        .ok_or("run soak inside the Nixa workspace")?;
    let bin_dir = o
        .bin_dir
        .clone()
        .unwrap_or_else(|| root.join("target").join("gpui").join("debug"));
    let bins = find_bins(&bin_dir, o.window)?;
    let stamp = clock(&["/bin/date", "+%Y%m%d-%H%M%S"]);
    let out = o
        .out
        .clone()
        .unwrap_or_else(|| root.join("target").join("soak").join(&stamp));
    let data = PathBuf::from(format!("{DATA_PREFIX}{}", std::process::id()));
    if data.exists() {
        return Err(format!(
            "{} already exists, remove it or run soak stop --data {}",
            data.display(),
            data.display()
        ));
    }
    fs::create_dir_all(&data).map_err(|e| format!("cannot make {}: {e}", data.display()))?;
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    fs::create_dir_all(&out).map_err(|e| format!("cannot make {}: {e}", out.display()))?;
    let run = Run {
        bins,
        data,
        out,
        port: o.port,
        window: o.window,
    };
    if let Err(e) = prepare(&run) {
        let _ = fs::remove_dir_all(&run.data);
        return Err(e);
    }
    if nixa(&run, &["brain", "status"]).0 {
        let _ = fs::remove_dir_all(&run.data);
        return Err(format!(
            "something already answers at 127.0.0.1:{}, pick another --port",
            o.port
        ));
    }
    let since = SystemTime::now();
    let mut st = State {
        started: Some(Instant::now()),
        started_text: clock(&["/bin/date", "+%Y-%m-%d %H:%M:%S %Z"]),
        ..State::default()
    };
    println!(
        "soak: {} with {} fake agents, data {}, report {}",
        span_text(o.run_for),
        o.agents,
        run.data.display(),
        run.out.display()
    );
    println!(
        "soak: stop early with: touch {}",
        run.out.join("stop").display()
    );
    let result = drive(&run, &o, &mut st);
    let ran = st.started.map(|s| s.elapsed()).unwrap_or_default();
    let (leftovers, alive) = shut_down(&run, &mut st);
    copy_logs(&run);
    let data_note = finish_data(&run.data, &alive, o.keep_data);
    let (stopped_by, error) = match &result {
        Ok(by) => (by.clone(), None),
        Err(e) => ("an error".to_string(), Some(e.clone())),
    };
    let brain_log = fs::read_to_string(run.out.join("brain.log")).unwrap_or_default();
    let end = Ending {
        ran,
        early: stopped_by == "the stop file",
        stopped_by,
        error,
        leftovers,
        alive,
        panics: panic_lines(&run.out),
        agent_starts: agent_starts(&brain_log),
        crash_reports: crash_reports(since),
        data_note,
    };
    let report = render(&o, &run, &st, &end);
    let path = run.out.join("report.md");
    fs::write(&path, &report).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    println!("soak: {}", report.lines().nth(2).unwrap_or(""));
    println!("soak: report {}", path.display());
    result?;
    match verdict(&problems(&st, &end, o.window), &end) {
        Verdict::Pass => Ok(()),
        Verdict::Check => Err(format!("the soak result is CHECK, see {}", path.display())),
        Verdict::Early => Err(format!(
            "the soak stopped early, the result is not final, see {}",
            path.display()
        )),
    }
}

fn stop_command(args: &[String]) -> Result<(), String> {
    let mut data = None;
    let mut bin_dir = None;
    let mut i = 0;
    while i < args.len() {
        let value = args
            .get(i + 1)
            .cloned()
            .ok_or_else(|| format!("{} needs a value", args[i]))?;
        match args[i].as_str() {
            "--data" => data = Some(PathBuf::from(value)),
            "--bin-dir" => bin_dir = Some(PathBuf::from(value)),
            other => return Err(format!("unknown soak stop option {other}")),
        }
        i += 2;
    }
    let data = data.ok_or("soak stop needs --data /tmp/nixa-soak-PID")?;
    if !is_soak_data(&data) {
        return Err(format!(
            "{} is not a soak data folder ({DATA_PREFIX}...)",
            data.display()
        ));
    }
    let (port, soak_pid) = read_manifest(&data).ok_or_else(|| {
        format!(
            "{} has no soak run file, so it is not a soak run; nothing was stopped",
            data.display()
        )
    })?;
    match is_alive(soak_pid) {
        Some(false) => {}
        Some(true) if soak_pid == std::process::id() => {}
        Some(true) => {
            return Err(format!(
                "the soak run pid {soak_pid} still runs; end it with its stop file, nothing was stopped"
            ));
        }
        None => {
            return Err(format!(
                "cannot check if the soak run pid {soak_pid} still runs, nothing was stopped"
            ));
        }
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = find_root(&cwd)
        .or_else(|| find_root(Path::new(env!("CARGO_MANIFEST_DIR"))))
        .ok_or("run soak inside the Nixa workspace")?;
    let dir = bin_dir.unwrap_or_else(|| root.join("target").join("gpui").join("debug"));
    let run = Run {
        bins: find_bins(&dir, false)?,
        out: data.clone(),
        data,
        port,
        window: false,
    };
    let (left, alive) = stop_run(&run);
    for line in &left {
        println!("soak stop: stopped {line}");
    }
    if !alive.is_empty() {
        return Err(format!(
            "still running after TERM and KILL: {}; {} was not removed",
            alive.join(", "),
            run.data.display()
        ));
    }
    fs::remove_dir_all(&run.data)
        .map_err(|e| format!("cannot remove {}: {e}", run.data.display()))?;
    println!("soak stop: {} removed", run.data.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn row(t: f64, part: usize, mb: u64, cpu: Option<f64>, fds: u32) -> Row {
        Row {
            t,
            part,
            pid: 10,
            rss_kb: mb * 1024,
            cpu_pct: cpu,
            threads: 4,
            fds,
        }
    }

    fn test_run() -> Run {
        let dir = PathBuf::from("/tmp/none");
        Run {
            bins: Bins {
                app: dir.join("a"),
                engine: dir.join("e"),
                brain: dir.join("b"),
                nixa: dir.join("m"),
            },
            data: PathBuf::from("/tmp/nixa-soak-1"),
            out: dir,
            port: DEFAULT_PORT,
            window: true,
        }
    }

    #[test]
    fn durations_read_units() {
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("4h").unwrap(), Duration::from_secs(14_400));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("12").unwrap(), Duration::from_secs(12));
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("-3m").is_err());
    }

    #[test]
    fn options_have_safe_defaults() {
        let o = parse_opts(&[]).unwrap();
        assert_eq!(o, Opts::default());
        assert_eq!(o.port, 7465);
        assert!(o.window);
        assert!(o.run_for >= Duration::from_secs(3600));
    }

    #[test]
    fn options_read_every_flag() {
        let o = parse_opts(&strs(&[
            "--for",
            "20m",
            "--agents",
            "3",
            "--sample",
            "10s",
            "--churn",
            "0",
            "--port",
            "7426",
            "--no-window",
            "--bin-dir",
            "/x/bin",
            "--out",
            "/x/out",
            "--keep-data",
        ]))
        .unwrap();
        assert_eq!(o.run_for, Duration::from_secs(1200));
        assert_eq!(o.agents, 3);
        assert_eq!(o.sample, Duration::from_secs(10));
        assert_eq!(o.churn, None);
        assert_eq!(o.port, 7426);
        assert!(!o.window);
        assert!(o.keep_data);
        assert_eq!(o.bin_dir, Some(PathBuf::from("/x/bin")));
        assert_eq!(o.out, Some(PathBuf::from("/x/out")));
    }

    #[test]
    fn options_refuse_bad_values() {
        for port in ["7321", "7330", "7331", "80"] {
            assert!(parse_opts(&strs(&["--port", port])).is_err(), "{port}");
        }
        assert!(parse_opts(&strs(&["--agents", "0"])).is_err());
        assert!(parse_opts(&strs(&["--agents", "65"])).is_err());
        assert!(parse_opts(&strs(&["--for", "5s"])).is_err());
        assert!(parse_opts(&strs(&["--sample", "200ms"])).is_err());
        assert!(parse_opts(&strs(&["--for"])).is_err());
        assert!(parse_opts(&strs(&["--fast"])).is_err());
    }

    #[test]
    fn stop_refuses_other_folders() {
        assert!(
            stop_command(&strs(&["--data", "/tmp/x"]))
                .unwrap_err()
                .contains("not a soak data folder")
        );
        assert!(
            stop_command(&strs(&[]))
                .unwrap_err()
                .contains("needs --data")
        );
        assert!(stop_command(&strs(&["--data", "/tmp/nixa-soak-1", "--port", "7321"])).is_err());
    }

    #[test]
    fn cpu_time_reads_ps_forms() {
        assert_eq!(parse_cpu_time("0:01.50"), Some(1.5));
        assert_eq!(parse_cpu_time("12:00.00"), Some(720.0));
        assert_eq!(parse_cpu_time("1:00:02.00"), Some(3602.0));
        assert_eq!(parse_cpu_time("2-00:00:01"), Some(172_801.0));
        assert_eq!(parse_cpu_time("x"), None);
    }

    #[test]
    fn own_process_is_measured() {
        let (rss, cpu) = ps_stat(std::process::id()).unwrap();
        assert!(rss > 0);
        assert!(cpu >= 0.0);
        assert!(thread_count(std::process::id()) >= 1);
        assert!(fd_count(std::process::id()) >= 3);
        assert_eq!(ps_stat(u32::MAX - 1), None);
    }

    #[test]
    fn pid_comes_from_json() {
        assert_eq!(
            json_u32(r#"{"running":true,"pid":4321,"socket":"/s"}"#, "pid"),
            Some(4321)
        );
        assert_eq!(json_u32(r#"{"pid": 7}"#, "pid"), Some(7));
        assert_eq!(json_u32(r#"{"running":true,"pid":null}"#, "pid"), None);
        assert_eq!(json_u32(r#"{"running":false}"#, "pid"), None);
    }

    #[test]
    fn only_soak_folders_are_removed() {
        assert!(is_soak_data(Path::new("/tmp/nixa-soak-123")));
        assert!(!is_soak_data(Path::new("/tmp/nixa-soak-")));
        assert!(!is_soak_data(Path::new("/tmp/nixa-soak-1/../x")));
        assert!(!is_soak_data(Path::new("/tmp/other")));
        assert!(!is_soak_data(Path::new("/")));
    }

    #[test]
    fn root_is_the_workspace_folder() {
        let dir = std::env::temp_dir().join(format!("soak-root-{}", std::process::id()));
        let deep = dir.join("crates").join("x");
        fs::create_dir_all(&deep).unwrap();
        fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        fs::write(deep.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert_eq!(find_root(&deep), Some(dir.clone()));
        let missing = |window| {
            find_bins(&deep, window)
                .unwrap_err()
                .split(". Build")
                .next()
                .unwrap_or("")
                .to_string()
        };
        assert!(missing(true).contains("nixa-app"));
        assert!(!missing(false).contains("nixa-app"));
        assert!(missing(false).contains("nixa-engine"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn watch_counts_stops_and_restarts() {
        let mut w = Watch::default();
        assert_eq!(track(&mut w, "nixa-brain", Some(5)), None);
        assert_eq!(track(&mut w, "nixa-brain", Some(5)), None);
        assert!(
            track(&mut w, "nixa-brain", Some(6))
                .unwrap()
                .contains("new pid 6")
        );
        assert!(track(&mut w, "nixa-brain", None).unwrap().contains("gone"));
        assert!(
            track(&mut w, "nixa-brain", Some(7))
                .unwrap()
                .contains("back")
        );
        assert_eq!(w.stops, 2);
        let mut never = Watch::default();
        assert_eq!(track(&mut never, "nixa-app", None), None);
        assert_eq!(never.stops, 0);
    }

    #[test]
    fn a_new_engine_needs_new_agents() {
        assert!(engine_changed(Some(1), Some(2)));
        assert!(engine_changed(None, Some(2)));
        assert!(!engine_changed(Some(2), Some(2)));
        assert!(!engine_changed(Some(2), None));
    }

    #[test]
    fn flat_memory_passes_and_a_leak_does_not() {
        let flat: Vec<Row> = (0..20)
            .map(|i| row(i as f64 * 120.0, BRAIN, 100 + (i % 3), Some(2.0), 30))
            .collect();
        let refs: Vec<&Row> = flat.iter().collect();
        let s = stats(&refs, 120.0).unwrap();
        assert!(memory_flat(&s));
        assert!(files_flat(&s));
        assert_eq!(s.avg_cpu, Some(2.0));
        assert!(s.slope_mb_h.unwrap().abs() < 5.0);
        let leak: Vec<Row> = (0..20)
            .map(|i| {
                row(
                    i as f64 * 120.0,
                    BRAIN,
                    100 + i * 5,
                    None,
                    30 + i as u32 * 3,
                )
            })
            .collect();
        let refs: Vec<&Row> = leak.iter().collect();
        let s = stats(&refs, 120.0).unwrap();
        assert!(!memory_flat(&s));
        assert!(!files_flat(&s));
        assert_eq!(s.avg_cpu, None);
        assert!((s.slope_mb_h.unwrap() - 150.0).abs() < 1.0);
        assert_eq!(growth(&s), 75.0);
        let short: Vec<&Row> = leak.iter().take(5).collect();
        assert_eq!(stats(&short, 120.0).unwrap().slope_mb_h, None);
        assert_eq!(s.peak_mb, 195.0);
    }

    #[test]
    fn report_says_pass_or_check_without_tables() {
        let o = Opts {
            run_for: Duration::from_secs(1200),
            ..Opts::default()
        };
        let run = test_run();
        let mut st = State {
            started_text: "today".into(),
            ..State::default()
        };
        for part in [APP, ENGINE, BRAIN] {
            for i in 0..10 {
                st.rows.push(row(i as f64 * 120.0, part, 80, Some(1.0), 20));
            }
        }
        st.events.push((3.0, "nixa-engine started".into()));
        let end = Ending {
            ran: Duration::from_secs(1200),
            stopped_by: "time is up".into(),
            data_note: "removed".into(),
            ..Ending::default()
        };
        let text = render(&o, &run, &st, &end);
        assert!(text.contains("Result: PASS"), "{text}");
        assert!(text.contains("## nixa-brain"));
        assert!(text.contains("+00:00:03 nixa-engine started"));
        assert!(!text.contains('|'));
        st.watch[ENGINE].stops = 1;
        let crashed = Ending {
            panics: vec!["brain.log: thread panicked at x".into()],
            ..end
        };
        let text = render(&o, &run, &st, &crashed);
        assert!(text.contains("Result: CHECK"));
        assert!(text.contains("nixa-engine stopped 1 time"));
        assert!(text.contains("1 panic lines"));
    }

    #[test]
    fn missing_window_samples_only_matter_with_a_window() {
        let st = State::default();
        let end = Ending {
            ran: Duration::from_secs(60),
            ..Ending::default()
        };
        assert_eq!(problems(&st, &end, true).len(), 3);
        assert_eq!(problems(&st, &end, false).len(), 2);
    }

    #[test]
    fn csv_line_has_every_field() {
        let line = csv_line(&Row {
            t: 61.4,
            part: ENGINE,
            pid: 9,
            rss_kb: 2048,
            cpu_pct: Some(3.25),
            threads: 7,
            fds: 12,
        });
        assert_eq!(line, "61,nixa-engine,9,2.0,3.2,7,12");
        let line = csv_line(&Row {
            t: 0.0,
            part: APP,
            pid: 1,
            rss_kb: 1024,
            cpu_pct: None,
            threads: 1,
            fds: 2,
        });
        assert_eq!(line, "0,nixa-app,1,1.0,,1,2");
    }

    #[test]
    fn times_print_plainly() {
        assert_eq!(clock_text(3725.0), "+01:02:05");
        assert_eq!(span_text(Duration::from_secs(14_400)), "4h 00m 00s");
    }

    #[test]
    fn env_points_at_the_run_only() {
        let dir = soak_dir("env");
        let run = Run {
            data: dir.clone(),
            ..test_run()
        };
        let d = dir.display().to_string();
        let mut cmd = Command::new("/usr/bin/env");
        with_env(&mut cmd, &run);
        let (_, text, _) = bounded(&mut cmd, TOOL_LIMIT).unwrap();
        let has = |line: &str| text.lines().any(|l| l == line);
        assert!(has(&format!("NIXA_DATA_DIR={d}")), "{text}");
        assert!(has("NIXA_BRAIN_ADDR=127.0.0.1:7465"));
        assert!(has(&format!("NIXA_ENGINE_SOCKET={d}/engine.sock")));
        assert!(has(&format!("HOME={d}/home")));
        assert!(has(&format!("PATH={FRESH_PATH}")));
        for key in AGENT_BIN_ENV {
            assert!(has(&format!("{key}={d}/no-agent")), "{key}");
        }
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        assert!(!text.contains("CARGO_"), "{text}");
        assert!(!text.contains("NIXA_TERM_ID"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_agent_stub_fails_at_once() {
        let dir = soak_dir("stub");
        let run = Run {
            data: dir.clone(),
            ..test_run()
        };
        prepare(&run).unwrap();
        let mut cmd = Command::new(dir.join("no-agent"));
        let (status, _, text) = bounded(&mut cmd, TOOL_LIMIT).unwrap();
        assert!(!status.success());
        assert!(text.contains("agent programs are off"));
        assert_eq!(
            read_manifest(&dir),
            Some((DEFAULT_PORT, std::process::id()))
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    fn soak_dir(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("{DATA_PREFIX}t{}{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bounded_calls_end_on_time() {
        let start = Instant::now();
        let err = bounded(
            Command::new("/bin/sleep").arg("5"),
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(err.contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(3));
        let (status, out, err) = bounded(Command::new("/bin/echo").arg("hi"), TOOL_LIMIT).unwrap();
        assert!(status.success());
        assert_eq!((out.as_str(), err.as_str()), ("hi\n", ""));
    }

    #[test]
    fn bounded_calls_end_when_a_child_keeps_the_output_open() {
        let dir = soak_dir("pipe");
        let pid_file = dir.join("child");
        let script = format!("/bin/sleep 30 & echo $! > {}; wait", pid_file.display());
        let start = Instant::now();
        let err = bounded(
            Command::new("/bin/sh").args(["-c", &script]),
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "{:?}",
            start.elapsed()
        );
        let child: u32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        sleep(Duration::from_millis(300));
        assert_eq!(is_alive(child), Some(false));
        let script = format!(
            "/bin/sleep 30 & echo $! > {}; echo done",
            pid_file.display()
        );
        let start = Instant::now();
        let (status, out, _) =
            bounded(Command::new("/bin/sh").args(["-c", &script]), TOOL_LIMIT).unwrap();
        assert!(status.success());
        assert_eq!(out, "done\n");
        assert!(start.elapsed() < DRAIN_LIMIT + Duration::from_secs(2));
        let child: u32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let _ = tool("/bin/kill", &["-KILL", &child.to_string()]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_processes_the_run_started_are_owned() {
        let p = |pid, ppid, start: &str, exe: &str, args: &str| Proc {
            pid,
            ppid,
            start: start.into(),
            exe: exe.into(),
            args: args.into(),
        };
        let t1 = "Sun Nov 22 23:00:01 2026";
        let t2 = "Sun Nov 22 23:40:07 2026";
        let script = "/tmp/nixa-soak-9/fake-agent.sh";
        let table = vec![
            p(10, 1, t1, "/b/nixa-brain", "/b/nixa-brain"),
            p(11, 10, t1, "/bin/zsh", "zsh"),
            p(12, 1, t1, "/b/nixa-engine", "/b/nixa-engine"),
            p(
                13,
                1,
                t1,
                "/usr/bin/tail",
                "tail -f /tmp/nixa-soak-9/brain.log",
            ),
            p(
                14,
                1,
                t1,
                "/bin/bash",
                "/bin/bash /tmp/nixa-soak-9/fake-agent.sh 3",
            ),
            p(15, 14, t1, "/bin/sleep", "sleep 1"),
            p(
                16,
                1,
                t1,
                "/usr/bin/less",
                "less /tmp/nixa-soak-9/fake-agent.sh",
            ),
            p(17, 1, t1, "/b/nixa-brain", "/b/nixa-brain"),
            p(18, 1, t1, "/x/nixa-app", "/x/nixa-app"),
            p(19, 1, t2, "/b/nixa-brain", "/b/nixa-brain"),
            p(21, 19, t2, "/bin/zsh", "zsh"),
            p(20, 1, t1, "/b/xtask", "xtask soak"),
        ];
        let bins = vec![
            "/b/nixa-brain".to_string(),
            "/b/nixa-engine".to_string(),
            "/b/nixa-app".to_string(),
        ];
        let roots: Vec<(u32, String)> = [10, 12, 18, 19, 20, 99]
            .iter()
            .map(|p| (*p, t1.to_string()))
            .collect();
        let owned = owned_from(&table, &roots, &bins, script, 20);
        let pids: Vec<u32> = owned.iter().map(|(p, _, _)| *p).collect();
        assert_eq!(pids, vec![10, 12, 14, 11, 15]);
        assert_eq!(owned[0].1, "nixa-brain");
        assert_eq!(owned[0].2, t1);
        let no_start = vec![p(10, 1, "", "/b/nixa-brain", "/b/nixa-brain")];
        assert!(owned_from(&no_start, &[(10, String::new())], &bins, script, 20).is_empty());
    }

    #[test]
    fn the_process_table_reads_ps_columns() {
        let comm =
            "    1     0 /sbin/launchd\n  356     1 /tmp/a b/nixa-app\n  357   356 /bin/sleep\n";
        let args = "  356 /tmp/a b/nixa-app 60\n  357 /bin/sleep 1\n";
        let starts = "  356 Sun Nov 22 23:00:01 2026\n    1 Tue Nov 17 13:54:24 2026\n";
        let table = parse_table(comm, args, starts);
        assert_eq!(table.len(), 3);
        assert_eq!((table[1].pid, table[1].ppid), (356, 1));
        assert_eq!(table[1].exe, "/tmp/a b/nixa-app");
        assert_eq!(table[1].args, "/tmp/a b/nixa-app 60");
        assert_eq!(table[1].start, "Sun Nov 22 23:00:01 2026");
        assert_eq!(table[0].start, "Tue Nov 17 13:54:24 2026");
        assert_eq!(table[2].ppid, 356);
        assert_eq!((table[0].args.as_str(), table[2].start.as_str()), ("", ""));
    }

    #[test]
    fn data_stays_when_the_process_check_failed() {
        let dir = soak_dir("keep");
        let failed =
            vec!["the process check failed (ps failed: x), so the data folder is kept".to_string()];
        assert!(finish_data(&dir, &failed, false).starts_with("not removed"));
        assert!(dir.exists());
        assert_eq!(finish_data(&dir, &[], true), "kept");
        assert!(dir.exists());
        assert_eq!(finish_data(&dir, &[], false), "removed");
        assert!(!dir.exists());
    }

    #[test]
    fn the_run_file_keeps_started_pids_with_their_start_time() {
        let dir = soak_dir("pids");
        let run = Run {
            data: dir.clone(),
            ..test_run()
        };
        prepare(&run).unwrap();
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let me = std::process::id();
        record_pid(&run, Some(me));
        record_pid(&run, Some(me));
        record_pid(&run, None);
        record_pid(&run, Some(child.id()));
        record_pid(&run, Some(u32::MAX - 1));
        let pids = manifest_pids(&dir);
        assert_eq!(
            pids.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
            vec![me, child.id()]
        );
        assert_eq!(Some(pids[0].1.clone()), start_time(me));
        assert!(!pids[0].1.is_empty());
        fs::write(dir.join("brain.pid"), "43\n/x/nixa-brain\n").unwrap();
        assert_eq!(brain_pid_file(&dir), Some(43));
        assert_eq!(read_manifest(&dir), Some((DEFAULT_PORT, me)));
        child.kill().unwrap();
        child.wait().unwrap();
        fs::write(
            dir.join(MANIFEST),
            format!(
                "port=1\nsoak=2\npid={} start=Sun Nov 22 23:00:01 2000\npid=5\n",
                child.id()
            ),
        )
        .unwrap();
        assert_eq!(
            manifest_pids(&dir),
            vec![(child.id(), "Sun Nov 22 23:00:01 2000".to_string())]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pid_with_another_start_time_is_never_signalled() {
        let mut c = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let start = start_time(c.id()).unwrap();
        assert_eq!(same_process(c.id(), &start), Some(true));
        assert_eq!(
            same_process(c.id(), "Sun Nov 22 23:00:01 2000"),
            Some(false)
        );
        let left = stop_pids(&[(c.id(), "Sun Nov 22 23:00:01 2000".to_string())]);
        assert!(left.is_empty());
        assert!(c.try_wait().unwrap().is_none());
        assert_eq!(is_alive(c.id()), Some(true));
        let left = stop_pids(&[(c.id(), start)]);
        assert!(left.is_empty());
        c.wait().unwrap();
    }

    #[test]
    fn a_reused_pid_is_not_ours() {
        let dir = soak_dir("reuse");
        let copy = dir.join("nixa-brain");
        fs::copy("/bin/sleep", &copy).unwrap();
        tool(
            "/usr/bin/codesign",
            &["-s", "-", "-f", &copy.display().to_string()],
        )
        .unwrap();
        let run = Run {
            bins: Bins {
                app: dir.join("nixa-app"),
                engine: dir.join("nixa-engine"),
                brain: copy.clone(),
                nixa: dir.join("nixa"),
            },
            data: dir.clone(),
            out: dir.clone(),
            ..test_run()
        };
        let mut other = Command::new(&copy).arg("60").spawn().unwrap();
        let started = start_time(other.id()).unwrap();
        fs::write(
            dir.join(MANIFEST),
            format!(
                "port=1\nsoak=2\npid={} start=Sun Nov 22 23:00:01 2000\n",
                other.id()
            ),
        )
        .unwrap();
        assert!(owned_pids(&run).unwrap().is_empty());
        fs::write(
            dir.join(MANIFEST),
            format!("port=1\nsoak=2\npid={} start={started}\n", other.id()),
        )
        .unwrap();
        assert_eq!(
            owned_pids(&run).unwrap(),
            vec![(other.id(), "nixa-brain".to_string())]
        );
        other.kill().unwrap();
        other.wait().unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_reused_pid_in_the_record_still_stops_the_new_process() {
        let dir = soak_dir("reuse-stop");
        let copy = dir.join("nixa-brain");
        fs::copy("/bin/sleep", &copy).unwrap();
        tool(
            "/usr/bin/codesign",
            &["-s", "-", "-f", &copy.display().to_string()],
        )
        .unwrap();
        let run = Run {
            bins: Bins {
                app: dir.join("nixa-app"),
                engine: dir.join("nixa-engine"),
                brain: copy.clone(),
                nixa: dir.join("nixa"),
            },
            data: dir.clone(),
            out: dir.clone(),
            ..test_run()
        };
        let mut brain = Command::new(&copy).arg("60").spawn().unwrap();
        let started = start_time(brain.id()).unwrap();
        fs::write(
            dir.join(MANIFEST),
            format!(
                "port=1\nsoak=2\npid={id} start=Sun Nov 22 23:00:01 2000\npid={id} start={started}\n",
                id = brain.id()
            ),
        )
        .unwrap();
        assert_eq!(
            owned_procs(&run).unwrap(),
            vec![(brain.id(), "nixa-brain".to_string(), started)]
        );
        let (_, alive) = stop_run(&run);
        assert!(alive.is_empty(), "{alive:?}");
        assert!(brain.try_wait().unwrap().is_some());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stalled_engine_is_recorded_and_stopped() {
        let dir = soak_dir("stall");
        let wrapper = dir.join("engine-wrapper");
        let run = Run {
            bins: Bins {
                app: dir.join("nixa-app"),
                engine: wrapper.clone(),
                brain: dir.join("nixa-brain"),
                nixa: dir.join("nixa"),
            },
            data: dir.clone(),
            out: dir.clone(),
            ..test_run()
        };
        fs::write(&wrapper, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        prepare(&run).unwrap();
        let start = Instant::now();
        let err = start_engine(&run).unwrap_err();
        assert!(err.contains("did not answer"), "{err}");
        assert!(start.elapsed() < ENGINE_WAIT + Duration::from_secs(5));
        let recorded = manifest_pids(&dir);
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        let pid = recorded[0].0;
        assert_eq!(is_alive(pid), Some(true));
        let run = Run {
            bins: Bins {
                engine: PathBuf::from("/bin/sleep"),
                ..run.bins
            },
            ..run
        };
        let (_, alive) = stop_run(&run);
        assert!(alive.is_empty(), "{alive:?}");
        sleep(Duration::from_millis(300));
        assert_eq!(is_alive(pid), Some(false));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_relative_bin_folder_becomes_absolute() {
        let dir = soak_dir("bins");
        for name in ["nixa-app", "nixa-engine", "nixa-brain", "nixa"] {
            fs::write(dir.join(name), "").unwrap();
        }
        let cwd = std::env::current_dir().unwrap();
        let up = "../".repeat(cwd.components().count() - 1);
        let rel = PathBuf::from(format!(
            "{up}{}",
            dir.display().to_string().trim_start_matches('/')
        ));
        assert!(rel.is_relative());
        let bins = find_bins(&rel, true).unwrap();
        assert!(bins.nixa.is_absolute(), "{}", bins.nixa.display());
        assert!(bins.nixa.is_file());
        assert!(bins.engine.is_absolute());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recovery_needs_a_soak_run_file_and_a_finished_run() {
        let dir = soak_dir("stop");
        let err = stop_command(&strs(&["--data", &dir.display().to_string()])).unwrap_err();
        assert!(err.contains("no soak run file"), "{err}");
        let err = stop_command(&strs(&["--data", "/tmp/nixa-soak-0none"])).unwrap_err();
        assert!(err.contains("no soak run file"), "{err}");
        let mut live = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        fs::write(dir.join(MANIFEST), manifest_text(7466, live.id())).unwrap();
        let err = stop_command(&strs(&["--data", &dir.display().to_string()])).unwrap_err();
        assert!(err.contains("still runs"), "{err}");
        assert!(dir.exists());
        assert!(
            stop_command(&strs(&[
                "--data",
                &dir.display().to_string(),
                "--port",
                "7465"
            ]))
            .is_err()
        );
        live.kill().unwrap();
        live.wait().unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stop_run_stops_only_what_the_run_started() {
        let dir = soak_dir("orph");
        let copy = |from: &str, to: &str| {
            fs::copy(from, dir.join(to)).unwrap();
            let path = dir.join(to).display().to_string();
            tool("/usr/bin/codesign", &["-s", "-", "-f", &path]).unwrap();
            dir.join(to)
        };
        let app = copy("/bin/sleep", "nixa-app");
        let engine = copy("/bin/bash", "nixa-engine");
        fs::write(dir.join("fake-agent.sh"), "/bin/sleep 60\n").unwrap();
        let run = Run {
            bins: Bins {
                app: app.clone(),
                engine: engine.clone(),
                brain: dir.join("nixa-brain"),
                nixa: dir.join("nixa"),
            },
            data: dir.clone(),
            out: dir.clone(),
            ..test_run()
        };
        fs::write(dir.join(MANIFEST), manifest_text(DEFAULT_PORT, 1)).unwrap();
        let spawn = |cmd: &mut Command| {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        };
        let mut a = spawn(Command::new(&app).arg("60"));
        let mut e = spawn(
            Command::new(&engine).args(["-c", "trap '' TERM; while :; do /bin/sleep 1; done"]),
        );
        let script = dir.join("fake-agent.sh").display().to_string();
        let mut f = spawn(Command::new("/bin/bash").args([script.as_str(), "1"]));
        let mut reader = spawn(Command::new("/bin/sleep").arg("60").current_dir(&dir));
        let mut not_recorded = spawn(Command::new(&app).arg("60"));
        record_pid(&run, Some(a.id()));
        record_pid(&run, Some(e.id()));
        sleep(Duration::from_millis(300));
        for c in [&mut a, &mut e, &mut f, &mut reader, &mut not_recorded] {
            assert!(c.try_wait().unwrap().is_none());
        }
        let owned: Vec<u32> = owned_pids(&run).unwrap().iter().map(|(p, _)| *p).collect();
        for pid in [a.id(), e.id(), f.id()] {
            assert!(owned.contains(&pid), "{pid} {owned:?}");
        }
        assert!(!owned.contains(&reader.id()));
        assert!(!owned.contains(&not_recorded.id()));
        let (_, alive) = stop_run(&run);
        assert!(alive.is_empty(), "{alive:?}");
        for c in [&mut a, &mut e, &mut f] {
            let _ = c.wait();
            assert_eq!(is_alive(c.id()), Some(false));
        }
        assert_eq!(is_alive(reader.id()), Some(true));
        assert_eq!(is_alive(not_recorded.id()), Some(true));
        assert!(owned_pids(&run).unwrap().is_empty());
        for c in [&mut reader, &mut not_recorded] {
            c.kill().unwrap();
            c.wait().unwrap();
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_gone_process_is_known_gone() {
        let mut c = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        assert_eq!(is_alive(c.id()), Some(true));
        c.kill().unwrap();
        c.wait().unwrap();
        assert_eq!(is_alive(c.id()), Some(false));
    }

    #[test]
    fn the_test_space_is_written_to_the_store() {
        let dir = soak_dir("space");
        let db = dir.join("nixa.db").display().to_string();
        let schema = "CREATE TABLE spaces (space_id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', folder TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL); CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL);";
        tool("/usr/bin/sqlite3", &[&db, schema]).unwrap();
        let run = Run {
            data: dir.clone(),
            ..test_run()
        };
        make_space(&run).unwrap();
        make_space(&run).unwrap();
        let rows = tool("/usr/bin/sqlite3", &[&db, "SELECT space_id, folder FROM spaces; SELECT value FROM settings WHERE key = 'app.active_space';"]).unwrap();
        assert_eq!(
            rows,
            format!("soak|{}\n\"soak\"\n", dir.join("home").display())
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_silent_part_keeps_its_process() {
        let owned = vec![
            (5, "bash".to_string()),
            (7, "nixa-engine".to_string()),
            (9, "nixa-brain".to_string()),
        ];
        assert_eq!(silent_pid(&owned, ENGINE), Some(7));
        assert_eq!(silent_pid(&owned, BRAIN), Some(9));
        assert_eq!(silent_pid(&owned[..1], BRAIN), None);
    }

    #[test]
    fn agent_starts_are_found_in_the_brain_log() {
        let log =
            "[1] brain: listening\n[2] cli-brain: starting codex\n[3] cli-brain: codex ready\n";
        assert_eq!(
            agent_starts(log),
            vec!["[2] cli-brain: starting codex".to_string()]
        );
        assert!(agent_starts("[1] brain: listening\n").is_empty());
    }

    #[test]
    fn average_counts_only_successful_calls() {
        let mut c = Calls::default();
        count_call(&mut c, true, Duration::from_millis(100));
        count_call(&mut c, false, Duration::from_millis(100));
        assert_eq!(
            calls_line("x", &c),
            "- x: 2 calls, 1 errors, slowest 100 ms and average 100 ms of the successful ones"
        );
        let mut bad = Calls::default();
        count_call(&mut bad, false, Duration::from_millis(5));
        assert!(calls_line("x", &bad).contains("no successful calls"));
    }

    #[test]
    fn failures_and_thin_runs_make_the_verdict_check() {
        let good = || {
            let mut st = State::default();
            for part in [APP, ENGINE, BRAIN] {
                for i in 0..10 {
                    st.rows.push(row(i as f64 * 120.0, part, 80, Some(1.0), 20));
                }
            }
            st
        };
        let end = Ending {
            ran: Duration::from_secs(1200),
            ..Ending::default()
        };
        assert_eq!(verdict(&problems(&good(), &end, true), &end), Verdict::Pass);
        let mut st = good();
        st.engine_calls = Calls {
            count: 10,
            errors: 10,
            ..Calls::default()
        };
        let found = problems(&st, &end, true);
        assert!(
            found.contains(&"10 of 10 engine calls failed".to_string()),
            "{found:?}"
        );
        assert_eq!(verdict(&found, &end), Verdict::Check);
        let mut st = good();
        st.app_exits.push("exit code 0".into());
        assert!(problems(&st, &end, true).contains(&"nixa-app stopped 1 time".to_string()));
        let mut st = good();
        st.agent_fails = 5;
        assert!(problems(&st, &end, true).contains(&"5 fake agent starts failed".to_string()));
        let mut st = good();
        st.window_calls = Calls {
            count: 4,
            errors: 4,
            ..Calls::default()
        };
        assert!(
            problems(&st, &end, true)
                .contains(&"the window never showed a fake agent pane".to_string())
        );
        let mut st = good();
        st.watch[BRAIN].hangs = 1;
        assert!(
            problems(&st, &end, true)
                .contains(&"nixa-brain did not answer 1 time while it ran".to_string())
        );
        let mut thin = State::default();
        for part in [APP, ENGINE, BRAIN] {
            thin.rows.push(row(0.0, part, 80, None, 20));
        }
        assert!(
            problems(&thin, &end, true)
                .iter()
                .any(|p| p.contains("only 1 samples"))
        );
        let starts = Ending {
            agent_starts: vec!["cli-brain: starting codex".into()],
            ..end.clone()
        };
        assert_eq!(
            verdict(&problems(&good(), &starts, true), &starts),
            Verdict::Check
        );
        let early = Ending {
            early: true,
            ..end.clone()
        };
        assert_eq!(
            verdict(&problems(&good(), &early, true), &early),
            Verdict::Early
        );
    }

    #[test]
    fn fake_agent_prints_status_and_output() {
        let dir = std::env::temp_dir().join(format!("soak-agent-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let script = write_script(&dir).unwrap();
        let mut child = Command::new("/bin/bash")
            .arg(&script)
            .arg("7")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        sleep(Duration::from_millis(1500));
        child.kill().unwrap();
        let out = child.wait_with_output().unwrap();
        let text = lossy(&out.stdout);
        assert!(
            text.contains("\u{1b}]7421;status=working;note=fake agent 7 step 1\u{7}"),
            "{text}"
        );
        assert!(text.lines().count() > 5, "{text}");
        assert!(
            !FAKE_AGENT
                .lines()
                .skip(1)
                .any(|l| l.trim_start().starts_with('#'))
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fake_agent_start_accepts_readable_and_legacy_terminal_ids() {
        let temp = tempfile::tempdir().unwrap();
        let mut run = test_run();
        run.data = temp.path().to_path_buf();
        run.out = run.data.clone();
        run.bins.nixa = run.data.join("fake-nixa");
        let mut state = State::default();

        for id in ["demo-bash-a1b2c3", "m7", "malformed-id"] {
            let script = format!("#!/bin/sh\nprintf '%s\\n' 'started' '{id}'\n");
            fs::write(&run.bins.nixa, script).unwrap();
            fs::set_permissions(&run.bins.nixa, fs::Permissions::from_mode(0o700)).unwrap();
            add_agent(&run, &mut state);
        }

        assert_eq!(state.agents, ["demo-bash-a1b2c3", "m7"]);
        assert_eq!(state.agent_fails, 1);
    }

    #[test]
    fn terminal_ids_come_from_the_list() {
        let json = r#"{"reply":"terms","terms":[{"id":"m1","name":"fake-1","labels":{"soak":"fake"},"children":["m3"]},{"id":"ma","name":"zsh"}]}"#;
        assert_eq!(term_ids(json), vec!["m1".to_string(), "ma".to_string()]);
        assert!(term_ids(r#"{"reply":"terms","terms":[]}"#).is_empty());
    }

    #[test]
    fn clients_come_from_lsof_fields() {
        let text = "p12\ncnixa-app\nf9\np40\ncsomething\n";
        assert_eq!(
            client_pids(text),
            vec![(12, "nixa-app".to_string()), (40, "something".to_string())]
        );
        assert!(client_pids("").is_empty());
    }

    #[test]
    fn middle_values_ignore_spikes() {
        assert_eq!(median(&mut [5.0, 1.0, 300.0]), 5.0);
        assert_eq!(median(&mut [4.0, 2.0]), 3.0);
        assert_eq!(median(&mut []), 0.0);
        let spiky: Vec<Row> = (0..40)
            .map(|i| {
                row(
                    i as f64 * 60.0,
                    ENGINE,
                    if i % 7 == 6 { 200 } else { 20 },
                    None,
                    30,
                )
            })
            .collect();
        let refs: Vec<&Row> = spiky.iter().collect();
        assert!(memory_flat(&stats(&refs, 0.0).unwrap()));
    }

    #[test]
    fn a_restarted_part_is_judged_by_its_last_process() {
        let mut st = State::default();
        for i in 0..10 {
            st.rows.push(Row {
                pid: 1,
                ..row(i as f64 * 60.0, APP, 100, None, 20)
            });
        }
        for i in 10..20 {
            st.rows.push(Row {
                pid: 2,
                ..row(i as f64 * 60.0, APP, 100, None, 60)
            });
        }
        let s = part_stats(&st, APP).unwrap();
        assert_eq!((s.pid, s.processes, s.samples), (2, 2, 10));
        assert!(files_flat(&s));
        assert_eq!(part_stats(&st, BRAIN), None);
    }

    #[test]
    fn outside_quits_are_told_apart_from_crashes() {
        let ok = ExitStatus::from_raw(0);
        let quit = "nixa-app: quit was asked from outside the app (Dock menu, a quit Apple Event or log out)\n";
        assert!(quit_reason(quit, ok).unwrap().contains("Dock menu"));
        let closed = "nixa-app: the main window was closed, so the app quits\n";
        assert!(quit_reason(closed, ok).unwrap().contains("close button"));
        assert!(quit_reason("", ExitStatus::from_raw(15)).unwrap().contains("TERM"));
        assert!(quit_reason("", ExitStatus::from_raw(9)).unwrap().contains("KILL"));
        assert_eq!(quit_reason("", ok), None);
        assert_eq!(quit_reason("", ExitStatus::from_raw(11)), None);
        assert_eq!(quit_reason("", ExitStatus::from_raw(1 << 8)), None);
        let mut st = State::default();
        st.closed_outside.push("+00:01:00 exit code 0: a quit came from outside".into());
        let end = Ending {
            ran: Duration::from_secs(60),
            ..Ending::default()
        };
        let found = problems(&st, &end, true);
        assert!(found.iter().any(|p| p.contains("closed from outside")), "{found:?}");
        assert!(!found.iter().any(|p| p.contains("nixa-app stopped")), "{found:?}");
    }

    #[test]
    fn exit_text_names_signals() {
        assert_eq!(exit_text(ExitStatus::from_raw(0)), "exit code 0");
        assert_eq!(exit_text(ExitStatus::from_raw(11)), "killed by signal 11");
    }
}
