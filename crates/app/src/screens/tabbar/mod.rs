pub mod context;
mod models;
mod prefs;
pub mod panels;
pub mod quick;
pub mod talk;
mod view;

#[cfg(test)]
mod tests;

pub use quick::QUICK_AGENTS;
pub use view::{render, title_double_click};

use crate::links::brain::{self, Link};
use crate::programs;
use crate::screens::new_menu::{self, MenuEvent, NewMenu, OnEvent, Setup};
use crate::workspace::{PanelKind, Workspace};
use context::BrainContext;
use gpui::{App, AppContext, Context, Entity, Pixels, Point, Window, point, px};
use nixa_ui::metrics::TAB_BAR_HEIGHT;
use serde_json::{Value, json};
use std::rc::Rc;
use std::time::Duration;

pub const ENV_OPEN_MENU: &str = "NIXA_APP_NEW_MENU";
const POLL: Duration = Duration::from_secs(5);
const RECHECK: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewMenu,
    ToggleChat,
    ToggleNixa,
    ToggleAgentVoice,
    NixaScreen,
}

pub struct Menu {
    pub view: Entity<NewMenu>,
    pub at: Point<Pixels>,
}

pub struct TabBar {
    pub name: String,
    pub context: Option<BrainContext>,
    pub nixa_on: bool,
    pub agent_voice: bool,
    pub waiting: usize,
    pub menu: Option<Menu>,
    pub offset_min: i32,
    pub link: Option<Link>,
    pub open_at_start: bool,
    pub quick: Vec<quick::Button>,
    pub talk_menu: Option<Point<Pixels>>,
}

#[derive(Default)]
pub struct Seen {
    pub context: Option<Option<BrainContext>>,
    pub nixa_on: Option<bool>,
    pub agent_voice: Option<bool>,
    pub quick: Option<Vec<quick::Button>>,
}

pub fn build(link: Option<Link>, offset_min: i32) -> TabBar {
    TabBar {
        name: "Nixa".into(),
        context: None,
        nixa_on: false,
        agent_voice: true,
        waiting: 0,
        menu: None,
        offset_min,
        link,
        open_at_start: false,
        quick: Vec::new(),
        talk_menu: None,
    }
}

const AGENT_VOICE_KEY: &str = "voice.agents";

fn saved_agent_voice(conn: Option<&nixa_store::Connection>) -> bool {
    conn.and_then(|c| nixa_store::settings::setting_bool(c, AGENT_VOICE_KEY, true).ok())
        .unwrap_or(true)
}

pub fn new(link: Option<Link>, cx: &mut App) -> Entity<TabBar> {
    let agent_voice = if cfg!(test) {
        true
    } else {
        saved_agent_voice(nixa_store::open_default().ok().as_ref())
    };
    let offset = if cfg!(test) { 0 } else { local_offset_min() };
    let open = !cfg!(test) && std::env::var(ENV_OPEN_MENU).is_ok_and(|v| v == "1");
    let view = cx.new(|_| TabBar {
        open_at_start: open,
        agent_voice,
        ..build(link, offset)
    });
    if !cfg!(test) {
        poll(&view, cx);
    }
    view
}

