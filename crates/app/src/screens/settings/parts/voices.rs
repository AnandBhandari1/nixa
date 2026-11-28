use crate::screens::voices::{self, VoicesView};
use gpui::{App, Entity};
use serde_json::Value;

pub const PART: &str = "Voices";

pub fn show(cx: &mut App) -> Entity<VoicesView> {
    voices::show(cx)
}

pub fn apply(msg: &Value, cx: &mut App) -> bool {
    match voices::shared(cx) {
        Some(view) => voices::apply(&view, msg, cx),
        None => false,
    }
}

