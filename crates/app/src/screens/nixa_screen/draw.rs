use gpui::{Hsla, PathBuilder, Pixels, Point, Window, point, px};

pub const TAU: f32 = std::f32::consts::TAU;
pub const PI: f32 = std::f32::consts::PI;

pub fn polar(x: f32, y: f32, r: f32, a: f32) -> (f32, f32) {
    (x + r * a.cos(), y + r * a.sin())
}

pub fn polygon(x: f32, y: f32, r: f32, sides: usize) -> Vec<(f32, f32)> {
    (0..sides)
        .map(|i| polar(x, y, r, TAU * i as f32 / sides as f32))
        .collect()
}

pub fn steps_for(span: f32, full: usize) -> usize {
    ((full as f32 * span.abs() / TAU).ceil() as usize).max(4)
}

pub struct Pen {
    b: PathBuilder,
    o: Point<Pixels>,
    any: bool,
}

impl Pen {
    pub fn stroke(o: Point<Pixels>, width: f32) -> Pen {
        Pen {
            b: PathBuilder::stroke(px(width)),
            o,
            any: false,
        }
    }

    pub fn dashed(o: Point<Pixels>, width: f32, dash: &[f32]) -> Pen {
        let dash: Vec<Pixels> = dash.iter().map(|d| px(*d)).collect();
        Pen {
            b: PathBuilder::stroke(px(width)).dash_array(&dash),
            o,
            any: false,
        }
    }

    pub fn fill(o: Point<Pixels>) -> Pen {
        Pen {
            b: PathBuilder::fill(),
            o,
            any: false,
        }
    }

    fn at(&self, x: f32, y: f32) -> Point<Pixels> {
        point(self.o.x + px(x), self.o.y + px(y))
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.b.move_to(p);
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.b.line_to(p);
        self.any = true;
    }

    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        self.move_to(x1, y1);
        self.line_to(x2, y2);
    }

    pub fn poly(&mut self, points: &[(f32, f32)]) {
        if points.len() < 3 {
            return;
        }
        let list: Vec<Point<Pixels>> = points.iter().map(|(x, y)| self.at(*x, *y)).collect();
        self.b.add_polygon(&list, true);
        self.any = true;
    }

    pub fn arc(&mut self, x: f32, y: f32, r: f32, from: f32, span: f32, steps: usize) {
        let n = steps.max(1);
        let (sx, sy) = polar(x, y, r, from);
        self.move_to(sx, sy);
        for i in 1..=n {
            let (px_, py_) = polar(x, y, r, from + span * i as f32 / n as f32);
            self.line_to(px_, py_);
        }
    }

    pub fn paint(self, window: &mut Window, color: Hsla, alpha: f32) {
        if !self.any || alpha <= 0.0 {
            return;
        }
        if let Ok(path) = self.b.build() {
            window.paint_path(path, color.opacity(alpha.min(1.0)));
        }
    }
}

pub fn disk(
    window: &mut Window,
    o: Point<Pixels>,
    (x, y, r): (f32, f32, f32),
    sides: usize,
    color: Hsla,
    alpha: f32,
) {
    let mut pen = Pen::fill(o);
    pen.poly(&polygon(x, y, r, sides));
    pen.paint(window, color, alpha);
}

pub fn circle(
    window: &mut Window,
    o: Point<Pixels>,
    (x, y, r): (f32, f32, f32),
    width: f32,
    steps: usize,
    color: Hsla,
    alpha: f32,
) {
    let mut pen = Pen::stroke(o, width);
    pen.arc(x, y, r, 0.0, TAU, steps);
    pen.paint(window, color, alpha);
}
