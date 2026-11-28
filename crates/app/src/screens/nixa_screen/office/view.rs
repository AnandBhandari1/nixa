use super::brush::{Brush, hex};
use super::bubble;
use super::call::{self, Call};
use super::leave::{self, Leave};
use super::camera::{self, Cam};
use super::furniture::{self as f, BACKDROP};
use super::plan::{self, Person, Plan, Role};
use super::moment;
use super::motion;
use super::path;
use super::npc::{self, Npc};
use super::social;
use super::soap::{self, Show};
use super::sync;
use super::sprite::{self, Act, Arm, Face, Looks, Pose, Prop};
use super::talk::{self, Phase, Playing};
use super::{Tab, rooms_of, shown_tab};
use crate::screens::nixa_screen::map::{NIXA, St};
use crate::screens::nixa_screen::mode::{Mode, Tone};
use crate::screens::nixa_screen::palette::{palette_of, status_color};
use crate::screens::nixa_screen::prefs::Look;
use crate::screens::nixa_screen::view::NixaScreen;
use crate::screens::nixa_screen::now_ms;
use crate::spaces;
use gpui::prelude::*;
use gpui::{
    AnyElement, Div, Entity, FontWeight, Hsla, MouseButton, SharedString, canvas, div, px, rgb,
    rgba,
};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub at: (f32, f32),
    pub pose: Pose,
    pub prop: Prop,
    pub act: Act,
    pub sip: f32,
}

fn slide(a: (f32, f32), b: (f32, f32), k: f32) -> (f32, f32) {
    let k = motion::out_cubic(k);
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
}

fn spot(at: (f32, f32), pose: Pose) -> Spot {
    Spot {
        at,
        pose,
        prop: Prop::None,
        act: Act::default(),
        sip: 0.0,
    }
}

fn standing(at: (f32, f32)) -> Spot {
    spot(at, Pose::Stand { face: Face::Down })
}

fn walking(r: &path::Route, t: f32, total: f32) -> Spot {
    let d = motion::trip(t, total) * r.len;
    let (at, face) = path::at(r, d);
    spot(
        at,
        Pose::Walk {
            face,
            step: sprite::step_of(d),
        },
    )
}

pub fn perch(p: &Person) -> (f32, f32) {
    (p.seat.0, p.seat.1 + sprite::H - sprite::SIT_TOP)
}

fn face_to(dx: f32) -> Face {
    if dx.abs() > 8.0 {
        sprite::face_of(dx, 0.0)
    } else {
        Face::Down
    }
}

pub fn cto_status(mode: Mode) -> St {
    match mode {
        Mode::Think | Mode::Nixa => St::Working,
        Mode::Off | Mode::Paused => St::Idle,
        _ => St::Done,
    }
}

fn rhythm_t(id: &str, now: u64) -> u64 {
    let s = motion::seed(id);
    let period = 9000 + s % 7000;
    (now + s) % period
}

pub fn rhythm(id: &str, now: u64) -> u8 {
    let t = rhythm_t(id, now);
    if t < 900 {
        0
    } else if t < 2200 {
        1
    } else {
        2
    }
}

pub fn seated(p: &Person, now: u64) -> Spot {
    let mut sitting = spot(
        p.seat,
        Pose::Sit {
            typing: false,
            away: p.away,
        },
    );
    if p.status != St::Working {
        return sitting;
    }
    match rhythm(&p.id, now) {
        0 if !p.away => {
            sitting.prop = Prop::Cup { lift: true };
            sitting.sip = motion::sip_k(rhythm_t(&p.id, now), 900);
        }
        0 => {}
        1 => sitting.act.lift = 1.0,
        _ => {
            sitting.pose = Pose::Sit {
                typing: true,
                away: p.away,
            };
            sitting.act.hands = motion::hands_of(motion::seed(&p.id), now);
        }
    }
    sitting
}

fn from_life(p: &Person, n: &Npc, now: u64) -> Spot {
    let mut s = spot(n.at, n.pose);
    s.prop = n.prop;
    if n.prop == (Prop::Cup { lift: true }) && n.hang == Some(plan::Hang::Tea) {
        let seed = motion::seed(&p.id);
        s.sip = motion::sip_k((now + seed % 3000) % 3000, 700);
    }
    s
}

struct Scene {
    meet: (f32, f32),
    t: u64,
    lead: u64,
    sits: bool,
}

fn scene_of(play: &Playing, from: &Person, meet: (f32, f32), now: u64) -> Scene {
    let sits = plan::distance(play.from_at, from.stand) < 0.5;
    Scene {
        meet,
        t: now.saturating_sub(play.start),
        lead: if sits { talk::SEAT_MS } else { 0 },
        sits,
    }
}

fn walker_spot(play: &Playing, plan: &Plan, from: &Person, to: &Person, sc: &Scene, sitting: Spot) -> Spot {
    let a = play.walk;
    let c = a + play.say1 + play.say2;
    let go = a.saturating_sub(sc.lead).max(1);
    let home = a.saturating_sub(talk::SEAT_MS).max(1);
    let route = || path::route(plan, play.from_at, sc.meet);
    match talk::phase(play, play.start + sc.t) {
        Phase::WalkTo(_) if sc.t < sc.lead => standing(slide(
            perch(from),
            from.stand,
            sc.t as f32 / sc.lead as f32,
        )),
        Phase::WalkTo(_) => walking(&route(), (sc.t - sc.lead) as f32, go as f32),
        Phase::Say1(_) | Phase::Say2(_) => spot(
            sc.meet,
            Pose::Stand {
                face: face_to(to.stand.0 - sc.meet.0),
            },
        ),
        Phase::WalkBack(_) => {
            let t2 = sc.t - c;
            if t2 < home {
                let r = if sc.sits {
                    path::reversed(&route())
                } else {
                    path::route(plan, sc.meet, from.stand)
                };
                walking(&r, t2 as f32, home as f32)
            } else if t2 < home + talk::SEAT_MS {
                standing(slide(from.stand, perch(from), (t2 - home) as f32 / talk::SEAT_MS as f32))
            } else {
                sitting
            }
        }
        Phase::Over => sitting,
    }
}

