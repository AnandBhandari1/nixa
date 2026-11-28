use super::cache::{AgentModels, CachedModels, Efforts, ModelCache, is_zen_model};
use crate::time::DAY_MS;
use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub const MODELS_MAX_AGE_MS: i64 = DAY_MS;
pub const LIST_TIMEOUT: Duration = Duration::from_secs(60);
pub const OUTPUT_GRACE: Duration = Duration::from_millis(500);
pub const LIST_AGENTS: [&str; 12] = ["claude", "codex", "omp", "agy", "cursor", "opencode", "gemini", "jcode", "hermes", "omo", "crush", "dim"];
const MAX_OUTPUT: usize = 16 * 1024 * 1024;
const EXTRA_DIRS: [&str; 6] = [".local/bin", ".opencode/bin", ".kimi-code/bin", ".cargo/bin", ".bun/bin", ".npm-global/bin"];
const SYSTEM_DIRS: [&str; 4] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];

pub const MODEL_COMMANDS: [(&str, &[&str]); 5] = [
    ("omp", &["omp", "models"]),
    ("agy", &["agy", "models"]),
    ("codex", &["codex", "debug", "models"]),
    ("cursor", &["cursor-agent", "models"]),
    ("opencode", &["opencode", "models"]),
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ListResult {
    pub agent: String,
    pub models: Vec<String>,
    pub efforts: Efforts,
    pub at: i64,
    pub stale: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("list pattern")
}

static SLUG: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-z0-9][a-z0-9.-]*$"));
static AGY_EFFORT: LazyLock<Regex> = LazyLock::new(|| re(r"^(.*)-(low|medium|high|xhigh|max)$"));
static AGY_SPLIT: LazyLock<Regex> = LazyLock::new(|| re(r"\t|\s{2,}"));
static OPENCODE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^[a-z0-9][a-z0-9._-]*/\S+$"));
static OMP_HEAD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^([a-z0-9][a-z0-9._-]*)\s+\(\d+\)$"));
static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-z]+$"));

fn all(efforts: &[&str]) -> Efforts {
    Efforts::from([("*".to_string(), efforts.iter().map(|e| e.to_string()).collect())])
}

pub fn fixed_models() -> Vec<(&'static str, AgentModels)> {
    let claude = ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];
    vec![
        ("claude", AgentModels { models: claude.iter().map(|m| m.to_string()).collect(), efforts: all(&["default", "low", "medium", "high", "xhigh", "max"]) }),
        ("gemini", AgentModels { models: Vec::new(), efforts: all(&["default"]) }),
        ("jcode", AgentModels { models: vec!["default".into()], efforts: all(&["default"]) }),
        ("hermes", AgentModels { models: vec!["default".into()], efforts: all(&["default"]) }),
    ]
}

pub fn fixed_in(dirs: &[PathBuf]) -> Vec<(&'static str, AgentModels)> {
    fixed_models().into_iter().filter(|(agent, _)| find_program(agent, dirs).is_some()).collect()
}

pub fn installed_fixed() -> Vec<(&'static str, AgentModels)> {
    if cfg!(test) {
        return fixed_models();
    }
    fixed_in(&search_dirs())
}

pub fn fixed_for(agent: &str) -> Option<AgentModels> {
    fixed_models().into_iter().find(|(a, _)| *a == agent).map(|(_, list)| list)
}

pub fn command_for(agent: &str) -> Option<&'static [&'static str]> {
    MODEL_COMMANDS.iter().find(|(a, _)| *a == agent).map(|(_, c)| *c)
}

fn parse_codex(output: &str) -> Result<AgentModels> {
    let start = output.find('{').unwrap_or(0);
    let raw: Value = serde_json::from_str(&output[start..]).context("codex gave no model JSON")?;
    let mut list = AgentModels::default();
    for item in raw.get("models").and_then(Value::as_array).into_iter().flatten() {
        let Some(slug) = item.get("slug").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            continue;
        };
        if matches!(item.get("visibility").and_then(Value::as_str), Some("hide" | "hidden")) {
            continue;
        }
        let mut efforts = vec!["default".to_string()];
        for level in item.get("supported_reasoning_levels").and_then(Value::as_array).into_iter().flatten() {
            if let Some(effort) = level.get("effort").and_then(Value::as_str).filter(|e| !e.is_empty()) {
                efforts.push(effort.to_string());
            }
        }
        list.models.push(slug.to_string());
        list.efforts.insert(slug.to_string(), efforts);
    }
    Ok(list)
}

