use crate::screens::chat::model::Snapshot;
use crate::screens::chat::wire::Command;
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, Corner, MouseButton, MouseDownEvent, SharedString, div, px};
use nixa_ui::menu::{self as ui_menu, MenuItem};
use nixa_ui::popover::{float_at, tooltip};
use nixa_ui::{Icon, Theme, icon};
use std::rc::Rc;

pub const TALK: &str = "tab-talk";
pub const TALK_MODE: &str = "tab-talk-mode";
pub const TALK_MENU: &str = "tab-talk-menu";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Hotkey,
    Always,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Talk {
    pub mode: Mode,
    pub recording: bool,
    pub starting: bool,
    pub label: String,
    pub tip: String,
}

pub fn mode_of(snap: &Snapshot) -> Option<Mode> {
    let listen = snap.listen.as_deref()?;
    Some(match (matches!(listen, "always" | "wake"), snap.mic_off) {
        (false, _) => Mode::Hotkey,
        (true, false) => Mode::Always,
        (true, true) => Mode::Off,
    })
}

pub fn talk(snap: &Snapshot) -> Option<Talk> {
    let mode = mode_of(snap)?;
    let name = snap.name();
    let recording = mode == Mode::Hotkey && snap.key_listen == "on";
    let starting = mode == Mode::Hotkey && snap.key_listen == "starting";
    let (label, tip) = match mode {
        Mode::Hotkey if recording => (
            "Listening… ⌘ to send".to_string(),
            format!("{name} is listening. Click or tap Right Command to send what you said."),
        ),
        Mode::Hotkey if starting => ("Starting…".to_string(), format!("{name} is starting the mic.")),
        Mode::Hotkey => (
            "⌘ Talk".to_string(),
            format!("Click or tap Right Command (or F9) and talk to {name}. Do it again to send. The key works while Nixa is in the background."),
        ),
        Mode::Always => (
            "Always on".to_string(),
            format!("Mic always on: {name} hears the room and picks what is meant for it. Click to choose how the mic listens."),
        ),
        Mode::Off => ("Mic off".to_string(), format!("The mic is off. {name} does not hear you. Click to choose how the mic listens.")),
    };
    Some(Talk { mode, recording, starting, label, tip })
}

pub fn picks(i: usize) -> Vec<Command> {
    match i {
        0 => vec![Command::Listen("hotkey")],
        1 => vec![Command::Listen("always"), Command::Mic(true)],
        _ => vec![Command::Listen("always"), Command::Mic(false)],
    }
}

fn open(ws: &mut Workspace, at: gpui::Point<gpui::Pixels>, cx: &mut Context<Workspace>) {
    ws.tab_bar.update(cx, |bar, _| {
        bar.talk_menu = if bar.talk_menu.is_some() { None } else { Some(at) };
    });
    cx.notify();
}

pub fn button(ws: &Workspace, t: &Theme, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let spec = talk(&ws.hub.chat.read(cx).snap)?;
    let mode = spec.mode;
    let hover = t.hover;
    let (fg, bg) = match mode {
        _ if spec.recording => (t.background, t.red),
        Mode::Hotkey => (t.foreground, t.chrome),
        Mode::Always => (t.cyan, t.chrome),
        Mode::Off => (t.muted, t.chrome),
    };
    let which = if mode == Mode::Off { Icon::MicOff } else { Icon::Mic };
    let main = div()
        .id(TALK)
        .debug_selector(|| TALK.into())
        .h_full()
        .px(px(8.0))
        .gap(px(5.0))
        .flex()
        .items_center()
        .text_size(px(11.5))
        .text_color(fg)
        .cursor_pointer()
        .when(!spec.recording, move |el| el.hover(move |s| s.bg(hover)))
        .tooltip(tooltip(spec.tip.clone()))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |ws, e: &ClickEvent, _, cx| {
            if mode == Mode::Hotkey {
                crate::actions::listen(ws, cx);
            } else {
                open(ws, e.position(), cx);
            }
        }))
        .child(icon(which).size(px(13.0)).text_color(fg))
        .child(SharedString::from(spec.label.clone()));
    let arrow = div()
        .id(TALK_MODE)
        .debug_selector(|| TALK_MODE.into())
        .h_full()
        .px(px(5.0))
        .flex()
        .items_center()
        .border_l_1()
        .border_color(t.border)
        .text_color(fg)
        .cursor_pointer()
        .when(!spec.recording, move |el| el.hover(move |s| s.bg(hover)))
        .tooltip(tooltip("Choose how the mic listens: Right Command hotkey, always on, or off.".to_string()))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|ws, e: &ClickEvent, _, cx| open(ws, e.position(), cx)))
        .child(icon(Icon::ChevronDown).size(px(11.0)));
    Some(
        div()
            .id("tab-talk-split")
            .h(px(26.0))
            .flex_none()
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(if spec.recording { t.red } else { t.border })
            .bg(bg)
            .overflow_hidden()
            .child(main)
            .child(arrow)
            .into_any_element(),
    )
}

