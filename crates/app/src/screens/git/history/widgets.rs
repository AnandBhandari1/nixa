use super::words::Outcome;
use gpui::prelude::*;
use gpui::{Div, ElementId, FontWeight, Hsla, SharedString, Stateful, div, px};
use nixa_ui::Theme;
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::popover::tooltip;

pub fn id(name: impl Into<String>) -> ElementId {
    ElementId::Name(SharedString::from(name.into()))
}

pub fn selector(el: Stateful<Div>, name: &str) -> Stateful<Div> {
    let n = name.to_string();
    el.debug_selector(move || n.clone())
}

pub fn small_button(t: &Theme, name: &str, label: impl Into<SharedString>, kind: ButtonKind, off: bool) -> Stateful<Div> {
    let el = button(t, id(name), label, kind)
        .h(px(24.0))
        .px(px(9.0))
        .text_size(px(11.5));
    let el = selector(el, name);
    if off { el.opacity(0.45).cursor_not_allowed() } else { el }
}

pub struct Spec {
    pub name: String,
    pub label: SharedString,
    pub kind: ButtonKind,
    pub off: bool,
    pub tip: Option<String>,
}

impl Spec {
    pub fn new(name: &str, label: impl Into<SharedString>, kind: ButtonKind) -> Spec {
        Spec { name: name.to_string(), label: label.into(), kind, off: false, tip: None }
    }

    pub fn off(mut self, off: bool) -> Spec {
        self.off = off;
        self
    }

    pub fn tip(mut self, tip: Option<String>) -> Spec {
        self.tip = tip;
        self
    }
}

pub fn spec_button(t: &Theme, s: &Spec) -> Stateful<Div> {
    with_tip(small_button(t, &s.name, s.label.clone(), s.kind, s.off), s.tip.clone())
}

pub fn with_tip(el: Stateful<Div>, tip: Option<String>) -> Stateful<Div> {
    match tip {
        Some(text) if !text.is_empty() => el.tooltip(tooltip(text)),
        _ => el,
    }
}

pub fn text_line(text: impl Into<SharedString>, color: Hsla, size: f32) -> Div {
    div().w_full().text_size(px(size)).text_color(color).child(text.into())
}

pub fn hint(t: &Theme, text: impl Into<SharedString>) -> Div {
    text_line(text, t.dim, 11.0)
}

pub fn say(t: &Theme, text: impl Into<SharedString>) -> Div {
    text_line(text, t.foreground, 12.0)
}

pub fn error_text(t: &Theme, text: impl Into<SharedString>) -> Div {
    text_line(text, t.red, 12.0).pt(px(6.0))
}

pub fn ok_text(t: &Theme, text: impl Into<SharedString>) -> Div {
    text_line(text, t.green, 12.0).pt(px(6.0))
}

pub fn check(t: &Theme, name: &str, on: bool, label: impl Into<SharedString>) -> Stateful<Div> {
    let hover = t.hover;
    let el = div()
        .id(id(name))
        .flex()
        .items_center()
        .gap(px(7.0))
        .py(px(3.0))
        .px(px(2.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(
            div()
                .flex_none()
                .size(px(14.0))
                .rounded(px(3.0))
                .border_1()
                .border_color(if on { t.accent } else { t.border_strong })
                .bg(if on { t.accent } else { t.input })
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.0))
                .text_color(t.accent_foreground)
                .child(if on { "✓" } else { "" }),
        )
        .child(div().text_size(px(12.0)).text_color(t.foreground).child(label.into()));
    selector(el, name)
}

pub fn radio(t: &Theme, name: &str, on: bool, label: &str, detail: &str) -> Stateful<Div> {
    let hover = t.hover;
    let el = div()
        .id(id(name))
        .flex()
        .items_start()
        .gap(px(8.0))
        .py(px(4.0))
        .px(px(4.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(
            div()
                .mt(px(2.0))
                .flex_none()
                .size(px(13.0))
                .rounded_full()
                .border_1()
                .border_color(if on { t.accent } else { t.border_strong })
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(7.0)).rounded_full().bg(if on { t.accent } else { gpui::transparent_black() })),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(div().text_size(px(12.0)).text_color(t.foreground).child(label.to_string()))
                .child(div().text_size(px(11.0)).text_color(t.dim).child(detail.to_string())),
        );
    selector(el, name)
}

pub fn spin(t: &Theme, name: &str) -> impl IntoElement {
    nixa_ui::icons::spinner(id(name), 12.0, t.dim)
}

pub fn busy_line(t: &Theme, name: &str, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(spin(t, name))
        .child(hint(t, text))
}

pub fn section(t: &Theme, text: &str) -> Div {
    div()
        .flex_none()
        .px(px(6.0))
        .pt(px(6.0))
        .pb(px(2.0))
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.muted)
        .child(text.to_string())
}

pub fn note_text(o: &Outcome) -> String {
    let title = if o.title.is_empty() { String::new() } else { super::words::sentence(&o.title) };
    [title, o.hint.clone()]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn note_view(t: &Theme, o: &Outcome) -> Div {
    let text = note_text(o);
    let words = if !o.ok {
        error_text(t, text)
    } else if o.warning {
        text_line(text, t.yellow, 12.0).pt(px(6.0))
    } else {
        ok_text(t, text)
    };
    div()
        .flex_none()
        .flex()
        .flex_col()
        .child(words)
        .when(!o.ok && !o.detail.is_empty(), |el| el.child(hint(t, o.detail.clone()).pt(px(2.0))))
}