fn parse_agy(lines: &[&str]) -> AgentModels {
    let mut bases: Vec<(String, Vec<String>)> = Vec::new();
    for line in lines {
        let slug = AGY_SPLIT.split(line).next().unwrap_or_default();
        if !SLUG.is_match(slug) {
            continue;
        }
        let (base, effort) = match AGY_EFFORT.captures(slug) {
            Some(found) => (found[1].to_string(), Some(found[2].to_string())),
            None => (slug.to_string(), None),
        };
        let at = match bases.iter().position(|(b, _)| *b == base) {
            Some(at) => at,
            None => {
                bases.push((base, Vec::new()));
                bases.len() - 1
            }
        };
        if let Some(effort) = effort {
            bases[at].1.push(effort);
        }
    }
    let mut list = AgentModels::default();
    for (base, efforts) in bases {
        list.efforts.insert(base.clone(), if efforts.is_empty() { vec!["default".into()] } else { efforts });
        list.models.push(base);
    }
    list
}

fn parse_omp(lines: &[&str]) -> AgentModels {
    let mut list = AgentModels::default();
    let mut provider = String::new();
    for line in lines {
        if let Some(head) = OMP_HEAD.captures(line) {
            provider = head[1].to_string();
            continue;
        }
        if provider.is_empty() || !line.starts_with('│') {
            continue;
        }
        let parts: Vec<&str> = line.split('│').map(str::trim).collect();
        let cells: Vec<&str> = parts.iter().enumerate().filter(|(i, _)| *i > 0 && *i + 1 < parts.len()).map(|(_, c)| *c).collect();
        if cells.len() < 4 || cells[0] == "model" {
            continue;
        }
        let id = format!("{provider}/{}", cells[0]);
        let mut efforts = vec!["default".to_string()];
        efforts.extend(cells[3].split(',').map(str::trim).filter(|w| WORD.is_match(w)).map(str::to_string));
        list.models.push(id.clone());
        list.efforts.insert(id, efforts);
    }
    list
}

pub fn parse_models(agent: &str, output: &str) -> Result<AgentModels> {
    let lines: Vec<&str> = output.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    Ok(match agent {
        "codex" => parse_codex(output)?,
        "agy" => parse_agy(&lines),
        "cursor" => AgentModels {
            models: lines.iter().map(|l| l.split(" - ").next().unwrap_or_default().trim().to_string()).filter(|s| SLUG.is_match(s)).collect(),
            efforts: all(&["default"]),
        },
        "opencode" => AgentModels { models: lines.iter().filter(|l| OPENCODE.is_match(l) && !is_zen_model("opencode", l)).map(|l| l.to_string()).collect(), efforts: all(&["default"]) },
        "omp" => parse_omp(&lines),
        _ => AgentModels { models: Vec::new(), efforts: all(&["default"]) },
    })
}

pub fn put_models(cache: &mut ModelCache, agent: &str, list: &AgentModels, at: i64) {
    let entry = cache.agents.entry(agent.to_string()).or_default();
    entry.error.clear();
    entry.at = at;
    entry.models = list.models.clone();
    entry.efforts = list.efforts.clone();
}

pub fn save_live(conn: &Connection, agent: &str, list: &AgentModels, now: i64) -> Result<bool> {
    if !LIST_AGENTS.contains(&agent) || fixed_for(agent).is_some() || list.models.is_empty() {
        return Ok(false);
    }
    super::update(conn, |cache| {
        put_models(cache, agent, list, now);
        Ok(())
    })?;
    Ok(true)
}

pub fn put_model_error(cache: &mut ModelCache, agent: &str, error: &str, at: i64) {
    let entry = cache.agents.entry(agent.to_string()).or_insert_with(|| CachedModels { at, ..CachedModels::default() });
    entry.error = error.to_string();
}

pub fn models_stale(entry: Option<&CachedModels>, now: i64) -> bool {
    entry.is_none_or(|e| e.models.is_empty() || now - e.at > MODELS_MAX_AGE_MS)
}

