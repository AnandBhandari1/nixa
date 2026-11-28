use super::brush::Brush;
use super::motion;
use super::talk::MOMENT_MS;
use gpui::Hsla;
use std::f32::consts::PI;

pub const POP: [u32; 2] = [0xffd166, 0x5bc0eb];
const CHECK: [(f32, f32); 5] = [(0.0, 2.0), (1.0, 3.0), (2.0, 2.0), (3.0, 1.0), (4.0, 0.0)];

pub fn jump(te: u64) -> (f32, bool) {
    if te >= MOMENT_MS {
        return (0.0, false);
    }
    let k = (te as f32 / 350.0).min(1.0);
    (-3.0 * (PI * k).sin(), (350..430).contains(&te))
}

pub fn fade(te: u64) -> f32 {
    (1.0 - (te as f32 / MOMENT_MS as f32).powi(2)).max(0.0)
}

pub fn confetti(te: u64, i: usize) -> (f32, f32) {
    let ang = (15.0 + 60.0 * i as f32).to_radians();
    let k = te as f32 / 900.0;
    let r = 18.0 * motion::out_cubic(k);
    (ang.cos() * r, -ang.sin() * r + 10.0 * k * k)
}

pub fn ring(now: u64) -> (f32, f32) {
    let k = (now % 1200) as f32 / 1200.0;
    (6.0 + 8.0 * k, 0.5 * (1.0 - k))
}

pub fn pull_in(te: u64) -> f32 {
    if te >= 250 {
        return 0.0;
    }
    2.0 * (1.0 - motion::out_cubic(te as f32 / 250.0))
}

pub fn dot_size(te: Option<u64>, base: f32) -> f32 {
    match te {
        Some(te) if te < 400 => base + 3.0 * (PI * te as f32 / 400.0).sin(),
        _ => base,
    }
}

pub fn blocked_dot(now: u64) -> f32 {
    0.55 + 0.45 * (now as f64 * 0.0044).sin().abs() as f32
}

pub fn paint_done(b: &mut Brush, (x, y): (f32, f32), te: u64, colors: [Hsla; 4]) {
    if te >= MOMENT_MS {
        return;
    }
    let a = fade(te);
    for i in 0..6 {
        let (dx, dy) = confetti(te, i);
        b.rect(((x + dx).round() - 1.0, (y + dy).round() - 1.0, 2.0, 2.0), colors[i % 4], a);
    }
    let rise = 2.0 * motion::out_cubic(te as f32 / 300.0);
    for (cx, cy) in CHECK {
        b.rect((x - 2.0 + cx, y - 9.0 - rise + cy, 1.0, 1.0), colors[0], a);
    }
}

pub fn paint_ring(b: &mut Brush, (x, y): (f32, f32), now: u64, color: Hsla) {
    let (r, a) = ring(now);
    let r = r.round();
    let d = 2.0 * r;
    b.rect((x - r, y - r, d, 1.0), color, a);
    b.rect((x - r, y + r - 1.0, d, 1.0), color, a);
    b.rect((x - r, y - r + 1.0, 1.0, d - 2.0), color, a);
    b.rect((x + r - 1.0, y - r + 1.0, 1.0, d - 2.0), color, a);
}