fn host_spot(play: &Playing, plan: &Plan, to: &Person, sc: &Scene, sitting: Spot) -> Spot {
    let a = play.walk;
    let c = a + play.say1 + play.say2;
    let away = plan::distance(play.to_at, to.stand) > 0.5;
    let up = spot(
        to.stand,
        Pose::Stand {
            face: face_to(sc.meet.0 - to.stand.0),
        },
    );
    let go = a.saturating_sub(sc.lead).max(1);
    let rise = sc.lead + motion::time_of(0.82, go as f32) as u64;
    let seat_ms = talk::SEAT_MS;
    match talk::phase(play, play.start + sc.t) {
        Phase::WalkTo(_) if away => {
            let span = a * 55 / 100;
            if sc.t < span {
                let r = path::route(plan, play.to_at, to.stand);
                walking(&r, sc.t as f32, span as f32)
            } else {
                up
            }
        }
        Phase::WalkTo(_) if sc.t < rise => sitting,
        Phase::WalkTo(_) if sc.t < rise + seat_ms => standing(slide(
            perch(to),
            to.stand,
            (sc.t - rise) as f32 / seat_ms as f32,
        )),
        Phase::WalkTo(_) | Phase::Say1(_) | Phase::Say2(_) => up,
        Phase::WalkBack(_) => {
            let t2 = sc.t - c;
            let back = a.saturating_sub(seat_ms);
            if t2 < back {
                up
            } else if t2 < a {
                standing(slide(to.stand, perch(to), (t2 - back) as f32 / seat_ms as f32))
            } else {
                sitting
            }
        }
        Phase::Over => sitting,
    }
}

fn talk_act(play: &Playing, sc: &Scene, walker: bool, seed: u64, now: u64) -> Act {
    let mut act = Act::default();
    let a = play.walk;
    let b = a + play.say1;
    let (speaking, tb, text) = match talk::phase(play, now) {
        Phase::Say1(_) => (walker, sc.t - a, &play.talk.first),
        Phase::Say2(_) => (!walker, sc.t - b, &play.talk.second),
        _ => return act,
    };
    if walker && matches!(talk::phase(play, now), Phase::Say1(_)) && tb < 500 {
        act.arm = Arm::Wave((now / 125).is_multiple_of(2));
    }
    if speaking {
        let n = text.chars().count();
        let on = tb >= play.lead
            && (play.open || bubble::shown_chars(n, tb - play.lead, sync::timer_ms(text)) < n);
        act.mouth = on && (now / 110).is_multiple_of(2);
        if act.arm == Arm::Down && (now + seed % 1600) % 1600 < 300 {
            act.arm = Arm::Up;
        }
    } else if (now + seed % 900) % 900 < 180 {
        act.lift = 1.0;
    }
    act
}

pub fn spot_of(
    p: &Person,
    playing: Option<&Playing>,
    plan: &Plan,
    now: u64,
    life: Option<&Npc>,
) -> Spot {
    let sitting = life
        .map(|n| from_life(p, n, now))
        .unwrap_or_else(|| seated(p, now));
    let Some(play) = playing else {
        return sitting;
    };
    let (Some(from), Some(to)) = (
        plan::find(plan, &play.talk.from),
        plan::find(plan, &play.talk.to),
    ) else {
        return sitting;
    };
    if p.id != from.id && p.id != to.id {
        return sitting;
    }
    let meet = plan::meet_point(from.stand, to.stand);
    let sc = scene_of(play, from, meet, now);
    let walker = p.id == from.id;
    let mut s = if walker {
        walker_spot(play, plan, from, to, &sc, sitting)
    } else {
        host_spot(play, plan, to, &sc, sitting)
    };
    if s.pose != sitting.pose || s.at != sitting.at {
        s.act = talk_act(play, &sc, walker, motion::seed(&p.id), now);
    }
    s
}

#[derive(Clone)]
pub struct Frame {
    pub plan: Plan,
    pub tab: Tab,
    pub playing: Option<Playing>,
    pub zoom: f32,
    pub bars: f32,
    pub cam: Cam,
    pub now: u64,
    pub time: f32,
    pub nixa: Hsla,
    pub done: Hsla,
    pub blocked: Hsla,
    pub life: Vec<Option<Npc>>,
    pub soap: Option<Show>,
    pub seen: Vec<(String, St, u64)>,
    pub spots: Vec<Spot>,
    pub call: Call,
    pub leave: Leave,
}

pub fn busy_of(screen: &NixaScreen, plan: &Plan) -> Vec<String> {
    screen
        .office
        .now
        .iter()
        .map(|p| &p.talk)
        .flat_map(|t| [t.from.as_str(), t.to.as_str()])
        .filter_map(|w| plan::find(plan, w).map(|p| p.id.clone()))
        .chain(screen.office.soap.show.iter().flat_map(|s| s.cast.iter().map(|c| c.id.clone())))
        .chain(leave::busy(&screen.office.leave))
        .collect()
}

