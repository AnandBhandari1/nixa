use serde::{Deserialize, Serialize};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const MARK: &str = "nixa heavy helper";
pub const HELD_ENV: &str = "NIXA_HEAVY_HELD";
pub const SETTINGS_FILE: &str = "heavy.json";
pub const TOOLS: [&str; 9] = ["cargo", "xcodebuild", "gradle", "swift", "go", "npm", "pnpm", "yarn", "bun"];
pub const DEFAULT_SLOTS: usize = 2;
pub const MAX_SLOTS: usize = 16;
pub const SCAN_BYTES: usize = 512;
pub const STALE_MS: u64 = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub on: bool,
    pub slots: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { on: true, slots: DEFAULT_SLOTS }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub pid: u32,
    pub command: String,
    pub folder: String,
    pub terminal: String,
    pub since: u64,
    pub slot: usize,
}

pub fn dir(data: &Path) -> PathBuf {
    data.join("heavy")
}

pub fn bin_dir(data: &Path) -> PathBuf {
    dir(data).join("bin")
}

pub fn slots_dir(data: &Path) -> PathBuf {
    dir(data).join("slots")
}

pub fn wait_dir(data: &Path) -> PathBuf {
    dir(data).join("wait")
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn clamp(slots: usize) -> usize {
    slots.clamp(1, MAX_SLOTS)
}

pub fn load(data: &Path) -> Settings {
    let mut s: Settings = std::fs::read_to_string(data.join(SETTINGS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    s.slots = clamp(s.slots);
    s
}

pub fn save(data: &Path, s: &Settings) -> Result<Settings, String> {
    let s = Settings { on: s.on, slots: clamp(s.slots) };
    let file = data.join(SETTINGS_FILE);
    let temp = data.join(format!(".{SETTINGS_FILE}.{}", std::process::id()));
    let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(data).map_err(|e| format!("cannot make {}: {e}", data.display()))?;
    std::fs::write(&temp, text).map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
    std::fs::rename(&temp, &file).map_err(|e| format!("cannot save {}: {e}", file.display()))?;
    Ok(s)
}

fn first_word(args: &[String]) -> Option<(usize, &str)> {
    args.iter().enumerate().find(|(_, a)| !a.starts_with('-') && !a.starts_with('+')).map(|(i, a)| (i, a.as_str()))
}

fn script_heavy(name: &str) -> bool {
    name == "build" || name == "test" || name.starts_with("build:") || name.starts_with("test:")
}

pub fn is_heavy(tool: &str, args: &[String]) -> bool {
    if args.iter().any(|a| a == "--watch" || a == "--help" || a == "--version") {
        return false;
    }
    let first = first_word(args);
    let word = first.map(|(_, w)| w).unwrap_or("");
    let second = first.and_then(|(i, _)| args.get(i + 1)).map(String::as_str).unwrap_or("");
    match tool {
        "cargo" => matches!(word, "build" | "b" | "check" | "c" | "clippy" | "test" | "t" | "bench" | "doc" | "nextest"),
        "xcodebuild" => !args.iter().any(|a| matches!(a.as_str(), "-version" | "-showsdks" | "-list" | "-help" | "-usage" | "-showBuildSettings" | "-showdestinations")),
        "gradle" => !args.iter().any(|a| matches!(a.as_str(), "-v" | "-h" | "--stop" | "--status")),
        "swift" => matches!(word, "build" | "test"),
        "go" => matches!(word, "build" | "test" | "vet" | "install"),
        "npm" => matches!(word, "test" | "t" | "tst") || (matches!(word, "run" | "run-script") && script_heavy(second)),
        "pnpm" | "yarn" | "bun" => script_heavy(word) || (word == "run" && script_heavy(second)),
        _ => false,
    }
}

pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

pub const SCRIPT: &str = r##"#!/bin/sh
n=@MARK@
if [ -z "$NIXA_HEAVY_HELD" ] && [ -x @BIN@ ]; then
  exec @BIN@ heavy --tool @TOOL@ -- "$@"
fi
PATH=$(printf '%s' "$PATH" | tr ':' '\n' | grep -vxF @DIR@ | paste -sd: -)
export PATH
command -v @TOOL@ >/dev/null 2>&1 || { echo "$n: @TOOL@ was not found on PATH." >&2; exit 127; }
exec @TOOL@ "$@"
"##;

pub fn script(nixa_bin: &Path, bin_dir: &Path, tool: &str) -> Result<String, String> {
    let bin = nixa_bin.display().to_string();
    let dir = bin_dir.display().to_string();
    if !nixa_bin.is_absolute() || bin.contains(['\n', '\r', '\0']) || dir.contains(['\n', '\r', '\0']) {
        return Err(format!("the nixa command path {bin} cannot be used for the {MARK}"));
    }
    Ok(SCRIPT.replace("@MARK@", &quote(MARK)).replace("@BIN@", &quote(&bin)).replace("@DIR@", &quote(&dir)).replace("@TOOL@", tool))
}

pub fn write_helpers(data: &Path, nixa_bin: &Path) -> Result<PathBuf, String> {
    let dir = bin_dir(data);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    for folder in dir.ancestors().take_while(|a| *a != data && a.starts_with(data)) {
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("cannot set {}: {e}", folder.display()))?;
    }
    for tool in TOOLS {
        let text = script(nixa_bin, &dir, tool)?;
        let file = dir.join(tool);
        let same = std::fs::read_to_string(&file).is_ok_and(|t| t == text) && std::fs::metadata(&file).is_ok_and(|m| m.permissions().mode() & 0o777 == 0o700);
        if same {
            continue;
        }
        let temp = dir.join(format!(".{tool}.{}", std::process::id()));
        std::fs::write(&temp, text).map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("cannot set {}: {e}", temp.display()))?;
        std::fs::rename(&temp, &file).map_err(|e| format!("cannot place {}: {e}", file.display()))?;
    }
    Ok(dir)
}

pub fn ensure(data: &Path, nixa_bin: Option<&Path>) -> String {
    let Some(bin) = nixa_bin else {
        return format!("{MARK}: the nixa command was not found, so builds run without slots.");
    };
    match write_helpers(data, bin) {
        Ok(dir) => format!("{MARK}: ready at {}", dir.display()),
        Err(e) => format!("{MARK}: {e}. Builds run without slots."),
    }
}

fn runs(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn ready(data: &Path) -> bool {
    TOOLS.iter().all(|t| runs(&bin_dir(data).join(t)))
}

pub fn path_with(data: &Path, path: &str) -> String {
    if !ready(data) {
        return path.to_string();
    }
    let dir = bin_dir(data).display().to_string();
    std::iter::once(dir.as_str()).chain(path.split(':').filter(|p| !p.is_empty() && *p != dir)).collect::<Vec<_>>().join(":")
}

pub fn line_prefix(data: &Path) -> Option<String> {
    ready(data).then(|| quote(&bin_dir(data).display().to_string()))
}

pub fn is_helper(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    String::from_utf8_lossy(&bytes[..bytes.len().min(SCAN_BYTES)]).contains(MARK)
}

fn id_of(path: &Path) -> Option<(u64, u64)> {
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

pub fn plain_name(tool: &str) -> bool {
    !tool.is_empty() && tool != "." && tool != ".." && !tool.contains('/') && !tool.contains('\0')
}

pub fn find_real(tool: &str, path: &str, skip: &[PathBuf]) -> Option<PathBuf> {
    if !plain_name(tool) {
        return None;
    }
    let blocked: Vec<(u64, u64)> = skip.iter().filter_map(|p| id_of(p)).collect();
    std::env::split_paths(path)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(tool))
        .find(|f| runs(f) && !is_helper(f) && id_of(f).is_some_and(|id| !blocked.contains(&id)))
}

pub fn ignore_stop_keys() {
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
    }
}

pub fn alive(pid: u32) -> bool {
    pid > 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 || *libc::__error() == libc::EPERM }
}

