use super::format::{self, usage_tip, usage_words};
use super::list::quiet_count;
use super::parts::stop_down;
use super::{ChatView, send_typed};
use gpui::prelude::*;
use gpui::{AnyElement, Context, SharedString, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::theme::tint;
use nixa_ui::{Icon, Theme, icon};

pub fn toggle_skipped(view: &mut ChatView) {
    view.show_quiet = !view.show_quiet;
    if !view.show_quiet {
        let quiet: Vec<u64> = view
            .snap
            .items
            .iter()
            .filter(|i| i.quiet)
            .map(|i| i.id)
            .collect();
        view.shown.retain(|id| !quiet.contains(id));
    }
}

pub fn render(view: &ChatView, t: &Theme, cx: &mut Context<ChatView>) -> AnyElement {
    let name = view.snap.name().to_string();
    let quiet = quiet_count(view);
    let send_bg = tint(t.blue, 0.15);
    let send_hover = tint(t.blue, 0.25);
    let send = stop_down(
        div()
            .id("chat-send")
            .debug_selector(|| "chat-send".into())
            .size(px(30.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.))
            .cursor_pointer()
            .bg(send_bg)
            .hover(move |s| s.bg(send_hover))
            .tooltip(tooltip(format!("Send to {name} (Enter). Shift+Enter starts a new line."))),
    )
    .on_click(cx.listener(|view, _, _, cx| {
        cx.stop_propagation();
        let text = view.input.read(cx).value().to_string();
        send_typed(view, text, cx);
    }))
    .child(icon(Icon::Send).size(px(14.)).text_color(t.blue));
    let show = view.show_quiet;
    let hover = t.hover;
    let skipped = (quiet > 0).then(|| {
        let tip = if show {
            format!("Hide the lines {name} skipped")
        } else {
            format!("Show the lines {name} did not send to its brain: short sounds like \"mm\" and a lone \"thanks\"")
        };
        stop_down(
            div()
                .id("chat-skipped")
                .debug_selector(|| "chat-skipped".into())
                .flex_none()
                .px(px(6.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(10.5))
                .text_color(if show { t.foreground } else { t.dim })
                .hover(move |s| s.bg(hover))
                .tooltip(tooltip(tip)),
        )
        .on_click(cx.listener(|view, _, _, cx| {
            cx.stop_propagation();
            toggle_skipped(view);
            cx.notify();
        }))
        .child(if show { format!("Hide skipped ({quiet})") } else { format!("Skipped ({quiet})") })
    });
    let usage = view.snap.usage.as_ref().map(|u| {
        div()
            .id("chat-usage")
            .flex_none()
            .text_size(px(10.5))
            .text_color(t.dim)
            .tooltip(tooltip(usage_tip(u)))
            .child(SharedString::from(usage_words(u)))
    });
    div()
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .px(px(10.))
        .pt(px(8.))
        .pb(px(6.))
        .gap(px(5.))
        .border_t_1()
        .border_color(t.border)
        .child(
            div()
                .w_full()
                .flex()
                .gap(px(6.))
                .items_end()
                .child(
                    div()
                        .id("chat-type")
                        .debug_selector(|| "chat-type".into())
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(view.input.clone()),
                )
                .child(send),
        )
        .child(
            div()
                .w_full()
                .h(px(18.))
                .flex()
                .gap(px(6.))
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(10.5))
                        .text_color(t.dim)
                        .child(format::hint(&view.snap)),
                )
                .children(skipped)
                .children(usage),
        )
        .into_any_element()
}
