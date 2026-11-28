use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

pub fn clip_flat(text: &str, max: usize) -> String {
    let flat = flat(text);
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

pub fn clip_note(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}\n[cut, {} more characters]", count - max)
}

pub fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn short_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub fn atomic_write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        nixa_protocol::paths::make_private_dirs(dir)?;
    }
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_extension(format!("tmp{}.{seq}", std::process::id()));
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&temp, path)
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    atomic_write(path, &format!("{text}\n"))
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn append_line(path: &Path, line: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        nixa_protocol::paths::make_private_dirs(dir)?;
    }
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    writeln!(file, "{line}")
}

pub fn day_of(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn log(area: &str, message: &str) {
    eprintln!("[{}] {area}: {}", now_ms(), crate::privacy::redact::redact(message));
    crate::log_file::check();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_by_chars() {
        assert_eq!(clip("abcdef", 3), "abc…");
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip_flat("a  b\n c", 10), "a b c");
        assert_eq!(clip_flat("abcdef", 4), "abc…");
    }

    #[test]
    fn days() {
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(day_of(1_795_305_600_000), "2026-11-22");
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(short_hash("x").len(), 16);
        assert_eq!(short_hash("x"), short_hash("x"));
    }

    #[test]
    fn writes_private_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b.json");
        write_json(&path, &serde_json::json!({"a": 1})).unwrap();
        let back: serde_json::Value = read_json(&path).unwrap();
        assert_eq!(back["a"], 1);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn concurrent_atomic_writes_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voice.json");
        std::thread::scope(|scope| {
            for i in 0..8 {
                let target = path.clone();
                scope.spawn(move || {
                    for j in 0..20 {
                        atomic_write(&target, &format!("{{\"t\":{i},\"n\":{j}}}\n")).unwrap();
                    }
                });
            }
        });
        assert!(read_json::<serde_json::Value>(&path).is_some());
    }
}