fn read_job(file: &Path) -> Option<Job> {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok())
}

fn age_ms(path: &Path) -> u64 {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn slot_number(name: &str) -> Option<usize> {
    name.strip_prefix("slot-").and_then(|n| n.parse().ok())
}

pub fn busy(data: &Path) -> Vec<Job> {
    let Ok(read) = std::fs::read_dir(slots_dir(data)) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(n) = slot_number(&name) else { continue };
        let path = entry.path();
        match read_job(&path.join("job.json")) {
            Some(job) if alive(job.pid) => out.push(Job { slot: n, ..job }),
            Some(_) => clear_slot(&path),
            None if age_ms(&path) > STALE_MS => clear_slot(&path),
            None => out.push(Job { slot: n, command: "starting".into(), ..Job::default() }),
        }
    }
    out.sort_by_key(|j| j.slot);
    out
}

pub fn waiting(data: &Path) -> Vec<Job> {
    let Ok(read) = std::fs::read_dir(wait_dir(data)) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        match read_job(&path) {
            Some(job) if alive(job.pid) => out.push(job),
            _ if age_ms(&path) > STALE_MS => {
                let _ = std::fs::remove_file(&path);
            }
            _ => {}
        }
    }
    out.sort_by_key(|j| (j.since, j.pid));
    out
}

fn clear_slot(path: &Path) {
    let _ = std::fs::remove_file(path.join("job.json"));
    let _ = std::fs::remove_dir(path);
}

