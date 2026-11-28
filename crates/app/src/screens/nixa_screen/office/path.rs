use super::plan::{Area, CELL_W, GAP, Plan, R, STRIP_H, distance};
use super::sprite::{self, Face};

pub const MAX: usize = 64;
const BAND: f32 = 12.0;
const SOFA_FRONT: f32 = 25.0;
const SOFA_SIDE: f32 = 44.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Route {
    pub pts: [(f32, f32); MAX],
    pub n: usize,
    pub len: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    Area(usize),
    Cabin,
    Meet,
    Lounge,
}

fn empty() -> Route {
    Route {
        pts: [(0.0, 0.0); MAX],
        n: 0,
        len: 0.0,
    }
}

fn same(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
}

fn push(r: &mut Route, p: (f32, f32)) {
    if r.n > 0 && same(r.pts[r.n - 1], p) {
        return;
    }
    if r.n < MAX {
        r.pts[r.n] = p;
        r.n += 1;
    } else {
        r.pts[MAX - 1] = p;
    }
}

fn inside(r: R, p: (f32, f32)) -> bool {
    p.0 >= r.x && p.0 <= r.x + r.w && p.1 >= r.y && p.1 <= r.y + r.h
}

pub fn room_of(plan: &Plan, p: (f32, f32)) -> Option<Where> {
    if let Some(i) = plan.areas.iter().position(|a| inside(a.rect, p)) {
        return Some(Where::Area(i));
    }
    if inside(plan.cabin, p) {
        Some(Where::Cabin)
    } else if inside(plan.meet, p) {
        Some(Where::Meet)
    } else if inside(plan.lounge, p) {
        Some(Where::Lounge)
    } else {
        None
    }
}

fn starts(plan: &Plan) -> impl Iterator<Item = usize> + '_ {
    (0..plan.areas.len())
        .filter(move |&j| j == 0 || plan.areas[j].rect.y != plan.areas[j - 1].rect.y)
}

fn row_count(plan: &Plan) -> usize {
    starts(plan).count()
}

fn row_of(plan: &Plan, ai: usize) -> usize {
    let y = plan.areas[ai].rect.y;
    starts(plan).filter(|&j| plan.areas[j].rect.y < y).count()
}

fn hall_y(plan: &Plan, i: usize) -> f32 {
    match starts(plan).nth(i) {
        Some(j) => plan.areas[j].rect.y - GAP / 2.0,
        None if plan.areas.is_empty() => STRIP_H + GAP / 2.0,
        None => {
            let bottom = plan
                .areas
                .iter()
                .map(|a| a.rect.y + a.rect.h)
                .fold(0.0, f32::max);
            bottom + GAP / 2.0
        }
    }
}

fn halls(plan: &Plan) -> usize {
    let rows = row_count(plan);
    if plan.narrow { rows + 1 } else { rows.max(1) }
}

fn nearest_hall(plan: &Plan, y: f32) -> usize {
    (0..halls(plan))
        .min_by(|a, b| {
            (hall_y(plan, *a) - y)
                .abs()
                .total_cmp(&(hall_y(plan, *b) - y).abs())
        })
        .unwrap_or(0)
}

fn area_in_row(plan: &Plan, k: usize, x: f32) -> usize {
    let mut best = (0, f32::MAX);
    for (ai, a) in plan.areas.iter().enumerate() {
        if row_of(plan, ai) != k {
            continue;
        }
        let r = a.rect;
        if x >= r.x && x <= r.x + r.w {
            return ai;
        }
        let d = (r.x + r.w / 2.0 - x).abs();
        if d < best.1 {
            best = (ai, d);
        }
    }
    best.0
}

fn lane(a: &Area, x: f32) -> f32 {
    let c = ((x - a.left) / CELL_W).round().clamp(0.0, a.cols as f32);
    a.left + c * CELL_W
}

fn mid(r: R) -> f32 {
    r.x + r.w / 2.0
}

fn clear(a: &Area, y: f32) -> bool {
    !a.cells.iter().any(|&(_, cy)| y >= cy + 4.0 && y <= cy + 54.0)
}

fn rank(plan: &Plan, w: Option<Where>, p: (f32, f32)) -> i32 {
    match w {
        Some(Where::Area(i)) => 2 * row_of(plan, i) as i32 + 1,
        Some(Where::Lounge) if plan.narrow => 2 * row_count(plan) as i32 + 1,
        Some(_) => -1,
        None => 2 * nearest_hall(plan, p.1) as i32,
    }
}

fn hall_for(plan: &Plan, w: Option<Where>, p: (f32, f32), other: i32) -> usize {
    match w {
        Some(Where::Area(i)) => {
            let k = row_of(plan, i);
            if other <= 2 * k as i32 + 1 { k } else { k + 1 }
        }
        Some(Where::Lounge) if plan.narrow => row_count(plan),
        Some(_) => 0,
        None => nearest_hall(plan, p.1),
    }
}

