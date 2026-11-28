use crate::theme::Theme;
use gpui::{
    AppContext, Axis, Bounds, Context, Div, ElementId, Empty, InteractiveElement, IntoElement,
    ParentElement, Pixels, Point, Render, SharedString, Stateful, StatefulInteractiveElement,
    Styled, Window, div, px,
};

pub const HANDLE_HIT: f32 = 6.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitDrag {
    pub id: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub fn split_handle(t: &Theme, id: impl Into<SharedString>, axis: Axis) -> Stateful<Div> {
    let id: SharedString = id.into();
    let line = t.border;
    let active = t.accent;
    let element_id = ElementId::Name(format!("nixa-split-{id}").into());
    let handle = div().id(element_id).flex_none().relative().flex().items_center().justify_center();
    let handle = match axis {
        Axis::Horizontal => handle.w(px(HANDLE_HIT)).h_full().cursor_col_resize(),
        Axis::Vertical => handle.h(px(HANDLE_HIT)).w_full().cursor_row_resize(),
    };
    let bar = match axis {
        Axis::Horizontal => div().w(px(1.0)).h_full(),
        Axis::Vertical => div().h(px(1.0)).w_full(),
    };
    handle
        .group("nixa-split")
        .child(bar.bg(line).group_hover("nixa-split", move |s| s.bg(active)))
        .on_drag(SplitDrag { id }, |_, _, _, cx| cx.new(|_| DragGhost))
}

pub fn size_from_drag(
    axis: Axis,
    edge: Edge,
    bounds: Bounds<Pixels>,
    mouse: Point<Pixels>,
    min: f32,
    max: f32,
) -> f32 {
    let raw = match (axis, edge) {
        (Axis::Horizontal, Edge::Start) => mouse.x - bounds.left(),
        (Axis::Horizontal, Edge::End) => bounds.right() - mouse.x,
        (Axis::Vertical, Edge::Start) => mouse.y - bounds.top(),
        (Axis::Vertical, Edge::End) => bounds.bottom() - mouse.y,
    };
    f32::from(raw).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    fn area() -> Bounds<Pixels> {
        Bounds::new(point(px(100.0), px(50.0)), size(px(1000.0), px(600.0)))
    }

    #[test]
    fn start_edge_measures_from_left() {
        let s = size_from_drag(Axis::Horizontal, Edge::Start, area(), point(px(400.0), px(0.0)), 44.0, 480.0);
        assert_eq!(s, 300.0);
    }

    #[test]
    fn end_edge_measures_from_right_and_clamps() {
        let s = size_from_drag(Axis::Horizontal, Edge::End, area(), point(px(700.0), px(0.0)), 280.0, 900.0);
        assert_eq!(s, 400.0);
        let s = size_from_drag(Axis::Horizontal, Edge::End, area(), point(px(1090.0), px(0.0)), 280.0, 900.0);
        assert_eq!(s, 280.0);
    }

    #[test]
    fn vertical_split() {
        let s = size_from_drag(Axis::Vertical, Edge::Start, area(), point(px(0.0), px(2000.0)), 90.0, 500.0);
        assert_eq!(s, 500.0);
    }
}
