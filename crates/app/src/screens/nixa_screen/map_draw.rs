use super::aura::work_ring;
use super::draw::{Pen, TAU, circle, disk, polygon};
use super::map::{
    Burst, Event, FLOW_LIFE_MS, FLOW_MS, Flow, Layout, Line, NIXA, NOVA_MS, PULSE_MS, Placed,
    WARP_MS, endpoints, node_of,
};
use super::palette::{aura_color, glow_color, mix, status_color};
use gpui::{Hsla, Pixels, Point, Window, rgb};
use nixa_ui::Theme;
use std::collections::BTreeMap;

const SPACE_DARK: u32 = 0x05070d;
const WHITE_WARM: u32 = 0xfff4e8;

fn hex(c: u32) -> Hsla {
    rgb(c).into()
}

pub fn link_line(layout: &Layout, placed: &Placed) -> Option<Line> {
    let from = match (&placed.parent, &placed.agent.parent) {
        (Some(p), _) => p.as_str(),
        (None, None) => NIXA,
        (None, Some(_)) => return None,
    };
    endpoints(layout, from, &placed.agent.id)
}

pub fn links(window: &mut Window, o: Point<Pixels>, layout: &Layout, link: Hsla) {
    if layout.agents.iter().any(|p| p.parent.is_none()) {
        let mut ring = Pen::dashed(o, 1.0, &[2.0, 7.0]);
        for i in 0..=160 {
            let a = TAU * i as f32 / 160.0;
            let x = layout.nixa.x + layout.ax * a.sin();
            let y = layout.nixa.y - layout.ay * a.cos();
            if i == 0 {
                ring.move_to(x, y);
            } else {
                ring.line_to(x, y);
            }
        }
        ring.paint(window, link, 0.08);
    }
    let mut lines = Pen::stroke(o, 1.0);
    for placed in &layout.agents {
        if let Some(l) = link_line(layout, placed) {
            lines.line(l.x1, l.y1, l.x2, l.y2);
        }
    }
    lines.paint(window, link, 0.07);
    for brain in &layout.brains {
        if let Some(l) = endpoints(layout, NIXA, &brain.agent.id) {
            let mut pen = Pen::dashed(o, 1.2, &[3.0, 5.0]);
            pen.line(l.x1, l.y1, l.x2, l.y2);
            pen.paint(window, brain.color, 0.2);
        }
    }
    constellations(window, o, layout);
}

pub fn groups(layout: &Layout) -> Vec<Vec<&Placed>> {
    let mut groups: BTreeMap<String, Vec<&Placed>> = BTreeMap::new();
    for placed in &layout.agents {
        let key = match (placed.agent.task(), &placed.parent) {
            (Some(task), _) => format!("task:{task}"),
            (None, Some(parent)) => format!("parent:{parent}"),
            _ => continue,
        };
        groups.entry(key).or_default().push(placed);
    }
    groups.into_values().filter(|g| g.len() >= 2).collect()
}

fn constellations(window: &mut Window, o: Point<Pixels>, layout: &Layout) {
    let mut lines = Pen::dashed(o, 0.8, &[1.0, 4.0]);
    let mut stars = Pen::fill(o);
    for members in groups(layout) {
        for pair in members.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            lines.line(a.x, a.y, b.x, b.y);
            stars.poly(&polygon((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, 1.3, 4));
        }
    }
    lines.paint(window, hex(0xb9d4ff), 0.3);
    stars.paint(window, hex(0xe8f4ff), 0.7);
}

pub fn nodes(
    window: &mut Window,
    o: Point<Pixels>,
    layout: &Layout,
    t: &Theme,
    time: f32,
    quiet: bool,
) {
    for placed in &layout.agents {
        let status = placed.agent.status;
        let color = status_color(t, status);
        let aura = aura_color(&placed.agent.id);
        let r = placed.radius;
        let (x, y) = (placed.x, placed.y);
        let pulse = if !quiet && status == super::map::St::Blocked {
            0.55 + 0.45 * (time * 4.0).sin()
        } else {
            1.0
        };
        let glow = glow_color(t, status).unwrap_or(aura);
        let strength = match status {
            super::map::St::Working => 1.0,
            super::map::St::Blocked => 0.9 * pulse,
            super::map::St::Done => 0.7,
            super::map::St::Idle => 0.3,
        };
        disk(window, o, (x, y, r + 9.0), 24, glow, 0.06 * strength);
        disk(window, o, (x, y, r + 4.0), 24, glow, 0.12 * strength);
        disk(
            window,
            o,
            (x, y, r),
            24,
            mix(aura, hex(SPACE_DARK), 0.55),
            0.95,
        );
        circle(window, o, (x, y, r), 1.0, 40, glow, 0.55 * pulse);
        disk(
            window,
            o,
            (x + r * 0.8, y + r * 0.8, 3.0),
            24,
            color,
            0.95 * pulse,
        );
        if status == super::map::St::Working {
            work_ring(window, o, (x, y, r), aura, time);
        }
    }
}