pub fn local_offset_min() -> i32 {
    std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|o| parse_offset(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(0)
}

pub fn parse_offset(text: &str) -> Option<i32> {
    let text = text.trim();
    let (sign, rest) = match text.chars().next()? {
        '+' => (1, &text[1..]),
        '-' => (-1, &text[1..]),
        _ => return None,
    };
    if rest.len() != 4 || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = rest[..2].parse().ok()?;
    let minutes: i32 = rest[2..].parse().ok()?;
    Some(sign * (hours * 60 + minutes))
}

pub fn nixa_on_of(value: &Value) -> Option<bool> {
    let settings = value.get("voice")?.get("settings")?;
    let flag = |key: &str| settings.get(key).and_then(Value::as_bool).unwrap_or(false);
    Some(flag("micOn") || flag("voiceOn"))
}

pub fn waiting_of(value: &Value) -> usize {
    let count = value.get("waiting").and_then(Value::as_u64).unwrap_or(0) as usize;
    let card = value.get("card").is_some_and(|c| !c.is_null());
    if card { count.max(1) } else { count }
}

pub fn gather(link: Option<&Link>) -> Seen {
    let linked = link.filter(|l| l.connected());
    Seen {
        context: linked
            .and_then(|l| l.get("/context").ok())
            .map(|v| context::parse(&v)),
        nixa_on: linked
            .and_then(|l| l.get("/voice").ok())
            .and_then(|v| nixa_on_of(&v)),
        agent_voice: linked
            .and_then(|l| l.get("/announce").ok())
            .and_then(|v| agent_voice_of(&v)),
        quick: None,
    }
}

pub fn gather_all(link: Option<&Link>) -> Seen {
    Seen {
        quick: Some(quick::buttons_in(
            &programs::default_path(),
            &programs::default_dirs(),
        )),
        ..gather(link)
    }
}

pub fn agent_voice_of(value: &Value) -> Option<bool> {
    value.get("on").and_then(Value::as_bool)
}

pub fn take(bar: &mut TabBar, seen: Seen) {
    if let Some(context) = seen.context {
        bar.context = context::keep_known(bar.context.take(), context);
    }
    if let Some(on) = seen.nixa_on {
        bar.nixa_on = on;
    }
    if let Some(on) = seen.agent_voice {
        bar.agent_voice = on;
    }
    if let Some(list) = seen.quick {
        bar.quick = list;
    }
}

fn poll(view: &Entity<TabBar>, cx: &mut App) {
    let weak = view.downgrade();
    cx.spawn(async move |cx| {
        loop {
            let Ok(link) = weak.read_with(cx, |bar, _| bar.link.clone()) else {
                break;
            };
            let seen = cx
                .background_executor()
                .spawn(async move { gather_all(link.as_ref()) })
                .await;
            if weak
                .update(cx, |bar, cx| {
                    take(bar, seen);
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
            cx.background_executor().timer(POLL).await;
        }
    })
    .detach();
}

pub fn refresh(view: &Entity<TabBar>, wait: Duration, cx: &mut App) {
    let weak = view.downgrade();
    let link = view.read(cx).link.clone();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(wait).await;
        let seen = cx
            .background_executor()
            .spawn(async move { gather_all(link.as_ref()) })
            .await;
        weak.update(cx, |bar, cx| {
            take(bar, seen);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn apply(view: &Entity<TabBar>, msg: &Value, cx: &mut App) {
    let kind = msg.get("type").and_then(Value::as_str).unwrap_or_default();
    let name = msg
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    match (kind, nixa_on_of(msg), name) {
        ("voice", Some(on), _) => view.update(cx, |bar, cx| {
            bar.nixa_on = on;
            cx.notify();
        }),
        ("prompts", _, _) => {
            let waiting = waiting_of(msg);
            view.update(cx, |bar, cx| {
                bar.waiting = waiting;
                cx.notify();
            });
        }
        ("agent_voice", _, _) if agent_voice_of(msg).is_some() => {
            let on = agent_voice_of(msg).unwrap_or(true);
            view.update(cx, |bar, cx| {
                bar.agent_voice = on;
                cx.notify();
            });
        }
        ("health", _, Some(name)) => view.update(cx, |bar, cx| {
            bar.name = name;
            cx.notify();
        }),
        ("usage", _, _) => {
            if let Some(pct) = crate::screens::chat::model::context_pct(msg) {
                view.update(cx, |bar, cx| {
                    bar.context = Some(context::from_pct(bar.context.take(), pct));
                    cx.notify();
                });
            }
        }
        ("reply" | "chat_cleaned", _, _) if !cfg!(test) => refresh(view, Duration::ZERO, cx),
        _ => {}
    }
}

pub fn commands() -> Vec<(&'static str, Action)> {
    vec![
        ("Open the new agent menu", Action::NewMenu),
        ("Show or hide Nixa chat", Action::ToggleChat),
        ("Turn Nixa on or off", Action::ToggleNixa),
        ("Turn agent voice on or off", Action::ToggleAgentVoice),
        ("Open the Nixa screen", Action::NixaScreen),
    ]
}

pub fn run(ws: &mut Workspace, action: Action, window: &mut Window, cx: &mut Context<Workspace>) {
    match action {
        Action::NewMenu => open_menu(ws, None, window, cx),
        Action::ToggleChat => toggle_chat(ws, cx),
        Action::ToggleNixa => toggle_nixa(ws, cx),
        Action::ToggleAgentVoice => toggle_agent_voice(ws, cx),
        Action::NixaScreen => crate::screens::nixa_screen::host::open(ws, cx),
    }
}

pub fn toggle_chat(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let next = if ws.panel == Some(PanelKind::Nixa) {
        None
    } else {
        Some(PanelKind::Nixa)
    };
    ws.set_panel(next, cx);
}

pub fn toggle_nixa(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let (on, link) = {
        let bar = ws.tab_bar.read(cx);
        (!bar.nixa_on, bar.link.clone())
    };
    let sent = match &link {
        Some(link) => link
            .send("set_mic", json!({"on": on}))
            .and_then(|_| link.send("set_voice", json!({"on": on}))),
        None => Err(anyhow::anyhow!(brain::NOT_CONNECTED)),
    };
    match sent {
        Ok(()) => {
            ws.tab_bar.update(cx, |bar, cx| {
                bar.nixa_on = on;
                cx.notify();
            });
            if !cfg!(test) {
                refresh(&ws.tab_bar, RECHECK, cx);
            }
        }
        Err(e) => ws.notice = Some(format!("Nixa was not switched: {e:#}")),
    }
    cx.notify();
}

pub fn toggle_agent_voice(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let (on, link) = {
        let bar = ws.tab_bar.read(cx);
        (!bar.agent_voice, bar.link.clone())
    };
    let saved = ws.store.as_ref().ok_or_else(|| anyhow::anyhow!("Nixa settings are unavailable"))
        .and_then(|conn| nixa_store::settings::set_setting_bool(conn, AGENT_VOICE_KEY, on));
    match saved {
        Ok(()) => {
            if let Some(link) = link {
                let _ = link.send("set_agent_voice", json!({"on": on}));
            }
            ws.tab_bar.update(cx, |bar, cx| {
                bar.agent_voice = on;
                cx.notify();
            });
            ws.hub.events.events.update(cx, |view, cx| view.show_agent_voice(on, cx));
        }
        Err(e) => ws.notice = Some(format!("Agent voice was not switched: {e:#}")),
    }
    cx.notify();
}

pub fn menu_point(click: Option<Point<Pixels>>, window_width: Pixels) -> Point<Pixels> {
    let top = px(TAB_BAR_HEIGHT - 4.0);
    match click.filter(|p| p.x > px(0.0)) {
        Some(p) => point(p.x + px(13.0), top),
        None => point(window_width - px(170.0), top),
    }
}

pub fn open_menu(
    ws: &mut Workspace,
    click: Option<Point<Pixels>>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if ws.tab_bar.read(cx).menu.is_some() {
        close_menu(ws, cx);
        return;
    }
    if ws.palette.is_some() {
        ws.close_palette(window, cx);
    }
    let at = menu_point(click, window.viewport_size().width);
    let weak = cx.entity().downgrade();
    let on_event: OnEvent = Rc::new(move |event, cx: &mut App| {
        weak.update(cx, |ws, cx| on_menu(ws, event, cx)).ok();
    });
    let setup = Setup {
        file: programs::default_path(),
        dirs: programs::default_dirs(),
        link: ws.hub.link.clone(),
        space: ws.spaces.active.clone(),
        on_event,
    };
    let view = new_menu::new(setup, window, cx);
    ws.tab_bar.update(cx, |bar, cx| {
        bar.menu = Some(Menu { view, at });
        cx.notify();
    });
    cx.notify();
}

pub fn close_menu(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    ws.tab_bar.update(cx, |bar, cx| {
        bar.menu = None;
        cx.notify();
    });
    ws.refocus = true;
    cx.notify();
}

pub fn on_menu(ws: &mut Workspace, event: MenuEvent, cx: &mut Context<Workspace>) {
    let close = !matches!(event, MenuEvent::Notice(_));
    match event {
        MenuEvent::Close => {}
        MenuEvent::Terminal => ws.new_tab(Vec::new(), None, cx),
        MenuEvent::Start { name, run } => ws.new_tab(run, Some(name), cx),
        MenuEvent::Settings(part) => ws.open_settings(part, cx),
        MenuEvent::Notice(text) => ws.notice = Some(text),
    }
    if close {
        close_menu(ws, cx);
    }
    cx.notify();
}
