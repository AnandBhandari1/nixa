use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

pub const SCREEN_LINES: usize = 40;
pub const QUESTION_WORDS: usize = 25;
pub const LOOK_SENTENCES: usize = 8;
pub const QUESTION_SENTENCES: usize = 4;
pub const CONTENT_LINES: usize = 24;
pub const ERROR_LINES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenState {
    Asking,
    Done,
    Error,
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub state: ScreenState,
    pub question: String,
    pub live_wait: bool,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("screen pattern")
}

static FRAME: LazyLock<Regex> = LazyLock::new(|| re("[│┃╭╮╰╯─━┌┐└┘├┤┬┴┼═║▌▐]"));
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)esc to interrupt|context left|\? for shortcuts|shift\+tab to cycle|bypass permissions|ctrl\+b to run|Tip: |Update installed|Restart to update|^\s*[⠀-⣿✻✶✳✢✽·∙●◐◓◑◒◴◵◶◷○◌◎*]\s+\S+…")
});
static BACKGROUND_WAIT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)^(?:[⏺●•◦▪✻✶✳✢✽·∙*]\s*)?Waiting for (?:(?:[1-9]\d*|all|the remaining)\s+)?(?:background agents?|(?:dynamic )?workflows?)(?: to finish)?[.!…]*(?:\s+·.*)?(?:\s+\([^)]*\))?$")
});
static MANY_BLANKS: LazyLock<Regex> = LazyLock::new(|| re(r"\n{3,}"));
static PROMPT: LazyLock<Regex> = LazyLock::new(|| re(r"^[❯›]\s*\S"));
static EMPTY_PROMPT: LazyLock<Regex> = LazyLock::new(|| re(r"^[❯›>]\s*$"));
static TURN_END: LazyLock<Regex> = LazyLock::new(|| re(r"^[✻✶✳✢✽✺*·∙]\s+\S.*\d+(?:\.\d+)?\s*[ms]\b|^(?:─\s*)?Worked for \d"));
static OWN_MARK: LazyLock<Regex> = LazyLock::new(|| re(r"^\[nixa\]"));
static UI: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"^[◤▸⚠⏵⧉]",
        r" · ~/",
        r"(?i)^Ask Codex to do anything",
        r"^Resume this session with",
        r"^claude --resume\b",
        r"\(/focus to show\)",
        r"^⎿\s+Tip:",
        r"^[←→]\s",
    ]
    .iter()
    .map(|p| re(p))
    .collect()
});
static MARKER: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:[-*•⏺●◦▪]|\d+[.)])\s+"));
static MENU: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:❯\s*)?[1-9][.)]\s+\S"));
static STATUS_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"\b(DONE|BLOCKED|NEEDS USER)\W*$"));
static QUESTION_END: LazyLock<Regex> = LazyLock::new(|| re(r#"\?["'”’)\]*]*$"#));
static DOUBLE_QUESTION: LazyLock<Regex> = LazyLock::new(|| re(r"\?\?\W*$"));
static ASK: LazyLock<Regex> = LazyLock::new(|| {
    let parts = [
        r"\bwant me to\b",
        r"\bshould i\b",
        r"\bshall i\b",
        r"\bdo you want\b",
        r"\bwould you like\b",
        r"\bcan i (?:go ahead|proceed|start|continue)\b",
        r"\bok(?:ay)? (?:to|if i) (?:proceed|continue|go ahead|start)\b",
        r"\blet me know (?:which|whether|if you want|how|what|when)\b",
        r"\btell me\b",
        r#"\bsay (?:["'“‘]?[\w ]{1,20}["'”’]? )?(?:when|to|if|and i)\b"#,
        r"\breply with\b",
        r"\b(?:pick|choose) (?:one|an option|which)\b",
        r"\byour call\b",
        r"\bneeds? (?:your|the user['’]?s|the owner['’]?s) (?:ok|okay|yes|approval|decision|go-ahead|answer|input|choice|word)\b",
        r"\b(?:waiting|waits) for (?:you|your|the user|the owner)\b",
        r"\bif you want,? i(?:['’]ll| will| can)\b",
        r"\bif you (?:want|like),? (?:i|we) can\b",
        r"\bi can\b.*\bif you (?:want|like)\b",
    ];
    re(&format!("(?i){}", parts.join("|")))
});
static CONDITIONAL: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)^if (?:anything|something|it|there|this|that|you (?:see|find|notice|cannot|can['’]?t|don['’]?t|hit|get))\b")
});
static EMPTY_OFFER: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(?:anything else|what else|what (?:would|can|should) (?:you|i)|what can i do|need anything|how can i help|let me know if you need)\b")
});
static CHOOSE_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:choose|pick|select|which)\b"));
static ERROR: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)^API Error\b",
        r"(?i)^(?:■\s*)?stream (?:error|disconnected)",
        r"(?i)^(?:■\s*)?(?:Request timed out|Connection (?:closed|error|refused|reset)|ECONNRESET|socket hang up|fetch failed)\b",
        r"^(?:■\s*)?(?:Error|error)(?:\s+\d{3})?:.*\b(?:overloaded|timed? ?out|rate.?limit|connection|network|ECONN\w*|socket|fetch failed|status code|5\d\d|529)\b",
        r"^(?:■\s*)?(?:overloaded_error|Overloaded)\b",
    ]
    .iter()
    .map(|p| re(p))
    .collect()
});
static FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:⏺\s*)?```"));
static STRIP_HEAD: LazyLock<Regex> = LazyLock::new(|| re(r"^⎿\s*"));
static STRIP_MARKS: LazyLock<Regex> = LazyLock::new(|| re(r"[*`_]"));