pub fn walker_at(plan: &Plan, play: &Playing, now: u64) -> Option<(f32, f32)> {
    let from = plan::find(plan, &play.talk.from)?;
    let to = plan::find(plan, &play.talk.to)?;
    Some(match talk::phase(play, now) {
        Phase::WalkTo(_) => spot_of(from, Some(play), plan, now, None).at,
        _ => plan::meet_point(from.stand, to.stand),
    })
}

pub fn frame_of(screen: &NixaScreen, w: f32, h: f32, t: &Theme) -> Frame {
    let tab = shown_tab(screen.tab.as_ref(), &screen.rooms);
    let list = rooms_of(&screen.rooms, &tab);
    screen.aspect.set(w / h.max(1.0));
    let mut plan = plan::build(&list, &screen.rooms.loose_deep, screen.feed.name(), w < h);
    let now = now_ms();
    let mode = screen.real_mode(std::time::Instant::now());
    if let Some(cto) = plan.people.iter_mut().find(|p| p.id == NIXA) {
        cto.status = cto_status(mode);
    }
    let busy = busy_of(screen, &plan);
    let busy: Vec<&str> = busy.iter().map(String::as_str).collect();
    let mut life = npc::life_after(&plan, now, &busy, &screen.office.talk_end);
    let ghosts = leave::ghosts(&screen.office.leave, &plan, now);
    plan.people.extend(ghosts);
    let show = screen.office.soap.show.clone();
    if let Some(show) = show.as_ref() {
        soap::hold_off(&mut life, &plan, show);
    }
    let playing = screen.office.now.clone();
    let mut shot = camera::overview(plan.w, plan.h, w, h);
    let mut zoom = 0.0;
    let mut bars = 0.0;
    if let Some(play) = playing.as_ref()
        && !screen.prefs.still
    {
        let cut = talk::cut(play, now);
        let host = plan::find(&plan, &play.talk.to).map(|p| p.stand);
        if let (Some(host), Some(focus)) = (host, walker_at(&plan, play, now))
            && cut.dolly > 0.0
        {
            let back = play.walk + play.say1 + play.say2 + 250;
            let zoom_in = now.saturating_sub(play.start) < back;
            shot = camera::blend(shot, camera::close(focus, host, w, h), cut.dolly, zoom_in);
            zoom = cut.dolly;
        }
        bars = cut.bars;
    }
    if playing.is_none()
        && !screen.prefs.still
        && let Some(show) = show.as_ref()
        && let Some(close) = soap::shot(show, w, h)
    {
        let k = soap::dolly(show, now);
        if k > 0.0 {
            shot = camera::blend(shot, close, k, false);
            zoom = k;
        }
    }
    let k = call::dolly(&screen.office.call, now);
    if k > 0.0
        && !screen.prefs.still
        && let Some(cto) = plan::find(&plan, NIXA)
    {
        let at = perch(cto);
        shot = camera::blend(shot, camera::close(at, at, w, h), k, false);
        zoom = k;
    }
    let mut fr = Frame {
        cam: camera::cam_of(shot, w, h),
        plan,
        tab,
        playing,
        zoom,
        bars,
        now,
        time: screen.time(),
        nixa: palette_of(Tone::Mode(Mode::Nixa), t).main,
        done: status_color(t, St::Done),
        blocked: status_color(t, St::Blocked),
        life,
        soap: show,
        seen: screen.office.seen.clone(),
        spots: Vec::new(),
        call: screen.office.call.clone(),
        leave: screen.office.leave.clone(),
    };
    fr.spots = (0..fr.plan.people.len()).map(|i| spot_in(&fr, i)).collect();
    fr
}

pub fn spot_in(fr: &Frame, i: usize) -> Spot {
    let p = &fr.plan.people[i];
    let life = fr.life.get(i).and_then(Option::as_ref);
    let left = leave::trip_of(&fr.leave, &p.id).and_then(|t| leave::spot(t, &fr.plan, p, fr.now));
    let acted = left.or_else(|| fr.soap.as_ref().and_then(|show| soap::spot_of(show, &fr.plan, p, fr.now)));
    let mut s = acted.unwrap_or_else(|| spot_of(p, fr.playing.as_ref(), &fr.plan, fr.now, life));
    if p.id == NIXA && call::line(&fr.call, fr.now).is_some_and(|l| l.talking) {
        s.act.mouth = (fr.now / 110).is_multiple_of(2);
    }
    let seed = motion::seed(&p.id);
    s.act = motion::merge(motion::act_of(seed, s.pose, fr.now), s.act);
    if let Some(te) = talk::age(&fr.seen, &p.id, fr.now) {
        if p.status == St::Done {
            let (jump, squash) = moment::jump(te);
            s.act.body += jump;
            s.act.squash |= squash;
        }
        if p.status == St::Working && matches!(s.pose, Pose::Sit { .. }) {
            s.at.1 += moment::pull_in(te);
        }
    }
    s
}

fn looks(p: &Person, nixa: Hsla) -> Looks {
    sprite::looks_of(&p.id, p.role, nixa)
}

#[derive(Clone, Copy, Debug)]
enum Draw {
    Set(usize),
    Body(usize),
    Chair(usize),
    Empty(usize, usize),
    Table,
}

fn tint_of(tints: &[(String, Hsla)], id: &str, or: Hsla) -> Hsla {
    tints
        .iter()
        .find(|(a, _)| a == id)
        .map(|(_, c)| *c)
        .unwrap_or(or)
}

