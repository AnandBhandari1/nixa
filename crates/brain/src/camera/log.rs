use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FILE: &str = "log.json";
pub const KEEP_MS: u64 = 7 * 24 * 60 * 60_000;
pub const MAX_LINES: usize = 500;
pub const ANSWER_CHARS: usize = 600;
pub const TEMP_HEAD: &str = "log.tmp";

static WRITING: Mutex<()> = Mutex::new(());

fn hold() -> std::sync::MutexGuard<'static, ()> {
    WRITING.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn remove_temps(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(TEMP_HEAD))
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Line {
    pub at: u64,
    pub phone: String,
    pub camera: String,
    pub question: String,
    pub answer: String,
    pub model: String,
    pub brain: String,
    pub frames: u8,
    pub cloud: bool,
}

pub fn file_of(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

pub fn fresh(lines: Vec<Line>, now: u64) -> Vec<Line> {
    let mut kept: Vec<Line> = lines.into_iter().filter(|l| now.saturating_sub(l.at) < KEEP_MS).collect();
    let extra = kept.len().saturating_sub(MAX_LINES);
    kept.drain(..extra);
    kept
}

pub fn read(dir: &Path, now: u64) -> Vec<Line> {
    let lines: Vec<Line> = std::fs::read_to_string(file_of(dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    fresh(lines, now)
}

fn save(dir: &Path, lines: &[Line]) -> Result<(), String> {
    nixa_protocol::paths::ensure_private_dir(dir).map_err(|e| format!("The camera folder was not made: {e}"))?;
    let text = serde_json::to_string(lines).map_err(|e| e.to_string())?;
    crate::util::atomic_write(&file_of(dir), &text).map_err(|e| format!("The camera log was not saved: {e}"))
}

pub fn add(dir: &Path, mut line: Line, now: u64) -> Result<(), String> {
    let _held = hold();
    line.answer = crate::util::clip(&line.answer, ANSWER_CHARS);
    let mut lines = read(dir, now);
    lines.push(line);
    save(dir, &fresh(lines, now))
}

pub fn prune(dir: &Path, now: u64) -> Result<bool, String> {
    let _held = hold();
    let temps = remove_temps(dir);
    let Some(lines) = std::fs::read_to_string(file_of(dir)).ok().and_then(|t| serde_json::from_str::<Vec<Line>>(&t).ok()) else {
        return Ok(temps > 0);
    };
    let count = lines.len();
    let kept = fresh(lines, now);
    if kept.len() == count {
        return Ok(temps > 0);
    }
    save(dir, &kept)?;
    Ok(true)
}

pub fn forget(dir: &Path, phone: &str, now: u64) -> Result<(), String> {
    let _held = hold();
    let lines = read(dir, now);
    let count = lines.len();
    let kept: Vec<Line> = lines.into_iter().filter(|l| l.phone != phone).collect();
    if kept.len() == count {
        return Ok(());
    }
    save(dir, &kept)
}

pub fn clear(dir: &Path) -> Result<(), String> {
    let _held = hold();
    remove_temps(dir);
    match std::fs::remove_file(file_of(dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("The camera log was not cleared: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forget_drops_only_that_phones_lines() {
        let dir = tempfile::tempdir().unwrap();
        let now = 10 * KEEP_MS;
        for phone in ["p1", "p2", "p1"] {
            add(dir.path(), Line { at: now, phone: phone.into(), ..Line::default() }, now).unwrap();
        }
        forget(dir.path(), "p1", now).unwrap();
        let left = read(dir.path(), now);
        assert_eq!(left.len(), 1);
        assert!(left.iter().all(|l| l.phone == "p2"));
    }
}
