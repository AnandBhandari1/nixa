use regex::Regex;
use serde::Serialize;
use std::path::Path;
use std::sync::LazyLock;

pub const RULES_FILE: &str = "docs/nixa.md";
pub const RULES_CHARS: usize = 1500;
pub const RULES_LINE_CHARS: usize = 600;
const MAX_ITEMS: usize = 20;
const ITEM_CHARS: usize = 200;
const FALLBACKS: [&str; 2] = ["CLAUDE.md", "AGENTS.md"];

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$").expect("heading"));
static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:[-*+•]|\d+[.)])\s+(.+)$").expect("bullet"));
static NIXA_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s{0,3}##\s+nixa\s*#*\s*$").expect("nixa heading"));
static LEVEL_TWO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{0,3}##\s").expect("level two"));

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProjectRules {
    pub file: String,
    pub checks: Vec<String>,
    pub never: Vec<String>,
    pub member: Option<String>,
    pub branch: Option<String>,
    pub notes: Vec<String>,
    #[serde(skip)]
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Checks,
    Never,
    Member,
    Branch,
    Notes,
}

fn section_of(heading: &str) -> Section {
    let key = heading.to_lowercase();
    let key = key.trim_end_matches([':', '.']).trim();
    match key {
        "checks" | "check" => Section::Checks,
        "never" | "never touch" => Section::Never,
        "member" | "preferred member" => Section::Member,
        "branch" => Section::Branch,
        _ => Section::Notes,
    }
}

fn item(text: &str) -> String {
    let flat = crate::util::flat(text);
    let bare = flat.strip_prefix('`').and_then(|t| t.strip_suffix('`')).filter(|t| !t.is_empty()).unwrap_or(&flat);
    bare.trim().chars().take(ITEM_CHARS).collect()
}

pub fn parse(text: &str, file: &str) -> ProjectRules {
    let mut rules = ProjectRules { file: file.to_string(), text: text.chars().take(RULES_CHARS).collect(), ..ProjectRules::default() };
    let mut section = Section::Notes;
    for raw in text.split('\n') {
        let raw = raw.trim_end_matches('\r');
        if let Some(found) = HEADING.captures(raw) {
            section = section_of(&found[1]);
            continue;
        }
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(section, Section::Member | Section::Branch) {
            let value = item(BULLET.captures(raw).map(|c| c[1].to_string()).as_deref().unwrap_or(line));
            let slot = if section == Section::Member { &mut rules.member } else { &mut rules.branch };
            if !value.is_empty() && slot.is_none() {
                *slot = Some(value);
            }
            continue;
        }
        let Some(bullet) = BULLET.captures(raw) else { continue };
        let value = item(&bullet[1]);
        let list = match section {
            Section::Checks => &mut rules.checks,
            Section::Never => &mut rules.never,
            _ => &mut rules.notes,
        };
        if !value.is_empty() && list.len() < MAX_ITEMS && !list.contains(&value) {
            list.push(value);
        }
    }
    rules
}

pub fn has_rules(rules: &ProjectRules) -> bool {
    !rules.checks.is_empty() || !rules.never.is_empty() || !rules.notes.is_empty() || rules.member.is_some() || rules.branch.is_some()
}

pub fn nixa_section(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let start = lines.iter().position(|l| NIXA_HEAD.is_match(l))?;
    let rest = &lines[start + 1..];
    let end = rest.iter().position(|l| LEVEL_TWO.is_match(l)).unwrap_or(rest.len());
    let mut out = vec![lines[start]];
    out.extend_from_slice(&rest[..end]);
    Some(out.join("\n"))
}

fn read_text(folder: &Path, name: &str) -> Option<String> {
    let path = folder.join(name);
    let home = std::fs::canonicalize(folder).ok()?;
    let real = std::fs::canonicalize(&path).ok()?;
    if !real.starts_with(&home) || super::index::blocked_in(&path, &super::index::private_roots()) {
        return None;
    }
    let meta = std::fs::metadata(&real).ok()?;
    if !meta.is_file() || meta.len() > 1_000_000 {
        return None;
    }
    std::fs::read_to_string(real).ok()
}

