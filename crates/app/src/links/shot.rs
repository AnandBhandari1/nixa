use nixa_protocol::control::ControlReply;
use nixa_protocol::paths;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const FOLDER: &str = "shots";
pub const KEEP: usize = 10;
const PARTS: [usize; 5] = [8, 4, 4, 4, 12];

pub fn capture_name(capture: &str) -> bool {
    let parts: Vec<&str> = capture.split('-').collect();
    parts.len() == PARTS.len()
        && parts
            .iter()
            .zip(PARTS)
            .all(|(part, size)| part.len() == size && part.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)))
}

pub fn folder() -> PathBuf {
    paths::data_dir().join(FOLDER)
}

pub fn prune(dir: &Path, keep: usize) -> usize {
    let Ok(read) = std::fs::read_dir(dir) else { return 0 };
    let mut files: Vec<(SystemTime, PathBuf)> = read
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("png"))
        .filter_map(|path| std::fs::metadata(&path).and_then(|m| m.modified()).ok().map(|at| (at, path)))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().skip(keep).filter(|(_, path)| std::fs::remove_file(path).is_ok()).count()
}

pub fn reply(name: &str, capture: &str) -> ControlReply {
    take(name, capture).unwrap_or_else(|message| ControlReply::Error { message })
}

fn take(name: &str, capture: &str) -> Result<ControlReply, String> {
    if !capture_name(capture) {
        return Err("The capture id is not valid. Nothing was captured.".into());
    }
    let dir = folder();
    paths::ensure_private_dir(&dir).map_err(|e| format!("The shot folder cannot be made: {e}"))?;
    prune(&dir, KEEP);
    let image = dir.join(format!("{capture}.png"));
    let made = nixa_desktop::own_shot(name, &image)?;
    Ok(ControlReply::Shot {
        image: image.to_string_lossy().into_owned(),
        window: made.get("window").and_then(Value::as_str).unwrap_or_default().to_string(),
        id: made.get("id").and_then(Value::as_i64).unwrap_or_default(),
        bytes: made.get("bytes").and_then(Value::as_u64).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_capture_ids_name_a_shot() {
        assert!(capture_name("0123abcd-4567-89ef-0123-456789abcdef"));
        assert!(!capture_name("0123ABCD-4567-89ef-0123-456789abcdef"));
        assert!(!capture_name("../../etc/passwd"));
        assert!(!capture_name("0123abcd-4567-89ef-0123-456789abcde"));
        assert!(!capture_name(""));
    }

    #[test]
    fn prune_keeps_the_newest_pictures() {
        let dir = std::env::temp_dir().join(format!("nixa-shot-prune-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        for n in 0..4 {
            std::fs::write(dir.join(format!("{n}.png")), b"x").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::fs::write(dir.join("keep.txt"), b"x").unwrap();
        assert_eq!(prune(&dir, 2), 2);
        assert!(dir.join("3.png").exists());
        assert!(dir.join("2.png").exists());
        assert!(!dir.join("0.png").exists());
        assert!(dir.join("keep.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