pub fn menu_items(mode: Option<Mode>) -> Vec<MenuItem> {
    vec![
        MenuItem::new("Right ⌘ hotkey").icon(Icon::Mic).checked(mode == Some(Mode::Hotkey)),
        MenuItem::new("Always on").icon(Icon::Mic).checked(mode == Some(Mode::Always)),
        MenuItem::new("Off").icon(Icon::MicOff).checked(mode == Some(Mode::Off)),
    ]
}

pub fn menu(ws: &Workspace, t: &Theme, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let Some(at) = ws.tab_bar.read(cx).talk_menu else { return Vec::new() };
    let mode = mode_of(&ws.hub.chat.read(cx).snap);
    let this = cx.entity();
    let pick = Rc::new(move |i: usize, _: &mut gpui::Window, cx: &mut gpui::App| {
        this.update(cx, |ws, cx| {
            ws.tab_bar.update(cx, |bar, _| bar.talk_menu = None);
            for cmd in picks(i) {
                crate::hub::send_chat(ws, cmd, cx);
            }
            cx.notify();
        });
    });
    vec![float_at(
        at,
        Corner::TopLeft,
        div()
            .id("tab-talk-holder")
            .on_mouse_down_out(cx.listener(|ws, _: &MouseDownEvent, _, cx| {
                ws.tab_bar.update(cx, |bar, _| bar.talk_menu = None);
                cx.notify();
            }))
            .child(ui_menu::menu(t, TALK_MENU, Some("How the mic listens".into()), &menu_items(mode), None, pick)),
    )
    .into_any_element()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_follows_the_mode() {
        let mut snap = Snapshot { listen: Some("hotkey".into()), mic_off: true, ..Snapshot::default() };
        let t = talk(&snap).unwrap();
        assert_eq!((t.mode, t.label.as_str(), t.recording), (Mode::Hotkey, "⌘ Talk", false));
        snap.key_listen = "on".into();
        let t = talk(&snap).unwrap();
        assert_eq!((t.label.as_str(), t.recording), ("Listening… ⌘ to send", true));
        snap.key_listen.clear();
        snap.listen = Some("always".into());
        assert_eq!(talk(&snap).unwrap().label, "Mic off");
        snap.mic_off = false;
        assert_eq!(talk(&snap).unwrap().mode, Mode::Always);
        snap.listen = Some("wake".into());
        assert_eq!(talk(&snap).unwrap().mode, Mode::Always);
        assert!(talk(&Snapshot::default()).is_none());
    }

    #[test]
    fn the_menu_sends_the_mode() {
        assert_eq!(picks(0), vec![Command::Listen("hotkey")]);
        assert_eq!(picks(1), vec![Command::Listen("always"), Command::Mic(true)]);
        assert_eq!(picks(2), vec![Command::Listen("always"), Command::Mic(false)]);
        let items = menu_items(Some(Mode::Hotkey));
        assert!(items[0].checked && !items[1].checked && !items[2].checked);
    }
}
