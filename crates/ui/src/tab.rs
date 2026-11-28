use crate::icons::{Icon, icon};
use crate::metrics::{PANE_HEADER_HEIGHT, TAB_BAR_HEIGHT};
use crate::text;
use crate::theme::Theme;
use gpui::{
    AnyElement, Div, ElementId, InteractiveElement, ParentElement, SharedString, Stateful, Styled,
    div, prelude::FluentBuilder, px,
};

const CLOSE_GROUP: &str = "nixa-tab-close";

pub fn tab_bar(t: &Theme) -> Div {
    div()
        .flex_none()
        .h(px(TAB_BAR_HEIGHT))
        .w_full()
        .min_w(px(0.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(3.0))
        .bg(t.chrome)
        .border_b_1()
        .border_color(t.border)
}

pub fn tab(
    t: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    lead: Option<AnyElement>,
    active: bool,
) -> Stateful<Div> {
    let hover = t.hover;
    div()
        .id(id)
        .relative()
        .flex_none()
        .h(px(28.0))
        .max_w(px(200.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .rounded(px(7.0))
        .text_size(px(12.5))
        .text_color(if active { t.bright } else { t.muted })
        .cursor_pointer()
        .when(active, |el| el.bg(t.selected))
        .when(!active, |el| el.hover(move |s| s.bg(hover)))
        .when_some(lead, |el, lead| el.child(lead))
        .child(div().min_w(px(0.0)).truncate().child(label.into()))
        .when(active, |el| {
            el.child(
                div()
                    .absolute()
                    .left(px(8.0))
                    .right(px(8.0))
                    .bottom(px(-7.0))
                    .h(px(2.0))
                    .rounded(px(1.0))
                    .bg(t.accent),
            )
        })
}

pub fn closable_tab(
    t: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    lead: Option<AnyElement>,
    active: bool,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    let hover = t.hover;
    let bright = t.bright;
    tab(t, id.clone(), label, lead, active).pr(px(4.0)).child(
        div()
            .id(crate::child_id(&id, "close"))
            .flex_none()
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .group(CLOSE_GROUP)
            .hover(move |s| s.bg(hover))
            .child(
                icon(Icon::X)
                    .size(px(11.0))
                    .text_color(t.dim)
                    .group_hover(CLOSE_GROUP, move |s| s.text_color(bright)),
            ),
    )
}

pub fn segmented(t: &Theme) -> Div {
    div()
        .flex_none()
        .h(px(28.0))
        .p(px(2.0))
        .flex()
        .items_center()
        .gap(px(2.0))
        .rounded(px(7.0))
        .bg(t.input)
        .border_1()
        .border_color(t.border)
}

pub fn segment(
    t: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    let hover = t.hover;
    div()
        .id(id)
        .h_full()
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .text_size(px(text::SM))
        .text_color(if active { t.bright } else { t.muted })
        .cursor_pointer()
        .when(active, |el| el.bg(t.selected))
        .when(!active, |el| el.hover(move |s| s.bg(hover)))
        .child(label.into())
}

pub fn pane_header(t: &Theme, title: impl Into<SharedString>, focused: bool) -> Div {
    div()
        .flex_none()
        .h(px(PANE_HEADER_HEIGHT))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .border_b_1()
        .border_color(t.border)
        .text_size(px(text::SM))
        .text_color(if focused { t.foreground } else { t.dim })
        .child(icon(Icon::Terminal).size(px(13.0)).text_color(if focused { t.accent } else { t.dim }))
        .child(div().flex_1().min_w(px(0.0)).truncate().child(title.into()))
}

pub fn pane(t: &Theme, focused: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .rounded(px(crate::metrics::PANE_RADIUS))
        .bg(t.background)
        .border_1()
        .border_color(if focused { t.accent } else { t.border })
}
