use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

pub const PROTECTED_BRANCHES: [&str; 2] = ["main", "master"];
pub const MESSAGE_CHARS: usize = 2000;
pub const AI_DIRS: [&str; 14] = [".claude", ".cursor", ".codex", ".gemini", ".agents", ".agent", ".omp", ".kimi-code", ".opencode", ".antigravitycli", ".windsurf", ".trail", ".vscode", ".idea"];

static CREDENTIALS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\w+://)[^@\s/]+@").expect("credential regex"));
static CO_AUTHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)^\s*co-authored-by:|generated with\s+\[?claude").expect("co-author regex"));

pub fn is_protected_branch(branch: &str) -> bool {
    let name = branch.trim();
    let name = name.strip_prefix("refs/heads/").unwrap_or(name).to_lowercase();
    PROTECTED_BRANCHES.contains(&name.as_str())
}

pub fn hide_credentials(text: &str) -> String {
    CREDENTIALS.replace_all(text, "$1").to_string()
}

pub fn never_commit(path: &str) -> Option<&'static str> {
    let parts: Vec<&str> = path.split('/').collect();
    let name = parts.last().copied().unwrap_or(path);
    let lower = name.to_lowercase();
    if lower.starts_with(".env") {
        return Some("an env file");
    }
    if crate::project_tools::policy::protected_path(Path::new(path)) || crate::project_tools::policy::sensitive_name(Path::new(path)) {
        return Some("a secret file");
    }
    if parts[..parts.len().saturating_sub(1)].iter().any(|p| AI_DIRS.contains(&p.to_lowercase().as_str())) {
        return Some("AI or editor config");
    }
    if matches!(lower.as_str(), "claude.md" | "agents.md" | "gemini.md") {
        return Some("AI config");
    }
    if lower.ends_with(".md") && lower != "readme.md" {
        return Some("a markdown file (only README.md is committed)");
    }
    None
}

pub fn clean_message(value: Option<&str>) -> Result<String, String> {
    let text = value.unwrap_or_default().replace("\r\n", "\n").trim().to_string();
    if text.is_empty() {
        return Err("Give the commit message.".into());
    }
    if text.chars().count() > MESSAGE_CHARS {
        return Err(format!("The commit message is longer than {MESSAGE_CHARS} characters."));
    }
    if CO_AUTHOR.is_match(&text) {
        return Err("No co-author or generated-with lines in the commit message.".into());
    }
    Ok(text)
}

pub fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

pub fn clip(text: &str, size: usize) -> String {
    crate::util::clip_flat(text, size)
}

pub fn tail(text: &str, size: usize) -> String {
    let hidden = hide_credentials(text);
    let kept: Vec<&str> = hidden.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = kept.len().saturating_sub(6);
    clip(&kept[start..].join(" | "), size)
}

pub fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}