fn seen(b: &Brush, (x, y): (f32, f32)) -> bool {
    let (vx, vy, vw, vh) = b.view();
    let (bx, by, bw, bh) = (x - 8.0, y - sprite::H - 10.0, 16.0, sprite::H + 12.0);
    bx + bw >= vx && bx <= vx + vw && by + bh >= vy && by <= vy + vh
}

fn screen_of(p: &Person, sitting: bool) -> f::Screen {
    match p.status {
        St::Working if sitting => f::Screen::Work,
        St::Blocked => f::Screen::Blocked,
        _ => f::Screen::Idle,
    }
}

fn blocked_mark(b: &mut Brush, (x, head): (f32, f32), now: u64) {
    let bounce = if (now / 400).is_multiple_of(2) { 0.0 } else { 1.0 };
    let top = head - 6.0 - 9.0 - bounce;
    b.solid((x - 5.0, top - 1.0, 9.0, 11.0), 0x1d1a24);
    b.solid((x - 2.0, top + 10.0, 3.0, 1.0), 0x1d1a24);
    b.solid((x - 4.0, top, 7.0, 9.0), 0xe0524f);
    b.solid((x - 1.0, top + 9.0, 1.0, 1.0), 0xe0524f);
    b.solid((x - 1.0, top + 2.0, 1.0, 4.0), 0xffffff);
    b.solid((x - 1.0, top + 7.0, 1.0, 1.0), 0xffffff);
}

fn prop(b: &mut Brush, s: &Spot, now: u64) {
    if s.prop == (Prop::Cup { lift: true }) {
        sprite::paint_sip(b, s.at, s.pose, &s.act, s.sip);
    } else {
        sprite::paint_prop(b, s.at, s.pose, s.prop, &s.act, now);
    }
}

fn screen_fade(fr: &Frame, p: &Person) -> f32 {
    match talk::age(&fr.seen, &p.id, fr.now) {
        Some(te) if p.status == St::Working => (te as f32 / 300.0).min(1.0),
        _ => 1.0,
    }
}

fn moments(b: &mut Brush, fr: &Frame, spots: &[Spot]) {
    let colors = [fr.done, fr.nixa, hex(moment::POP[0]), hex(moment::POP[1])];
    for (i, p) in fr.plan.people.iter().enumerate() {
        let s = &spots[i];
        if !seen(b, s.at) {
            continue;
        }
        let head = (s.at.0, sprite::head_y(s.pose, s.at) + s.act.body);
        match p.status {
            St::Done => {
                if let Some(te) = talk::age(&fr.seen, &p.id, fr.now) {
                    moment::paint_done(b, head, te, colors);
                }
            }
            St::Blocked => {
                let mid = (s.at.0, (head.1 + s.at.1) / 2.0);
                moment::paint_ring(b, mid, fr.now, fr.blocked);
            }
            _ => {}
        }
    }
}

pub fn at_desk(p: &Person, d: plan::R, s: &Spot) -> bool {
    !matches!(s.pose, Pose::Sit { .. })
        && (s.at.0 - p.seat.0).abs() < 1.0
        && s.at.1 >= p.seat.1 - 1.0
        && s.at.1 < d.y + d.h
}

