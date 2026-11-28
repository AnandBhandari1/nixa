use std::process::Stdio;
use std::time::Duration;

pub const GIT_PASS: [&str; 4] = ["GIT_SSH_COMMAND", "GIT_CONFIG_GLOBAL", "GH_CONFIG_DIR", "XDG_CONFIG_HOME"];
pub const GIT_ENV: [(&str, &str); 5] = [("GIT_TERMINAL_PROMPT", "0"), ("GIT_EDITOR", "true"), ("GIT_SEQUENCE_EDITOR", "true"), ("GCM_INTERACTIVE", "never"), ("LC_ALL", "C")];
pub const READ_MS: u64 = 20_000;
pub const LONG_MS: u64 = 180_000;
pub const MAX_OUTPUT: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    pub fn text(&self) -> Option<String> {
        self.ok().then(|| self.stdout.clone())
    }

    pub fn last_line(&self) -> String {
        let all = if self.stderr.trim().is_empty() { &self.stdout } else { &self.stderr };
        all.trim().lines().last().map(str::to_string).unwrap_or_else(|| format!("git exited {}", self.code))
    }
}

fn cut(bytes: &[u8]) -> String {
    let end = bytes.len().min(MAX_OUTPUT);
    String::from_utf8_lossy(&bytes[..end]).to_string()
}

pub async fn git_args(root: &str, args: &[String], ms: u64) -> Out {
    let mut command = tokio::process::Command::new("git");
    command.arg("-C").arg(root).args(args).env_clear().envs(crate::review_flow::checks::safe_env(&GIT_PASS)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    for (key, value) in GIT_ENV {
        command.env(key, value);
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return Out { code: 127, stdout: String::new(), stderr: format!("git did not start: {e}") },
    };
    match tokio::time::timeout(Duration::from_millis(ms), child.wait_with_output()).await {
        Ok(Ok(out)) => Out { code: out.status.code().unwrap_or(1), stdout: cut(&out.stdout), stderr: cut(&out.stderr) },
        Ok(Err(e)) => Out { code: 1, stdout: String::new(), stderr: format!("git failed: {e}") },
        Err(_) => Out { code: 124, stdout: String::new(), stderr: format!("git took longer than {} seconds and was stopped", ms / 1000) },
    }
}

pub async fn git(root: &str, args: &[&str], ms: u64) -> Out {
    let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    git_args(root, &owned, ms).await
}

pub async fn read(root: &str, args: &[&str]) -> Option<String> {
    git(root, args, READ_MS).await.text()
}
