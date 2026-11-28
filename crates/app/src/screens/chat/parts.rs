use super::format::{Mark, Tone};
use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, AnyElement, Div, ElementId, FontWeight, Hsla, MouseButton,
    SharedString, Stateful, Transformation, div, percentage, px,
};
use nixa_ui::popover::tooltip;
use nixa_ui::theme::tint;
use nixa_ui::{Icon, Theme, icon};
use std::time::Duration;

pub const KIND_TAG_PR: f32 = 78.0;

pub fn tone(t: &Theme, tone: Tone) -> Hsla {
    match tone {
        Tone::Blue => t.blue,
        Tone::Red => t.red,
        Tone::Green => t.green,
        Tone::Yellow => t.yellow,
        Tone::Magenta => t.magenta,
        Tone::Cyan => t.cyan,
    }
}

pub fn mark(which: Mark, color: Hsla) -> AnyElement {
    let shape: AnyElement = match which {
        Mark::Ring => div()
            .size(px(10.))
            .rounded_full()
            .border_2()
            .border_color(color)
            .into_any_element(),
        Mark::Pulse => div()
            .size(px(9.))
            .rounded_full()
            .bg(color)
            .into_any_element(),
        Mark::Hollow => div()
            .size(px(8.))
            .rounded_full()
            .border_1()
            .border_color(color)
            .into_any_element(),
        Mark::Spin => icon(Icon::Spinner)
            .size(px(12.))
            .text_color(color)
            .with_animation(
                "chat-status-spin",
                Animation::new(Duration::from_millis(900)).repeat(),
                |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
            )
            .into_any_element(),
    };
    div()
        .size(px(12.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(shape)
        .into_any_element()
}

pub fn stop_down(el: Stateful<Div>) -> Stateful<Div> {
    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

pub fn head_chip(
    t: &Theme,
    id: impl Into<ElementId>,
    which: Icon,
    label: impl Into<SharedString>,
    tip: impl Into<SharedString>,
    color: Hsla,
    max_w: Option<f32>,
) -> Stateful<Div> {
    let hover = t.hover;
    let label: SharedString = label.into();
    let text = match max_w {
        Some(w) => div().min_w(px(0.)).max_w(px(w)).truncate().child(label),
        None => div().whitespace_nowrap().child(label),
    };
    stop_down(
        div()
            .id(id)
            .h(px(24.))
            .px(px(7.))
            .gap(px(4.))
            .flex()
            .flex_none()
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(t.border)
            .text_size(px(11.))
            .text_color(color)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(tip))
            .when(max_w.is_some(), |el| el.flex_shrink().min_w(px(0.)))
            .child(icon(which).size(px(12.)).flex_none().text_color(color))
            .child(text),
    )
}

pub fn box_chip(
    t: &Theme,
    id: impl Into<ElementId>,
    which: Icon,
    label: impl Into<SharedString>,
    tip: impl Into<SharedString>,
    color: Hsla,
    enabled: bool,
) -> Stateful<Div> {
    let hover = t.hover;
    let fg = if enabled { color } else { t.dim };
    let label: SharedString = label.into();
    stop_down(
        div()
            .id(id)
            .h(px(24.))
            .px(px(8.))
            .gap(px(5.))
            .flex()
            .flex_none()
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(t.border)
            .text_size(px(11.5))
            .text_color(fg)
            .tooltip(tooltip(tip))
            .when(enabled, |el| {
                el.cursor_pointer().hover(move |s| s.bg(hover))
            })
            .child(icon(which).size(px(13.)).text_color(fg))
            .when(!label.is_empty(), |el| el.child(label)),
    )
}

pub fn icon_btn(
    t: &Theme,
    id: impl Into<ElementId>,
    which: Icon,
    tip: impl Into<SharedString>,
    color: Hsla,
    on: bool,
) -> Stateful<Div> {
    let hover = t.hover;
    stop_down(
        div()
            .id(id)
            .size(px(26.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .when(on, |el| el.bg(t.selected))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(tip))
            .child(icon(which).size(px(14.)).text_color(color)),
    )
}

pub fn avatar(color: Hsla, size: f32, letter: impl Into<SharedString>) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(tint(color, 0.15))
        .border_1()
        .border_color(tint(color, 0.4))
        .text_size(px(size * 0.46))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(letter.into())
}

pub fn first_letter(text: &str) -> String {
    text.trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "A".into())
}

pub fn kind_tag(t: &Theme, label: impl Into<SharedString>) -> Div {
    div()
        .absolute()
        .top(px(1.))
        .right(px(6.))
        .text_size(px(9.))
        .line_height(px(10.))
        .text_color(tint(t.dim, 0.6))
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_for_avatars() {
        assert_eq!(first_letter(" codex"), "C");
        assert_eq!(first_letter(""), "A");
        assert_eq!(first_letter("ärger"), "Ä");
    }
}