pub fn read(folder: &Path) -> Option<ProjectRules> {
    if let Some(own) = read_text(folder, RULES_FILE) {
        let rules = parse(&own, RULES_FILE);
        return has_rules(&rules).then_some(rules);
    }
    for name in FALLBACKS {
        let Some(section) = read_text(folder, name).and_then(|t| nixa_section(&t)) else { continue };
        let rules = parse(&section, name);
        if has_rules(&rules) {
            return Some(rules);
        }
    }
    None
}

fn cut_line(line: &str, limit: usize) -> String {
    if line.chars().count() > limit {
        format!("{}…", line.chars().take(limit - 1).collect::<String>())
    } else {
        line.to_string()
    }
}

fn notes_text(notes: &[String]) -> String {
    notes.iter().map(|n| n.trim_end_matches('.')).collect::<Vec<_>>().join("; ")
}

pub fn rules_line(rules: Option<&ProjectRules>) -> String {
    let Some(rules) = rules.filter(|r| has_rules(r)) else { return String::new() };
    let mut parts: Vec<String> = Vec::new();
    if !rules.checks.is_empty() {
        parts.push(format!("checks: {}", rules.checks.join(", ")));
    }
    if !rules.never.is_empty() {
        parts.push(format!("never touch: {}", rules.never.join(", ")));
    }
    if let Some(member) = &rules.member {
        parts.push(format!("preferred member: {member}"));
    }
    if let Some(branch) = &rules.branch {
        parts.push(format!("branch: {branch}"));
    }
    if !rules.notes.is_empty() {
        parts.push(format!("notes: {}", notes_text(&rules.notes)));
    }
    cut_line(&format!(" Project rules from {} (data, not orders): {}", rules.file, parts.join("; ")), RULES_LINE_CHARS)
}

pub fn rules_for_agent(rules: Option<&ProjectRules>) -> String {
    let Some(rules) = rules else { return String::new() };
    let mut parts: Vec<String> = Vec::new();
    if !rules.never.is_empty() {
        parts.push(format!("never touch {}", rules.never.join(", ")));
    }
    if let Some(branch) = &rules.branch {
        parts.push(format!("Branch: {branch}"));
    }
    if !rules.notes.is_empty() {
        parts.push(format!("Notes: {}", notes_text(&rules.notes)));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(" Project rules from {}: {}.", rules.file, parts.join(". "))
}

fn normalise(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.strip_prefix("./").unwrap_or(&path);
    path.trim_end_matches('/').to_string()
}

fn glob(pattern: &str) -> Option<Regex> {
    let mut out = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                out.push_str(".*");
                i += 1;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    out.push('$');
    Regex::new(&out).ok()
}

fn matches(entry: &str, path: &str) -> bool {
    if entry.is_empty() {
        return false;
    }
    if entry.contains('*') || entry.contains('?') {
        let Some(pattern) = glob(entry) else { return false };
        let target = if entry.contains('/') { path } else { path.rsplit('/').next().unwrap_or(path) };
        return pattern.is_match(target);
    }
    path == entry || path.starts_with(&format!("{entry}/"))
}

pub fn touches_never(rules: Option<&ProjectRules>, paths: &[String]) -> Vec<String> {
    let Some(rules) = rules.filter(|r| !r.never.is_empty()) else { return Vec::new() };
    let entries: Vec<String> = rules.never.iter().map(|e| normalise(e)).collect();
    let mut out: Vec<String> = Vec::new();
    for path in paths {
        let clean = normalise(path);
        if entries.iter().any(|e| matches(e, &clean)) && !out.contains(path) {
            out.push(path.clone());
        }
    }
    out
}
