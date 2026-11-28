use crate::icons::{Icon, icon};
use crate::text;
use crate::theme::Theme;
use gpui::{
    Div, ElementId, FontWeight, InteractiveElement, ParentElement, SharedString, Stateful, Styled,
    div, prelude::FluentBuilder, px,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

fn colors(t: &Theme, kind: ButtonKind) -> (gpui::Hsla, gpui::Hsla, gpui::Hsla, gpui::Hsla) {
    match kind {
        ButtonKind::Primary => (t.accent, t.accent_foreground, t.accent_hover, t.accent),
        ButtonKind::Secondary => (t.raised, t.foreground, t.hover, t.border),
        ButtonKind::Ghost => (
            gpui::transparent_black(),
            t.muted,
            t.hover,
            gpui::transparent_black(),
        ),
        ButtonKind::Danger => (
            t.red.opacity(0.14),
            t.red,
            t.red.opacity(0.24),
            t.red.opacity(0.35),
        ),
    }
}

fn base(t: &Theme, id: impl Into<ElementId>, kind: ButtonKind) -> Stateful<Div> {
    let (bg, fg, hover, border) = colors(t, kind);
    let hover_fg = if kind == ButtonKind::Ghost {
        t.foreground
    } else {
        fg
    };
    div()
        .id(id)
        .flex_none()
        .h(px(28.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_size(px(text::MENU))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(hover_fg))
}

pub fn button(
    t: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
) -> Stateful<Div> {
    base(t, id, kind).child(label.into())
}

pub fn icon_label_button(
    t: &Theme,
    id: impl Into<ElementId>,
    which: Icon,
    label: impl Into<SharedString>,
    kind: ButtonKind,
) -> Stateful<Div> {
    let fg = colors(t, kind).1;
    base(t, id, kind)
        .pl(px(9.0))
        .child(icon(which).size(px(14.0)).text_color(fg))
        .child(label.into())
}

pub fn disabled(el: Stateful<Div>, off: bool) -> Stateful<Div> {
    el.when(off, |el| {
        el.opacity(0.45)
            .cursor_not_allowed()
            .capture_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .capture_any_mouse_up(|_, _, cx| cx.stop_propagation())
            .capture_key_up(|event, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                }
            })
    })
}
