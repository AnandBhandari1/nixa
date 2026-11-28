#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cam {
    pub s: f32,
    pub ox: f32,
    pub oy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub cx: f32,
    pub cy: f32,
    pub span: f32,
}

pub const MARGIN: f32 = 14.0;
pub const CLOSE_SPAN: f32 = 92.0;
pub const TOP_ROOM: f32 = 58.0;

pub fn overview(plan_w: f32, plan_h: f32, w: f32, h: f32) -> Shot {
    let room_h = (h - TOP_ROOM).max(40.0);
    let fit_h = plan_h + MARGIN * 2.0;
    let fit_w = (plan_w + MARGIN * 2.0) * room_h / w.max(1.0);
    let span = fit_h.max(fit_w) * h / room_h;
    Shot {
        cx: plan_w / 2.0,
        cy: plan_h / 2.0 - TOP_ROOM / 2.0 * span / h,
        span,
    }
}

pub fn close(a: (f32, f32), b: (f32, f32), w: f32, h: f32) -> Shot {
    let w = w.max(1.0);
    let dx = (a.0 - b.0).abs();
    let dy = (a.1 - b.1).abs();
    let need = (dx + 64.0).max(dx / 0.7);
    let span = CLOSE_SPAN.max(need * h / w).max(dy + 60.0);
    let half = 0.35 * span * w / h.max(1.0);
    let (lo, hi) = (a.0.max(b.0) - half, a.0.min(b.0) + half);
    let mid = (a.0 + b.0) / 2.0;
    let cx = if lo <= hi { mid.clamp(lo, hi) } else { mid };
    let feet = (a.1 + b.1) / 2.0;
    Shot {
        cx,
        cy: feet - span * 0.16,
        span,
    }
}

pub fn blend(a: Shot, b: Shot, k: f32, zoom_in: bool) -> Shot {
    let m = super::motion::in_out_cubic(k);
    let z = if zoom_in {
        super::motion::out_back(k, 0.9)
    } else {
        m
    };
    let span = a.span * (b.span / a.span).powf(z);
    Shot {
        cx: a.cx + (b.cx - a.cx) * m,
        cy: a.cy + (b.cy - a.cy) * m,
        span,
    }
}

pub fn cam_of(shot: Shot, w: f32, h: f32) -> Cam {
    let s = h / shot.span.max(1.0);
    Cam {
        s,
        ox: w / 2.0 - shot.cx * s,
        oy: h / 2.0 - shot.cy * s,
    }
}

impl Cam {
    pub fn to_screen(self, x: f32, y: f32) -> (f32, f32) {
        (x * self.s + self.ox, y * self.s + self.oy)
    }
}
