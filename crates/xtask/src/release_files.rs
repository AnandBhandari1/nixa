use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub fn candidate(path: &Path) -> Result<String, String> {
    let before =
        fingerprint(path).map_err(|e| format!("cannot fingerprint {}: {e}", path.display()))?;
    crate::sign::verify(path)?;
    let after = fingerprint(path).map_err(|e| format!("cannot recheck {}: {e}", path.display()))?;
    if after != before {
        return Err(format!(
            "{} changed during signature verification",
            path.display()
        ));
    }
    Ok(after)
}

pub fn fingerprint(path: &Path) -> io::Result<String> {
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(io::Error::other("bundle root must be a real directory"));
    }
    let mut hash = Sha256::new();
    visit(path, Path::new(""), &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn visit(root: &Path, relative: &Path, hash: &mut Sha256) -> io::Result<()> {
    let path = root.join(relative);
    let meta = fs::symlink_metadata(&path)?;
    let bytes = relative.as_os_str().as_bytes();
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    hash.update((meta.permissions().mode() & 0o7777).to_le_bytes());
    if meta.is_dir() {
        hash.update(b"D");
        let mut entries = fs::read_dir(&path)?.collect::<io::Result<Vec<_>>>()?;
        entries.sort_unstable_by_key(|e| e.file_name());
        for entry in entries {
            visit(root, &relative.join(entry.file_name()), hash)?;
        }
    } else if meta.is_file() {
        hash.update(b"F");
        hash.update(meta.len().to_le_bytes());
        let mut file = File::open(&path)?;
        let mut buffer = [0; 32 * 1024];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
    } else {
        return Err(io::Error::other(format!(
            "non-regular entry: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn exchange(installed: &Path, slot: &Path) -> io::Result<()> {
    let a = CString::new(installed.as_os_str().as_bytes())?;
    let b = CString::new(slot.as_os_str().as_bytes())?;
    let result = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_and_rollback_keep_both_bundles() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("Nixa.app");
        let slot = root.path().join("backup/Nixa.app");
        fs::create_dir_all(&installed).unwrap();
        fs::create_dir_all(&slot).unwrap();
        fs::write(installed.join("version"), b"old").unwrap();
        fs::write(slot.join("version"), b"new").unwrap();
        let old = fingerprint(&installed).unwrap();
        let new = fingerprint(&slot).unwrap();
        assert_ne!(old, new);
        assert!(exchange(&installed, &slot.join("missing")).is_err());
        assert_eq!(fingerprint(&installed).unwrap(), old);
        exchange(&installed, &slot).unwrap();
        assert_eq!(fingerprint(&installed).unwrap(), new);
        assert_eq!(fingerprint(&slot).unwrap(), old);
        exchange(&installed, &slot).unwrap();
        assert_eq!(fingerprint(&installed).unwrap(), old);
        assert_eq!(fingerprint(&slot).unwrap(), new);
    }

    #[test]
    fn digest_is_location_independent_but_rejects_symlinks_and_detects_modes() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("one");
        let b = root.path().join("two");
        for path in [&a, &b] {
            fs::create_dir(path).unwrap();
            fs::write(path.join("file"), b"data").unwrap();
        }
        assert_eq!(fingerprint(&a).unwrap(), fingerprint(&b).unwrap());
        fs::set_permissions(b.join("file"), fs::Permissions::from_mode(0o700)).unwrap();
        assert_ne!(fingerprint(&a).unwrap(), fingerprint(&b).unwrap());
        std::os::unix::fs::symlink(b.join("file"), a.join("link")).unwrap();
        assert!(
            fingerprint(&a)
                .unwrap_err()
                .to_string()
                .contains("non-regular")
        );
    }
}
