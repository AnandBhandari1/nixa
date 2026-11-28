use super::motion;
use super::npc;
use super::path;
use super::plan::{self, Plan};
use super::sync;
use crate::screens::nixa_screen::map::{Agent, DEEP, NIXA, St};

pub const WALK_SPEED: f32 = 70.0;
pub const WALK_MIN: u64 = 700;
pub const WALK_MAX: u64 = 5200;
pub const WALK_EXTRA: u64 = 440;
pub const SEAT_MS: u64 = 180;
pub const OUT_MS: u64 = 1250;
pub const MOMENT_MS: u64 = 1400;
pub const SAME_MS: u64 = 4000;
pub const STALE_MS: u64 = 25_000;
pub const QUEUE_MAX: usize = 6;
pub const TEXT_MAX: usize = 80;
pub const HOLD_MS: u64 = 2800;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Talk {
    pub from: String,
    pub to: String,
    pub first: String,
    pub second: String,
    pub given: bool,
    pub at: u64,
    pub ask: u64,
    pub asked: u64,
    pub ready: bool,
    pub banter: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Playing {
    pub talk: Talk,
    pub start: u64,
    pub walk: u64,
    pub say1: u64,
    pub say2: u64,
    pub said: u8,
    pub from_at: (f32, f32),
    pub to_at: (f32, f32),
    pub lead: u64,
    pub open: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stage {
    pub queue: Vec<Talk>,
    pub now: Option<Playing>,
    pub seen: Vec<(String, St, u64)>,
    pub talk_end: Vec<(String, u64)>,
    pub soap: super::soap::Soap,
    pub fun: super::fun::Fun,
    pub voice: super::sync::Sync,
    pub call: super::call::Call,
    pub leave: super::leave::Leave,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cut {
    pub dolly: f32,
    pub bars: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    WalkTo(f32),
    Say1(f32),
    Say2(f32),
    WalkBack(f32),
    Over,
}

pub fn cut_text(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let first = one
        .char_indices()
        .find(|(i, c)| matches!(c, '.' | '!' | '?') && one[i + c.len_utf8()..].starts_with(' '))
        .map(|(i, c)| one[..i + c.len_utf8()].to_string())
        .unwrap_or(one);
    if first.chars().count() <= TEXT_MAX {
        return first;
    }
    let head: String = first.chars().take(TEXT_MAX - 1).collect();
    let cut = head.rfind(' ').filter(|i| *i > TEXT_MAX / 2).map(|i| &head[..i]).unwrap_or(&head);
    format!("{}…", cut.trim_end_matches([',', ';', ':', ' ']))
}

fn is_brain(who: &str) -> bool {
    who == NIXA || who == DEEP
}

pub fn lines_of(from: &str, to: &str, kind: Option<&str>, agents: &[Agent]) -> (String, String) {
    let status = |id: &str| agents.iter().find(|a| a.id == id).map(|a| a.status);
    let task = |id: &str| agents.iter().find(|a| a.id == id).and_then(Agent::task);
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    if from == NIXA && to == DEEP {
        return pair("I need your advice on this one.", "Let me think it through.");
    }
    if from == DEEP && to == NIXA {
        return pair("Here is my advice. Go ahead.", "Thanks, that helps a lot.");
    }
    if kind == Some("send") {
        return pair("I have a message for you.", "Got it, thanks!");
    }
    let parent_of = |kid: &str, parent: &str| {
        agents.iter().any(|a| a.id == kid && a.parent.as_deref() == Some(parent))
    };
    if is_brain(to) || parent_of(from, to) {
        return match status(from) {
            Some(St::Blocked) => pair("I am stuck. I need your help.", "Let me look at it."),
            _ => pair("My work is done. Please check it.", "Great work, thank you!"),
        };
    }
    if !is_brain(from) && !parent_of(to, from) {
        return pair("Please take over this task.", "Sure, I will take it.");
    }
    match task(to) {
        Some(n) => (format!("Please work on task #{n}."), "On it!".into()),
        None => pair("I have a new job for you.", "On it!"),
    }
}

pub fn talk_of(from: &str, to: &str, kind: Option<&str>, text: Option<&str>, agents: &[Agent], at: u64) -> Talk {
    let (first, second) = lines_of(from, to, kind, agents);
    let given = text.map(str::trim).filter(|t| !t.is_empty());
    Talk {
        from: from.to_string(),
        to: to.to_string(),
        first: given.map(cut_text).unwrap_or(first),
        second,
        given: given.is_some(),
        at,
        ask: 0,
        asked: 0,
        ready: false,
        banter: false,
    }
}

pub fn waiting(t: &Talk, now: u64) -> bool {
    t.ask != 0 && !t.ready && now.saturating_sub(t.asked) < HOLD_MS
}

pub fn push(stage: &mut Stage, talk: Talk) {
    let same = |t: &Talk| t.from == talk.from && t.to == talk.to && talk.at.abs_diff(t.at) < SAME_MS;
    if let Some(old) = stage.queue.iter_mut().find(|t| same(t)) {
        if talk.given && !old.given {
            old.first = talk.first;
            old.given = true;
            old.ask = 0;
            old.ready = false;
        }
        return;
    }
    if let Some(p) = stage.now.as_mut().filter(|p| same(&p.talk)) {
        if talk.given && !p.talk.given && p.said == 0 {
            p.talk.first = talk.first;
            p.talk.given = true;
            p.talk.ask = 0;
        }
        return;
    }
    stage.queue.push(talk);
    if stage.queue.len() > QUEUE_MAX {
        stage.queue.remove(0);
    }
}

pub fn say_ms(text: &str, low: u64, high: u64) -> u64 {
    (1200 + 55 * text.chars().count() as u64).clamp(low, high)
}

pub fn say1_ms(text: &str) -> u64 {
    say_ms(text, 2400, 8000)
}

pub fn say2_ms(text: &str) -> u64 {
    say_ms(text, 1600, 4000)
}

pub fn phase(p: &Playing, now: u64) -> Phase {
    let t = now.saturating_sub(p.start);
    let k = |from: u64, len: u64| (t - from) as f32 / len.max(1) as f32;
    let a = p.walk;
    let b = a + p.say1;
    let c = b + p.say2;
    if t < a {
        Phase::WalkTo(k(0, a))
    } else if t < b {
        Phase::Say1(k(a, p.say1))
    } else if t < c {
        Phase::Say2(k(b, p.say2))
    } else if t < end_ms(p) {
        Phase::WalkBack(k(c, p.walk).min(1.0))
    } else {
        Phase::Over
    }
}

pub fn end_ms(p: &Playing) -> u64 {
    p.walk + p.say1 + p.say2 + p.walk.max(OUT_MS)
}

fn ramp(t: f32, from: f32, len: f32) -> f32 {
    ((t - from) / len).clamp(0.0, 1.0)
}

pub fn cut(p: &Playing, now: u64) -> Cut {
    let t = now.saturating_sub(p.start) as f32;
    let a = p.walk as f32;
    let c = a + p.say1 as f32 + p.say2 as f32;
    let din = (a - 750.0).max(0.0);
    let dout = c + 250.0;
    let dolly = if t < dout {
        motion::in_out_cubic(ramp(t, din, 900.0))
    } else {
        1.0 - motion::in_out_cubic(ramp(t, dout, 1000.0))
    };
    let bin = din - 120.0;
    let bout = dout + 400.0;
    let bars = if t < bin {
        0.0
    } else if t < bout {
        motion::out_cubic(ramp(t, bin, 380.0))
    } else {
        1.0 - motion::in_out_cubic(ramp(t, bout, 500.0))
    };
    Cut { dolly, bars }
}

pub fn walk_ms(len: f32) -> u64 {
    ((len / WALK_SPEED * 1000.0) as u64 + WALK_EXTRA).clamp(WALK_MIN, WALK_MAX)
}

pub fn start(talk: Talk, plan: &Plan, now: u64, ends: &[(String, u64)]) -> Option<Playing> {
    let from = plan::find(plan, &talk.from)?;
    let to = plan::find(plan, &talk.to)?;
    if from.id == to.id {
        return None;
    }
    let life = npc::life_after(plan, now, &[], ends);
    let where_now = |p: &plan::Person| {
        plan.people
            .iter()
            .position(|q| q.id == p.id)
            .and_then(|i| life.get(i).copied().flatten())
            .map(|n| n.at)
            .unwrap_or(p.stand)
    };
    let (from_at, to_at) = (where_now(from), where_now(to));
    let meet = plan::meet_point(from.stand, to.stand);
    let len = path::route(plan, from_at, meet).len;
    Some(Playing {
        from_at,
        to_at,
        walk: walk_ms(len),
        say1: say1_ms(&talk.first),
        say2: say2_ms(&talk.second),
        talk,
        start: now,
        said: 0,
        lead: 0,
        open: false,
    })
}

pub fn mark_end(ends: &mut Vec<(String, u64)>, plan: &Plan, who: &str, at: u64) {
    let Some(p) = plan::find(plan, who) else {
        return;
    };
    match ends.iter_mut().find(|(id, _)| id == &p.id) {
        Some(e) => e.1 = at,
        None => ends.push((p.id.clone(), at)),
    }
}

pub fn advance(stage: &mut Stage, plan: &Plan, now: u64) {
    if let Some(p) = stage.now.take_if(|p| phase(p, now) == Phase::Over) {
        mark_end(&mut stage.talk_end, plan, &p.talk.from, now);
        mark_end(&mut stage.talk_end, plan, &p.talk.to, now);
    }
    stage.talk_end.retain(|(id, _)| plan.people.iter().any(|p| &p.id == id));
    stage.queue.retain(|t| now.saturating_sub(t.at) < STALE_MS);
    while stage.now.is_none() && stage.queue.first().is_some_and(|t| !waiting(t, now)) {
        let talk = stage.queue.remove(0);
        stage.now = start(talk, plan, now, &stage.talk_end);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    First(String, String),
    Second(String, String),
}

pub fn due_line(p: &mut Playing, now: u64) -> Option<Line> {
    match phase(p, now) {
        Phase::Say1(_) if p.said == 0 => {
            p.said = 1;
            p.lead = 0;
            p.open = false;
            Some(Line::First(p.talk.from.clone(), p.talk.first.clone()))
        }
        Phase::Say2(_) if p.said < 2 => {
            p.said = 2;
            p.lead = 0;
            p.open = false;
            Some(Line::Second(p.talk.to.clone(), p.talk.second.clone()))
        }
        _ => None,
    }
}

pub fn hold(p: &mut Playing, line: &sync::Line, now: u64) {
    let begin = match phase(p, now) {
        Phase::Say1(_) if p.said == 1 && line.text == p.talk.first => p.start + p.walk,
        Phase::Say2(_) if p.said == 2 && line.text == p.talk.second => p.start + p.walk + p.say1,
        _ => return,
    };
    p.lead = sync::shown_at(line).saturating_sub(begin);
    p.open = sync::open(line, now);
    let len = sync::done_at(line).saturating_sub(begin).max(1);
    if p.said == 1 {
        p.say1 = len;
    } else {
        p.say2 = len;
    }
}

pub fn see(stage: &mut Stage, plan: &Plan, wall: u64) {
    stage.seen.retain(|(id, _, _)| plan.people.iter().any(|p| &p.id == id));
    for p in &plan.people {
        match stage.seen.iter_mut().find(|(id, _, _)| id == &p.id) {
            Some(e) if e.1 != p.status => {
                e.1 = p.status;
                e.2 = wall;
            }
            Some(_) => {}
            None => stage.seen.push((p.id.clone(), p.status, 0)),
        }
    }
}

pub fn age(seen: &[(String, St, u64)], id: &str, now: u64) -> Option<u64> {
    seen.iter()
        .find(|(i, _, t)| i == id && *t > 0)
        .map(|(_, _, t)| now.saturating_sub(*t))
}

pub fn fresh(seen: &[(String, St, u64)], now: u64) -> bool {
    seen.iter().any(|(_, _, t)| *t > 0 && now.saturating_sub(*t) < MOMENT_MS)
}
