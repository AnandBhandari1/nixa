use super::draw::Pen;
use gpui::{Pixels, Point, Window, rgb};
use std::sync::OnceLock;

pub const SKY_W: f32 = 1600.0;
pub const SKY_H: f32 = 1000.0;

struct Layer {
    count: usize,
    small: f64,
    big: f64,
    alpha: f32,
    color: u32,
}

const LAYERS: [Layer; 3] = [
    Layer {
        count: 260,
        small: 0.35,
        big: 0.7,
        alpha: 0.28,
        color: 0x8a93b8,
    },
    Layer {
        count: 90,
        small: 0.5,
        big: 1.1,
        alpha: 0.38,
        color: 0xa9b4d8,
    },
    Layer {
        count: 26,
        small: 0.8,
        big: 1.5,
        alpha: 0.5,
        color: 0xc0caf5,
    },
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Star {
    pub x: f32,
    pub y: f32,
    pub r: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub scale: f32,
    pub left: f32,
    pub top: f32,
}

pub fn random(index: usize, seed: usize) -> f64 {
    let v = (index as f64 * 127.1 + seed as f64 * 311.7).sin() * 43_758.545_3;
    v - v.floor()
}

fn make() -> Vec<Vec<Star>> {
    LAYERS
        .iter()
        .enumerate()
        .map(|(k, layer)| {
            let seed = 10 + k * 7;
            (0..layer.count)
                .map(|i| Star {
                    x: (random(i, seed) * f64::from(SKY_W)) as f32,
                    y: (random(i, seed + 1) * f64::from(SKY_H)) as f32,
                    r: (layer.small + random(i, seed + 2).powi(4) * (layer.big - layer.small))
                        as f32,
                })
                .collect()
        })
        .collect()
}

pub fn stars() -> &'static [Vec<Star>] {
    static SKY: OnceLock<Vec<Vec<Star>>> = OnceLock::new();
    SKY.get_or_init(make)
}

pub fn fit(w: f32, h: f32) -> Fit {
    let scale = (w / SKY_W).max(h / SKY_H);
    Fit {
        scale,
        left: (w - SKY_W * scale) / 2.0,
        top: (h - SKY_H * scale) / 2.0,
    }
}

pub fn paint(window: &mut Window, o: Point<Pixels>, w: f32, h: f32) {
    let f = fit(w, h);
    for (layer, list) in LAYERS.iter().zip(stars()) {
        let mut pen = Pen::fill(o);
        for s in list {
            let x = f.left + s.x * f.scale;
            let y = f.top + s.y * f.scale;
            let r = s.r * f.scale;
            if x < -2.0 || y < -2.0 || x > w + 2.0 || y > h + 2.0 {
                continue;
            }
            pen.poly(&[(x, y - r), (x + r, y), (x, y + r), (x - r, y)]);
        }
        pen.paint(window, rgb(layer.color).into(), layer.alpha);
    }
}