fn paint_world(b: &mut Brush, fr: &Frame, tints: &[(String, Hsla)]) {
    let plan = &fr.plan;
    f::floor(b, plan);
    for area in &plan.areas {
        f::area_floor(b, area, tint_of(tints, &area.id, fr.nixa));
    }
    f::strip_floor(b, plan);
    f::sun(b, plan);
    for area in &plan.areas {
        f::area_walls(b, area);
    }
    f::strip(b, plan);
    f::outer_walls(b, plan);
    let n = plan.people.len();
    let mut spots: Vec<Spot> = fr.spots.clone();
    let pairs = social::pairs(fr);
    for pair in &pairs {
        if social::nod(pair, fr.now) {
            spots[pair.listener].act.lift += 1.0;
        }
    }
    if fr.zoom < 0.5 {
        for line in social::lines(fr) {
            let Some(s) = spots.get_mut(line.who) else { continue };
            if social::waves(line.text) {
                s.act.arm = Arm::Wave((fr.now / 125).is_multiple_of(2));
            }
            s.act.mouth = (fr.now / 110).is_multiple_of(2);
        }
    }
    let looks: Vec<Looks> = plan.people.iter().map(|p| looks(p, fr.nixa)).collect();
    let empties: usize = plan.areas.iter().map(|a| a.empty.len()).sum();
    let mut list: Vec<(f32, Draw)> = Vec::with_capacity(n * 2 + empties + 1);
    let mut laptops: Vec<f32> = Vec::with_capacity(n);
    for (i, p) in plan.people.iter().enumerate() {
        let s = &spots[i];
        let sitting = matches!(s.pose, Pose::Sit { .. });
        match p.desk {
            Some(d) => {
                list.push((d.y + d.h, Draw::Set(i)));
                if !sitting && !at_desk(p, d, s) {
                    list.push((s.at.1, Draw::Body(i)));
                }
            }
            None => {
                list.push((s.at.1, Draw::Body(i)));
                if matches!(p.role, Role::Deep(_)) {
                    let nudge = if p.away { 0.1 } else { -0.1 };
                    list.push((p.seat.1 + nudge, Draw::Chair(i)));
                    if !p.away {
                        laptops.push(p.seat.0);
                    }
                }
            }
        }
    }
    for (ai, a) in plan.areas.iter().enumerate() {
        for (k, d) in a.empty.iter().enumerate() {
            list.push((d.y + d.h, Draw::Empty(ai, k)));
        }
    }
    list.push((plan.table.y + plan.table.h, Draw::Table));
    list.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(_, item) in &list {
        match item {
            Draw::Set(i) => {
                let p = &plan.people[i];
                let (s, l) = (&spots[i], &looks[i]);
                let Some(d) = p.desk else { continue };
                let sitting = matches!(s.pose, Pose::Sit { .. });
                f::chair_back(b, p.seat);
                let sip = sitting && s.prop != Prop::None;
                if sitting {
                    sprite::paint(b, s.at, l, s.pose, &s.act, 1.0);
                } else if at_desk(p, d, s) {
                    sprite::paint(b, s.at, l, s.pose, &s.act, 1.0);
                    prop(b, s, fr.now);
                }
                let away = leave::trip_of(&fr.leave, &p.id).is_some_and(|t| leave::gone(t, plan, p, fr.now));
                let screen = if away { f::Screen::Off } else { screen_of(p, sitting) };
                let glow = l.shirt.opacity(screen_fade(fr, p));
                f::desk(b, d, screen, glow, motion::seed(&p.id), fr.now, sip);
                if sip {
                    prop(b, s, fr.now);
                }
                if sitting && !sip && p.status == St::Working {
                    sprite::typing_hands(b, (p.seat.0, d.y + 3.0), l, &s.act);
                }
                if sitting && screen == f::Screen::Work {
                    f::face_light(b, (s.at.0, sprite::head_y(s.pose, s.at)), l.shirt);
                }
            }
            Draw::Body(i) => {
                let s = &spots[i];
                if !seen(b, s.at) {
                    continue;
                }
                sprite::paint(b, s.at, &looks[i], s.pose, &s.act, 1.0);
                prop(b, s, fr.now);
            }
            Draw::Chair(i) => {
                let p = &plan.people[i];
                if p.away {
                    f::deep_chair_front(b, p.seat);
                } else {
                    f::deep_chair_back(b, p.seat);
                }
            }
            Draw::Empty(ai, k) => {
                let d = plan.areas[ai].empty[k];
                f::chair_back(b, (d.x + d.w / 2.0, d.y - 2.0));
                let tint = tint_of(tints, &plan.areas[ai].id, fr.nixa);
                f::desk(b, d, f::Screen::Off, tint, (ai * 31 + k) as u64, fr.now, false);
            }
            Draw::Table => f::meet_table(b, plan, &laptops),
        }
    }
    if fr.zoom <= 0.5 {
        social::paint(b, fr, &spots);
    }
    for (i, p) in plan.people.iter().enumerate() {
        let s = &spots[i];
        if p.status == St::Blocked && seen(b, s.at) {
            blocked_mark(b, (s.at.0, sprite::head_y(s.pose, s.at)), fr.now);
        }
    }
    moments(b, fr, &spots);
}

fn world_layer(fr: &Frame, w: f32, h: f32, t: &Theme) -> gpui::Canvas<()> {
    let fr = fr.clone();
    let tints: Vec<(String, Hsla)> = fr
        .plan
        .areas
        .iter()
        .map(|a| (a.id.clone(), spaces::color(t, a.color)))
        .collect();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let mut b = Brush {
                window,
                o: bounds.origin,
                cam: fr.cam,
                w,
                h,
            };
            b.screen((0.0, 0.0, w, h), hex(BACKDROP), 1.0);
            paint_world(&mut b, &fr, &tints);
            if fr.bars > 0.0 {
                let bar = h * 0.085 * fr.bars;
                b.screen((0.0, 0.0, w, bar), hex(0x000000), 0.9);
                b.screen((0.0, h - bar, w, bar), hex(0x000000), 0.9);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .w(px(w))
    .h(px(h))
}

const TAG_BG: u32 = 0x14171ee8;

fn pill(text: SharedString, dot: Option<(Hsla, f32, f32)>, size: f32) -> Div {
    let body = div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(4.0))
        .bg(rgba(TAG_BG))
        .border_1()
        .border_color(rgba(0xffffff1f))
        .text_size(px(size))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(0xf2f4f8))
        .whitespace_nowrap()
        .when_some(dot, |el, (c, a, d)| {
            el.child(
                div()
                    .size(px(d))
                    .rounded_full()
                    .border_1()
                    .border_color(rgba(TAG_BG))
                    .bg(c.opacity(a)),
            )
        })
        .child(text);
    div()
        .flex()
        .flex_col()
        .items_center()
        .child(body)
        .child(div().w(px(4.0)).h(px(1.0)).bg(rgba(TAG_BG)))
        .child(div().w(px(2.0)).h(px(1.0)).bg(rgba(TAG_BG)))
}

fn tag_dot(p: &Person, t: &Theme, nixa: Hsla) -> Hsla {
    match (p.role, p.status) {
        (Role::Cto, St::Working) => nixa,
        (_, St::Idle) => rgb(0x8b95a3).into(),
        (_, s) => status_color(t, s),
    }
}

