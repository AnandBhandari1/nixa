use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

pub const SAME_WORDS: f64 = 0.5;

static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}\p{N}][\p{L}\p{N}._/-]*[\p{L}\p{N}]|[\p{L}\p{N}]").expect("token pattern"));
static NOT_CONTRACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"n['’]t\b").expect("not pattern"));
static COLON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":\s+").expect("colon pattern"));

const FILLER: &[&str] = &[
    "a", "an", "the", "i", "me", "my", "we", "us", "our", "you", "your", "it", "its", "this", "that", "these", "those", "them", "they", "to", "of", "for", "in", "on", "at", "by", "with", "and", "or", "as", "so", "if", "then", "than", "from", "into", "now", "first", "also", "just", "again", "still", "please", "ok", "okay", "go", "ahead", "here", "there", "should", "shall", "can", "could", "would", "will", "may", "might", "must", "want", "wants", "like", "need", "needs", "do", "does", "did", "is", "are", "was", "were", "be", "been", "am", "has", "have", "had", "get", "let", "know", "tell", "what", "which", "how", "whether", "ready", "next", "yes", "y", "n",
];
const NOT: &[&str] = &["not", "no", "never", "without", "dont", "don", "nt"];
const STRONG: &[&str] = &[
    "push", "deploy", "publish", "release", "delete", "remove", "drop", "reset", "revert", "rebase", "merge", "force", "overwrite", "skip", "stop", "cancel", "kill", "undo", "rollback", "wipe", "destroy", "uninstall", "production", "prod",
];
const GENERIC: &[&str] = &["fix", "change", "work", "update", "file", "everything", "all", "rest", "step", "result", "thing"];

fn cut(word: &str, n: usize) -> String {
    let chars: Vec<char> = word.chars().collect();
    chars[..chars.len().saturating_sub(n)].iter().collect()
}

fn consonant(c: char) -> bool {
    c.is_ascii_lowercase() && !"aeiou".contains(c)
}

pub fn stem(word: &str) -> String {
    let mut w = word.to_string();
    let len = |w: &str| w.chars().count();
    if len(&w) > 4 && w.ends_with("ies") {
        w = format!("{}y", cut(&w, 3));
    } else if len(&w) > 4 && ["sses", "xes", "zes", "ches", "shes"].iter().any(|e| w.ends_with(e)) {
        w = cut(&w, 2);
    } else if len(&w) > 3 && w.ends_with('s') && !w.ends_with("ss") {
        w = cut(&w, 1);
    }
    if len(&w) > 5 && w.ends_with("ing") {
        w = cut(&w, 3);
    } else if len(&w) > 4 && w.ends_with("ed") {
        w = cut(&w, 2);
    }
    let chars: Vec<char> = w.chars().collect();
    if chars.len() >= 2 {
        let (a, b) = (chars[chars.len() - 2], chars[chars.len() - 1]);
        if a == b && consonant(b) && !matches!(b, 'l' | 's' | 'z') {
            w = cut(&w, 1);
        }
    }
    if len(&w) > 3 && w.ends_with('e') {
        w = cut(&w, 1);
    }
    w
}

