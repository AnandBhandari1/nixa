use nixa_protocol::{client, control, paths, reply::Reply, request::Request};
use serde_json::Value;
use std::ffi::CStr;
use std::fs::{self, File};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PROBE: Duration = Duration::from_millis(350);
const STOP: Duration = Duration::from_secs(8);
const PAUSE: Duration = Duration::from_millis(150);

pub struct Runtime {
    pub data: PathBuf,
    pub addr: SocketAddr,
    pub socket: PathBuf,
    pub wait: Duration,
}

impl Runtime {
    pub fn window(&self) -> PathBuf {
        self.data.join(control::APP_SOCKET_FILE)
    }

    pub fn engine_pid(&self) -> Result<u32, String> {
        let ping = || -> anyhow::Result<Reply> {
            let mut client = client::connect(&self.socket)?;
            client.set_timeout(Some(PROBE))?;
            client.call(&Request::Ping)
        };
        match ping() {
            Ok(Reply::Pong { pid, .. }) if pid > 1 => Ok(pid),
            _ => Err("the existing terminal engine did not answer; nothing will restart it".into()),
        }
    }

    pub fn idle(&self, installed: &Path, slot: &Path) -> Result<(), String> {
        if TcpStream::connect_timeout(&self.addr, PROBE).is_ok()
            || std::os::unix::net::UnixStream::connect(self.window()).is_ok()
        {
            return Err("quit the Nixa window normally, then stop the old Nixa Brain normally. Leave the terminal engine running".into());
        }
        for pid in process_ids()? {
            if let Some(path) = process_path(pid)
                && [installed, slot].into_iter().any(|bundle| {
                    let macos = crate::layout::macos_dir(bundle);
                    path == macos.join("nixa-app")
                })
            {
                return Err(format!(
                    "{} (PID {pid}) is still running; stop it normally before retrying",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    pub fn retire(&self, installed: &Path, brain: u32, engine: u32) -> Result<(), String> {
        let bin = crate::layout::macos_dir(installed);
        let app = match control::call(&self.window(), &control::ControlRequest::Ping, PROBE) {
            Ok(control::ControlReply::Pong { pid, .. }) => Some(pid),
            _ => None,
        };
        let mut targets = Vec::new();
        if let Some(pid) = app {
            targets.push((pid, bin.join("nixa-app")));
        }
        targets.push((brain, bin.join("nixa-app")));
        for (pid, path) in &targets {
            if *pid <= 1 || *pid == engine || process_path(*pid as i32).as_deref() != Some(path) {
                return Err("the app or brain identity changed before restart".into());
            }
        }
        if self.engine_pid()? != engine {
            return Err("the terminal engine changed before restart".into());
        }
        for (pid, path) in targets {
            if process_path(pid as i32).as_deref() != Some(&path) {
                return Err("the process identity changed before shutdown".into());
            }
            if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
                return Err(format!("could not stop {} normally", path.display()));
            }
            let until = Instant::now() + STOP;
            while process_path(pid as i32).as_deref() == Some(&path) {
                if Instant::now() >= until {
                    return Err("the old process did not stop normally; no force kill was sent".into());
                }
                thread::sleep(PAUSE);
            }
        }
        self.idle(installed, installed)
    }

    fn start(&self, bin: &Path, nonce: &str, log: &File, isolated: bool) -> Result<Child, String> {
        let clone_log = || {
            log.try_clone()
                .map_err(|e| format!("cannot clone launch log: {e}"))
        };
        let cli = bin.with_file_name("nixa");
        let mut cmd = Command::new(bin);
        if isolated {
            cmd.env_clear()
                .env("HOME", &self.data)
                .env("PATH", "/usr/bin:/bin");
        }
        cmd.current_dir(&self.data)
            .stdin(Stdio::null())
            .stdout(clone_log()?)
            .stderr(clone_log()?)
            .process_group(0)
            .env(paths::ENV_DATA_DIR, &self.data)
            .env(paths::ENV_BRAIN_ADDR, self.addr.to_string())
            .env(paths::ENV_ENGINE_SOCKET, &self.socket)
            .env(paths::ENV_NIXA_BIN, cli)
            .env(paths::ENV_RELEASE_NONCE, nonce)
            .env(control::ENV_APP_SOCKET, self.window());
        for var in [
            paths::ENV_TERM_ID,
            "NIXA_AGENT",
            "NIXA_SOAK",
            "NIXA_APP_QUIT_AFTER_MS",
            "NIXA_BRAIN_QUIT_AFTER_MS",
        ] {
            cmd.env_remove(var);
        }
        cmd.spawn()
            .map_err(|e| format!("could not start {}: {e}", bin.display()))
    }

    pub fn launch(
        &self,
        installed: &Path,
        nonce: &str,
        engine: u32,
        allow_legacy: bool,
        log: &File,
        live: &mut Children,
    ) -> Result<(), String> {
        let bin = crate::layout::macos_dir(installed);
        live.brain = Some(self.start(&crate::layout::bin_dir(installed).join("nixa-brain"), nonce, log, false)?);
        let brain = live.brain.as_mut().unwrap();
        let brain_pid = brain.id();
        let until = Instant::now() + self.wait;
        while !brain_ready(self.addr, brain_pid, nonce, allow_legacy) {
            wait(
                brain,
                until,
                "brain did not report candidate identity and engine readiness",
            )?;
        }
        live.app = Some(self.start(&bin.join("nixa-app"), nonce, log, false)?);
        let app = live.app.as_mut().unwrap();
        let app_pid = app.id();
        let until = Instant::now() + self.wait;
        loop {
            let window = self.window();
            let identified = matches!(control::call(&window, &control::ControlRequest::Ping, PROBE), Ok(control::ControlReply::Pong { pid, .. }) if pid == app_pid);
            let attached = identified
                && matches!(control::call(&window, &control::ControlRequest::State, PROBE), Ok(control::ControlReply::State { state }) if state.engine == "up");
            if attached
                && self.engine_pid()? == engine
                && brain_ready(self.addr, brain_pid, nonce, allow_legacy)
            {
                return Ok(());
            }
            wait(app, until, "window did not attach to the original engine")?;
            wait(
                brain,
                until,
                "brain lost readiness before the window opened",
            )?;
        }
    }
}

pub fn preflight(installed: &Path, nonce: &str, data: &Path, log: &File) -> Result<(), String> {
    fs::create_dir(data).map_err(|e| format!("cannot make scratch data: {e}"))?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    drop(listener);
    let runtime = Runtime {
        data: data.to_owned(),
        addr,
        socket: data.join("engine.sock"),
        wait: Duration::from_secs(12),
    };
    let mut child = runtime.start(
        &crate::layout::bin_dir(installed).join("nixa-brain"),
        nonce,
        log,
        true,
    )?;
    let pid = child.id();
    let until = Instant::now() + runtime.wait;
    let check = loop {
        if health(addr).is_some_and(|body| health_matches(&body, pid, nonce, false)) {
            break Ok(());
        }
        if let Err(error) = wait(
            &mut child,
            until,
            "candidate did not identify itself on the scratch port",
        ) {
            break Err(error);
        }
    };
    terminate(&mut child).and(check)
}

fn wait(child: &mut Child, until: Instant, message: &str) -> Result<(), String> {
    if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
        return Err(format!("{message}: process exited with {status}"));
    }
    if Instant::now() >= until {
        return Err(message.into());
    }
    thread::sleep(PAUSE);
    Ok(())
}

fn health(addr: SocketAddr) -> Option<Value> {
    let out = Command::new("/usr/bin/curl")
        .args([
            "-q",
            "--noproxy",
            "*",
            "--silent",
            "--fail",
            "--max-time",
            "1",
        ])
        .arg(format!("http://{addr}/health"))
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.len() > 64 * 1024 {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

fn health_matches(body: &Value, pid: u32, nonce: &str, engine: bool) -> bool {
    body["ok"] == true
        && body["engine"] == engine
        && body["releaseNonce"] == nonce
        && body["releasePid"] == pid
}

fn brain_ready(addr: SocketAddr, pid: u32, nonce: &str, allow_legacy: bool) -> bool {
    let Some(body) = health(addr) else {
        return false;
    };
    health_matches(&body, pid, nonce, true)
        || (allow_legacy
            && body["ok"] == true
            && body["engine"] == true
            && body.get("releaseNonce").is_none()
            && Command::new("/usr/sbin/lsof")
                .args([
                    "-nP",
                    "-a",
                    "-p",
                    &pid.to_string(),
                    &format!("-iTCP:{}", addr.port()),
                    "-sTCP:LISTEN",
                    "-t",
                ])
                .output()
                .is_ok_and(|out| {
                    out.status.success()
                        && String::from_utf8_lossy(&out.stdout).trim() == pid.to_string()
                }))
}

fn process_ids() -> Result<Vec<i32>, String> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if !(1..=100_000).contains(&count) {
        return Err("cannot enumerate processes safely".into());
    }
    let mut pids = vec![0i32; count as usize + 256];
    let found = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            (pids.len() * size_of::<i32>()) as i32,
        )
    };
    if found <= 0 || found as usize >= pids.len() {
        return Err("the process list could not be read without truncation".into());
    }
    pids.truncate(found as usize);
    Ok(pids)
}

pub(crate) fn process_path(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let result = unsafe { libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    if result <= 0 {
        return None;
    }
    let c = CStr::from_bytes_until_nul(&bytes).ok()?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(c.to_bytes())))
}

#[derive(Default)]
pub struct Children {
    app: Option<Child>,
    brain: Option<Child>,
}

impl Children {
    pub fn empty(&self) -> bool {
        self.app.is_none() && self.brain.is_none()
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if let Some(child) = &mut self.app {
            terminate(child)?;
        }
        if let Some(child) = &mut self.brain {
            terminate(child)?;
        }
        Ok(())
    }
}

fn terminate(child: &mut Child) -> Result<(), String> {
    if child.try_wait().map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    let sent = unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    if sent != 0 && child.try_wait().map_err(|e| e.to_string())?.is_none() {
        return Err(format!("could not stop new PID {}", child.id()));
    }
    let until = Instant::now() + STOP;
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() >= until {
            return Err(format!("new PID {} would not stop", child.id()));
        }
        thread::sleep(PAUSE);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_refuses_a_stale_process_and_a_wrong_engine() {
        let mut value =
            serde_json::json!({"ok":true,"engine":true,"releaseNonce":"current","releasePid":7});
        assert!(health_matches(&value, 7, "current", true));
        assert!(!health_matches(&value, 8, "current", true));
        assert!(!health_matches(&value, 7, "old", true));
        value["engine"] = false.into();
        assert!(!health_matches(&value, 7, "current", true));
    }

    #[test]
    fn process_lookup_finds_the_test_binary() {
        let pid = std::process::id() as i32;
        assert!(process_ids().unwrap().contains(&pid));
        assert_eq!(
            process_path(pid).unwrap(),
            std::env::current_exe().unwrap().canonicalize().unwrap()
        );
    }
}
