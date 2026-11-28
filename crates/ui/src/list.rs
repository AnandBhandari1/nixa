use crate::icons::{Icon, icon};
use crate::metrics::{RADIUS_LG, ROW_HEIGHT};
use crate::text;
use crate::theme::Theme;
use gpui::{
    AnyElement, Div, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    Stateful, Styled, div, prelude::FluentBuilder, px,
};

pub fn list(gap: f32) -> Div {
    div().flex().flex_col().gap(px(gap)).px(px(6.0))
}

pub fn list_row(t: &Theme, id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    let hover = t.hover;
    div()
        .id(id)
        .relative()
        .flex_none()
        .h(px(ROW_HEIGHT))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(9.0))
        .rounded(px(RADIUS_LG))
        .text_size(px(text::BASE))
        .text_color(if selected { t.bright } else { t.foreground })
        .cursor_pointer()
        .when(selected, |el| el.bg(t.selected))
        .when(!selected, |el| el.hover(move |s| s.bg(hover)))
}

pub fn row(
    t: &Theme,
    id: impl Into<ElementId>,
    lead: Option<AnyElement>,
    label: impl Into<SharedString>,
    trailing: Option<AnyElement>,
    selected: bool,
) -> Stateful<Div> {
    list_row(t, id, selected)
        .when_some(lead, |el, lead| el.child(lead))
        .child(div().flex_1().min_w(px(0.0)).truncate().child(label.into()))
        .when_some(trailing, |el, trailing| el.child(trailing))
}

pub fn icon_lead(which: Icon, color: gpui::Hsla) -> AnyElement {
    div()
        .flex_none()
        .size(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(which).size(px(15.0)).text_color(color))
        .into_any_element()
}

pub fn selection_bar(color: gpui::Hsla) -> Div {
    div()
        .absolute()
        .left(px(0.0))
        .top(px(7.0))
        .w(px(3.0))
        .h(px(16.0))
        .rounded(px(2.0))
        .bg(color)
}

pub fn empty_row(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .h(px(28.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .text_size(px(12.0))
        .text_color(t.dim)
        .child(text.into())
}
