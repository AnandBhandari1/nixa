use super::policy::refused;
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const SEARCH_MATCHES: usize = 50;
pub const SEARCH_MAX_MATCHES: usize = 200;
pub const SEARCH_FILES: usize = 5000;
pub const SEARCH_FILE_BYTES: u64 = 1024 * 1024;
pub const SEARCH_MS: u64 = 5000;
pub const LINE_CHARS: usize = 300;
pub const RANGE_LINES: usize = 200;
pub const RANGE_MAX_LINES: usize = 400;
pub const RANGE_LINE_CHARS: usize = 2000;
pub const SKIP_DIRS: [&str; 16] = ["node_modules", ".git", "dist", "build", "out", "target", ".next", ".turbo", ".cache", "coverage", "vendor", "__pycache__", ".venv", "venv", "Pods", "DerivedData"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Match {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Found {
    pub folder: String,
    pub matches: Vec<Match>,
    pub files: usize,
    pub more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Range {
    pub path: String,
    pub from: usize,
    pub to: usize,
    pub lines: Vec<String>,
    pub end: bool,
}

pub fn clip_line(text: &str, size: usize) -> String {
    let one = text.replace('\t', "  ");
    let one = one.trim_end();
    if one.chars().count() > size {
        let mut out: String = one.chars().take(size.saturating_sub(1)).collect();
        out.push('…');
        out
    } else {
        one.to_string()
    }
}

pub fn matcher(query: &str, regex: bool, case_sensitive: bool) -> Result<Regex, String> {
    if query.trim().is_empty() {
        return Err("query is needed.".into());
    }
    let pattern = if regex { query.to_string() } else { regex::escape(query) };
    RegexBuilder::new(&pattern).case_insensitive(!case_sensitive).size_limit(1 << 20).build().map_err(|e| format!("The regex does not compile: {e}"))
}

pub fn is_text(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else { return false };
    let mut buffer = [0u8; 4096];
    match file.read(&mut buffer) {
        Ok(read) => !buffer[..read].contains(&0),
        Err(_) => false,
    }
}

fn lines_of(path: &Path) -> Option<impl Iterator<Item = String>> {
    let file = File::open(path).ok()?;
    Some(BufReader::new(file).split(b'\n').map_while(Result::ok).map(|raw| {
        let text = String::from_utf8_lossy(&raw).to_string();
        text.strip_suffix('\r').map(str::to_string).unwrap_or(text)
    }))
}

fn relative(folder: &Path, path: &Path) -> String {
    path.strip_prefix(folder).ok().map(|p| p.display().to_string()).filter(|p| !p.is_empty()).unwrap_or_else(|| path.display().to_string())
}

struct Scan<'a> {
    folder: &'a Path,
    pattern: &'a Regex,
    limit: usize,
    found: Found,
}

impl Scan<'_> {
    fn file(&mut self, path: &Path) {
        let Ok(info) = std::fs::metadata(path) else { return };
        if !info.is_file() || info.len() > SEARCH_FILE_BYTES || !is_text(path) || refused(path) {
            return;
        }
        self.found.files += 1;
        let Some(lines) = lines_of(path) else { return };
        for (index, line) in lines.enumerate() {
            if !self.pattern.is_match(&line) {
                continue;
            }
            if self.found.matches.len() >= self.limit {
                self.found.more = true;
                return;
            }
            self.found.matches.push(Match { path: relative(self.folder, path), line: index + 1, text: clip_line(&line, LINE_CHARS) });
        }
    }
}

pub fn search_folder(folder: &Path, start: &Path, pattern: &Regex, limit: usize, budget: Duration) -> Found {
    let began = Instant::now();
    let mut scan = Scan { folder, pattern, limit, found: Found { folder: folder.display().to_string(), ..Found::default() } };
    if start.is_file() {
        scan.file(start);
        return scan.found;
    }
    let mut queue: VecDeque<PathBuf> = VecDeque::from([start.to_path_buf()]);
    'walk: while let Some(dir) = queue.pop_front() {
        if scan.found.more {
            break;
        }
        let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir).map(|r| r.filter_map(|e| e.ok()).collect()).unwrap_or_default();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if scan.found.more {
                break 'walk;
            }
            if scan.found.files >= SEARCH_FILES {
                scan.found.stopped = Some(format!("stopped after {SEARCH_FILES} files"));
                break 'walk;
            }
            if began.elapsed() > budget {
                scan.found.stopped = Some(format!("stopped after {} seconds", budget.as_secs()));
                break 'walk;
            }
            let full = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() || super::policy::protected_path(&full) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if kind.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    queue.push_back(full);
                }
                continue;
            }
            if kind.is_file() {
                scan.file(&full);
            }
        }
    }
    scan.found
}

pub fn read_range(path: &Path, from: usize, count: usize) -> Result<Range, String> {
    let info = std::fs::metadata(path).map_err(|e| format!("{} cannot be read: {e}.", path.display()))?;
    if !info.is_file() {
        return Err("Not a file.".into());
    }
    if !is_text(path) {
        return Err("Not a text file.".into());
    }
    let from = from.max(1);
    let mut lines = Vec::new();
    let mut end = true;
    let all = lines_of(path).ok_or("The file cannot be read.")?;
    for (index, line) in all.enumerate() {
        let number = index + 1;
        if number < from {
            continue;
        }
        if lines.len() >= count {
            end = false;
            break;
        }
        lines.push(format!("{number}: {}", clip_line(&line, RANGE_LINE_CHARS)));
    }
    let to = if lines.is_empty() { from - 1 } else { from + lines.len() - 1 };
    Ok(Range { path: path.display().to_string(), from, to, lines, end })
}

pub fn positive(value: Option<&serde_json::Value>, fallback: usize, max: usize) -> usize {
    let number = value.and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))).filter(|n| n.is_finite()).map(|n| n.floor());
    match number {
        Some(n) if n >= 1.0 => (n.min(max as f64)) as usize,
        _ => fallback,
    }
}
