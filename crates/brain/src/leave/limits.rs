use crate::schedules::zone::{self, Zone};
use regex::Regex;
use std::sync::LazyLock;

pub const LAST_LINES: usize = 12;
pub const LINE_MAX: usize = 400;
pub const BEFORE_LINES: usize = 3;
const DAY_S: i64 = 86_400;

const HIT: [&str; 20] = [
    "usage limit",
    "hit your limit",
    "reached your limit",
    "weekly limit",
    "5-hour limit",
    "5 hour limit",
    "session limit reached",
    "daily limit",
    "rate limit reached",
    "rate limit exceeded",
    "quota exceeded",
    "exceeded your current quota",
    "insufficient_quota",
    "resource_exhausted",
    "exhausted your",
    "out of credits",
    "credit balance is too low",
    "usage cap",
    "quota will reset",
    "limit will reset",
];

const NOT: [&str; 12] = [
    "approaching",
    "% of",
    "% used",
    "less than",
    " left",
    "remaining",
    "will reach",
    "close to",
    "context",
    "nearly",
    "almost",
    "if you hit",
];

const STRONG: [&str; 20] = [
    "you've hit",
    "you have hit",
    "you've reached",
    "you have reached",
    "you exceeded",
    "you've exceeded",
    "you have exceeded",
    "you've exhausted",
    "you have exhausted",
    "limit reached",
    "quota exceeded",
    "rate limit exceeded",
    "insufficient_quota",
    "resource_exhausted",
    "out of credits",
    "credit balance is too low",
    "reset",
    "try again",
    "/upgrade",
    "upgrade to",
];

const TOOL_MARK: &str = "⎿";
const MARKS: [&str; 10] = ["■", "✕", "✗", "×", "⚠️", "⚠", "🖐", "❌", "⛔", "!"];
const BANNERS: [&str; 14] = [
    "you've hit your limit",
    "you've hit your usage limit",
    "you have hit your usage limit",
    "you've reached your usage limit",
    "claude ai usage limit reached",
    "claude usage limit reached",
    "weekly limit reached",
    "5-hour limit reached",
    "5 hour limit reached",
    "session limit reached",
    "usage limit reached for all",
    "you have exhausted your capacity",
    "you are out of credits",
    "credit balance is too low to access the anthropic api",
];
const OWN: [&str; 7] = ["claude", "anthropic", "codex", "chatgpt", "gemini", "antigravity", "opencode"];
const ERROR_HEADS: [&str; 6] = ["[api error", "api error", "error", "stream error", "fatal", "ai_apicallerror"];
const PROSE_HEADS: [&str; 13] = ["⏺", "•", "◦", "- ", "* ", "> ", "+", "#", "$ ", "›", "└", "├", "❯"];
const CODE_BITS: [&str; 14] = ["`", "==", "=>", "->", "::", "fn ", "let ", "const ", "def ", "/*", "assert", "contains(", "grep ", "println"];
const BOX: [char; 8] = ['│', '┃', '║', '╎', '▌', '|', '╭', '╰'];

static FILE_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.[A-Za-z]{1,5}:\d+").expect("file line"));
static LIST_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+[.)]\s").expect("list head"));
static EPOCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\|(\d{10})\b").expect("epoch"));
static SPAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:in|after)\s+((?:\d+\s*(?:days?|d|hours?|hrs?|hr|h|minutes?|mins?|min|m|seconds?|secs?|sec|s)[\s,]*(?:and\s+)?)+)").expect("span")
});
static PART: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\d+)\s*(days?|d|hours?|hrs?|hr|h|minutes?|mins?|min|m|seconds?|secs?|sec|s)").expect("part")
});
static AT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:resets?|reset at|again at|until|after|back at|available at)\b\s*(?:at\s+|on\s+)?(.{1,60})").expect("at")
});
static WEEKDAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(sun|mon|tue|wed|thu|fri|sat)[a-z]*\b").expect("weekday"));
static MONTH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+(\d{1,2})\b").expect("month")
});
static AMPM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,2})(?::(\d{2}))?\s*([ap])\.?m\b").expect("ampm"));
static CLOCK24: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(\d{1,2}):(\d{2})\b").expect("clock"));
static ZONE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([A-Za-z_]+/[A-Za-z_]+(?:/[A-Za-z_]+)?)\)").expect("zone"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub line: String,
    pub until: Option<u64>,
    pub before: String,
}

