use crate::icons::{Icon, icon, spinner};
use crate::text;
use crate::theme::{Theme, status_color, status_label};
use gpui::{
    AnyElement, Div, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, Stateful, Styled, div, px,
};
use nixa_protocol::{Status, TermInfo};

pub fn mono_box(t: &Theme, label: &str, size: f32) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .bg(t.raised)
        .text_size(px(size * 0.5))
        .font_weight(FontWeight::BOLD)
        .text_color(t.bright)
        .child(SharedString::from(label.to_string()))
}

pub fn program_badge(t: &Theme, mono: Option<&str>, size: f32) -> AnyElement {
    match mono {
        Some(label) => mono_box(t, label, size).into_any_element(),
        None => icon(Icon::Terminal)
            .size(px(size - 2.))
            .text_color(t.dim)
            .into_any_element(),
    }
}

pub fn term_mark(t: &Theme, id: impl Into<ElementId>, term: Option<&TermInfo>) -> AnyElement {
    let Some(term) = term else {
        return div().into_any_element();
    };
    if !term.running {
        return chip("Ended", t.dim).into_any_element();
    }
    match term.status {
        Some(status @ (Status::Blocked | Status::Error)) => {
            status_chip(t, status).into_any_element()
        }
        Some(status) => status_dot(t, id, status),
        None => div().into_any_element(),
    }
}

pub fn chip(label: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .flex_none()
        .h(px(18.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(5.0))
        .bg(color.opacity(0.15))
        .text_size(px(text::XS))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(label.into())
}

pub fn outline_chip(t: &Theme, label: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .h(px(20.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(t.border)
        .text_size(px(11.0))
        .text_color(t.muted)
        .child(label.into())
}

pub fn count_chip(count: usize, color: Hsla) -> Div {
    div()
        .flex_none()
        .h(px(16.0))
        .px(px(4.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .bg(color.opacity(0.15))
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(count.to_string())
}

pub fn status_chip(t: &Theme, status: Status) -> Div {
    chip(status_label(status), status_color(t, status))
}

pub fn status_dot(t: &Theme, id: impl Into<ElementId>, status: Status) -> AnyElement {
    let color = status_color(t, status);
    match status {
        Status::Working => spinner(id, 12.0, color).into_any_element(),
        Status::Blocked => div()
            .flex_none()
            .size(px(14.0))
            .rounded(px(4.0))
            .bg(color.opacity(0.15))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(10.5))
            .font_weight(FontWeight::BOLD)
            .text_color(color)
            .child("!")
            .into_any_element(),
        Status::Idle => div()
            .flex_none()
            .size(px(7.0))
            .mx(px(3.0))
            .rounded_full()
            .border_1()
            .border_color(color)
            .into_any_element(),
        Status::Done | Status::Error => div()
            .flex_none()
            .size(px(7.0))
            .mx(px(3.0))
            .rounded_full()
            .bg(color)
            .into_any_element(),
    }
}

pub fn model_chip(
    t: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    let hover = t.hover;
    div()
        .id(id)
        .flex_none()
        .h(px(24.0))
        .pl(px(8.0))
        .pr(px(5.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(t.border)
        .bg(t.raised)
        .text_size(px(11.5))
        .text_color(t.foreground)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(icon(Icon::Zap).size(px(12.0)).text_color(t.magenta))
        .child(label.into())
        .child(icon(Icon::ChevronDown).size(px(12.0)).text_color(t.dim))
}
