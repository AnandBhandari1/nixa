use gpui::prelude::*;
use gpui::{Div, SharedString, Stateful, div, px};
use nixa_ui::Theme;
use nixa_ui::popover::tooltip;

pub fn cell(text: impl Into<SharedString>, width: f32) -> Div {
    div()
        .w(px(width))
        .flex_none()
        .px(px(6.0))
        .truncate()
        .child(text.into())
}

pub fn wrap_cell(text: impl Into<SharedString>, width: f32) -> Div {
    div()
        .w(px(width))
        .flex_none()
        .px(px(6.0))
        .child(text.into())
}

pub fn tip_cell(
    id: String,
    text: impl Into<SharedString>,
    width: f32,
    tip: impl Into<SharedString>,
) -> Stateful<Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .w(px(width))
        .flex_none()
        .px(px(6.0))
        .truncate()
        .tooltip(tooltip(tip))
        .child(text.into())
}

pub fn text_button(
    t: &Theme,
    id: String,
    label: impl Into<SharedString>,
    tip: impl Into<SharedString>,
) -> Stateful<Div> {
    let hover = t.hover;
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(7.0))
        .py(px(4.0))
        .rounded(px(4.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(tip))
        .child(label.into())
}

pub fn chip(t: &Theme, id: String, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    let hover = t.hover;
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(7.0))
        .py(px(3.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .border_1()
        .border_color(if on { t.accent } else { t.border })
        .text_color(if on { t.foreground } else { t.dim })
        .when(on, |el| el.bg(t.selected))
        .hover(move |s| s.bg(hover))
        .child(label.into())
}

pub fn line(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .px(px(12.0))
        .py(px(6.0))
        .text_color(t.dim)
        .child(text.into())
}

pub fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(text.into())
}
