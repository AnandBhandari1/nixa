use crate::icon_button::{IconSize, icon_button};
use crate::icons::Icon;
use crate::metrics::RADIUS_MODAL;
use crate::popover::shadow;
use crate::text;
use crate::theme::Theme;
use gpui::{
    AnyElement, App, Deferred, Div, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, deferred,
    div, prelude::FluentBuilder, px,
};

pub fn dialog(
    t: &Theme,
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    width: f32,
    body: impl IntoElement,
    footer: Option<AnyElement>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    div()
        .id(id.clone())
        .w(px(width))
        .max_w_full()
        .flex()
        .flex_col()
        .rounded(px(RADIUS_MODAL))
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .shadow(shadow(t))
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .h(px(44.0))
                .pl(px(16.0))
                .pr(px(9.0))
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(t.border)
                .child(
                    div()
                        .text_size(px(text::MD))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.bright)
                        .child(title.into()),
                )
                .child(
                    icon_button(t, crate::child_id(&id, "close"), Icon::X, IconSize::Toolbar, false)
                        .on_click(move |_, window, cx| on_close(window, cx)),
                ),
        )
        .child(div().p(px(16.0)).flex().flex_col().gap(px(10.0)).child(body))
        .when_some(footer, |el, footer| {
            el.child(
                div()
                    .flex_none()
                    .px(px(16.0))
                    .py(px(12.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.0))
                    .border_t_1()
                    .border_color(t.border)
                    .bg(t.chrome)
                    .child(footer),
            )
        })
}

pub fn overlay(
    t: &Theme,
    id: impl Into<ElementId>,
    dialog: impl IntoElement,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> Deferred {
    deferred(
        div()
            .id(id)
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.scrim)
            .occlude()
            .on_click(move |_, window, cx| on_dismiss(window, cx))
            .child(div().id("nixa-modal-body").occlude().on_click(|_, _, cx| cx.stop_propagation()).child(dialog)),
    )
    .with_priority(2)
}