fn labels(fr: &Frame, t: &Theme, w: f32) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let show_all = fr.zoom < 0.5;
    let cam = fr.cam;
    let size = (cam.s * 4.2).clamp(8.0, 11.0);
    let tag_size = (cam.s * 3.4).clamp(9.0, 12.0);
    let narrow = w < 420.0 && show_all;
    let crowded = show_all && fr.plan.people.len() > CROWD;
    if show_all {
        let mut room = |x: f32, y: f32, text: String| {
            let (sx, sy) = cam.to_screen(x + 6.0, y + 5.0);
            if sx < w {
                out.push(
                    div()
                        .absolute()
                        .left(px(sx))
                        .top(px(sy))
                        .px(px(6.0))
                        .rounded(px(6.0))
                        .bg(rgba(0xffffffb0))
                        .text_size(px(size))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(0x6b6f78))
                        .whitespace_nowrap()
                        .child(text)
                        .into_any_element(),
                );
            }
        };
        for area in &fr.plan.areas {
            room(area.rect.x, area.rect.y, area.name.clone());
        }
        room(fr.plan.meet.x, fr.plan.meet.y + fr.plan.meet.h - 16.0, "Deep brain room".into());
    }
    let mut talkers: Vec<&str> = fr
        .playing
        .as_ref()
        .map(|p| vec![p.talk.from.as_str(), p.talk.to.as_str()])
        .unwrap_or_default();
    talkers.extend(fr.soap.iter().flat_map(|s| s.cast.iter().map(|c| c.id.as_str())));
    for (i, p) in fr.plan.people.iter().enumerate() {
        let talking = talkers.iter().any(|w| plan::find(&fr.plan, w).is_some_and(|q| q.id == p.id));
        if !show_all && !talking {
            continue;
        }
        let spot = fr.spots[i];
        let head = sprite::head_y(spot.pose, spot.at);
        let (sx, sy) = cam.to_screen(spot.at.0, head);
        let (_, foot) = cam.to_screen(spot.at.0, spot.at.1);
        let tag_h = tag_size * 1.3 + 8.0;
        let key = matches!(p.role, Role::Cto | Role::Deep(_))
            || matches!(p.status, St::Working | St::Blocked)
            || talking;
        if !narrow || key {
            let dot = tag_dot(p, t, fr.nixa);
            let a = match p.status {
                St::Working => 0.6 + 0.4 * (fr.time * 4.0).sin(),
                St::Blocked => moment::blocked_dot(fr.now),
                _ => 1.0,
            };
            let te = talk::age(&fr.seen, &p.id, fr.now);
            let d = moment::dot_size(te, 8.0);
            out.push(
                div()
                    .absolute()
                    .left(px(sx - 80.0))
                    .top(px(sy - tag_h - 4.0))
                    .w(px(160.0))
                    .flex()
                    .justify_center()
                    .child(pill(if crowded { p.short.clone() } else { p.tag.clone() }.into(), Some((dot, a, d)), tag_size))
                    .into_any_element(),
            );
        }
        let hit_w = (sprite::W * cam.s).max(14.0);
        out.push(
            div()
                .id(SharedString::from(format!("office-person-{}", p.id)))
                .debug_selector({
                    let id = format!("office-person-{}", p.id);
                    move || id
                })
                .absolute()
                .left(px(sx - hit_w / 2.0))
                .top(px(sy))
                .w(px(hit_w))
                .h(px((foot - sy).max(14.0)))
                .tooltip(tooltip(p.tip.clone()))
                .into_any_element(),
        );
    }
    out
}

fn head_px(fr: &Frame, i: usize) -> (f32, f32) {
    let spot = fr.spots.get(i).copied().unwrap_or_else(|| spot_in(fr, i));
    let (x, y) = fr.cam.to_screen(spot.at.0, sprite::head_y(spot.pose, spot.at));
    (x, y - 26.0)
}

fn bubbles(fr: &Frame, h: f32) -> Vec<AnyElement> {
    let Some(play) = fr.playing.as_ref() else {
        return Vec::new();
    };
    let (Some(from), Some(to)) = (
        plan::find(&fr.plan, &play.talk.from),
        plan::find(&fr.plan, &play.talk.to),
    ) else {
        return Vec::new();
    };
    let big = fr.zoom > 0.5;
    let t = fr.now.saturating_sub(play.start);
    let a = play.walk;
    let b = a + play.say1;
    let c = b + play.say2;
    let index = |q: &Person| fr.plan.people.iter().position(|x| x.id == q.id).unwrap_or(0);
    let (wh, hh) = (head_px(fr, index(from)), head_px(fr, index(to)));
    let side = if wh.0 < hh.0 { -1.0 } else { 1.0 };
    let first = &play.talk.first;
    let second = &play.talk.second;
    let all = |s: &String| s.chars().count();
    let one = |pop, shown| bubble::bubble(first, shown, wh, h, pop, big, side);
    let two = |pop, shown| bubble::bubble(second, shown, hh, h, pop, big, -side);
    match talk::phase(play, fr.now) {
        Phase::WalkTo(_) if t + 500 >= a => {
            let k = (t + 500 - a) as f32 / 120.0;
            vec![bubble::dots(wh, h, fr.now, k.min(1.0))]
        }
        Phase::Say1(_) if t - a < play.lead => vec![bubble::dots(wh, h, fr.now, 1.0)],
        Phase::Say1(_) => {
            let tb = t - a - play.lead;
            vec![one(bubble::pop_in(tb), bubble::shown_chars(all(first), tb, sync::timer_ms(first)))]
        }
        Phase::Say2(_) if t - b < play.lead => vec![
            one(bubble::dimmed(bubble::pop_in(t - a), t - b), all(first)),
            bubble::dots(hh, h, fr.now, 1.0),
        ],
        Phase::Say2(_) => {
            let tb = t - b - play.lead;
            vec![
                one(bubble::dimmed(bubble::pop_in(t - a), t - b), all(first)),
                two(bubble::pop_in(tb), bubble::shown_chars(all(second), tb, sync::timer_ms(second))),
            ]
        }
        Phase::WalkBack(_) if t - c < 160 => {
            let tw = t - c;
            vec![
                one(bubble::popped_out(bubble::dimmed(bubble::pop_in(t - a), t - b), tw), all(first)),
                two(bubble::popped_out(bubble::pop_in(t - b), tw), all(second)),
            ]
        }
        _ => Vec::new(),
    }
}

