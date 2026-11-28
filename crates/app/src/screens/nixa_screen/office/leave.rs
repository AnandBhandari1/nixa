use super::path;
use super::plan::{Person, Plan};
use super::sprite::{self, Face, Pose};
use super::talk;
use super::view::{Spot, perch};
use super::motion;
use crate::screens::nixa_screen::map::St;
use serde_json::Value;

pub const LEAVE_EVENT: &str = "agent_leave";
pub const BACK_EVENT: &str = "agent_back";
pub const SAY_MS: u64 = 2600;
pub const ASK_MS: u64 = 2400;
pub const LINE_MAX: usize = 110;
pub const GONE: (f32, f32) = (-10_000.0, -10_000.0);
pub const SEEN_MS: u64 = 120_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trip {
    pub id: String,
    pub name: String,
    pub line: String,
    pub to_name: String,
    pub until: u64,
    pub at: u64,
    pub back: bool,
    pub who: Option<Person>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Leave {
    pub trips: Vec<Trip>,
    pub seen: Vec<(Person, u64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Away {
    pub id: String,
    pub name: String,
    pub until: u64,
}

fn text(msg: &Value, key: &str) -> String {
    let t = msg.get(key).and_then(Value::as_str).unwrap_or("");
    let one = t.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(LINE_MAX).collect()
}

pub fn take(leave: &mut Leave, msg: &Value, back: bool, now: u64) -> bool {
    let id = text(msg, "id");
    if id.is_empty() {
        return false;
    }
    let was_gone = leave.trips.iter().any(|t| t.id == id && !t.back);
    if back && !was_gone {
        return false;
    }
    if !back && was_gone {
        return false;
    }
    let who = leave.seen.iter().find(|(p, _)| p.id == id).map(|(p, _)| ghost(p));
    leave.trips.retain(|t| t.id != id);
    leave.trips.push(Trip {
        who,
        id,
        name: text(msg, "name"),
        line: text(msg, "line"),
        to_name: if back { String::new() } else { text(msg, "to_name") },
        until: msg.get("until").and_then(Value::as_u64).unwrap_or(0),
        at: now,
        back,
    });
    true
}

fn ghost(p: &Person) -> Person {
    Person { desk: None, area: None, away: false, ..p.clone() }
}

pub fn remember(leave: &mut Leave, plan: &Plan, now: u64) {
    for p in &plan.people {
        match leave.seen.iter_mut().find(|(s, _)| s.id == p.id) {
            Some(e) => *e = (p.clone(), now),
            None => leave.seen.push((p.clone(), now)),
        }
    }
    leave.seen.retain(|(_, at)| now.saturating_sub(*at) < SEEN_MS);
    for t in leave.trips.iter_mut() {
        if let Some((p, _)) = leave.seen.iter().find(|(p, _)| p.id == t.id) {
            t.who = Some(ghost(p));
        }
    }
}

pub fn ghosts(leave: &Leave, plan: &Plan, now: u64) -> Vec<Person> {
    leave
        .trips
        .iter()
        .filter(|t| !plan.people.iter().any(|p| p.id == t.id))
        .filter_map(|t| t.who.as_ref().filter(|p| !gone(t, plan, p, now)).cloned())
        .collect()
}

pub fn seed(leave: &mut Leave, away: &[Away]) -> bool {
    let before = leave.trips.clone();
    leave.trips.retain(|t| t.back || away.iter().any(|a| a.id == t.id));
    for a in away {
        if leave.trips.iter().any(|t| t.id == a.id && !t.back) {
            continue;
        }
        leave.trips.retain(|t| t.id != a.id);
        let who = leave.seen.iter().find(|(p, _)| p.id == a.id).map(|(p, _)| ghost(p));
        leave.trips.push(Trip { id: a.id.clone(), name: a.name.clone(), until: a.until, at: 0, who, ..Trip::default() });
    }
    leave.trips != before
}

pub fn away_of(reply: &Value) -> Vec<Away> {
    let now = reply.get("data").unwrap_or(reply);
    now.get("items")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|i| {
                    let id = i.get("terminal").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty())?;
                    let known = i.get("known").and_then(Value::as_bool).unwrap_or(false);
                    let until = if known { i.get("until").and_then(Value::as_u64).unwrap_or(0) } else { 0 };
                    let name = i.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    Some(Away { id: id.to_string(), name, until })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn stale(leave: &mut Leave, msg: &Value, back: bool) -> bool {
    if back {
        let id = text(msg, "id");
        let before = leave.trips.len();
        leave.trips.retain(|t| t.id != id);
        return before != leave.trips.len();
    }
    take(leave, msg, false, 0)
}

pub fn cast_plan(plan: &Plan, leave: &Leave) -> Plan {
    let mut out = plan.clone();
    for p in out.people.iter_mut().filter(|p| leave.trips.iter().any(|t| t.id == p.id)) {
        p.status = St::Blocked;
    }
    out
}

pub fn exit(plan: &Plan) -> (f32, f32) {
    let l = plan.lounge;
    (l.x + 10.0, l.y + l.h - 10.0)
}

fn walk_out(plan: &Plan, p: &Person) -> (path::Route, u64) {
    let r = path::route(plan, p.stand, exit(plan));
    let ms = talk::walk_ms(r.len);
    (r, ms)
}

fn walking(r: &path::Route, t: u64, total: u64) -> Spot {
    let d = motion::trip(t as f32, total as f32) * r.len;
    let (at, face) = path::at(r, d);
    plain(at, Pose::Walk { face, step: sprite::step_of(d) })
}

fn plain(at: (f32, f32), pose: Pose) -> Spot {
    Spot { at, pose, prop: sprite::Prop::None, act: sprite::Act::default(), sip: 0.0 }
}

fn lerp(a: (f32, f32), b: (f32, f32), k: f32) -> (f32, f32) {
    let k = k.clamp(0.0, 1.0);
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
}

pub fn trip_of<'a>(leave: &'a Leave, id: &str) -> Option<&'a Trip> {
    leave.trips.iter().find(|t| t.id == id)
}

pub fn back_ms(plan: &Plan, p: &Person) -> u64 {
    walk_out(plan, p).1 + talk::SEAT_MS
}

pub fn spot(trip: &Trip, plan: &Plan, p: &Person, now: u64) -> Option<Spot> {
    let t = now.saturating_sub(trip.at);
    let (r, walk) = walk_out(plan, p);
    let rise = talk::SEAT_MS;
    let stand = plain(p.stand, Pose::Stand { face: Face::Down });
    if trip.back {
        if t < walk {
            return Some(walking(&path::reversed(&r), t, walk));
        }
        if t < walk + rise {
            let k = (t - walk) as f32 / rise as f32;
            return Some(plain(lerp(p.stand, perch(p), k), Pose::Stand { face: Face::Down }));
        }
        return None;
    }
    if t < rise {
        return Some(plain(lerp(perch(p), p.stand, t as f32 / rise as f32), Pose::Stand { face: Face::Down }));
    }
    if t < rise + SAY_MS {
        return Some(stand);
    }
    if t < rise + SAY_MS + walk {
        return Some(walking(&r, t - rise - SAY_MS, walk));
    }
    Some(plain(GONE, Pose::Stand { face: Face::Down }))
}

pub fn gone(trip: &Trip, plan: &Plan, p: &Person, now: u64) -> bool {
    !trip.back && now.saturating_sub(trip.at) >= talk::SEAT_MS + SAY_MS + walk_out(plan, p).1
}

pub fn said(trip: &Trip, short: &str, plan: &Plan, p: &Person, now: u64) -> Option<(String, u64)> {
    let t = now.saturating_sub(trip.at);
    let name = if trip.name.is_empty() { short } else { trip.name.as_str() };
    let (start, len) = if trip.back {
        (0, back_ms(plan, p) + SAY_MS)
    } else {
        (talk::SEAT_MS, SAY_MS + walk_out(plan, p).1 / 2)
    };
    if t < start || t >= start + len {
        return None;
    }
    let line = match (trip.line.is_empty(), trip.back) {
        (false, _) => trip.line.clone(),
        (true, false) => format!("{name} is on leave."),
        (true, true) => format!("{name} is back."),
    };
    Some((line, t - start))
}

pub fn ask_line(trip: &Trip, now: u64) -> Option<(String, u64)> {
    if trip.back || trip.to_name.is_empty() {
        return None;
    }
    let t = now.saturating_sub(trip.at);
    let start = talk::SEAT_MS + SAY_MS;
    (t >= start && t < start + ASK_MS).then(|| (format!("Asking {}.", trip.to_name), t - start))
}

fn ghost_ms(plan: &Plan, p: &Person) -> u64 {
    talk::SEAT_MS + SAY_MS + walk_out(plan, p).1.max(ASK_MS)
}

pub fn prune(leave: &mut Leave, plan: &Plan, now: u64) {
    leave.trips.retain(|t| {
        let t_age = now.saturating_sub(t.at);
        if let Some(p) = plan.people.iter().find(|p| p.id == t.id) {
            return !t.back || t_age < back_ms(plan, p) + SAY_MS;
        }
        let Some(p) = t.who.as_ref() else {
            return false;
        };
        if t.back {
            return t_age < back_ms(plan, p) + SAY_MS;
        }
        t_age < ghost_ms(plan, p)
    });
}

pub fn busy(leave: &Leave) -> Vec<String> {
    leave.trips.iter().map(|t| t.id.clone()).collect()
}
