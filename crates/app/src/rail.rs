// The rail sits on the left; its top part holds the window lights.
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{Context, WindowControlArea, div, px};
use nixa_ui::metrics::{RAIL_WIDTH, TAB_BAR_HEIGHT};
use nixa_ui::theme;

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let t = theme(cx).clone();
    div()
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .bg(t.chrome)
        .border_r_1()
        .border_color(t.border)
        .child(
            div()
                .w(px(RAIL_WIDTH))
                .h(px(TAB_BAR_HEIGHT))
                .flex_none()
                .border_b_1()
                .border_color(t.border)
                .window_control_area(WindowControlArea::Drag),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .child(crate::screens::rail::host::host(ws, cx)),
        )
}
