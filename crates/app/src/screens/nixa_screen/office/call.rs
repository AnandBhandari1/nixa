use super::motion;
use super::sync;

pub const QUIET_MS: u64 = 4000;
pub const REPLY_WAIT_MS: u64 = 30_000;
pub const PART_MAX_MS: u64 = 60_000;
pub const CHAR_MS: u64 = 65;
pub const SENTENCE_MIN_MS: u64 = 900;
pub const LINGER_MS: u64 = 1200;
pub const CATCH_MS: u64 = 700;
pub const ZOOM_MS: u64 = 900;
pub const USER_MAX: usize = 160;
pub const SENTENCE_MAX: usize = 140;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Part {
    pub sentences: Vec<String>,
    pub start: u64,
    pub end: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Call {
    pub user: Option<(String, u64)>,
    pub reply: Option<(Vec<String>, u64)>,
    pub part: Option<Part>,
    pub voiced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Said {
    pub text: String,
    pub age: u64,
    pub talking: bool,
}

fn cut(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let head: String = one.chars().take(max - 1).collect();
    format!("{}…", head.trim_end())
}

pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut from = 0;
        for (k, (i, c)) in chars.iter().enumerate() {
            let end = matches!(c, '.' | '!' | '?') && chars.get(k + 1).is_none_or(|(_, n)| n.is_whitespace());
            if end {
                out.push(line[from..i + c.len_utf8()].to_string());
                from = i + c.len_utf8();
            }
        }
        out.push(line[from..].to_string());
    }
    out.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| cut(s, SENTENCE_MAX)).collect()
}

pub fn user(call: &mut Call, text: &str, now: u64) -> bool {
    let text = cut(text, USER_MAX);
    if text.is_empty() {
        return false;
    }
    *call = Call { user: Some((text, now)), ..Call::default() };
    true
}

pub fn reply(call: &mut Call, text: &str, now: u64) -> bool {
    if call.user.is_none() || call.reply.is_some() || !on(call, now) {
        return false;
    }
    let list = sentences(text);
    if list.is_empty() {
        return false;
    }
    call.reply = Some((list, now));
    true
}

pub fn voice(call: &mut Call, text: &str, state: &str, now: u64) -> bool {
    if call.user.is_none() || !on(call, now) {
        return false;
    }
    match state {
        "start" => {
            let list = sentences(text);
            if list.is_empty() {
                return false;
            }
            call.part = Some(Part { sentences: list, start: now, end: None });
            call.voiced = true;
            true
        }
        _ => match call.part.as_mut().filter(|p| p.end.is_none()) {
            Some(p) => {
                p.end = Some(now);
                true
            }
            None => false,
        },
    }
}

fn sentence_ms(s: &str) -> u64 {
    (CHAR_MS * s.chars().count() as u64).max(SENTENCE_MIN_MS)
}

fn index_at(p: &Part, t: u64) -> (usize, u64) {
    let last = p.sentences.len().saturating_sub(1);
    let mut acc = 0;
    for (i, s) in p.sentences.iter().enumerate() {
        let len = sentence_ms(s);
        if t < acc + len || i == last {
            return (i, t - acc);
        }
        acc += len;
    }
    (last, 0)
}

fn index_start(p: &Part, i: usize) -> u64 {
    p.sentences.iter().take(i).map(|s| sentence_ms(s)).sum()
}

fn left_at_end(p: &Part, end: u64) -> usize {
    let last = p.sentences.len().saturating_sub(1);
    last - index_at(p, end.saturating_sub(p.start)).0.min(last)
}

fn part_end(p: &Part) -> Option<u64> {
    p.end.map(|e| e + left_at_end(p, e) as u64 * CATCH_MS + LINGER_MS)
}

fn reply_end(list: &[String], at: u64) -> u64 {
    at + sync::WAIT_MS + list.iter().map(|s| sync::timer_ms(s)).sum::<u64>()
}

fn busy_until(call: &Call) -> u64 {
    if let Some(p) = &call.part {
        return part_end(p).unwrap_or(p.start + PART_MAX_MS);
    }
    if let Some((list, at)) = &call.reply {
        return reply_end(list, *at);
    }
    call.user.as_ref().map(|(_, at)| at + REPLY_WAIT_MS).unwrap_or(0)
}

pub fn quiet_at(call: &Call) -> u64 {
    match call.user {
        Some(_) => busy_until(call) + QUIET_MS,
        None => 0,
    }
}

pub fn on(call: &Call, now: u64) -> bool {
    call.user.is_some() && now < quiet_at(call)
}

pub fn line(call: &Call, now: u64) -> Option<Said> {
    if let Some(p) = &call.part {
        if part_end(p).is_some_and(|e| now >= e) {
            return None;
        }
        let Some(end) = p.end.filter(|e| now >= *e) else {
            let (i, age) = index_at(p, now.saturating_sub(p.start));
            return p.sentences.get(i).map(|s| Said { text: s.clone(), age, talking: true });
        };
        let last = p.sentences.len().saturating_sub(1);
        let from = last - left_at_end(p, end);
        let after = now - end;
        let (i, age) = if from == last {
            (last, now.saturating_sub(p.start).saturating_sub(index_start(p, last)))
        } else {
            let i = (from + 1 + (after / CATCH_MS) as usize).min(last);
            (i, after - (i - from - 1) as u64 * CATCH_MS)
        };
        return p.sentences.get(i).map(|s| Said { text: s.clone(), age, talking: false });
    }
    if call.voiced {
        return None;
    }
    let (list, at) = call.reply.as_ref()?;
    let begin = at + sync::WAIT_MS;
    if now < begin {
        return None;
    }
    let t = now - begin;
    let mut acc = 0;
    for s in list {
        let len = sync::timer_ms(s);
        if t < acc + len {
            let age = t - acc;
            return Some(Said { text: s.clone(), age, talking: age + 400 < len });
        }
        acc += len;
    }
    None
}

pub fn thinking(call: &Call, now: u64) -> bool {
    if !on(call, now) || line(call, now).is_some() {
        return false;
    }
    match (&call.part, &call.reply) {
        (Some(_), _) => false,
        (None, Some((_, at))) => now < at + sync::WAIT_MS,
        (None, None) => call.user.as_ref().is_some_and(|(_, at)| now < at + REPLY_WAIT_MS),
    }
}

pub fn dolly(call: &Call, now: u64) -> f32 {
    let Some((_, at)) = call.user.as_ref() else {
        return 0.0;
    };
    let out = busy_until(call) + QUIET_MS;
    let ramp = |from: u64| (now.saturating_sub(from) as f32 / ZOOM_MS as f32).clamp(0.0, 1.0);
    if now < out {
        motion::in_out_cubic(ramp(*at))
    } else {
        1.0 - motion::in_out_cubic(ramp(out))
    }
}
