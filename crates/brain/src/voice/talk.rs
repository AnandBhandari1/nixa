use regex::Regex;
use std::sync::LazyLock;

static SAY_YES_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Say yes or no\.?$").expect("say yes"));
static EMPTY_OFFER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(anything else|what else|what (would|can|should) (you|i)|what can i do|need anything|how can i help|let me know if you need)\b").expect("empty offer")
});
static QUESTION_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\?["')\]]*\s*$"#).expect("question end"));
static ASK_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(want me to|should i|shall i|do you want|would you like|approve|say yes|asks whether|asks if|wants to know)\b").expect("ask words")
});

pub fn asks_user(text: &str) -> bool {
    let clean = text.trim();
    if SAY_YES_END.is_match(clean) || EMPTY_OFFER.is_match(clean) {
        return false;
    }
    QUESTION_END.is_match(clean) || ASK_WORDS.is_match(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_open_the_ask_window() {
        assert!(asks_user("Do you want the tests too?"));
        assert!(asks_user("Should I send it to Claude"));
        assert!(asks_user("Codex asks whether to keep the file."));
        assert!(asks_user("Is that right?\")"));
        assert!(!asks_user("The build is done."));
    }

    #[test]
    fn empty_offers_and_confirms_do_not() {
        assert!(!asks_user("Done. Anything else?"));
        assert!(!asks_user("How can I help?"));
        assert!(!asks_user("Send this to m3? Say yes or no."));
    }
}
