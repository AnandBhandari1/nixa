use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const GIT_ENV: [(&str, &str); 4] = [
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GCM_INTERACTIVE", "never"),
    ("GIT_EDITOR", "true"),
    ("GIT_MERGE_AUTOEDIT", "no"),
];

pub const SSH_BATCH: &str = "ssh -o BatchMode=yes";

pub const JOB_SECS: u64 = 600;

const POLL: Duration = Duration::from_millis(25);

const DRAIN: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Out {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub cancelled: bool,
    pub timed_out: bool,
    pub failed_to_start: bool,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    pub fn said(&self) -> String {
        said(self)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Job {
    pub cancel: Option<Arc<AtomicBool>>,
    pub timeout: Option<Duration>,
    pub env: Vec<(String, String)>,
}

impl Job {
    pub fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.load(Ordering::SeqCst))
    }
}

pub fn said(out: &Out) -> String {
    [out.stderr.as_str(), out.stdout.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.trim().split('\n').filter(|l| !l.is_empty()).collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

pub fn failure_text(out: &Out, args: &[&str]) -> String {
    let text = tail(if out.stderr.trim().is_empty() { &out.stdout } else { &out.stderr }, 6);
    if !text.is_empty() {
        return text;
    }
    let code = out.code.map(|c| c.to_string()).unwrap_or_else(|| "none".into());
    format!("git {} failed ({code})", args.first().copied().unwrap_or(""))
}

fn command(root: &Path, args: &[&str], env: &[(String, String)]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(root).stdin(Stdio::null());
    for (key, value) in GIT_ENV {
        cmd.env(key, value);
    }
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub fn git(root: &Path, args: &[&str]) -> Out {
    git_env(root, args, &[])
}

pub fn git_env(root: &Path, args: &[&str], env: &[(String, String)]) -> Out {
    match command(root, args, env).output() {
        Ok(out) => Out {
            code: out.status.code(),
            stdout: lossy(&out.stdout),
            stderr: lossy(&out.stderr),
            ..Out::default()
        },
        Err(error) => Out { stderr: format!("cannot run git: {error}"), failed_to_start: true, ..Out::default() },
    }
}

pub fn git_ok(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = git(root, args);
    if out.ok() { Ok(out.stdout) } else { Err(failure_text(&out, args)) }
}

pub fn read(root: &Path, args: &[&str]) -> Out {
    let mut all = vec!["--no-optional-locks"];
    all.extend_from_slice(args);
    git(root, &all)
}

pub fn read_ok(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut all = vec!["--no-optional-locks"];
    all.extend_from_slice(args);
    git_ok(root, &all)
}

fn ssh_env(root: &Path, env: &[(String, String)]) -> Vec<(String, String)> {
    let mut all = env.to_vec();
    let set = |key: &str| env.iter().any(|(k, _)| k == key) || std::env::var_os(key).is_some();
    if !set("GIT_SSH_COMMAND") && !set("GIT_SSH") && !git(root, &["config", "--get", "core.sshCommand"]).ok() {
        all.push(("GIT_SSH_COMMAND".into(), SSH_BATCH.into()));
    }
    all
}

fn reader(mut pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = send.send(buf);
    });
    recv
}

fn stop(child: &mut Child) {
    let pid = child.id() as i32;
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    let began = Instant::now();
    while began.elapsed() < Duration::from_millis(500) {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        std::thread::sleep(POLL);
    }
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn job(root: &Path, args: &[&str], job: &Job) -> Out {
    use std::os::unix::process::CommandExt;
    if job.cancelled() {
        return Out { cancelled: true, ..Out::default() };
    }
    let env = ssh_env(root, &job.env);
    let mut cmd = command(root, args, &env);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return Out { stderr: format!("cannot run git: {error}"), failed_to_start: true, ..Out::default() },
    };
    let out_rx = reader(child.stdout.take());
    let err_rx = reader(child.stderr.take());
    let limit = job.timeout.unwrap_or(Duration::from_secs(JOB_SECS));
    let began = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if job.cancelled() {
            stop(&mut child);
            return Out { cancelled: true, ..Out::default() };
        }
        if began.elapsed() > limit {
            stop(&mut child);
            return Out { timed_out: true, ..Out::default() };
        }
        std::thread::sleep(POLL);
    };
    let stdout = out_rx.recv_timeout(DRAIN).unwrap_or_default();
    let stderr = err_rx.recv_timeout(DRAIN).unwrap_or_default();
    Out {
        code: status.and_then(|s| s.code()),
        stdout: lossy(&stdout),
        stderr: lossy(&stderr),
        ..Out::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn said_joins_both_streams() {
        let out = Out { stderr: "bad\n".into(), stdout: "  ".into(), ..Out::default() };
        assert_eq!(said(&out), "bad\n");
        assert_eq!(tail("a\n\nb\nc\n", 2), "b\nc");
    }

    #[test]
    fn job_stops_on_cancel_and_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        let out = job(dir.path(), &["version"], &Job { cancel: Some(cancel), ..Job::default() });
        assert!(out.cancelled);
        let out = job(dir.path(), &["version"], &Job::default());
        assert!(out.ok());
        assert!(out.stdout.starts_with("git version"));
        let slow = Job { timeout: Some(Duration::from_millis(100)), env: vec![("GIT_TRACE".into(), "0".into())], ..Job::default() };
        let out = job(dir.path(), &["-c", "alias.wait=!sleep 5", "wait"], &slow);
        assert!(out.timed_out);
    }

    #[test]
    fn failure_text_names_the_command() {
        let out = Out { code: Some(2), ..Out::default() };
        assert_eq!(failure_text(&out, &["push"]), "git push failed (2)");
    }
}
