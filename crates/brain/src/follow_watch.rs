use crate::agent;
use crate::app::{App, lock};
use crate::deep::Ask;
use crate::util::{clip_flat, log};
use nixa_protocol::{Status, TermInfo};
use regex::Regex;
use std::sync::{Arc, LazyLock, Mutex};

pub const PROMISE_MS: u64 = 2 * 60_000;
pub const DEEP_FOLLOW_MS: u64 = 30_000;
pub const DEEP_FOLLOW_MAX_MS: u64 = 10 * 60_000;
const NAME_MIN: usize = 3;

static PROMISE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:i'?ll|i will|will|going to)\b[^.!?]{0,80}\b(?:check|report|watch|look|follow up|get back|update you|let you know|tell you|read)\b[^.!?]{0,100}\b(?:when|once|after|as soon as)\b|\b(?:i'?m|i am|i'?ll|i will)\s+(?:be\s+)?wait(?:ing)?\b").expect("promise")
});

#[derive(Debug, Clone, PartialEq)]
pub struct Promise {
    pub line: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeepFollow {
    pub ask: String,
    pub question: String,
    pub said: String,
    pub at: u64,
}

#[derive(Default)]
pub struct Follow {
    pub promise: Option<Promise>,
    pub deep: Vec<DeepFollow>,
}

static FOLLOW: LazyLock<Mutex<Follow>> = LazyLock::new(|| Mutex::new(Follow::default()));

pub fn promise_in(text: &str) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let found = PROMISE.find(&flat)?;
    let start = flat[..found.start()].rfind(['.', '!', '?']).map(|i| i + 1).unwrap_or(0);
    let end = flat[found.end()..].find(['.', '!', '?']).map(|i| found.end() + i + 1).unwrap_or(flat.len());
    Some(clip_flat(flat[start..end].trim(), 240))
}

pub fn note_reply(text: &str, now: u64) {
    let mut follow = lock(&FOLLOW);
    follow.promise = promise_in(text).map(|line| Promise { line, at: now });
}

pub fn note_answer(ask: &Ask, now: u64) {
    let said = [ask.pick.as_deref().unwrap_or_default(), ask.why.as_deref().unwrap_or_default()].join(" ");
    let mut follow = lock(&FOLLOW);
    follow.deep.retain(|d| now.saturating_sub(d.at) < DEEP_FOLLOW_MAX_MS && d.ask != ask.id);
    follow.deep.push(DeepFollow { ask: ask.id.clone(), question: clip_flat(&ask.question, 160), said, at: now });
}

pub fn promise_text(promise: &Promise, now: u64) -> String {
    let minutes = (now.saturating_sub(promise.at) / 60_000).max(1);
    format!(
        "Event: Follow-up. {minutes} min ago you said: \"{}\". Do not wait any longer. Check it now with your tools (term ls, term read, task ls, deep list). If it came or finished, act on it and report. If not, take the next useful step yourself and say it in one short line.",
        promise.line
    )
}

fn status_word(status: Option<Status>) -> &'static str {
    match status {
        Some(Status::Working) => "working",
        Some(Status::Done) => "done",
        Some(Status::Idle) => "idle",
        Some(Status::Blocked) => "blocked",
        Some(Status::Error) => "error",
        _ => "starting",
    }
}

pub fn named_helpers<'a>(follow: &DeepFollow, terms: &'a [TermInfo]) -> Vec<&'a TermInfo> {
    let said = follow.said.to_lowercase();
    terms
        .iter()
        .filter(|t| t.running && t.created_ms + 1_000 >= follow.at)
        .filter(|t| t.labels.get(crate::deep::ROLE_LABEL).is_none_or(|r| r != crate::deep::ROLE_VALUE))
        .filter(|t| {
            let name = t.name.trim().to_lowercase();
            name.chars().count() >= NAME_MIN && said.contains(&name)
        })
        .collect()
}

pub fn helpers_text(follow: &DeepFollow, found: &[&TermInfo]) -> String {
    let list: Vec<String> = found.iter().map(|t| format!("{} ({}, {})", t.id, clip_flat(&t.name, 40), status_word(t.status))).collect();
    format!(
        "Event: Follow-up on deep brain answer {} (\"{}\"). The helper it named started after the answer: {}. Do not wait on the deep brain again. Watch the helper; when it is done, read its result with term read and report it in one short line, then take the next step.",
        follow.ask,
        follow.question,
        list.join(", ")
    )
}

