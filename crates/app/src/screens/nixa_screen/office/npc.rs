use super::motion::{self, seed};
use super::path;
use super::plan::{self, GAP, Hang, Person, Plan, Role};
use super::sprite::{self, Face, Pose, Prop};
use super::talk;
use super::view::perch;
use crate::screens::nixa_screen::map::St;

pub const STROLL_SPEED: f32 = 28.0;
pub const CTO_SPEED: f32 = 24.0;
pub const CTO_PERIOD: u64 = 70_000;
pub const CTO_STAY: u64 = 3000;
pub const STRETCH_MS: u64 = 2800;
const WALK_LOW: u64 = 1500;
const WALK_HIGH: u64 = 12_000;
pub const RISE: u64 = talk::SEAT_MS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Npc {
    pub at: (f32, f32),
    pub pose: Pose,
    pub prop: Prop,
    pub hang: Option<Hang>,
    pub resting: bool,
    pub spot: Option<(usize, u8)>,
    pub host: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trip {
    pub hang: usize,
    pub slot: u8,
    pub start: i64,
    pub rise: u64,
    pub walk: u64,
    pub stay: u64,
    pub to: (f32, f32),
    pub face: Face,
    pub host: Option<usize>,
}

type Memo = Vec<(usize, i64, Option<Trip>)>;

pub fn is_free(p: &Person) -> bool {
    p.role == Role::Staff && matches!(p.status, St::Idle | St::Done)
}

fn end(t: &Trip) -> i64 {
    t.start + (2 * t.rise + 2 * t.walk + t.stay) as i64
}

fn period_of(s: u64) -> u64 {
    35_000 + s % 40_000
}

fn round_at(s: u64, t: i64) -> i64 {
    let period = period_of(s);
    (t + (s % period) as i64).div_euclid(period as i64)
}

fn round_end(s: u64, round: i64) -> i64 {
    let period = period_of(s);
    (round + 1) * period as i64 - (s % period) as i64
}

fn stay_of(kind: Hang, s: u64, round: i64) -> u64 {
    let u = motion::hash(s, round as u64 ^ 0x5bd1);
    match kind {
        Hang::Tea => 6000 + u % 3001,
        Hang::Sofa => 9000 + u % 5001,
        Hang::Window | Hang::Stroll => 4000 + u % 2001,
        Hang::Plant => 4000,
        Hang::Visit => 5000 + u % 2001,
        Hang::Stretch => STRETCH_MS,
    }
}

fn open_to(p: &Person, h: &plan::Hangout) -> bool {
    h.area.is_none() || h.area == p.area
}

fn pick_n(p: &Person, plan: &Plan, round: i64, n: u64) -> Option<usize> {
    let total: u64 = plan
        .hangouts
        .iter()
        .filter(|h| open_to(p, h))
        .map(|h| plan::weight(h.kind))
        .sum();
    if total == 0 {
        return None;
    }
    let s = seed(&p.id) ^ (round as u64).wrapping_mul(2_654_435_761);
    let mut r = motion::hash(s, n) % total;
    for (i, h) in plan.hangouts.iter().enumerate() {
        if !open_to(p, h) {
            continue;
        }
        let w = plan::weight(h.kind);
        if r < w {
            return Some(i);
        }
        r -= w;
    }
    None
}

fn host_of(p: &Person, plan: &Plan, round: i64) -> Option<usize> {
    let mate = |q: &Person| q.role == Role::Staff && q.area == p.area && q.id != p.id;
    let count = plan.people.iter().filter(|q| mate(q)).count() as u64;
    if count == 0 {
        return None;
    }
    let k = (motion::hash(seed(&p.id), round as u64 ^ 0x77) % count) as usize;
    plan.people
        .iter()
        .enumerate()
        .filter(|(_, q)| mate(q))
        .nth(k)
        .filter(|(_, q)| q.status == St::Working)
        .map(|(i, _)| i)
}

fn walk_ms(len: f32, speed: f32) -> u64 {
    ((len / speed * 1000.0) as u64).clamp(WALK_LOW, WALK_HIGH)
}

fn trip_with(p: &Person, plan: &Plan, round: i64, hang: usize, slot: u8) -> Option<Trip> {
    let h = plan.hangouts.get(hang)?;
    let s = seed(&p.id);
    let mut host = None;
    let (to, face) = match h.kind {
        Hang::Stretch => (p.seat, Face::Down),
        Hang::Visit => {
            let hi = host_of(p, plan, round)?;
            host = Some(hi);
            let hs = plan.people[hi].stand;
            let to = plan::meet_point(p.stand, hs);
            (to, sprite::face_of(hs.0 - to.0, 0.0))
        }
        _ => (plan::slot_at(h, slot), h.face),
    };
    let walk = if h.kind == Hang::Stretch {
        0
    } else {
        walk_ms(path::route(plan, p.stand, to).len, STROLL_SPEED)
    };
    let stay = stay_of(h.kind, s, round);
    let rise = if h.kind == Hang::Stretch { 0 } else { RISE };
    let total = 2 * rise + 2 * walk + stay;
    if total >= period_of(s) {
        return None;
    }
    Some(Trip {
        hang,
        slot,
        start: round_end(s, round) - total as i64,
        rise,
        walk,
        stay,
        to,
        face,
        host,
    })
}

#[cfg(test)]
pub fn trip_of(p: &Person, plan: &Plan, round: i64) -> Option<Trip> {
    trip_with(p, plan, round, pick_n(p, plan, round, 0)?, 0)
}

fn candidates(p: &Person, plan: &Plan, round: i64) -> ([(usize, u8); 9], usize) {
    let mut out = [(0, 0); 9];
    let mut n = 0;
    let mut add = |c: (usize, u8)| {
        if n < out.len() && !out[..n].contains(&c) {
            out[n] = c;
            n += 1;
        }
    };
    for k in 0..3 {
        let Some(i) = pick_n(p, plan, round, k) else {
            continue;
        };
        let h = &plan.hangouts[i];
        for slot in 0..h.slots {
            add((i, slot));
        }
        if k == 0 && h.kind == Hang::Sofa {
            for j in [i.wrapping_sub(1), i + 1] {
                if plan.hangouts.get(j).is_some_and(|o| o.kind == Hang::Sofa) {
                    add((j, 0));
                }
            }
        }
    }
    (out, n)
}

fn higher(plan: &Plan, q: usize, i: usize) -> bool {
    let (a, b) = (seed(&plan.people[q].id), seed(&plan.people[i].id));
    a < b || (a == b && q < i)
}

fn clashes(plan: &Plan, i: usize, t: &Trip, memo: &mut Memo) -> bool {
    if plan.hangouts[t.hang].kind == Hang::Stretch {
        return false;
    }
    for q in 0..plan.people.len() {
        let other = &plan.people[q];
        if q == i || !is_free(other) || !higher(plan, q, i) {
            continue;
        }
        let sq = seed(&other.id);
        for r in round_at(sq, t.start)..=round_at(sq, end(t)) {
            let (list, n) = candidates(other, plan, r);
            if !list[..n].contains(&(t.hang, t.slot)) {
                continue;
            }
            if let Some(u) = resolve(plan, q, r, memo)
                && u.hang == t.hang
                && u.slot == t.slot
                && u.start < end(t)
                && t.start < end(&u)
            {
                return true;
            }
        }
    }
    false
}

fn resolve(plan: &Plan, i: usize, round: i64, memo: &mut Memo) -> Option<Trip> {
    if let Some(m) = memo.iter().find(|m| m.0 == i && m.1 == round) {
        return m.2;
    }
    let p = &plan.people[i];
    let (list, n) = candidates(p, plan, round);
    let mut out = None;
    for &(hang, slot) in &list[..n] {
        let Some(t) = trip_with(p, plan, round, hang, slot) else {
            continue;
        };
        if !clashes(plan, i, &t, memo) {
            out = Some(t);
            break;
        }
    }
    memo.push((i, round, out));
    out
}

fn stretch_pose(k: u64) -> Pose {
    match k {
        400..1200 | 1600..2400 => Pose::Stretch,
        _ => Pose::Stand { face: Face::Down },
    }
}

fn walker(r: &path::Route, k: u64, walk: u64) -> ((f32, f32), Pose) {
    let d = motion::trip(k as f32, walk as f32) * r.len;
    let (at, face) = path::at(r, d);
    (
        at,
        Pose::Walk {
            face,
            step: sprite::step_of(d),
        },
    )
}

fn lerp(a: (f32, f32), b: (f32, f32), k: f32) -> (f32, f32) {
    let k = k.clamp(0.0, 1.0);
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
}

pub fn rise_at(p: &Person, k: u64, len: u64, rise: u64) -> Option<(f32, f32)> {
    if rise == 0 {
        return None;
    }
    if k < rise {
        return Some(lerp(perch(p), p.stand, k as f32 / rise as f32));
    }
    let back = len.saturating_sub(rise);
    (k >= back).then(|| lerp(p.stand, perch(p), (k - back) as f32 / rise as f32))
}

fn npc_at(p: &Person, plan: &Plan, t: &Trip, now: u64) -> Option<Npc> {
    let k = now as i64 - t.start;
    if k < 0 || k >= end(t) - t.start {
        return None;
    }
    let k = k as u64;
    let h = plan.hangouts[t.hang];
    let base = Npc {
        at: t.to,
        pose: Pose::Stand { face: t.face },
        prop: Prop::None,
        hang: Some(h.kind),
        resting: false,
        spot: Some((t.hang, t.slot)),
        host: t.host,
    };
    if h.kind == Hang::Stretch {
        return Some(Npc {
            at: (p.seat.0, p.seat.1 + 2.0),
            pose: stretch_pose(k),
            ..base
        });
    }
    if let Some(at) = rise_at(p, k, (end(t) - t.start) as u64, t.rise) {
        return Some(Npc {
            at,
            pose: Pose::Stand { face: Face::Down },
            ..base
        });
    }
    let k = k - t.rise;
    let r = path::route(plan, p.stand, t.to);
    let s = seed(&p.id);
    if k < t.walk {
        let (at, pose) = walker(&r, k, t.walk);
        return Some(Npc { at, pose, ..base });
    }
    if k < t.walk + t.stay {
        let (pose, prop) = match h.kind {
            Hang::Sofa => (Pose::Sofa, Prop::None),
            Hang::Tea => (
                Pose::Stand { face: t.face },
                Prop::Cup {
                    lift: (now + s % 3000) % 3000 < 700,
                },
            ),
            Hang::Plant => (Pose::Stand { face: t.face }, Prop::Can),
            _ => (Pose::Stand { face: t.face }, Prop::None),
        };
        return Some(Npc {
            pose,
            prop,
            resting: true,
            ..base
        });
    }
    let (at, pose) = walker(&path::reversed(&r), k - t.walk - t.stay, t.walk);
    let prop = if h.kind == Hang::Tea {
        Prop::Cup { lift: false }
    } else {
        Prop::None
    };
    Some(Npc {
        at,
        pose,
        prop,
        ..base
    })
}

pub fn cto_target(plan: &Plan, round: u64) -> (f32, f32) {
    if plan.areas.is_empty() {
        let l = plan.lounge;
        return (l.x + l.w / 2.0, l.y + l.h - 20.0);
    }
    let r = plan.areas[(round % plan.areas.len() as u64) as usize].rect;
    (r.x + r.w / 2.0 - 24.0, r.y - GAP / 2.0)
}

fn cto(p: &Person, plan: &Plan, now: u64, after: Option<u64>) -> Option<Npc> {
    if !matches!(p.status, St::Idle | St::Done) {
        return None;
    }
    let round = now / CTO_PERIOD;
    let to = cto_target(plan, round);
    let r = path::route(plan, p.stand, to);
    let walk = ((r.len / CTO_SPEED * 1000.0) as u64).max(WALK_LOW);
    let total = 2 * RISE + 2 * walk + CTO_STAY;
    if total >= CTO_PERIOD {
        return None;
    }
    let t = now % CTO_PERIOD;
    if t < CTO_PERIOD - total {
        return None;
    }
    let k = t - (CTO_PERIOD - total);
    if after.is_some_and(|e| now - k < e) {
        return None;
    }
    let base = Npc {
        at: to,
        pose: Pose::Stand { face: Face::Down },
        prop: Prop::None,
        hang: None,
        resting: false,
        spot: None,
        host: None,
    };
    if let Some(at) = rise_at(p, k, total, RISE) {
        return Some(Npc {
            at,
            pose: Pose::Stand { face: Face::Down },
            ..base
        });
    }
    let k = k - RISE;
    Some(if k < walk {
        let (at, pose) = walker(&r, k, walk);
        Npc { at, pose, ..base }
    } else if k < walk + CTO_STAY {
        let face = if k - walk < CTO_STAY / 2 { Face::Down } else { Face::Up };
        Npc {
            pose: Pose::Stand { face },
            ..base
        }
    } else {
        let (at, pose) = walker(&path::reversed(&r), k - walk - CTO_STAY, walk);
        Npc { at, pose, ..base }
    })
}

pub fn life(plan: &Plan, now: u64, busy: &[&str]) -> Vec<Option<Npc>> {
    life_after(plan, now, busy, &[])
}

pub fn last_end(ends: &[(String, u64)], id: &str) -> Option<u64> {
    ends.iter().find(|(i, _)| i == id).map(|(_, t)| *t)
}

pub fn life_after(plan: &Plan, now: u64, busy: &[&str], ends: &[(String, u64)]) -> Vec<Option<Npc>> {
    let mut out = Vec::with_capacity(plan.people.len());
    let mut memo: Memo = Vec::new();
    let is_busy = |id: &str| busy.contains(&id);
    for (i, p) in plan.people.iter().enumerate() {
        let after = last_end(ends, &p.id);
        let n = if is_busy(&p.id) {
            None
        } else if p.role == Role::Cto {
            cto(p, plan, now, after)
        } else if is_free(p) {
            let round = round_at(seed(&p.id), now as i64);
            resolve(plan, i, round, &mut memo)
                .filter(|t| t.host.is_none_or(|h| !is_busy(&plan.people[h].id)))
                .filter(|t| after.is_none_or(|e| t.start >= e as i64))
                .and_then(|t| npc_at(p, plan, &t, now))
        } else {
            None
        };
        out.push(n);
    }
    out
}
