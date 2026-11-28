use crate::app::App;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

pub const KEEP: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const EVERY: Duration = Duration::from_secs(60 * 60);
pub const ENGINE_LOG: &str = "engine.log";

pub async fn run(app: Arc<App>) {
    let dir = app.dir.clone();
    loop {
        let root = dir.clone();
        let removed = tokio::task::spawn_blocking(move || sweep(&root, SystemTime::now())).await.unwrap_or(0);
        if removed > 0 {
            crate::util::log("sweep", &format!("removed {removed} old files from the data folder"));
        }
        tokio::time::sleep(EVERY).await;
    }
}

pub fn sweep(dir: &Path, now: SystemTime) -> usize {
    let desktop = dir.join(crate::desktop::FOLDER);
    let mut removed = old_files(&desktop, now, |name| name.ends_with(".png"));
    removed += old_files(&desktop, now, |name| name == crate::desktop::gate::STEPS_FILE);
    let frames = dir.join(crate::connectors::FOLDER).join(crate::connectors::command::PRIVATE_FOLDER).join(crate::connectors::builtin::CAMERA);
    removed += old_files(&frames, now, |_| true);
    removed += old_files(&dir.join(crate::term_tools::close::ARCHIVE), now, |name| name.ends_with(".json"));
    removed += old_files(&dir.join(PASTED_FOLDER), now, |name| name.starts_with("paste-"));
    rotate_engine_log(dir);
    removed
}

const PASTED_FOLDER: &str = "pasted-images";

fn old_files(dir: &Path, now: SystemTime, pick: impl Fn(&str) -> bool) -> usize {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| pick(&e.file_name().to_string_lossy()))
        .filter(|e| e.metadata().and_then(|m| m.modified()).ok().and_then(|at| now.duration_since(at).ok()).is_some_and(|age| age > KEEP))
        .filter(|e| fs::remove_file(e.path()).is_ok())
        .count()
}

pub fn rotate_engine_log(dir: &Path) {
    let path = dir.join(ENGINE_LOG);
    let Ok(file) = fs::OpenOptions::new().write(true).open(&path) else { return };
    if file.metadata().is_ok_and(|m| m.len() > crate::log_file::MAX_BYTES) {
        let _ = crate::log_file::rotate(&path, &file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aged(path: &Path, age: Duration) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let file = fs::File::create(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn old_files_go_and_new_files_stay() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let old = KEEP + Duration::from_secs(3600);
        let frames = d.join(crate::connectors::FOLDER).join(crate::connectors::command::PRIVATE_FOLDER).join(crate::connectors::builtin::CAMERA);
        let gone = [
            d.join("desktop/a.png"),
            d.join("desktop/steps.jsonl"),
            frames.join("f.jpg"),
            d.join("closed-terminal-reports/t1.json"),
            d.join("pasted-images/paste-1.png"),
        ];
        for path in &gone {
            aged(path, old);
        }
        let stay = [d.join("desktop/new.png"), d.join("desktop/keep.txt"), d.join("pasted-images/other.png")];
        aged(&stay[0], Duration::from_secs(60));
        aged(&stay[1], old);
        aged(&stay[2], old);
        assert_eq!(sweep(d, SystemTime::now()), gone.len());
        for path in &gone {
            assert!(!path.exists(), "{}", path.display());
        }
        for path in &stay {
            assert!(path.exists(), "{}", path.display());
        }
    }

    #[test]
    fn a_big_engine_log_is_rotated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ENGINE_LOG);
        let file = fs::File::create(&path).unwrap();
        file.set_len(crate::log_file::MAX_BYTES + 1).unwrap();
        rotate_engine_log(dir.path());
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        assert!(crate::log_file::old(&path, 1).exists());
    }
}
