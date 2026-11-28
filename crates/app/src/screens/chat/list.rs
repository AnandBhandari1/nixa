use super::format;
use super::kinds::{is_brain_event, is_sent};
use super::model::Item;
use super::parts::{avatar, first_letter, stop_down};
use super::{ChatView, jump_newest};
use gpui::prelude::*;
use gpui::{AnyElement, ClipboardItem, Context, FontWeight, Hsla, Pixels, SharedString, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::theme::tint;
use nixa_ui::{Icon, Theme, icon};

pub const RENDER_LAST: usize = 150;

pub fn chat_font(view: &ChatView) -> gpui::Font {
    let family = view
        .font_family
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .unwrap_or(nixa_termview::DEFAULT_FONT);
    nixa_termview::terminal_font(family)
}
pub const AT_BOTTOM_PX: f32 = 16.0;
const CHAT_FONT: f32 = 12.5;
const CHAT_LINE: f32 = 18.0;
const ROUTINE_FONT: f32 = 10.0;
const ROUTINE_LINE: f32 = 13.0;
const INFO_FONT: f32 = 11.0;
const INFO_LINE: f32 = 15.0;
const META_FONT: f32 = 9.0;
const FOOT_LINE: f32 = 12.0;
const ROW_TOP: f32 = 10.0;
const ROW_PAD: f32 = 12.0;
pub const DRAFT_ID: u64 = u64::MAX;

pub fn visible(view: &ChatView) -> Vec<&Item> {
    let list: Vec<&Item> = view
        .snap
        .items
        .iter()
        .filter(|i| !is_brain_event(i))
        .filter(|i| view.show_quiet || !i.quiet || view.shown.contains(&i.id))
        .collect();
    let start = list.len().saturating_sub(RENDER_LAST);
    list[start..].to_vec()
}

pub fn quiet_count(view: &ChatView) -> usize {
    view.snap.items.iter().filter(|i| i.quiet).count()
}

fn tray_key(view: &ChatView) -> String {
    let confirm = view
        .snap
        .confirm
        .as_ref()
        .map(|c| c.id.as_str())
        .unwrap_or("");
    format!("{}:{}", confirm, view.clean_ask)
}

fn distance_from_bottom(view: &ChatView) -> f32 {
    let max = f32::from(view.scroll.max_offset().height);
    let top = f32::from(view.scroll.offset().y);
    (max + top).max(0.0)
}

pub fn follow(view: &mut ChatView) {
    if view.watch_scroll {
        view.watch_scroll = false;
        view.user_moved = distance_from_bottom(view) > AT_BOTTOM_PX;
    }
    let ids: Vec<u64> = visible(view).iter().map(|i| i.id).collect();
    view.shown = ids.iter().copied().collect();
    let tray = tray_key(view);
    let moved = tray != view.tray_key;
    view.tray_key = tray;
    let last = ids.last().copied();
    let fresh = match (last, view.last_id) {
        (Some(l), Some(seen)) => l > seen,
        (Some(_), None) => true,
        _ => false,
    };
    if let Some(l) = last {
        view.last_id = Some(view.last_id.map_or(l, |seen| seen.max(l)));
    }
    if ids.is_empty() && view.snap.draft.is_empty() {
        view.user_moved = false;
        return;
    }
    if view.follow {
        view.follow = false;
        view.user_moved = false;
        view.scroll.scroll_to_bottom();
        return;
    }
    if view.user_moved {
        return;
    }
    if fresh || moved || !view.snap.draft.is_empty() {
        view.scroll.scroll_to_bottom();
    }
}

struct Style {
    bg: Hsla,
    edge: Hsla,
    text: Hsla,
    italic: bool,
    small: bool,
}

fn foot(view: &ChatView, t: &Theme, item: &Item, now: u64) -> AnyElement {
    let text = if item.id == DRAFT_ID {
        format!("{} · writing…", view.snap.name())
    } else {
        format::foot_text(item, view.snap.name(), now, view.offset_min)
    };
    let copy_text = item.text.clone();
    let id = item.id;
    div()
        .w_full()
        .px(px(10.))
        .py(px(4.))
        .flex()
        .gap(px(6.))
        .items_start()
        .border_t_1()
        .border_color(tint(t.border, 0.8))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(META_FONT))
                .line_height(px(FOOT_LINE))
                .text_color(tint(t.dim, 0.6))
                .child(text),
        )
        .when(id != DRAFT_ID && item.who == "user", |el| {
            let me = view.me.clone();
            let again = item.text.clone();
            el.child(foot_btn(t, format!("chat-resend-{id}"), "Send this message again", Icon::Refresh).on_click(move |_, _, cx| {
                cx.stop_propagation();
                let text = again.clone();
                me.update(cx, |view, cx| {
                    jump_newest(view);
                    super::send(super::wire::Command::Typed(text), cx);
                })
                .ok();
            }))
        })
        .when(id != DRAFT_ID, |el| {
            el.child(foot_btn(t, format!("chat-copy-{id}"), "Copy this message", Icon::Copy).on_click(move |_, _, cx| {
                cx.stop_propagation();
                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
            }))
        })
        .into_any_element()
}