pub fn take(data: &Path, slots: usize, job: &Job) -> Option<PathBuf> {
    let root = slots_dir(data);
    std::fs::create_dir_all(&root).ok()?;
    busy(data);
    for n in 1..=clamp(slots) {
        let path = root.join(format!("slot-{n}"));
        if std::fs::create_dir(&path).is_ok() {
            let job = Job { slot: n, ..job.clone() };
            let text = serde_json::to_string(&job).unwrap_or_default();
            if std::fs::write(path.join("job.json"), text).is_err() {
                clear_slot(&path);
                return None;
            }
            return Some(path);
        }
    }
    None
}

pub fn release(slot: &Path) {
    clear_slot(slot);
}

pub fn wait_file(data: &Path, pid: u32) -> PathBuf {
    wait_dir(data).join(format!("{pid}.json"))
}

pub fn join_queue(data: &Path, job: &Job) -> Option<PathBuf> {
    std::fs::create_dir_all(wait_dir(data)).ok()?;
    let file = wait_file(data, job.pid);
    std::fs::write(&file, serde_json::to_string(job).ok()?).ok()?;
    Some(file)
}

pub fn my_turn(data: &Path, slots: usize, pid: u32) -> bool {
    let held = busy(data);
    let used = held.iter().filter(|j| j.slot <= clamp(slots)).count();
    let free = clamp(slots).saturating_sub(used);
    let rank = waiting(data).iter().filter(|j| !held.iter().any(|h| h.pid == j.pid)).position(|j| j.pid == pid).unwrap_or(0);
    rank < free
}

pub fn status(data: &Path) -> serde_json::Value {
    let s = load(data);
    serde_json::json!({"on": s.on, "slots": s.slots, "tools": TOOLS, "busy": busy(data), "waiting": waiting(data), "ready": ready(data)})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn builds_and_tests_are_heavy_and_light_commands_are_not() {
        for (tool, args) in [("cargo", "build -p x"), ("cargo", "+nightly test"), ("cargo", "-q clippy"), ("go", "test ./..."), ("npm", "run build"), ("npm", "test"), ("pnpm", "build"), ("bun", "run test:unit"), ("xcodebuild", "-scheme App build"), ("gradle", "assemble"), ("swift", "build")] {
            assert!(is_heavy(tool, &words(args)), "{tool} {args}");
        }
        for (tool, args) in [("cargo", "fmt"), ("cargo", "run"), ("cargo", "--version"), ("npm", "install"), ("npm", "run dev"), ("bun", "test --watch"), ("pnpm", "add x"), ("xcodebuild", "-version"), ("go", "mod tidy"), ("git", "status")] {
            assert!(!is_heavy(tool, &words(args)), "{tool} {args}");
        }
    }

    #[test]
    fn slots_fill_up_and_a_dead_holder_frees_its_slot() {
        let dir = tempfile::tempdir().unwrap();
        let me = Job { pid: std::process::id(), command: "cargo test".into(), ..Job::default() };
        let a = take(dir.path(), 2, &me).unwrap();
        let b = take(dir.path(), 2, &me).unwrap();
        assert!(take(dir.path(), 2, &me).is_none());
        assert_eq!(busy(dir.path()).len(), 2);
        release(&a);
        assert_eq!(busy(dir.path()).len(), 1);
        std::fs::write(b.join("job.json"), serde_json::to_string(&Job { pid: 999_999, ..me.clone() }).unwrap()).unwrap();
        assert!(busy(dir.path()).is_empty());
        assert!(take(dir.path(), 2, &me).is_some());
    }

    #[test]
    fn the_oldest_waiter_goes_first() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let parent = std::os::unix::process::parent_id();
        join_queue(dir.path(), &Job { pid: parent, since: 1, ..Job::default() }).unwrap();
        join_queue(dir.path(), &Job { pid: me, since: 2, ..Job::default() }).unwrap();
        assert!(!my_turn(dir.path(), 1, me));
        assert!(my_turn(dir.path(), 2, me));
    }

    #[test]
    fn settings_default_to_two_slots_and_stay_in_range() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Settings { on: true, slots: 2 });
        assert_eq!(save(dir.path(), &Settings { on: false, slots: 99 }).unwrap().slots, MAX_SLOTS);
        assert_eq!(load(dir.path()), Settings { on: false, slots: MAX_SLOTS });
    }

    #[test]
    fn helpers_are_written_and_never_found_as_the_real_tool() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("nixa");
        std::fs::write(&bin, "").unwrap();
        let helpers = write_helpers(dir.path(), &bin).unwrap();
        assert!(ready(dir.path()));
        assert!(is_helper(&helpers.join("cargo")));
        let path = format!("{}:/usr/bin", helpers.display());
        assert_eq!(find_real("cargo", &path, &[]), None);
        assert_eq!(find_real("/bin/sh", "/usr/bin:/bin", &[]), None);
        assert_eq!(find_real("../bin/sh", "/usr/bin:/bin", &[]), None);
        assert!(find_real("sh", "/usr/bin:/bin", &[]).is_some());
        assert!(path_with(dir.path(), "/usr/bin:/bin").starts_with(&helpers.display().to_string()));
    }
}
