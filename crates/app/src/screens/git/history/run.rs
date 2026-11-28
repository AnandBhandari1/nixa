use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub const JOB_LIMIT: Duration = Duration::from_secs(600);
pub const READ_FLAGS: [&str; 5] = [
    "--no-optional-locks",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.quotepath=off",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Out {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub cancelled: bool,
    pub timed_out: bool,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    pub fn said(&self) -> String {
        [self.stderr.trim(), self.stdout.trim()]
            .iter()
            .filter(|p| !p.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn line(&self) -> String {
        self.stdout.trim().to_string()
    }
}

pub type Cancel = Arc<AtomicBool>;

pub fn cancel_flag() -> Cancel {
    Arc::new(AtomicBool::new(false))
}

fn command(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
    let mut c = Command::new("git");
    c.current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    for (k, v) in env {
        c.env(k, v);
    }
    c
}

fn failed(e: std::io::Error) -> Out {
    Out {
        code: None,
        stderr: format!("cannot start git: {e}"),
        ..Out::default()
    }
}

pub fn git_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Out {
    match command(root, args, env).output() {
        Ok(o) => Out {
            code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            ..Out::default()
        },
        Err(e) => failed(e),
    }
}

pub fn git(root: &Path, args: &[&str]) -> Out {
    git_env(root, args, &[])
}

pub fn read(root: &Path, args: &[&str]) -> Out {
    let mut all: Vec<&str> = READ_FLAGS.to_vec();
    all.extend_from_slice(args);
    git(root, &all)
}

pub fn read_ok(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = read(root, args);
    if out.ok() {
        Ok(out.stdout)
    } else {
        Err(non_empty(out.said(), "git failed"))
    }
}

pub fn non_empty(text: String, fallback: &str) -> String {
    if text.trim().is_empty() { fallback.to_string() } else { text }
}

fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

pub fn job(root: &Path, args: &[&str], cancel: Option<&Cancel>, limit: Duration) -> Out {
    let mut c = command(root, args, &[]);
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match c.spawn() {
        Ok(child) => child,
        Err(e) => return failed(e),
    };
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let start = Instant::now();
    let mut result = Out::default();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                result.code = status.code();
                break;
            }
            Ok(None) => {}
            Err(e) => {
                result.stderr = e.to_string();
                break;
            }
        }
        let stop = cancel.is_some_and(|c| c.load(Ordering::SeqCst));
        if stop || start.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            result.cancelled = stop;
            result.timed_out = !stop;
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    result.stdout = out.and_then(|h| h.join().ok()).unwrap_or_default();
    let tail = err.and_then(|h| h.join().ok()).unwrap_or_default();
    result.stderr = format!("{}{}", result.stderr, tail);
    result
}

pub fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn split_z(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