pub fn clean_screen(text: &str, lines: usize) -> String {
    let kept: Vec<String> = text
        .split('\n')
        .map(|line| FRAME.replace_all(line, " ").trim_end().to_string())
        .filter(|line| !NOISE.is_match(line) || BACKGROUND_WAIT.is_match(line.trim()))
        .collect();
    let joined = MANY_BLANKS.replace_all(&kept.join("\n"), "\n\n").trim().to_string();
    let all: Vec<&str> = joined.split('\n').collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

pub fn strip(text: &str) -> String {
    let text = STRIP_HEAD.replace(text, "");
    let text = MARKER.replace(&text, "");
    STRIP_MARKS.replace_all(&text, "").trim().to_string()
}

fn content_of(screen: &str) -> Vec<String> {
    let cleaned = clean_screen(screen, SCREEN_LINES);
    let lines: Vec<&str> = cleaned.split('\n').map(str::trim).collect();
    let mut out: Vec<String> = Vec::new();
    for line in lines.iter().rev() {
        if out.len() >= CONTENT_LINES {
            break;
        }
        let started = out.iter().any(|l| !l.is_empty());
        if line.is_empty() {
            if started {
                out.push(String::new());
            }
            continue;
        }
        if (TURN_END.is_match(line) && !BACKGROUND_WAIT.is_match(line)) || PROMPT.is_match(line) {
            if started {
                break;
            }
            continue;
        }
        if OWN_MARK.is_match(line) || (!started && (EMPTY_PROMPT.is_match(line) || UI.iter().any(|p| p.is_match(line)))) {
            continue;
        }
        out.push(line.to_string());
    }
    out.reverse();
    out
}

fn outside_fences(lines: &[String]) -> Vec<String> {
    let mut fenced = false;
    let mut out = Vec::new();
    for line in lines {
        if FENCE.is_match(line) {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push(line.clone());
        }
    }
    out
}

pub fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], '.' | '!' | '?') && i + 1 < chars.len() && chars[i + 1].is_whitespace() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let mut k = j;
            if k < chars.len() && matches!(chars[k], '"' | '“' | '\'' | '(') {
                k += 1;
            }
            if k < chars.len() && (chars[k].is_ascii_uppercase() || chars[k].is_ascii_digit()) {
                out.push(chars[start..=i].iter().collect::<String>());
                start = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out.push(chars[start..].iter().collect::<String>());
    out
}

