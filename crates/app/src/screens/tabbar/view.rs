use super::context::BrainContext;
use super::quick::{self, Button, QUICK_AGENTS};
use super::{TabBar, close_menu, context, models, open_menu, toggle_agent_voice, toggle_chat, toggle_nixa};
use crate::programs;
use crate::screens::new_menu::NewMenu;
use crate::screens::new_menu::glyph;
use crate::workspace::{PanelKind, Workspace};
use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, AnyElement, ClickEvent, Context, Corner, Entity, MouseButton, Pixels,
    Point, SharedString, Window, WindowControlArea, anchored, deferred, div, point,
    pulsating_between, px,
};
use nixa_ui::icon_button::{IconSize, tinted_icon_button};
use nixa_ui::metrics::{RAIL_WIDTH, TRAFFIC_LIGHTS_SPACE};
use nixa_ui::popover::tooltip;
use nixa_ui::tab::tab_bar;
use nixa_ui::{Icon, Theme, icon, theme};
use std::time::Duration;

pub(super) fn titlebar_double_click(count: usize, fullscreen: bool) -> bool {
    count == 2 && !fullscreen
}

pub fn title_double_click(window: &Window) -> bool {
    let run = titlebar_double_click(2, window.is_fullscreen());
    if run {
        window.titlebar_double_click();
    }
    run
}

pub fn lights_pad(mac: bool, fullscreen: bool, left_w: f32) -> f32 {
    if mac && !fullscreen {
        (TRAFFIC_LIGHTS_SPACE - left_w).max(10.0)
    } else {
        10.0
    }
}

pub fn window_buttons(mac: bool) -> bool {
    !mac
}

fn pulse(id: &'static str, period_ms: u64, low: f32, el: gpui::Div) -> AnyElement {
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(period_ms))
            .repeat()
            .with_easing(pulsating_between(low, 1.0)),
        |el, delta| el.opacity(delta),
    )
    .into_any_element()
}

pub struct Shown {
    pub name: String,
    pub nixa_on: bool,
    pub agent_voice: bool,
    pub waiting: usize,
    pub context: Option<BrainContext>,
    pub offset_min: i32,
    pub menu: Option<(Entity<NewMenu>, Point<Pixels>)>,
    pub quick: Vec<Button>,
}

pub fn shown(bar: &TabBar) -> Shown {
    Shown {
        name: bar.name.clone(),
        nixa_on: bar.nixa_on,
        agent_voice: bar.agent_voice,
        waiting: bar.waiting,
        context: bar.context.clone(),
        offset_min: bar.offset_min,
        menu: bar.menu.as_ref().map(|m| (m.view.clone(), m.at)),
        quick: bar.quick.clone(),
    }
}

pub fn nixa_tip(bar: &Shown) -> String {
    let name = &bar.name;
    let head = if bar.nixa_on {
        format!(
            "{name}: on. {name} listens and talks with you. Click to turn {name} off. While off, agents are silent too."
        )
    } else {
        format!(
            "{name}: off. No mic and no voice: {name} and agents are silent. Click to turn {name} on."
        )
    };
    let more = context::tip(bar.context.as_ref(), name, bar.offset_min);
    if more.is_empty() {
        head
    } else {
        format!("{head}\n{more}")
    }
}

pub fn chat_tip(name: &str, chat_on: bool, waits: bool) -> String {
    if chat_on {
        format!(
            "Hide {name} chat. Mic and voice stay unchanged. Reopen with Show chat in this top bar."
        )
    } else {
        format!(
            "{}Show {name} chat. Mic and voice stay unchanged.",
            if waits {
                "Something waits for your answer. "
            } else {
                ""
            }
        )
    }
}

pub fn voice_tip(name: &str, on: bool) -> String {
    if on {
        format!(
            "Agent voice: on. Agent replies and alerts are read aloud. Click to turn off. {name} is not changed."
        )
    } else {
        format!(
            "Agent voice: off. Agent replies and alerts are not read aloud. Click to turn on. {name} is not changed."
        )
    }
}

