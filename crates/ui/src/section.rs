use crate::metrics::SECTION_HEADER_HEIGHT;
use crate::text;
use crate::theme::Theme;
use gpui::{Div, FontWeight, ParentElement, SharedString, Styled, div, px};

pub fn section_header(t: &Theme, title: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .h(px(SECTION_HEADER_HEIGHT))
        .pl(px(16.0))
        .pr(px(10.0))
        .flex()
        .items_center()
        .justify_between()
        .text_size(px(text::SM))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.dim)
        .child(title.into())
}

pub fn actions() -> Div {
    div().flex().items_center().gap(px(2.0))
}

pub fn separator(t: &Theme) -> Div {
    div().flex_none().h(px(1.0)).mx(px(7.0)).my(px(4.0)).bg(t.border)
}

pub fn rule(t: &Theme) -> Div {
    div().flex_none().h(px(1.0)).w_full().bg(t.border)
}

pub fn short_rule(t: &Theme) -> Div {
    div().flex_none().w(px(20.0)).h(px(1.0)).my(px(2.0)).bg(t.border)
}

pub fn group_title(t: &Theme, title: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(text::XS))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.dim)
        .child(title.into())
}
