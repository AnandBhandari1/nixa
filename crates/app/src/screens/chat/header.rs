use super::format::{self, busy};
use super::parts::{box_chip, head_chip, icon_btn, mark, tone};
use super::wire::Command;
use super::work::keyword;
use super::{ChatEvent, ChatView, SETTINGS_PART, send};
use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, Corner, FontWeight, MouseButton, MouseDownEvent, Pixels, Point, SharedString, div, px};
use nixa_ui::menu::{MenuItem, menu_row};
use nixa_ui::popover::{float_at, popover};
use nixa_ui::{Icon, Theme};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatMenu {
    VoicePlace,
}

pub const PLACE_MENU: &str = "chat-voice-place-menu";

fn open_menu(view: &mut ChatView, which: ChatMenu, at: Point<Pixels>, cx: &mut Context<ChatView>) {
    view.menu = match view.menu {
        Some((now, _)) if now == which => None,
        _ => Some((which, at)),
    };
    cx.notify();
}

fn keyword_chip(view: &ChatView, t: &Theme, cx: &mut Context<ChatView>) -> Option<AnyElement> {
    let k = keyword(&view.snap.work, view.snap.name())?;
    let what = k.what;
    Some(
        head_chip(t, k.id, Icon::X, k.label, k.tip, t.yellow, Some(160.))
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.stop_propagation();
                send(Command::KeywordClear(what), cx);
            }))
            .into_any_element(),
    )
}

fn phone_chips(t: &Theme, name: &str, cx: &mut Context<ChatView>) -> Vec<AnyElement> {
    let state = crate::screens::phone_chip::current(cx);
    let spec = crate::screens::phone_chip::phone_chip(&state, name, t);
    vec![crate::screens::phone_chip::chip(t, &spec)
        .on_click(cx.listener(|_, _, _, cx| {
            cx.stop_propagation();
            cx.emit(ChatEvent::OpenSettings(crate::screens::phone_chip::chips::PART));
        }))
        .into_any_element()]
}

fn chips(view: &ChatView, t: &Theme, cx: &mut Context<ChatView>) -> AnyElement {
    let snap = &view.snap;
    let name = snap.name().to_string();
    let off = snap.off();
    let model = model_chip(view, t, false, cx);
    div()
        .w_full()
        .flex()
        .flex_wrap()
        .gap(px(6.))
        .items_center()
        .child(model)
        .when(!off, |row| row.children(keyword_chip(view, t, cx)))
        .children(phone_chips(t, &name, cx))
        .into_any_element()
}

fn model_chip(view: &ChatView, t: &Theme, compact: bool, cx: &mut Context<ChatView>) -> AnyElement {
    let snap = &view.snap;
    let name = snap.name();
    let tip = if compact && snap.brain.is_none() {
        format!("Pick a brain. Click to open {name} settings at Chat brain.")
    } else if compact {
        format!(
            "Brain: {}. Click to open {name} settings at Chat brain.",
            format::model_name(snap)
        )
    } else {
        format!(
            "Brain: {}. Click to open {name} settings at Chat brain: pick the brain and model, or add your own.",
            format::model_name(snap)
        )
    };
    head_chip(
        t,
        "chat-model",
        Icon::Layers,
        format::short_model(snap),
        tip,
        t.magenta,
        Some(110.),
    )
    .debug_selector(|| "chat-model".into())
    .on_click(cx.listener(|_, _, _, cx| {
        cx.stop_propagation();
        cx.emit(ChatEvent::OpenSettings(SETTINGS_PART));
    }))
    .into_any_element()
}

fn talk_buttons(view: &ChatView, t: &Theme, cx: &mut Context<ChatView>) -> AnyElement {
    let snap = &view.snap;
    let name = snap.name().to_string();
    div()
        .flex()
        .flex_none()
        .gap(px(2.))
        .items_center()
        .child({
            let voice_off = snap.voice_off;
            let tip = if voice_off {
                format!("Voice is off. Nothing speaks: not {name}, not agents, on the Mac or the phone. Click to turn voice on.")
            } else {
                format!("Voice is on. Click to turn all voice off: {name} and agents stop speaking on the Mac and the phone.")
            };
            icon_btn(t, "chat-voice", if voice_off { Icon::VolumeOff } else { Icon::Volume }, tip, if voice_off { t.red } else { t.muted }, false)
                .debug_selector(|| "chat-voice".into())
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    send(Command::Voice(voice_off), cx);
                }))
        })
        .child({
            let phone = crate::screens::phone_chip::current(cx);
            let mut spec = crate::screens::phone_chip::chips::voice_spec(&phone, &name, t, crate::screens::phone_chip::chips::VOICE_ID);
            spec.label = format!("{} ▾", if phone.mobile { "Phone" } else { "Mac" });
            spec.tip = match (phone.mobile, phone.sound()) {
                (false, _) => format!("{name} speaks on the Mac. Click to choose the Mac or the phone."),
                (true, true) => format!("{name} speaks on the phone. The Mac is silent. Click to choose the Mac or the phone."),
                (true, false) => format!("Voice is set to the phone, but no phone has sound on. Open the {name} app on the phone. Missed lines play there later. Click to choose the Mac or the phone."),
            };
            if snap.voice_off {
                spec.color = t.dim;
                spec.tip = format!("Voice is off. When it is on, {name} speaks on the {}. Click to choose the Mac or the phone.", if phone.mobile { "phone" } else { "Mac" });
            }
            crate::screens::phone_chip::chip(t, &spec).on_click(cx.listener(move |v, e: &ClickEvent, _, cx| {
                cx.stop_propagation();
                open_menu(v, ChatMenu::VoicePlace, e.position(), cx);
            }))
        })
        .into_any_element()
}

