use crate::workspace::{SIDEBAR_SPLIT, Workspace};
use gpui::prelude::*;
use gpui::{Context, FontWeight, MouseButton, div, px};
use nixa_ui::icon_button::{IconSize, icon_button};
use nixa_ui::metrics::SIDEBAR_NARROW;
use nixa_ui::metrics::BRAND_HEIGHT;
use nixa_ui::popover::tooltip;
use nixa_ui::split::split_handle;
use nixa_ui::{Icon, Theme, theme};

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let t = theme(cx).clone();
    let narrow = ws.sidebar_w < SIDEBAR_NARROW;
    div()
        .id("sidebar")
        .debug_selector(|| "sidebar".into())
        .relative()
        .w(px(ws.sidebar_w))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .bg(t.chrome)
        .border_l_1()
        .border_color(t.border)
        .child(brand(&t, narrow, cx))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(spaces_list(ws))
                .child(crate::screens::agents::render(&ws.hub.agents)),
        )
        .child(crate::screens::menu::footer::render(
            &t,
            narrow,
            false,
            cx.listener(|ws, _, _, cx| ws.open_settings("General", cx)),
        ))
        .child(
            split_handle(&t, SIDEBAR_SPLIT, gpui::Axis::Horizontal)
                .absolute()
                .top_0()
                .left(px(-3.)),
        )
}

// Navigation brand controls rendered with Nixa's GPUI actions.
fn brand(t: &Theme, narrow: bool, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    div()
        .h(px(BRAND_HEIGHT))
        .window_control_area(gpui::WindowControlArea::Drag)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(12.))
        .pr(px(10.))
        .when(!narrow, |el| {
            el.child(
                div()
                    .size(px(20.))
                    .flex_none()
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.accent)
                    .text_color(t.background)
                    .text_size(px(12.))
                    .child("N"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.bright)
                    .child("Nixa"),
            )
            .child(
                icon_button(t, "brand-search", Icon::Search, IconSize::Toolbar, false)
                    .tooltip(tooltip("Go to (⌘P)"))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|ws, _, window, cx| ws.open_picker(window, cx))),
            )
            .child(
                icon_button(t, "brand-sidebar", Icon::Sidebar, IconSize::Toolbar, false)
                    .tooltip(tooltip("Toggle sidebar (⌘B)"))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|ws, _, _, cx| ws.toggle_sidebar(cx))),
            )
        })
}

fn spaces_list(ws: &Workspace) -> impl IntoElement + use<> {
    div()
        .flex_1()
        .min_h(px(90.))
        .flex()
        .flex_col()
        .child(crate::screens::spaces::render(&ws.spaces_view))
}
