use regex::Regex;
use std::sync::LazyLock;

static HARD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\b(?:git\s+push|push(?:es|ed|ing)?\s+(?:to\s+)?(?:origin|upstream|main|master|the\s+branch|it|the\s+code|changes)|force[- ]?push\w*|deploy\w*|publish\w*|release\s+to|production|prod\s+(?:db|database|server)|reset\s+--hard|git\s+clean\s+-\w*f|rm\s+-\w*r\w*f|rm\s+-\w*f\w*r|drop\s+(?:table|database)|delete\s+(?:the\s+)?(?:repo|repository|branch|database)|branch\s+-D|wipe|destroy)\b|--force(?:-with-lease)?\b)").unwrap()
});

static GIT_VERB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(commit|push|merge|rebase|reset|revert|tag|clean|stash|cherry-pick|discard|deploy|publish|production)\w*\b").unwrap()
});

static RISKY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:git\s+\w+|commit\w*|merge\w*|rebase\w*|revert\w*|stash\w*|cherry-pick\w*|delete\s+(?:the\s+)?files?|rm\s+-|remove\s+(?:the\s+)?(?:files?|folder|directory)|discard\w*|chmod|sudo|curl\s+[^|]*\|\s*(?:sh|bash))\b").unwrap()
});

pub fn hard_stop_text(text: &str) -> bool {
    HARD.is_match(text)
}

pub fn hard_parts(text: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    for found in HARD.find_iter(text) {
        let part = found.as_str().to_lowercase();
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    parts
}

pub fn risky_text(text: &str) -> bool {
    hard_stop_text(text) || RISKY.is_match(text) || !git_verbs(text).is_empty()
}

pub fn git_verbs(text: &str) -> Vec<String> {
    let mut verbs: Vec<String> = Vec::new();
    for found in GIT_VERB.captures_iter(text) {
        let verb = found[1].to_lowercase();
        if !verbs.contains(&verb) {
            verbs.push(verb);
        }
    }
    verbs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_hard_parts() {
        assert_eq!(hard_parts("git push --force origin main && rm -rf ."), vec!["git push", "--force", "rm -rf"]);
        assert!(hard_parts("ls").is_empty());
    }

    #[test]
    fn hard_stops() {
        assert!(hard_stop_text("git push origin main"));
        assert!(hard_stop_text("Deploy the site"));
        assert!(hard_stop_text("run rm -rf build"));
        assert!(!hard_stop_text("run the unit tests"));
    }

    #[test]
    fn risky_and_verbs() {
        assert!(risky_text("git commit -m fix"));
        assert!(!risky_text("read the file"));
        assert_eq!(git_verbs("commit then push, then commit"), vec!["commit", "push"]);
    }
}