pub fn events(window: &mut Window, o: Point<Pixels>, layout: &Layout, list: &[Event], now: u64) {
    for event in list {
        let Some(node) = node_of(layout, &event.id) else {
            continue;
        };
        if event.at > now {
            continue;
        }
        let age = now - event.at;
        match event.kind {
            Burst::Nova if age < NOVA_MS => {
                let k = age as f32 / NOVA_MS as f32;
                let fade = (1.0 - k).powi(2);
                let mut rays = Pen::stroke(o, 1.4);
                for i in 0..14 {
                    let a = TAU * i as f32 / 14.0 + 0.2;
                    let (r1, r2) = (node.r + 4.0 + 50.0 * k, node.r + 14.0 + 80.0 * k);
                    rays.line(
                        node.x + r1 * a.cos(),
                        node.y + r1 * a.sin(),
                        node.x + r2 * a.cos(),
                        node.y + r2 * a.sin(),
                    );
                }
                disk(
                    window,
                    o,
                    (node.x, node.y, node.r * (1.2 + 2.5 * k)),
                    40,
                    hex(0xfff6e0),
                    0.45 * fade,
                );
                circle(
                    window,
                    o,
                    (node.x, node.y, node.r + 6.0 + 90.0 * k),
                    6.0,
                    72,
                    hex(0x7dffb0),
                    0.25 * fade,
                );
                circle(
                    window,
                    o,
                    (node.x, node.y, node.r + 6.0 + 90.0 * k),
                    1.8,
                    72,
                    hex(0xd8ffe6),
                    0.85 * fade,
                );
                rays.paint(window, hex(0xfff6e0), 0.8 * fade);
            }
            Burst::Warp if age < WARP_MS => {
                let k = age as f32 / WARP_MS as f32;
                let angle = (node.y - layout.nixa.y).atan2(node.x - layout.nixa.x);
                let head = (k * 1.6).min(1.0);
                let ease = 1.0 - (1.0 - head).powi(3);
                let far = 420.0;
                let sx = node.x + far * angle.cos() * (1.0 - ease);
                let sy = node.y + far * angle.sin() * (1.0 - ease);
                let ex = sx + 140.0 * (1.0 - ease) * angle.cos();
                let ey = sy + 140.0 * (1.0 - ease) * angle.sin();
                if head < 1.0 {
                    let mut wide = Pen::stroke(o, 5.0);
                    wide.line(ex, ey, sx, sy);
                    wide.paint(window, hex(0x9fe7ff), 0.18);
                    let mut thin = Pen::stroke(o, 1.6);
                    thin.line(ex, ey, sx, sy);
                    thin.paint(window, hex(0xe8fbff), 0.9);
                } else {
                    let q = (k - 1.0 / 1.6) / (1.0 - 1.0 / 1.6);
                    circle(
                        window,
                        o,
                        (node.x, node.y, node.r + 4.0 + 30.0 * q),
                        2.0,
                        48,
                        hex(0x9fe7ff),
                        0.8 * (1.0 - q),
                    );
                    disk(
                        window,
                        o,
                        (node.x, node.y, node.r * (1.0 + q)),
                        32,
                        hex(0xe8fbff),
                        0.35 * (1.0 - q),
                    );
                }
            }
            _ => {}
        }
    }
}

pub fn flows(
    window: &mut Window,
    o: Point<Pixels>,
    layout: &Layout,
    list: &[Flow],
    now: u64,
    color_of: &dyn Fn(&str) -> Hsla,
) {
    for flow in list {
        let Some(line) = endpoints(layout, &flow.from, &flow.to) else {
            continue;
        };
        if flow.at > now {
            continue;
        }
        let age = now - flow.at;
        if age >= FLOW_LIFE_MS {
            continue;
        }
        let color = color_of(&flow.from);
        if age >= FLOW_MS {
            pulse(window, o, &line, flow, age, color);
        } else {
            meteor(window, o, &line, age as f32 / FLOW_MS as f32, color);
        }
    }
}