pub fn search_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if let Some(path) = std::env::var_os("PATH") {
        std::env::split_paths(&path).filter(|p| !p.as_os_str().is_empty()).for_each(&mut add);
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        EXTRA_DIRS.iter().for_each(|d| add(home.join(d)));
    }
    SYSTEM_DIRS.iter().for_each(|d| add(PathBuf::from(d)));
    out
}

fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

pub fn find_program(program: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    if program.contains('/') {
        let path = PathBuf::from(program);
        return runnable(&path).then_some(path);
    }
    dirs.iter().map(|d| d.join(program)).find(|p| runnable(p))
}

fn read_capped(mut from: impl Read) -> String {
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    while let Ok(n) = from.read(&mut buf) {
        if n == 0 {
            break;
        }
        if out.len() < MAX_OUTPUT {
            out.extend_from_slice(&buf[..n.min(MAX_OUTPUT - out.len())]);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

static LIVE_GROUPS: LazyLock<Mutex<HashSet<u32>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

fn live_groups() -> MutexGuard<'static, HashSet<u32>> {
    LIVE_GROUPS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn group_owned(leader_held: bool, group: u32, member_groups: &[u32]) -> bool {
    group > 1 && (leader_held || member_groups.contains(&group))
}

pub fn may_signal_group(group: u32, tree: &Tree, leader_held: bool) -> bool {
    let member_groups: Vec<u32> = tree
        .iter()
        .filter(|(pid, seen)| native::info(**pid).is_some_and(|(_, start)| start == seen.start))
        .filter_map(|(pid, _)| native::group_of(*pid))
        .collect();
    group_owned(leader_held, group, &member_groups)
}

pub fn kill_group(group: u32, tree: &Tree, leader_held: bool) {
    if may_signal_group(group, tree, leader_held)
        && let Ok(group) = i32::try_from(group)
    {
        native::send(-group, SIGKILL);
    }
}

pub fn running_lists() -> usize {
    live_groups().len()
}

pub const WATCH_EVERY: Duration = Duration::from_millis(100);
pub const TERM_WAIT: Duration = Duration::from_millis(500);
const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;
const LOOK_ROUNDS: usize = 16;

#[cfg(target_os = "linux")]
pub type Handle = Arc<std::os::fd::OwnedFd>;
#[cfg(not(target_os = "linux"))]
pub type Handle = ();

#[derive(Clone, Debug)]
pub struct Seen {
    pub start: u64,
    pub handle: Option<Handle>,
}

pub type Tree = HashMap<u32, Seen>;

#[cfg(target_os = "macos")]
pub mod native {
    use super::Handle;
    use std::ffi::c_void;

    const PROC_PIDTBSDINFO: i32 = 3;

    #[repr(C)]
    pub struct BsdInfo {
        pub flags: u32,
        pub status: u32,
        pub xstatus: u32,
        pub pid: u32,
        pub ppid: u32,
        pub uid: u32,
        pub gid: u32,
        pub ruid: u32,
        pub rgid: u32,
        pub svuid: u32,
        pub svgid: u32,
        pub rfu: u32,
        pub comm: [u8; 16],
        pub name: [u8; 32],
        pub nfiles: u32,
        pub pgid: u32,
        pub pjobc: u32,
        pub tdev: u32,
        pub tpgid: u32,
        pub nice: i32,
        pub start_sec: u64,
        pub start_usec: u64,
    }

    unsafe extern "C" {
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, size: i32) -> i32;
        fn proc_listchildpids(ppid: i32, buffer: *mut c_void, size: i32) -> i32;
        fn kill(pid: i32, sig: i32) -> i32;
        fn waitid(idtype: u32, id: u32, infop: *mut c_void, options: i32) -> i32;
    }

    const P_PID: u32 = 1;
    const WEXITED: i32 = 0x04;
    const WNOHANG: i32 = 0x01;
    const WNOWAIT: i32 = 0x20;
    const SI_PID_AT: usize = 12;

    pub fn exited(pid: u32) -> Option<bool> {
        let mut info = [0u8; 128];
        let done = unsafe { waitid(P_PID, pid, info.as_mut_ptr().cast(), WEXITED | WNOHANG | WNOWAIT) };
        if done != 0 {
            return None;
        }
        let found = i32::from_ne_bytes(info[SI_PID_AT..SI_PID_AT + 4].try_into().ok()?);
        Some(found != 0)
    }

    pub fn info(pid: u32) -> Option<(u32, u64)> {
        let size = std::mem::size_of::<BsdInfo>() as i32;
        let mut out = std::mem::MaybeUninit::<BsdInfo>::zeroed();
        let got = unsafe { proc_pidinfo(i32::try_from(pid).ok()?, PROC_PIDTBSDINFO, 0, out.as_mut_ptr().cast(), size) };
        if got != size {
            return None;
        }
        let out = unsafe { out.assume_init() };
        (out.pid == pid).then_some((out.ppid, out.start_sec.saturating_mul(1_000_000).saturating_add(out.start_usec)))
    }

    pub fn children(pid: u32) -> Vec<u32> {
        let Ok(pid) = i32::try_from(pid) else { return Vec::new() };
        let mut found = vec![0i32; 4096];
        let bytes = (found.len() * std::mem::size_of::<i32>()) as i32;
        let count = unsafe { proc_listchildpids(pid, found.as_mut_ptr().cast(), bytes) };
        if count <= 0 {
            return Vec::new();
        }
        found.truncate((count as usize).min(found.len()));
        found.into_iter().filter(|p| *p > 1).map(|p| p as u32).collect()
    }

    pub fn group_of(pid: u32) -> Option<u32> {
        let size = std::mem::size_of::<BsdInfo>() as i32;
        let mut out = std::mem::MaybeUninit::<BsdInfo>::zeroed();
        let got = unsafe { proc_pidinfo(i32::try_from(pid).ok()?, PROC_PIDTBSDINFO, 0, out.as_mut_ptr().cast(), size) };
        if got != size {
            return None;
        }
        let out = unsafe { out.assume_init() };
        (out.pid == pid).then_some(out.pgid)
    }

    pub fn open(_pid: u32) -> Option<Handle> {
        None
    }

    pub fn send_handle(_handle: &Handle, _sig: i32) -> bool {
        false
    }

    pub fn send(pid: i32, sig: i32) {
        unsafe {
            kill(pid, sig);
        }
    }
}

#[cfg(target_os = "linux")]
pub mod native {
    use super::Handle;
    use std::ffi::{c_int, c_long, c_uint, c_void};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::Arc;

    const SYS_PIDFD_SEND_SIGNAL: c_long = 424;
    const SYS_PIDFD_OPEN: c_long = 434;

    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
        fn kill(pid: c_int, sig: c_int) -> c_int;
        fn waitid(idtype: c_uint, id: c_uint, infop: *mut c_void, options: c_int) -> c_int;
    }

    const P_PID: c_uint = 1;
    const WEXITED: c_int = 0x04;
    const WNOHANG: c_int = 0x01;
    const WNOWAIT: c_int = 0x0100_0000;
    const SI_PID_AT: usize = 16;

    pub fn exited(pid: u32) -> Option<bool> {
        let mut info = [0u8; 128];
        let done = unsafe { waitid(P_PID, pid, info.as_mut_ptr().cast(), WEXITED | WNOHANG | WNOWAIT) };
        if done != 0 {
            return None;
        }
        let found = i32::from_ne_bytes(info[SI_PID_AT..SI_PID_AT + 4].try_into().ok()?);
        Some(found != 0)
    }

    pub fn info(pid: u32) -> Option<(u32, u64)> {
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let rest = &text[text.rfind(')')? + 1..];
        let fields: Vec<&str> = rest.split_whitespace().collect();
        Some((fields.get(1)?.parse().ok()?, fields.get(19)?.parse().ok()?))
    }

    pub fn group_of(pid: u32) -> Option<u32> {
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let rest = &text[text.rfind(')')? + 1..];
        rest.split_whitespace().nth(2)?.parse().ok()
    }

    pub fn children(pid: u32) -> Vec<u32> {
        let Ok(dirs) = std::fs::read_dir("/proc") else { return Vec::new() };
        dirs.filter_map(|d| d.ok()?.file_name().to_str()?.parse::<u32>().ok()).filter(|p| info(*p).is_some_and(|(ppid, _)| ppid == pid)).collect()
    }

    pub fn open(pid: u32) -> Option<Handle> {
        let fd = unsafe { syscall(SYS_PIDFD_OPEN, pid as c_int, 0 as c_uint) };
        if fd < 0 {
            return None;
        }
        Some(Arc::new(unsafe { OwnedFd::from_raw_fd(fd as c_int) }))
    }

    pub fn send_handle(handle: &Handle, sig: i32) -> bool {
        unsafe { syscall(SYS_PIDFD_SEND_SIGNAL, handle.as_raw_fd(), sig as c_int, std::ptr::null::<c_void>(), 0 as c_uint) == 0 }
    }

    pub fn send(pid: i32, sig: i32) {
        unsafe {
            kill(pid, sig);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub mod native {
    use super::Handle;

    pub fn info(_pid: u32) -> Option<(u32, u64)> {
        None
    }

    pub fn exited(_pid: u32) -> Option<bool> {
        None
    }

    pub fn children(_pid: u32) -> Vec<u32> {
        Vec::new()
    }

    pub fn group_of(_pid: u32) -> Option<u32> {
        None
    }

    pub fn open(_pid: u32) -> Option<Handle> {
        None
    }

    pub fn send_handle(_handle: &Handle, _sig: i32) -> bool {
        false
    }

    pub fn send(_pid: i32, _sig: i32) {}
}

static LIVE_TREES: LazyLock<Mutex<HashMap<u32, Tree>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn live_trees() -> MutexGuard<'static, HashMap<u32, Tree>> {
    LIVE_TREES.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn seed_tree(root: u32) -> Tree {
    let mut tree = Tree::new();
    if let Some((_, start)) = native::info(root) {
        let handle = native::open(root);
        if native::info(root).is_some_and(|(_, again)| again == start) {
            tree.insert(root, Seen { start, handle });
        }
    }
    tree
}

pub fn grow_tree(tree: &mut Tree, list: &[(u32, u32, u64)]) -> Vec<u32> {
    let mut added = Vec::new();
    loop {
        let parents: HashSet<u32> = list.iter().filter(|(pid, _, start)| tree.get(pid).is_some_and(|s| s.start == *start)).map(|(pid, _, _)| *pid).collect();
        let new: Vec<(u32, u64)> = list.iter().filter(|(pid, ppid, _)| !tree.contains_key(pid) && parents.contains(ppid)).map(|(pid, _, start)| (*pid, *start)).collect();
        if new.is_empty() {
            return added;
        }
        for (pid, start) in new {
            tree.insert(pid, Seen { start, handle: None });
            added.push(pid);
        }
    }
}

pub fn alive_in(tree: &Tree, list: &[(u32, u32, u64)]) -> Vec<u32> {
    list.iter().filter(|(pid, _, start)| tree.get(pid).is_some_and(|s| s.start == *start)).map(|(pid, _, _)| *pid).collect()
}

fn snapshot(tree: &Tree) -> Vec<(u32, u32, u64)> {
    let mut list: Vec<(u32, u32, u64)> = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    for pid in tree.keys() {
        for each in std::iter::once(*pid).chain(native::children(*pid)) {
            if seen.insert(each)
                && let Some((ppid, start)) = native::info(each)
            {
                list.push((each, ppid, start));
            }
        }
    }
    list
}

pub fn look(tree: &mut Tree) -> Vec<u32> {
    let mut added = Vec::new();
    for _ in 0..LOOK_ROUNDS {
        if tree.is_empty() {
            break;
        }
        let new = grow_tree(tree, &snapshot(tree));
        if new.is_empty() {
            break;
        }
        for pid in &new {
            let handle = native::open(*pid);
            let start = tree.get(pid).map(|s| s.start);
            if native::info(*pid).map(|(_, s)| s) == start {
                if let Some(seen) = tree.get_mut(pid) {
                    seen.handle = handle;
                }
            } else {
                tree.remove(pid);
            }
        }
        added.extend(new.into_iter().filter(|p| tree.contains_key(p)));
    }
    added
}

fn alive(tree: &Tree) -> Vec<u32> {
    tree.iter().filter(|(pid, seen)| native::info(**pid).is_some_and(|(_, start)| start == seen.start)).map(|(pid, _)| *pid).collect()
}

fn send_to(tree: &Tree, pid: u32, sig: i32) {
    let Some(seen) = tree.get(&pid) else { return };
    if let Some(handle) = &seen.handle
        && native::send_handle(handle, sig)
    {
        return;
    }
    if native::info(pid).is_some_and(|(_, start)| start == seen.start)
        && let Ok(pid) = i32::try_from(pid)
    {
        native::send(pid, sig);
    }
}

pub fn end_tree(tree: &mut Tree) {
    look(tree);
    let first = alive(tree);
    if first.is_empty() {
        return;
    }
    first.iter().for_each(|pid| send_to(tree, *pid, SIGTERM));
    let until = Instant::now() + TERM_WAIT;
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
        for pid in look(tree) {
            send_to(tree, pid, SIGTERM);
        }
        if alive(tree).is_empty() {
            return;
        }
    }
    look(tree);
    for pid in alive(tree) {
        send_to(tree, pid, SIGKILL);
    }
}

pub fn stop_all_lists() {
    let groups: Vec<u32> = live_groups().iter().copied().collect();
    let trees: Vec<(u32, Tree)> = live_trees().iter().map(|(g, t)| (*g, t.clone())).collect();
    for group in groups {
        let tree = trees.iter().find(|(g, _)| *g == group).map(|(_, t)| t.clone()).unwrap_or_default();
        kill_group(group, &tree, false);
    }
    for (_, tree) in trees {
        for pid in alive(&tree) {
            send_to(&tree, pid, SIGKILL);
        }
    }
}

static SLOTS: LazyLock<Mutex<HashMap<String, Arc<AtomicUsize>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn slots() -> MutexGuard<'static, HashMap<String, Arc<AtomicUsize>>> {
    SLOTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn take_slot(name: &str) -> Result<Arc<AtomicUsize>> {
    let mut slots = slots();
    if slots.contains_key(name) {
        bail!("a list of {name} is already running");
    }
    let holders = Arc::new(AtomicUsize::new(1));
    slots.insert(name.to_string(), holders.clone());
    Ok(holders)
}

fn leave_slot(name: &str, holders: &Arc<AtomicUsize>) {
    if holders.fetch_sub(1, Ordering::SeqCst) == 1 {
        let mut slots = slots();
        if slots.get(name).is_some_and(|held| Arc::ptr_eq(held, holders)) {
            slots.remove(name);
        }
    }
}

pub fn stuck_readers(name: &str) -> usize {
    slots().get(name).map(|held| held.load(Ordering::SeqCst)).unwrap_or(0)
}

fn reader(from: impl Read + Send + 'static, name: &str, holders: &Arc<AtomicUsize>) -> std::sync::mpsc::Receiver<String> {
    let (send, receive) = std::sync::mpsc::channel();
    holders.fetch_add(1, Ordering::SeqCst);
    let (name, holders) = (name.to_string(), holders.clone());
    std::thread::spawn(move || {
        let text = read_capped(from);
        leave_slot(&name, &holders);
        let _ = send.send(text);
    });
    receive
}

fn collect(from: std::sync::mpsc::Receiver<String>, until: Instant) -> Option<String> {
    from.recv_timeout(until.saturating_duration_since(Instant::now())).ok()
}

pub fn run_list_command(argv: &[&str], timeout: Duration) -> Result<String> {
    run_watched(argv, timeout, true)
}

pub fn run_watched(argv: &[&str], timeout: Duration, watch: bool) -> Result<String> {
    let Some((name, args)) = argv.split_first() else {
        bail!("No command to run.");
    };
    let holders = take_slot(name)?;
    let done = run_in_slot(name, args, timeout, &holders, watch);
    leave_slot(name, &holders);
    done
}

fn run_in_slot(name: &str, args: &[&str], timeout: Duration, holders: &Arc<AtomicUsize>, watch: bool) -> Result<String> {
    let program = find_program(name, &search_dirs()).ok_or_else(|| anyhow!("{name} is not installed"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .process_group(0)
        .spawn()
        .with_context(|| format!("could not start {name}"))?;
    let group = child.id();
    live_groups().insert(group);
    let out = child.stdout.take().map(|s| reader(s, name, holders));
    let err = child.stderr.take().map(|s| reader(s, name, holders));
    let until = Instant::now() + timeout;
    let mut tree = if watch { seed_tree(group) } else { Tree::new() };
    let watch_tree = |tree: &mut Tree| {
        if !tree.is_empty() {
            look(tree);
            live_trees().insert(group, tree.clone());
        }
    };
    let mut leader_held = true;
    let mut reaped = None;
    let ended = loop {
        watch_tree(&mut tree);
        match native::exited(group) {
            Some(true) => break Ok(true),
            Some(false) => {}
            None => match child.try_wait() {
                Ok(Some(status)) => {
                    leader_held = false;
                    reaped = Some(status);
                    break Ok(true);
                }
                Ok(None) => {}
                Err(error) => {
                    leader_held = false;
                    break Err(error);
                }
            },
        }
        if Instant::now() >= until {
            break Ok(false);
        }
        std::thread::sleep(WATCH_EVERY);
    };
    watch_tree(&mut tree);
    kill_group(group, &tree, leader_held);
    let _ = child.kill();
    let waited = child.wait();
    let status = ended.map(|done| done.then(|| reaped.or(waited.ok())).flatten());
    end_tree(&mut tree);
    live_groups().remove(&group);
    live_trees().remove(&group);
    let grace = Instant::now() + OUTPUT_GRACE;
    let stdout = out.map(|r| collect(r, grace));
    let stderr = err.map(|r| collect(r, grace));
    let Some(status) = status.with_context(|| format!("could not watch {name}"))? else {
        bail!("timed out after {} s", timeout.as_secs());
    };
    let (Some(stdout), Some(stderr)) = (stdout.unwrap_or(Some(String::new())), stderr.unwrap_or(Some(String::new()))) else {
        bail!("timed out: a helper of {name} kept its output open");
    };
    if !status.success() {
        let text = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
        if text.is_empty() {
            bail!("{name} stopped ({status})");
        }
        bail!("{}", text.chars().take(300).collect::<String>());
    }
    Ok(stdout)
}

fn saved_result(agent: &str, entry: &CachedModels, stale: bool) -> ListResult {
    ListResult { agent: agent.into(), models: entry.models.clone(), efforts: entry.efforts.clone(), at: entry.at, stale, error: String::new() }
}

pub fn refresh_list(conn: &Connection, agent: &str, now: i64, run: &dyn Fn(&[&str]) -> Result<String>) -> Result<AgentModels> {
    let Some(command) = command_for(agent) else {
        bail!("No model list for agent \"{agent}\".");
    };
    let line = command.join(" ");
    let parsed = run(command).and_then(|out| parse_models(agent, &out));
    match parsed {
        Ok(list) => {
            super::update(conn, |cache| {
                put_models(cache, agent, &list, now);
                Ok(())
            })?;
            Ok(list)
        }
        Err(error) => {
            let text: String = format!("{line} failed: {error:#}").chars().take(400).collect();
            let _ = super::update(conn, |cache| {
                put_model_error(cache, agent, &text, now);
                Ok(())
            });
            Err(anyhow!(text))
        }
    }
}

pub fn model_list(conn: &Connection, agent: &str, refresh: bool, now: i64, run: &dyn Fn(&[&str]) -> Result<String>) -> Result<ListResult> {
    if !LIST_AGENTS.contains(&agent) {
        bail!("No model list for agent \"{agent}\". Use one of: {}.", LIST_AGENTS.join(", "));
    }
    let cache = super::load(conn)?;
    if let Some(fixed) = fixed_for(agent) {
        let entry = cache.agents.get(agent);
        if entry.is_none_or(|e| e.models != fixed.models) {
            super::update(conn, |cache| {
                put_models(cache, agent, &fixed, now);
                Ok(())
            })?;
        }
        let at = entry.filter(|e| e.models == fixed.models).map(|e| e.at).unwrap_or(now);
        return Ok(ListResult { agent: agent.into(), models: fixed.models, efforts: fixed.efforts, at, stale: false, error: String::new() });
    }
    if command_for(agent).is_none() {
        bail!("No model list for agent \"{agent}\".");
    }
    let entry = cache.agents.get(agent);
    let saved = entry.filter(|e| !e.models.is_empty());
    let stale = models_stale(entry, now);
    if let Some(saved) = saved
        && !refresh
    {
        return Ok(saved_result(agent, saved, stale));
    }
    match refresh_list(conn, agent, now, run) {
        Ok(list) => Ok(ListResult { agent: agent.into(), models: list.models, efforts: list.efforts, at: now, stale: false, error: String::new() }),
        Err(error) => match saved {
            Some(saved) => Ok(ListResult { error: error.to_string(), ..saved_result(agent, saved, true) }),
            None => Err(error),
        },
    }
}
