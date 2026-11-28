use anyhow::{Context, Result};
use nixa_protocol::paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const FILE: &str = "favourites.json";

static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Favourite {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    projects: BTreeMap<String, Vec<Favourite>>,
}

pub fn default_file() -> PathBuf {
    paths::data_dir().join(FILE)
}

fn key(project: &Path) -> String {
    project.display().to_string()
}

fn read(file: &Path) -> Result<Book> {
    match fs::read_to_string(file) {
        Ok(text) if text.trim().is_empty() => Ok(Book::default()),
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("Cannot read favourites from {}", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Book::default()),
        Err(e) => Err(e).with_context(|| format!("Cannot open {}", file.display())),
    }
}

fn write(file: &Path, book: &Book) -> Result<()> {
    if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        paths::ensure_private_dir(dir)
            .with_context(|| format!("Cannot make the folder {}", dir.display()))?;
    }
    let tmp = file.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let text = serde_json::to_string_pretty(book)?;
    let result = put(&tmp, file, &text);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn put(tmp: &Path, file: &Path, text: &str) -> Result<()> {
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(tmp)
        .with_context(|| format!("Cannot write {}", tmp.display()))?;
    out.write_all(text.as_bytes())?;
    out.sync_all()?;
    fs::rename(tmp, file).with_context(|| format!("Cannot save {}", file.display()))?;
    Ok(())
}

pub fn load(file: &Path, project: &Path) -> Result<Vec<Favourite>> {
    Ok(read(file)?
        .projects
        .remove(&key(project))
        .unwrap_or_default())
}

pub fn save(file: &Path, project: &Path, title: &str, url: &str) -> Result<()> {
    let mut book = read(file)?;
    let list = book.projects.entry(key(project)).or_default();
    let title = if title.trim().is_empty() {
        url
    } else {
        title.trim()
    };
    match list.iter_mut().find(|f| f.url == url) {
        Some(found) => found.title = title.to_string(),
        None => list.push(Favourite {
            title: title.to_string(),
            url: url.to_string(),
        }),
    }
    write(file, &book)
}

pub fn remove(file: &Path, project: &Path, url: &str) -> Result<bool> {
    let mut book = read(file)?;
    let k = key(project);
    let Some(list) = book.projects.get_mut(&k) else {
        return Ok(false);
    };
    let before = list.len();
    list.retain(|f| f.url != url);
    let gone = list.len() != before;
    if list.is_empty() {
        book.projects.remove(&k);
    }
    if gone {
        write(file, &book)?;
    }
    Ok(gone)
}
