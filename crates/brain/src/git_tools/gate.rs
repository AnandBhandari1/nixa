use super::gh_gate::{gh_write, parents};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const GATE_NAME: &str = "gh";
pub const MARK: &str = "nixa gh gate";
pub const DEPTH_ENV: &str = "NIXA_GH_GATE_DEPTH";
pub const LOOP_CODE: i32 = 125;
pub const MISSING_CODE: i32 = 127;
pub const START_FAILED_CODE: i32 = 126;
pub const SCAN_BYTES: usize = 512;
pub const MAX_DEPTH: u32 = 3;

#[derive(Clone, Debug, Default)]
pub struct GateEnv {
    pub args: Vec<String>,
    pub terminal: Option<String>,
    pub depth: Option<String>,
    pub path: Option<String>,
    pub real_gh: Option<String>,
    pub own: Vec<PathBuf>,
    pub gate_dir: Option<PathBuf>,
    pub chain: Vec<String>,
    pub now: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    Refuse(i32, String),
    Run(PathBuf),
}

pub fn gate_dir(data: &Path) -> PathBuf {
    super::identity::git_dir(data).join("bin")
}

pub fn gate_file(data: &Path) -> PathBuf {
    gate_dir(data).join(GATE_NAME)
}

pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

pub const SCRIPT: &str = r##"#!/bin/sh
d=$NIXA_GH_GATE_DEPTH
case "$d" in ''|*[!0-9]*) d=0 ;; esac
if [ "$d" -ge @MAX@ ]; then
  echo "@MARK@: the gate called itself @MAX@ times. Check NIXA_REAL_GH and PATH. Nothing was sent to GitHub." >&2
  exit @LOOP@
fi
if [ -x @BIN@ ]; then
  exec @BIN@ gh "$@"
fi
echo "@MARK@: the nixa command is missing, so gh runs without the gate." >&2
PATH=$(printf '%s' "$PATH" | tr ':' '\n' | grep -vxF @DIR@ | paste -sd: -)
NIXA_GH_GATE_DEPTH=$((d+1))
export PATH NIXA_GH_GATE_DEPTH
exec gh "$@"
"##;

pub fn gate_script(nixa_bin: &Path, gate_dir: &Path) -> Result<String, String> {
    let bin = nixa_bin.display().to_string();
    let dir = gate_dir.display().to_string();
    if !nixa_bin.is_absolute() || bin.contains(['\n', '\r', '\0']) || dir.contains(['\n', '\r', '\0']) {
        return Err(format!("The nixa command path {bin} cannot be used for the {MARK}."));
    }
    Ok(SCRIPT.replace("@MAX@", &MAX_DEPTH.to_string()).replace("@LOOP@", &LOOP_CODE.to_string()).replace("@MARK@", MARK).replace("@BIN@", &quote(&bin)).replace("@DIR@", &quote(&dir)))
}

pub fn write_gate(data: &Path, nixa_bin: &Path) -> Result<PathBuf, String> {
    let dir = gate_dir(data);
    let script = gate_script(nixa_bin, &dir)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    for folder in dir.ancestors().take_while(|a| *a != data && a.starts_with(data)) {
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("cannot set {}: {e}", folder.display()))?;
    }
    let file = dir.join(GATE_NAME);
    let same = std::fs::read_to_string(&file).is_ok_and(|t| t == script) && std::fs::metadata(&file).is_ok_and(|m| m.permissions().mode() & 0o777 == 0o700);
    if same {
        return Ok(file);
    }
    let temp = dir.join(format!(".{GATE_NAME}.{}", std::process::id()));
    std::fs::write(&temp, script).map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
    std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("cannot set {}: {e}", temp.display()))?;
    std::fs::rename(&temp, &file).map_err(|e| format!("cannot place {}: {e}", file.display()))?;
    Ok(file)
}

pub fn ensure(data: &Path) -> String {
    let Some(bin) = nixa_protocol::paths::nixa_bin() else {
        return format!("{MARK}: the nixa command was not found, so agents use the normal gh.");
    };
    match write_gate(data, &bin) {
        Ok(file) => format!("{MARK}: ready at {}", file.display()),
        Err(e) => format!("{MARK}: {e} Agents use the normal gh."),
    }
}

