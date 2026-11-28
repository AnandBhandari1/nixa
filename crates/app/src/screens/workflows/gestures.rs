use super::flow::{self, BOX_W, Spot};
use super::view::{DRAG_MIN, Purpose, WorkflowsView};
use gpui::{Context, MouseDownEvent, MouseMoveEvent, Pixels, Point, Window};

pub struct BoxDrag {
    pub id: String,
    link: bool,
    start: Point<Pixels>,
    end: Point<Pixels>,
    origin: Spot,
    zoom: f32,
    moved: bool,
}

impl BoxDrag {
    fn update(&mut self, position: Point<Pixels>) {
        self.end = position;
        let delta = self.end - self.start;
        self.moved |= f32::from(delta.x).hypot(f32::from(delta.y)) >= DRAG_MIN;
    }

    pub fn box_spot(&self) -> Option<Spot> {
        (self.moved && !self.link).then(|| {
            let delta = self.end - self.start;
            Spot {
                x: (self.origin.x + f64::from(f32::from(delta.x) / self.zoom)).round(),
                y: (self.origin.y + f64::from(f32::from(delta.y) / self.zoom)).round(),
            }
        })
    }

    pub fn link_line(&self) -> Option<(Point<Pixels>, Point<Pixels>)> {
        (self.moved && self.link).then_some((self.start, self.end))
    }
}

impl WorkflowsView {
    pub fn begin_box_drag(
        &mut self,
        id: &str,
        link: bool,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus);
        self.select(id, cx);
        if self.editable()
            && let Some(cam) = self.cam
            && let Some(origin) = self.spots.get(id)
        {
            self.box_drag = Some(BoxDrag {
                id: id.into(),
                link,
                start: event.position,
                end: event.position,
                origin: *origin,
                zoom: cam.z,
                moved: false,
            });
        }
        cx.stop_propagation();
    }

    pub fn move_box_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.dragging() {
            if let Some(drag) = &mut self.box_drag {
                drag.update(event.position);
            }
        } else {
            self.box_drag = None;
        }
        cx.notify();
    }

    pub fn finish_box_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(mut drag) = self.box_drag.take() else {
            return;
        };
        drag.update(position);
        cx.notify();
        if !self.editable() {
            return;
        }
        if let Some(spot) = drag.box_spot() {
            self.edit(cx, |flow, _| {
                flow.layout.insert(drag.id, spot);
            });
        } else if drag.link && drag.moved {
            self.link_to_box(drag.id, position, cx);
        }
    }

    fn link_to_box(&mut self, from: String, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (Some(flow), Some(cam)) = (&self.flow, self.cam) else {
            return;
        };
        let (ox, oy, _, _) = self.area.get();
        let x = f32::from(position.x) - ox;
        let y = f32::from(position.y) - oy;
        let target = flow::ids(flow).into_iter().rev().find(|id| {
            let p = self.spots.get(id).copied().unwrap_or_default();
            let height = flow::rows(flow, id, false, &self.options);
            (cam.sx(p.x) - x).abs() <= BOX_W * cam.z / 2.0
                && (cam.sy(p.y) - y).abs() <= height * cam.z / 2.0
        });
        if let Some(to) = target.filter(|id| id != &from && flow.nodes[id].kind != "start") {
            self.open_picker(
                format!("Connect to {}", flow.nodes[&to].title),
                Purpose::LinkTo { from, to },
                flow::kind_options(),
                None,
                cx,
            );
        }
    }
}
