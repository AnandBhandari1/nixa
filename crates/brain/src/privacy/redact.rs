use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

pub const LONG_STRING: usize = 200;
pub const CONTENT_CHARS: usize = 40;
pub const MAX_DEPTH: usize = 4;

static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"\b(?:sk|rk|pk)-(?:ant-|proj-|or-|svcacct-)?[A-Za-z0-9_-]{16,}", "<key>"),
        (r"\bxai-[A-Za-z0-9_-]{16,}", "<key>"),
        (r"\bAIza[0-9A-Za-z_-]{30,}", "<key>"),
        (r"\bgh[pousr]_[A-Za-z0-9]{20,}", "<key>"),
        (r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}", "<token>"),
        (r"(?i)\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{12,}", "${1} <token>"),
        (r#"(?i)([?&](?:key|token|access_token|refresh_token|api_key|apikey|code|sig|signature)=)[^&\s"']+"#, "${1}<redacted>"),
        (r#"(?i)("(?:[A-Za-z_]*(?:key|token|secret|password)[A-Za-z_]*)"\s*:\s*)"[^"]{6,}""#, "${1}\"<redacted>\""),
        (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", "<email>"),
    ]
    .into_iter()
    .map(|(re, to)| (Regex::new(re).expect("redact pattern"), to))
    .collect()
});

static SECRET_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)key|token|secret|password|auth").expect("secret key"));
static CONTENT_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)typed|text|prompt|input|content|message|note|body|cmd|command").expect("content key"));
static ALWAYS_LENGTH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^typed|prompt$").expect("always length"));

pub struct Who {
    pub home: String,
    pub user: String,
}

pub fn who() -> Who {
    Who { home: std::env::var("HOME").unwrap_or_default(), user: std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default() }
}

fn word_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric())
}

fn replace_word(text: &str, word: &str, to: &str) -> String {
    let Ok(re) = Regex::new(&format!("(?i){}", regex::escape(word))) else { return text.to_string() };
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for found in re.find_iter(text) {
        let before = text[..found.start()].chars().next_back();
        let after = text[found.end()..].chars().next();
        if word_char(before) || word_char(after) {
            continue;
        }
        out.push_str(&text[last..found.start()]);
        out.push_str(to);
        last = found.end();
    }
    out.push_str(&text[last..]);
    out
}

pub fn redact_with(text: &str, who: &Who) -> String {
    let mut out = nixa_store::text::hide_secrets(text, &[]);
    if who.home.chars().count() > 3 {
        let mut homes = vec![who.home.clone()];
        let slashed = who.home.replace('\\', "/");
        if slashed != who.home {
            homes.push(slashed);
        }
        for home in homes {
            if let Ok(re) = Regex::new(&format!("(?i){}", regex::escape(&home))) {
                out = re.replace_all(&out, "~").into_owned();
            }
        }
    }
    for (re, to) in PATTERNS.iter() {
        out = re.replace_all(&out, *to).into_owned();
    }
    if who.user.chars().count() > 2 {
        out = replace_word(&out, &who.user, "<user>");
    }
    out
}

pub fn redact(text: &str) -> String {
    redact_with(text, &who())
}

fn length_of(text: &str) -> String {
    format!("<{} chars>", text.chars().count())
}

fn value_at(value: &Value, depth: usize, who: &Who) -> Value {
    match value {
        Value::String(text) => {
            let cleaned = redact_with(text, who);
            if cleaned.chars().count() > LONG_STRING { Value::String(length_of(&cleaned)) } else { Value::String(cleaned) }
        }
        _ if depth >= MAX_DEPTH => value.clone(),
        Value::Array(list) => Value::Array(list.iter().map(|item| value_at(item, depth + 1, who)).collect()),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, item) in map {
                let next = match item.as_str() {
                    Some(text) if !text.is_empty() => {
                        if SECRET_KEY.is_match(key) {
                            Value::String("<redacted>".into())
                        } else if ALWAYS_LENGTH.is_match(key) || (CONTENT_KEY.is_match(key) && text.chars().count() > CONTENT_CHARS) {
                            Value::String(length_of(text))
                        } else {
                            value_at(item, depth + 1, who)
                        }
                    }
                    _ => value_at(item, depth + 1, who),
                };
                out.insert(key.clone(), next);
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}

pub fn redact_value_with(value: &Value, who: &Who) -> Value {
    value_at(value, 0, who)
}

pub fn redact_value(value: &Value) -> Value {
    redact_value_with(value, &who())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn me() -> Who {
        Who { home: "/Users/someone".into(), user: "someone".into() }
    }

    #[test]
    fn keys_tokens_mail_home_and_user_are_hidden() {
        let key = format!("sk-{}", "a".repeat(24));
        let text = format!("key {key} at /Users/someone/x.txt, mail a.b@c.io, Bearer abcdefghijklmnop, url ?token=xyz&b=1, someone said, someones");
        let out = redact_with(&text, &me());
        assert_eq!(out, "key <key hidden> at ~/x.txt, mail <email>, Bearer <token hidden>, url ?token=<redacted>&b=1, <user> said, someones");
        assert_eq!(redact_with(r#"{"apiKey": "abcdefgh"}"#, &me()), r#"{"apiKey": "<redacted>"}"#);
        assert_eq!(redact_with(&format!("ghp_{}", "B".repeat(22)), &me()), "<key hidden>");
    }

    #[test]
    fn new_secret_shapes_are_hidden() {
        for case in [format!("github_pat_{}", "a".repeat(30)), "AKIAABCDEFGHIJKLMNOP".into(), "xoxp-1234567890-abc".into(), "-----BEGIN PRIVATE KEY-----\nMIIabc\n-----END PRIVATE KEY-----".into(), "https://me:pw1234@host.io/a".into(), "OPENAI_API_KEY=abcdefgh123".into(), "x-api-key: abcdef123456".into(), "http://1.2.3.4:9/k/abcdefgh1234".into()] {
            assert_ne!(redact_with(&case, &me()), case);
            assert!(redact_with(&case, &me()).contains("hidden>"), "{case}");
        }
    }

    #[test]
    fn values_hide_secret_keys_and_long_content() {
        let value = json!({
            "token": "abc",
            "typed": "hi",
            "prompt": "short",
            "text": "x".repeat(41),
            "note": "short note",
            "list": ["/Users/someone/a", "y".repeat(201)],
            "deep": {"a": {"b": {"c": {"d": "/Users/someone"}}}},
            "n": 4
        });
        let out = redact_value_with(&value, &me());
        assert_eq!(out["token"], "<redacted>");
        assert_eq!(out["typed"], "<2 chars>");
        assert_eq!(out["prompt"], "<5 chars>");
        assert_eq!(out["text"], "<41 chars>");
        assert_eq!(out["note"], "short note");
        assert_eq!(out["list"], json!(["~/a", "<201 chars>"]));
        assert_eq!(out["deep"]["a"]["b"]["c"], json!({"d": "/Users/someone"}));
        assert_eq!(out["n"], 4);
    }
}
