use super::confirm::ASK_MS;
use super::words::SmallTalk;
use crate::reply::AgentReply;

pub const EARLIER_HEARD_MS: u64 = 2 * 60 * 1000;
pub const EARLIER_HEARD_LINES: usize = 2;
pub const EARLIER_HEARD_CHARS: usize = 500;
pub const AFTER_STOP_MS: u64 = 3000;
pub const NO_AFTER_MS: u64 = 10_000;
pub const NO_REPLY_TEXT: &str = "Sorry, I did not catch that. Please say it again.";
pub const NOTHING_SAID: &str = "I have not said anything yet.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeardLine {
    pub text: String,
    pub at: u64,
}

#[derive(Debug, Default)]
pub struct Heard {
    pub before: Vec<HeardLine>,
    pub stopped_at: u64,
    pub quiet_was: bool,
}

pub fn keep_heard(lines: &mut Vec<HeardLine>, line: HeardLine) {
    lines.push(line);
    if lines.len() > EARLIER_HEARD_LINES {
        let extra = lines.len() - EARLIER_HEARD_LINES;
        lines.drain(..extra);
    }
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}

pub fn earlier_heard_note(lines: &[HeardLine], at: u64) -> String {
    let recent: Vec<&HeardLine> = lines.iter().filter(|l| at >= l.at && at - l.at <= EARLIER_HEARD_MS).collect();
    if recent.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = recent
        .iter()
        .map(|l| format!("{} s before: \"{}\"", ((at - l.at) as f64 / 1000.0).round().max(1.0) as u64, cut(&l.text, EARLIER_HEARD_CHARS)))
        .collect();
    let (line, them) = if recent.len() == 1 { ("line", "it") } else { ("lines", "them") };
    format!(
        "Note: The user's last heard {line} before the next Heard line, oldest first: {}. The next line may go on from {them}: when it is unclear alone, read them together and act on all of it.",
        parts.join("; ")
    )
}

pub fn skip_heard(kind: Option<SmallTalk>, typed: bool, confirm_open: bool, prompts_open: usize, asked_ago_ms: u64) -> bool {
    match kind {
        _ if typed => false,
        None => false,
        Some(SmallTalk::Noise | SmallTalk::Thanks) => true,
        Some(SmallTalk::Assent) => !confirm_open && prompts_open == 0 && asked_ago_ms > ASK_MS,
    }
}

pub fn wake_allows(always: bool, typed: bool, named: bool, open: bool, confirm_open: bool) -> bool {
    always || typed || named || open || confirm_open
}

pub fn silent_to_user(user_turn: bool, reply: &AgentReply, since_stop_ms: u64) -> bool {
    user_turn && (!reply.speak || reply.text.trim().is_empty()) && reply.heard != "unfinished" && since_stop_ms > AFTER_STOP_MS
}
