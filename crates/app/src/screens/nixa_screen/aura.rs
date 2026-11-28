use super::draw::{Pen, TAU, polygon, steps_for};
use super::palette::Tones;
use gpui::{Hsla, Pixels, Point, Window};

pub const BASE: u32 = 0x0f1117;
pub const GLOW_RINGS: usize = 4;

pub fn radii(core: f32) -> [f32; GLOW_RINGS] {
    [0, 1, 2, 3].map(|i| core * (0.55 + 0.18 * i as f32))
}

pub fn paint(
    window: &mut Window,
    o: Point<Pixels>,
    (x, y, core): (f32, f32, f32),
    glow: f32,
    tones: &Tones,
) {
    for (i, r) in radii(core).into_iter().enumerate() {
        let mut pen = Pen::fill(o);
        pen.poly(&polygon(x, y, r, 64));
        let color = if i < GLOW_RINGS / 2 {
            tones.main
        } else {
            tones.second
        };
        pen.paint(window, color, 0.03 * glow);
    }
}

pub fn work_ring(
    window: &mut Window,
    o: Point<Pixels>,
    (x, y, r): (f32, f32, f32),
    main: Hsla,
    time: f32,
) {
    let s = (r / 18.0).clamp(0.67, 1.6);
    let orbit = r + 3.0 * s;
    let steps = (48.0 * (r / 18.0).max(1.0)).round() as usize;
    let mut halo = Pen::stroke(o, 4.0 * s);
    halo.arc(x, y, orbit, 0.0, TAU, steps);
    halo.paint(window, main, 0.12);
    let mut arcs = Pen::stroke(o, 2.0 * s);
    let turn = time * 1.2;
    for i in 0..3 {
        arcs.arc(
            x,
            y,
            orbit,
            turn + TAU * i as f32 / 3.0,
            0.9,
            steps_for(0.9, steps),
        );
    }
    arcs.paint(window, main, 0.85);
}
