use super::draw::{PI, Pen, TAU, polar, polygon};
use super::mode::Mode;
use super::palette::Tones;
use gpui::{Hsla, Pixels, Point, Window, point, px};

pub const BARS: usize = 72;
pub const TICKS: usize = 120;
pub const METER: usize = 40;
const GLOW: [(f32, f32); 5] = [
    (24.0, 0.04),
    (15.0, 0.07),
    (9.0, 0.13),
    (5.0, 0.26),
    (2.0, 1.0),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Core {
    pub size: f32,
    pub t: f32,
    pub mode: Mode,
    pub level: f32,
    pub mic: Option<f32>,
    pub tones: Tones,
    pub breathing: bool,
}

fn arc_steps(span: f32) -> usize {
    ((span.abs() / 0.1).ceil() as usize).max(6)
}

pub fn voice_amp(mode: Mode, i: usize, t: f32, level: f32) -> f32 {
    let half = BARS / 2;
    let k = if i < half { i } else { BARS - i } as f32;
    match mode {
        Mode::You => {
            let shape = 0.55
                + 0.45
                    * (k * 0.61 + t * 5.3).sin().abs()
                    * (0.7 + 0.3 * (k * 0.23 - t * 2.1).sin());
            level * shape
        }
        Mode::Nixa => {
            let voice = 0.42
                + 0.22 * (t * 8.7).sin()
                + 0.18 * (t * 21.3 + 1.1).sin()
                + 0.1 * (t * 3.1).sin();
            let shape = 0.5 + 0.5 * (k * 0.37 + t * 6.2).sin().abs();
            voice.clamp(0.0, 1.0) * shape
        }
        Mode::Think => 0.12 + 0.1 * (k * 0.5 - t * 7.0).sin().max(0.0),
        _ => 0.0,
    }
}

pub fn breath(t: f32) -> f32 {
    0.725 + 0.275 * (t * TAU / 4.0).sin()
}

pub fn spin(mode: Mode) -> f32 {
    match mode {
        Mode::Think => 1.9,
        Mode::Off => 0.12,
        _ => 0.35,
    }
}

pub fn lit(mic: f32) -> usize {
    (mic.clamp(0.0, 1.0) * (METER / 2) as f32).floor() as usize
}

pub fn paint(window: &mut Window, o: Point<Pixels>, core: &Core) {
    let size = core.size;
    let c = size / 2.0;
    let r0 = c - 6.0;
    let r1 = c - 26.0;
    let level = core.level;
    let r_glow = size * 0.33 * (1.0 + 0.05 * level);
    let r_dash = size * 0.245;
    let r_disk = size * 0.215;
    let tones = core.tones;
    let t = core.t;
    let spin = spin(core.mode);
    let strength = if core.mode == Mode::Off {
        0.4
    } else {
        0.55 + level * 0.9
    };

    let mut disk = Pen::fill(o);
    disk.poly(&polygon(c, c, r_disk, 96));
    disk.paint(window, tones.main, 0.07 + level * 0.12);

    let mut dash = Pen::dashed(o, 1.0, &[3.0, 6.0]);
    dash.arc(c, c, r_dash, 0.0, TAU, arc_steps(TAU));
    dash.paint(window, tones.second, 0.7);

    let mut ticks = Pen::stroke(o, 1.2);
    for i in 0..TICKS {
        let a = TAU * i as f32 / TICKS as f32;
        let long = if i % 10 == 0 { 10.0 } else { 4.0 };
        let (x1, y1) = polar(c, c, r0, a);
        let (x2, y2) = polar(c, c, r0 - long, a);
        ticks.line(x1, y1, x2, y2);
    }
    ticks.paint(window, tones.third, 0.55);

    segments(window, o, (c, r1, -t * spin), (3, 0.95), tones.main, 0.85);
    segments(
        window,
        o,
        (c, r1 - 9.0, t * spin * 0.7),
        (5, 0.32),
        tones.third,
        0.6,
    );

    let breathe = if core.breathing { breath(t) } else { 1.0 };
    let r = (r_glow * 2.0).round() / 2.0;
    for (width, alpha) in GLOW {
        let mut pen = Pen::stroke(o, width);
        pen.arc(c, c, r, 0.0, TAU, arc_steps(TAU));
        let color = if width > 12.0 {
            tones.second
        } else {
            tones.main
        };
        let a = if width > 2.0 { alpha * strength } else { 1.0 };
        pen.paint(window, color, a * breathe);
    }

    bars(window, o, (c, r_glow + 14.0, size * 0.12), core);

    if let Some(mic) = core.mic {
        meter(window, o, size, lit(mic), &tones);
    }
}

fn segments(
    window: &mut Window,
    o: Point<Pixels>,
    (c, r, turn): (f32, f32, f32),
    (count, span): (usize, f32),
    color: Hsla,
    alpha: f32,
) {
    let mut pen = Pen::stroke(o, 3.0);
    for i in 0..count {
        pen.arc(
            c,
            c,
            r,
            turn + TAU * i as f32 / count as f32,
            span,
            arc_steps(span),
        );
    }
    pen.paint(window, color, alpha);
}

fn bars(window: &mut Window, o: Point<Pixels>, (c, inner, length): (f32, f32, f32), core: &Core) {
    let palette = [core.tones.main, core.tones.second, core.tones.third];
    let mut groups: Vec<Pen> = palette.iter().map(|_| Pen::stroke(o, 2.4)).collect();
    let shift = core.t * 0.08;
    for i in 0..BARS {
        let amp = voice_amp(core.mode, i, core.t, core.level);
        if amp < 0.02 {
            continue;
        }
        let a = TAU * i as f32 / BARS as f32 - PI / 2.0;
        let slot = (((i as f32 / BARS as f32 + shift).rem_euclid(1.0) * palette.len() as f32 * 2.0)
            .floor() as usize)
            % palette.len();
        let (x1, y1) = polar(c, c, inner, a);
        let (x2, y2) = polar(c, c, inner + 3.0 + amp * length, a);
        groups[slot].line(x1, y1, x2, y2);
    }
    for (pen, color) in groups.into_iter().zip(palette) {
        pen.paint(window, color, 0.9);
    }
}

fn meter(window: &mut Window, o: Point<Pixels>, size: f32, lit: usize, tones: &Tones) {
    let c = size / 2.0;
    let r = size * 0.232;
    let step = TAU / METER as f32;
    let gap = step * 0.22;
    let width = (size * 0.022).max(3.0);
    let mut empty = Pen::stroke(o, width);
    let mut full = Pen::stroke(o, width);
    for i in 0..METER {
        let rank = if i < METER / 2 { i } else { METER - 1 - i };
        let from = -PI / 2.0 + i as f32 * step + gap / 2.0;
        let pen = if rank < lit { &mut full } else { &mut empty };
        pen.arc(c, c, r, from, step - gap, arc_steps(step - gap));
    }
    empty.paint(window, tones.third, 0.22);
    full.paint(window, tones.main, 0.95);
}

pub fn origin(o: Point<Pixels>, x: f32, y: f32, size: f32) -> Point<Pixels> {
    point(o.x + px(x - size / 2.0), o.y + px(y - size / 2.0))
}