pub fn runs(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn gate_ready(data: &Path) -> bool {
    let dir = gate_dir(data);
    std::fs::metadata(&dir).is_ok_and(|m| m.is_dir() && m.permissions().mode() & 0o100 != 0) && runs(&dir.join(GATE_NAME))
}

pub fn path_with_gate(data: &Path, path: &str) -> (String, Option<String>) {
    let dir = gate_dir(data);
    if !gate_ready(data) {
        return (path.to_string(), Some(format!("{MARK}: {} is missing or cannot run, so agents use the normal gh.", dir.display())));
    }
    let dir_text = dir.display().to_string();
    let rest: Vec<&str> = path.split(':').filter(|p| !p.is_empty() && *p != dir_text).collect();
    let mut out = vec![dir_text.as_str()];
    out.extend(rest);
    (out.join(":"), None)
}

pub fn gated_line(data: &Path, line: &str) -> String {
    let gate = gate_ready(data).then(|| quote(&gate_dir(data).display().to_string()));
    let dirs: Vec<String> = [nixa_protocol::heavy::line_prefix(data), gate].into_iter().flatten().collect();
    if dirs.is_empty() {
        return line.to_string();
    }
    format!("PATH={}:\"$PATH\" {line}", dirs.join(":"))
}

fn id_of(path: &Path) -> Option<(u64, u64)> {
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

pub fn is_gate_script(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    let head = &bytes[..bytes.len().min(SCAN_BYTES)];
    String::from_utf8_lossy(head).contains(MARK)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

pub fn not_gate(file: &Path, own: &[PathBuf], gate_dir: Option<&Path>) -> bool {
    if !runs(file) || is_gate_script(file) {
        return false;
    }
    let Some(found) = id_of(file) else { return false };
    let mut blocked: Vec<(u64, u64)> = own.iter().filter_map(|p| id_of(p)).collect();
    if let Some(dir) = gate_dir {
        if let Some(gate) = id_of(&dir.join(GATE_NAME)) {
            blocked.push(gate);
        }
        if file.parent().is_some_and(|p| same_dir(p, dir)) {
            return false;
        }
    }
    !blocked.contains(&found)
}

pub fn find_real_gh(env: &GateEnv) -> Result<PathBuf, String> {
    let own = env.own.as_slice();
    let dir = env.gate_dir.as_deref();
    if let Some(given) = env.real_gh.as_deref().filter(|g| !g.trim().is_empty()) {
        let path = PathBuf::from(given);
        if !path.is_absolute() || !not_gate(&path, own, dir) {
            return Err(format!("{MARK}: NIXA_REAL_GH ({given}) is not a real gh. Point it to the real gh program, not the gate."));
        }
        return Ok(path);
    }
    let path = env.path.clone().unwrap_or_default();
    std::env::split_paths(&path)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(GATE_NAME))
        .find(|f| not_gate(f, own, dir))
        .ok_or_else(|| format!("{MARK}: the real gh was not found on PATH. Install gh or set NIXA_REAL_GH to its full path."))
}

pub fn depth_of(text: Option<&str>) -> u32 {
    text.and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

pub fn plan(env: &GateEnv) -> Plan {
    if depth_of(env.depth.as_deref()) >= MAX_DEPTH {
        return Plan::Refuse(LOOP_CODE, format!("{MARK}: the gate called itself {MAX_DEPTH} times ({DEPTH_ENV}). Check NIXA_REAL_GH and PATH. Nothing was sent to GitHub."));
    }
    match find_real_gh(env) {
        Ok(path) => Plan::Run(path),
        Err(text) => Plan::Refuse(MISSING_CODE, text),
    }
}

pub fn run_plan(env: &GateEnv) -> (i32, String) {
    match plan(env) {
        Plan::Refuse(code, text) => (code, text),
        Plan::Run(real) => {
            let depth = (depth_of(env.depth.as_deref()) + 1).to_string();
            match Command::new(&real).args(&env.args).env(DEPTH_ENV, depth).status() {
                Ok(status) => (status.code().unwrap_or(1), String::new()),
                Err(e) => (START_FAILED_CODE, format!("{MARK}: gh at {} did not start: {e}", real.display())),
            }
        }
    }
}

pub fn note_auto(data: &Path, write: &str, agent: &str, terminal: Option<&str>, folder: Option<&str>) {
    let Ok(conn) = nixa_store::open(&data.join(nixa_store::DB_FILE)) else { return };
    let now = crate::util::now_ms();
    let input = crate::modes::journal::DecisionInput {
        kind: "auto-yes".into(),
        choice: format!("let {agent} run gh {write}"),
        terminal: terminal.map(str::to_string),
        project: folder.map(str::to_string),
        reason: Some("agents write to GitHub through the gh gate".into()),
        ..Default::default()
    };
    if let Err(e) = crate::modes::journal::ensure(&conn, now).and_then(|()| crate::modes::journal::record(&conn, &input, now)) {
        crate::util::log("gh", &format!("could not record the GitHub write: {e}"));
    }
}

pub fn env_now(args: &[String]) -> GateEnv {
    let data = nixa_protocol::paths::data_dir();
    let folder = std::env::current_dir().ok().map(|d| d.display().to_string());
    let write = gh_write(args);
    let chain = if write.is_some() { parents(std::os::unix::process::parent_id()) } else { Vec::new() };
    if let Some(write) = &write
        && let Some(agent) = super::gh_gate::agent_chain(&chain)
    {
        note_auto(&data, write, &agent, std::env::var("NIXA_TERM_ID").ok().as_deref(), folder.as_deref());
    }
    let own: Vec<PathBuf> = [std::env::current_exe().ok(), nixa_protocol::paths::nixa_bin()].into_iter().flatten().collect();
    GateEnv {
        args: args.to_vec(),
        terminal: std::env::var("NIXA_TERM_ID").ok(),
        depth: std::env::var(DEPTH_ENV).ok(),
        path: std::env::var("PATH").ok(),
        real_gh: std::env::var(super::gh_gate::REAL_GH_ENV).ok(),
        own,
        gate_dir: Some(gate_dir(&data)),
        chain,
        now: crate::util::now_ms(),
    }
}
