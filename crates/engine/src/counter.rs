use std::fs;
use std::io;
use std::path::Path;

pub const COUNTER_FILE: &str = "term-counter";

pub fn load(path: &Path) -> u64 {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(1)
        .max(1)
}

pub fn save(path: &Path, next: u64) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, format!("{next}\n"))?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_round_trip() {
        let dir = std::env::temp_dir().join(format!("nixa-counter-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(COUNTER_FILE);
        assert_eq!(load(&file), 1);
        save(&file, 42).unwrap();
        assert_eq!(load(&file), 42);
        fs::remove_dir_all(&dir).unwrap();
    }
}
