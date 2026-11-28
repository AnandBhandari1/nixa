use super::brush::{Brush, hex};
use super::plan::{Area, CELL_W, Plan, R, WINDOW_W};
use gpui::Hsla;

pub const BACKDROP: u32 = 0x161922;
pub const PLANK: u32 = 0xf4e8d2;
pub const PLANK_ALT: u32 = 0xf0e2c9;
pub const SEAM: u32 = 0xe3d1b4;
pub const OUTER_TOP: u32 = 0x3a404c;
pub const OUTER_FACE: u32 = 0x2a2f3a;
pub const WALL_TOP: u32 = 0xa9b0bb;
pub const WALL_FACE: u32 = 0x6f7684;
pub const DOOR_FRAME: u32 = 0x5d6470;
pub const GLASS: u32 = 0xbfe3f5;
pub const WOOD: u32 = 0xc7925c;
pub const WOOD_HI: u32 = 0xdcae7a;
pub const WOOD_EDGE: u32 = 0x8a5a33;
pub const WOOD_FEET: u32 = 0x5e3c22;
pub const CHAIR: u32 = 0x353a46;
pub const CHAIR_HI: u32 = 0x4b5163;
pub const LINE: u32 = 0x1d1a24;
const BLACK: u32 = 0x000000;
const WHITE: u32 = 0xffffff;
const PLANK_H: f32 = 6.0;
const JOINT: f32 = 48.0;
const WALL_T: f32 = 4.0;
const DOOR: f32 = 22.0;
const SPINES: [u32; 5] = [0xd65a5a, 0x4a86d6, 0x5fa85a, 0xe0b84a, 0x8a6bc2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Off,
    Idle,
    Work,
    Blocked,
}

fn black(b: &mut Brush, r: (f32, f32, f32, f32), a: f32) {
    b.rect(r, hex(BLACK), a);
}

fn white(b: &mut Brush, r: (f32, f32, f32, f32), a: f32) {
    b.rect(r, hex(WHITE), a);
}

