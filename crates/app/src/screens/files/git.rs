use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Added,
    Modified,
    Deleted,
    Conflict,
}

impl Mark {
    pub fn letter(self) -> &'static str {
        match self {
            Mark::Added => "A",
            Mark::Modified => "M",
            Mark::Deleted => "D",
            Mark::Conflict => "U",
        }
    }
}

pub const MAX_MARKS: usize = 5000;

pub fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn top(dir: &Path) -> Option<PathBuf> {
    let text = git(dir, &["rev-parse", "--show-toplevel"])?;
    let line = text.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

pub fn mark_of(xy: &str) -> Mark {
    if xy == "??" || xy.contains('A') {
        Mark::Added
    } else if xy.contains('U') {
        Mark::Conflict
    } else if xy.contains('D') {
        Mark::Deleted
    } else {
        Mark::Modified
    }
}

pub fn parse_status(text: &str) -> Vec<(String, Mark)> {
    text.split('\0')
        .filter(|e| e.len() > 3)
        .take(MAX_MARKS)
        .map(|e| {
            let path = e[3..].trim_end_matches('/').to_string();
            (path, mark_of(&e[..2]))
        })
        .collect()
}

pub fn marks(root: &str) -> HashMap<String, Mark> {
    let root = Path::new(root);
    let Some(top) = top(root) else {
        return HashMap::new();
    };
    let Some(text) = git(
        root,
        &[
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=normal",
        ],
    ) else {
        return HashMap::new();
    };
    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let top = top.canonicalize().unwrap_or(top);
    parse_status(&text)
        .into_iter()
        .filter_map(|(path, mark)| {
            let full = top.join(&path);
            if let Ok(rel) = full.strip_prefix(&base) {
                return Some((rel.to_string_lossy().to_string(), mark));
            }
            (mark == Mark::Added && base.starts_with(&full)).then(|| (String::new(), mark))
        })
        .collect()
}

pub fn mark_for(marks: &HashMap<String, Mark>, path: &str) -> Option<Mark> {
    if let Some(mark) = marks.get(path) {
        return Some(*mark);
    }
    let mut at = path;
    loop {
        at = match at.rfind('/') {
            Some(cut) => &at[..cut],
            None if at.is_empty() => return None,
            None => "",
        };
        if marks.get(at) == Some(&Mark::Added) {
            return Some(Mark::Added);
        }
        if at.is_empty() {
            return None;
        }
    }
}

pub fn index_path(root: &str) -> Option<PathBuf> {
    let dir = git(Path::new(root), &["rev-parse", "--absolute-git-dir"])?;
    Some(Path::new(dir.trim()).join("index"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn status_letters() {
        assert_eq!(mark_of("??"), Mark::Added);
        assert_eq!(mark_of("A "), Mark::Added);
        assert_eq!(mark_of(" M"), Mark::Modified);
        assert_eq!(mark_of(" D"), Mark::Deleted);
        assert_eq!(mark_of("UU"), Mark::Conflict);
        let list = parse_status(" M src/a.rs\0?? new/\0D  gone.txt\0");
        assert_eq!(
            list,
            vec![
                ("src/a.rs".to_string(), Mark::Modified),
                ("new".to_string(), Mark::Added),
                ("gone.txt".to_string(), Mark::Deleted),
            ]
        );
    }

    #[test]
    fn marks_are_relative_to_the_shown_folder() {
        let dir = std::env::temp_dir().join(format!("nixa-files-git-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("sub")).unwrap();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        fs::write(dir.join("sub/new.txt"), "x").unwrap();
        fs::write(dir.join("top.txt"), "x").unwrap();
        let sub = marks(&dir.join("sub").display().to_string());
        assert_eq!(mark_for(&sub, "new.txt"), Some(Mark::Added));
        assert_eq!(sub.len(), 1);
        let all = marks(&dir.display().to_string());
        assert_eq!(all.get("top.txt"), Some(&Mark::Added));
        assert_eq!(all.get("sub"), Some(&Mark::Added));
        assert_eq!(mark_for(&all, "sub/new.txt"), Some(Mark::Added));
        assert_eq!(mark_for(&all, "other.txt"), None);
        fs::remove_dir_all(&dir).ok();
        assert!(marks("/").is_empty() || top(Path::new("/")).is_some());
    }
}