fn quick_buttons(t: &Theme, list: &[Button], cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let hover = t.hover;
    list.iter()
        .filter_map(|b| QUICK_AGENTS.get(b.index).map(|agent| (b.index, agent, b.off)))
        .map(|(index, agent, off)| {
            let id = format!("quick-agent-{}", agent.id);
            let selector = id.clone();
            let launch = div()
                .id(SharedString::from(id))
                .debug_selector(move || selector.clone())
                .size(px(30.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .tooltip(tooltip(quick::tip(agent, off)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |ws, _: &ClickEvent, _, cx| models::launch_saved(ws, index, cx)))
                .child(glyph(t, Some(&programs::icon_path(agent.icon)), 18.0));
            launch.into_any_element()
        })
        .collect()
}

fn glyph_icon(t: &Theme, which: Icon, size: f32) -> AnyElement {
    icon(which)
        .size(px(size))
        .text_color(t.dim)
        .into_any_element()
}

fn plain_button(
    t: &Theme,
    id: &'static str,
    label: AnyElement,
    tip: String,
) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    let bright = t.foreground;
    div()
        .id(id)
        .debug_selector(move || id.into())
        .size(px(26.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .text_color(t.dim)
        .cursor_pointer()
        .tooltip(tooltip(tip))
        .hover(move |s| s.bg(hover).text_color(bright))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label)
}

fn nixa_pill(t: &Theme, bar: &Shown, cx: &mut Context<Workspace>) -> AnyElement {
    let on = bar.nixa_on;
    let color = if on { t.cyan } else { t.muted };
    let hover = t.hover;
    let words = context::words(bar.context.as_ref());
    let words_color = context::color(bar.context.as_ref(), t);
    div()
        .id("tab-nixa")
        .debug_selector(|| "tab-nixa".into())
        .h(px(26.0))
        .px(px(7.0))
        .gap(px(5.0))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(6.0))
        .text_size(px(11.5))
        .text_color(color)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(nixa_tip(bar)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|ws, _: &ClickEvent, _, cx| toggle_nixa(ws, cx)))
        .when(on, |el| {
            el.child(pulse(
                "tab-nixa-dot",
                1400,
                0.35,
                div().size(px(7.0)).flex_none().rounded_full().bg(t.cyan),
            ))
        })
        .child(
            icon(if on { Icon::Mic } else { Icon::MicOff })
                .size(px(13.0))
                .text_color(color),
        )
        .child(SharedString::from(bar.name.clone()))
        .child(
            div()
                .flex_none()
                .text_size(px(10.5))
                .text_color(t.dim)
                .child(concat!("v", env!("CARGO_PKG_VERSION"))),
        )
        .when(!words.is_empty(), |el| {
            el.child(
                div()
                    .id("tab-nixa-context")
                    .debug_selector(|| "tab-nixa-context".into())
                    .flex_none()
                    .text_size(px(10.5))
                    .text_color(words_color)
                    .child(SharedString::from(words)),
            )
        })
        .into_any_element()
}

fn chat_button(t: &Theme, bar: &Shown, chat_on: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let waits = !chat_on && bar.waiting > 0;
    let hover = t.hover;
    let fg = if chat_on {
        t.foreground
    } else if bar.nixa_on {
        t.blue
    } else {
        t.muted
    };
    div()
        .id("tab-nixa-card")
        .tab_index(0)
        .debug_selector(|| "tab-nixa-card".into())
        .relative()
        .size(px(28.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(t.border)
        .bg(if chat_on { t.selected } else { t.chrome })
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(chat_tip(&bar.name, chat_on, waits)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|ws, _: &ClickEvent, _, cx| toggle_chat(ws, cx)))
        .child(icon(Icon::Chat).size(px(13.0)).text_color(fg))
        .when(waits, |el| {
            el.child(pulse(
                "tab-chat-dot",
                900,
                0.4,
                div()
                    .debug_selector(|| "tab-chat-dot".into())
                    .absolute()
                    .top(px(4.0))
                    .right(px(4.0))
                    .size(px(7.0))
                    .rounded_full()
                    .bg(t.yellow),
            ))
        })
        .into_any_element()
}

fn window_row(t: &Theme, window: &Window, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let maximized = window.is_maximized();
    vec![
        plain_button(
            t,
            "minimize",
            glyph_icon(t, Icon::Minus, 13.0),
            "Minimize".into(),
        )
        .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.minimize_window()))
        .into_any_element(),
        plain_button(
            t,
            "maximize",
            glyph_icon(t, if maximized { Icon::Copy } else { Icon::Square }, 12.0),
            if maximized {
                "Restore size".into()
            } else {
                "Maximize".into()
            },
        )
        .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.zoom_window()))
        .into_any_element(),
        plain_button(
            t,
            "close-window",
            glyph_icon(t, Icon::X, 13.0),
            "Close window".into(),
        )
        .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.remove_window()))
        .into_any_element(),
    ]
}

