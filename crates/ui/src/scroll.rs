use crate::theme::Theme;
use gpui::{
    Bounds, Div, ElementId, InteractiveElement, IntoElement, ParentElement, ScrollHandle,
    StatefulInteractiveElement, Styled, canvas, div, fill, point, px, size,
};

pub const MIN_THUMB: f32 = 24.0;

pub fn thumb(viewport: f32, max_offset: f32, offset: f32) -> Option<(f32, f32)> {
    if viewport <= 0.0 || max_offset <= 0.5 {
        return None;
    }
    let content = viewport + max_offset;
    let height = (viewport * viewport / content).clamp(MIN_THUMB.min(viewport), viewport);
    let scrolled = (-offset).clamp(0.0, max_offset);
    let top = scrolled / max_offset * (viewport - height);
    Some((top, height))
}

pub fn scroll_area(
    t: &Theme,
    id: impl Into<ElementId>,
    handle: &ScrollHandle,
    content: impl IntoElement,
) -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .overflow_hidden()
        .child(
            div()
                .id(id)
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .track_scroll(handle)
                .child(content),
        )
        .child(scrollbar(t, handle))
}

pub fn scrollbar(t: &Theme, handle: &ScrollHandle) -> impl IntoElement {
    let handle = handle.clone();
    let color = t.border_strong.opacity(0.8);
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let viewport = handle.bounds();
            let max = f32::from(handle.max_offset().height);
            let offset = f32::from(handle.offset().y);
            if let Some((top, height)) = thumb(f32::from(viewport.size.height), max, offset) {
                let bounds = Bounds::new(
                    point(viewport.right() - px(6.0), viewport.top() + px(top)),
                    size(px(4.0), px(height)),
                );
                window.paint_quad(fill(bounds, color).corner_radii(px(2.0)));
            }
        },
    ).absolute().size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_thumb_without_overflow() {
        assert_eq!(thumb(300.0, 0.0, 0.0), None);
        assert_eq!(thumb(0.0, 100.0, 0.0), None);
    }

    #[test]
    fn thumb_moves_to_bottom() {
        let (top, height) = thumb(300.0, 300.0, 0.0).unwrap();
        assert_eq!(top, 0.0);
        assert_eq!(height, 150.0);
        let (top, height) = thumb(300.0, 300.0, -300.0).unwrap();
        assert_eq!(top + height, 300.0);
    }

    #[test]
    fn thumb_has_min_size() {
        let (_, height) = thumb(100.0, 100_000.0, 0.0).unwrap();
        assert_eq!(height, MIN_THUMB);
    }
}
