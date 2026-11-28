use crate::id::TermId;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const ENV_ENGINE_SOCKET: &str = "NIXA_ENGINE_SOCKET";
pub const ENV_TERM_ID: &str = "NIXA_TERM_ID";
pub const ENV_PARENT_ID: &str = "NIXA_PARENT_ID";
pub const ENV_DATA_DIR: &str = "NIXA_DATA_DIR";
pub const WINDOW_TOKEN_FILE: &str = "window-token";
pub const WINDOW_TOKEN_HEADER: &str = "x-nixa-window-token";
pub const GATE_TOKEN_HEADER: &str = "x-nixa-gate-token";
pub const GATE_TOKEN_FILE: &str = "gate-token";
pub const BRAIN_TOOL_TOKEN_FILE: &str = "tool-token";
pub const ENV_BRAIN_ADDR: &str = "NIXA_BRAIN_ADDR";
pub const ENV_NIXA_BIN: &str = "NIXA_BIN";
pub const ENV_RELEASE_NONCE: &str = "NIXA_RELEASE_NONCE";
pub const NIXA_BIN_FILE: &str = "nixa";
pub const SOCKET_FILE: &str = "engine.sock";
pub const DEFAULT_BRAIN_ADDR: &str = "127.0.0.1:7421";
pub const TERM_PROGRAM: &str = "nixa";
pub const APP_DATA_NAME: &str = "Nixa";
pub const DEV_DATA_NAME: &str = "Nixa Dev";
pub const HOME_TOOL_DIRS: [&str; 6] = [
    ".local/bin",
    ".opencode/bin",
    ".kimi-code/bin",
    ".cargo/bin",
    ".bun/bin",
    ".npm-global/bin",
];
pub const SYSTEM_TOOL_DIRS: [&str; 4] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];

#[cfg(debug_assertions)]
pub fn dev_root() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().and_then(Path::parent).map(Path::to_path_buf)
}

#[cfg(not(debug_assertions))]
pub fn dev_root() -> Option<PathBuf> {
    None
}

pub fn home_dir() -> Option<PathBuf> {
    env_path("HOME")
}

fn support_dir(name: &str) -> PathBuf {
    home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Library")
        .join("Application Support")
        .join(name)
}

pub fn app_data_dir() -> PathBuf {
    support_dir(APP_DATA_NAME)
}

pub fn dev_data_dir() -> PathBuf {
    support_dir(DEV_DATA_NAME)
}

pub fn default_data_dir() -> PathBuf {
    if dev_root().is_some() { dev_data_dir() } else { app_data_dir() }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn data_dir() -> PathBuf {
    env_path(ENV_DATA_DIR).unwrap_or_else(default_data_dir)
}

pub fn socket_path() -> PathBuf {
    env_path(ENV_ENGINE_SOCKET).unwrap_or_else(|| data_dir().join(SOCKET_FILE))
}

pub fn window_token() -> Option<String> {
    token_in(&data_dir(), WINDOW_TOKEN_FILE)
}

pub fn gate_token() -> Option<String> {
    token_in(&data_dir(), GATE_TOKEN_FILE)
}

pub fn token_in(dir: &Path, file: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(file)).ok()?;
    let token = text.trim();
    (!token.is_empty() && token.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| token.to_string())
}

pub fn brain_addr() -> String {
    std::env::var(ENV_BRAIN_ADDR)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_BRAIN_ADDR.to_string())
}

pub fn current_term_id() -> Option<TermId> {
    std::env::var(ENV_TERM_ID).ok()?.parse().ok()
}

