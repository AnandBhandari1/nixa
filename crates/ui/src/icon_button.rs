use crate::icons::{Icon, icon};
use crate::theme::Theme;
use gpui::{
    Div, ElementId, FontWeight, Hsla, InteractiveElement, ParentElement, Stateful, Styled, div,
    prelude::FluentBuilder, px,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSize {
    Small,
    Toolbar,
    Rail,
}

impl IconSize {
    pub fn box_size(self) -> f32 {
        match self {
            IconSize::Small => 20.0,
            IconSize::Toolbar => 26.0,
            IconSize::Rail => 32.0,
        }
    }

    pub fn glyph_size(self) -> f32 {
        match self {
            IconSize::Small => 12.0,
            IconSize::Toolbar => 14.0,
            IconSize::Rail => 17.0,
        }
    }

    fn radius(self) -> f32 {
        match self {
            IconSize::Small => 4.0,
            IconSize::Toolbar => 6.0,
            IconSize::Rail => 7.0,
        }
    }
}

pub const GROUP: &str = "nixa-icon-button";

pub fn icon_button(
    t: &Theme,
    id: impl Into<ElementId>,
    which: Icon,
    size: IconSize,
    selected: bool,
) -> Stateful<Div> {
    let fg = if selected { t.foreground } else if size == IconSize::Rail { t.muted } else { t.dim };
    tinted_icon_button(t, id, which, size, selected, fg)
}

pub fn tinted_icon_button(
    t: &Theme,
    id: impl Into<ElementId>,
    which: Icon,
    size: IconSize,
    selected: bool,
    fg: Hsla,
) -> Stateful<Div> {
    let hover_bg = t.hover;
    let hover_fg = t.foreground;
    div()
        .id(id)
        .relative()
        .flex_none()
        .size(px(size.box_size()))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size.radius()))
        .text_color(fg)
        .cursor_pointer()
        .group(GROUP)
        .when(selected, |el| el.bg(t.selected))
        .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
        .child(
            icon(which)
                .size(px(size.glyph_size()))
                .text_color(fg)
                .group_hover(GROUP, move |s| s.text_color(hover_fg)),
        )
}

pub fn count_badge(t: &Theme, count: usize, color: Hsla) -> Div {
    let label = if count > 99 { "99+".to_string() } else { count.to_string() };
    div()
        .absolute()
        .top(px(0.0))
        .right(px(0.0))
        .h(px(14.0))
        .min_w(px(14.0))
        .px(px(3.0))
        .rounded_full()
        .bg(color)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(9.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.background)
        .child(label)
}

pub fn attention_dot(color: Hsla) -> Div {
    div()
        .absolute()
        .top(px(4.0))
        .right(px(4.0))
        .size(px(7.0))
        .rounded_full()
        .bg(color)
}
