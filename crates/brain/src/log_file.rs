use std::fs::{self, File};
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const NAME: &str = "brain.log";
pub const MAX_BYTES: u64 = 2_000_000;
pub const KEEP: usize = 2;

struct Watched {
    path: PathBuf,
    file: File,
}

static WATCHED: Mutex<Option<Watched>> = Mutex::new(None);

pub fn watch(dir: &Path) {
    let path = dir.join(NAME);
    let Some(file) = stderr_file() else { return };
    if !same_file(&file, &path) {
        return;
    }
    *WATCHED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Watched { path, file });
    check();
}

pub fn check() {
    let watched = WATCHED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(w) = watched.as_ref()
        && too_big(&w.file)
    {
        let _ = rotate(&w.path, &w.file);
    }
}

pub fn rotate(path: &Path, file: &File) -> io::Result<()> {
    for n in (1..KEEP).rev() {
        let from = old(path, n);
        if from.exists() {
            fs::rename(&from, old(path, n + 1))?;
        }
    }
    fs::copy(path, old(path, 1))?;
    file.set_len(0)
}

pub fn old(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

fn too_big(file: &File) -> bool {
    file.metadata().is_ok_and(|m| m.len() > MAX_BYTES)
}

fn stderr_file() -> Option<File> {
    io::stderr().as_fd().try_clone_to_owned().ok().map(File::from)
}

fn same_file(file: &File, path: &Path) -> bool {
    match (file.metadata(), fs::metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn rotate_keeps_two_old_files_and_empties_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NAME);
        let mut file = fs::OpenOptions::new().create(true).append(true).open(&path).unwrap();
        for round in ["one", "two", "three"] {
            file.write_all(round.as_bytes()).unwrap();
            rotate(&path, &file).unwrap();
        }
        file.write_all(b"four").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "four");
        assert_eq!(fs::read_to_string(old(&path, 1)).unwrap(), "three");
        assert_eq!(fs::read_to_string(old(&path, 2)).unwrap(), "two");
        assert!(!old(&path, 3).exists());
    }

    #[test]
    fn only_a_big_file_is_too_big() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NAME);
        let file = File::create(&path).unwrap();
        assert!(!too_big(&file));
        file.set_len(MAX_BYTES + 1).unwrap();
        assert!(too_big(&file));
        assert!(same_file(&file, &path));
        assert!(!same_file(&file, &dir.path().join("other.log")));
    }

    #[test]
    fn old_files_get_a_number() {
        assert_eq!(old(Path::new("/data/brain.log"), 2), PathBuf::from("/data/brain.log.2"));
    }
}
