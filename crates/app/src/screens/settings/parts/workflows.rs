use crate::screens::workflows::{self, WorkflowsView};
use gpui::prelude::*;
use gpui::{AnyElement, App, ClickEvent, Entity, div, px};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::popover::tooltip;
use nixa_ui::theme;

pub const PART: &str = "Workflows";
pub const TEXT: &str = "Edit workflows and runs right here, or pop out the full window.";
pub const OPEN: &str = "settings-workflows-open";

pub fn new(cx: &mut App) -> Entity<WorkflowsView> {
    workflows::new(workflows::brain(), cx)
}

pub fn render(view: &Entity<WorkflowsView>, cx: &mut App) -> AnyElement {
    let t = theme(cx).clone();
    div()
        .flex()
        .flex_col()
        .w_full()
        .h_full()
        .min_h_0()
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(8.0))
                .border_b_1()
                .border_color(t.border)
                .child(div().text_size(px(12.0)).text_color(t.dim).child(TEXT))
                .child(
                    button(&t, OPEN, "Open in window", ButtonKind::Secondary)
                        .debug_selector(|| OPEN.to_string())
                        .tooltip(tooltip("Open the full Workflows window"))
                        .on_click(|_: &ClickEvent, _, cx| workflows::open(cx)),
                ),
        )
        .child(div().flex_1().min_h_0().w_full().child(view.clone()))
        .into_any_element()
}