fn foot_btn(t: &Theme, key: String, tip: &'static str, which: Icon) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    let selector = key.clone();
    stop_down(
        div()
            .id(SharedString::from(key))
            .debug_selector(move || selector.clone())
            .size(px(14.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(tip)),
    )
    .child(icon(which).size(px(10.)).text_color(tint(t.dim, 0.8)))
}

fn card_width(view: &ChatView, share: f32) -> Pixels {
    px(((view.width - 2.0 - 2.0 * ROW_PAD) * share).max(2.0))
}

fn message_body(item: &Item, card_width: Pixels) -> gpui::Div {
    let id = item.id;
    div()
        .debug_selector(move || format!("chat-message-{id}"))
        .w(card_width - px(2.0))
        .whitespace_normal()
        .child(item.text.clone())
}

fn card(
    view: &ChatView,
    t: &Theme,
    item: &Item,
    style: Style,
    now: u64,
    width: Pixels,
) -> gpui::Div {
    let (font, line) = if style.small {
        (ROUTINE_FONT, ROUTINE_LINE)
    } else {
        (CHAT_FONT, CHAT_LINE)
    };
    let text = message_body(item, width)
        .px(px(10.))
        .py(px(6.))
        .font(chat_font(view))
        .text_size(px(font))
        .line_height(px(line))
        .text_color(style.text)
        .when(style.italic, |el| el.italic());
    div()
        .w(width)
        .flex_none()
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(style.edge)
        .bg(style.bg)
        .child(text)
        .child(foot(view, t, item, now))
}

fn routine_style(t: &Theme) -> Style {
    Style {
        bg: gpui::transparent_black(),
        edge: tint(t.border, 0.6),
        text: t.dim,
        italic: false,
        small: true,
    }
}

fn row_box(id: u64) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(format!("chat-row-{id}")))
        .w_full()
        .flex_none()
        .flex()
        .px(px(ROW_PAD))
        .pt(px(ROW_TOP))
        .items_start()
}

fn right_row(view: &ChatView, t: &Theme, item: &Item, style: Style, now: u64) -> AnyElement {
    row_box(item.id)
        .justify_end()
        .child(card(view, t, item, style, now, card_width(view, 0.85)))
        .into_any_element()
}

fn left_row(
    view: &ChatView,
    t: &Theme,
    item: &Item,
    face: (Hsla, String),
    style: Style,
    now: u64,
) -> AnyElement {
    let (shade, letter) = face;
    row_box(item.id)
        .gap(px(8.))
        .child(avatar(shade, 22., letter))
        .child(card(view, t, item, style, now, card_width(view, 0.82)))
        .into_any_element()
}

fn info_row(view: &ChatView, t: &Theme, item: &Item, now: u64) -> AnyElement {
    let width = card_width(view, 0.9);
    row_box(item.id)
        .justify_center()
        .child(
            div()
                .w(width)
                .flex_none()
                .flex()
                .flex_col()
                .rounded(px(8.))
                .border_1()
                .border_color(t.border)
                .child(
                    message_body(item, width)
                        .px(px(9.))
                        .py(px(3.))
                        .font(chat_font(view))
                        .text_size(px(INFO_FONT))
                        .line_height(px(INFO_LINE))
                        .text_color(t.muted),
                )
                .child(foot(view, t, item, now)),
        )
        .into_any_element()
}

