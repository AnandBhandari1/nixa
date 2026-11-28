pub mod doc;
pub mod text_area;

use doc::{DocEditor, Document};
use gpui::{App, Entity};
use serde_json::Value;

pub fn new(addr: String, cx: &mut App) -> Entity<DocEditor> {
    doc::new(Document::Memory, addr, false, cx)
}

pub fn feed(view: &Entity<DocEditor>, msg: &Value, cx: &mut App) -> bool {
    doc::feed(view, msg, cx)
}

#[cfg(test)]
mod tests;
