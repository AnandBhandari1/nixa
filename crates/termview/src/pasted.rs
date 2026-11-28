use gpui::ImageFormat;
use nixa_protocol::paths;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const FOLDER: &str = "pasted-images";
const PREFIX: &str = "paste-";
const KEEP: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const ESCAPE: &str = "'\"\\()&;$`!*?[]{}<>|#~";

pub fn folder() -> PathBuf {
    paths::data_dir().join(FOLDER)
}

pub fn ext(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
    }
}

pub fn save(dir: &Path, format: ImageFormat, bytes: &[u8], now: SystemTime) -> io::Result<PathBuf> {
    paths::ensure_private_dir(dir)?;
    prune(dir, now);
    let stamp = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let ext = ext(format);
    let mut path = dir.join(format!("{PREFIX}{stamp}.{ext}"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{PREFIX}{stamp}-{n}.{ext}"));
        n += 1;
    }
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
    file.write_all(bytes)?;
    Ok(path)
}

pub fn prune(dir: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let ours = entry.file_name().to_string_lossy().starts_with(PREFIX);
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > KEEP);
        if ours && old {
            fs::remove_file(entry.path()).ok();
        }
    }
}

pub fn shell_quote(path: &Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        if c.is_whitespace() || ESCAPE.contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nixa-paste-{tag}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn saves_with_the_right_name_and_never_overwrites() {
        let dir = temp("save");
        let now = SystemTime::now();
        let first = save(&dir, ImageFormat::Png, b"one", now).unwrap();
        let second = save(&dir, ImageFormat::Png, b"two", now).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.extension().unwrap(), "png");
        assert!(first.file_name().unwrap().to_string_lossy().starts_with(PREFIX));
        assert_eq!(fs::read(&first).unwrap(), b"one");
        assert_eq!(fs::read(&second).unwrap(), b"two");
        let jpg = save(&dir, ImageFormat::Jpeg, b"j", now).unwrap();
        assert_eq!(jpg.extension().unwrap(), "jpg");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn old_pastes_are_removed() {
        let dir = temp("prune");
        let now = SystemTime::now();
        let old = save(&dir, ImageFormat::Png, b"old", now).unwrap();
        let other = dir.join("keep.txt");
        fs::write(&other, "x").unwrap();
        let later = now + KEEP + Duration::from_secs(60);
        let fresh = save(&dir, ImageFormat::Png, b"new", later).unwrap();
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(other.exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quotes_spaces_and_shell_marks() {
        assert_eq!(
            shell_quote(Path::new("/a b/c(1).png")),
            "/a\\ b/c\\(1\\).png"
        );
        assert_eq!(shell_quote(Path::new("/plain/x.png")), "/plain/x.png");
    }
}
