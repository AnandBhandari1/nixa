use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const READ_CHARS: usize = 12_000;
pub const ALLOWED_KEY: &str = "knowledge.skills";
const SKILL_FILE: &str = "SKILL.md";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
}

pub fn good_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-')) && name.len() <= 100
}

fn block_marker(value: &str) -> bool {
    matches!(value, ">" | "|" | ">-" | "|-" | "")
}

fn continued(line: &str) -> bool {
    line.starts_with([' ', '\t']) && !line.trim().is_empty()
}

pub fn front_matter(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return out };
    let lines: Vec<&str> = rest.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let Some(end) = lines.iter().position(|l| *l == "---") else { return out };
    let lines = &lines[..end];
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let Some((key, value)) = line.split_once(':') else {
            i += 1;
            continue;
        };
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphabetic() || c == '_' || c == '-') {
            i += 1;
            continue;
        }
        let mut value = value.trim().to_string();
        if block_marker(&value) {
            let mut parts: Vec<&str> = Vec::new();
            while i + 1 < lines.len() && continued(lines[i + 1]) {
                i += 1;
                parts.push(lines[i].trim());
            }
            value = parts.join(" ");
        } else {
            while i + 1 < lines.len() && continued(lines[i + 1]) {
                i += 1;
                value.push(' ');
                value.push_str(lines[i].trim());
            }
        }
        let value = value.strip_prefix(['"', '\'']).unwrap_or(&value);
        let value = value.strip_suffix(['"', '\'']).unwrap_or(value);
        out.insert(key.to_string(), value.to_string());
        i += 1;
    }
    out
}

pub fn source(dir: &Path, name: &str) -> Option<String> {
    if !good_name(name) {
        return None;
    }
    let folder = dir.join(name);
    let file: PathBuf = folder.join(SKILL_FILE);
    let skills = std::fs::canonicalize(dir).ok()?;
    let own = std::fs::canonicalize(&folder).ok()?;
    let real = std::fs::canonicalize(&file).ok()?;
    let roots: Vec<PathBuf> = super::index::private_roots().into_iter().filter(|r| !skills.starts_with(r)).collect();
    if !real.starts_with(&own) || super::index::blocked_in(&own, &roots) || super::index::blocked_in(&file, &roots) {
        return None;
    }
    std::fs::read_to_string(real).ok()
}

pub fn installed(dir: &Path) -> Vec<String> {
    crate::teams::sources::skills_in(dir).into_iter().filter(|n| good_name(n)).collect()
}

pub fn clean_allowed(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names.iter().map(|n| n.trim()) {
        if good_name(name) && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

pub fn list(dir: &Path, allowed: &[String], query: &str) -> Vec<SkillInfo> {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|w| w.chars().count() > 2).collect();
    let mut found = Vec::new();
    for name in allowed {
        let Some(text) = source(dir, name) else { continue };
        let description = front_matter(&text).remove("description").unwrap_or_default();
        let hay = format!("{name} {description}").to_lowercase();
        if words.is_empty() || words.iter().any(|w| hay.contains(w)) {
            found.push(SkillInfo { name: name.clone(), description });
        }
    }
    found
}

pub fn read(dir: &Path, allowed: &[String], name: &str, assistant: &str) -> String {
    let name = name.trim();
    if !allowed.iter().any(|n| n == name) {
        return format!("{name} is not on {assistant}'s skill list.");
    }
    let Some(text) = source(dir, name) else { return format!("The {name} skill is not installed.") };
    if text.chars().count() > READ_CHARS {
        format!("{}\n…(cut)", text.chars().take(READ_CHARS).collect::<String>())
    } else {
        text
    }
}