fn chain(plan: &Plan, w: Option<Where>, p: (f32, f32), hall: usize) -> Route {
    let mut r = empty();
    let hy = hall_y(plan, hall);
    push(&mut r, p);
    match w {
        Some(Where::Area(i)) => {
            let a = &plan.areas[i];
            let top = hall == row_of(plan, i);
            let l = lane(a, p.0);
            let band = if top {
                a.rect.y + BAND
            } else {
                a.rect.y + a.rect.h - BAND
            };
            let m = mid(a.rect);
            push(&mut r, (l, p.1));
            push(&mut r, (l, band));
            push(&mut r, (m, band));
            push(&mut r, (m, hy));
        }
        Some(Where::Cabin) | Some(Where::Meet) => {
            let room = if w == Some(Where::Cabin) {
                plan.cabin
            } else {
                plan.meet
            };
            let m = mid(room);
            push(&mut r, (m, p.1));
            push(&mut r, (m, hy));
        }
        Some(Where::Lounge) => match plan.sofa {
            Some(s) if p.1 < s.y + s.h + 4.0 && (p.0 - mid(s)).abs() <= s.w / 2.0 + 6.0 => {
                let front = s.y + SOFA_FRONT;
                let side = if p.0 <= mid(s) {
                    mid(s) - SOFA_SIDE
                } else {
                    mid(s) + SOFA_SIDE
                };
                push(&mut r, (p.0, front));
                push(&mut r, (side, front));
                push(&mut r, (side, hy));
            }
            _ => push(&mut r, (p.0, hy)),
        },
        None => push(&mut r, (p.0, hy)),
    }
    r
}

fn cross_row(plan: &Plan, r: &mut Route, k: usize, x: f32, down: bool) -> f32 {
    let a = &plan.areas[area_in_row(plan, k, x)];
    let m = mid(a.rect);
    let l = lane(a, m);
    let (top, bottom) = (a.rect.y + BAND, a.rect.y + a.rect.h - BAND);
    let (first, second) = if down { (top, bottom) } else { (bottom, top) };
    let (enter, leave) = if down {
        (hall_y(plan, k), hall_y(plan, k + 1))
    } else {
        (hall_y(plan, k + 1), hall_y(plan, k))
    };
    push(r, (m, enter));
    push(r, (m, first));
    push(r, (l, first));
    push(r, (l, second));
    push(r, (m, second));
    push(r, (m, leave));
    m
}

fn traverse(plan: &Plan, r: &mut Route, from: usize, to: usize, x: f32) {
    let mut x = x;
    if from < to {
        for k in from..to {
            x = cross_row(plan, r, k, x, true);
        }
    } else {
        for k in (to..from).rev() {
            x = cross_row(plan, r, k, x, false);
        }
    }
}

fn local(a: &Area, r: &mut Route, from: (f32, f32), to: (f32, f32)) {
    push(r, from);
    if (from.1 - to.1).abs() < 0.5 && clear(a, from.1) {
        push(r, to);
        return;
    }
    let (lf, lt) = (lane(a, from.0), lane(a, to.0));
    let y = if clear(a, to.1) {
        to.1
    } else if clear(a, from.1) {
        from.1
    } else {
        a.rect.y + a.rect.h - BAND
    };
    push(r, (lf, from.1));
    push(r, (lf, y));
    push(r, (lt, y));
    push(r, (lt, to.1));
    push(r, to);
}

fn between(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let dot = (b.0 - a.0) * (c.0 - b.0) + (b.1 - a.1) * (c.1 - b.1);
    cross.abs() < 0.01 && dot >= 0.0
}

fn finish(r: &Route) -> Route {
    let mut out = empty();
    for &p in &r.pts[..r.n] {
        if out.n > 0 && same(out.pts[out.n - 1], p) {
            continue;
        }
        if out.n >= 2 && between(out.pts[out.n - 2], out.pts[out.n - 1], p) {
            out.pts[out.n - 1] = p;
            continue;
        }
        out.pts[out.n] = p;
        out.n += 1;
    }
    out.len = (1..out.n)
        .map(|i| distance(out.pts[i - 1], out.pts[i]))
        .sum();
    out
}

pub fn route(plan: &Plan, from: (f32, f32), to: (f32, f32)) -> Route {
    let mut r = empty();
    let (wf, wt) = (room_of(plan, from), room_of(plan, to));
    match (wf, wt) {
        (Some(Where::Area(a)), Some(Where::Area(b))) if a == b => {
            local(&plan.areas[a], &mut r, from, to);
        }
        _ => {
            let hf = hall_for(plan, wf, from, rank(plan, wt, to));
            let ht = hall_for(plan, wt, to, rank(plan, wf, from));
            let out = chain(plan, wf, from, hf);
            for &p in &out.pts[..out.n] {
                push(&mut r, p);
            }
            let x = out.pts[out.n - 1].0;
            traverse(plan, &mut r, hf, ht, x);
            let back = chain(plan, wt, to, ht);
            for &p in back.pts[..back.n].iter().rev() {
                push(&mut r, p);
            }
        }
    }
    r.pts[r.n - 1] = to;
    finish(&r)
}

pub fn at(r: &Route, d: f32) -> ((f32, f32), Face) {
    if r.n == 0 {
        return ((0.0, 0.0), Face::Down);
    }
    let mut left = d.max(0.0);
    let mut face = Face::Down;
    for i in 1..r.n {
        let (a, b) = (r.pts[i - 1], r.pts[i]);
        let l = distance(a, b);
        face = sprite::face_of(b.0 - a.0, b.1 - a.1);
        if left <= l {
            let k = if l > 0.0 { left / l } else { 1.0 };
            return ((a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k), face);
        }
        left -= l;
    }
    (r.pts[r.n - 1], face)
}

pub fn reversed(r: &Route) -> Route {
    let mut out = *r;
    for i in 0..r.n {
        out.pts[i] = r.pts[r.n - 1 - i];
    }
    out
}