pub fn take_due_deep(now: u64) -> Vec<DeepFollow> {
    let mut follow = lock(&FOLLOW);
    let (due, wait): (Vec<DeepFollow>, Vec<DeepFollow>) = follow.deep.drain(..).partition(|d| now.saturating_sub(d.at) >= DEEP_FOLLOW_MS);
    follow.deep = wait;
    due
}

pub fn take_due_promise(now: u64, busy: bool) -> Option<Promise> {
    let mut follow = lock(&FOLLOW);
    let due = follow.promise.as_ref().is_some_and(|p| now.saturating_sub(p.at) >= PROMISE_MS);
    if busy || !due {
        return None;
    }
    follow.promise.take()
}

pub async fn tick(app: &Arc<App>, now: u64) {
    if !crate::settings_api::nixa_on(&app.dir) || !agent::has_brain(app) {
        return;
    }
    let due = take_due_deep(now);
    if !due.is_empty()
        && let Ok(terms) = crate::engine::list().await
    {
        for follow in &due {
            let found = named_helpers(follow, &terms);
            if found.is_empty() {
                continue;
            }
            log("follow", &format!("deep answer {} named a helper that started later: {}", follow.ask, found.len()));
            crate::watch::tell_event(app, &helpers_text(follow, &found));
        }
    }
    if let Some(promise) = take_due_promise(now, agent::busy(app)) {
        log("follow", &format!("woke Nixa for its promise: {}", clip_flat(&promise.line, 120)));
        crate::watch::tell_event(app, &promise_text(&promise, now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_protocol::TermId;

    fn term(id: &str, name: &str, created_ms: u64) -> TermInfo {
        TermInfo {
            id: id.parse::<TermId>().unwrap(),
            name: name.into(),
            labels: Default::default(),
            parent: None,
            children: Vec::new(),
            cmd: Vec::new(),
            cwd: "/tmp".into(),
            pid: None,
            running: true,
            exit_code: None,
            status: Some(Status::Working),
            status_note: None,
            keep: false,
            draft: false,
            title: None,
            rows: 24,
            cols: 80,
            created_ms,
            seq: 0,
        }
    }

    #[test]
    fn finds_a_wait_promise_in_a_reply() {
        let said = "In nixa: no such terminal is running. I will not start a second one now; I'll check the deep brain's answer when it comes. Thanks.";
        assert_eq!(promise_in(said).as_deref(), Some("I will not start a second one now; I'll check the deep brain's answer when it comes."));
        assert!(promise_in("I'll report the findings once the helper finishes.").is_some());
        assert!(promise_in("I am waiting for the review.").is_some());
        assert!(promise_in("Done. The tests pass.").is_none());
        assert!(promise_in("I fixed the login page and ran the checks.").is_none());
    }

    #[test]
    fn a_promise_wakes_once_after_its_wait_and_never_while_busy() {
        note_reply("I'll report when the helper is done.", 1_000);
        assert!(take_due_promise(1_000 + PROMISE_MS - 1, false).is_none());
        assert!(take_due_promise(1_000 + PROMISE_MS, true).is_none());
        let woke = take_due_promise(1_000 + PROMISE_MS, false).unwrap();
        assert!(promise_text(&woke, 1_000 + PROMISE_MS).contains("Do not wait any longer"));
        assert!(take_due_promise(1_000 + 2 * PROMISE_MS, false).is_none());
        note_reply("I'll report when the helper is done.", 5_000);
        note_reply("All done.", 6_000);
        assert!(take_due_promise(6_000 + PROMISE_MS, false).is_none(), "a newer reply without a promise replaces it");
    }

    #[test]
    fn a_helper_named_in_a_deep_answer_and_started_later_is_found() {
        let follow = DeepFollow { ask: "d362".into(), question: "Open source check?".into(), said: "helper oss-review, terminal. A read-only helper checks secrets.".into(), at: 10_000 };
        let terms = vec![term("m1", "oss-review", 15_000), term("m2", "oss-review", 2_000), term("m3", "Developer", 16_000), term("m4", "ab", 16_000)];
        let found = named_helpers(&follow, &terms);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.as_str(), "m1");
        let text = helpers_text(&follow, &found);
        assert!(text.contains("d362") && text.contains("m1 (oss-review, working)") && text.contains("Do not wait on the deep brain again"));
    }
}
