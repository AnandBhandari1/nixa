use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

pub const NO_OWNER: &str = "the owner";
pub const ASSISTANT: &str = "Nixa";
pub const PRIVATE_REPLY: &str = "That is private and confidential, so I will not answer it.";
pub const FOLLOW_UP_MS: u64 = 3 * 60 * 1000;
const FOLLOW_WORDS: usize = 4;
const SHORT_ASK: usize = 3;

const ASK_WORDS: &str = "what|what's|whats|which|who|who's|whom|whose|where|where's|when|how|how's|why";
const AUX_WORDS: &str = "is|are|was|were|does|do|did|has|have|had|can|could|will|would";
const DEVICE: &str = "computer|machine|mac|macbook|laptop|pc|desktop|workstation";
const EMPTY_WORDS: [&str; 30] = [
    "hey", "hi", "hello", "ok", "okay", "so", "and", "also", "but", "then", "please", "well", "now", "just", "um", "uh", "hmm", "listen", "nixa", "thanks", "thank", "you", "yes", "no", "sure", "fine", "right", "alright", "got", "it",
];

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("privacy pattern")
}

static FILLER: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:(?:hey|hi|hello|ok|okay|so|and|also|but|then|please|well|now|just|um|uh|listen)[,!.]?\s+)+"));
static FOLLOW_FILLER: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:(?:hey|hi|hello|ok|okay|so|um|uh|well)[,!.]?\s+)+"));
static REQUEST: LazyLock<Regex> = LazyLock::new(|| {
    re(r"\b(?:tell|show|give) (?:me|us)\b|\blet (?:me|us) know\b|\b(?:i|we) (?:want|would like|'d like) to know\b|\bdo you know\b|\bany idea\b|\b(?:can|could) you (?:say|share)\b|^(?:list|describe|explain|share)\b")
});
static ACTIVITY: LazyLock<Regex> = LazyLock::new(|| {
    re(r"\b(?:doing|did|does all day|up to|working on|work(?:s|ed)? on|busy|been|spend(?:s|ing)?|spent|schedule|routine|calendar|meetings?|plans?|yesterday|today|tonight|tomorrow|this (?:morning|afternoon|evening|week|month)|last (?:night|week|month)|earlier|lately|recently|all day|where (?:is|was)|whereabouts|location|at (?:home|work|his desk)|in (?:the )?office|available|free|online|offline|away|asleep|awake)\b")
});
static TOPIC: LazyLock<Regex> = LazyLock::new(|| {
    re(r"\b(?:apps?|applications?|programs?|software|tools?|agents?|projects?|repos?|repositories|accounts?|e-?mails?|passwords?|logins?|keys?|tokens?|files?|folders?|documents?|docs|notes?|messages?|chats?|contacts?|history|browser|installed|stored|saved|running|open|uses?|using|used|setup|settings?|data)\b")
});
static PRONOUN: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:he|him|his|he's|himself)\b"));
static IT: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:it|its|it's)\b"));
static ASK_START: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^(?:{ASK_WORDS})\b")));
static ASKED: LazyLock<Regex> = LazyLock::new(|| re(r"\?\s*$"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));
static NOT_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[^\p{L}\p{N}']"));
static HAS_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[\p{L}\p{N}]"));
static SEP_AND: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^,\s*(?:and|also|but|plus|then)\s+"));
static SEP_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^\s+(?:and|also|but|plus)\s+"));
static SEP_COMMA: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^,\s+"));
static SPLIT_AFTER_WORD: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"(?i)^(?:also\s+)?(?:{})\b", split_start())));
static SPLIT_AFTER_COMMA: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"(?i)^(?:{})\b", split_start())));
static STOPS: LazyLock<Regex> = LazyLock::new(|| re(r"[?.!,;:]+"));
static FOLLOW_LEAD: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:(?:and|or|also|then|plus) )?(?:(?:what|how) about )?"));
static ASKING_LEAD: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:(?:and|or|also|then|plus) )?(?:what|how) about "));
static TIME_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^(?:(?:and|or|also|then|plus) )?(?:(?:what|how) about )?(?:yesterday|today|tonight|tomorrow|now|right now|earlier|lately|recently|before that|after that|the day before(?: yesterday)?|this (?:morning|afternoon|evening|week|month|weekend)|last (?:night|week|month|weekend))$")
});
static END_MARK: LazyLock<Regex> = LazyLock::new(|| re(r"[.?!;,]$"));

