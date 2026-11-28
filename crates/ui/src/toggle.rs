use crate::text;
use crate::theme::Theme;
use gpui::{
    Div, ElementId, InteractiveElement, ParentElement, SharedString, Stateful, Styled, div,
    prelude::FluentBuilder, px,
};

pub const TRACK_WIDTH: f32 = 30.0;
pub const TRACK_HEIGHT: f32 = 17.0;
pub const KNOB: f32 = 13.0;

pub fn knob_left(on: bool) -> f32 {
    if on { TRACK_WIDTH - KNOB - 2.0 } else { 2.0 }
}

pub fn toggle(t: &Theme, id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    let track = if on { t.accent } else { t.border_strong };
    let knob = if on { t.accent_foreground } else { t.foreground };
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(TRACK_WIDTH))
        .h(px(TRACK_HEIGHT))
        .rounded_full()
        .bg(track)
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .top(px(2.0))
                .left(px(knob_left(on)))
                .size(px(KNOB))
                .rounded_full()
                .bg(knob),
        )
}

pub fn toggle_row(
    t: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    note: Option<SharedString>,
    on: bool,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    let hover = t.hover;
    div()
        .id(id.clone())
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(text::label(t, label))
                .when_some(note, |el, note| el.child(text::muted(t, note))),
        )
        .child(toggle(t, crate::child_id(&id, "switch"), on))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_moves_inside_track() {
        assert_eq!(knob_left(false), 2.0);
        assert_eq!(knob_left(true) + KNOB + 2.0, TRACK_WIDTH);
    }
}
