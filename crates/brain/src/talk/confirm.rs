use super::words::Reply;
use crate::util::clip;
use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;
use tokio::sync::oneshot;

pub const CONFIRM_VOICE_MS: u64 = 30_000;
pub const ASK_MS: u64 = 30_000;
pub const ASK_HEARD_MS: u64 = 2_000;
pub const CONFIRM_SPOKEN_CHARS: usize = 160;
pub const CONFIRM_MS: u64 = 120_000;
pub const NOT_READY: u64 = u64::MAX;
pub const WAIT_LINE: &str = "Say \"yes\" to confirm, or click Yes.";

static SEND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^Send "[\s\S]*" to (.+?)\??$"#).expect("send ask"));
static TERM_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\(m\w+\)$").expect("term id"));
static START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Start (.+?) with the first message").expect("start ask"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Yes,
    No,
    Wait,
    Cancel,
    Pass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum End {
    Yes,
    No,
    Timeout,
    Cancelled,
}

impl End {
    pub fn word(self) -> &'static str {
        match self {
            End::Yes => "Yes",
            End::No => "No",
            End::Timeout => "No answer",
            End::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub text: String,
    pub typed: bool,
    pub started_at: u64,
    pub at: u64,
    pub pressed: bool,
    pub mac: bool,
}

impl Said {
    pub fn typed(text: &str, at: u64) -> Said {
        Said { text: text.to_string(), typed: true, started_at: at, at, pressed: false, mac: true }
    }

    pub fn heard(text: &str, started_at: u64, at: u64) -> Said {
        Said { text: text.to_string(), typed: false, started_at, at, pressed: false, mac: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub id: u64,
    pub message: String,
    pub why: String,
    pub asked_at: u64,
    #[serde(skip)]
    pub ready_at: u64,
    #[serde(skip)]
    pub wait_said: bool,
    pub deadline: u64,
}

#[derive(Default)]
pub struct Confirms {
    pub open: Option<Ask>,
    pub count: u64,
    pub last_end: Option<End>,
    waiter: Option<oneshot::Sender<bool>>,
}

impl Confirms {
    pub fn ask(&mut self, message: &str, why: &str, now: u64) -> Option<(u64, oneshot::Receiver<bool>)> {
        if self.open.is_some() {
            return None;
        }
        self.count += 1;
        let id = self.count;
        let (tx, rx) = oneshot::channel();
        self.open = Some(Ask {
            id,
            message: message.to_string(),
            why: why.to_string(),
            asked_at: now,
            ready_at: NOT_READY,
            wait_said: false,
            deadline: now + CONFIRM_MS,
        });
        self.waiter = Some(tx);
        Some((id, rx))
    }

    pub fn finish(&mut self, ok: bool, id: Option<u64>, end: End) -> Option<(Ask, End)> {
        let open = self.open.as_ref()?;
        if id.is_some_and(|id| id != open.id) {
            return None;
        }
        let ask = self.open.take()?;
        let end = if ok { End::Yes } else if end == End::Yes { End::No } else { end };
        self.last_end = Some(end);
        if let Some(tx) = self.waiter.take() {
            let _ = tx.send(ok);
        }
        Some((ask, end))
    }

    pub fn reask(&mut self, now: u64) -> Option<(u64, String)> {
        let open = self.open.as_mut()?;
        open.ready_at = NOT_READY;
        open.asked_at = now;
        Some((open.id, open.message.clone()))
    }

    pub fn ready(&mut self, id: u64, now: u64) {
        if let Some(open) = self.open.as_mut()
            && open.id == id
        {
            open.ready_at = now;
        }
    }

    pub fn expired(&self, now: u64) -> Option<u64> {
        self.open.as_ref().filter(|a| now >= a.deadline).map(|a| a.id)
    }
}

pub fn confirm_step(reply: Option<Reply>, said: &Said, asked_at: u64, ready_at: u64) -> Step {
    confirm_step_with(reply, said, asked_at, ready_at, ASK_HEARD_MS, CONFIRM_VOICE_MS)
}

pub fn confirm_step_with(reply: Option<Reply>, said: &Said, asked_at: u64, ready_at: u64, heard_ms: u64, voice_ms: u64) -> Step {
    match reply {
        Some(Reply::No) => Step::No,
        Some(Reply::Unclear) => Step::Wait,
        Some(Reply::Yes) => {
            if said.typed {
                return Step::Yes;
            }
            let after_ask = said.started_at >= ready_at;
            let late_in_ask = said.started_at >= asked_at.saturating_add(heard_ms);
            if (after_ask || late_in_ask) && said.at.saturating_sub(ready_at) <= voice_ms { Step::Yes } else { Step::Wait }
        }
        None if said.typed || said.at >= ready_at => Step::Cancel,
        None => Step::Pass,
    }
}

fn trim_question(text: &str) -> &str {
    text.trim_end_matches('?')
}

pub fn spoken_confirm(message: &str) -> String {
    spoken_confirm_max(message, CONFIRM_SPOKEN_CHARS)
}

pub fn spoken_confirm_max(message: &str, max: usize) -> String {
    let trimmed = message.trim();
    let ask = format!("{}?", trim_question(trimmed));
    if ask.chars().count() <= max {
        return format!("{ask} Say yes or no.");
    }
    if let Some(found) = SEND.captures(trimmed) {
        let who = TERM_ID.replace(&found[1], "").to_string();
        return format!("Send this message to {who}? Say yes or no.");
    }
    if let Some(found) = START.captures(trimmed) {
        return format!("Start {} with a first message? Say yes or no.", &found[1]);
    }
    let cut: String = ask.chars().take(max).collect();
    let space = cut.rfind(' ').map(|i| cut[..i].chars().count()).unwrap_or(0);
    let keep: String = cut.chars().take(space.max(40)).collect();
    format!("{keep}… Say yes or no.")
}

pub fn end_line(ask: &Ask, end: End) -> String {
    format!("{}: {}", end.word(), clip(trim_question(&ask.message), 90))
}

pub fn timeout_note(ask: &Ask) -> String {
    let text = clip(trim_question(&ask.message), 300).replace(['"', '\u{201c}', '\u{201d}'], "'");
    format!("Note: Nobody answered this card in {} minutes, so it was not done. The user did not say no: \"{text}\"", CONFIRM_MS / 60_000)
}
