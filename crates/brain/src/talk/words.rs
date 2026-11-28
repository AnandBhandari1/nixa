use nixa_voice::{Words, names_for, normalise};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

const HESITATION: &[&str] = &["uh", "um", "uhm", "umm", "er", "erm", "hmm", "hm", "mm", "mmm", "ah", "oh", "eh", "huh"];
const NOISE_MORE: &[&str] = &[
    "well", "so", "like", "you know", "the", "a", "an", "and", "i", "but", "or", "then", "shoot", "oops", "whoa", "wow", "ha",
    "haha", "hehe", "lol",
];
const THANKS: &[&str] = &["thanks", "thank you", "thank you so much", "thank you very much", "thanks a lot", "got it", "i see", "see"];
const ASSENT: &[&str] = &[
    "yes", "yeah", "yep", "yup", "ya", "yah", "yea", "ok", "okay", "alright", "all right", "right", "sure", "fine", "good",
    "great", "perfect", "cool", "nice", "correct", "exactly", "no", "nope", "nah", "uh huh", "mm hmm", "mhm",
];
const ACTION: &[&str] = &[
    "do it", "send it", "go ahead", "go for it", "confirm", "confirm it", "approve", "approve it", "allow", "allow it", "run it",
    "press it",
];
const YES_LEAD: &[&str] = &["yes", "yeah", "yep", "ok", "okay", "sure"];
const NO_WORDS: &[&str] = &["no", "nope", "nah"];
const NO: &[&str] = &["dont", "do not", "deny", "reject", "dont do it", "do not do it", "never mind", "no dont", "no do not"];
const CALL_LEAD: &[&str] = &["hey", "hi", "hello", "ok", "okay", "oh", "yo", "so", "uh", "um", "hmm", "and"];
const FILLER: &[&str] = &["uh", "um", "uhm", "umm", "er", "erm", "hmm", "hm", "ah", "oh", "well", "so", "like", "you know"];
const TYPED_YES: &[&str] = &[
    "yes", "yeah", "yep", "yup", "sure", "ok", "okay", "do it", "go ahead", "yes please", "yes do it", "confirm", "approve", "allow",
    "correct",
];
const TYPED_NO: &[&str] = &[
    "no", "nope", "nah", "dont", "do not", "no thanks", "cancel", "stop", "deny", "reject", "dont do it", "do not do it", "never mind",
];
const YES_START: &[&str] = &["yes", "yeah", "yep", "yup", "sure", "ok", "okay", "alright", "right", "correct"];
const YES_TAIL: &[&str] = &[
    "send it", "do it", "go ahead", "go", "send", "sure", "ok", "okay", "right", "thats right", "that is right", "correct", "yes",
    "yeah", "of course", "fine", "good",
];
const REPEAT_LEAD: &str = r"(?:(?:sorry|pardon|what|huh|hey|ok|okay|so|please|sir)\s+)*";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("talk pattern")
}

static SOUNDS_LIKE_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^m(?:i|y|ee|ea|e|ai)l+(?:o|oh|ow|oe|ou|ook)s?$"));
static APPROVAL: LazyLock<Regex> = LazyLock::new(|| re(r"\b(yes|approve|allow|go ahead|do it|accept|proceed|confirm)\b"));
static POLITE: LazyLock<Regex> = LazyLock::new(|| re(r"\b(please|thanks|thank you)\b"));
static YOU_KNOW: LazyLock<Regex> = LazyLock::new(|| re(r"\byou know\b"));
static ALWAYS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\balways\b"));
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| re(r"[.?!]+"));
static ONE_LETTER: LazyLock<Regex> = LazyLock::new(|| re(r"^\p{L}\.?$"));
static CARD_REPLY: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(?:(?:yes,?\s+)?always|(?:option\s+)?(?:\d+|one|two|three|four|five|\p{L}))\.?$"));
static REPEAT: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        re(&format!(
            r"^{REPEAT_LEAD}(?:i\s+)?(?:cannot|cant|could not|couldnt|did not|didnt|do not|dont|can not)\s+hear\s+(?:you|that|it|what you said|anything)(?:\s+(?:again|properly|well|clearly|now))*$"
        )),
        re(&format!(
            r"^{REPEAT_LEAD}(?:(?:can|could|would|will)\s+you\s+)?(?:please\s+)?(?:say|tell me|tell|repeat)(?:\s+(?:it|that|this|what you said|me))?\s+(?:again|one more time|once more)(?:\s+please)?$"
        )),
        re(&format!(
            r"^{REPEAT_LEAD}(?:(?:can|could|would|will)\s+you\s+)?(?:please\s+)?repeat(?:\s+(?:it|that|this|what you said))?(?:\s+please)?$"
        )),
        re(&format!(
            r"^{REPEAT_LEAD}(?:what did you (?:just )?say|come again|pardon(?: me)?|once more|one more time|again(?: please)?)$"
        )),
    ]
});
static SMALL: LazyLock<Vec<Vec<&'static str>>> = LazyLock::new(|| {
    let mut list: Vec<Vec<&'static str>> =
        HESITATION.iter().chain(NOISE_MORE).chain(THANKS).chain(ASSENT).map(|p| p.split(' ').collect()).collect();
    list.sort_by_key(|words| std::cmp::Reverse(words.len()));
    list
});
static CLEAR_YES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut set: HashSet<String> = ["yes", "yes yes"].iter().map(|s| s.to_string()).collect();
    set.extend(ACTION.iter().map(|s| s.to_string()));
    for lead in YES_LEAD {
        for action in ACTION {
            set.insert(format!("{lead} {action}"));
        }
    }
    set
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmallTalk {
    Noise,
    Thanks,
    Assent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Yes,
    No,
    Unclear,
}