type Pick = Rc<dyn Fn(usize, &mut gpui::App)>;

fn menu_panel(t: &Theme, id: &'static str, title: &'static str, items: &[MenuItem], pick: Pick) -> gpui::Stateful<gpui::Div> {
    popover(t)
        .id(id)
        .min_w(px(180.))
        .child(div().px(px(8.)).pt(px(2.)).pb(px(4.)).text_size(px(11.)).text_color(t.dim).truncate().child(title))
        .children(items.iter().enumerate().map(|(i, item)| {
            let pick = pick.clone();
            let selector = format!("{id}-{i}");
            menu_row(t, SharedString::from(format!("{id}-{i}")), item, false)
                .debug_selector(move || selector)
                .on_click(move |_, _, cx| pick(i, cx))
        }))
}

pub fn menus(view: &ChatView, t: &Theme, cx: &mut Context<ChatView>) -> Vec<AnyElement> {
    let Some((which, at)) = view.menu else { return Vec::new() };
    let this = cx.entity();
    let (id, title, items, picks): (&'static str, &'static str, Vec<MenuItem>, Vec<Command>) = match which {
        ChatMenu::VoicePlace => {
            let mobile = crate::screens::phone_chip::current(cx).mobile;
            (
                PLACE_MENU,
                "Where the voice plays",
                vec![
                    MenuItem::new("Mac").icon(Icon::Volume).checked(!mobile),
                    MenuItem::new("Phone").icon(Icon::Smartphone).checked(mobile),
                ],
                vec![Command::PhoneVoice(false), Command::PhoneVoice(true)],
            )
        }
    };
    let pick = Rc::new(move |i: usize, cx: &mut gpui::App| {
        let cmd = picks.get(i).cloned();
        this.update(cx, |v, cx| {
            v.menu = None;
            if let Some(cmd) = cmd {
                send(cmd, cx);
            }
            cx.notify();
        });
    });
    vec![float_at(
        at,
        Corner::TopLeft,
        div()
            .id("chat-menu-holder")
            .on_mouse_down_out(cx.listener(|v, _: &MouseDownEvent, _, cx| {
                v.menu = None;
                cx.notify();
            }))
            .child(menu_panel(t, id, title, &items, pick)),
    )
    .into_any_element()]
}

fn sending(view: &ChatView, cx: &Context<ChatView>) -> bool {
    view.question.as_ref().is_some_and(|q| {
        q.read(cx)
            .snap
            .card
            .as_ref()
            .is_some_and(|c| c.state == crate::screens::question::card::CardState::Sending)
    })
}

pub fn render(
    view: &ChatView,
    t: &Theme,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let snap = &view.snap;
    let up = snap.link == super::model::Link::Up;
    let name = snap.name().to_string();
    let (word, color, which) = if up {
        let mobile = crate::screens::phone_chip::current(cx).mobile;
        let (word, shade, which) = format::status(snap, mobile, sending(view, cx));
        (crate::screens::progress::status_word(word, cx), tone(t, shade), which)
    } else if snap.link == super::model::Link::Off {
        ("off".to_string(), t.dim, format::Mark::Hollow)
    } else {
        ("starting".to_string(), t.dim, format::Mark::Hollow)
    };
    let talk = up && !snap.off();
    let tools = talk;
    let stop = up && busy(snap);
    let row = div()
        .h(px(26.))
        .flex()
        .gap(px(8.))
        .items_center()
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.bright)
                .child(name.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .gap(px(6.))
                .items_center()
                .child(mark(which, color))
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(12.))
                        .text_color(color)
                        .child(word),
                )
                .when(tools, |row| row.children(crate::screens::quiet_switch::render(t, &name, cx))),
        )
        .when(tools, |el| {
            el.child(
                icon_btn(
                    t,
                    "chat-clean",
                    Icon::Eraser,
                    "Clean the chat and start a fresh brain chat",
                    if view.clean_ask { t.foreground } else { t.dim },
                    view.clean_ask,
                )
                .debug_selector(|| "chat-clean".into())
                .on_click(cx.listener(|view, _, _, cx| {
                    cx.stop_propagation();
                    view.clean_ask = !view.clean_ask;
                    cx.notify();
                })),
            )
        })
        .when(talk, |el| el.child(talk_buttons(view, t, cx)))
        .when(stop, |el| {
            el.child(
                icon_btn(
                    t,
                    "chat-stop",
                    Icon::X,
                    format!("Stop {name} speaking or thinking"),
                    t.red,
                    false,
                )
                .debug_selector(|| "chat-stop".into())
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.stop_propagation();
                    send(Command::Stop, cx);
                })),
            )
        })
        .child(
            box_chip(
                t,
                "chat-hide",
                Icon::X,
                "",
                "Hide chat only. Mic and voice stay unchanged. Reopen with Show chat in the top bar.",
                t.muted,
                true,
            )
            .debug_selector(|| "chat-hide".into())
            .size(px(26.))
            .px(px(0.))
            .justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(ChatEvent::Hide);
            })),
        );
    let chips = up.then(|| chips(view, t, cx));
    div()
        .flex_none()
        .flex()
        .flex_col()
        .px(px(12.))
        .pt(px(8.))
        .pb(px(7.))
        .gap(px(6.))
        .border_b_1()
        .border_color(t.border)
        .child(row)
        .children(chips)
        .into_any_element()
}
