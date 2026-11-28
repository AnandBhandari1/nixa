use gpui::prelude::*;
use gpui::{AnyElement, App, AppContext, Div, ElementId, Entity, Global, Hsla, MouseButton, SharedString, Stateful, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::theme::tint;
use nixa_ui::{Icon, Theme, icon};
use serde_json::{Value, json};

pub const CARD_ID: &str = "talk-confirm";
pub const YES_ID: &str = "talk-confirm-yes";
pub const NO_ID: &str = "talk-confirm-no";
pub const TEXT_CHARS: usize = 300;
pub const WHY_CHARS: usize = 160;
pub const KIND: &str = "talk";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub id: u64,
    pub message: String,
    pub why: String,
}

#[derive(Default)]
pub struct TalkCard {
    pub card: Option<Card>,
    pub sent: Vec<Value>,
}

struct Hub(Entity<TalkCard>);

impl Global for Hub {}

pub fn hub(cx: &mut App) -> Entity<TalkCard> {
    if let Some(h) = cx.try_global::<Hub>() {
        return h.0.clone();
    }
    let h = cx.new(|_| TalkCard::default());
    cx.set_global(Hub(h.clone()));
    h
}

pub fn current(cx: &App) -> Option<Card> {
    cx.try_global::<Hub>().and_then(|h| h.0.read(cx).card.clone())
}

fn text_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string()
}

pub fn card_of(msg: &Value) -> Option<Option<Card>> {
    if msg.get("type").and_then(Value::as_str) != Some(KIND) {
        return None;
    }
    let confirm = msg.get("confirm")?;
    let Some(id) = confirm.get("id").and_then(Value::as_u64) else {
        return Some(None);
    };
    let message = text_of(confirm, "message");
    if message.is_empty() {
        return Some(None);
    }
    Some(Some(Card { id, message, why: text_of(confirm, "why") }))
}

pub fn feed(msg: &Value, cx: &mut App) -> bool {
    let Some(card) = card_of(msg) else {
        return false;
    };
    let h = hub(cx);
    h.update(cx, |h, cx| {
        if h.card != card {
            h.card = card;
            cx.notify();
        }
    });
    true
}

pub fn answer(id: u64, approve: bool) -> Value {
    json!({"type": "confirm", "approve": approve, "id": id})
}

pub fn tell_brain(value: &Value, cx: &mut App) -> bool {
    let Some((kind, fields)) = crate::hub::split_kind(value) else {
        return false;
    };
    let link = cx
        .windows()
        .iter()
        .find_map(|w| w.downcast::<crate::workspace::Workspace>())
        .and_then(|w| w.read(cx).ok().and_then(|ws| ws.hub.link.clone()));
    link.is_some_and(|link| link.send(&kind, fields).is_ok())
}

pub fn pick(id: u64, approve: bool, cx: &mut App) {
    let value = answer(id, approve);
    let h = hub(cx);
    h.update(cx, |h, cx| {
        if h.card.as_ref().is_some_and(|c| c.id == id) {
            h.card = None;
        }
        h.sent.push(value.clone());
        cx.notify();
    });
    tell_brain(&value, cx);
}

pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

fn button(t: &Theme, id: &'static str, which: Icon, label: &'static str, tip: &'static str, color: Hsla) -> Stateful<Div> {
    let hover = t.hover;
    div()
        .id(ElementId::from(SharedString::from(id)))
        .debug_selector(move || id.to_string())
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
        .text_color(color)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(tip))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(icon(which).size(px(13.)).text_color(color))
        .child(label)
}

pub fn view(card: &Card, name: &str, t: &Theme) -> Stateful<Div> {
    let why = (!card.why.is_empty()).then(|| clip(&card.why, WHY_CHARS));
    let yes = card.id;
    let no = card.id;
    div()
        .id(CARD_ID)
        .debug_selector(|| CARD_ID.to_string())
        .w_full()
        .relative()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.yellow)
        .child(div().absolute().top(px(1.)).right(px(6.)).text_size(px(9.)).line_height(px(10.)).text_color(tint(t.dim, 0.6)).child("Confirm"))
        .child(div().text_size(px(11.)).text_color(t.blue).child(format!("{} ASKS", name.to_uppercase())))
        .child(div().text_size(px(13.)).text_color(t.yellow).child(clip(&card.message, TEXT_CHARS)))
        .children(why.map(|w| div().text_size(px(11.5)).text_color(t.magenta).child(w)))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .items_center()
                .child(div().flex_1().text_size(px(11.5)).text_color(t.magenta).child("Say yes or no, or click."))
                .child(button(t, YES_ID, Icon::Check, "Yes", "Allow this action", t.green).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    pick(yes, true, cx);
                }))
                .child(button(t, NO_ID, Icon::X, "No", "Do not do it", t.red).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    pick(no, false, cx);
                })),
        )
}

pub fn render(t: &Theme, name: &str, cx: &App) -> Option<AnyElement> {
    let card = current(cx)?;
    Some(view(&card, name, t).into_any_element())
}

#[cfg(test)]
mod tests;
