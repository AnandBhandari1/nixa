use super::model::{Action, Side};
use crate::actions::{ClosePane, CloseTab, NewTab, RenameSpace, RenameTab, SplitDown, SplitRight, ZoomPane};
use gpui::{KeyBinding, Window};

pub fn key_action(action: &Action) -> Option<Box<dyn gpui::Action>> {
    match action {
        Action::Split {
            side: Side::Right, ..
        } => Some(Box::new(SplitRight)),
        Action::Split {
            side: Side::Down, ..
        } => Some(Box::new(SplitDown)),
        Action::Zoom { .. } => Some(Box::new(ZoomPane)),
        Action::ClosePane { .. } => Some(Box::new(ClosePane)),
        Action::NewTab { .. } => Some(Box::new(NewTab)),
        Action::RenameTab { .. } => Some(Box::new(RenameTab)),
        Action::RenameSpace { .. } => Some(Box::new(RenameSpace)),
        Action::CloseTab { .. } => Some(Box::new(CloseTab)),
        _ => None,
    }
}

fn key_sign(key: &str) -> String {
    let sign = match key {
        "enter" => "↩",
        "escape" => "⎋",
        "tab" => "⇥",
        "backspace" => "⌫",
        "delete" => "⌦",
        "space" => "␣",
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        other => return other.to_uppercase(),
    };
    sign.to_string()
}

pub fn key_text(control: bool, alt: bool, shift: bool, platform: bool, key: &str) -> String {
    let mut text = String::new();
    for (on, sign) in [(control, "⌃"), (alt, "⌥"), (shift, "⇧"), (platform, "⌘")] {
        if on {
            text.push_str(sign);
        }
    }
    text.push_str(&key_sign(key));
    text
}

pub fn keys_text(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|k| {
            let m = k.modifiers();
            key_text(m.control, m.alt, m.shift, m.platform, k.key())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn shortcut(window: &Window, action: &Action) -> Option<String> {
    let key = key_action(action)?;
    window
        .highest_precedence_binding_for_action(key.as_ref())
        .map(|binding| keys_text(&binding))
}
