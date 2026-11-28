use std::path::{Path, PathBuf};

pub const AUTO_CHECKS_FILE: &str = "auto-checks";

pub fn file_of(dir: &Path) -> PathBuf {
    dir.join(AUTO_CHECKS_FILE)
}

pub fn on(dir: &Path) -> bool {
    file_of(dir).exists()
}

pub fn set(dir: &Path, on: bool) -> std::io::Result<()> {
    let file = file_of(dir);
    if on {
        crate::util::atomic_write(&file, "on\n")
    } else {
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}
