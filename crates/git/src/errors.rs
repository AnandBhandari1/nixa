use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

const RULES: [(&str, &str); 12] = [
    ("timeout", r"took longer than \d+ ?s and was stopped|timed out waiting"),
    ("identity", r"Please tell me who you are|empty ident name|unable to auto-detect email|user\.useConfigOnly"),
    ("noUpstream", r"has no upstream branch|no upstream configured|There is no tracking information"),
    ("rejected", r"\[rejected\]|non-fast-forward|fetch first|Updates were rejected|stale info|\(stale info\)"),
    (
        "auth",
        r"Authentication failed|could not read (Username|Password)|terminal prompts disabled|Invalid username or (password|token)|Permission to .* denied to|denied to [\w-]+|returned error: 40[13]|HTTP 40[13]|Bad credentials|Permission denied \(publickey|Write access to repository not granted|requested URL returned error: 403",
    ),
    ("notFound", r"Repository not found|repository '.*' not found|does not appear to be a git repository|returned error: 404"),
    (
        "network",
        r"Could not resolve host|Failed to connect|Connection (timed out|refused|reset)|Operation timed out|unable to access .*: (SSL|LibreSSL|Recv failure|Send failure)|The requested URL returned error: 5\d\d",
    ),
    ("lock", r"index\.lock|Another git process seems to be running|Unable to create '.*\.lock'"),
    (
        "conflict",
        r"CONFLICT \(|Automatic merge failed|could not apply|fix conflicts and then|Resolve all conflicts|you need to resolve your current index first|unmerged files",
    ),
    (
        "dirty",
        r"would be overwritten by (merge|checkout)|Please commit your changes or stash them|cannot (pull|rebase) with rebase: You have unstaged changes|Your local changes to the following files",
    ),
    ("hook", r"hook (declined|failed)|pre-commit hook|commit-msg hook|pre-push hook|husky|lint-staged"),
    ("nothing", r"nothing to commit|no changes added to commit|nothing added to commit"),
];

static COMPILED: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    RULES
        .iter()
        .map(|(kind, pattern)| (*kind, Regex::new(&format!("(?i){pattern}")).expect("error rule")))
        .collect()
});

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("ansi"));

static HINT_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*hint:").expect("hint"));

static PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(error|fatal|remote):\s*").expect("prefix"));

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Words {
    pub kind: String,
    pub title: String,
    pub hint: String,
    pub detail: String,
}

pub fn classify(text: &str) -> &'static str {
    COMPILED
        .iter()
        .find(|(_, rule)| rule.is_match(text))
        .map(|(kind, _)| *kind)
        .unwrap_or("other")
}

pub fn words(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "timeout" => ("Git took too long", "It was stopped. Try again, or run it in a terminal."),
        "identity" => ("No git identity for this repo", "Pick a profile for this repo."),
        "noUpstream" => ("This branch has no remote branch yet", "Push once to publish it."),
        "rejected" => ("The remote has newer commits", "Pull first, or force push with lease."),
        "auth" => ("Git sign-in failed", "Check the account and token for this repo."),
        "notFound" => ("Remote repository not found", "Check the remote URL and that this account can see the repo."),
        "network" => ("Cannot reach the remote", "Check the network and try again."),
        "lock" => ("Another git command is running", "Wait a moment, then try again. If it stays, remove .git/index.lock."),
        "conflict" => ("There are conflicts", "Resolve the conflicted files, then continue."),
        "dirty" => ("Local changes are in the way", "Commit or stash your changes first."),
        "hook" => ("A git hook stopped this", "Fix what the hook says, or run without hooks."),
        "nothing" => ("Nothing to commit", "Stage some changes first."),
        _ => ("Git failed", ""),
    }
}

pub fn detail(text: &str) -> String {
    let clean = ANSI.replace_all(text, "");
    let lines: Vec<String> = clean
        .trim()
        .split('\n')
        .filter(|line| !HINT_LINE.is_match(line))
        .map(|line| PREFIX.replace(line, "").trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    lines[lines.len().saturating_sub(4)..].join("\n")
}

pub fn describe(text: &str) -> Words {
    let kind = classify(text);
    let (title, hint) = words(kind);
    Words { kind: kind.into(), title: title.into(), hint: hint.into(), detail: detail(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_git_errors() {
        assert_eq!(classify("fatal: Authentication failed for 'https://x'"), "auth");
        assert_eq!(classify(" ! [rejected]        main -> main (fetch first)"), "rejected");
        assert_eq!(classify("CONFLICT (content): Merge conflict in a.txt"), "conflict");
        assert_eq!(classify("Unable to create '/r/.git/index.lock': File exists."), "lock");
        assert_eq!(classify("fatal: The current branch x has no upstream branch."), "noUpstream");
        assert_eq!(classify("ssh: Could not resolve host: github.com"), "network");
        assert_eq!(classify("nothing to commit, working tree clean"), "nothing");
        assert_eq!(classify("something odd"), "other");
    }

    #[test]
    fn describe_keeps_last_four_clean_lines() {
        let text = "\x1b[31merror: one\nhint: skip me\nfatal: two\nremote: three\nfour\nfive\n";
        let w = describe(text);
        assert_eq!(w.kind, "other");
        assert_eq!(w.title, "Git failed");
        assert_eq!(w.detail, "two\nthree\nfour\nfive");
    }
}
