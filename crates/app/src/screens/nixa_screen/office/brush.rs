use super::camera::Cam;
use gpui::{Bounds, Hsla, Pixels, Point, Window, fill, point, px, rgb, size};

pub fn hex(c: u32) -> Hsla {
    rgb(c).into()
}

pub struct Brush<'a> {
    pub window: &'a mut Window,
    pub o: Point<Pixels>,
    pub cam: Cam,
    pub w: f32,
    pub h: f32,
}

impl Brush<'_> {
    pub fn rect(&mut self, (x, y, w, h): (f32, f32, f32, f32), color: Hsla, alpha: f32) {
        if alpha <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let x1 = (x * self.cam.s + self.cam.ox).floor();
        let y1 = (y * self.cam.s + self.cam.oy).floor();
        let x2 = ((x + w) * self.cam.s + self.cam.ox).floor().max(x1 + 1.0);
        let y2 = ((y + h) * self.cam.s + self.cam.oy).floor().max(y1 + 1.0);
        if x2 < 0.0 || y2 < 0.0 || x1 > self.w || y1 > self.h {
            return;
        }
        let bounds = Bounds::new(
            point(self.o.x + px(x1), self.o.y + px(y1)),
            size(px(x2 - x1), px(y2 - y1)),
        );
        self.window
            .paint_quad(fill(bounds, color.opacity(alpha.min(1.0))));
    }

    pub fn view(&self) -> (f32, f32, f32, f32) {
        let s = self.cam.s.max(0.0001);
        (-self.cam.ox / s, -self.cam.oy / s, self.w / s, self.h / s)
    }

    pub fn solid(&mut self, r: (f32, f32, f32, f32), c: u32) {
        self.rect(r, hex(c), 1.0);
    }

    pub fn screen(&mut self, (x, y, w, h): (f32, f32, f32, f32), color: Hsla, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let bounds = Bounds::new(point(self.o.x + px(x), self.o.y + px(y)), size(px(w), px(h)));
        self.window
            .paint_quad(fill(bounds, color.opacity(alpha.min(1.0))));
    }
}