fn strip_box(line: &str) -> &str {
    line.trim().trim_start_matches(|c: char| BOX.contains(&c) || c.is_whitespace()).trim_end_matches(|c: char| BOX.contains(&c) || c.is_whitespace())
}

fn code_like(text: &str, error: bool) -> bool {
    let low = text.to_lowercase();
    let no_links = low.replace("://", " ");
    if CODE_BITS.iter().any(|b| low.contains(b)) || no_links.contains("//") || FILE_LINE.is_match(text) {
        return true;
    }
    if low.contains("src/") || low.ends_with(';') || low.ends_with('{') || low.ends_with(',') {
        return true;
    }
    !error && (text.contains('"') || text.contains('{'))
}

fn continuation(raw: &str, line: &str) -> bool {
    raw.starts_with(char::is_whitespace) && !line.starts_with(TOOL_MARK) && !MARKS.iter().any(|m| line.starts_with(m))
}

pub fn limit_line(raw: &str) -> bool {
    let line = strip_box(raw);
    if line.is_empty() || line.chars().count() > LINE_MAX || continuation(raw, line) {
        return false;
    }
    if PROSE_HEADS.iter().any(|h| line.starts_with(h)) || LIST_HEAD.is_match(line) {
        return false;
    }
    let mut body = line;
    let tool = match body.strip_prefix(TOOL_MARK) {
        Some(rest) => {
            body = rest.trim_start();
            true
        }
        None => false,
    };
    let mut marked = false;
    for mark in MARKS {
        if let Some(rest) = body.strip_prefix(mark) {
            body = rest.trim_start();
            marked = true;
            break;
        }
    }
    let low = body.to_lowercase();
    let error = ERROR_HEADS.iter().any(|h| low.starts_with(h));
    if code_like(body, error) || NOT.iter().any(|n| low.contains(n)) {
        return false;
    }
    if !HIT.iter().any(|h| low.contains(h)) || !STRONG.iter().any(|s| low.contains(s)) {
        return false;
    }
    let banner = BANNERS.iter().any(|b| low.contains(b));
    let own = OWN.iter().any(|o| low.contains(o));
    if tool {
        return banner || own;
    }
    let banner_head = BANNERS.iter().any(|b| low.starts_with(b));
    marked || error || banner_head || (banner && own)
}

pub fn last_lines(screen: &str, count: usize) -> Vec<&str> {
    let lines: Vec<&str> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
    let from = lines.len().saturating_sub(count);
    lines[from..].to_vec()
}

pub fn find(screen: &str, now: u64, here: &Zone) -> Option<Hit> {
    let lines = last_lines(screen, LAST_LINES);
    let at = lines.iter().rposition(|l| limit_line(l))?;
    let line = strip_box(lines[at]).to_string();
    let next = lines.get(at + 1).map(|l| strip_box(l)).unwrap_or("");
    let until = parse_until(&line, now, here).or_else(|| (!next.is_empty() && !limit_line(next)).then(|| parse_until(next, now, here)).flatten());
    let start = at.saturating_sub(BEFORE_LINES);
    let before = lines[start..at].iter().map(|l| strip_box(l)).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    Some(Hit { line, until, before: crate::util::clip_flat(&before, 300) })
}

fn span_ms(text: &str) -> Option<u64> {
    let caps = SPAN.captures(text)?;
    let mut total: u64 = 0;
    for part in PART.captures_iter(&caps[1]) {
        let n: u64 = part[1].parse().ok()?;
        let unit = part[2].to_lowercase();
        let each = match unit.chars().next()? {
            'd' => 86_400_000,
            'h' => 3_600_000,
            'm' => 60_000,
            _ => 1000,
        };
        total = total.saturating_add(n.saturating_mul(each));
    }
    (total > 0).then_some(total)
}

