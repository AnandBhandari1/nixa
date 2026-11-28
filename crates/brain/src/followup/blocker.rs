use crate::risk::{hard_stop_text, risky_text};
use regex::Regex;
use std::sync::LazyLock;

pub const BLOCKER_TAIL: usize = 1500;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("blocker pattern")
}

const OWN_DEVICE: &str = r"on (?:your|the user['’]?s|their|his|her|the real|a real) (?:phone|device|iphone|android|ipad|tablet|laptop|mac|pc|computer)|physical(?:ly)?|in person|by hand|manual(?:ly)? (?:test|check)";

static DESTROYS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(?:delete|drop|wipe|truncate|erase|remove)\s+(?:the\s+|all\s+|a\s+|every\s+|this\s+|that\s+)?(?:\w+\s+)?(?:database|db|tables?|data|records?|branch(?:es)?|repos?|repository|folders?|files?|accounts?|users?)\b|\bdestroy|\bforce\b|\birreversible\b|\bcannot be undone\b")
});
static LOGIN: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(?:log(?:ged)?[- ]?in|sign(?:ed)?[- ]?in|passwords?|passcodes?|2fa|mfa|otp|one[- ]time code|verification code|keys?|tokens?|credentials?|secrets?|captcha|access to|permissions? (?:to|on|for)|invite|accounts?)\b")
});
static MONEY_OR_LEGAL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:pay|paid|payment|billing|invoice|purchase|buy|subscription|credit card|contract|legal|licen[cs]es?)\b"));
static MONEY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:pay|paid|payment|billing|invoice|purchase|buy|subscription|credit card|contract)\b"));
static USER_STEP: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"(?i)\b(?:{OWN_DEVICE}|waiting for you|for the user to)\b")));
static DEVICE_STEP: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"(?i)\b(?:{OWN_DEVICE})\b")));
static CHOICE: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(?:only (?:you|the user) can|your (?:own )?(?:decision|choice|call|approval|yes|ok|okay)|the user['’]?s (?:decision|choice|call|approval|yes|ok|okay)|product decision|business decision|policy decision|ask the user|needs? the user to)\b")
});

const DESTROYS_WHY: &str = "it names a step that destroys work or data";
const LOGIN_WHY: &str = "it needs a login, a key or access that only the user can give";
const STEP_WHY: &str = "it needs a step that only the user can do";

fn user_only() -> [(&'static LazyLock<Regex>, &'static str); 5] {
    [
        (&DESTROYS, DESTROYS_WHY),
        (&LOGIN, LOGIN_WHY),
        (&MONEY_OR_LEGAL, "it needs money, a plan or a legal step"),
        (&USER_STEP, STEP_WHY),
        (&CHOICE, "it asks for a choice or a yes that only the user can give"),
    ]
}

fn deep_first() -> [(&'static LazyLock<Regex>, &'static str); 4] {
    [(&DESTROYS, DESTROYS_WHY), (&LOGIN, LOGIN_WHY), (&MONEY, "it needs money or a plan"), (&DEVICE_STEP, STEP_WHY)]
}

pub fn blocker_tail(report: &str) -> String {
    let text = report.trim();
    let count = text.chars().count();
    if count <= BLOCKER_TAIL {
        return text.to_string();
    }
    text.chars().skip(count - BLOCKER_TAIL).collect()
}

pub fn user_blocker(report: &str) -> Option<String> {
    let tail = blocker_tail(report);
    if tail.is_empty() {
        return Some("the report is empty".into());
    }
    if risky_text(&tail) {
        return Some("it names a git write, deploy, publish or production step".into());
    }
    user_only().into_iter().find(|(p, _)| p.is_match(&tail)).map(|(_, why)| why.to_string())
}

pub fn deep_first_blocker(report: &str) -> Option<String> {
    let tail = blocker_tail(report);
    if tail.is_empty() {
        return Some("the report is empty".into());
    }
    if hard_stop_text(&tail) {
        return Some("it names a push, deploy, publish or production step, or one that destroys work or history".into());
    }
    deep_first().into_iter().find(|(p, _)| p.is_match(&tail)).map(|(_, why)| why.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end() {
        let long = format!("{}END", "x".repeat(2000));
        let tail = blocker_tail(&long);
        assert_eq!(tail.chars().count(), BLOCKER_TAIL);
        assert!(tail.ends_with("END"));
        assert_eq!(blocker_tail("  short  "), "short");
    }

    #[test]
    fn user_only_steps() {
        assert_eq!(user_blocker("").as_deref(), Some("the report is empty"));
        assert_eq!(user_blocker("Ready to git push the branch").as_deref(), Some("it names a git write, deploy, publish or production step"));
        assert_eq!(user_blocker("I need the API key for the shop").as_deref(), Some(LOGIN_WHY));
        assert_eq!(user_blocker("Please test it on your phone").as_deref(), Some(STEP_WHY));
        assert_eq!(user_blocker("This needs a paid plan").as_deref(), Some("it needs money, a plan or a legal step"));
        assert_eq!(user_blocker("Which license should we use?").as_deref(), Some("it needs money, a plan or a legal step"));
        assert_eq!(user_blocker("This is a product decision for you").as_deref(), Some("it asks for a choice or a yes that only the user can give"));
        assert_eq!(user_blocker("Should the button be blue or green?"), None);
    }

    #[test]
    fn deep_first_is_narrower() {
        assert_eq!(deep_first_blocker("Which license should we use?"), None);
        assert_eq!(deep_first_blocker("This is a product decision"), None);
        assert_eq!(deep_first_blocker("Waiting for you to say yes"), None);
        assert_eq!(deep_first_blocker("Then deploy to production").as_deref(), Some("it names a push, deploy, publish or production step, or one that destroys work or history"));
        assert_eq!(deep_first_blocker("Should I drop the users table?").as_deref(), Some(DESTROYS_WHY));
        assert_eq!(deep_first_blocker("Check it physically on the device").as_deref(), Some(STEP_WHY));
        assert_eq!(deep_first_blocker("It needs a subscription").as_deref(), Some("it needs money or a plan"));
        assert_eq!(deep_first_blocker("   ").as_deref(), Some("the report is empty"));
    }
}
