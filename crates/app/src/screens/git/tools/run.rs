use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub type Cancel = Arc<AtomicBool>;

pub const READ_WAIT: Duration = Duration::from_secs(20);
pub const LONG_WAIT: Duration = Duration::from_secs(900);
pub const JOB_WAIT: Duration = Duration::from_secs(600);
pub const LOCALE: [(&str, &str); 2] = [("LC_ALL", "C"), ("LANG", "C")];

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
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn first_line(&self) -> String {
        self.stdout
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    }
}

pub fn new_cancel() -> Cancel {
    Arc::new(AtomicBool::new(false))
}

pub fn git(root: &Path, args: &[&str]) -> Out {
    git_with(root, args, &[], READ_WAIT, None)
}

pub fn read(root: &Path, args: &[&str]) -> Out {
    let mut all = vec!["--no-optional-locks"];
    all.extend_from_slice(args);
    git(root, &all)
}

pub fn git_with(
    root: &Path,
    args: &[&str],
    env: &[(String, String)],
    wait: Duration,
    cancel: Option<&Cancel>,
) -> Out {
    let mut all: Vec<(String, String)> = LOCALE.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    all.extend(env.iter().cloned());
    let job = nixa_git::run::Job {
        cancel: cancel.cloned(),
        timeout: Some(wait),
        env: all,
    };
    let out = nixa_git::run::job(root, args, &job);
    let stderr = if out.timed_out {
        format!("git took longer than {} s and was stopped", wait.as_secs())
    } else {
        out.stderr
    };
    Out {
        code: out.code,
        stdout: out.stdout,
        stderr,
        cancelled: out.cancelled,
        timed_out: out.timed_out,
    }
}