fn month_of(word: &str) -> Option<i64> {
    let names = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    names.iter().position(|m| word.to_lowercase().starts_with(m)).map(|i| i as i64 + 1)
}

fn weekday_of(word: &str) -> Option<i64> {
    let names = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    names.iter().position(|d| word.to_lowercase().starts_with(d)).map(|i| i as i64)
}

fn clock_of(rest: &str) -> Option<i64> {
    if let Some(c) = AMPM.captures(rest) {
        let hour: i64 = c[1].parse().ok()?;
        let min: i64 = c.get(2).map_or(Some(0), |m| m.as_str().parse().ok())?;
        if !(1..=12).contains(&hour) || min > 59 {
            return None;
        }
        let pm = c[3].eq_ignore_ascii_case("p");
        let hour = (hour % 12) + if pm { 12 } else { 0 };
        return Some(hour * 3600 + min * 60);
    }
    let c = CLOCK24.captures(rest)?;
    let hour: i64 = c[1].parse().ok()?;
    let min: i64 = c[2].parse().ok()?;
    (hour < 24 && min < 60).then_some(hour * 3600 + min * 60)
}

fn utc_ms(place: &Zone, local: i64) -> Option<u64> {
    zone::to_utc(place, local).filter(|s| *s >= 0).map(|s| s as u64 * 1000)
}

fn at_ms(rest: &str, now: u64, here: &Zone) -> Option<u64> {
    let named = ZONE_NAME.captures(rest).and_then(|c| zone::named(&c[1]));
    let place = named.as_ref().unwrap_or(here);
    let clock = clock_of(rest);
    let day_word = MONTH.captures(rest);
    let week = WEEKDAY.captures(rest).and_then(|c| weekday_of(&c[1]));
    if clock.is_none() && day_word.is_none() && week.is_none() {
        return None;
    }
    let secs = clock.unwrap_or(0);
    let local_now = zone::local_s(place, now as i64);
    let today = local_now.div_euclid(DAY_S);
    let after_now = |t: Option<u64>| t.filter(|t| *t > now);
    if let Some(c) = day_word {
        let month = month_of(&c[1])?;
        let day: i64 = c[2].parse().ok()?;
        let (year, _, _) = zone::civil(today);
        for y in [year, year + 1] {
            let local = zone::days_from_civil(y, month, day) * DAY_S + secs;
            if let Some(t) = after_now(utc_ms(place, local)) {
                return Some(t);
            }
        }
        return None;
    }
    if let Some(want) = week {
        let delta = (want - zone::weekday(today)).rem_euclid(7);
        for extra in [0, 7] {
            let local = (today + delta + extra) * DAY_S + secs;
            if let Some(t) = after_now(utc_ms(place, local)) {
                return Some(t);
            }
        }
        return None;
    }
    for extra in [0, 1] {
        let local = (today + extra) * DAY_S + secs;
        if let Some(t) = after_now(utc_ms(place, local)) {
            return Some(t);
        }
    }
    None
}

pub fn parse_until(text: &str, now: u64, here: &Zone) -> Option<u64> {
    if let Some(c) = EPOCH.captures(text) {
        let secs: u64 = c[1].parse().ok()?;
        return Some(secs * 1000).filter(|t| *t > now);
    }
    if let Some(span) = span_ms(text) {
        return Some(now + span);
    }
    let rest = AT.captures(text)?.get(1)?.as_str();
    at_ms(rest, now, here)
}

pub fn parse_given(text: &str, now: u64, here: &Zone) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(ms) = text.parse::<u64>()
        && ms > now
    {
        return Some(ms);
    }
    parse_until(text, now, here)
        .or_else(|| parse_until(&format!("in {text}"), now, here))
        .or_else(|| parse_until(&format!("resets {text}"), now, here))
}
