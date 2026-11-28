pub mod deep;
pub mod model;
pub mod toasts;
pub mod view;

#[cfg(test)]
mod tests;

pub use model::{Action, Item, Level, Notice, Offer, resume_argv};
pub use toasts::{ToastEvent, ToastsView};
pub use view::{EventsCommand, EventsEvent, EventsView, commands};

use gpui::{App, Entity};
use serde_json::Value;

pub struct Screens {
    pub events: Entity<EventsView>,
    pub toasts: Entity<ToastsView>,
}

pub fn new(cx: &mut App) -> Screens {
    Screens {
        events: view::new(cx),
        toasts: toasts::new(cx),
    }
}

pub fn notify(screens: &Screens, notice: Notice, cx: &mut App) {
    screens.events.update(cx, |view, cx| {
        view.post(&notice, cx);
    });
    screens.toasts.update(cx, |view, cx| {
        view.push(&notice, cx);
    });
}

pub fn resume_notice(offer: Offer) -> Notice {
    let mut title = offer.title.clone();
    if model::skips_checks(&offer.command) && !title.contains(model::SKIP_WORDS) {
        title = format!("{title} ({})", model::SKIP_WORDS);
    }
    let mut body = offer.body.clone();
    if !body.contains(&offer.command) {
        body = format!("{body} Runs: {}", offer.command).trim().to_string();
    }
    Notice {
        title,
        body,
        level: Level::Info,
        kind: Some("resume".into()),
        terminal: offer.terminal.clone(),
        timeout_ms: Some(offer.timeout_ms),
        action: Some(Action::Resume(offer)),
    }
}

pub fn apply(screens: &Screens, value: &Value, cx: &mut App) -> bool {
    match value.get("type").and_then(Value::as_str).unwrap_or_default() {
        "agent_event" => {
            let Some(item) = value.get("event").and_then(model::item_of) else {
                return false;
            };
            if item.kind == "error" {
                let title = model::row_text(&item, &screens.events.read(cx).numbers);
                let mut notice = Notice::new(Level::Error, title);
                notice.terminal = item.terminal.clone();
                notice.action = item.action.clone();
                screens.toasts.update(cx, |view, cx| view.push(&notice, cx));
            }
            screens.events.update(cx, |view, cx| view.add_agent(item, cx));
            true
        }
        "resume_offer" => {
            let Some(offer) = value.get("offer").and_then(model::offer_of) else {
                return false;
            };
            notify(screens, resume_notice(offer), cx);
            true
        }
        "announce" => {
            let list: Vec<Item> = value
                .get("events")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(model::item_of).collect())
                .unwrap_or_default();
            let on = value.get("on").and_then(Value::as_bool);
            screens.events.update(cx, |view, cx| {
                view.load(list, cx);
                if let Some(on) = on {
                    view.show_agent_voice(on, cx);
                }
            });
            true
        }
        "agent_voice" => {
            let Some(on) = value.get("on").and_then(Value::as_bool) else {
                return false;
            };
            screens
                .events
                .update(cx, |view, cx| view.show_agent_voice(on, cx));
            true
        }
        "toast" => {
            let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            let level = Level::parse(&text("level"));
            let mut notice = Notice::new(level, text("title")).body(text("body"));
            notice.terminal = text("terminal");
            if !notice.terminal.is_empty() {
                notice.action = Some(Action::Focus(notice.terminal.clone()));
            }
            notice.timeout_ms = value.get("timeout_ms").and_then(Value::as_u64);
            if notice.title.trim().is_empty() && notice.body.trim().is_empty() {
                return false;
            }
            notify(screens, notice, cx);
            true
        }
        "notice" if value.get("loud").and_then(Value::as_bool) == Some(true) => {
            let text = value.get("text").and_then(Value::as_str).unwrap_or_default();
            if text.trim().is_empty() {
                return false;
            }
            notify(screens, Notice::new(Level::Info, text), cx);
            true
        }
        "deep" => {
            let Some(notice) = value.get("ask").and_then(deep::deep_notice) else {
                return false;
            };
            screens.events.update(cx, |view, cx| {
                view.post(&notice, cx);
            });
            true
        }
        _ => false,
    }
}

pub fn unseen(screens: &Screens, cx: &App) -> usize {
    screens.events.read(cx).unseen()
}

pub fn is_open(screens: &Screens, cx: &App) -> bool {
    screens.events.read(cx).open
}