fn soap_bubbles(fr: &Frame, h: f32) -> Vec<AnyElement> {
    let Some(show) = fr.soap.as_ref() else {
        return Vec::new();
    };
    let Some((i, tb)) = soap::line_at(show, fr.now) else {
        return Vec::new();
    };
    let big = fr.zoom > 0.5;
    let index = |id: &str| fr.plan.people.iter().position(|p| p.id == id);
    let xs: Vec<f32> = show.cast.iter().map(|c| c.mark.to.0).collect();
    let mid = xs.iter().sum::<f32>() / xs.len().max(1) as f32;
    let one = |k: usize, pop: bubble::Pop, shown: usize| -> Option<AnyElement> {
        let line = show.scene.lines.get(k)?;
        let at = head_px(fr, index(&line.who)?);
        let me = show.cast.iter().find(|c| c.id == line.who).map(|c| c.mark.to.0).unwrap_or(mid);
        let side = if me < mid - 1.0 { -1.0 } else if me > mid + 1.0 { 1.0 } else { 0.0 };
        Some(bubble::bubble(&line.text, shown, at, h, pop, big, side))
    };
    let (_, len) = show.times[i];
    let n = show.scene.lines[i].text.chars().count();
    if tb < show.lead {
        let at = index(&show.scene.lines[i].who).map(|k| head_px(fr, k));
        return at.map(|at| vec![bubble::dots(at, h, fr.now, (tb as f32 / 120.0).min(1.0))]).unwrap_or_default();
    }
    let mut out = Vec::new();
    if i > 0 && tb < len && show.scene.lines[i - 1].who != show.scene.lines[i].who {
        let (ps, _) = show.times[i - 1];
        let prev_n = show.scene.lines[i - 1].text.chars().count();
        let pop = bubble::dimmed(bubble::pop_in(show.times[i].0 - ps), tb);
        out.extend(one(i - 1, pop, prev_n));
    }
    let tl = tb - show.lead;
    let pop = if tb < len {
        bubble::pop_in(tl)
    } else {
        bubble::popped_out(bubble::pop_in(len.saturating_sub(show.lead)), tb - len)
    };
    let reveal = sync::timer_ms(&show.scene.lines[i].text);
    out.extend(one(i, pop, bubble::shown_chars(n, tl, reveal)));
    out
}

fn call_bubbles(fr: &Frame, w: f32, h: f32) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if !call::on(&fr.call, fr.now) {
        return out;
    }
    let nixa = fr.plan.people.iter().position(|p| p.id == NIXA);
    if let Some(i) = nixa {
        let at = head_px(fr, i);
        if let Some(said) = call::line(&fr.call, fr.now) {
            let n = said.text.chars().count();
            let shown = bubble::shown_chars(n, said.age, sync::timer_ms(&said.text));
            out.push(bubble::bubble(&said.text, shown, at, h, bubble::pop_in(said.age), true, 0.0));
        } else if call::thinking(&fr.call, fr.now) {
            out.push(bubble::dots(at, h, fr.now, 1.0));
        }
    }
    if let Some((text, at)) = fr.call.user.as_ref() {
        let age = fr.now.saturating_sub(*at);
        let pop = bubble::pop_in(age);
        out.push(
            div()
                .absolute()
                .left(px(16.0))
                .bottom(px(16.0 + h * 0.09))
                .w(px((w - 32.0).max(40.0)))
                .flex()
                .justify_center()
                .opacity(pop.alpha.clamp(0.0, 1.0))
                .child(
                    div()
                        .max_w(px(420.0))
                        .px(px(12.0))
                        .py(px(7.0))
                        .rounded(px(14.0))
                        .bg(rgba(0x1d2129e8))
                        .text_size(px(13.0))
                        .text_color(rgb(0xffffff))
                        .child(text.clone()),
                )
                .into_any_element(),
        );
    }
    out
}

fn leave_bubbles(fr: &Frame, h: f32) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let big = fr.zoom > 0.5;
    for trip in &fr.leave.trips {
        if let Some((i, p)) = fr.plan.people.iter().enumerate().find(|(_, p)| p.id == trip.id)
            && let Some((text, age)) = leave::said(trip, &p.short, &fr.plan, p, fr.now)
        {
            let n = text.chars().count();
            let shown = bubble::shown_chars(n, age, sync::timer_ms(&text));
            out.push(bubble::bubble(&text, shown, head_px(fr, i), h, bubble::pop_in(age), big, 0.0));
        }
        if let Some((text, age)) = leave::ask_line(trip, fr.now)
            && let Some(k) = fr.plan.people.iter().position(|p| p.id == NIXA)
        {
            let n = text.chars().count();
            let shown = bubble::shown_chars(n, age, sync::timer_ms(&text));
            out.push(bubble::bubble(&text, shown, head_px(fr, k), h, bubble::pop_in(age), big, 0.0));
        }
    }
    out
}

