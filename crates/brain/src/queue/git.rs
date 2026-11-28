use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

pub type Print = BTreeMap<String, String>;

pub const MAX_ENTRIES: usize = 3000;
pub const DIR_FILES: usize = 200;
pub const CLEAN: &str = "clean";
pub const GONE: &str = "deleted";

pub fn tool_cache(path: &str) -> bool {
    path.starts_with(".trail/cache/") || path.contains("/.trail/cache/")
}

fn hash_of(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => Sha256::digest(&bytes)
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect(),
        Err(_) if path.is_dir() => "folder".into(),
        Err(_) => GONE.into(),
    }
}

pub fn parse_status(out: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut parts = out.split(|b| *b == 0).filter(|p| !p.is_empty());
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let code = &entry[..2];
        paths.push(String::from_utf8_lossy(&entry[3..]).to_string());
        if (code[0] == b'R' || code[0] == b'C')
            && let Some(from) = parts.next()
        {
            paths.push(String::from_utf8_lossy(from).to_string());
        }
    }
    paths
}

fn status(folder: &str, untracked: &str) -> Option<Vec<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["status", "--porcelain=v1", "-z", untracked])
        .output()
        .ok()?;
    out.status.success().then(|| parse_status(&out.stdout))
}

pub fn short_list(all: &[String], normal: &[String], most: usize) -> Vec<(String, Option<usize>)> {
    let mut kept = Vec::new();
    for path in normal {
        if !path.ends_with('/') {
            kept.push((path.clone(), None));
            continue;
        }
        let from = all.partition_point(|p| p.as_str() < path.as_str());
        let inside: Vec<&String> = all[from..].iter().take_while(|p| p.starts_with(path.as_str())).collect();
        if inside.len() <= most {
            kept.extend(inside.into_iter().map(|p| (p.clone(), None)));
        } else {
            kept.push((path.clone(), Some(inside.len())));
        }
    }
    kept
}

pub fn in_print(print: &Print, path: &str) -> bool {
    print.contains_key(path)
        || print.keys().any(|k| k.ends_with('/') && path.starts_with(k.as_str()))
        || (path.ends_with('/') && print.keys().any(|k| k.starts_with(path)))
}

pub fn fingerprint_now(folder: &str) -> Option<Print> {
    let mut all = status(folder, "--untracked-files=all")?;
    let mut paths: Vec<(String, Option<usize>)> = Vec::new();
    if all.len() > MAX_ENTRIES {
        all.sort();
        let normal = status(folder, "--untracked-files=normal")?;
        paths = short_list(&all, &normal, DIR_FILES);
        if paths.len() > MAX_ENTRIES {
            paths = short_list(&all, &normal, 0);
        }
    } else {
        paths.extend(all.into_iter().map(|p| (p, None)));
    }
    if paths.len() > MAX_ENTRIES {
        return None;
    }
    let top = Command::new("git").arg("-C").arg(folder).args(["rev-parse", "--show-toplevel"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|t| !t.is_empty());
    let root = Path::new(top.as_deref().unwrap_or(folder));
    Some(
        paths
            .into_iter()
            .filter(|(p, _)| !tool_cache(p))
            .map(|(p, count)| {
                let hash = match count {
                    Some(n) => format!("folder of {n} files"),
                    None => hash_of(&root.join(&p)),
                };
                (p, hash)
            })
            .collect(),
    )
}

pub async fn fingerprint(folder: &str) -> Option<Print> {
    let folder = folder.to_string();
    tokio::task::spawn_blocking(move || fingerprint_now(&folder))
        .await
        .ok()
        .flatten()
}

pub fn read_base(text: &str) -> Option<Print> {
    if text.trim().is_empty() {
        return None;
    }
    serde_json::from_str(text).ok()
}

pub async fn base_of(folder: &str) -> String {
    match fingerprint(folder).await {
        Some(print) => serde_json::to_string(&print).unwrap_or_default(),
        None => String::new(),
    }
}

pub fn changed_paths(base: &Print, now: &Print) -> Vec<String> {
    let folders: Vec<&String> = base.keys().chain(now.keys()).filter(|k| k.ends_with('/')).collect();
    let mut all: Vec<&String> = base.keys().chain(now.keys()).collect();
    all.sort();
    all.dedup();
    all.into_iter()
        .filter(|p| !tool_cache(p))
        .filter(|p| p.ends_with('/') || !folders.iter().any(|f| p.starts_with(f.as_str())))
        .filter(|p| {
            base.get(*p).map(String::as_str).unwrap_or(CLEAN)
                != now.get(*p).map(String::as_str).unwrap_or(CLEAN)
        })
        .cloned()
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Changes {
    Unread,
    NoBase,
    Files(Vec<String>),
}

pub async fn changes(base: &str, folder: &str) -> Changes {
    let Some(start) = read_base(base) else {
        return if base.trim().is_empty() {
            Changes::NoBase
        } else {
            Changes::Unread
        };
    };
    match fingerprint(folder).await {
        Some(now) => Changes::Files(changed_paths(&start, &now)),
        None => Changes::Unread,
    }
}
