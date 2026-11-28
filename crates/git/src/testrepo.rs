use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Repo {
    pub dir: tempfile::TempDir,
}

impl Repo {
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    pub fn write(&self, rel: &str, text: &str) {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap()
    }

    pub fn git(&self, args: &[&str]) -> String {
        sh_git(self.root(), args)
    }

    pub fn commit_file(&self, rel: &str, text: &str, message: &str) -> String {
        self.write(rel, text);
        self.git(&["add", "--", rel]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }
}

pub fn sh_git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn configure(root: &Path) {
    for (key, value) in [
        ("user.name", "Test User"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("core.hooksPath", ".git/hooks"),
        ("init.defaultBranch", "main"),
        ("pull.rebase", "false"),
    ] {
        sh_git(root, &["config", key, value]);
    }
}

pub fn empty() -> Repo {
    let dir = tempfile::Builder::new().prefix("mg").tempdir().unwrap();
    sh_git(dir.path(), &["init", "-q", "-b", "main"]);
    configure(dir.path());
    Repo { dir }
}

pub fn new() -> Repo {
    let repo = empty();
    repo.commit_file("a.txt", "one\ntwo\nthree\n", "first");
    repo
}

pub fn bare() -> Repo {
    let dir = tempfile::Builder::new().prefix("mgb").tempdir().unwrap();
    sh_git(dir.path(), &["init", "-q", "--bare", "-b", "main"]);
    Repo { dir }
}

pub fn clone_of(remote: &Repo) -> Repo {
    let dir = tempfile::Builder::new().prefix("mgc").tempdir().unwrap();
    let url = remote.root().to_string_lossy().into_owned();
    sh_git(dir.path(), &["clone", "-q", &url, "."]);
    configure(dir.path());
    Repo { dir }
}

pub fn with_remote() -> (Repo, Repo) {
    let remote = bare();
    let repo = new();
    let url = remote.root().to_string_lossy().into_owned();
    repo.git(&["remote", "add", "origin", &url]);
    repo.git(&["push", "-q", "-u", "origin", "main"]);
    (repo, remote)
}
