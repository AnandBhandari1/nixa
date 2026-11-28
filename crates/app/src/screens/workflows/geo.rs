use super::flow::{BOX_W, HEAD, Options, Spot, Workflow, rows};
use super::layout;
use std::collections::BTreeMap;

pub const MIN_FIT: f32 = 0.35;
pub const MAX_FIT: f32 = 1.0;
pub const MIN_ZOOM: f32 = 0.3;
pub const MAX_ZOOM: f32 = 2.5;
pub const MARGIN: f32 = 40.0;
pub const ZOOM_IN: f32 = 1.25;
pub const ZOOM_OUT: f32 = 0.8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cam {
    pub z: f32,
    pub x: f32,
    pub y: f32,
}

impl Default for Cam {
    fn default() -> Self {
        Cam {
            z: 1.0,
            x: 0.0,
            y: 0.0,
        }
    }
}

impl Cam {
    pub fn sx(&self, x: f64) -> f32 {
        x as f32 * self.z + self.x
    }

    pub fn sy(&self, y: f64) -> f32 {
        y as f32 * self.z + self.y
    }
}

pub fn shape(flow: &Workflow, run: bool, options: &Options) -> String {
    let ids = super::flow::ids(flow);
    let boxes: Vec<String> = ids
        .iter()
        .map(|id| format!("{id}:{}", rows(flow, id, run, options)))
        .collect();
    let links: Vec<String> = flow
        .links
        .iter()
        .map(|l| format!("{}>{}", l.from, l.to))
        .collect();
    format!("{}|{}", boxes.join(","), links.join(","))
}

pub fn auto_spots(flow: &Workflow, run: bool, options: &Options) -> BTreeMap<String, Spot> {
    let ids = super::flow::ids(flow);
    let index: BTreeMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let edges: Vec<(usize, usize)> = flow
        .links
        .iter()
        .filter_map(|l| Some((*index.get(l.from.as_str())?, *index.get(l.to.as_str())?)))
        .collect();
    let size: Vec<(f32, f32)> = ids
        .iter()
        .map(|id| (BOX_W, rows(flow, id, run, options)))
        .collect();
    let out = layout::layout(ids.len(), &edges, &size);
    ids.into_iter()
        .zip(out)
        .map(|(id, (x, y))| {
            (
                id,
                Spot {
                    x: x as f64,
                    y: y as f64,
                },
            )
        })
        .collect()
}

pub fn spots(flow: &Workflow, auto: &BTreeMap<String, Spot>) -> BTreeMap<String, Spot> {
    flow.nodes
        .keys()
        .map(|id| {
            let p = flow
                .layout
                .get(id)
                .or_else(|| auto.get(id))
                .copied()
                .unwrap_or_default();
            (id.clone(), p)
        })
        .collect()
}

pub fn bounds(
    flow: &Workflow,
    spots: &BTreeMap<String, Spot>,
    run: bool,
    options: &Options,
) -> Option<(f32, f32, f32, f32)> {
    let mut b: Option<(f32, f32, f32, f32)> = None;
    for (id, p) in spots {
        let h = rows(flow, id, run, options);
        let (x0, y0) = (p.x as f32 - BOX_W / 2.0, p.y as f32 - h / 2.0);
        let (x1, y1) = (p.x as f32 + BOX_W / 2.0, p.y as f32 + h / 2.0);
        b = Some(match b {
            None => (x0, y0, x1, y1),
            Some((a, c, d, e)) => (a.min(x0), c.min(y0), d.max(x1), e.max(y1)),
        });
    }
    b
}

pub fn fit(area: (f32, f32), b: (f32, f32, f32, f32)) -> Cam {
    let (w, h) = area;
    let (x0, y0, x1, y1) = b;
    let bw = (x1 - x0).max(1.0);
    let bh = (y1 - y0).max(1.0);
    let z = ((w - MARGIN) / bw)
        .min((h - MARGIN) / bh)
        .clamp(MIN_FIT, MAX_FIT);
    Cam {
        z,
        x: (w - bw * z) / 2.0 - x0 * z,
        y: (h - bh * z) / 2.0 - y0 * z,
    }
}

pub fn zoom_at(cam: Cam, f: f32, px: f32, py: f32) -> Cam {
    let z = (cam.z * f).clamp(MIN_ZOOM, MAX_ZOOM);
    let k = z / cam.z;
    Cam {
        z,
        x: px - (px - cam.x) * k,
        y: py - (py - cam.y) * k,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub kind: String,
    pub start: (f32, f32),
    pub c1: (f32, f32),
    pub c2: (f32, f32),
    pub end: (f32, f32),
    pub tip: (f32, f32),
    pub head: f32,
    pub mid: (f32, f32),
}

pub fn curves(
    flow: &Workflow,
    spots: &BTreeMap<String, Spot>,
    cam: Cam,
    run: bool,
    options: &Options,
) -> Vec<Curve> {
    let z = cam.z;
    let mut out = Vec::new();
    for l in &flow.links {
        let (Some(a), Some(b)) = (spots.get(&l.from), spots.get(&l.to)) else {
            continue;
        };
        let ha = rows(flow, &l.from, run, options);
        let hb = rows(flow, &l.to, run, options);
        let ax = cam.sx(a.x) + BOX_W / 2.0 * z;
        let ay = cam.sy(a.y) + (-ha / 2.0 + HEAD / 2.0 + 4.0) * z;
        let bx = cam.sx(b.x) - BOX_W / 2.0 * z;
        let by = cam.sy(b.y) + (-hb / 2.0 + HEAD / 2.0 + 4.0) * z;
        let dx = (60.0 * z).max((bx - ax).abs() / 2.0);
        let head = (8.0 * z).max(5.0);
        out.push(Curve {
            kind: l.kind.clone(),
            start: (ax, ay),
            c1: (ax + dx, ay),
            c2: (bx - dx, by),
            end: (bx - head * 0.8, by),
            tip: (bx, by),
            head,
            mid: (
                (ax + 3.0 * (ax + dx) + 3.0 * (bx - dx) + bx) / 8.0,
                (ay + 3.0 * ay + 3.0 * by + by) / 8.0,
            ),
        });
    }
    out
}
