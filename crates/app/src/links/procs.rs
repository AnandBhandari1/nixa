use anyhow::{Context, Result, anyhow};
use nixa_protocol::{Reply, Request, connect, paths};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const LOG: &str = "brain.log";
pub const RESTART_GAP: Duration = Duration::from_secs(60);
pub const HANG_LIMIT: Duration = Duration::from_secs(30);
pub const TICK: Duration = Duration::from_secs(2);
const PROBE_TIMEOUT: Duration = Duration::from_millis(800);
const BRAIN_STOP_WAIT: Duration = Duration::from_secs(5);
const REFUSED_PORTS: [u16; 3] = [7321, 7330, 7331];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Engine,
    Brain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Checking,
    Starting,
    Running { pid: Option<u32>, ours: bool },
    NoAnswer,
    Dead { why: String, retry_in: Duration },
    Stopped,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Good,
    Busy,
    Bad,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Procs {
    pub engine: State,
    pub brain: State,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    pub socket: PathBuf,
    pub brain_addr: String,
    pub gap: Duration,
    pub hang: Duration,
    pub tick: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    RestartBrain,
    StopBrain,
    Shutdown,
}

enum Runner {
    Engine(nixa_engine::Running),
    Brain(nixa_brain::Handle),
}

struct Slot {
    kind: Kind,
    runner: Option<Runner>,
    started: Option<Instant>,
    last_ok: Option<Instant>,
    why: String,
    stopped: bool,
    state: State,
}

pub struct Keeper {
    cfg: Config,
    engine: Slot,
    brain: Slot,
}

static QUIT: Mutex<Option<(Sender<Cmd>, Receiver<()>)>> = Mutex::new(None);

pub fn config() -> Result<Config> {
    Ok(Config {
        data_dir: paths::data_dir(),
        socket: paths::socket_path(),
        brain_addr: paths::brain_addr(),
        gap: RESTART_GAP,
        hang: HANG_LIMIT,
        tick: TICK,
    })
}

pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Engine => "terminal engine",
        Kind::Brain => "brain",
    }
}

pub fn log_path(cfg: &Config) -> PathBuf {
    cfg.data_dir.join(LOG)
}

pub fn open_log(cfg: &Config) -> Result<()> {
    paths::ensure_private_dir(&cfg.data_dir)
        .with_context(|| format!("cannot make {}", cfg.data_dir.display()))?;
    let log = log_path(cfg);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log)
        .with_context(|| format!("cannot open {}", log.display()))?;
    fs::set_permissions(&log, fs::Permissions::from_mode(0o600))?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    writeln!(file, "nixa starts at unix time {secs}")?;
    if unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
        return Err(anyhow!("cannot send the log to {}", log.display()));
    }
    nixa_brain::log_file::watch(&cfg.data_dir);
    Ok(())
}

pub fn check_addr(addr: &str) -> Result<SocketAddr, String> {
    let found = addr
        .to_socket_addrs()
        .map_err(|e| format!("bad brain address {addr}: {e}"))?
        .next()
        .ok_or_else(|| format!("bad brain address {addr}"))?;
    if !found.ip().is_loopback() {
        return Err(format!("brain address {addr} is not on this Mac only"));
    }
    if REFUSED_PORTS.contains(&found.port()) {
        return Err(format!("brain port {} is refused", found.port()));
    }
    Ok(found)
}

pub fn engine_pid(socket: &Path) -> Option<u32> {
    let mut client = connect(socket).ok()?;
    client.set_timeout(Some(PROBE_TIMEOUT)).ok()?;
    match client.call(&Request::Ping).ok()? {
        Reply::Pong { pid, .. } => Some(pid),
        _ => None,
    }
}

