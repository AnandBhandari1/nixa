// Agent navigation marks for tabs and sidebar rows.
// Colour marks remain images; monochrome marks follow the surrounding theme.
use gpui::prelude::*;
use gpui::{AnyElement, Hsla, SharedString, img, px, svg};

pub const NAMES: &[&str] = &[
    "claude",
    "codex",
    "opencode",
    "crush",
    "jcode",
    "hermes",
    "dim",
    "cursor",
    "antigravity",
    "pi",
    "omo",
];

pub fn bytes(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "claude" => include_bytes!("../assets/agents/claude.svg"),
        "codex" => include_bytes!("../assets/agents/codex.svg"),
        "opencode" => include_bytes!("../assets/agents/opencode.svg"),
        "crush" => include_bytes!("../assets/agents/crush.svg"),
        "jcode" => include_bytes!("../assets/agents/jcode.svg"),
        "hermes" => include_bytes!("../assets/agents/hermes.svg"),
        "dim" => include_bytes!("../assets/agents/dim.svg"),
        "cursor" => include_bytes!("../assets/agents/cursor.svg"),
        "antigravity" => include_bytes!("../assets/agents/antigravity.svg"),
        "pi" => include_bytes!("../assets/agents/pi.svg"),
        "omo" => include_bytes!("../assets/agents/omo.svg"),
        _ => return None,
    })
}

pub fn glyph(name: &str, color: Hsla, size: f32) -> Option<AnyElement> {
    if !NAMES.contains(&name) {
        return None;
    }
    let path = SharedString::from(format!("icons/{name}.svg"));
    Some(
        if matches!(name, "claude" | "codex" | "crush" | "antigravity" | "omo") {
            img(path).size(px(size)).flex_none().into_any_element()
        } else {
            svg()
                .path(path)
                .size(px(size))
                .flex_none()
                .text_color(color)
                .into_any_element()
        },
    )
}