fn split_start() -> String {
    format!("{ASK_WORDS}|{AUX_WORDS}|tell|show|give|list|describe|explain|share")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Activity,
    Private,
}

pub struct Names {
    pub owner: String,
    pub assistant: String,
    pub aliases: Vec<String>,
    named: Regex,
    aux_subject: Regex,
    to_owner: Regex,
    computer: Regex,
    on_computer: Regex,
    vocative_start: Regex,
    vocative_end: Regex,
}

fn with(first: &str, rest: &str) -> String {
    if rest.is_empty() { first.to_string() } else { format!("{rest}|{first}") }
}

pub fn names(owner: &str, assistant: &str, aliases: &[String]) -> Names {
    let named_owner = owner.trim().to_string();
    let assistant = if assistant.trim().is_empty() { ASSISTANT.to_string() } else { assistant.trim().to_string() };
    let aliases: Vec<String> = aliases.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    let all: Vec<String> = std::iter::once(named_owner.clone()).chain(aliases.iter().cloned()).map(|n| regex::escape(&n.to_lowercase())).filter(|n| !n.is_empty()).collect();
    let who = all.iter().flat_map(|n| [format!("{n}'s"), format!("{n}s"), format!(r"mr\.? {n}"), n.clone()]).collect::<Vec<_>>().join("|");
    let owners = with("your (?:boss|owner|master|creator|maker|human)", &who);
    let whose = with("this|that|the|your|his", &all.iter().map(|n| format!("{n}'s|{n}s")).collect::<Vec<_>>().join("|"));
    let called = [assistant.as_str(), named_owner.as_str(), "nixa"].iter().filter(|n| !n.is_empty()).map(|n| regex::escape(&n.to_lowercase())).collect::<Vec<_>>().join("|");
    let owner = if named_owner.is_empty() { NO_OWNER.to_string() } else { named_owner };
    Names {
        named: re(&format!(r"\b(?:{owners})\b")),
        aux_subject: re(&format!(r"^(?:{AUX_WORDS}) (?:you|he|she|they|we|i|it|this|that|there|{owners})\b")),
        to_owner: re(&format!(r"\b(?:tell|ask|remind|message|text|ping|show|send|call|let|inform|update|notify) (?:{})\b", with("him", &who))),
        computer: re(&format!(r"\b(?:{whose}) (?:{DEVICE})\b")),
        on_computer: re(&format!(r"\b(?:on|in|inside) (?:(?:{whose}) (?:{DEVICE})|it)\b")),
        vocative_start: re(&format!(r"^(?:(?:hey|hi|hello|ok|okay)[,!]?\s+)?(?:{called})\s*[,!:.]\s*")),
        vocative_end: re(&format!(r",\s*(?:{called})\s*([?.!]*)$")),
        owner,
        assistant,
        aliases,
    }
}

pub fn only_owner_reply(owner: &str) -> String {
    format!("I cannot answer that. Only {owner} can answer that question.")
}

pub fn reply_of(kind: Kind, owner: &str) -> String {
    match kind {
        Kind::Activity => only_owner_reply(owner),
        Kind::Private => PRIVATE_REPLY.to_string(),
    }
}

pub fn plain(text: &str) -> String {
    let lower = text.to_lowercase().replace(['\u{2019}', '\u{2018}', '\u{60}'], "'");
    SPACES.replace_all(&lower, " ").trim().to_string()
}

fn strip_vocative(text: &str, names: &Names) -> String {
    let first = names.vocative_start.replace(text, "").into_owned();
    names.vocative_end.replace(&first, "${1}").into_owned()
}

