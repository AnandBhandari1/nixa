use super::bubble;
use super::camera::{self, Shot};
use super::motion;
use super::npc::{self, Npc};
use super::path;
use super::plan::{self, Hang, Person, Plan, Role};
use super::sprite::{self, Act, Arm, Face, Pose, Prop};
use super::sync;
use super::talk;
use super::view::{Spot, perch, seated};
use crate::screens::nixa_screen::map::{Agent, St};
use serde_json::{Value, json};

pub const KIND: &str = "office_soap";
pub const MODELS_KIND: &str = "office_soap_models";
pub const MODEL_KIND: &str = "office_soap_model";
pub const SCENE_EVENT: &str = "office_scene";
pub const PENDING_MS: u64 = 30_000;
pub const REST_MS: u64 = 45_000;
pub const CAST_MAX: usize = 8;
pub const SCENE_CAST_MAX: usize = 3;
pub const WORK_MAX: usize = 6;
pub const TASK_MAX: usize = 60;
pub const TEXT_MAX: usize = 90;
pub const LINES_MAX: usize = 8;
pub const GAP_MS: u64 = 300;
pub const ZOOM_MS: u64 = 1200;
pub const RISE: u64 = talk::SEAT_MS;
pub const MODEL_MAX: usize = 200;
const OFFICE: &str = "office";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Sofa,
    Tea,
    Window,
    Desk,
    Meeting,
}

