use gpui::prelude::*;
use gpui::{
    ClickEvent, Context, Div, FontWeight, Hsla, MouseButton, SharedString, Stateful, Window, div,
    px,
};
use nixa_ui::Theme;
use nixa_ui::popover::tooltip;

#[derive(Clone, Copy, Debug)]
pub struct Pal {
    pub text: Hsla,
    pub label: Hsla,
    pub hint: Hsla,
    pub accent: Hsla,
    pub ok: Hsla,
    pub bad: Hsla,
    pub warn: Hsla,
    pub info: Hsla,
    pub purple: Hsla,
    pub cyan: Hsla,
    pub bg: Hsla,
    pub card: Hsla,
    pub raised: Hsla,
    pub border: Hsla,
    pub input: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub solid: Hsla,
    pub solid_text: Hsla,
}

pub fn pal(t: &Theme) -> Pal {
    Pal {
        text: t.foreground,
        label: t.muted,
        hint: t.dim,
        accent: t.accent,
        ok: t.green,
        bad: t.red,
        warn: t.yellow,
        info: t.blue,
        purple: t.magenta,
        cyan: t.cyan,
        bg: t.popover,
        card: t.background,
        raised: t.raised,
        border: t.border,
        input: t.border_strong,
        hover: t.hover,
        selected: t.selected,
        solid: t.solid,
        solid_text: t.solid_foreground,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Look {
    pub primary: bool,
    pub danger: bool,
    pub small: bool,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ChipLook {
    pub color: Option<Hsla>,
    pub active: bool,
    pub disabled: bool,
}

fn named(id: &str) -> Stateful<Div> {
    let selector = id.to_string();
    div()
        .id(SharedString::from(id.to_string()))
        .debug_selector(move || selector.clone())
}

pub fn button<V: 'static>(
    p: &Pal,
    id: &str,
    text: impl Into<SharedString>,
    look: Look,
    tip: impl Into<SharedString>,
    cx: &mut Context<V>,
    run: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> Stateful<Div> {
    let fill = if look.primary {
        if look.danger { p.bad } else { p.solid }
    } else {
        p.raised
    };
    let ink = if look.primary {
        if look.danger { p.card } else { p.solid_text }
    } else if look.danger {
        p.bad
    } else {
        p.text
    };
    let edge = if look.primary {
        fill
    } else if look.danger {
        p.bad
    } else {
        p.input
    };
    let tip: SharedString = tip.into();
    let el = named(id)
        .h(px(if look.small { 24.0 } else { 30.0 }))
        .px(px(if look.small { 9.0 } else { 13.0 }))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .rounded(px(if look.small { 6.0 } else { 8.0 }))
        .border_1()
        .border_color(edge)
        .bg(fill)
        .text_size(px(if look.small { 11.5 } else { 12.5 }))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if look.disabled { p.hint } else { ink })
        .when(look.disabled, |el| el.opacity(0.55).cursor_not_allowed())
        .when(!tip.is_empty(), |el| el.tooltip(tooltip(tip)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(text.into());
    if look.disabled {
        return el;
    }
    el.cursor_pointer()
        .hover(|s| s.opacity(0.86))
        .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            run(view, window, cx);
            cx.notify();
        }))
}

pub fn chip_box(p: &Pal, id: &str, text: impl Into<SharedString>, look: ChipLook) -> Stateful<Div> {
    named(id)
        .h(px(22.0))
        .px(px(7.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(if look.active { p.accent } else { p.border })
        .bg(if look.active { p.selected } else { p.raised })
        .text_size(px(11.0))
        .text_color(if look.disabled {
            p.hint
        } else {
            look.color.unwrap_or(p.label)
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(text.into())
}

pub fn chip<V: 'static>(
    p: &Pal,
    id: &str,
    text: impl Into<SharedString>,
    look: ChipLook,
    tip: impl Into<SharedString>,
    cx: &mut Context<V>,
    run: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> Stateful<Div> {
    let tip: SharedString = tip.into();
    let hover = p.hover;
    let el = chip_box(p, id, text, look).when(!tip.is_empty(), |el| el.tooltip(tooltip(tip)));
    if look.disabled {
        return el.cursor_not_allowed();
    }
    el.cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            run(view, window, cx);
            cx.notify();
        }))
}

pub fn tag(
    p: &Pal,
    id: &str,
    text: impl Into<SharedString>,
    color: Hsla,
    tip: impl Into<SharedString>,
) -> Stateful<Div> {
    let tip: SharedString = tip.into();
    chip_box(
        p,
        id,
        text,
        ChipLook {
            color: Some(color),
            ..ChipLook::default()
        },
    )
    .when(!tip.is_empty(), |el| el.tooltip(tooltip(tip)))
}

pub fn hint(p: &Pal, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(p.hint)
        .whitespace_normal()
        .child(text.into())
}

pub fn error_text(p: &Pal, text: impl Into<SharedString>) -> Div {
    div()
        .pt(px(8.0))
        .text_size(px(12.0))
        .text_color(p.bad)
        .whitespace_normal()
        .child(text.into())
}

pub fn ok_text(p: &Pal, text: impl Into<SharedString>) -> Div {
    div()
        .pt(px(8.0))
        .text_size(px(12.0))
        .text_color(p.ok)
        .whitespace_normal()
        .child(text.into())
}

pub fn spinner(p: &Pal, id: &str) -> impl IntoElement {
    nixa_ui::icons::spinner(SharedString::from(id.to_string()), 12.0, p.accent)
}

pub fn root(p: &Pal, id: &str) -> Stateful<Div> {
    named(id)
        .size_full()
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .p(px(18.0))
        .bg(p.bg)
        .text_color(p.text)
        .text_size(px(12.8))
}

pub fn head<V: 'static>(
    p: &Pal,
    id: &str,
    heading: impl Into<SharedString>,
    cx: &mut Context<V>,
    close: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> Div {
    let hint = p.hint;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text)
                .child(heading.into()),
        )
        .child(
            named(&format!("{id}-close"))
                .px(px(6.0))
                .text_size(px(11.0))
                .text_color(hint)
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                    close(view, window, cx);
                    cx.notify();
                }))
                .child("Esc ✕"),
        )
}

pub fn field_box(p: &Pal, id: &str, input: impl IntoElement) -> Stateful<Div> {
    named(id)
        .flex()
        .items_center()
        .rounded(px(8.0))
        .bg(p.card)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(input)
}

pub fn row() -> Div {
    div().flex().flex_row().items_center()
}

pub fn col() -> Div {
    div().flex().flex_col()
}
