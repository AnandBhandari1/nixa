use super::Room;
use super::sprite::Face;
use crate::screens::nixa_screen::map::{Agent, DEEP, Kind, NIXA, St, deep_kind};
use std::collections::BTreeSet;

pub const CELL_W: f32 = 64.0;
pub const CELL_H: f32 = 70.0;
pub const STRIP_H: f32 = 120.0;
pub const CABIN_W: f32 = 150.0;
pub const MEET_W: f32 = 176.0;
pub const CABIN_W_NARROW: f32 = 112.0;
pub const MEET_W_NARROW: f32 = 136.0;
pub const LOUNGE_H: f32 = 96.0;
pub const SOFA_W: f32 = 72.0;
pub const SOFA_H: f32 = 20.0;
pub const SOFA_SEAT: f32 = 17.0;
pub const WINDOW_EVERY: f32 = 56.0;
pub const WINDOW_W: f32 = 28.0;
pub const LOUNGE_MIN: f32 = 150.0;
pub const GAP: f32 = 16.0;
pub const AREA_TOP: f32 = 34.0;
pub const AREA_PAD: f32 = 18.0;
pub const MIN_AREA_W: f32 = 160.0;
pub const SIDE_STEP: f32 = 18.0;
pub const LOUNGE_ROOM: f32 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn t(&self) -> (f32, f32, f32, f32) {
        (self.x, self.y, self.w, self.h)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Cto,
    Deep(Kind),
    Staff,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub id: String,
    pub tag: String,
    pub short: String,
    pub tip: String,
    pub role: Role,
    pub status: St,
    pub seat: (f32, f32),
    pub stand: (f32, f32),
    pub desk: Option<R>,
    pub away: bool,
    pub area: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hang {
    Tea,
    Sofa,
    Stroll,
    Window,
    Plant,
    Visit,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hangout {
    pub at: (f32, f32),
    pub kind: Hang,
    pub area: Option<usize>,
    pub slots: u8,
    pub face: Face,
}

pub fn hangout(at: (f32, f32), kind: Hang, area: Option<usize>, face: Face) -> Hangout {
    let slots = if kind == Hang::Tea { 2 } else { 1 };
    Hangout { at, kind, area, slots, face }
}

pub fn slot_at(h: &Hangout, slot: u8) -> (f32, f32) {
    if h.kind != Hang::Tea {
        return h.at;
    }
    let dx = if slot == 0 { 3.0 } else { -10.0 };
    let dx = if h.face == Face::Left { -dx } else { dx };
    (h.at.0 + dx, h.at.1)
}

pub fn weight(h: Hang) -> u64 {
    match h {
        Hang::Tea | Hang::Sofa => 3,
        Hang::Visit | Hang::Stretch => 2,
        Hang::Window | Hang::Plant | Hang::Stroll => 1,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    pub id: String,
    pub name: String,
    pub color: usize,
    pub rect: R,
    pub empty: Vec<R>,
    pub cols: usize,
    pub left: f32,
    pub cells: Vec<(f32, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub w: f32,
    pub h: f32,
    pub cabin: R,
    pub lounge: R,
    pub meet: R,
    pub table: R,
    pub areas: Vec<Area>,
    pub people: Vec<Person>,
    pub hangouts: Vec<Hangout>,
    pub windows: Vec<f32>,
    pub plants: Vec<((f32, f32), u8)>,
    pub coolers: Vec<(f32, f32)>,
    pub sofa: Option<R>,
    pub narrow: bool,
}

pub fn cell_top(rect: R, row: usize) -> f32 {
    rect.y + AREA_TOP - 18.0 + row as f32 * CELL_H
}

pub fn name_of(agent: &Agent) -> String {
    match agent.persona.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => name.to_string(),
        None => short_tag(agent),
    }
}

pub fn short_tag(agent: &Agent) -> String {
    let program = agent.program.trim();
    let program = if program.is_empty() { "agent" } else { program };
    let base = match agent.number {
        Some(n) => format!("{program} {n}"),
        None => program.to_string(),
    };
    match agent.persona.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => format!("{name} · {base}"),
        None => base,
    }
}

fn cols_for(n: usize) -> usize {
    ((n as f32).sqrt().ceil() as usize).clamp(2, 5)
}

fn area_size(n: usize) -> (f32, f32) {
    let cols = cols_for(n);
    let rows = n.div_ceil(cols).max(1);
    (
        (cols as f32 * CELL_W + AREA_PAD * 2.0).max(MIN_AREA_W),
        rows as f32 * CELL_H + AREA_TOP + 12.0,
    )
}

pub fn desk_at(cell_x: f32, cell_y: f32) -> (R, (f32, f32), (f32, f32)) {
    let cx = cell_x + CELL_W / 2.0;
    let desk = R {
        x: cx - 22.0,
        y: cell_y + 32.0,
        w: 44.0,
        h: 16.0,
    };
    (desk, (cx, cell_y + 30.0), (cx, cell_y + 64.0))
}

fn place_room(room: &Room, rect: R, index: usize, people: &mut Vec<Person>) -> Area {
    let n = room.agents.len();
    let cols = cols_for(n);
    let used = cols as f32 * CELL_W;
    let left = rect.x + (rect.w - used) / 2.0;
    let mut empty = Vec::new();
    let count = n.max(2);
    let mut cells = Vec::with_capacity(count);
    for i in 0..count {
        let col = (i % cols) as f32;
        let cell = (left + col * CELL_W, cell_top(rect, i / cols));
        cells.push(cell);
        let (desk, seat, stand) = desk_at(cell.0, cell.1);
        let Some(agent) = room.agents.get(i) else {
            empty.push(desk);
            continue;
        };
        people.push(Person {
            id: agent.id.clone(),
            tag: short_tag(agent),
            short: name_of(agent),
            tip: format!("{} · {}", agent.label(), agent.status.word()),
            role: Role::Staff,
            status: agent.status,
            seat,
            stand,
            desk: Some(desk),
            away: false,
            area: Some(index),
        });
    }
    Area {
        id: room.id.clone(),
        name: room.name.clone(),
        color: room.color,
        rect,
        empty,
        cols,
        left,
        cells,
    }
}

fn rows_of(sizes: &[(f32, f32)], narrow: bool) -> Vec<Vec<usize>> {
    let per = match sizes.len() {
        _ if narrow => 1,
        0 | 1 => 1,
        2..=4 => 2,
        _ => 3,
    };
    (0..sizes.len())
        .collect::<Vec<_>>()
        .chunks(per)
        .map(|c| c.to_vec())
        .collect()
}


pub fn build(rooms: &[&Room], loose_deep: &[Agent], nixa_name: &str, narrow: bool) -> Plan {
    let sizes: Vec<(f32, f32)> = rooms.iter().map(|r| area_size(r.agents.len())).collect();
    let rows = rows_of(&sizes, narrow);
    let (cabin_w, meet_w, lounge_min) = if narrow {
        (CABIN_W_NARROW, MEET_W_NARROW, 0.0)
    } else {
        (CABIN_W, MEET_W, LOUNGE_MIN)
    };
    let row_w = |row: &Vec<usize>| {
        row.iter().map(|i| sizes[*i].0).sum::<f32>() + GAP * (row.len().saturating_sub(1)) as f32
    };
    let w = rows
        .iter()
        .map(row_w)
        .fold(cabin_w + meet_w + lounge_min, f32::max);
    let mut people = Vec::new();
    let mut areas = Vec::new();
    let mut y = STRIP_H + GAP;
    for row in &rows {
        let extra = (w - row_w(row)) / row.len() as f32;
        let tall = row.iter().map(|i| sizes[*i].1).fold(0.0, f32::max);
        let mut x = 0.0;
        for i in row {
            let rect = R {
                x,
                y,
                w: sizes[*i].0 + extra,
                h: tall,
            };
            let index = areas.len();
            areas.push(place_room(rooms[*i], rect, index, &mut people));
            x += rect.w + GAP;
        }
        y += tall + GAP;
    }
    let cabin = R {
        x: 0.0,
        y: 0.0,
        w: cabin_w,
        h: STRIP_H,
    };
    let meet = R {
        x: w - meet_w,
        y: 0.0,
        w: meet_w,
        h: STRIP_H,
    };
    let (lounge, h) = if narrow {
        let lounge = R {
            x: 0.0,
            y,
            w,
            h: LOUNGE_H,
        };
        (lounge, y + LOUNGE_H)
    } else {
        let lounge = R {
            x: CABIN_W,
            y: 0.0,
            w: w - CABIN_W - MEET_W,
            h: STRIP_H,
        };
        (lounge, (y - GAP).max(STRIP_H + 60.0))
    };
    let table = R {
        x: meet.x + 26.0,
        y: 38.0,
        w: meet_w - 52.0,
        h: 24.0,
    };
    let (desk, seat, stand) = desk_at(cabin.x + (cabin_w - CELL_W) / 2.0, 22.0);
    people.push(Person {
        id: NIXA.into(),
        tag: nixa_name.to_string(),
        short: nixa_name.to_string(),
        tip: format!("{nixa_name} · the CTO of this office"),
        role: Role::Cto,
        status: St::Idle,
        seat,
        stand,
        desk: Some(desk),
        away: false,
        area: None,
    });
    let mut seen = BTreeSet::new();
    let deep: Vec<&Agent> = loose_deep
        .iter()
        .chain(rooms.iter().flat_map(|r| r.deep.iter()))
        .filter(|a| seen.insert(a.id.clone()))
        .collect();
    let n = deep.len();
    let bottom_seat = table.y + table.h + 20.0;
    for (i, agent) in deep.into_iter().enumerate() {
        let top = i % 2 == 0;
        let slot = (i / 2) as f32;
        let across = n.div_ceil(2).max(1) as f32;
        let x = table.x + table.w * (slot + 0.5) / across;
        let seat = if top {
            (x, table.y + 2.0)
        } else {
            (x, bottom_seat)
        };
        let kind = deep_kind(agent);
        people.push(Person {
            id: agent.id.clone(),
            tag: format!("Deep · {}", short_tag(agent)),
            short: name_of(agent),
            tip: format!("Deep brain · {} · {}", agent.label(), agent.status.word()),
            role: Role::Deep(kind),
            status: agent.status,
            seat,
            stand: (x, bottom_seat + 12.0),
            desk: None,
            away: !top,
            area: None,
        });
    }
    let mut hangouts = Vec::new();
    let mut plants = vec![
        ((cabin.x + cabin.w - 14.0, cabin.y + 24.0), 0),
        ((meet.x + meet.w - 14.0, meet.y + 24.0), 1),
    ];
    let mut coolers = Vec::new();
    for (i, a) in areas.iter().enumerate() {
        let r = a.rect;
        coolers.push((r.x + r.w - 14.0, r.y + r.h - 8.0));
        let green = (r.x + r.w - 12.0, r.y + 22.0);
        plants.push((green, (i % 3) as u8));
        plants.push(((r.x + 12.0, r.y + r.h - 8.0), ((i + 2) % 3) as u8));
        let centre = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let here = Some(i);
        hangouts.push(hangout((r.x + r.w - 28.0, r.y + r.h - 9.0), Hang::Tea, here, Face::Right));
        hangouts.push(hangout((r.x + r.w / 2.0, r.y - GAP / 2.0 + 4.0), Hang::Stroll, here, Face::Up));
        hangouts.push(hangout((green.0 - 10.0, green.1), Hang::Plant, here, Face::Right));
        hangouts.push(hangout(centre, Hang::Visit, here, Face::Down));
        hangouts.push(hangout(centre, Hang::Stretch, here, Face::Down));
    }
    let mut sofa = None;
    if lounge.w >= LOUNGE_ROOM {
        let mid = lounge.x + lounge.w / 2.0;
        let s = R {
            x: mid - SOFA_W / 2.0,
            y: lounge.y + 14.0,
            w: SOFA_W,
            h: SOFA_H,
        };
        sofa = Some(s);
        let cooler = (lounge.x + 16.0, lounge.y + lounge.h - 24.0);
        coolers.push(cooler);
        plants.push(((lounge.x + lounge.w - 14.0, lounge.y + 30.0), 1));
        plants.push(((lounge.x + lounge.w - 14.0, lounge.y + lounge.h - 10.0), 2));
        hangouts.push(hangout((cooler.0 + 13.0, cooler.1 + 3.0), Hang::Tea, None, Face::Left));
        for dx in [-16.0, 0.0, 16.0] {
            hangouts.push(hangout((mid + dx, s.y + SOFA_SEAT), Hang::Sofa, None, Face::Down));
        }
    }
    let mut windows = Vec::new();
    let mut wx = 20.0;
    while wx + WINDOW_W <= w - 8.0 {
        windows.push(wx);
        wx += WINDOW_EVERY;
    }
    for &x in &windows {
        let c = x + WINDOW_W / 2.0;
        let open = if narrow {
            c >= cabin.x + cabin.w + 10.0 && c <= meet.x - 10.0
        } else {
            let clear = sofa.is_none_or(|s| (c - (s.x + s.w / 2.0)).abs() > 44.0);
            clear && c >= lounge.x + 30.0 && c <= lounge.x + lounge.w - 30.0
        };
        if open {
            hangouts.push(hangout((c, 14.0), Hang::Window, None, Face::Up));
        }
    }
    Plan {
        w,
        h,
        cabin,
        lounge,
        meet,
        table,
        areas,
        people,
        hangouts,
        windows,
        plants,
        coolers,
        sofa,
        narrow,
    }
}

pub fn find<'a>(plan: &'a Plan, who: &str) -> Option<&'a Person> {
    if who == DEEP {
        let deep = || plan.people.iter().filter(|p| matches!(p.role, Role::Deep(_)));
        return deep()
            .find(|p| p.status == St::Working)
            .or_else(|| deep().next());
    }
    plan.people.iter().find(|p| p.id == who)
}

pub fn meet_point(walker: (f32, f32), host: (f32, f32)) -> (f32, f32) {
    let side = if walker.0 <= host.0 { -1.0 } else { 1.0 };
    (host.0 + side * SIDE_STEP * 1.4, host.1)
}

pub fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}