fn split_after_stops(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if matches!(c, '.' | '!' | '?') && chars.peek().is_some_and(|n| n.is_whitespace()) {
            while chars.peek().is_some_and(|n| n.is_whitespace()) {
                chars.next();
            }
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

pub fn ask_part(text: &str) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let sentences = split_after_stops(&clean);
    let sentence = sentences.iter().rev().find(|s| s.contains('?')).or(sentences.last()).cloned().unwrap_or_else(|| clean.clone());
    let after = COLON.split(&sentence).last().unwrap_or(&sentence).to_string();
    if after.split_whitespace().count() >= 2 { after } else { sentence }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskWords {
    pub verb: String,
    pub words: Vec<String>,
    pub strong: Vec<String>,
    pub numbers: Vec<String>,
    pub not: bool,
}

fn unique(list: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    list.into_iter().filter(|w| seen.insert(w.clone())).collect()
}

pub fn ask_words(text: &str) -> AskWords {
    let low = ask_part(text).to_lowercase();
    let raw = NOT_CONTRACTION.replace_all(&low, " not").replace(['\'', '’'], "");
    let tokens: Vec<String> = TOKEN.find_iter(&raw).map(|m| m.as_str().to_string()).collect();
    let digits = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit());
    let mut numbers: Vec<String> = tokens.iter().filter(|t| digits(t)).cloned().collect();
    numbers.sort();
    let not = tokens.iter().any(|t| NOT.contains(&t.as_str()));
    let words: Vec<String> = tokens
        .iter()
        .filter(|t| !FILLER.contains(&t.as_str()) && !NOT.contains(&t.as_str()) && !digits(t))
        .map(|t| if t.contains(['.', '_', '/']) { t.clone() } else { stem(t) })
        .collect();
    let strong_set: Vec<String> = STRONG.iter().map(|w| stem(w)).collect();
    let mut strong = unique(words.iter().filter(|w| strong_set.contains(w)).cloned());
    strong.sort();
    AskWords { verb: words.first().cloned().unwrap_or_default(), words: unique(words), strong, numbers, not }
}

fn overlap(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let shared = a.iter().filter(|w| b.contains(w)).count();
    shared as f64 / (a.len() + b.len() - shared) as f64
}

pub fn ask_score(a: &str, b: &str) -> f64 {
    overlap(&ask_words(a).words, &ask_words(b).words)
}

pub fn same_ask(a: &str, b: &str) -> bool {
    let left = ask_words(a);
    let right = ask_words(b);
    if left.verb.is_empty() || left.verb != right.verb || left.not != right.not {
        return false;
    }
    if left.strong != right.strong || left.numbers != right.numbers {
        return false;
    }
    let generic: Vec<String> = GENERIC.iter().map(|w| stem(w)).collect();
    let objects = |words: &[String]| words.iter().skip(1).filter(|w| !generic.contains(w)).cloned().collect::<Vec<_>>();
    let l = objects(&left.words);
    let r = objects(&right.words);
    if l.is_empty() || r.is_empty() {
        return l.len() + r.len() <= 1;
    }
    overlap(&l, &r) >= SAME_WORDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems() {
        assert_eq!(stem("libraries"), "library");
        assert_eq!(stem("boxes"), "box");
        assert_eq!(stem("tests"), "test");
        assert_eq!(stem("running"), "run");
        assert_eq!(stem("pushed"), "push");
        assert_eq!(stem("make"), "mak");
        assert_eq!(stem("class"), "class");
        assert_eq!(stem("installing"), "install");
    }

    #[test]
    fn ask_part_takes_the_question() {
        assert_eq!(ask_part("I fixed it. Should I run the tests? Then done."), "Should I run the tests?");
        assert_eq!(ask_part("Next step: push the branch"), "push the branch");
        assert_eq!(ask_part("Note: go"), "Note: go");
    }

    #[test]
    fn words_and_flags() {
        let w = ask_words("Should I not delete the 2 old files?");
        assert!(w.not);
        assert_eq!(w.numbers, vec!["2"]);
        assert_eq!(w.verb, "delet");
        assert_eq!(w.strong, vec!["delet"]);
        assert!(ask_words("Don't push").not);
    }

    #[test]
    fn same_asks() {
        assert!(same_ask("Should I run the unit tests now?", "Want me to run the unit tests?"));
        assert!(!same_ask("Should I run the tests?", "Should I not run the tests?"));
        assert!(!same_ask("Should I push the branch?", "Should I run the branch?"));
        assert!(!same_ask("Delete 2 files?", "Delete 3 files?"));
        assert!(!same_ask("", "run"));
        assert!(ask_score("run the tests", "run the tests") > 0.99);
    }
}