pub fn make_private_dirs(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    make_private_dirs(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

#[cfg(test)]
#[test]
fn private_dirs_ignore_a_loose_umask() {
    let base = std::env::temp_dir().join(format!("nixa-private-{}", std::process::id()));
    let deep = base.join("a").join("b");
    make_private_dirs(&deep).unwrap();
    for dir in [base.join("a"), deep.clone()] {
        assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o077, 0, "{}", dir.display());
    }
    fs::remove_dir_all(&base).unwrap();
}

pub fn ensure_data_dir() -> io::Result<PathBuf> {
    let dir = data_dir();
    ensure_private_dir(&dir)?;
    Ok(dir)
}

pub fn sibling_bin(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let bin = dir.join(NIXA_BIN_FILE);
    if bin.is_file() {
        return Some(bin);
    }
    let launcher = dir.parent()?.join("Resources").join("bin").join(NIXA_BIN_FILE);
    (dir.file_name()? == "MacOS" && launcher.is_file()).then_some(launcher)
}

pub fn app_bundle(path: &Path) -> Option<&Path> {
    let bin = path.parent()?;
    let contents = bin.parent()?;
    let bundle = contents.parent()?;
    (path.is_absolute()
        && bin.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then_some(bundle)
}

pub fn bundled_cli(exe: &Path) -> Option<PathBuf> {
    app_bundle(exe)?;
    sibling_bin(exe).filter(|path| {
        fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
    })
}

pub fn nixa_bin() -> Option<PathBuf> {
    env_path(ENV_NIXA_BIN).or_else(|| sibling_bin(&std::env::current_exe().ok()?))
}

fn prepend_bin_dir(bin_dir: &Path, path: &OsStr) -> Result<OsString, std::env::JoinPathsError> {
    std::env::join_paths(
        std::iter::once(bin_dir.to_path_buf())
            .chain(std::env::split_paths(path).filter(|dir| dir != bin_dir)),
    )
}

pub fn dima_node_bin(home: &Path) -> Option<PathBuf> {
    let nvm = std::env::var_os("NVM_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".nvm"));
    fs::read_dir(nvm.join("versions/node")).ok()?.filter_map(Result::ok)
        .filter_map(|entry| {
            let version = entry.file_name();
            let mut parts = version.to_str()?.strip_prefix('v')?.split('.');
            let version = (parts.next()?.parse::<u32>().ok()?, parts.next()?.parse::<u32>().ok()?, parts.next()?.parse::<u32>().ok()?);
            let bin = entry.path().join("bin");
            ["dima", "node"].iter().all(|name| fs::metadata(bin.join(name)).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0))
                .then_some((version, bin))
        })
        .max().map(|(_, bin)| bin)
}

pub fn tool_path(base: &str, home: Option<&Path>) -> String {
    let mut parts: Vec<String> = base.split(':').filter(|p| !p.is_empty()).map(str::to_string).collect();
    let home_dirs = home
        .into_iter()
        .flat_map(|h| HOME_TOOL_DIRS.iter().map(move |d| h.join(d).display().to_string()));
    let dima = home.and_then(dima_node_bin).and_then(|path| path.into_os_string().into_string().ok());
    for dir in home_dirs.chain(dima).chain(SYSTEM_TOOL_DIRS.iter().map(|d| d.to_string())) {
        if !parts.contains(&dir) {
            parts.push(dir);
        }
    }
    parts.join(":")
}

pub fn tool_path_with_bin(base: &str, home: Option<&Path>, bin_dir: Option<&Path>) -> String {
    let path = tool_path(base, home);
    bin_dir
        .and_then(|dir| prepend_bin_dir(dir, path.as_ref()).ok())
        .and_then(|path| path.into_string().ok())
        .unwrap_or(path)
}

pub const HOST_SESSION_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_ENTRYPOINT_ON_REQUESTS",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_HOST_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_CHILD_NOTES_DIR",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_DESKTOP_APP_VERSION",
    "CLAUDE_CODE_SDK_HAS_HOST_AUTH_REFRESH",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_AGENT_SDK_VERSION",
    "CLAUDE_AGENT_SDK_CLIENT_APP",
    "CLAUDE_AGENT_SDK_MCP_NO_PREFIX",
    "CLAUDE_PID",
];

pub fn term_env(id: &TermId, socket: &Path) -> Vec<(String, String)> {
    let mut env = vec![
        (ENV_TERM_ID.to_string(), id.to_string()),
        (ENV_ENGINE_SOCKET.to_string(), socket.display().to_string()),
        ("TERM_PROGRAM".to_string(), TERM_PROGRAM.to_string()),
    ];
    if let Some(bin) = nixa_bin() {
        env.push((ENV_NIXA_BIN.to_string(), bin.display().to_string()));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dir_is_in_repo() {
        let dir = default_data_dir();
        assert!(dir.ends_with("Library/Application Support/Nixa Dev"));
        assert!(!dir.starts_with(dev_root().unwrap()));
        assert!(dev_root().unwrap().join("Cargo.toml").exists());
        assert!(app_data_dir().ends_with("Library/Application Support/Nixa"));
    }

    #[test]
    fn nixa_bin_is_found_next_to_the_program() {
        let dir = std::env::temp_dir().join(format!("nixa-bin-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("nixa-engine");
        assert_eq!(sibling_bin(&exe), None);
        fs::write(dir.join(NIXA_BIN_FILE), "").unwrap();
        assert_eq!(sibling_bin(&exe), Some(dir.join(NIXA_BIN_FILE)));
        fs::remove_dir_all(&dir).ok();
        let env = term_env(&TermId::from_number(1), Path::new("/s"));
        assert_eq!(env[0], (ENV_TERM_ID.to_string(), "m1".to_string()));
    }

    #[test]
    fn tool_path_adds_agent_folders_once() {
        let path = tool_path("/a::/usr/bin:/h/.local/bin", Some(Path::new("/h")));
        let parts: Vec<&str> = path.split(':').collect();
        assert_eq!(&parts[..3], ["/a", "/usr/bin", "/h/.local/bin"]);
        assert!(parts.contains(&"/h/.opencode/bin"));
        assert!(parts.contains(&"/opt/homebrew/bin"));
        assert_eq!(parts.iter().filter(|p| **p == "/usr/bin").count(), 1);
        assert_eq!(tool_path("", None), SYSTEM_TOOL_DIRS.join(":"));
    }

    #[test]
    fn bundled_bin_stays_first_with_a_terminal_path_override() {
        let bin = Path::new("/Applications/Nixa.app/Contents/MacOS");
        let path = tool_path_with_bin("/custom/bin:/Applications/Nixa.app/Contents/MacOS:/usr/bin", None, Some(bin));
        let entries: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(entries.first().map(PathBuf::as_path), Some(bin));
        assert_eq!(entries.iter().filter(|dir| dir.as_path() == bin).count(), 1);
        assert!(entries.contains(&PathBuf::from("/custom/bin")));
    }
}
