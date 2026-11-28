use std::collections::HashSet;

pub const OWN_VOICE_KEEP_MS: u64 = 20_000;
pub const OWN_VOICE_LINES: usize = 8;
pub const OWN_VOICE_SHARE: f32 = 0.7;
pub const OWN_VOICE_MIN_WORDS: usize = 3;

fn words_of(text: &str) -> Vec<String> {
    let clean: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c.is_whitespace() || c == '\'' { c } else { ' ' })
        .collect();
    clean.split_whitespace().map(String::from).collect()
}

pub fn real_words(text: &str) -> bool {
    words_of(text).iter().any(|w| w.chars().count() >= 2)
}

#[derive(Default)]
pub struct OwnVoice {
    said: Vec<(HashSet<String>, u64)>,
}

impl OwnVoice {
    fn fresh(&mut self, now: u64) {
        let from = now.saturating_sub(OWN_VOICE_KEEP_MS);
        self.said.retain(|(_, at)| *at >= from);
        let extra = self.said.len().saturating_sub(OWN_VOICE_LINES);
        self.said.drain(..extra);
    }

    pub fn remember(&mut self, text: &str, now: u64) {
        let words: HashSet<String> = words_of(text).into_iter().collect();
        if words.is_empty() {
            return;
        }
        self.said.push((words, now));
        self.fresh(now);
    }

    pub fn is_own(&mut self, heard: &str, now: u64) -> bool {
        self.fresh(now);
        let words = words_of(heard);
        if words.len() < OWN_VOICE_MIN_WORDS || self.said.is_empty() {
            return false;
        }
        let found = words.iter().filter(|w| self.said.iter().any(|(set, _)| set.contains(*w))).count();
        found as f32 / words.len() as f32 >= OWN_VOICE_SHARE
    }

    pub fn is_echo(&mut self, heard: &str, now: u64) -> bool {
        self.fresh(now);
        let words = words_of(heard);
        !words.is_empty() && self.said.iter().any(|(set, _)| words.iter().all(|w| set.contains(w)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hears_its_own_line_back() {
        let mut v = OwnVoice::default();
        v.remember("The build is done and all tests pass.", 1000);
        assert!(v.is_own("build is done and all tests", 2000));
        assert!(!v.is_own("please open the settings window now", 2000));
        assert!(!v.is_own("tests pass", 2000));
    }

    #[test]
    fn a_short_line_is_an_echo_only_when_all_words_were_said() {
        let mut v = OwnVoice::default();
        v.remember("Yes?", 1000);
        assert!(!v.is_own("Yes?", 1500));
        assert!(v.is_echo("yes", 1500));
        assert!(!v.is_echo("no", 1500));
        assert!(!v.is_echo("", 1500));
        assert!(!v.is_echo("yes", 1000 + OWN_VOICE_KEEP_MS + 1));
    }

    #[test]
    fn forgets_after_keep_time() {
        let mut v = OwnVoice::default();
        v.remember("the build is done", 1000);
        assert!(!v.is_own("the build is done", 1000 + OWN_VOICE_KEEP_MS + 1));
    }

    #[test]
    fn real_words_needs_two_letters() {
        assert!(real_words("ok"));
        assert!(!real_words("a . i"));
        assert!(!real_words(""));
    }
}
