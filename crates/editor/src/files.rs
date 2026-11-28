use crate::doc::{self, Doc};
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

pub const EDIT_LIMIT: u64 = 1024 * 1024;
pub const VIEW_LIMIT: u64 = 4 * 1024 * 1024;
pub const IMAGE_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewOnly {
    pub size: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    Text { doc: Doc, view_only: Option<ViewOnly> },
    Missing,
    Folder,
    Binary,
    Large(String),
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Save {
    Saved(Doc),
    Conflict(Doc),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
    pub exists: bool,
}

pub fn stamp(path: &str) -> Stamp {
    match fs::metadata(path) {
        Ok(m) => Stamp { modified: m.modified().ok(), len: m.len(), exists: true },
        Err(_) => Stamp { modified: None, len: 0, exists: false },
    }
}

pub fn expand(path: &str, home: Option<&str>) -> String {
    let p = path.trim();
    match (p, home) {
        ("~", Some(h)) => h.to_string(),
        (p, Some(h)) if p.starts_with("~/") => format!("{}/{}", h.trim_end_matches('/'), &p[2..]),
        (p, _) => p.to_string(),
    }
}

pub fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            p => out.push(p),
        }
    }
    format!("/{}", out.join("/"))
}

pub fn pretty_folder(path: &str, home: Option<&str>) -> String {
    match home {
        Some(h) if !h.is_empty() && path == h => "~".into(),
        Some(h) if !h.is_empty() && path.starts_with(&format!("{h}/")) => format!("~{}", &path[h.len()..]),
        _ => path.to_string(),
    }
}

fn text_of(bytes: Vec<u8>, cut: bool) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    match String::from_utf8(bytes) {
        Ok(s) => Some(s),
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            let tail_ok = cut && e.utf8_error().error_len().is_none();
            if !tail_ok {
                return None;
            }
            let mut bytes = e.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).ok()
        }
    }
}

pub fn read_raw(path: &str) -> io::Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn read(path: &str) -> Read {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Read::Missing,
        Err(e) => return Read::Error(e.to_string()),
    };
    if meta.is_dir() {
        return Read::Folder;
    }
    let size = meta.len();
    let limit = if size > EDIT_LIMIT { VIEW_LIMIT } else { EDIT_LIMIT * 2 };
    let mut bytes = Vec::with_capacity(size.min(limit) as usize);
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return Read::Error(e.to_string()),
    };
    if let Err(e) = file.take(limit).read_to_end(&mut bytes) {
        return Read::Error(e.to_string());
    }
    let truncated = (bytes.len() as u64) < size;
    let Some(text) = text_of(bytes, truncated) else {
        return Read::Binary;
    };
    let view_only = (size > EDIT_LIMIT).then_some(ViewOnly { size, truncated });
    Read::Text { doc: doc::open(&text), view_only }
}

pub fn view_only_notice(v: &ViewOnly) -> String {
    let mb = v.size as f64 / 1048576.0;
    if v.truncated {
        format!("Read only. Showing the first 4 MiB of {mb:.1} MiB. Use Open for the whole file.")
    } else {
        format!("Read only. This file is {mb:.1} MiB, too big to save from here.")
    }
}

pub fn link_outside(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    let real = fs::canonicalize(p).ok()?;
    let folder = fs::canonicalize(p.parent()?).ok()?;
    (!real.starts_with(&folder)).then_some(real)
}

fn real_target(path: &str) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

static SAVES: AtomicUsize = AtomicUsize::new(0);

fn temp_beside(target: &Path) -> PathBuf {
    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let n = SAVES.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".{name}.nixa-save-{}-{n}", std::process::id()))
}

fn write_atomic(target: &Path, text: &str) -> io::Result<()> {
    let tmp = temp_beside(target);
    let result = (|| {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        if let Ok(meta) = fs::metadata(target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, target)
    })();
    if result.is_err() {
        fs::remove_file(&tmp).ok();
    }
    result
}

pub fn write_checked(path: &str, text: &str, expected: Option<&str>) -> io::Result<()> {
    let target = real_target(path);
    let now = read_raw(&target.to_string_lossy())?;
    if now.as_deref() != expected {
        return Err(io::Error::other("changed on disk"));
    }
    write_atomic(&target, text)
}