pub struct Talk {
    words: Words,
    list: Vec<String>,
    single: HashSet<String>,
    multi: Vec<Vec<String>>,
}

fn tokens_of(text: &str) -> Vec<String> {
    text.split(' ').filter(|t| !t.is_empty()).map(String::from).collect()
}

fn is_noise(phrase: &str) -> bool {
    HESITATION.contains(&phrase) || NOISE_MORE.contains(&phrase)
}

fn sounds_like_name(text: &str) -> bool {
    SOUNDS_LIKE_NAME.is_match(text)
}

fn drop_lead(tokens: &mut Vec<String>) {
    let lead = tokens.iter().take_while(|t| CALL_LEAD.contains(&t.as_str())).count();
    tokens.drain(..lead);
}

fn split_small(tokens: &[String]) -> Option<Vec<String>> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let hit = SMALL.iter().find(|words| words.iter().enumerate().all(|(k, word)| tokens.get(i + k).is_some_and(|t| t == word)))?;
        found.push(hit.join(" "));
        i += hit.len();
    }
    Some(found)
}

fn drop_multi(tokens: &[String], multi: &[Vec<String>]) -> (Vec<String>, bool) {
    let mut out = Vec::with_capacity(tokens.len());
    let mut removed = false;
    let mut i = 0;
    while i < tokens.len() {
        if let Some(name) = multi.iter().find(|name| name.iter().enumerate().all(|(k, w)| tokens.get(i + k) == Some(w))) {
            i += name.len();
            removed = true;
            continue;
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    (out, removed)
}

pub fn one_letter(text: &str) -> bool {
    ONE_LETTER.is_match(text.trim())
}

impl Talk {
    pub fn new(assistant: &str) -> Talk {
        let names = names_for(assistant);
        let list: Vec<String> = names.iter().map(|n| normalise(n)).filter(|n| !n.is_empty()).collect();
        let single = list.iter().filter(|n| !n.contains(' ')).cloned().collect();
        let mut multi: Vec<Vec<String>> = list.iter().filter(|n| n.contains(' ')).map(|n| tokens_of(n)).collect();
        multi.sort_by_key(|words| std::cmp::Reverse(words.join(" ").len()));
        Talk { words: Words::new(&names), list, single, multi }
    }

    pub fn strip_name(&self, heard: &str) -> (bool, String) {
        self.words.strip_name(heard)
    }

    pub fn is_stop(&self, heard: &str) -> bool {
        self.words.is_stop(heard)
    }

    pub fn named(&self, heard: &str) -> bool {
        self.words.named(heard)
    }

    pub fn only_name(&self, heard: &str) -> bool {
        self.words.only_name(heard)
    }

    fn name_at(&self, tokens: &[String]) -> usize {
        if tokens.len() >= 2 {
            let two = format!("{} {}", tokens[0], tokens[1]);
            if self.list.contains(&two) || sounds_like_name(&format!("{}{}", tokens[0], tokens[1])) {
                return 2;
            }
        }
        match tokens.first() {
            Some(first) if self.list.contains(first) || sounds_like_name(first) => 1,
            _ => 0,
        }
    }

    pub fn without_name(&self, heard: &str) -> Vec<String> {
        let (named, rest) = self.strip_name(heard);
        let (all, removed) = drop_multi(&tokens_of(&normalise(&rest)), &self.multi);
        let mut tokens: Vec<String> = all.iter().filter(|t| !self.single.contains(*t)).cloned().collect();
        let mut found = named || removed || tokens.len() != all.len();
        let lead = tokens.iter().take_while(|t| CALL_LEAD.contains(&t.as_str())).count();
        let size = self.name_at(&tokens[lead..]);
        if size > 0 {
            tokens.drain(..lead + size);
            found = true;
        }
        let end = tokens.len();
        let tail = if end >= 2 && sounds_like_name(&format!("{}{}", tokens[end - 2], tokens[end - 1])) {
            2
        } else if end >= 1 && sounds_like_name(&tokens[end - 1]) {
            1
        } else {
            0
        };
        tokens.truncate(end - tail);
        if found || tail > 0 {
            drop_lead(&mut tokens);
        }
        tokens
    }

    pub fn small_talk(&self, heard: &str) -> Option<SmallTalk> {
        let (named, rest) = self.strip_name(heard);
        if named {
            return None;
        }
        let tokens = tokens_of(&normalise(&rest));
        if tokens.iter().any(|t| self.single.contains(t)) {
            return None;
        }
        if tokens.is_empty() {
            return Some(SmallTalk::Noise);
        }
        if tokens.len() == 1 {
            let mut chars = tokens[0].chars();
            if chars.next().is_some_and(char::is_alphabetic) && chars.next().is_none() {
                return Some(SmallTalk::Noise);
            }
        }
        let found = split_small(&tokens)?;
        let meaning: Vec<&String> = found.iter().filter(|p| !is_noise(p)).collect();
        if meaning.is_empty() {
            return Some(SmallTalk::Noise);
        }
        if meaning.iter().all(|p| THANKS.contains(&p.as_str())) {
            return Some(SmallTalk::Thanks);
        }
        Some(SmallTalk::Assent)
    }

    pub fn confirm_reply(&self, heard: &str) -> Option<Reply> {
        let joined = self.without_name(heard).join(" ");
        let polite = POLITE.replace_all(&joined, " ");
        let mut tokens = tokens_of(&polite);
        let lead = tokens.iter().take_while(|t| HESITATION.contains(&t.as_str())).count();
        tokens.drain(..lead);
        let text = tokens.join(" ");
        if CLEAR_YES.contains(&text) {
            return Some(Reply::Yes);
        }
        let loose: Vec<&str> = tokens.iter().map(String::as_str).filter(|t| !HESITATION.contains(t)).collect();
        let joined_loose = loose.join(" ");
        if NO.contains(&joined_loose.as_str()) || (!loose.is_empty() && loose.iter().all(|t| NO_WORDS.contains(t))) {
            return Some(Reply::No);
        }
        self.small_talk(&text).map(|_| Reply::Unclear)
    }

    fn bare(&self, heard: &str) -> String {
        let rest = normalise(&self.strip_name(heard).1);
        POLITE.replace_all(&rest, "").split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn without_filler(&self, heard: &str) -> String {
        let bare = self.bare(heard);
        YOU_KNOW
            .replace_all(&bare, " ")
            .split(' ')
            .filter(|w| !w.is_empty() && !FILLER.contains(w) && !self.single.contains(*w))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn yes_or_no(&self, heard: &str) -> Option<Reply> {
        let text = self.without_filler(heard);
        if TYPED_YES.contains(&text.as_str()) {
            return Some(Reply::Yes);
        }
        if TYPED_NO.contains(&text.as_str()) {
            return Some(Reply::No);
        }
        let (first, rest) = text.split_once(' ').unwrap_or((text.as_str(), ""));
        (YES_START.contains(&first) && YES_TAIL.contains(&rest)).then_some(Reply::Yes)
    }

    pub fn typed_reply(&self, text: &str) -> Option<Reply> {
        self.yes_or_no(text).or_else(|| self.without_filler(text).is_empty().then_some(Reply::Unclear))
    }

    pub fn prompt_answer_problem(&self, kind: &str, choice: Option<&str>, text: &str, typed: bool) -> Option<&'static str> {
        if typed || choice == Some("no") {
            return None;
        }
        if choice == Some("always") && !ALWAYS.is_match(text) {
            return Some("The user did not say always. Use choice yes only if they said a clear yes.");
        }
        if self.small_talk(text).is_some() {
            return match self.confirm_reply(text) {
                Some(Reply::Yes) => None,
                _ => Some("That was not a clear answer. Ask the user to say yes, or the option number."),
            };
        }
        if matches!(kind, "permission" | "plan") && choice.is_some() && !APPROVAL.is_match(&normalise(text)) {
            return Some("The user did not clearly approve. Ask them to say yes or no.");
        }
        None
    }

    pub fn wants_repeat(&self, heard: &str) -> bool {
        let parts: Vec<String> =
            SENTENCE_END.split(heard).map(|part| self.without_name(part).join(" ")).filter(|p| !p.is_empty()).collect();
        !parts.is_empty() && parts.len() <= 3 && parts.iter().all(|part| REPEAT.iter().any(|p| p.is_match(part)))
    }

    pub fn is_card_reply(&self, text: &str) -> bool {
        matches!(self.confirm_reply(text), Some(Reply::Yes | Reply::No)) || CARD_REPLY.is_match(text.trim())
    }
}