pub fn brain_answers(addr: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return false;
    };
    if stream.set_read_timeout(Some(PROBE_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(PROBE_TIMEOUT)).is_err()
    {
        return false;
    }
    let ask = format!("GET /health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if stream.write_all(ask.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    let mut got = 0;
    while got < head.len() {
        match stream.read(&mut head[got..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
    }
    let head = String::from_utf8_lossy(&head[..got]);
    head.starts_with("HTTP/1.") && head.get(9..12) == Some("200")
}

fn probe(cfg: &Config, kind: Kind) -> Option<Option<u32>> {
    match kind {
        Kind::Engine => engine_pid(&cfg.socket).map(Some),
        Kind::Brain => {
            let addr = check_addr(&cfg.brain_addr).ok()?;
            brain_answers(addr).then_some(None)
        }
    }
}

fn listening(cfg: &Config, kind: Kind) -> bool {
    match kind {
        Kind::Engine => UnixStream::connect(&cfg.socket).is_ok(),
        Kind::Brain => check_addr(&cfg.brain_addr)
            .is_ok_and(|addr| TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok()),
    }
}

fn start(cfg: &Config, kind: Kind) -> Result<Option<Runner>> {
    match kind {
        Kind::Engine if cfg!(test) => Ok(Some(Runner::Engine(nixa_engine::start(cfg.socket.clone())?))),
        Kind::Engine => spawn_engine(cfg).map(|_| None),
        Kind::Brain => Ok(Some(Runner::Brain(nixa_brain::start()?))),
    }
}

fn spawn_engine(cfg: &Config) -> Result<()> {
    let exe = std::env::current_exe().context("cannot find the Nixa program")?;
    let log = cfg.socket.with_file_name("engine.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log)
        .with_context(|| format!("cannot open {}", log.display()))?;
    let mut child = std::process::Command::new(exe)
        .arg("engine")
        .arg("--socket")
        .arg(&cfg.socket)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file.try_clone()?))
        .stderr(std::process::Stdio::from(file))
        .process_group(0)
        .spawn()
        .context("cannot start the terminal engine")?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn stop_runner(slot: &mut Slot) -> bool {
    match slot.runner.take() {
        Some(Runner::Brain(mut handle)) => nixa_brain::stop(&mut handle, BRAIN_STOP_WAIT),
        Some(Runner::Engine(running)) => {
            nixa_engine::stop(&running);
            true
        }
        None => true,
    }
}

fn slot(kind: Kind) -> Slot {
    Slot {
        kind,
        runner: None,
        started: None,
        last_ok: None,
        why: "not running".to_string(),
        stopped: false,
        state: State::Checking,
    }
}

pub fn new(cfg: Config) -> Keeper {
    Keeper {
        cfg,
        engine: slot(Kind::Engine),
        brain: slot(Kind::Brain),
    }
}

pub fn snapshot(keeper: &Keeper) -> Procs {
    Procs {
        engine: keeper.engine.state.clone(),
        brain: keeper.brain.state.clone(),
    }
}

pub fn tick(keeper: &mut Keeper, now: Instant) -> bool {
    let before = snapshot(keeper);
    tick_slot(&keeper.cfg, &mut keeper.engine, now);
    tick_slot(&keeper.cfg, &mut keeper.brain, now);
    snapshot(keeper) != before
}

fn watch_runner(cfg: &Config, slot: &mut Slot, now: Instant) -> bool {
    let ended = match slot.runner.as_ref() {
        None => return false,
        Some(Runner::Brain(handle)) => nixa_brain::finished(handle),
        Some(Runner::Engine(_)) => false,
    };
    if ended {
        slot.runner = None;
        slot.why = format!("the {} stopped", kind_name(slot.kind));
        return false;
    }
    if probe(cfg, slot.kind).is_some() {
        slot.last_ok = Some(now);
        slot.state = State::Running {
            pid: Some(std::process::id()),
            ours: true,
        };
        return true;
    }
    let since = slot.last_ok.or(slot.started).unwrap_or(now);
    if slot.kind == Kind::Brain && now.duration_since(since) >= cfg.hang {
        stop_runner(slot);
        slot.why = format!(
            "the {} did not answer for {} s",
            kind_name(slot.kind),
            cfg.hang.as_secs()
        );
        return false;
    }
    slot.state = if slot.last_ok.is_some() {
        State::NoAnswer
    } else {
        State::Starting
    };
    true
}

fn tick_slot(cfg: &Config, slot: &mut Slot, now: Instant) {
    if slot.kind == Kind::Brain
        && let Err(why) = check_addr(&cfg.brain_addr)
    {
        slot.state = State::Failed(why);
        return;
    }
    if watch_runner(cfg, slot, now) {
        return;
    }
    if let Some(pid) = probe(cfg, slot.kind) {
        slot.last_ok = Some(now);
        slot.state = State::Running { pid, ours: false };
        return;
    }
    if slot.stopped {
        slot.state = State::Stopped;
        return;
    }
    if listening(cfg, slot.kind) {
        slot.state = State::NoAnswer;
        return;
    }
    if slot.kind == Kind::Engine && std::env::var_os(paths::ENV_RELEASE_NONCE).is_some() {
        slot.state = State::Failed("release launch requires the existing terminal engine".into());
        return;
    }
    if let Some(at) = slot.started {
        let gone = now.duration_since(at);
        if gone < cfg.gap {
            slot.state = State::Dead {
                why: slot.why.clone(),
                retry_in: round_up(cfg.gap - gone),
            };
            return;
        }
    }
    slot.started = Some(now);
    slot.last_ok = None;
    match start(cfg, slot.kind) {
        Ok(runner) => {
            slot.runner = runner;
            slot.state = State::Starting;
        }
        Err(e) => {
            slot.why = format!("{e:#}");
            slot.state = State::Failed(slot.why.clone());
        }
    }
}

fn round_up(d: Duration) -> Duration {
    Duration::from_secs(d.as_secs() + u64::from(d.subsec_nanos() > 0))
}

pub fn command(keeper: &mut Keeper, cmd: Cmd, now: Instant) -> bool {
    match cmd {
        Cmd::Shutdown => {
            stop_runner(&mut keeper.brain);
            stop_runner(&mut keeper.engine);
            keeper.brain.stopped = true;
            keeper.engine.stopped = true;
        }
        Cmd::StopBrain => {
            stop_runner(&mut keeper.brain);
            keeper.brain.stopped = true;
            keeper.brain.why = "stopped from the app".to_string();
        }
        Cmd::RestartBrain => {
            stop_runner(&mut keeper.brain);
            keeper.brain.stopped = false;
            keeper.brain.started = None;
        }
    }
    tick(keeper, now)
}

pub fn spawn_wake(
    cfg: Config,
    wake: futures::channel::mpsc::UnboundedSender<()>,
) -> (Receiver<Procs>, Sender<Cmd>) {
    let (state_tx, state_rx) = channel();
    let (cmd_tx, cmd_rx) = channel();
    let (done_tx, done_rx) = channel();
    *QUIT.lock().unwrap_or_else(|e| e.into_inner()) = Some((cmd_tx.clone(), done_rx));
    thread::Builder::new()
        .name("nixa-procs".into())
        .spawn(move || run(new(cfg), state_tx, cmd_rx, wake, done_tx))
        .ok();
    (state_rx, cmd_tx)
}

pub fn shutdown() {
    let held = QUIT.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((tx, done)) = held
        && tx.send(Cmd::Shutdown).is_ok()
    {
        let _ = done.recv_timeout(BRAIN_STOP_WAIT + Duration::from_secs(2));
    }
}

fn run(
    mut keeper: Keeper,
    tx: Sender<Procs>,
    rx: Receiver<Cmd>,
    wake: futures::channel::mpsc::UnboundedSender<()>,
    done: Sender<()>,
) {
    let mut changed = true;
    tick(&mut keeper, Instant::now());
    loop {
        if changed {
            if tx.send(snapshot(&keeper)).is_err() {
                return;
            }
            let _ = wake.unbounded_send(());
        }
        changed = match rx.recv_timeout(keeper.cfg.tick) {
            Ok(Cmd::Shutdown) => {
                command(&mut keeper, Cmd::Shutdown, Instant::now());
                let _ = done.send(());
                return;
            }
            Ok(cmd) => command(&mut keeper, cmd, Instant::now()),
            Err(RecvTimeoutError::Timeout) => tick(&mut keeper, Instant::now()),
            Err(RecvTimeoutError::Disconnected) => return,
        };
    }
}

pub fn label(kind: Kind, state: &State) -> (Level, String) {
    let name = match kind {
        Kind::Engine => "Engine",
        Kind::Brain => "Brain",
    };
    let (level, text) = match state {
        State::Checking => (Level::Busy, "checking".to_string()),
        State::Starting => (Level::Busy, "starting".to_string()),
        State::Running { .. } => (Level::Good, "on".to_string()),
        State::NoAnswer => (Level::Busy, "not answering".to_string()),
        State::Dead { retry_in, .. } => (
            Level::Bad,
            format!("down, restart in {} s", retry_in.as_secs()),
        ),
        State::Stopped => (Level::Bad, "off".to_string()),
        State::Failed(_) => (Level::Bad, "failed".to_string()),
    };
    (level, format!("{name}: {text}"))
}

pub fn detail(cfg: &Config, kind: Kind, state: &State) -> String {
    let log = log_path(cfg);
    let name = kind_name(kind);
    match state {
        State::Running { ours: true, .. } => {
            format!("The {name} runs inside Nixa. Log: {}", log.display())
        }
        State::Running { ours: false, .. } => {
            format!("The {name} runs, started outside the app")
        }
        State::Dead { why, retry_in } => format!(
            "{why}. Next start in {} s. Log: {}",
            retry_in.as_secs(),
            log.display()
        ),
        State::Failed(why) => format!("{why}. Log: {}", log.display()),
        other => format!("The {name}: {other:?}. Log: {}", log.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        dir: PathBuf,
        cfg: Config,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn fixture() -> Fixture {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("nixa-procs-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cfg = Config {
            data_dir: dir.clone(),
            socket: dir.join("engine.sock"),
            brain_addr: format!("127.0.0.1:{}", free_port()),
            gap: Duration::from_secs(60),
            hang: Duration::from_secs(30),
            tick: Duration::from_millis(50),
        };
        Fixture { dir, cfg }
    }

    #[test]
    fn refuses_live_ports_and_open_hosts() {
        assert!(check_addr("127.0.0.1:7421").is_ok());
        assert!(check_addr("localhost:7421").is_ok());
        assert!(check_addr("127.0.0.1:7321").is_err());
        assert!(check_addr("127.0.0.1:7330").is_err());
        assert!(check_addr("127.0.0.1:7331").is_err());
        assert!(check_addr("0.0.0.0:7421").is_err());
        assert!(check_addr("nonsense").is_err());
    }

    #[test]
    fn a_refused_brain_port_is_never_started() {
        let mut fx = fixture();
        fx.cfg.brain_addr = "127.0.0.1:7321".to_string();
        let mut keeper = new(fx.cfg.clone());
        keeper.engine.stopped = true;
        tick(&mut keeper, Instant::now());
        assert!(matches!(keeper.brain.state, State::Failed(_)));
        assert!(keeper.brain.runner.is_none());
    }

    #[test]
    fn the_engine_runs_inside_the_app_and_answers() {
        let fx = fixture();
        let mut keeper = new(fx.cfg.clone());
        keeper.brain.stopped = true;
        tick(&mut keeper, Instant::now());
        assert!(matches!(keeper.engine.runner, Some(Runner::Engine(_))));
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until
            && !matches!(keeper.engine.state, State::Running { ours: true, .. })
        {
            thread::sleep(Duration::from_millis(50));
            tick(&mut keeper, Instant::now());
        }
        assert_eq!(
            keeper.engine.state,
            State::Running {
                pid: Some(std::process::id()),
                ours: true
            }
        );
        assert_eq!(engine_pid(&fx.cfg.socket), Some(std::process::id()));
        command(&mut keeper, Cmd::Shutdown, Instant::now());
        assert!(keeper.engine.runner.is_none());
        assert!(!fx.cfg.socket.exists());
    }

    #[test]
    fn stop_and_restart_brain_change_only_the_brain_flag() {
        let fx = fixture();
        let mut keeper = new(fx.cfg.clone());
        keeper.engine.stopped = true;
        command(&mut keeper, Cmd::StopBrain, Instant::now());
        assert_eq!(keeper.brain.state, State::Stopped);
        assert!(keeper.brain.stopped);
        keeper.brain.stopped = true;
        command(&mut keeper, Cmd::RestartBrain, Instant::now());
        assert!(!keeper.brain.stopped);
    }

    #[test]
    fn labels_and_details_name_the_state() {
        assert_eq!(
            label(Kind::Brain, &State::Stopped),
            (Level::Bad, "Brain: off".to_string())
        );
        assert_eq!(
            label(Kind::Engine, &State::Running { pid: Some(1), ours: true }).0,
            Level::Good
        );
        let fx = fixture();
        let text = detail(&fx.cfg, Kind::Brain, &State::Running { pid: Some(1), ours: true });
        assert!(text.contains("runs inside Nixa"), "{text}");
    }
}
