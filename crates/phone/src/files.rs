use crate::secret::{is_hex, is_key, random_key};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const PHONES_FILE: &str = "phones.json";
pub const KEY_FILE: &str = "door-key";
pub const CERT_DIR: &str = "phone";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneRecord {
    pub id: String,
    pub name: String,
    pub key_hash: String,
    pub paired_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
}

pub fn phones_path(dir: &Path) -> PathBuf {
    dir.join(PHONES_FILE)
}

pub fn key_path(dir: &Path) -> PathBuf {
    dir.join(KEY_FILE)
}

pub fn cert_dir(dir: &Path) -> PathBuf {
    dir.join(CERT_DIR)
}

fn make_dirs(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

pub fn private_dir(dir: &Path) -> Result<()> {
    make_dirs(dir).with_context(|| format!("could not make {}", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).with_context(|| format!("could not lock {}", dir.display()))
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        make_dirs(parent).with_context(|| format!("could not make {}", parent.display()))?;
    }
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".tmp");
    let temp = path.with_file_name(name);
    let _ = fs::remove_file(&temp);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .with_context(|| format!("could not write {}", temp.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path).with_context(|| format!("could not save {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn good(record: &PhoneRecord) -> bool {
    is_hex(&record.id, 8) && is_hex(&record.key_hash, 64)
}

pub fn read_phones(path: &Path) -> Vec<PhoneRecord> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
        return Vec::new();
    };
    list.into_iter()
        .filter_map(|item| serde_json::from_value::<PhoneRecord>(item).ok())
        .filter(good)
        .collect()
}

pub fn write_phones(path: &Path, phones: &[PhoneRecord]) -> Result<()> {
    let mut text = serde_json::to_string_pretty(phones)?;
    text.push('\n');
    write_private(path, text.as_bytes())
}

pub fn read_door_key(path: &Path) -> Result<String> {
    if let Ok(saved) = fs::read_to_string(path) {
        let saved = saved.trim();
        if is_key(saved) {
            return Ok(saved.to_string());
        }
    }
    write_door_key(path)
}

pub fn write_door_key(path: &Path) -> Result<String> {
    let key = random_key()?;
    write_private(path, format!("{key}\n").as_bytes())?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> PhoneRecord {
        PhoneRecord { id: id.into(), name: "iPhone".into(), key_hash: "a".repeat(64), paired_at: 5, last_seen: None }
    }

    #[test]
    fn phones_round_trip_and_bad_rows_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = phones_path(dir.path());
        assert!(read_phones(&path).is_empty());
        write_phones(&path, &[record("0123abcd")]).unwrap();
        assert_eq!(read_phones(&path), vec![record("0123abcd")]);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        fs::write(&path, r#"[{"id":"bad","name":"x","key_hash":"aa","paired_at":1},{"id":"0123abcd","name":"x","key_hash":"ZZ","paired_at":1},7]"#).unwrap();
        assert!(read_phones(&path).is_empty());
        fs::write(&path, "not json").unwrap();
        assert!(read_phones(&path).is_empty());
    }

    #[test]
    fn door_key_is_kept_and_replaced_when_bad() {
        let dir = tempfile::tempdir().unwrap();
        let path = key_path(dir.path());
        let first = read_door_key(&path).unwrap();
        assert!(is_key(&first));
        assert_eq!(read_door_key(&path).unwrap(), first);
        fs::write(&path, "broken").unwrap();
        let second = read_door_key(&path).unwrap();
        assert_ne!(second, first);
        assert!(is_key(&second));
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