fn ends_with_stop(text: &str) -> bool {
    matches!(text.chars().last(), Some('.' | '!' | '?' | ':'))
}

fn sentences_of(lines: &[String]) -> Vec<String> {
    let mut segments: Vec<String> = Vec::new();
    let mut open = false;
    for line in lines {
        if line.is_empty() {
            open = false;
            continue;
        }
        let join = open && segments.last().is_some_and(|last| !MARKER.is_match(line) && !ends_with_stop(last));
        if join {
            if let Some(last) = segments.last_mut() {
                last.push(' ');
                last.push_str(line);
            }
        } else {
            segments.push(line.clone());
        }
        open = true;
    }
    segments
        .iter()
        .map(|s| strip(s))
        .flat_map(|s| split_sentences(&s))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn words(text: &str, limit: usize) -> String {
    text.split_whitespace().take(limit).collect::<Vec<_>>().join(" ")
}

fn asks(sentence: &str, last: bool) -> bool {
    if EMPTY_OFFER.is_match(sentence) || CONDITIONAL.is_match(sentence) {
        return false;
    }
    (last && QUESTION_END.is_match(sentence) && !DOUBLE_QUESTION.is_match(sentence)) || ASK.is_match(sentence)
}

fn menu_question(lines: &[String]) -> Option<String> {
    let first = lines.iter().position(|l| MENU.is_match(l))?;
    if first < 1 || lines.iter().filter(|l| MENU.is_match(l)).count() < 2 {
        return None;
    }
    let head = lines[..first].iter().rev().find(|l| !l.is_empty())?;
    (QUESTION_END.is_match(head) || CHOOSE_WORD.is_match(head)).then(|| strip(head))
}

fn without_status(sentence: &str) -> usize {
    STATUS_WORD.replace(sentence, "").trim().chars().count()
}

fn question_at(sentences: &[String], index: usize) -> String {
    let sentence = &sentences[index];
    if sentence.ends_with(':') && index + 1 < sentences.len() {
        let next = sentences[index + 1..sentences.len().min(index + 3)].join(" ");
        return words(&format!("{sentence} {next}"), QUESTION_WORDS);
    }
    if STATUS_WORD.is_match(sentence) && without_status(sentence) < 3 && index > 0 {
        return question_at(sentences, index - 1);
    }
    words(sentence, QUESTION_WORDS)
}

pub fn read_screen(screen: &str) -> Option<Reading> {
    let lines = content_of(screen);
    if !lines.iter().any(|l| !l.is_empty()) {
        return None;
    }
    let filled: Vec<&String> = lines.iter().filter(|l| !l.is_empty()).collect();
    let tail = &filled[filled.len().saturating_sub(ERROR_LINES)..];
    let error = tail.iter().map(|l| strip(l)).any(|l| ERROR.iter().any(|p| p.is_match(&l)));
    let sentences = sentences_of(&outside_fences(&lines));
    let last = sentences.last().map(String::as_str).unwrap_or("");
    if BACKGROUND_WAIT.is_match(last) {
        return Some(Reading { state: ScreenState::Waiting, question: String::new(), live_wait: true });
    }
    let look = &sentences[sentences.len().saturating_sub(LOOK_SENTENCES)..];
    let from = look.len().saturating_sub(QUESTION_SENTENCES);
    let at = (0..look.len()).rev().find(|&i| asks(&look[i], i >= from));
    let menu = menu_question(&lines);
    let status = STATUS_WORD.captures(last).map(|c| c[1].to_string());
    let asking = at.is_some() || menu.is_some() || matches!(status.as_deref(), Some("NEEDS USER" | "BLOCKED"));
    if error {
        return if asking { None } else { Some(Reading { state: ScreenState::Error, question: String::new(), live_wait: false }) };
    }
    if !asking {
        return Some(Reading { state: ScreenState::Done, question: String::new(), live_wait: false });
    }
    if status.as_deref() == Some("DONE") {
        return None;
    }
    let question = match (menu, at) {
        (Some(menu), _) => menu,
        (None, Some(at)) => question_at(look, at),
        (None, None) => {
            let kept = look.iter().rfind(|s| !STATUS_WORD.is_match(s) || without_status(s) > 2);
            words(kept.or(look.last()).map(String::as_str).unwrap_or(""), QUESTION_WORDS)
        }
    };
    Some(Reading { state: ScreenState::Asking, question, live_wait: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asking(screen: &str) -> String {
        let reading = read_screen(screen).expect("a reading");
        assert_eq!(reading.state, ScreenState::Asking, "{screen}");
        reading.question
    }

    #[test]
    fn frames_and_noise_go() {
        let text = "╭────╮\n│ hello │\n╰────╯\n esc to interrupt\n\n\n\nbye";
        assert_eq!(clean_screen(text, 40), "hello\n\nbye");
        assert_eq!(clean_screen("a\nb\nc", 2), "b\nc");
    }

    #[test]
    fn finds_a_question_at_the_end() {
        let screen = "I changed the parser.\nAll tests pass.\nShould I also update the docs?\n\n❯ \n";
        assert_eq!(asking(screen), "Should I also update the docs?");
    }

    #[test]
    fn finds_an_ask_phrase() {
        let screen = "Plan is ready. Let me know which option fits best.";
        assert_eq!(asking(screen), "Let me know which option fits best.");
    }

    #[test]
    fn finds_a_menu_question() {
        let screen = "Which database should I use?\n1. SQLite\n2. Postgres\n";
        assert_eq!(asking(screen), "Which database should I use?");
    }

    #[test]
    fn colon_takes_the_next_sentences() {
        let screen = "Two ways to go, your call:\nUse the cache. Or skip it.";
        assert_eq!(asking(screen), "Two ways to go, your call: Use the cache. Or skip it.");
    }

    #[test]
    fn status_words() {
        assert_eq!(asking("I need the key to go on.\nNEEDS USER"), "I need the key to go on.");
        assert_eq!(read_screen("All done here.\nDONE").unwrap().state, ScreenState::Done);
    }

    #[test]
    fn done_error_and_waiting() {
        assert_eq!(read_screen("Fixed the bug and ran the tests.").unwrap().state, ScreenState::Done);
        assert_eq!(read_screen("Working on it\nAPI Error: 529 overloaded").unwrap().state, ScreenState::Error);
        let wait = read_screen("⏺ Waiting for 2 background agents to finish").unwrap();
        assert_eq!(wait.state, ScreenState::Waiting);
        assert!(wait.live_wait);
        assert!(read_screen("\n\n").is_none());
    }

    #[test]
    fn empty_offers_and_conditionals_do_not_ask() {
        assert_eq!(read_screen("Done. Anything else?").unwrap().state, ScreenState::Done);
        assert_eq!(read_screen("If you see an error, tell me.").unwrap().state, ScreenState::Done);
    }

    #[test]
    fn fenced_code_is_skipped() {
        let screen = format!("Here is the code.\n{f}\nwant me to = 1?\n{f}\nIt runs now.", f = "```");
        assert_eq!(read_screen(&screen).unwrap().state, ScreenState::Done);
    }

    #[test]
    fn old_turns_stop_the_read() {
        let screen = "Should I delete it?\n✻ Worked for 3m 2s\n❯ go\nRemoved the file. Tests pass.";
        assert_eq!(read_screen(screen).unwrap().state, ScreenState::Done);
    }

    #[test]
    fn sentences_split_before_capitals() {
        assert_eq!(split_sentences("One. Two? 3 ok. done"), vec!["One.", "Two?", "3 ok. done"]);
        assert_eq!(split_sentences("Say \"Yes\". (Then) go"), vec!["Say \"Yes\".", "(Then) go"]);
    }

    #[test]
    fn long_questions_are_cut() {
        let long = format!("Should I {}?", vec!["really"; 40].join(" "));
        assert_eq!(asking(&long).split_whitespace().count(), QUESTION_WORDS);
    }
}
