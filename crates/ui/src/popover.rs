use crate::metrics::RADIUS_POPOVER;
use crate::text;
use crate::theme::{Theme, theme};
use gpui::{
    AnyView, App, AppContext, BoxShadow, Context, Corner, Deferred, Div, IntoElement, ParentElement,
    Pixels, Point, Render, SharedString, Styled, Window, anchored, deferred, div, point, px,
};

pub fn shadow(t: &Theme) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: t.shadow,
            offset: point(px(0.0), px(8.0)),
            blur_radius: px(24.0),
            spread_radius: px(0.0),
        },
        BoxShadow {
            color: t.shadow.opacity(0.25),
            offset: point(px(0.0), px(1.0)),
            blur_radius: px(3.0),
            spread_radius: px(0.0),
        },
    ]
}

pub fn popover(t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .p(px(5.0))
        .rounded(px(RADIUS_POPOVER))
        .bg(t.popover)
        .border_1()
        .border_color(t.border)
        .shadow(shadow(t))
        .text_size(px(text::MENU))
        .text_color(t.foreground)
}

pub fn float_at(position: Point<Pixels>, corner: Corner, child: impl IntoElement) -> Deferred {
    deferred(
        anchored()
            .position(position)
            .anchor(corner)
            .snap_to_window_with_margin(px(8.0))
            .child(child),
    )
    .with_priority(1)
}

pub struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div().pl(px(8.0)).pt(px(10.0)).child(
            div()
                .max_w(px(320.0))
                .px(px(8.0))
                .py(px(5.0))
                .rounded(px(6.0))
                .bg(t.raised)
                .border_1()
                .border_color(t.border_strong)
                .shadow(shadow(t))
                .text_size(px(text::SM))
                .text_color(t.bright)
                .child(self.text.clone()),
        )
    }
}

pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_window, cx| {
        let text = text.clone();
        cx.new(|_| Tooltip { text }).into()
    }
}
