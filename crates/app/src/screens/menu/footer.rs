use gpui::prelude::*;
use gpui::{App, ClickEvent, Div, Hsla, Stateful, Window, div, px};
use nixa_ui::metrics::SIDEBAR_FOOTER_HEIGHT;
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon};

pub const SETTINGS_TIP: &str = "Nixa settings: every Nixa setting in one place";

fn row(
    t: &Theme,
    id: &'static str,
    which: Icon,
    label: &'static str,
    tip: &'static str,
    color: Hsla,
    narrow: bool,
) -> Stateful<Div> {
    let hover = t.hover;
    let bright = t.bright;
    div()
        .id(id)
        .h(px(28.))
        .flex()
        .items_center()
        .gap(px(9.))
        .rounded(px(8.))
        .cursor_pointer()
        .text_color(color)
        .hover(move |s| s.bg(hover).text_color(bright))
        .tooltip(tooltip(tip))
        .map(|el| {
            if narrow {
                el.w_full().justify_center()
            } else {
                el.px(px(8.))
            }
        })
        .child(icon(which).size(px(14.)).flex_none().text_color(color))
        .when(!narrow, |el| {
            el.child(div().flex_1().min_w_0().truncate().child(label))
        })
}

pub fn render(
    t: &Theme,
    narrow: bool,
    update_ready: bool,
    on_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let settings = row(
        t,
        "nixa-footer-settings",
        Icon::Settings,
        "Settings",
        SETTINGS_TIP,
        t.muted,
        narrow,
    )
    .on_click(on_settings)
    .when(update_ready, |el| {
        el.relative().child(
            div()
                .id("nixa-footer-update")
                .when(narrow, |d| d.absolute().top(px(6.)).right(px(6.)))
                .size(px(6.))
                .flex_none()
                .rounded_full()
                .bg(t.accent),
        )
    });
    div()
        .id("nixa-sidebar-footer")
        .h(px(SIDEBAR_FOOTER_HEIGHT))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(1.))
        .bg(t.chrome)
        .border_t_1()
        .border_color(t.border)
        .px(px(if narrow { 6. } else { 8. }))
        .pt(px(4.))
        .pb(px(8.))
        .text_size(px(13.))
        .child(settings)
}
