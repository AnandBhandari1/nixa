use regex::Regex;
use std::collections::HashSet;

pub const WAKE_FOLLOW_MS: u64 = 8_000;
pub const WAKE_ASK_MS: u64 = 15_000;
pub const WAKE_CALL_MS: u64 = 15_000;

const STOP: &[&str] = &[
    "stop", "stop it", "stop talking", "stop now", "stop that", "quiet", "be quiet", "shut up", "hush",
    "shush", "enough", "silence", "cancel", "cancel that", "never mind", "nevermind",
];
const CALL_LEAD: &[&str] = &["hey", "hi", "hello", "ok", "okay", "oh", "yo", "so", "uh", "um", "hmm", "and"];
const SOUNDS_LIKE_NAME: &str = r"^n(?:i|y|ee|ea|e)(?:x|ks|cks|kz|ckz|cs)(?:a|ah|ar|er|uh|aa)s?$";

pub const DEFAULT_NAME: &str = "nixa";
pub const NAME_VARIANTS: &[&str] = &["niksa", "nicksa", "neeksa", "nixah", "nikza", "nik sa", "nick sa", "nicks a"];

pub fn normalise(heard: &str) -> String {
    let lower: String = heard.to_lowercase().chars().filter(|c| *c != '\'' && *c != '\u{2019}').collect();
    let mut out = String::with_capacity(lower.len());
    let mut gap = false;
    for c in lower.chars() {
        if c.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

pub fn names_for(assistant: &str) -> Vec<String> {
    let mut names = Vec::new();
    if !assistant.trim().is_empty() && !assistant.trim().eq_ignore_ascii_case(DEFAULT_NAME) {
        names.push(assistant.trim().to_string());
    }
    names.push(DEFAULT_NAME.to_string());
    names.extend(NAME_VARIANTS.iter().map(|s| s.to_string()));
    names
}

pub struct Words {
    prefix: Regex,
    thanks: Regex,
    sounds: Regex,
    list: Vec<String>,
}

impl Words {
    pub fn new(names: &[String]) -> Self {
        let mut alts: Vec<String> = names.iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
        alts.sort_by_key(|n| std::cmp::Reverse(n.len()));
        let alts: Vec<String> = alts.iter().map(|n| regex::escape(n)).collect();
        let body = if alts.is_empty() { "a^".to_string() } else { alts.join("|") };
        let prefix = Regex::new(&format!(r"(?i)^(?:hey |hi |ok |okay )?(?:{body})\b[,.:!?]?\s*")).expect("name prefix");
        Words {
            prefix,
            thanks: Regex::new(r"\b(please|thanks|thank you)\b").expect("thanks"),
            sounds: Regex::new(SOUNDS_LIKE_NAME).expect("sounds"),
            list: names.iter().map(|n| normalise(n)).filter(|n| !n.is_empty()).collect(),
        }
    }

    pub fn strip_name(&self, heard: &str) -> (bool, String) {
        let trimmed = heard.trim();
        match self.prefix.find(trimmed) {
            Some(m) => (true, trimmed[m.end()..].trim().to_string()),
            None => (false, trimmed.to_string()),
        }
    }

    fn bare(&self, heard: &str) -> String {
        let rest = normalise(&self.strip_name(heard).1);
        let cut = self.thanks.replace_all(&rest, "");
        cut.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    pub fn is_stop(&self, heard: &str) -> bool {
        STOP.contains(&self.bare(heard).as_str())
    }

    fn after_lead(heard: &str) -> Vec<String> {
        let mut tokens: Vec<String> = normalise(heard).split(' ').filter(|t| !t.is_empty()).map(String::from).collect();
        while tokens.first().is_some_and(|t| CALL_LEAD.contains(&t.as_str())) {
            tokens.remove(0);
        }
        tokens
    }

    fn name_at(&self, tokens: &[String]) -> usize {
        if tokens.len() >= 2 {
            let two = format!("{} {}", tokens[0], tokens[1]);
            if self.list.contains(&two) || self.sounds.is_match(&format!("{}{}", tokens[0], tokens[1])) {
                return 2;
            }
        }
        if let Some(first) = tokens.first()
            && (self.list.contains(first) || self.sounds.is_match(first))
        {
            return 1;
        }
        0
    }

    pub fn named(&self, heard: &str) -> bool {
        if self.strip_name(heard).0 {
            return true;
        }
        let text = format!(" {} ", normalise(heard));
        if self.list.iter().any(|name| text.contains(&format!(" {name} "))) {
            return true;
        }
        self.name_at(&Self::after_lead(heard)) > 0
    }

    pub fn only_name(&self, heard: &str) -> bool {
        let tokens = Self::after_lead(heard);
        let size = self.name_at(&tokens);
        size > 0 && tokens.len() == size
    }

    pub fn name_words(&self) -> HashSet<String> {
        self.list.iter().filter(|n| !n.contains(' ')).cloned().collect()
    }
}

pub fn wake_open_until(spoke_at: u64, asked_at: u64, called_until: u64) -> u64 {
    let spoke = if spoke_at > 0 { spoke_at + WAKE_FOLLOW_MS } else { 0 };
    let asked = if asked_at > 0 { asked_at + WAKE_ASK_MS } else { 0 };
    called_until.max(spoke).max(asked)
}

pub fn wake_open(now: u64, spoke_at: u64, asked_at: u64, called_until: u64) -> bool {
    now < wake_open_until(spoke_at, asked_at, called_until)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Words {
        Words::new(&names_for("nixa"))
    }

    #[test]
    fn normalise_drops_marks_and_apostrophes() {
        assert_eq!(normalise("  Hey, Nixa!  Don't stop… "), "hey nixa dont stop");
        assert_eq!(normalise("it\u{2019}s"), "its");
    }

    #[test]
    fn strip_name_takes_the_call_off_the_front() {
        let w = words();
        assert_eq!(w.strip_name("Hey Nixa, open the tasks"), (true, "open the tasks".to_string()));
        assert_eq!(w.strip_name("open the tasks"), (false, "open the tasks".to_string()));
        assert!(w.strip_name("nick sa what is next").0);
    }

    #[test]
    fn stop_words() {
        let w = words();
        assert!(w.is_stop("Stop."));
        assert!(w.is_stop("Nixa, stop talking please"));
        assert!(!w.is_stop("stop the build and run tests"));
    }

    #[test]
    fn named_finds_the_name_anywhere_or_sound_alike() {
        let w = words();
        assert!(w.named("ok so nixa what is next"));
        assert!(w.named("Niksa"));
        assert!(w.named("uh neeksa"));
        assert!(!w.named("open the file"));
        assert!(w.only_name("hey nixa"));
        assert!(w.only_name("Nixa?"));
        assert!(!w.only_name("nixa run the tests"));
    }

    #[test]
    fn other_assistant_name_is_heard_too() {
        let w = Words::new(&names_for("Nova"));
        assert!(w.named("nova what is next"));
        assert!(w.named("nixa what is next"));
    }

    #[test]
    fn wake_window() {
        assert!(!wake_open(1000, 0, 0, 0));
        assert!(wake_open(5000, 1000, 0, 0));
        assert!(!wake_open(1000 + WAKE_FOLLOW_MS, 1000, 0, 0));
        assert!(wake_open(10_000, 0, 1000, 0));
        assert!(wake_open(10_000, 0, 0, 10_001));
    }
}
