use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

pub const GIT_SECS: u64 = 15;
pub type Changes = BTreeMap<String, String>;

fn stamp(folder: &Path, path: &str) -> String {
    std::fs::metadata(folder.join(path))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|| "gone".into())
}

pub fn parse_status(folder: &Path, out: &[u8]) -> Changes {
    let text = String::from_utf8_lossy(out);
    let parts: Vec<&str> = text.split('\0').collect();
    let mut found = Changes::new();
    let mut at = 0;
    while at < parts.len() {
        let entry = parts[at];
        at += 1;
        if entry.len() < 4 || !entry.is_char_boundary(3) {
            continue;
        }
        let code = &entry[..2];
        let path = &entry[3..];
        if code.starts_with('R') || code.starts_with('C') {
            at += 1;
        }
        found.insert(path.to_string(), format!("{code}:{}", stamp(folder, path)));
    }
    found
}

pub async fn git_changes(folder: &str) -> Changes {
    let run = tokio::process::Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["status", "--porcelain=v1", "-z", "-uall"])
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(Duration::from_secs(GIT_SECS), run).await {
        Ok(Ok(out)) if out.status.success() => parse_status(Path::new(folder), &out.stdout),
        _ => Changes::new(),
    }
}

pub fn changed_files(before: &Changes, after: &Changes) -> Vec<String> {
    let mut out: Vec<String> = after
        .keys()
        .chain(before.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}
