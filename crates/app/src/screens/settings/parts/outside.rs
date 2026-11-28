use crate::links::brain as link;
use gpui::prelude::*;
use gpui::{Div, Entity, SharedString, div, px};
use nixa_protocol::paths;
use nixa_ui::Theme;
use nixa_ui::text_input::TextInput;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub const CALL_WAIT: Duration = Duration::from_secs(20);
pub const INDEX_WAIT: Duration = Duration::from_secs(120);

pub type Call = Arc<dyn Fn(&str, &str, Option<Value>) -> Result<Value, String> + Send + Sync>;

pub fn brain() -> Call {
    Arc::new(|method, path, body| {
        let wait = if path.ends_with("/index") { INDEX_WAIT } else { CALL_WAIT };
        link::http(&paths::brain_addr(), method, path, body.as_ref(), wait).map_err(|e| format!("{e:#}"))
    })
}

pub fn error_of(reply: &Value) -> Option<String> {
    (reply.get("ok").and_then(Value::as_bool) == Some(false)).then(|| reply.get("error").and_then(Value::as_str).unwrap_or("Nixa did not take it.").to_string())
}

pub fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn card(t: &Theme) -> Div {
    div().w_full().flex().flex_col().gap(px(8.)).p(px(12.)).rounded(px(8.)).border_1().border_color(t.border)
}

pub fn title(t: &Theme, text: &str) -> Div {
    div().text_size(px(13.)).text_color(t.bright).child(text.to_string())
}

pub fn note(color: gpui::Hsla, size: f32, text: impl Into<SharedString>) -> Div {
    div().text_size(px(size)).text_color(color).child(text.into())
}

pub fn field(t: &Theme, label: &str, input: &Entity<TextInput>, tag: &'static str, about: &str) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(note(t.muted, 11., label.to_string()))
        .child(div().w_full().flex().flex_col().debug_selector(move || tag.to_string()).child(input.clone()))
        .when(!about.is_empty(), |el| el.child(note(t.dim, 10.5, about.to_string())))
}

pub fn row() -> Div {
    div().flex().flex_row().flex_wrap().gap(px(8.)).items_center()
}
