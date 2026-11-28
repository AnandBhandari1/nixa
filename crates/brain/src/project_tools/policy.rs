use regex::Regex;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

pub const SECRET_REFUSED: &str = "Secret and environment files are not available through Nixa.";
pub const MAX_HOPS: usize = 40;
pub const PRIVATE_HOME_DIRS: [&str; 17] = [
    ".config", ".codex", ".claude", ".omp", ".kimi-code", ".opencode", ".gemini", ".docker", ".kube", ".cursor", ".local", ".azure", ".oci", ".fly", ".vercel", ".terraform.d", "Library",
];
pub const OPEN_LIBRARY_DIRS: [&str; 2] = ["CloudStorage", "Mobile Documents"];
pub const SENSITIVE_PATTERNS: [&str; 37] = [
    "auth.json", "hosts.yml", "logins.json", "accesstokens.json", "msal_token_cache.json", "local state", ".env", ".env.*", "*.env", ".secret", "*.secrets", "credentials.*", "*.pem", "*.key", "*.p12", "*.pfx", "*.jks", "id_rsa", "id_ed25519", "id_ecdsa", "id_dsa", "id_ecdsa_sk", "id_ed25519_sk", "*.kdbx", ".npmrc", ".pypirc", ".htpasswd", ".pgpass", ".netrc", ".git-credentials", "credentials", "shadow", "gshadow", "pass.txt", "password.txt", "passwords.txt", "*.keychain-db",
];

static SECRET_PART: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\.env(?:\..*|rc)?$|\.env$|^\.(?:zshrc|zshenv|zprofile|zlogin|zlogout|bashrc|bash_profile|bash_login|bash_logout|profile|inputrc|claude\.json(?:\..*)?)$|^\.[\w.-]*history$|^\.histfile$|^(?:\.git|\.ssh|\.aws|\.gnupg|\.npmrc|\.netrc|\.pypirc|\.gitconfig|\.git-credentials|\.dev\.vars|id_rsa|id_dsa|id_ecdsa|id_ed25519)$|^\.?(?:secrets?|credentials?)(?:\..*)?$|\.(?:pem|key|p12|pfx|gpg|asc)$").expect("secret part regex")
});

static SENSITIVE: LazyLock<Vec<Regex>> = LazyLock::new(|| SENSITIVE_PATTERNS.iter().map(|p| glob_regex(p)).collect());

pub fn glob_regex(pattern: &str) -> Regex {
    let mut out = String::from("^");
    for c in pattern.chars() {
        if c == '*' {
            out.push_str(".*");
        } else {
            out.push_str(&regex::escape(&c.to_string()));
        }
    }
    out.push('$');
    Regex::new(&out).expect("glob regex")
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

pub fn private_roots(home: Option<&Path>, data: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |path: PathBuf| {
        if !out.contains(&path) {
            out.push(path);
        }
    };
    if let Some(home) = home {
        let home_real = std::fs::canonicalize(home).ok();
        for dir in PRIVATE_HOME_DIRS {
            add(home.join(dir));
            if let Some(real) = &home_real {
                add(real.join(dir));
            }
            if let Ok(real) = std::fs::canonicalize(home.join(dir)) {
                add(real);
            }
        }
    }
    add(data.to_path_buf());
    if let Ok(real) = std::fs::canonicalize(data) {
        add(real);
    }
    out
}

fn names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name.to_string_lossy().to_string()),
            _ => None,
        })
        .collect()
}

pub fn secret_part(name: &str) -> bool {
    SECRET_PART.is_match(name)
}

pub fn nixa_data_inside(path: &Path) -> bool {
    let parts = names(path);
    parts.windows(2).any(|w| w[0] == ".config" && w[1].eq_ignore_ascii_case("nixa"))
}

fn open_library_part(path: &Path, root: &Path) -> bool {
    root.file_name().is_some_and(|n| n == "Library")
        && path.strip_prefix(root).ok().and_then(|rest| rest.components().next()).is_some_and(|c| OPEN_LIBRARY_DIRS.iter().any(|d| c.as_os_str() == *d))
}

