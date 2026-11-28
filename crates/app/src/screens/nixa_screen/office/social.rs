use super::brush::Brush;
use super::motion::{hash, seed};
use super::plan::{self, Hang, Role};
use super::sprite;
use super::view::{Frame, Spot};
use crate::screens::nixa_screen::map::{St, deep_color};

const EDGE: u32 = 0x2a2e38;

pub const SLOT_MS: u64 = 1800;
pub const SAY_MS: u64 = 1500;
pub const SOLO_MS: u64 = 7000;

pub const CHATTER: [&str; 18] = [
    "Hi!", "Hey there!", "Coffee?", "Nice work!", "Ha ha!", "How's it going?", "All good!", "Tea time!",
    "Same here!", "Lunch later?", "Ship it!", "Yay!", "Need a break", "Good one!", "So true", "Busy day!",
    "Hello!", "See you!",
];

pub const SOLO: [&str; 8] = ["Hi!", "Mmm, tea", "Nice view", ":)", "Stretch!", "Yay!", "Hello!", "Ahh"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub who: usize,
    pub text: &'static str,
    pub age: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub speaker: usize,
    pub listener: usize,
    pub seed: u64,
}

fn paired(fr: &Frame, i: usize, j: usize) -> bool {
    let (Some(Some(a)), Some(Some(b))) = (fr.life.get(i), fr.life.get(j)) else {
        return false;
    };
    if !a.resting || !b.resting {
        return false;
    }
    match (a.hang, b.hang, a.spot, b.spot) {
        (Some(Hang::Sofa), Some(Hang::Sofa), Some((x, _)), Some((y, _))) => x.abs_diff(y) == 1,
        (Some(Hang::Tea), Some(Hang::Tea), Some((x, s)), Some((y, t))) => x == y && s != t,
        _ => false,
    }
}

fn visit(fr: &Frame, i: usize) -> Option<usize> {
    let n = fr.life.get(i)?.as_ref()?;
    if n.resting && n.hang == Some(Hang::Visit) {
        n.host
    } else {
        None
    }
}

pub fn make(a: usize, b: usize, ids: (&str, &str), now: u64) -> Pair {
    let s = seed(ids.0) ^ seed(ids.1).rotate_left(17);
    let first = ((now + s) / 1800).is_multiple_of(2);
    let (speaker, listener) = if first { (a, b) } else { (b, a) };
    Pair { speaker, listener, seed: s }
}

pub fn pairs(fr: &Frame) -> Vec<Pair> {
    let n = fr.plan.people.len();
    let mut used = vec![false; n];
    let mut out = Vec::new();
    let id = |i: usize| fr.plan.people[i].id.as_str();
    for i in 0..n {
        if used[i] {
            continue;
        }
        let other = visit(fr, i)
            .filter(|h| *h < n && !used[*h])
            .or_else(|| (i + 1..n).find(|&j| !used[j] && paired(fr, i, j)));
        if let Some(j) = other {
            used[i] = true;
            used[j] = true;
            out.push(make(i, j, (id(i), id(j)), fr.now));
        }
    }
    out
}

pub fn nod(pair: &Pair, now: u64) -> bool {
    (now + pair.seed) % 900 < 150
}

fn head(s: &Spot) -> (f32, f32) {
    (s.at.0, sprite::head_y(s.pose, s.at) + s.act.body)
}

pub fn pair_line(pair: &Pair, now: u64) -> Option<Line> {
    let t = now + pair.seed;
    let age = t % SLOT_MS;
    (age < SAY_MS).then(|| Line {
        who: pair.speaker,
        text: CHATTER[(hash(pair.seed, t / SLOT_MS) % CHATTER.len() as u64) as usize],
        age,
    })
}

pub fn solo_line(fr: &Frame, i: usize, paired: bool) -> Option<Line> {
    if paired {
        return None;
    }
    let n = fr.life.get(i)?.as_ref()?;
    if !n.resting {
        return None;
    }
    let s = seed(&fr.plan.people.get(i)?.id);
    let t = fr.now + s;
    let round = t / SOLO_MS;
    let age = t % SOLO_MS;
    (age < SAY_MS && hash(s, round).is_multiple_of(3)).then(|| Line {
        who: i,
        text: SOLO[(hash(s ^ 7, round) % SOLO.len() as u64) as usize],
        age,
    })
}

pub fn lines(fr: &Frame) -> Vec<Line> {
    let pairs = pairs(fr);
    let mut out: Vec<Line> = pairs.iter().filter_map(|p| pair_line(p, fr.now)).collect();
    for i in 0..fr.plan.people.len() {
        let paired = pairs.iter().any(|p| p.speaker == i || p.listener == i);
        out.extend(solo_line(fr, i, paired));
    }
    out
}

pub fn waves(text: &str) -> bool {
    matches!(text, "Hi!" | "Hello!" | "Hey there!" | "See you!")
}

pub fn thought(b: &mut Brush, (x, y): (f32, f32), left: bool, color: gpui::Hsla, now: u64) {
    let at = |dx: f32, w: f32| if left { x - dx - w } else { x + dx };
    b.solid((at(4.0, 1.0), y - 1.0, 1.0, 1.0), EDGE);
    b.solid((at(6.0, 2.0), y - 4.0, 2.0, 2.0), EDGE);
    let (cx, cy) = (at(7.0, 12.0), y - 13.0);
    b.solid((cx - 1.0, cy - 1.0, 14.0, 9.0), EDGE);
    b.solid((cx + 2.0, cy - 2.0, 4.0, 1.0), EDGE);
    b.solid((cx + 7.0, cy - 2.0, 3.0, 1.0), EDGE);
    b.solid((cx, cy, 12.0, 7.0), 0xffffff);
    b.solid((cx + 2.0, cy - 1.0, 4.0, 1.0), 0xffffff);
    b.solid((cx + 7.0, cy - 1.0, 3.0, 1.0), 0xffffff);
    let lit = (now / 300) % 3;
    for k in 0..3u64 {
        let a = if k == lit { 1.0 } else { 0.45 };
        b.rect((cx + 2.0 + 3.5 * k as f32, cy + 3.0, 2.0, 1.0), color, a);
    }
}

pub fn paint(b: &mut Brush, fr: &Frame, spots: &[Spot]) {
    let talkers: Vec<&str> = fr
        .playing
        .iter()
        .flat_map(|p| [p.talk.from.as_str(), p.talk.to.as_str()])
        .filter_map(|w| plan::find(&fr.plan, w).map(|p| p.id.as_str()))
        .collect();
    for (i, p) in fr.plan.people.iter().enumerate() {
        let Role::Deep(kind) = p.role else {
            continue;
        };
        if p.status != St::Working || talkers.contains(&p.id.as_str()) {
            continue;
        }
        if let Some(s) = spots.get(i) {
            thought(b, head(s), p.away, deep_color(kind), fr.now);
        }
    }
}
