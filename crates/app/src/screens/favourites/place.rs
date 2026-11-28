use anyhow::{Result, bail};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub const OUTSIDE: &str = "That path must stay inside the project";

pub fn project_root(folder: &Path) -> PathBuf {
    let folder = &fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    let out = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() {
                folder.to_path_buf()
            } else {
                PathBuf::from(text)
            }
        }
        _ => folder.to_path_buf(),
    }
}

pub fn is_web(url: &str) -> bool {
    let low = url.trim().to_ascii_lowercase();
    low.starts_with("http://") || low.starts_with("https://")
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(high * 16 + low);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn file_part(url: &str, root: &Path) -> PathBuf {
    let url = url.trim();
    let raw = if let Some(rest) = url.strip_prefix("file://") {
        percent_decode(rest.strip_prefix("localhost").unwrap_or(rest))
    } else {
        url.to_string()
    };
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn real(path: &Path) -> PathBuf {
    let clean = normalize(path);
    let mut base = clean.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(found) = fs::canonicalize(&base) {
            let mut out = found;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (base.file_name().map(|n| n.to_os_string()), base.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                base = parent.to_path_buf();
            }
            _ => return clean,
        }
    }
}

pub fn inside(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = real(root);
    let path = real(path);
    if path.starts_with(&root) {
        Ok(path)
    } else {
        bail!(OUTSIDE)
    }
}

pub fn resolve(url: &str, root: &Path) -> Result<PathBuf> {
    inside(root, &file_part(url, root))
}

pub fn stored_value(text: &str, root: &Path) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        bail!("Type a link or a file path");
    }
    if is_web(text) {
        return Ok(text.to_string());
    }
    let path = resolve(text, root)?;
    let rel = path
        .strip_prefix(real(root))
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let rel = rel.to_string_lossy().to_string();
    Ok(if rel.is_empty() { ".".into() } else { rel })
}

pub const SNIFF: u64 = 8192;

pub fn is_text(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let mut bytes = Vec::new();
    let read = fs::File::open(path).and_then(|f| f.take(SNIFF).read_to_end(&mut bytes));
    if read.is_err() || bytes.contains(&0) {
        return false;
    }
    match std::str::from_utf8(&bytes) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && bytes.len() as u64 == SNIFF,
    }
}

pub fn open_outside(target: &str) -> Result<()> {
    let status = Command::new("/usr/bin/open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        bail!("Cannot open {target}");
    }
    Ok(())
}
