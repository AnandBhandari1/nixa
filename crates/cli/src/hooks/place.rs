use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn stamp_at(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}-{:02}{:02}{:02}Z", rem / 3_600, rem % 3_600 / 60, rem % 60)
}

pub fn stamp() -> String {
    stamp_at(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

pub fn backup_path(path: &Path) -> PathBuf {
    let base = format!("{}.nixa-backup-{}", file_name(path), stamp());
    let mut candidate = path.with_file_name(&base);
    let mut n = 1;
    while fs::symlink_metadata(&candidate).is_ok() {
        candidate = path.with_file_name(format!("{base}-{n}"));
        n += 1;
    }
    candidate
}

pub fn real_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn inside_dir(path: &Path) -> Result<PathBuf> {
    let dir = path.parent().map(real_path).context("the settings file has no folder")?;
    let real = if fs::symlink_metadata(path).is_ok() { real_path(path) } else { dir.join(file_name(path)) };
    if !real.starts_with(&dir) {
        anyhow::bail!("{} leads to {}, outside {}, so Nixa does not change it", path.display(), real.display(), dir.display());
    }
    Ok(real)
}

fn copy_private(from: &Path, to: &Path) -> std::io::Result<()> {
    let data = fs::read(from)?;
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(to)?;
    f.write_all(&data)?;
    f.sync_all()
}

pub fn write(path: &Path, text: &str) -> Result<Option<PathBuf>> {
    let real = inside_dir(path)?;
    let (backup, mode) = match fs::metadata(&real) {
        Ok(meta) => {
            let backup = backup_path(&real);
            copy_private(&real, &backup).with_context(|| format!("cannot back up {} to {}", real.display(), backup.display()))?;
            (Some(backup), meta.permissions().mode() & 0o7777)
        }
        Err(_) => (None, 0o600),
    };
    let tmp = real.with_file_name(format!(".{}.nixa-tmp-{}", file_name(&real), std::process::id()));
    let _ = fs::remove_file(&tmp);
    let written = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        fs::rename(&tmp, &real)
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("cannot write {}", real.display()));
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_are_utc_dates() {
        assert_eq!(stamp_at(0), "19700101-000000Z");
        assert_eq!(stamp_at(1_795_316_645), "20261122-030405Z");
        assert_eq!(stamp_at(951_782_400), "20000229-000000Z");
    }

    #[test]
    fn write_backs_up_keeps_mode_and_follows_links() {
        let dir = std::env::temp_dir().join(format!("nixa-place-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real.json");
        fs::write(&real, "old").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.join("settings.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let backup = write(&link, "new").unwrap().unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "old");
        assert!(file_name(&backup).starts_with("real.json.nixa-backup-"));
        assert_eq!(fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o640);
        let second = backup_path(&real);
        assert_ne!(second, backup);
        let fresh = dir.join("fresh.json");
        assert!(write(&fresh, "x").unwrap().is_none());
        assert_eq!(fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o600);
        let names: Vec<String> = fs::read_dir(&dir).unwrap().map(|e| file_name(&e.unwrap().path())).collect();
        assert!(!names.iter().any(|n| n.contains("nixa-tmp")));
        assert_eq!(fs::metadata(&backup).unwrap().permissions().mode() & 0o777, 0o600);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_refuses_a_link_out_of_the_folder() {
        let base = std::env::temp_dir().join(format!("nixa-place-out-{}", std::process::id()));
        let dir = base.join("config");
        let away = base.join("away");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&away).unwrap();
        fs::write(away.join("real.json"), "old").unwrap();
        let link = dir.join("settings.json");
        std::os::unix::fs::symlink(away.join("real.json"), &link).unwrap();
        assert!(write(&link, "new").is_err());
        assert_eq!(fs::read_to_string(away.join("real.json")).unwrap(), "old");
        fs::remove_dir_all(&base).ok();
    }
}