fn clip(b: &Brush, r: R) -> Option<(f32, f32, f32, f32)> {
    let (vx, vy, vw, vh) = b.view();
    let x0 = r.x.max(vx);
    let y0 = r.y.max(vy);
    let x1 = (r.x + r.w).min(vx + vw);
    let y1 = (r.y + r.h).min(vy + vh);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

pub fn floor(b: &mut Brush, plan: &Plan) {
    let (w, h) = (plan.w, plan.h);
    b.solid((-8.0, -8.0, w + 16.0, h + 16.0), OUTER_TOP);
    b.solid((0.0, 0.0, w, h), PLANK);
    let Some((x0, y0, x1, y1)) = clip(b, R { x: 0.0, y: 0.0, w, h }) else {
        return;
    };
    let first = (y0 / PLANK_H).floor() as i64;
    let last = (y1 / PLANK_H).ceil() as i64;
    for row in first..last {
        let y = row as f32 * PLANK_H;
        if row % 2 == 1 {
            b.solid((x0, y, x1 - x0, PLANK_H), PLANK_ALT);
        }
        b.solid((x0, y, x1 - x0, 0.6), SEAM);
        let shift = ((row * 19) % JOINT as i64) as f32;
        let mut x = shift + ((x0 - shift) / JOINT).floor() * JOINT;
        while x < x1 {
            if x >= 0.0 {
                b.solid((x, y, 0.6, PLANK_H), SEAM);
            }
            x += JOINT;
        }
    }
}

pub fn outer_walls(b: &mut Brush, plan: &Plan) {
    let (w, h) = (plan.w, plan.h);
    black(b, (0.0, 0.0, w, 6.0), 0.08);
    black(b, (0.0, 6.0, w, 3.0), 0.05);
    black(b, (0.0, 0.0, 4.0, h), 0.05);
    b.solid((-8.0, -8.0, w + 16.0, 5.0), OUTER_TOP);
    b.solid((-8.0, -3.0, w + 16.0, 3.0), OUTER_FACE);
    b.solid((-8.0, 0.0, 8.0, h + 8.0), OUTER_TOP);
    b.solid((w, 0.0, 8.0, h + 8.0), OUTER_TOP);
    b.solid((-8.0, h, w + 16.0, 8.0), OUTER_TOP);
    for &x in &plan.windows {
        b.solid((x - 1.0, -8.0, WINDOW_W + 2.0, 6.0), 0x2a2f3a);
        b.solid((x, -8.0, WINDOW_W, 5.0), 0x9fd0ea);
        b.solid((x + WINDOW_W / 2.0 - 1.0, -8.0, 2.0, 5.0), 0x7fb3d1);
        white(b, (x + 2.0, -7.0, 6.0, 1.0), 0.8);
        white(b, (x + 16.0, -7.0, 3.0, 1.0), 0.6);
    }
}

pub fn sun(b: &mut Brush, plan: &Plan) {
    let c = hex(0xfff4d6);
    for &x in &plan.windows {
        b.rect((x + 2.0, 0.0, 24.0, 10.0), c, 0.18);
        b.rect((x + 5.0, 10.0, 24.0, 10.0), c, 0.18);
        b.rect((x + 8.0, 20.0, 24.0, 8.0), c, 0.18);
    }
}

fn hwall(b: &mut Brush, (x, y, w): (f32, f32, f32)) {
    if w <= 0.0 {
        return;
    }
    b.solid((x, y, w, WALL_T), WALL_TOP);
    b.solid((x, y + WALL_T, w, 2.0), WALL_FACE);
}

fn vwall(b: &mut Brush, (x, y, h): (f32, f32, f32)) {
    b.solid((x, y, WALL_T, h), WALL_TOP);
    b.solid((x + WALL_T - 1.0, y, 1.0, h), 0x959ca8);
}

fn glass(b: &mut Brush, (x, y, h): (f32, f32, f32)) {
    b.rect((x, y, 3.0, h), hex(GLASS), 0.55);
    let mut py = y;
    while py < y + h {
        b.solid((x, py, 3.0, 1.0), 0x8fa3b5);
        white(b, (x + 1.0, py + 4.0, 1.0, 3.0), 0.6);
        white(b, (x + 1.0, py + 12.0, 1.0, 3.0), 0.6);
        py += 24.0;
    }
}

fn gap_wall(b: &mut Brush, r: R, y: f32, door: f32) {
    if door <= 0.0 {
        hwall(b, (r.x, y, r.w));
        return;
    }
    let mid = r.x + r.w / 2.0;
    let a = mid - door / 2.0;
    let z = mid + door / 2.0;
    hwall(b, (r.x, y, a - r.x));
    hwall(b, (z, y, r.x + r.w - z));
    b.solid((a - 2.0, y, 2.0, 6.0), DOOR_FRAME);
    b.solid((z, y, 2.0, 6.0), DOOR_FRAME);
}

fn walls(b: &mut Brush, r: R, door: f32, top_door: bool, glass_side: (bool, bool)) {
    black(b, (r.x + WALL_T, r.y + 6.0, r.w - 2.0 * WALL_T, 3.0), 0.07);
    if glass_side.0 {
        glass(b, (r.x, r.y, r.h));
    } else {
        vwall(b, (r.x, r.y, r.h));
    }
    if glass_side.1 {
        glass(b, (r.x + r.w - 3.0, r.y, r.h));
    } else {
        vwall(b, (r.x + r.w - WALL_T, r.y, r.h));
    }
    gap_wall(b, r, r.y, if top_door { DOOR } else { 0.0 });
    gap_wall(b, r, r.y + r.h - 6.0, door);
}

pub fn room_walls(b: &mut Brush, r: R, door: f32, top_door: bool) {
    walls(b, r, door, top_door, (false, false));
}

pub fn partition(b: &mut Brush, x: f32, y: f32, w: f32) {
    b.solid((x, y, w, 1.0), 0xdfe3ea);
    b.solid((x, y + 1.0, w, 3.0), 0xb7bdc8);
    b.solid((x, y + 4.0, w, 5.0), 0x7c8492);
    black(b, (x, y + 9.0, w, 2.0), 0.12);
}

pub fn area_floor(b: &mut Brush, a: &Area, tint: Hsla) {
    b.rect(a.rect.t(), tint, 0.06);
}

pub fn area_walls(b: &mut Brush, a: &Area) {
    for &(cx, cy) in &a.cells {
        partition(b, cx + 8.0, cy + 6.0, CELL_W - 16.0);
    }
    room_walls(b, a.rect, DOOR, true);
}

pub fn strip_floor(b: &mut Brush, plan: &Plan) {
    let c = plan.cabin;
    b.solid(c.t(), 0xeadbd6);
    if b.cam.s > 4.0
        && let Some((x0, y0, x1, y1)) = clip(b, c)
    {
        let mut y = (y0 / 8.0).ceil() * 8.0;
        while y < y1 {
            let mut x = (x0 / 8.0).ceil() * 8.0 + if (y / 8.0) as i64 % 2 == 0 { 0.0 } else { 4.0 };
            while x < x1 {
                b.solid((x, y, 1.0, 1.0), 0xdcc9c3);
                x += 8.0;
            }
            y += 8.0;
        }
    }
    rug(b, R { x: c.x + 18.0, y: c.y + 78.0, w: c.w - 36.0, h: 26.0 });
    let m = plan.meet;
    b.solid(m.t(), 0xe9dabf);
    let mut x = m.x + 12.0;
    while x < m.x + m.w {
        b.solid((x, m.y, 0.6, m.h), 0xdcc9a8);
        x += 12.0;
    }
    let mut y = m.y + 12.0;
    while y < m.y + m.h {
        b.solid((m.x, y, m.w, 0.6), 0xdcc9a8);
        y += 12.0;
    }
    if let Some(s) = plan.sofa {
        let mid = s.x + s.w / 2.0;
        rug(b, R { x: mid - 44.0, y: s.y + 10.0, w: 88.0, h: 50.0 });
    }
}

fn rug(b: &mut Brush, r: R) {
    b.solid(r.t(), 0xb98d6a);
    b.solid((r.x + 2.0, r.y + 2.0, r.w - 4.0, r.h - 4.0), 0xdcc4a3);
    let (x, y, w, h) = (r.x + 5.0, r.y + 5.0, r.w - 10.0, r.h - 10.0);
    b.solid((x, y, w, 1.0), 0xcbb08c);
    b.solid((x, y + h - 1.0, w, 1.0), 0xcbb08c);
    b.solid((x, y, 1.0, h), 0xcbb08c);
    b.solid((x + w - 1.0, y, 1.0, h), 0xcbb08c);
}

fn bookshelf(b: &mut Brush, (x, y): (f32, f32)) {
    black(b, (x + 2.0, y + 2.0, 30.0, 26.0), 0.12);
    b.solid((x, y, 30.0, 26.0), 0x6b4a2e);
    for shelf in 0..4 {
        let sy = y + 2.0 + shelf as f32 * 6.0;
        b.solid((x + 2.0, sy, 26.0, 5.0), 0x3d2a1a);
        let mut bx = x + 3.0;
        let mut i = shelf * 3;
        while bx < x + 26.0 {
            let wide = if i % 3 == 0 { 3.0 } else { 2.0 };
            let tall = if i % 4 == 1 { 3.0 } else { 4.0 };
            b.solid((bx, sy + 5.0 - tall, wide, tall), SPINES[i % SPINES.len()]);
            bx += wide + if i % 5 == 2 { 2.0 } else { 0.0 };
            i += 1;
        }
    }
    b.solid((x, y, 30.0, 1.0), 0x8a6440);
}

fn sofa(b: &mut Brush, s: R) {
    let (x, y) = (s.x, s.y);
    black(b, (x + 2.0, y + 2.0, s.w, s.h), 0.12);
    b.solid((x, y, s.w, 9.0), 0x3f4f86);
    b.solid((x + 2.0, y + 1.0, s.w - 4.0, 2.0), 0x6a7cbf);
    b.solid((x + 4.0, y + 9.0, s.w - 8.0, 8.0), 0x4a5c9a);
    for i in 0..3 {
        let cx = x + 4.0 + i as f32 * 22.0;
        b.solid((cx, y + 9.0, 20.0, 8.0), 0x5b6fb0);
        b.solid((cx + 1.0, y + 9.0, 18.0, 1.0), 0x7488c8);
    }
    b.solid((x + 4.0, y + 17.0, s.w - 8.0, 3.0), 0x45578f);
    b.solid((x, y + 5.0, 5.0, 15.0), 0x45578f);
    b.solid((x + s.w - 5.0, y + 5.0, 5.0, 15.0), 0x45578f);
    b.solid((x + 1.0, y + 5.0, 3.0, 1.0), 0x6a7cbf);
    b.solid((x + s.w - 4.0, y + 5.0, 3.0, 1.0), 0x6a7cbf);
    black(b, (x, y + s.h, s.w, 2.0), 0.15);
}

fn coffee_table(b: &mut Brush, (x, y): (f32, f32)) {
    black(b, (x + 2.0, y + 2.0, 22.0, 10.0), 0.12);
    b.solid((x, y, 22.0, 10.0), WOOD);
    b.solid((x, y, 22.0, 1.0), WOOD_HI);
    b.solid((x, y + 7.0, 22.0, 3.0), WOOD_EDGE);
    b.solid((x + 3.0, y + 2.0, 4.0, 4.0), LINE);
    b.solid((x + 4.0, y + 3.0, 2.0, 2.0), WHITE);
    b.solid((x + 11.0, y + 1.0, 8.0, 5.0), 0xd65a5a);
    b.solid((x + 12.0, y + 2.0, 5.0, 1.0), WHITE);
    b.solid((x + 12.0, y + 4.0, 3.0, 1.0), 0xf2c6c6);
}

pub fn cooler(b: &mut Brush, (x, y): (f32, f32)) {
    black(b, (x - 4.0, y - 1.0, 12.0, 3.0), 0.12);
    b.solid((x - 6.0, y - 17.0, 12.0, 17.0), LINE);
    b.solid((x - 5.0, y - 16.0, 10.0, 16.0), 0xe8eef5);
    b.solid((x + 3.0, y - 16.0, 2.0, 16.0), 0xc9d3de);
    b.solid((x - 5.0, y - 16.0, 10.0, 1.0), 0xc9d3de);
    b.solid((x - 3.0, y - 12.0, 2.0, 2.0), 0xd65a5a);
    b.solid((x + 1.0, y - 12.0, 2.0, 2.0), 0x4a86d6);
    b.solid((x - 3.0, y - 7.0, 6.0, 1.0), 0x9aa5b3);
    b.rect((x - 4.0, y - 27.0, 8.0, 10.0), hex(0x8cc4ee), 0.9);
    b.solid((x - 2.0, y - 28.0, 4.0, 1.0), 0x4a86d6);
    b.solid((x - 3.0, y - 25.0, 1.0, 6.0), 0xd2ecfb);
    b.solid((x + 6.0, y - 15.0, 4.0, 7.0), LINE);
    b.solid((x + 7.0, y - 14.0, 2.0, 5.0), 0xf4f1ea);
}

pub fn plant(b: &mut Brush, (x, y): (f32, f32), kind: u8) {
    black(b, (x - 4.0, y - 1.0, 11.0, 3.0), 0.12);
    match kind % 3 {
        0 => {
            b.solid((x - 4.0, y - 6.0, 8.0, 6.0), 0xb5653a);
            b.solid((x - 5.0, y - 7.0, 10.0, 2.0), 0x8f4a28);
            b.solid((x - 7.0, y - 12.0, 14.0, 5.0), 0x3f7f3a);
            b.solid((x - 5.0, y - 16.0, 10.0, 5.0), 0x4f9a4a);
            b.solid((x - 3.0, y - 19.0, 6.0, 4.0), 0x4f9a4a);
            b.solid((x - 8.0, y - 9.0, 2.0, 2.0), 0x3f7f3a);
            b.solid((x + 6.0, y - 9.0, 2.0, 2.0), 0x3f7f3a);
            b.solid((x - 6.0, y - 11.0, 3.0, 2.0), 0x6cc160);
            b.solid((x + 2.0, y - 15.0, 3.0, 2.0), 0x6cc160);
            b.solid((x - 1.0, y - 18.0, 2.0, 2.0), 0x6cc160);
        }
        1 => {
            b.solid((x - 4.0, y - 6.0, 8.0, 6.0), 0x8c929c);
            b.solid((x - 5.0, y - 7.0, 10.0, 2.0), 0x6f7684);
            for (dx, tall) in [(-4.0, 8.0), (-2.0, 12.0), (0.0, 15.0), (2.0, 11.0), (4.0, 7.0)] {
                b.solid((dx + x - 1.0, y - 7.0 - tall, 2.0, tall), 0x3f7f3a);
                b.solid((dx + x - 1.0, y - 7.0 - tall, 1.0, tall), 0xd8c75a);
                b.solid((dx + x, y - 5.0 - tall, 1.0, 2.0), 0x6cc160);
            }
        }
        _ => {
            b.solid((x - 4.0, y - 4.0, 8.0, 4.0), 0xb5653a);
            b.solid((x - 4.0, y - 4.0, 8.0, 1.0), 0x8f4a28);
            b.solid((x - 6.0, y - 13.0, 12.0, 9.0), 0x4f9a4a);
            b.solid((x - 5.0, y - 14.0, 10.0, 1.0), 0x4f9a4a);
            b.solid((x - 6.0, y - 6.0, 12.0, 2.0), 0x3f7f3a);
            b.solid((x - 3.0, y - 12.0, 3.0, 2.0), 0x6cc160);
            b.solid((x + 2.0, y - 9.0, 2.0, 2.0), 0x6cc160);
        }
    }
}

pub fn strip(b: &mut Brush, plan: &Plan) {
    let c = plan.cabin;
    bookshelf(b, (c.x + 8.0, c.y + 8.0));
    walls(b, c, DOOR, false, (false, true));
    let m = plan.meet;
    walls(b, m, DOOR, false, (true, false));
    if let Some(s) = plan.sofa {
        sofa(b, s);
        coffee_table(b, (s.x + s.w / 2.0 - 11.0, s.y + 30.0));
    }
    for &(at, kind) in &plan.plants {
        plant(b, at, kind);
    }
    for &at in &plan.coolers {
        cooler(b, at);
    }
}

pub fn meet_table(b: &mut Brush, plan: &Plan, laptops: &[f32]) {
    let t = plan.table;
    black(b, (t.x + 2.0, t.y + 2.0, t.w, t.h), 0.12);
    b.solid(t.t(), WOOD);
    b.solid((t.x, t.y, t.w, 1.0), WOOD_HI);
    b.solid((t.x, t.y + t.h - 4.0, t.w, 4.0), WOOD_EDGE);
    b.solid((t.x + 2.0, t.y + t.h, 3.0, 2.0), WOOD_FEET);
    b.solid((t.x + t.w - 5.0, t.y + t.h, 3.0, 2.0), WOOD_FEET);
    for &x in laptops {
        b.solid((x - 3.0, t.y + 3.0, 6.0, 4.0), 0xc9ced6);
        b.solid((x - 3.0, t.y + 3.0, 6.0, 1.0), 0x9aa1ad);
    }
}

pub fn chair_back(b: &mut Brush, (x, y): (f32, f32)) {
    black(b, (x - 5.0, y - 19.0, 14.0, 11.0), 0.12);
    b.solid((x - 7.0, y - 21.0, 14.0, 11.0), CHAIR);
    b.solid((x - 6.0, y - 21.0, 12.0, 2.0), CHAIR_HI);
    b.solid((x - 9.0, y - 11.0, 2.0, 5.0), 0x2a2e38);
    b.solid((x + 7.0, y - 11.0, 2.0, 5.0), 0x2a2e38);
}

pub fn deep_chair_back(b: &mut Brush, (x, y): (f32, f32)) {
    b.solid((x - 5.0, y - 18.0, 10.0, 8.0), CHAIR);
    b.solid((x - 5.0, y - 18.0, 10.0, 2.0), CHAIR_HI);
}

pub fn deep_chair_front(b: &mut Brush, (x, y): (f32, f32)) {
    black(b, (x - 3.0, y + 4.0, 10.0, 2.0), 0.12);
    b.solid((x - 5.0, y - 4.0, 10.0, 8.0), CHAIR);
    b.solid((x - 5.0, y - 4.0, 10.0, 2.0), CHAIR_HI);
}

fn desk_prop(b: &mut Brush, d: R, seed: u64) {
    let (x, y) = (d.x, d.y);
    match seed % 4 {
        0 => {
            b.solid((x + 4.0, y + 2.0, 5.0, 5.0), LINE);
            b.solid((x + 5.0, y + 3.0, 3.0, 3.0), WHITE);
            b.solid((x + 5.0, y + 3.0, 3.0, 1.0), 0x6b4226);
            b.solid((x + 9.0, y + 3.0, 1.0, 2.0), LINE);
        }
        1 => {
            b.solid((x + 4.0, y + 3.0, 8.0, 6.0), 0xd9d2c3);
            b.solid((x + 3.0, y + 2.0, 8.0, 6.0), 0xf7f3ea);
            b.solid((x + 4.0, y + 4.0, 5.0, 1.0), 0xd9d2c3);
            b.solid((x + 4.0, y + 6.0, 4.0, 1.0), 0xd9d2c3);
        }
        2 => {
            b.solid((x + 4.0, y + 4.0, 5.0, 4.0), 0xc56a3d);
            b.solid((x + 5.0, y - 2.0, 3.0, 6.0), 0x5fa85a);
            b.solid((x + 4.0, y, 1.0, 2.0), 0x5fa85a);
            b.solid((x + 6.0, y - 1.0, 1.0, 3.0), 0x86c97f);
        }
        _ => {
            b.rect((x + 6.0, y + 1.0, 8.0, 5.0), hex(0xfff4d6), 0.35);
            b.solid((x + 3.0, y + 6.0, 6.0, 2.0), 0x3a3f4b);
            b.solid((x + 5.0, y - 4.0, 1.0, 10.0), 0x3a3f4b);
            b.solid((x + 5.0, y - 5.0, 6.0, 3.0), 0xe0b84a);
        }
    }
}

pub fn desk(b: &mut Brush, d: R, screen: Screen, glow: Hsla, seed: u64, now: u64, sip: bool) {
    black(b, (d.x + 2.0, d.y + d.h, d.w - 2.0, 3.0), 0.18);
    b.solid((d.x + 1.0, d.y + d.h, 2.0, 2.0), WOOD_FEET);
    b.solid((d.x + d.w - 3.0, d.y + d.h, 2.0, 2.0), WOOD_FEET);
    b.solid((d.x, d.y, d.w, d.h - 4.0), WOOD);
    b.solid((d.x, d.y, d.w, 1.0), WOOD_HI);
    b.solid((d.x, d.y + d.h - 4.0, d.w, 4.0), WOOD_EDGE);
    let cx = d.x + d.w / 2.0;
    let mx = d.x + d.w - 8.0;
    b.solid((cx - 7.0, d.y + 3.0, 12.0, 3.0), 0xe9ecf0);
    b.solid((cx - 6.0, d.y + 4.0, 10.0, 1.0), 0xc5cad3);
    if !(sip && seed.is_multiple_of(4)) {
        desk_prop(b, d, seed);
    }
    b.solid((mx - 4.0, d.y + 3.0, 8.0, 1.0), SCREEN_BASE);
    b.solid((mx - 1.0, d.y + 1.0, 2.0, 2.0), 0x15171d);
    b.solid((mx - 8.0, d.y - 8.0, 16.0, 11.0), 0x15171d);
    b.solid((mx - 7.0, d.y - 7.0, 14.0, 9.0), SCREEN_BASE);
    b.solid((mx - 1.0, d.y - 4.0, 2.0, 2.0), 0x4a5060);
    let red = hex(0xe0524f);
    match screen {
        Screen::Work => {
            let a = work_glow(seed, now);
            b.rect((mx - 7.0, d.y - 7.0, 14.0, 9.0), glow, a * 0.55);
            b.rect((mx - 6.0, d.y - 8.0, 12.0, 1.0), glow, a + 0.2);
            b.rect((mx - 8.0, d.y - 6.0, 16.0, 12.0), glow, a * 0.24);
        }
        Screen::Blocked => {
            let a = blocked_glow(now);
            b.rect((mx - 7.0, d.y - 7.0, 14.0, 9.0), red, a);
            b.rect((mx - 6.0, d.y - 8.0, 12.0, 1.0), red, 0.8);
            b.rect((mx - 8.0, d.y - 6.0, 16.0, 12.0), red, a * 0.35);
        }
        Screen::Off | Screen::Idle => {}
    }
}

const SCREEN_BASE: u32 = 0x2b2f3a;

pub fn face_light(b: &mut Brush, (x, head): (f32, f32), glow: Hsla) {
    b.rect((x - 4.0, head + 3.0, 8.0, 6.0), glow, 0.10);
}

pub fn work_glow(seed: u64, now: u64) -> f32 {
    let phase = (seed % 628) as f64 / 100.0;
    let base = 0.5 + 0.08 * (now as f64 * 0.0021 + phase).sin() as f32;
    let spark = if super::motion::hash(seed, now / 100).is_multiple_of(23) { 0.15 } else { 0.0 };
    base + spark
}

pub fn blocked_glow(now: u64) -> f32 {
    0.35 + 0.25 * (now as f64 * 0.004).sin().abs() as f32
}
