use super::info::PhoneState;
use gpui::prelude::*;
use gpui::{Div, Hsla, MouseButton, SharedString, Stateful, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon};

pub const PHONE_ID: &str = "chat-phone";
pub const VOICE_ID: &str = "chat-voice-place";
pub const PART: &str = "Phone";

#[derive(Clone, Debug, PartialEq)]
pub struct ChipSpec {
    pub id: SharedString,
    pub which: Icon,
    pub label: String,
    pub tip: String,
    pub color: Hsla,
}

pub fn phone_chip(state: &PhoneState, name: &str, t: &Theme) -> ChipSpec {
    let live = state.live();
    let on = state.door_on();
    let label = if live > 0 {
        format!("Phone · {live}")
    } else {
        "Phone".into()
    };
    let color = if live > 0 {
        t.green
    } else if on {
        t.blue
    } else {
        t.dim
    };
    let tip = if live > 0 {
        let who = if live == 1 { "phone is" } else { "phones are" };
        format!("{live} {who} connected in the {name} app. Click to see the phone door.")
    } else if on {
        format!(
            "The phone door is open on your Wi-Fi. Click to see the pairing code for the {name} app."
        )
    } else {
        format!("Use {name} on your Android phone. Click to open the phone door.")
    };
    ChipSpec {
        id: PHONE_ID.into(),
        which: Icon::Smartphone,
        label,
        tip,
        color,
    }
}

pub fn voice_spec(state: &PhoneState, name: &str, t: &Theme, id: &'static str) -> ChipSpec {
    let sound = state.sound();
    let (which, label, color) = if state.mobile {
        (
            Icon::Smartphone,
            "Voice: Phone",
            if sound { t.green } else { t.yellow },
        )
    } else {
        (Icon::Volume, "Voice: Mac", t.dim)
    };
    let tip = match (state.mobile, sound) {
        (true, true) => format!(
            "{name} speaks on the phone. The Mac is silent. Click to make {name} speak on the Mac."
        ),
        (true, false) => format!(
            "Voice is set to the phone, but no phone has sound on. Open the {name} app on the phone. Missed lines play there later. Click to make {name} speak on the Mac."
        ),
        (false, _) => format!(
            "{name} speaks on the Mac. Click to make {name} speak on the phone instead."
        ),
    };
    ChipSpec {
        id: id.into(),
        which,
        label: label.into(),
        tip,
        color,
    }
}

pub fn chip(t: &Theme, spec: &ChipSpec) -> Stateful<Div> {
    let hover = t.hover;
    let id = spec.id.clone();
    div()
        .id(spec.id.clone())
        .debug_selector(move || id.to_string())
        .h(px(24.))
        .px(px(7.))
        .gap(px(4.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .text_size(px(11.))
        .text_color(spec.color)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(tooltip(spec.tip.clone()))
        .child(icon(spec.which).size(px(12.)).text_color(spec.color))
        .child(div().whitespace_nowrap().child(spec.label.clone()))
}
