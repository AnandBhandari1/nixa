use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub kind: &'static str,
    pub title: String,
    pub hint: String,
    pub detail: String,
    pub warning: bool,
    pub up_to_date: bool,
    pub files: Vec<String>,
    pub folder: Option<String>,
    pub branch: Option<String>,
    pub stashed: bool,
    pub needs_profile: bool,
    pub count: usize,
}

pub fn success(title: impl Into<String>) -> Outcome {
    Outcome {
        ok: true,
        title: title.into(),
        ..Outcome::default()
    }
}

pub fn refusal(kind: &'static str, title: impl Into<String>, hint: impl Into<String>) -> Outcome {
    Outcome {
        kind,
        title: title.into(),
        hint: hint.into(),
        ..Outcome::default()
    }
}

pub fn failure(text: &str) -> Outcome {
    let (kind, title, hint, detail) = describe(text);
    Outcome {
        kind,
        title: title.to_string(),
        hint: hint.to_string(),
        detail,
        needs_profile: kind == "identity",
        ..Outcome::default()
    }
}

pub fn unchanged(mut o: Outcome) -> Outcome {
    o.hint = format!("{} Nothing was changed.", o.hint).trim().to_string();
    o
}

fn has_any(text: &str, words: &[&str]) -> bool {
    let low = text.to_lowercase();
    words.iter().any(|w| low.contains(&w.to_lowercase()))
}

pub fn classify(text: &str) -> &'static str {
    let rules: [(&'static str, &[&str]); 12] = [
        ("timeout", &["timed out waiting", "and was stopped"]),
        (
            "identity",
            &[
                "please tell me who you are",
                "empty ident name",
                "unable to auto-detect email",
                "user.useconfigonly",
            ],
        ),
        (
            "noUpstream",
            &["has no upstream branch", "no upstream configured", "there is no tracking information"],
        ),
        (
            "rejected",
            &["[rejected]", "non-fast-forward", "fetch first", "updates were rejected", "stale info"],
        ),
        (
            "auth",
            &[
                "authentication failed",
                "could not read username",
                "could not read password",
                "terminal prompts disabled",
                "invalid username or",
                "bad credentials",
                "permission denied (publickey",
                "returned error: 403",
                "returned error: 401",
                "write access to repository not granted",
            ],
        ),
        (
            "notFound",
            &["repository not found", "does not appear to be a git repository", "returned error: 404"],
        ),
        (
            "network",
            &[
                "could not resolve host",
                "failed to connect",
                "connection timed out",
                "connection refused",
                "connection reset",
                "operation timed out",
            ],
        ),
        ("lock", &["index.lock", "another git process seems to be running"]),
        (
            "conflict",
            &[
                "conflict (",
                "automatic merge failed",
                "could not apply",
                "fix conflicts and then",
                "resolve all conflicts",
                "you need to resolve your current index first",
                "unmerged files",
            ],
        ),
        (
            "dirty",
            &[
                "would be overwritten by merge",
                "would be overwritten by checkout",
                "please commit your changes or stash them",
                "you have unstaged changes",
                "your local changes to the following files",
            ],
        ),
        (
            "hook",
            &["hook declined", "hook failed", "pre-commit hook", "commit-msg hook", "pre-push hook", "husky", "lint-staged"],
        ),
        ("nothing", &["nothing to commit", "no changes added to commit", "nothing added to commit"]),
    ];
    rules
        .iter()
        .find(|(_, words)| has_any(text, words))
        .map_or("other", |(kind, _)| kind)
}

pub fn kind_words(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "timeout" => ("Git took too long", "It was stopped. Try again, or run it in a terminal."),
        "identity" => ("No git identity for this repo", "Pick a profile for this repo."),
        "noUpstream" => ("This branch has no remote branch yet", "Push once to publish it."),
        "rejected" => ("The remote has newer commits", "Pull first, or force push with lease."),
        "auth" => ("Git sign-in failed", "Check the account and token for this repo."),
        "notFound" => (
            "Remote repository not found",
            "Check the remote URL and that this account can see the repo.",
        ),
        "network" => ("Cannot reach the remote", "Check the network and try again."),
        "lock" => (
            "Another git command is running",
            "Wait a moment, then try again. If it stays, remove .git/index.lock.",
        ),
        "conflict" => ("There are conflicts", "Resolve the conflicted files, then continue."),
        "dirty" => ("Local changes are in the way", "Commit or stash your changes first."),
        "hook" => ("A git hook stopped this", "Fix what the hook says, or run without hooks."),
        "nothing" => ("Nothing to commit", "Stage some changes first."),
        _ => ("Git failed", ""),
    }
}

fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if ('@'..='~').contains(&n) {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn drop_prefix(line: &str) -> &str {
    let low = line.to_lowercase();
    for p in ["error:", "fatal:", "remote:"] {
        if low.starts_with(p) {
            return line[p.len()..].trim();
        }
    }
    line.trim()
}

pub fn describe(text: &str) -> (&'static str, &'static str, &'static str, String) {
    let kind = classify(text);
    let (title, hint) = kind_words(kind);
    let clean = strip_escapes(text);
    let kept: Vec<&str> = clean
        .trim()
        .lines()
        .filter(|l| !l.trim_start().to_lowercase().starts_with("hint:"))
        .map(drop_prefix)
        .filter(|l| !l.is_empty())
        .collect();
    let detail = kept[kept.len().saturating_sub(4)..].join("\n");
    (kind, title, hint, detail)
}

pub fn branch_name_problem(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        return Some("Type a name.");
    }
    if name.chars().any(char::is_whitespace) {
        return Some("Spaces are not allowed.");
    }
    if name.starts_with('-') {
        return Some("It cannot start with a dash.");
    }
    if name == "@" {
        return Some("A lone @ is not allowed.");
    }
    if name == "HEAD" {
        return Some("HEAD is not allowed.");
    }
    if name
        .chars()
        .any(|c| (c as u32) < 0x20 || c == '\u{7f}' || "~^:?*[\\".contains(c))
    {
        return Some("These signs are not allowed: ~ ^ : ? * [ \\");
    }
    if name.contains("..") {
        return Some("Two dots in a row are not allowed.");
    }
    if name.contains("@{") {
        return Some("@{ is not allowed.");
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Some("A slash must sit between two words.");
    }
    if name.ends_with('.') {
        return Some("It cannot end with a dot.");
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.iter().any(|p| p.starts_with('.')) {
        return Some("A part cannot start with a dot.");
    }
    if parts.iter().any(|p| p.ends_with(".lock")) {
        return Some("A part cannot end with .lock.");
    }
    None
}

pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

pub fn sentence(text: &str) -> String {
    let t = text.trim_end();
    if t.ends_with(['.', '!', '?', '…', ':']) {
        t.to_string()
    } else {
        format!("{t}.")
    }
}

pub fn few_paths(paths: &[String], max: usize) -> String {
    if paths.len() <= max {
        paths.join(", ")
    } else {
        format!("{} and {} more", paths[..max].join(", "), paths.len() - max)
    }
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

pub fn ago_at(seconds: i64, now: i64) -> String {
    if seconds <= 0 {
        return String::new();
    }
    let diff = (now - seconds).max(0);
    match diff {
        d if d < 60 => "just now".into(),
        d if d < 3600 => format!("{} min ago", d / 60),
        d if d < 86400 => format!("{} h ago", d / 3600),
        d if d < 604800 => format!("{} ago", plural((d / 86400) as usize, "day")),
        d if d < 2592000 => format!("{} ago", plural((d / 604800) as usize, "week")),
        d if d < 31536000 => format!("{} ago", plural((d / 2592000) as usize, "month")),
        d => format!("{} ago", plural((d / 31536000) as usize, "year")),
    }
}

pub fn ago(seconds: i64) -> String {
    ago_at(seconds, now_secs())
}

pub fn short_ago_at(seconds: i64, now: i64) -> String {
    let diff = (now - seconds).max(0);
    match diff {
        d if d < 60 => "now".into(),
        d if d < 3600 => format!("{}m ago", d / 60),
        d if d < 86400 => format!("{}h ago", d / 3600),
        d if d < 604800 => format!("{}d ago", d / 86400),
        d if d < 2592000 => format!("{}w ago", d / 604800),
        d if d < 31536000 => format!("{}mo ago", d / 2592000),
        d => format!("{}y ago", d / 31536000),
    }
}

pub fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

pub fn is_full_sha(text: &str) -> bool {
    (40..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

pub fn is_sha_like(text: &str) -> bool {
    (4..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit())
}