impl Place {
    pub fn from_word(word: &str) -> Place {
        match word.trim() {
            "sofa" => Place::Sofa,
            "window" => Place::Window,
            "desk" => Place::Desk,
            "meeting" => Place::Meeting,
            _ => Place::Tea,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    None,
    Wave,
    Laugh,
    Nod,
    Sip,
}

impl Cue {
    pub fn from_word(word: &str) -> Cue {
        match word.trim() {
            "wave" => Cue::Wave,
            "laugh" => Cue::Laugh,
            "nod" => Cue::Nod,
            "sip" => Cue::Sip,
            _ => Cue::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoapLine {
    pub who: String,
    pub text: String,
    pub cue: Cue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub place: Place,
    pub cast: Vec<String>,
    pub mood: String,
    pub lines: Vec<SoapLine>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub to: (f32, f32),
    pub pose: Pose,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Actor {
    pub id: String,
    pub from: (f32, f32),
    pub seated: bool,
    pub mark: Mark,
    pub walk: u64,
    pub home: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Show {
    pub scene: Scene,
    pub place: Place,
    pub start: u64,
    pub lines_at: u64,
    pub times: Vec<(u64, u64)>,
    pub cast: Vec<Actor>,
    pub hangs: Vec<usize>,
    pub said: usize,
    pub lead: u64,
    pub open: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Soap {
    pub asked_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub show: Option<Show>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Models {
    pub model: String,
    pub url: String,
    pub models: Vec<String>,
    pub error: Option<String>,
    pub waiting: bool,
    pub known: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    pub office: bool,
    pub open: bool,
    pub on: bool,
    pub up: bool,
    pub idle: bool,
    pub talk: bool,
}

pub fn line_ms(text: &str) -> u64 {
    (700 + 55 * text.chars().count() as u64).clamp(1800, 4800)
}

pub fn cut(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    one.chars().take(max).collect::<String>().trim_end().to_string()
}

pub fn expire(soap: &mut Soap, now: u64) {
    if soap
        .asked_at
        .is_some_and(|a| now.saturating_sub(a) >= PENDING_MS)
    {
        soap.asked_at = None;
        soap.ended_at = Some(now);
    }
}

pub fn should_ask(soap: &Soap, g: Gate, now: u64) -> bool {
    g.office
        && g.open
        && g.on
        && g.up
        && g.idle
        && !g.talk
        && soap.show.is_none()
        && soap
            .asked_at
            .is_none_or(|a| now.saturating_sub(a) >= PENDING_MS)
        && soap
            .ended_at
            .is_none_or(|e| now.saturating_sub(e) >= REST_MS)
}

pub fn is_free(p: &Person) -> bool {
    match p.role {
        Role::Staff => npc::is_free(p),
        Role::Deep(_) | Role::Cto => matches!(p.status, St::Idle | St::Done),
    }
}

fn role_word(p: &Person) -> &'static str {
    match p.role {
        Role::Cto => "cto",
        Role::Deep(_) => "deep",
        Role::Staff => "staff",
    }
}

pub fn at_word(p: &Person, n: Option<&Npc>) -> &'static str {
    if matches!(p.role, Role::Deep(_)) {
        return "meeting";
    }
    match n {
        None => "desk",
        Some(n) if !n.resting => "walk",
        Some(n) => match n.hang {
            Some(Hang::Sofa) => "sofa",
            Some(Hang::Tea) => "tea",
            Some(Hang::Window) => "window",
            Some(Hang::Plant) => "plant",
            Some(Hang::Stroll) => "walk",
            _ => "desk",
        },
    }
}

fn space_of(plan: &Plan, p: &Person) -> String {
    p.area
        .and_then(|a| plan.areas.get(a))
        .map(|a| a.name.clone())
        .unwrap_or_else(|| OFFICE.into())
}

pub fn payload(plan: &Plan, life: &[Option<Npc>], agents: &[Agent], hour: u32, now: u64) -> Option<Value> {
    let free: Vec<usize> = (0..plan.people.len())
        .filter(|&i| is_free(&plan.people[i]))
        .collect();
    if free.len() < 2 {
        return None;
    }
    let skip = (motion::hash(now / 1000, 7) % free.len() as u64) as usize;
    let cast: Vec<Value> = free
        .iter()
        .cycle()
        .skip(skip)
        .take(free.len().min(CAST_MAX))
        .map(|&i| {
            let p = &plan.people[i];
            json!({
                "id": p.id,
                "name": p.short,
                "role": role_word(p),
                "space": space_of(plan, p),
                "status": p.status.word(),
                "at": at_word(p, life.get(i).and_then(Option::as_ref)),
            })
        })
        .collect();
    let work: Vec<Value> = plan
        .people
        .iter()
        .filter(|p| p.role == Role::Staff && p.status == St::Working)
        .filter_map(|p| {
            let a = agents.iter().find(|a| a.id == p.id)?;
            let task = cut(&a.title, TASK_MAX);
            (!task.is_empty()).then(|| json!({"who": p.id, "name": p.short, "task": task}))
        })
        .take(WORK_MAX)
        .collect();
    Some(json!({"cast": cast, "work": work, "hour": hour.min(23)}))
}

pub fn local_hour() -> u32 {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let got = unsafe { libc::localtime_r(&now, &mut tm) };
    if got.is_null() {
        12
    } else {
        tm.tm_hour.clamp(0, 23) as u32
    }
}

pub fn scene_of(msg: &Value, plan: &Plan) -> Option<Scene> {
    let scene = msg.get("scene")?;
    let place = Place::from_word(scene.get("where").and_then(Value::as_str).unwrap_or(""));
    let mut cast: Vec<String> = Vec::new();
    for id in scene.get("cast")?.as_array()?.iter().filter_map(Value::as_str) {
        let ok = plan.people.iter().any(|p| p.id == id && is_free(p));
        if ok && !cast.iter().any(|c| c == id) && cast.len() < SCENE_CAST_MAX {
            cast.push(id.to_string());
        }
    }
    let lines: Vec<SoapLine> = scene
        .get("lines")?
        .as_array()?
        .iter()
        .filter_map(|l| {
            let who = l.get("who").and_then(Value::as_str)?;
            let text = cut(l.get("text").and_then(Value::as_str)?, TEXT_MAX);
            if text.is_empty() || !cast.iter().any(|c| c == who) {
                return None;
            }
            let cue = Cue::from_word(l.get("act").and_then(Value::as_str).unwrap_or(""));
            Some(SoapLine { who: who.to_string(), text, cue })
        })
        .take(LINES_MAX)
        .collect();
    if cast.len() < 2 || lines.is_empty() {
        return None;
    }
    let mood = cut(scene.get("mood").and_then(Value::as_str).unwrap_or(""), 24);
    Some(Scene { place, cast, mood, lines })
}

fn taken(plan: &Plan, life: &[Option<Npc>], cast: &[String]) -> Vec<usize> {
    plan.people
        .iter()
        .zip(life)
        .filter(|(p, _)| !cast.contains(&p.id))
        .filter_map(|(_, n)| n.as_ref().and_then(|n| n.spot).map(|s| s.0))
        .collect()
}

fn stand(at: (f32, f32), face: Face) -> Mark {
    Mark { to: at, pose: Pose::Stand { face } }
}

fn sofa_marks(plan: &Plan, busy: &[usize], n: usize) -> Option<(Vec<Mark>, Vec<usize>)> {
    let seats: Vec<usize> = (0..plan.hangouts.len())
        .filter(|&i| plan.hangouts[i].kind == Hang::Sofa)
        .collect();
    if seats.len() < n || seats.iter().any(|i| busy.contains(i)) {
        return None;
    }
    let marks = seats
        .iter()
        .take(n)
        .map(|&i| Mark { to: plan.hangouts[i].at, pose: Pose::Sofa })
        .collect();
    Some((marks, seats))
}

fn tea_marks(plan: &Plan, busy: &[usize], n: usize) -> Option<(Vec<Mark>, Vec<usize>)> {
    let free = |i: &usize| plan.hangouts[*i].kind == Hang::Tea && !busy.contains(i);
    let pick = (0..plan.hangouts.len())
        .filter(free)
        .min_by_key(|i| plan.hangouts[*i].area.is_some())?;
    let h = plan.hangouts[pick];
    let dir = if h.face == Face::Left { -1.0 } else { 1.0 };
    let spots = [
        plan::slot_at(&h, 0),
        plan::slot_at(&h, 1),
        (h.at.0 - 23.0 * dir, h.at.1),
    ];
    let marks = spots.iter().take(n).map(|&at| stand(at, h.face)).collect();
    Some((marks, vec![pick]))
}

fn window_marks(plan: &Plan, busy: &[usize], n: usize) -> Option<(Vec<Mark>, Vec<usize>)> {
    let list: Vec<usize> = (0..plan.hangouts.len())
        .filter(|i| plan.hangouts[*i].kind == Hang::Window && !busy.contains(i))
        .collect();
    let pick = *list.get(list.len() / 2)?;
    let c = plan.hangouts[pick].at;
    let near: Vec<usize> = (0..plan.hangouts.len())
        .filter(|&i| plan.hangouts[i].kind == Hang::Window && plan::distance(plan.hangouts[i].at, c) < 40.0)
        .collect();
    if near.iter().any(|i| busy.contains(i)) {
        return None;
    }
    let spots = [
        stand((c.0 - 11.0, c.1 + 4.0), Face::Right),
        stand((c.0 + 11.0, c.1 + 4.0), Face::Left),
        stand((c.0, c.1 + 16.0), Face::Up),
    ];
    Some((spots.into_iter().take(n).collect(), near))
}

fn desk_marks(plan: &Plan, cast: &[String]) -> Option<(Vec<Mark>, Vec<String>)> {
    let host = cast
        .iter()
        .filter_map(|id| plan.people.iter().find(|p| &p.id == id))
        .find(|p| p.desk.is_some() && p.role == Role::Staff)
        .or_else(|| {
            cast.iter()
                .filter_map(|id| plan.people.iter().find(|p| &p.id == id))
                .find(|p| p.desk.is_some())
        })?;
    let mut order = vec![host.id.clone()];
    order.extend(cast.iter().filter(|c| **c != host.id).cloned());
    let s = host.stand;
    let step = plan::SIDE_STEP * 1.4;
    let spots = [
        stand(s, Face::Left),
        stand((s.0 - step, s.1), Face::Right),
        stand((s.0 + step, s.1), Face::Left),
    ];
    let marks = spots.into_iter().take(order.len()).collect();
    Some((marks, order))
}

pub fn meeting_marks(plan: &Plan, n: usize) -> Vec<Mark> {
    let t = plan.table;
    let left = t.x - 13.0;
    let right = t.x + t.w + 13.0;
    let mid = t.y + t.h / 2.0 + 8.0;
    let low = t.y + t.h + 34.0;
    let spots = [
        stand((left, mid), Face::Right),
        stand((right, mid), Face::Left),
        stand((left, low), Face::Right),
        stand((right, low), Face::Left),
    ];
    spots.into_iter().take(n).collect()
}

pub fn marks_for(
    place: Place,
    plan: &Plan,
    life: &[Option<Npc>],
    cast: &[String],
) -> (Place, Vec<Mark>, Vec<usize>, Vec<String>) {
    let busy = taken(plan, life, cast);
    let n = cast.len();
    let same = cast.to_vec();
    let chain = match place {
        Place::Sofa => [Place::Sofa, Place::Tea, Place::Meeting],
        Place::Window => [Place::Window, Place::Tea, Place::Meeting],
        Place::Desk => [Place::Desk, Place::Tea, Place::Meeting],
        Place::Tea => [Place::Tea, Place::Window, Place::Meeting],
        Place::Meeting => [Place::Meeting, Place::Meeting, Place::Meeting],
    };
    for p in chain {
        let got = match p {
            Place::Sofa => sofa_marks(plan, &busy, n).map(|(m, h)| (m, h, same.clone())),
            Place::Tea => tea_marks(plan, &busy, n).map(|(m, h)| (m, h, same.clone())),
            Place::Window => window_marks(plan, &busy, n).map(|(m, h)| (m, h, same.clone())),
            Place::Desk => desk_marks(plan, cast).map(|(m, o)| (m, Vec::new(), o)),
            Place::Meeting => Some((meeting_marks(plan, n), Vec::new(), same.clone())),
        };
        if let Some((marks, hangs, order)) = got {
            return (p, marks, hangs, order);
        }
    }
    (Place::Meeting, meeting_marks(plan, n), Vec::new(), same)
}

fn where_now(plan: &Plan, life: &[Option<Npc>], p: &Person) -> ((f32, f32), bool) {
    let n = plan
        .people
        .iter()
        .position(|q| q.id == p.id)
        .and_then(|i| life.get(i).copied().flatten());
    match n {
        Some(n) if !matches!(n.pose, Pose::Sit { .. }) => (n.at, false),
        _ => (p.stand, true),
    }
}

pub fn start(scene: Scene, plan: &Plan, life: &[Option<Npc>], now: u64) -> Option<Show> {
    let (place, marks, hangs, order) = marks_for(scene.place, plan, life, &scene.cast);
    let mut cast = Vec::with_capacity(order.len());
    for (id, mark) in order.iter().zip(marks) {
        let p = plan.people.iter().find(|p| &p.id == id)?;
        let (from, seated) = where_now(plan, life, p);
        let walk = talk::walk_ms(path::route(plan, from, mark.to).len);
        let home = talk::walk_ms(path::route(plan, mark.to, p.stand).len);
        cast.push(Actor { id: id.clone(), from, seated, mark, walk, home });
    }
    if cast.len() < 2 {
        return None;
    }
    let gather = cast
        .iter()
        .map(|c| c.walk + if c.seated { RISE } else { 0 })
        .max()
        .unwrap_or(0);
    let lines_at = gather.max(ZOOM_MS);
    let mut times = Vec::with_capacity(scene.lines.len());
    let mut at = lines_at;
    for line in &scene.lines {
        let len = line_ms(&line.text);
        times.push((at, len));
        at += len + GAP_MS;
    }
    Some(Show { scene, place, start: now, lines_at, times, cast, hangs, said: 0, lead: 0, open: false })
}

pub fn lines_end(show: &Show) -> u64 {
    show.times.last().map(|(s, l)| s + l).unwrap_or(show.lines_at)
}

pub fn back_at(show: &Show) -> u64 {
    lines_end(show) + GAP_MS
}

pub fn over_at(show: &Show) -> u64 {
    back_at(show) + show.cast.iter().map(|c| c.home + RISE).max().unwrap_or(0)
}

pub fn is_over(show: &Show, now: u64) -> bool {
    now.saturating_sub(show.start) >= over_at(show)
}

pub fn in_cast(show: &Show, id: &str) -> bool {
    show.cast.iter().any(|c| c.id == id)
}

pub fn line_at(show: &Show, now: u64) -> Option<(usize, u64)> {
    let t = now.saturating_sub(show.start);
    show.times
        .iter()
        .position(|(s, l)| t >= *s && t < s + l + GAP_MS)
        .map(|i| (i, t - show.times[i].0))
}

pub fn due(show: &mut Show, now: u64) -> Option<(String, String)> {
    let (i, _) = line_at(show, now)?;
    if i < show.said {
        return None;
    }
    show.said = i + 1;
    show.lead = 0;
    show.open = false;
    let line = &show.scene.lines[i];
    Some((line.who.clone(), line.text.clone()))
}

pub fn hold(show: &mut Show, line: &sync::Line, now: u64) {
    let Some((i, _)) = line_at(show, now) else {
        return;
    };
    if i + 1 != show.said || show.scene.lines[i].text != line.text {
        return;
    }
    let begin = show.start + show.times[i].0;
    show.lead = sync::shown_at(line).saturating_sub(begin);
    show.open = sync::open(line, now);
    let len = sync::done_at(line).saturating_sub(begin).max(1);
    let old = show.times[i].1;
    show.times[i].1 = len;
    for t in show.times.iter_mut().skip(i + 1) {
        t.0 = (t.0 + len).saturating_sub(old);
    }
}

fn ramp(t: f32, from: f32, len: f32) -> f32 {
    ((t - from) / len).clamp(0.0, 1.0)
}

pub fn dolly(show: &Show, now: u64) -> f32 {
    let t = now.saturating_sub(show.start) as f32;
    let din = show.lines_at.saturating_sub(ZOOM_MS) as f32;
    let out = back_at(show) as f32;
    let z = ZOOM_MS as f32;
    if t < out {
        motion::in_out_cubic(ramp(t, din, z))
    } else {
        1.0 - motion::in_out_cubic(ramp(t, out, z))
    }
}

pub fn speak_ok(still: bool, mute: bool, up: bool, close: f32) -> bool {
    !still && !mute && up && close > 0.5
}

pub fn shot(show: &Show, w: f32, h: f32) -> Option<Shot> {
    let left = show.cast.iter().map(|c| c.mark.to).min_by(|a, b| a.0.total_cmp(&b.0))?;
    let right = show.cast.iter().map(|c| c.mark.to).max_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(camera::close(left, right, w, h))
}

pub fn held(show: &Show, plan: &Plan, life: &[Option<Npc>]) -> Vec<String> {
    plan.people
        .iter()
        .zip(life)
        .filter(|(p, _)| !in_cast(show, &p.id))
        .filter(|(_, n)| n.and_then(|n| n.spot).is_some_and(|s| show.hangs.contains(&s.0)))
        .map(|(p, _)| p.id.clone())
        .collect()
}

pub fn hold_off(life: &mut [Option<Npc>], plan: &Plan, show: &Show) {
    let ids = held(show, plan, life);
    for (p, n) in plan.people.iter().zip(life.iter_mut()) {
        if ids.contains(&p.id) {
            *n = None;
        }
    }
}

fn mark(ends: &mut Vec<(String, u64)>, id: &str, at: u64) {
    match ends.iter_mut().find(|(i, _)| i == id) {
        Some(e) => e.1 = at,
        None => ends.push((id.to_string(), at)),
    }
}

pub fn finish(soap: &mut Soap, ends: &mut Vec<(String, u64)>, plan: &Plan, now: u64) {
    let Some(show) = soap.show.take() else {
        return;
    };
    let life = npc::life(plan, now, &[]);
    for id in held(&show, plan, &life) {
        mark(ends, &id, now);
    }
    for c in &show.cast {
        mark(ends, &c.id, now);
    }
    soap.ended_at = Some(now);
    soap.asked_at = None;
}

fn lerp(a: (f32, f32), b: (f32, f32), k: f32) -> (f32, f32) {
    let k = motion::out_cubic(k.clamp(0.0, 1.0));
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
}

fn plain(at: (f32, f32), pose: Pose) -> Spot {
    Spot { at, pose, prop: Prop::None, act: Act::default(), sip: 0.0 }
}

fn walking(r: &path::Route, t: u64, total: u64) -> Spot {
    let d = motion::trip(t as f32, total as f32) * r.len;
    let (at, face) = path::at(r, d);
    plain(at, Pose::Walk { face, step: sprite::step_of(d) })
}

fn act_of(show: &Show, c: &Actor, now: u64) -> (Act, Prop, f32) {
    let mut act = Act::default();
    let Some((i, tb)) = line_at(show, now) else {
        return (act, Prop::None, 0.0);
    };
    let line = &show.scene.lines[i];
    let len = show.times[i].1;
    if line.who != c.id {
        if (now + motion::seed(&c.id) % 900) % 900 < 180 {
            act.lift = 1.0;
        }
        return (act, Prop::None, 0.0);
    }
    let n = line.text.chars().count();
    let tl = tb.saturating_sub(show.lead);
    let typing = tb >= show.lead && tb < len && (show.open || bubble::shown_chars(n, tl, sync::timer_ms(&line.text)) < n);
    act.mouth = typing && (now / 110).is_multiple_of(2);
    let mut prop = Prop::None;
    let mut sip = 0.0;
    match line.cue {
        Cue::Wave if tb < 700 => act.arm = Arm::Wave((now / 125).is_multiple_of(2)),
        Cue::Laugh if tb < 1400 => {
            act.mouth = (now / 90).is_multiple_of(2);
            act.body = -1.5 * (tb as f32 / 180.0 * std::f32::consts::PI).sin().abs();
        }
        Cue::Nod if tb < 1500 && (tb / 300).is_multiple_of(2) => act.lift = 1.0,
        Cue::Sip => {
            prop = Prop::Cup { lift: true };
            sip = motion::sip_k(tb, 1400);
        }
        _ => {}
    }
    (act, prop, sip)
}

pub fn spot_of(show: &Show, plan: &Plan, p: &Person, now: u64) -> Option<Spot> {
    let c = show.cast.iter().find(|c| c.id == p.id)?;
    let t = now.saturating_sub(show.start);
    let rise = if c.seated { RISE } else { 0 };
    let back = back_at(show);
    if t < rise {
        return Some(plain(lerp(perch(p), p.stand, t as f32 / rise as f32), Pose::Stand { face: Face::Down }));
    }
    if t < rise + c.walk {
        let r = path::route(plan, c.from, c.mark.to);
        return Some(walking(&r, t - rise, c.walk));
    }
    if t < back {
        let (act, prop, sip) = act_of(show, c, now);
        return Some(Spot { at: c.mark.to, pose: c.mark.pose, prop, act, sip });
    }
    let t2 = t - back;
    if t2 < c.home {
        let r = path::route(plan, c.mark.to, p.stand);
        return Some(walking(&r, t2, c.home));
    }
    if t2 < c.home + RISE {
        let k = (t2 - c.home) as f32 / RISE as f32;
        return Some(plain(lerp(p.stand, perch(p), k), Pose::Stand { face: Face::Down }));
    }
    Some(seated(p, now))
}