fn row(view: &ChatView, t: &Theme, item: &Item, now: u64) -> AnyElement {
    let nixa = t.magenta;
    match item.who.as_str() {
        "user" => {
            let style = if item.quiet {
                Style {
                    bg: gpui::transparent_black(),
                    edge: t.border,
                    text: t.dim,
                    italic: true,
                    small: false,
                }
            } else {
                Style {
                    bg: tint(t.blue, 0.23),
                    edge: tint(t.blue, 0.44),
                    text: t.foreground,
                    italic: false,
                    small: false,
                }
            };
            right_row(view, t, item, style, now)
        }
        "nixa" => {
            let style = if item.routine {
                routine_style(t)
            } else {
                Style {
                    bg: tint(nixa, 0.11),
                    edge: tint(nixa, 0.33),
                    text: t.foreground,
                    italic: false,
                    small: false,
                }
            };
            left_row(
                view,
                t,
                item,
                (nixa, first_letter(view.snap.name())),
                style,
                now,
            )
        }
        "agent" if is_sent(item) => {
            let style = if item.routine {
                routine_style(t)
            } else {
                Style {
                    bg: tint(nixa, 0.15),
                    edge: tint(nixa, 0.4),
                    text: t.foreground,
                    italic: false,
                    small: false,
                }
            };
            right_row(view, t, item, style, now)
        }
        "agent" => {
            let shade = t.yellow;
            let style = if item.routine {
                routine_style(t)
            } else {
                Style {
                    bg: tint(shade, 0.11),
                    edge: tint(shade, 0.33),
                    text: t.foreground,
                    italic: false,
                    small: false,
                }
            };
            let letter = first_letter(item.label.as_deref().unwrap_or("A"));
            left_row(view, t, item, (shade, letter), style, now)
        }
        _ => info_row(view, t, item, now),
    }
}

fn empty(view: &ChatView, t: &Theme) -> AnyElement {
    let name = view.snap.name().to_string();
    div()
        .id("chat-zone")
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px(px(20.))
        .gap(px(8.))
        .child(avatar(t.magenta, 36., first_letter(&name)))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.foreground)
                .child(format!("Talk to {name}")),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(t.dim)
                .text_center()
                .child("Speak, or type below. Try: what is Claude doing? Show the shop space."),
        )
        .into_any_element()
}

pub fn render(view: &mut ChatView, t: &Theme, cx: &mut Context<ChatView>) -> AnyElement {
    follow(view);
    let items: Vec<Item> = visible(view).into_iter().cloned().collect();
    let draft = (!view.snap.draft.is_empty()).then(|| Item {
        id: DRAFT_ID,
        who: "nixa".into(),
        text: view.snap.draft.clone(),
        at: format::now_ms(),
        ..Item::default()
    });
    if items.is_empty() && draft.is_none() {
        return empty(view, t);
    }
    let now = format::now_ms();
    let view: &ChatView = view;
    let rows: Vec<AnyElement> = items
        .iter()
        .chain(draft.iter())
        .map(|item| row(view, t, item, now))
        .collect();
    div()
        .id("chat-view")
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .flex_col()
        .on_scroll_wheel(cx.listener(|view, _, _, cx| {
            view.watch_scroll = true;
            view.follow = false;
            cx.notify();
        }))
        .child(
            div()
                .id("chat-zone")
                .debug_selector(|| "chat-zone".into())
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .track_scroll(&view.scroll)
                .children(rows)
                .child(div().h(px(12.)).flex_none()),
        )
        .into_any_element()
}

pub fn jump(view: &ChatView, t: &Theme, cx: &mut Context<ChatView>) -> Option<AnyElement> {
    if !view.user_moved || (view.snap.items.is_empty() && view.snap.draft.is_empty()) {
        return None;
    }
    let hover = t.hover;
    Some(
        stop_down(
            div()
                .id("chat-jump")
                .debug_selector(|| "chat-jump".into())
                .w_full()
                .h(px(26.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_t_1()
                .border_color(t.border)
                .cursor_pointer()
                .hover(move |s| s.bg(hover)),
        )
        .on_click(cx.listener(|view, _, _, cx| {
            jump_newest(view);
            cx.notify();
        }))
        .child(
            div()
                .text_size(px(11.))
                .text_color(t.dim)
                .child("Jump to newest message \u{2193}"),
        )
        .into_any_element(),
    )
}
