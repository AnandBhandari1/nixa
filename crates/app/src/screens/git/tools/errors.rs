#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Timeout,
    Identity,
    NoUpstream,
    Rejected,
    Auth,
    NotFound,
    Network,
    Lock,
    Conflict,
    Dirty,
    Hook,
    Nothing,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Said {
    pub kind: Kind,
    pub title: String,
    pub hint: String,
    pub detail: String,
}

const RULES: [(Kind, &[&str]); 12] = [
    (Kind::Timeout, &["and was stopped", "timed out waiting"]),
    (
        Kind::Identity,
        &[
            "please tell me who you are",
            "empty ident name",
            "unable to auto-detect email",
            "user.useconfigonly",
        ],
    ),
    (
        Kind::NoUpstream,
        &[
            "has no upstream branch",
            "no upstream configured",
            "there is no tracking information",
        ],
    ),
    (
        Kind::Rejected,
        &[
            "[rejected]",
            "non-fast-forward",
            "fetch first",
            "updates were rejected",
            "stale info",
        ],
    ),
    (
        Kind::Auth,
        &[
            "authentication failed",
            "could not read username",
            "could not read password",
            "terminal prompts disabled",
            "invalid username or password",
            "invalid username or token",
            "returned error: 401",
            "returned error: 403",
            "http 401",
            "http 403",
            "bad credentials",
            "permission denied (publickey",
            "write access to repository not granted",
        ],
    ),
    (
        Kind::NotFound,
        &[
            "repository not found",
            "does not appear to be a git repository",
            "returned error: 404",
        ],
    ),
    (
        Kind::Network,
        &[
            "could not resolve host",
            "failed to connect",
            "connection timed out",
            "connection refused",
            "connection reset",
            "operation timed out",
        ],
    ),
    (
        Kind::Lock,
        &["index.lock", "another git process seems to be running"],
    ),
    (
        Kind::Conflict,
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
        Kind::Dirty,
        &[
            "would be overwritten by merge",
            "would be overwritten by checkout",
            "please commit your changes or stash them",
            "you have unstaged changes",
            "your local changes to the following files",
        ],
    ),
    (
        Kind::Hook,
        &[
            "hook declined",
            "hook failed",
            "pre-commit hook",
            "commit-msg hook",
            "pre-push hook",
            "husky",
            "lint-staged",
        ],
    ),
    (
        Kind::Nothing,
        &[
            "nothing to commit",
            "no changes added to commit",
            "nothing added to commit",
        ],
    ),
];

pub fn words(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Timeout => (
            "Git took too long",
            "It was stopped. Try again, or run it in a terminal.",
        ),
        Kind::Identity => (
            "No git identity for this repo",
            "Pick a profile for this repo.",
        ),
        Kind::NoUpstream => (
            "This branch has no remote branch yet",
            "Push once to publish it.",
        ),
        Kind::Rejected => (
            "The remote has newer commits",
            "Pull first, or force push with lease.",
        ),
        Kind::Auth => (
            "Git sign-in failed",
            "Check the account and token for this repo.",
        ),
        Kind::NotFound => (
            "Remote repository not found",
            "Check the remote URL and that this account can see the repo.",
        ),
        Kind::Network => (
            "Cannot reach the remote",
            "Check the network and try again.",
        ),
        Kind::Lock => (
            "Another git command is running",
            "Wait a moment, then try again. If it stays, remove .git/index.lock.",
        ),
        Kind::Conflict => (
            "There are conflicts",
            "Resolve the conflicted files, then continue.",
        ),
        Kind::Dirty => (
            "Local changes are in the way",
            "Commit or stash your changes first.",
        ),
        Kind::Hook => (
            "A git hook stopped this",
            "Fix what the hook says, or run without hooks.",
        ),
        Kind::Nothing => ("Nothing to commit", "Stage some changes first."),
        Kind::Other => ("Git failed", ""),
    }
}

pub fn classify(text: &str) -> Kind {
    let low = text.to_lowercase();
    RULES
        .iter()
        .find(|(_, keys)| keys.iter().any(|k| low.contains(k)))
        .map(|(kind, _)| *kind)
        .unwrap_or(Kind::Other)
}

pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn clean_line(line: &str) -> String {
    let trimmed = line.trim();
    let low = trimmed.to_lowercase();
    for head in ["error:", "fatal:", "remote:"] {
        if low.starts_with(head) {
            return trimmed
                .get(head.len()..)
                .unwrap_or(trimmed)
                .trim()
                .to_string();
        }
    }
    trimmed.to_string()
}

pub fn describe(text: &str) -> Said {
    let kind = classify(text);
    let lines: Vec<String> = strip_ansi(text)
        .lines()
        .filter(|l| !l.trim().to_lowercase().starts_with("hint:"))
        .map(clean_line)
        .filter(|l| !l.is_empty())
        .collect();
    let detail = lines[lines.len().saturating_sub(4)..].join("\n");
    let (title, hint) = words(kind);
    Said {
        kind,
        title: title.into(),
        hint: hint.into(),
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_the_text() {
        assert_eq!(classify("fatal: Authentication failed for x"), Kind::Auth);
        assert_eq!(
            classify("CONFLICT (content): Merge conflict"),
            Kind::Conflict
        );
        assert_eq!(
            classify("git took longer than 20 s and was stopped"),
            Kind::Timeout
        );
        assert_eq!(classify("Please tell me who you are."), Kind::Identity);
        assert_eq!(classify("something odd"), Kind::Other);
    }

    #[test]
    fn detail_keeps_the_last_four_lines() {
        let said =
            describe("hint: skip\nerror: one\nfatal: two\n\u{1b}[31mthree\u{1b}[0m\nfour\nfive");
        assert_eq!(said.detail, "two\nthree\nfour\nfive");
        assert_eq!(said.title, "Git failed");
    }
}