pub fn protected_in(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root) && !open_library_part(path, root)) || nixa_data_inside(path) || names(path).iter().any(|part| secret_part(part))
}

pub fn protected_path(path: &Path) -> bool {
    protected_in(path, &private_roots(home().as_deref(), &nixa_protocol::paths::data_dir()))
}

pub fn sensitive_name(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let alias = name.split(':').next().unwrap_or_default().trim_end_matches([' ', '.']).to_string();
    SENSITIVE.iter().any(|p| p.is_match(&name) || p.is_match(&alias))
}

pub fn link_chain(path: &Path) -> Vec<PathBuf> {
    let mut chain = vec![path.to_path_buf()];
    let mut at = path.to_path_buf();
    for _ in 0..MAX_HOPS {
        let Ok(target) = std::fs::read_link(&at) else { break };
        at = if target.is_absolute() { target } else { at.parent().map(|p| p.join(&target)).unwrap_or(target) };
        chain.push(at.clone());
    }
    if let Ok(real) = std::fs::canonicalize(path) {
        chain.push(real);
    }
    chain
}

pub fn sensitive_file(path: &Path) -> bool {
    link_chain(path).iter().any(|p| sensitive_name(p))
}

pub fn refused(path: &Path) -> bool {
    protected_path(path) || sensitive_file(path)
}

pub fn work_folder_in(path: &Path, home: Option<&Path>, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let real = std::fs::canonicalize(path).map_err(|_| format!("Folder {} does not exist.", path.display()))?;
    let real = real.strip_prefix("/System/Volumes/Data").map(|p| Path::new("/").join(p)).unwrap_or(real);
    let home_real = home.and_then(|h| std::fs::canonicalize(h).ok());
    if real.parent().is_none() || home_real.as_ref().is_some_and(|h| h.starts_with(&real)) {
        return Err("Choose a project folder, not the home folder, a folder that holds it, or the filesystem root.".into());
    }
    if protected_in(&real, roots) {
        return Err(format!("{} is a private folder or Nixa's own data folder. Choose a project folder.", real.display()));
    }
    Ok(real)
}

pub fn work_folder(path: &Path) -> Result<PathBuf, String> {
    let home = home();
    work_folder_in(path, home.as_deref(), &private_roots(home.as_deref(), &nixa_protocol::paths::data_dir()))
}

pub fn project_path_in(project: &str, path: &str, home: Option<&Path>, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let given = Path::new(project);
    if !given.is_absolute() {
        return Err("Choose an absolute project folder.".into());
    }
    let root = std::fs::canonicalize(given).map_err(|e| format!("The folder {project} cannot be used: {e}."))?;
    let home_real = home.and_then(|h| std::fs::canonicalize(h).ok());
    if root.parent().is_none() || home_real.as_ref().is_some_and(|h| h.starts_with(&root)) {
        return Err("Choose a project directory, not the home folder, a folder that holds it, or the filesystem root.".into());
    }
    let wanted = if path.trim().is_empty() { root.clone() } else { root.join(path.trim()) };
    let target = std::fs::canonicalize(&wanted).map_err(|e| format!("{} cannot be used: {e}.", wanted.display()))?;
    if !target.starts_with(&root) {
        return Err("Path is outside the selected project.".into());
    }
    let inside = target.strip_prefix(&root).unwrap_or(Path::new(""));
    if protected_in(&target, roots) || names(inside).iter().any(|p| secret_part(p)) || sensitive_file(&wanted) || sensitive_file(&target) {
        return Err(SECRET_REFUSED.into());
    }
    Ok(target)
}

pub fn project_path(project: &str, path: &str) -> Result<PathBuf, String> {
    let home = home();
    project_path_in(project, path, home.as_deref(), &private_roots(home.as_deref(), &nixa_protocol::paths::data_dir()))
}