pub fn sentences_of(text: &str) -> Vec<String> {
    let flat = SPACES.replace_all(text, " ").trim().to_string();
    let mut out = Vec::new();
    let mut start = 0;
    let mut before: Option<char> = None;
    for (at, c) in flat.char_indices() {
        if c == ' ' && before.is_some_and(|b| ".?!;".contains(b)) {
            out.push(flat[start..at].to_string());
            start = at + 1;
        }
        before = Some(c);
    }
    out.push(flat[start..].to_string());
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

fn separator_at(rest: &str) -> Option<usize> {
    if let Some(found) = SEP_AND.find(rest) {
        return Some(found.end());
    }
    if let Some(found) = SEP_WORD.find(rest)
        && SPLIT_AFTER_WORD.is_match(&rest[found.end()..])
    {
        return Some(found.end());
    }
    if let Some(found) = SEP_COMMA.find(rest)
        && SPLIT_AFTER_COMMA.is_match(&rest[found.end()..])
    {
        return Some(found.end());
    }
    None
}

pub fn clauses_of(sentence: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < sentence.len() {
        match separator_at(&sentence[at..]) {
            Some(length) if length > 0 => {
                parts.push(sentence[start..at].to_string());
                at += length;
                start = at;
            }
            _ => at += sentence[at..].chars().next().map(char::len_utf8).unwrap_or(1),
        }
    }
    parts.push(sentence[start..].to_string());
    parts.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

pub fn ask_parts(text: &str) -> Vec<String> {
    sentences_of(text).iter().flat_map(|s| clauses_of(s)).collect()
}

fn is_ask(part: &str, names: &Names) -> bool {
    let body = FILLER.replace(part, "");
    ASKED.is_match(&body) || ASK_START.is_match(&body) || REQUEST.is_match(&body) || names.aux_subject.is_match(&body)
}

fn words_of(part: &str) -> Vec<String> {
    let text = plain(part);
    FILLER.replace(&text, "").split(' ').filter(|w| HAS_WORD.is_match(w)).map(str::to_string).collect()
}

fn meaningful(part: &str, names: &Names) -> bool {
    let assistant = names.assistant.to_lowercase();
    words_of(part).iter().map(|w| NOT_WORD.replace_all(w, "").into_owned()).any(|w| !w.is_empty() && !EMPTY_WORDS.contains(&w.as_str()) && w != assistant)
}

#[derive(Default)]
struct Seen {
    owner: bool,
    computer: bool,
}

fn judge(part: &str, seen: &mut Seen, names: &Names) -> Option<Kind> {
    let text = strip_vocative(&plain(part), names);
    let about = names.to_owner.replace_all(&text, " ").into_owned();
    let owner = names.named.is_match(&about) || (seen.owner && PRONOUN.is_match(&about));
    let computer = names.computer.is_match(&about) || (seen.computer && IT.is_match(&about));
    if names.named.is_match(&text) {
        seen.owner = true;
    }
    if names.computer.is_match(&text) {
        seen.computer = true;
    }
    if !is_ask(&about, names) {
        return None;
    }
    if owner {
        return Some(if ACTIVITY.is_match(&about) { Kind::Activity } else { Kind::Private });
    }
    if computer && (TOPIC.is_match(&about) || names.on_computer.is_match(&about)) {
        return Some(Kind::Private);
    }
    None
}

fn join_parts(parts: &[String]) -> String {
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| if index + 1 < parts.len() && !END_MARK.is_match(part) { format!("{part},") } else { part.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    pub kind: Option<Kind>,
    pub refused: Vec<String>,
    pub rest: String,
}

pub fn check(text: &str, names: &Names) -> Check {
    let mut seen = Seen::default();
    let mut judged: Vec<(String, Option<Kind>)> = Vec::new();
    for sentence in sentences_of(text) {
        let clauses = clauses_of(&sentence);
        let kinds: Vec<Option<Kind>> = clauses.iter().map(|c| judge(c, &mut seen, names)).collect();
        let found = kinds.iter().flatten().next().copied();
        for (clause, kind) in clauses.into_iter().zip(kinds) {
            let kind = kind.or_else(|| found.filter(|_| is_ask(&plain(&clause), names) && words_of(&clause).len() <= SHORT_ASK));
            judged.push((clause, kind));
        }
    }
    let refused: Vec<String> = judged.iter().filter(|(_, k)| k.is_some()).map(|(c, _)| c.clone()).collect();
    if refused.is_empty() {
        return Check { kind: None, refused, rest: text.trim().to_string() };
    }
    let kind = if judged.iter().any(|(_, k)| *k == Some(Kind::Activity)) { Kind::Activity } else { Kind::Private };
    let kept: Vec<String> = judged.iter().filter(|(c, k)| k.is_none() && meaningful(c, names)).map(|(c, _)| c.clone()).collect();
    Check { kind: Some(kind), refused, rest: join_parts(&kept) }
}

pub fn follow_up_kind(text: &str, last: Kind, names: &Names) -> Option<Kind> {
    let body = FOLLOW_FILLER.replace(&strip_vocative(&plain(text), names), "").into_owned();
    let kind_of = |words: &str| if ACTIVITY.is_match(words) { Kind::Activity } else { last };
    if PRONOUN.is_match(&body) && is_ask(&body, names) {
        return Some(kind_of(&body));
    }
    let bare = SPACES.replace_all(&STOPS.replace_all(&body, " "), " ").trim().to_string();
    if TIME_ONLY.is_match(&bare) {
        return Some(kind_of(&bare));
    }
    let lead = FOLLOW_LEAD.find(&bare).map(|m| m.as_str()).unwrap_or("");
    if lead.is_empty() {
        return None;
    }
    let rest: Vec<&str> = bare[lead.len()..].split(' ').filter(|w| !w.is_empty()).collect();
    if rest.is_empty() || rest.len() > FOLLOW_WORDS {
        return None;
    }
    (ASKED.is_match(&body) || ASKING_LEAD.is_match(&bare)).then(|| kind_of(&bare))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Guarded {
    pub reply: Option<String>,
    pub kind: Option<Kind>,
    pub brain_text: String,
    pub refused: usize,
    pub follow_up: bool,
}

#[derive(Debug, Default)]
pub struct Guard {
    pub last: Option<(u64, Kind)>,
}

impl Guard {
    pub fn guard(&mut self, text: &str, on: bool, now: u64, names: &Names) -> Guarded {
        if !on {
            self.last = None;
            return Guarded { brain_text: text.to_string(), ..Guarded::default() };
        }
        let found = check(text, names);
        if let Some(kind) = found.kind {
            self.last = Some((now, kind));
            return Guarded { reply: Some(reply_of(kind, &names.owner)), kind: Some(kind), brain_text: found.rest, refused: found.refused.len(), follow_up: false };
        }
        let recent = self.last.filter(|(at, _)| now.saturating_sub(*at) <= FOLLOW_UP_MS);
        let kind = recent.and_then(|(_, kind)| follow_up_kind(text, kind, names));
        let Some(kind) = kind else {
            if meaningful(text, names) {
                self.last = None;
            }
            return Guarded { brain_text: text.to_string(), ..Guarded::default() };
        };
        self.last = Some((now, kind));
        Guarded { reply: Some(reply_of(kind, &names.owner)), kind: Some(kind), brain_text: String::new(), refused: 1, follow_up: true }
    }
}

pub fn rest_note(names: &Names, reply: &str) -> String {
    let quoted = serde_json::to_string(reply).unwrap_or_default();
    let owner = &names.owner;
    let assistant = &names.assistant;
    format!("Note: Part of the next line asked about {owner}'s private things. {assistant}'s privacy check already answered it with {quoted}, and that part is left out. Answer only the rest, and say nothing about {owner}'s private things.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n() -> Names {
        names("Sam", "Nixa", &["Lee".to_string()])
    }

    #[test]
    fn splits_sentences_and_clauses() {
        assert_eq!(sentences_of("Hi there.  What is up? ok"), vec!["Hi there.", "What is up?", "ok"]);
        assert_eq!(clauses_of("open the file, and what is Sam doing"), vec!["open the file", "what is Sam doing"]);
        assert_eq!(clauses_of("fix the bug and where is he"), vec!["fix the bug", "where is he"]);
        assert_eq!(clauses_of("salt and pepper"), vec!["salt and pepper"]);
        assert_eq!(clauses_of("first, what is it"), vec!["first", "what is it"]);
        assert_eq!(clauses_of("red, blue"), vec!["red, blue"]);
    }

    #[test]
    fn owner_activity_and_private_asks_are_refused() {
        let found = check("What is Sam doing today?", &n());
        assert_eq!(found.kind, Some(Kind::Activity));
        assert_eq!(found.rest, "");
        let found = check("Which apps are on Sam's laptop?", &n());
        assert_eq!(found.kind, Some(Kind::Private));
        let found = check("What is installed on this computer?", &n());
        assert_eq!(found.kind, Some(Kind::Private));
        let found = check("Who is Mr Lee?", &n());
        assert_eq!(found.kind, Some(Kind::Private));
    }

    #[test]
    fn plain_work_and_messages_to_the_owner_pass() {
        for text in ["Start the build in m3.", "Tell Sam the tests passed.", "What time is it?", "Sam wants the report.", "Nixa, open the tasks window."] {
            let found = check(text, &n());
            assert_eq!(found.kind, None, "{text}");
            assert_eq!(found.rest, text.trim());
        }
    }

    #[test]
    fn only_the_private_part_is_cut_out() {
        let found = check("Open the tasks window, and where is Sam right now?", &n());
        assert_eq!(found.kind, Some(Kind::Activity));
        assert_eq!(found.refused, vec!["where is Sam right now?"]);
        assert_eq!(found.rest, "Open the tasks window");
        let found = check("Where is Sam? Also check m2.", &n());
        assert_eq!(found.rest, "Also check m2.");
        let found = check("What is Sam doing, and why?", &n());
        assert_eq!(found.refused.len(), 2);
        assert_eq!(found.rest, "");
    }

    #[test]
    fn pronouns_follow_the_owner_inside_one_message() {
        let found = check("Sam is great. What is he doing?", &n());
        assert_eq!(found.kind, Some(Kind::Activity));
        assert_eq!(found.rest, "Sam is great.");
    }

    #[test]
    fn follow_ups_are_caught_for_three_minutes() {
        let names = n();
        let mut guard = Guard::default();
        let first = guard.guard("What did Sam do yesterday?", true, 1_000, &names);
        assert_eq!(first.reply.as_deref(), Some("I cannot answer that. Only Sam can answer that question."));
        let next = guard.guard("And today?", true, 2_000, &names);
        assert!(next.follow_up);
        assert_eq!(next.brain_text, "");
        let pronoun = guard.guard("what about his files?", true, 3_000, &names);
        assert!(pronoun.follow_up);
        assert_eq!(pronoun.kind, Some(Kind::Activity));
        let late = guard.guard("And today?", true, 3_000 + FOLLOW_UP_MS + 1, &names);
        assert_eq!(late.reply, None);
        assert_eq!(late.brain_text, "And today?");
    }

    #[test]
    fn a_new_topic_ends_the_follow_up_and_off_lets_all_through() {
        let names = n();
        let mut guard = Guard::default();
        guard.guard("Where is Sam?", true, 0, &names);
        assert_eq!(guard.guard("Run the tests in m4.", true, 10, &names).reply, None);
        assert_eq!(guard.guard("And yesterday?", true, 20, &names).reply, None);
        guard.guard("Where is Sam?", true, 30, &names);
        let off = guard.guard("And yesterday?", false, 40, &names);
        assert_eq!(off.reply, None);
        assert!(guard.last.is_none());
        assert_eq!(guard.guard("Where is Sam?", false, 50, &names).brain_text, "Where is Sam?");
    }

    #[test]
    fn other_owner_names_and_the_rest_note() {
        let other = names("Kai", "Juno", &[]);
        assert_eq!(check("What is Kai doing?", &other).kind, Some(Kind::Activity));
        assert_eq!(check("Who is Lee?", &other).kind, None);
        assert_eq!(check("Juno, what is your owner working on?", &other).kind, Some(Kind::Activity));
        let nobody = names("", "Nixa", &[]);
        assert_eq!(nobody.owner, NO_OWNER);
        assert_eq!(check("What is your owner doing?", &nobody).kind, Some(Kind::Activity));
        assert_eq!(check("Where is the shop?", &nobody).kind, None);
        assert_eq!(check("Tell him the build passed.", &nobody).kind, None);
        assert_eq!(reply_of(Kind::Activity, &nobody.owner), "I cannot answer that. Only the owner can answer that question.");
        let note = rest_note(&n(), PRIVATE_REPLY);
        assert!(note.contains("asked about Sam's private things. Nixa's privacy check"));
        assert!(note.contains("\"That is private and confidential, so I will not answer it.\""));
    }
}
