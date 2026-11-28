use super::view::TasksView;
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, MouseButton, SharedString, Stateful,
    Window, div, px,
};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon};

pub const LIST_END: f32 = 24.0;

#[derive(Clone, Copy, Default)]
pub struct Look {
    pub off: bool,
    pub on: bool,
    pub color: Option<Hsla>,
    pub icon: Option<Icon>,
}

pub fn look(icon: Icon) -> Look {
    Look {
        icon: Some(icon),
        ..Look::default()
    }
}

impl Look {
    pub fn color(mut self, color: Hsla) -> Look {
        self.color = Some(color);
        self
    }

    pub fn on(mut self, on: bool) -> Look {
        self.on = on;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Look {
        self.off = !enabled;
        self
    }
}

pub fn tint(color: Hsla, alpha: u8) -> Hsla {
    Hsla {
        a: color.a * alpha as f32 / 255.0,
        ..color
    }
}

pub fn chip(
    t: &Theme,
    id: &str,
    label: &str,
    tip: &str,
    l: Look,
    cx: &mut Context<TasksView>,
    run: impl Fn(&mut TasksView, &mut Window, &mut Context<TasksView>) + 'static,
) -> Stateful<Div> {
    let shade = if l.off {
        t.dim
    } else {
        l.color.unwrap_or(if l.on { t.green } else { t.magenta })
    };
    let hover = t.hover;
    let selector = id.to_string();
    let mut el = div()
        .id(SharedString::from(id.to_string()))
        .debug_selector(move || selector.clone())
        .min_h(px(25.0))
        .px(px(8.0))
        .py(px(4.0))
        .gap(px(5.0))
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(if l.on { tint(shade, 0xaa) } else { t.border })
        .when(l.on, |el| el.bg(tint(shade, 0x12)))
        .text_size(px(11.5))
        .text_color(shade)
        .tooltip(tooltip(tip.to_string()))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    if !l.off {
        el = el
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                run(view, window, cx);
                cx.notify();
            }));
    }
    if let Some(which) = l.icon {
        el = el.child(icon(which).size(px(12.0)).text_color(shade));
    }
    if !label.is_empty() {
        el = el.child(label.to_string());
    }
    el
}

pub fn caption(t: &Theme, text: &str) -> Div {
    div()
        .pt(px(4.0))
        .text_size(px(11.0))
        .text_color(t.magenta)
        .child(text.to_string())
}

pub fn line(text: impl Into<SharedString>, color: Hsla, size: f32) -> Div {
    div()
        .text_size(px(size))
        .text_color(color)
        .whitespace_normal()
        .child(text.into())
}

pub fn boxed(t: &Theme, color: Hsla) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p(px(11.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(color)
        .bg(t.surface)
}

pub fn list_end() -> Div {
    div().w_full().h(px(LIST_END)).flex_none()
}

pub fn pill(text: impl Into<SharedString>, color: Hsla, bg: Hsla) -> Div {
    div()
        .flex_none()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(5.0))
        .bg(bg)
        .text_color(color)
        .child(text.into())
}

pub fn info(t: &Theme, label: &str, value: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .w_full()
        .flex()
        .flex_row()
        .gap(px(8.0))
        .items_start()
        .child(
            div()
                .w(px(96.0))
                .flex_none()
                .text_size(px(11.0))
                .text_color(t.dim)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(11.5))
                .text_color(color)
                .whitespace_normal()
                .child(value.into()),
        )
}

pub fn pair(left: impl IntoElement, right: impl IntoElement) -> Div {
    div()
        .w_full()
        .flex()
        .flex_row()
        .gap(px(8.0))
        .items_start()
        .child(div().flex_1().min_w_0().flex().flex_col().child(left))
        .child(div().flex_1().min_w_0().flex().flex_col().child(right))
}

pub fn row() -> Div {
    div().flex().flex_row().items_center()
}

pub fn col() -> Div {
    div().flex().flex_col()
}

pub fn panel_frame(
    t: &Theme,
    title: String,
    half: bool,
    body: Vec<AnyElement>,
    cx: &mut Context<TasksView>,
) -> Div {
    let close = chip(
        t,
        "tasks-panel-close",
        "",
        "Close this panel",
        look(Icon::X),
        cx,
        |view, _, cx| view.close_panel(cx),
    );
    col()
        .when(half, |el| el.flex_1().min_w_0())
        .when(!half, |el| el.w(px(470.0)).flex_none())
        .h_full()
        .min_h_0()
        .border_l_1()
        .border_color(t.border)
        .bg(t.background)
        .child(
            row()
                .w_full()
                .flex_none()
                .px(px(12.0))
                .py(px(9.0))
                .gap(px(8.0))
                .border_b_1()
                .border_color(t.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.0))
                        .child(title),
                )
                .child(close),
        )
        .child(
            col()
                .id("tasks-panel-body")
                .w_full()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(12.0))
                .gap(px(10.0))
                .children(body)
                .child(list_end()),
        )
}