fn chatter(fr: &Frame, h: f32) -> Vec<AnyElement> {
    if fr.zoom >= 0.5 {
        return Vec::new();
    }
    social::lines(fr)
        .into_iter()
        .map(|line| {
            let n = line.text.chars().count();
            let out = (line.age + 160).saturating_sub(social::SAY_MS);
            let pop = bubble::popped_out(bubble::pop_in(line.age), out);
            let shown = bubble::shown_chars(n, line.age, 600);
            bubble::bubble(line.text, shown, head_px(fr, line.who), h, pop, false, 0.0)
        })
        .collect()
}

fn chip(id: String, label: String, on: bool, t: &Theme) -> gpui::Stateful<Div> {
    let hover = if on { t.selected } else { t.hover };
    let fg = t.foreground;
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .flex_none()
        .h(px(22.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .cursor_pointer()
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .when(on, |el| el.bg(t.selected).text_color(t.foreground))
        .when(!on, |el| el.text_color(t.dim))
        .hover(move |s| s.bg(hover).text_color(fg))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label)
}

pub const WINDOW_BUTTONS: f32 = 72.0;
pub const CROWD: usize = 8;

pub fn look_switch(entity: &Entity<NixaScreen>, current: Look, t: &Theme) -> Div {
    let mut row = div()
        .flex()
        .gap(px(2.0))
        .p(px(2.0))
        .rounded(px(6.0))
        .bg(t.surface.opacity(0.92))
        .border_1()
        .border_color(t.border);
    for look in Look::ALL {
        let pick = entity.clone();
        row = row.child(
            chip(format!("nixa-look-{}", look.word()), look.label().into(), look == current, t)
                .tooltip(tooltip(match look {
                    Look::Universe => "Show your agents as a galaxy",
                    Look::Office => "Show your agents as an office",
                }))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    pick.update(cx, |s, cx| s.set_look(look, cx));
                }),
        );
    }
    row
}

fn tool(id: &'static str, which: Icon, on: bool, tip: String, t: &Theme) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex_none()
        .size(px(26.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(t.surface.opacity(0.92))
        .border_1()
        .border_color(t.border)
        .hover(|s| s.bg(gpui::rgba(0xffffff14)))
        .tooltip(tooltip(tip))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            icon(which)
                .size(px(14.0))
                .text_color(if on { t.cyan } else { t.dim }),
        )
}

fn bar(screen: &NixaScreen, entity: &Entity<NixaScreen>, t: &Theme, tab: &Tab, gap: f32) -> Div {
    let still = screen.prefs.still;
    let mute = screen.prefs.mute;
    let e1 = entity.clone();
    let e2 = entity.clone();
    let mut tabs = div()
        .id("office-tabs")
        .debug_selector(|| "office-tabs".into())
        .flex()
        .gap(px(2.0))
        .p(px(2.0))
        .rounded(px(6.0))
        .bg(t.surface.opacity(0.92))
        .border_1()
        .border_color(t.border)
        .overflow_x_scroll();
    let pick = entity.clone();
    tabs = tabs.child(
        chip("office-tab-company".into(), "Company".into(), *tab == Tab::Company, t)
            .tooltip(tooltip("The whole company: every space as one office"))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                pick.update(cx, |s, cx| s.set_tab(Some(Tab::Company), cx));
            }),
    );
    for room in &screen.rooms.list {
        let pick = entity.clone();
        let this = Tab::Space(room.id.clone());
        let on = *tab == this;
        tabs = tabs.child(
            chip(format!("office-tab-{}", room.id), room.name.clone(), on, t)
                .tooltip(tooltip(format!("Only the {} office", room.name)))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    let next = this.clone();
                    pick.update(cx, |s, cx| s.set_tab(Some(next), cx));
                }),
        );
    }
    div()
        .absolute()
        .top(px(8.0))
        .left(px(8.0))
        .right(px(8.0 + gap))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .gap(px(4.0))
                .items_center()
                .pl(px(WINDOW_BUTTONS))
                .child(look_switch(entity, Look::Office, t))
                .child(div().flex_1())
                .child(
                    tool(
                        "office-camera",
                        Icon::Search,
                        !still,
                        if still {
                            "Camera cut is off. Click to zoom in when people talk.".into()
                        } else {
                            "Camera cut is on. Click to keep the top view.".into()
                        },
                        t,
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        e1.update(cx, |s, cx| s.set_still(!still, cx));
                    }),
                )
                .child(
                    tool(
                        "office-voice",
                        if mute { Icon::VolumeOff } else { Icon::Volume },
                        !mute,
                        if mute {
                            "Office voices are off. Click to hear the talks.".into()
                        } else {
                            "Office voices are on in the close-up. Click to mute.".into()
                        },
                        t,
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        e2.update(cx, |s, cx| s.set_mute(!mute, cx));
                    }),
                ),
        )
        .child(div().flex().child(tabs))
}

pub fn scene(
    screen: &NixaScreen,
    entity: &Entity<NixaScreen>,
    (w, h): (f32, f32),
    t: &Theme,
    gap: f32,
) -> gpui::Stateful<Div> {
    let fr = frame_of(screen, w, h, t);
    div()
        .id("nixa-office")
        .debug_selector(|| "nixa-office".into())
        .relative()
        .w(px(w))
        .h(px(h))
        .overflow_hidden()
        .bg(rgb(BACKDROP))
        .child(world_layer(&fr, w, h, t))
        .children(labels(&fr, t, w))
        .children(chatter(&fr, h))
        .children(bubbles(&fr, h))
        .children(soap_bubbles(&fr, h))
        .children(leave_bubbles(&fr, h))
        .children(call_bubbles(&fr, w, h))
        .child(bar(screen, entity, t, &fr.tab, gap))
}
