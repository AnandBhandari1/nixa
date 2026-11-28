use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub struct Repo(pub PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Repo {
    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .env("GIT_AUTHOR_DATE", "1700000000 +0000")
            .env("GIT_COMMITTER_DATE", "1700000000 +0000")
            .env("GIT_EDITOR", "true")
            .env("LC_ALL", "C")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn try_git(&self, args: &[&str]) -> bool {
        Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .env("GIT_EDITOR", "true")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn write(&self, file: &str, text: &str) {
        let path = self.0.join(file);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    pub fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.0.join(file)).unwrap()
    }

    pub fn commit(&self, file: &str, text: &str, message: &str) -> String {
        self.write(file, text);
        self.git(&["add", "--", file]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }
}

pub fn repo() -> Repo {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("mgt-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let r = Repo(dir);
    r.git(&["init", "-q", "-b", "main"]);
    r.git(&["config", "user.name", "Test Person"]);
    r.git(&["config", "user.email", "test@example.com"]);
    r.git(&["config", "commit.gpgsign", "false"]);
    r.git(&["config", "core.hooksPath", "/dev/null"]);
    r
}