fn menu_layer(bar: &Shown, window: &Window, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let Some((view, at)) = bar.menu.clone() else {
        return Vec::new();
    };
    let size = window.viewport_size();
    vec![
        deferred(
            anchored().position(point(px(0.0), px(0.0))).child(
                div()
                    .id("new-menu-scrim")
                    .debug_selector(|| "new-menu-scrim".into())
                    .w(size.width)
                    .h(size.height)
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ws, _, _, cx| close_menu(ws, cx)),
                    ),
            ),
        )
        .with_priority(3)
        .into_any_element(),
        deferred(
            anchored()
                .position(at)
                .anchor(Corner::TopRight)
                .snap_to_window_with_margin(px(8.0))
                .child(view),
        )
        .with_priority(4)
        .into_any_element(),
    ]
}

pub fn render(ws: &Workspace, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    render_with_titlebar_action(ws, window, cx, Window::titlebar_double_click)
}

pub(super) fn render_with_titlebar_action(
    ws: &Workspace,
    window: &Window,
    cx: &mut Context<Workspace>,
    titlebar_action: impl Fn(&Window) + 'static,
) -> AnyElement {
    if ws.palette.is_some() && ws.tab_bar.read(cx).menu.is_some() {
        ws.tab_bar.update(cx, |bar, _| bar.menu = None);
    }
    if ws.tab_bar.read(cx).open_at_start {
        ws.tab_bar.update(cx, |bar, _| bar.open_at_start = false);
        cx.defer_in(window, |ws, window, cx| open_menu(ws, None, window, cx));
    }
    let t = theme(cx).clone();
    let bar = shown(ws.tab_bar.read(cx));
    let chat_on = ws.panel == Some(PanelKind::Nixa);
    let mac = cfg!(target_os = "macos");
    let pad = lights_pad(mac, window.is_fullscreen(), RAIL_WIDTH);
    let name = bar.name.clone();
    let nixa_on = bar.nixa_on;
    let agent_voice = bar.agent_voice;
    let menu_open = bar.menu.is_some();
    let pill = nixa_pill(&t, &bar, cx);
    let chat = chat_button(&t, &bar, chat_on, cx);
    let talk = super::talk::button(ws, &t, cx);
    let layer = menu_layer(&bar, window, cx);
    let quick = quick_buttons(&t, &bar.quick, cx);
    let buttons = if window_buttons(mac) {
        window_row(&t, window, cx)
    } else {
        Vec::new()
    };
    let toolbar = tab_bar(&t)
        .id("tab-bar")
        .tab_group()
        .debug_selector(|| "tab-bar".into())
        .window_control_area(WindowControlArea::Drag)
        .pl(px(pad))
        .child(
            plain_button(&t, "tab-settings", glyph_icon(&t, Icon::Settings, 14.0), "Nixa settings".into())
                .on_click(|_, _, cx| cx.defer(|cx| crate::screens::settings::open("General", cx))),
        )
        .child(pill)
        .children(talk)
        .child(chat)
        .child(
            tinted_icon_button(
                &t,
                "tab-nixa-core",
                Icon::Core,
                IconSize::Toolbar,
                false,
                if nixa_on { t.cyan } else { t.accent },
            )
            .debug_selector(|| "tab-nixa-core".into())
            .tooltip(tooltip(format!(
                "{name} screen: {name}, the deep brain and the agents as a live map ({}).", crate::screens::nixa_screen::KEY
            )))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|ws, _: &ClickEvent, _, cx| {
                crate::screens::nixa_screen::host::open(ws, cx)
            })),
        )
        .child(
            tinted_icon_button(
                &t,
                "tab-voice",
                if agent_voice { Icon::VoiceOn } else { Icon::VoiceOff },
                IconSize::Toolbar,
                false,
                if agent_voice { t.dim } else { t.muted },
            )
            .debug_selector(|| "tab-voice".into())
            .tooltip(tooltip(voice_tip(&name, agent_voice)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|ws, _: &ClickEvent, _, cx| toggle_agent_voice(ws, cx))),
        )
        .child(
            div()
                .id("quick-agent-strip")
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .items_center()
                .gap(px(3.0))
                .overflow_x_scroll()
                .child(
                    div()
                        .id("titlebar-drag-space")
                        .debug_selector(|| "titlebar-drag-space".into())
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                            if titlebar_double_click(event.click_count, window.is_fullscreen()) {
                                cx.stop_propagation();
                                titlebar_action(window);
                            }
                        }),
                )
                .children(quick),
        )
        .child(
            plain_button(&t, "tab-new", glyph_icon(&t, Icon::Plus, 14.0), "New agent or tab".into())
                .when(menu_open, |el| el.bg(t.selected))
                .on_click(cx.listener(|ws, event: &ClickEvent, window, cx| {
                    open_menu(ws, Some(event.position()), window, cx)
                })),
        )
        .children(buttons)
        .children(layer)
        .children(super::talk::menu(ws, &t, cx));
    div().flex().flex_col().flex_none().min_w_0().child(toolbar).into_any_element()
}
