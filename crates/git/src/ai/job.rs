use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub type Cancel = Arc<AtomicBool>;

pub const TICK: Duration = Duration::from_millis(40);
pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct Spec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub stdin: Option<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub cancelled: bool,
    pub timed_out: bool,
}

pub fn cancel() -> Cancel {
    Arc::new(AtomicBool::new(false))
}

fn reader(mut from: impl Read + Send + 'static) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match from.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = MAX_OUTPUT.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n.min(room)]);
                }
            }
        }
        String::from_utf8_lossy(&kept).into_owned()
    })
}

fn kill_group(child: &mut Child) {
    let group = format!("-{}", child.id());
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &group])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

pub fn run(spec: &Spec, stop: &AtomicBool) -> io::Result<Outcome> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(if spec.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn()?;
    let feeder = match (child.stdin.take(), spec.stdin.clone()) {
        (Some(mut pipe), Some(text)) => Some(thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        })),
        _ => None,
    };
    let out = child.stdout.take().map(reader);
    let err = child.stderr.take().map(reader);
    let began = Instant::now();
    let mut outcome = Outcome::default();
    loop {
        if let Some(status) = child.try_wait()? {
            outcome.code = status.code();
            break;
        }
        if stop.load(Ordering::SeqCst) {
            outcome.cancelled = true;
            break;
        }
        if began.elapsed() >= spec.timeout {
            outcome.timed_out = true;
            break;
        }
        thread::sleep(TICK);
    }
    kill_group(&mut child);
    let _ = child.wait();
    if let Some(f) = feeder {
        let _ = f.join();
    }
    let ended = outcome.code.is_some();
    outcome.stdout = out.and_then(|h| h.join().ok()).unwrap_or_default();
    outcome.stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
    if !ended {
        outcome.stdout.clear();
        outcome.stderr.clear();
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str, timeout_ms: u64) -> Spec {
        Spec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            timeout: Duration::from_millis(timeout_ms),
            ..Spec::default()
        }
    }

    #[test]
    fn returns_output_and_code() {
        let mut spec = sh("cat; echo err >&2; exit 3", 5000);
        spec.stdin = Some("hello".into());
        let out = run(&spec, &AtomicBool::new(false)).unwrap();
        assert_eq!(out.code, Some(3));
        assert_eq!(out.stdout, "hello");
        assert_eq!(out.stderr, "err\n");
    }

    #[test]
    fn times_out_and_kills_children() {
        let begin = Instant::now();
        let out = run(&sh("sleep 30 & sleep 30", 300), &AtomicBool::new(false)).unwrap();
        assert!(out.timed_out);
        assert_eq!(out.code, None);
        assert!(begin.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn stops_when_cancelled() {
        let stop = cancel();
        let flag = stop.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            flag.store(true, Ordering::SeqCst);
        });
        let out = run(&sh("sleep 30", 20_000), &stop).unwrap();
        assert!(out.cancelled);
        assert!(!out.timed_out);
    }

    #[test]
    fn missing_program_is_an_error() {
        let spec = Spec {
            program: "/nonexistent/nixa-git-ai-test".into(),
            timeout: Duration::from_secs(1),
            ..Spec::default()
        };
        let err = run(&spec, &AtomicBool::new(false)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn env_and_cwd_are_used() {
        let mut spec = sh("printf '%s %s' \"$NIXA_X\" \"$(pwd -P)\"", 5000);
        spec.env.push(("NIXA_X".into(), "yes".into()));
        spec.cwd = Some("/".into());
        let out = run(&spec, &AtomicBool::new(false)).unwrap();
        assert_eq!(out.stdout, "yes /");
    }
}