pub fn save(path: &str, d: &Doc, value: &str) -> Save {
    let text = doc::disk_text(d, value);
    let expected = d.disk.as_deref();
    match write_checked(path, &text, expected) {
        Ok(()) => Save::Saved(doc::saved(d, text, value)),
        Err(e) => {
            let now = read_raw(path);
            match now {
                Ok(Some(raw)) if raw == text => Save::Saved(doc::saved(d, text, value)),
                Ok(raw) if raw.as_deref() != expected => Save::Conflict(Doc {
                    conflict: Some(doc::Conflict { disk: raw, from_save: true }),
                    ..d.clone()
                }),
                _ => Save::Failed(e.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nixa-ed-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_every_kind() {
        let dir = temp("kinds");
        let p = |n: &str| dir.join(n).to_string_lossy().into_owned();
        fs::write(p("a.txt"), "hi\n").unwrap();
        fs::write(p("b.bin"), [1u8, 0, 2]).unwrap();
        fs::write(p("c.bin"), [0xffu8, 0xfe, 0x41]).unwrap();
        fs::create_dir_all(p("d")).unwrap();
        assert!(matches!(read(&p("a.txt")), Read::Text { view_only: None, .. }));
        assert_eq!(read(&p("b.bin")), Read::Binary);
        assert_eq!(read(&p("c.bin")), Read::Binary);
        assert_eq!(read(&p("d")), Read::Folder);
        assert_eq!(read(&p("none")), Read::Missing);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn big_files_are_view_only_and_cut() {
        let dir = temp("big");
        let path = dir.join("big.log").to_string_lossy().into_owned();
        let line = "é".repeat(50) + "\n";
        let text = line.repeat((5 * 1024 * 1024) / line.len() + 1);
        fs::write(&path, &text).unwrap();
        match read(&path) {
            Read::Text { doc, view_only: Some(v) } => {
                assert!(v.truncated);
                assert!(doc.base.len() as u64 <= VIEW_LIMIT);
                assert!(view_only_notice(&v).contains("first 4 MiB"));
            }
            other => panic!("{other:?}"),
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_keeps_mode_and_line_endings() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("save");
        let path = dir.join("x.sh").to_string_lossy().into_owned();
        fs::write(&path, "a\r\nb\r\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        let Read::Text { doc, .. } = read(&path) else { panic!() };
        let Save::Saved(next) = save(&path, &doc, "a\nb\nc\n") else { panic!() };
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\r\nb\r\nc\r\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o750);
        assert!(!doc::is_dirty(&next, "a\nb\nc\n"));
        let left: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(left.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_through_a_link_writes_the_real_file() {
        let dir = temp("link");
        let real = dir.join("real.txt");
        let link = dir.join("link.txt");
        fs::write(&real, "one\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let path = link.to_string_lossy().into_owned();
        let Read::Text { doc, .. } = read(&path) else { panic!() };
        assert!(matches!(save(&path, &doc, "two\n"), Save::Saved(_)));
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "two\n");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn link_outside_its_folder_is_found() {
        let dir = temp("outside");
        fs::create_dir_all(dir.join("proj/sub")).unwrap();
        fs::create_dir_all(dir.join("away")).unwrap();
        fs::write(dir.join("away/secret.txt"), "s\n").unwrap();
        fs::write(dir.join("proj/sub/near.txt"), "n\n").unwrap();
        std::os::unix::fs::symlink(dir.join("away/secret.txt"), dir.join("proj/out.txt")).unwrap();
        std::os::unix::fs::symlink(dir.join("proj/sub/near.txt"), dir.join("proj/in.txt")).unwrap();
        let p = |n: &str| dir.join(n).to_string_lossy().into_owned();
        assert_eq!(link_outside(&p("proj/out.txt")), Some(fs::canonicalize(dir.join("away/secret.txt")).unwrap()));
        assert_eq!(link_outside(&p("proj/in.txt")), None);
        assert_eq!(link_outside(&p("proj/sub/near.txt")), None);
        assert_eq!(link_outside(&p("proj/none.txt")), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_finds_a_change_on_disk() {
        let dir = temp("race");
        let path = dir.join("x.txt").to_string_lossy().into_owned();
        fs::write(&path, "one\n").unwrap();
        let Read::Text { doc, .. } = read(&path) else { panic!() };
        fs::write(&path, "other\n").unwrap();
        match save(&path, &doc, "mine\n") {
            Save::Conflict(d) => {
                let c = d.conflict.unwrap();
                assert!(c.from_save);
                assert_eq!(c.disk.as_deref(), Some("other\n"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "other\n");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_makes_a_deleted_file_again() {
        let dir = temp("gone");
        let path = dir.join("x.txt").to_string_lossy().into_owned();
        fs::write(&path, "one\n").unwrap();
        let Read::Text { doc, .. } = read(&path) else { panic!() };
        fs::remove_file(&path).unwrap();
        let (d, _) = doc::on_disk(&doc, None, "one\n");
        let mine = doc::keep_mine(&d);
        assert!(matches!(save(&path, &mine, "one\n"), Save::Saved(_)));
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\n");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn paths() {
        assert_eq!(expand("~/x", Some("/home/u")), "/home/u/x");
        assert_eq!(expand("~", Some("/home/u")), "/home/u");
        assert_eq!(normalize("/a/b/../c/./d"), "/a/c/d");
        assert_eq!(pretty_folder("/home/u/p", Some("/home/u")), "~/p");
        assert_eq!(pretty_folder("/tmp", Some("/home/u")), "/tmp");
    }
}