fn pulse(window: &mut Window, o: Point<Pixels>, line: &Line, flow: &Flow, age: u64, color: Hsla) {
    let q = (age - FLOW_MS) as f32 / PULSE_MS as f32;
    let radius = line.to.r + 3.0 + 14.0 * q;
    let mut burst = Pen::stroke(o, 1.3);
    for i in 0..10 {
        let a = TAU * i as f32 / 10.0 + (flow.at % 7) as f32;
        let from = line.to.r + 2.0 + 22.0 * q;
        let r1 = from * 0.4;
        let r2 = from * 0.4 + 6.0 + 10.0 * q;
        burst.line(
            line.x2 + r1 * a.cos(),
            line.y2 + r1 * a.sin(),
            line.x2 + r2 * a.cos(),
            line.y2 + r2 * a.sin(),
        );
    }
    burst.paint(window, hex(WHITE_WARM), 0.8 * (1.0 - q));
    circle(
        window,
        o,
        (line.to.x, line.to.y, radius),
        7.0,
        48,
        color,
        0.16 * (1.0 - q),
    );
    circle(
        window,
        o,
        (line.to.x, line.to.y, radius),
        2.0,
        48,
        color,
        0.72 * (1.0 - q),
    );
    let flash = (1.0 - (age - FLOW_MS) as f32 / 140.0).max(0.0).powi(2);
    disk(
        window,
        o,
        (line.x2, line.y2, 5.0 + 5.0 * q),
        16,
        hex(WHITE_WARM),
        0.95 * flash,
    );
}

pub fn point_on(line: &Line, p: f32) -> (f32, f32) {
    (
        line.x1 + (line.x2 - line.x1) * p,
        line.y1 + (line.y2 - line.y1) * p,
    )
}

fn meteor(window: &mut Window, o: Point<Pixels>, line: &Line, t: f32, color: Hsla) {
    let p = t * t;
    let dx = line.x2 - line.x1;
    let dy = line.y2 - line.y1;
    let length = dx.hypot(dy);
    if length < 1.0 {
        return;
    }
    let side = if dx >= 0.0 { -1.0 } else { 1.0 };
    let nx = (-dy / length) * side;
    let ny = (dx / length) * side;
    let (x, y) = point_on(line, p);
    let tail = (t - 0.5).max(0.0).powi(2);
    let mut track = Pen::stroke(o, 1.3);
    track.line(line.x1, line.y1, line.x2, line.y2);
    track.paint(window, color, 0.22 * (1.0 - 0.6 * p));
    let (tx, ty) = point_on(line, tail);
    let mut haze = Pen::stroke(o, 18.0);
    haze.line(tx, ty, x, y);
    haze.paint(window, color, 0.05);
    let mut glow = Pen::stroke(o, 9.0);
    glow.move_to(tx, ty);
    let mut previous = (tx, ty);
    for i in 1..=16 {
        let fade = i as f32 / 16.0;
        let next = point_on(line, tail + (p - tail) * fade);
        glow.line_to(next.0, next.1);
        let mut wake = Pen::stroke(o, 0.6 + 3.0 * fade);
        wake.line(previous.0, previous.1, next.0, next.1);
        wake.paint(window, color, 0.75 * fade * fade);
        previous = next;
    }
    glow.paint(window, color, 0.14);
    let mut sparks = Pen::fill(o);
    for i in 1..=3 {
        let behind = t - i as f32 * 0.065;
        if behind <= 0.0 {
            continue;
        }
        let (sx, sy) = point_on(line, behind * behind);
        let drift = (i as f32 * 4.7 + t * 9.0).sin() * (2.0 + i as f32 * 1.5);
        sparks.poly(&polygon(
            sx + nx * drift,
            sy + ny * drift,
            1.6 - i as f32 * 0.25,
            6,
        ));
    }
    sparks.paint(window, hex(WHITE_WARM), 0.65 * (t * 4.0).min(1.0));
    disk(window, o, (x, y, 10.0), 18, color, 0.16);
    disk(window, o, (x, y, 4.0), 16, color, 0.8);
    disk(window, o, (x, y, 2.2), 12, hex(WHITE_WARM), 0.98);
}
