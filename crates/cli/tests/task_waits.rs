use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nixa-waits-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn nixa(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nixa"))
        .args(args)
        .env("NIXA_DATA_DIR", dir)
        .env("NIXA_ENGINE_SOCKET", dir.join("no-engine.sock"))
        .env("NIXA_BRAIN_ADDR", "127.0.0.1:9")
        .env_remove("NIXA_TERM_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn a_task_names_the_tasks_it_waits_on() {
    let dir = temp_dir();
    for title in ["Build the store", "Write the docs", "Use the store"] {
        assert!(nixa(&dir, &["task", "add", title]).status.success());
    }
    let set = nixa(&dir, &["task", "wait", "3", "#1", "2"]);
    assert!(set.status.success(), "{}", text(&set));
    assert!(text(&set).contains("Task #3 now waits on task #1, task #2."));
    let shown = nixa(&dir, &["task", "wait", "3"]);
    assert!(text(&shown).contains("Still open: task #1, task #2."), "{}", text(&shown));
    let looped = nixa(&dir, &["task", "wait", "1", "3"]);
    assert!(!looped.status.success());
    assert!(text(&looped).contains("would be a loop"));
    assert!(nixa(&dir, &["task", "status", "3", "blocked", "--on", "task"]).status.success());
    let cleared = nixa(&dir, &["task", "wait", "3", "--clear"]);
    assert!(text(&cleared).contains("so it is back in todo"), "{}", text(&cleared));
    assert!(text(&nixa(&dir, &["task", "show", "3"])).contains("[todo]"));
    assert!(text(&nixa(&dir, &["task", "wait", "3"])).contains("Task #3 waits on no task."));
    std::fs::remove_dir_all(&dir).ok();
}
