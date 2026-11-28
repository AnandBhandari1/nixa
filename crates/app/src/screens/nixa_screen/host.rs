use super::prefs::Look;
use super::view::{NixaScreen, ScreenEvent};
use super::{Command, PART, Shared, input, mode, now_ms};
use crate::links::brain::Link;
use crate::screens::menu::switches::{self, Switch, SwitchEvent};
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Subscription, deferred, div};
use nixa_protocol::paths;

pub const PRIORITY: usize = 20;

pub fn new(link: Option<Link>, cx: &mut Context<Workspace>) -> Subscription {
    let store = if cfg!(test) {
        None
    } else {
        nixa_store::open_default().ok()
    };
    let screen = cx.new(|cx| NixaScreen::build(link, store, cx));
    cx.set_global(Shared(screen.clone()));
    let events = cx.subscribe(&screen, |ws, _, event: &ScreenEvent, cx| {
        on_event(ws, event, cx)
    });
    if !cfg!(test) && screen.read(cx).prefs.keep_open {
        screen.update(cx, |s, cx| s.show(cx));
    }
    events
}

pub fn screen(cx: &App) -> Option<Entity<NixaScreen>> {
    super::shared(cx)
}

pub fn open(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(screen) = screen(cx) else {
        return;
    };
    screen.update(cx, |s, cx| s.show(cx));
    sync(ws, cx);
    cx.notify();
}

pub fn close(cx: &mut Context<Workspace>) {
    if let Some(screen) = screen(cx) {
        screen.update(cx, |s, cx| s.hide(cx));
    }
    cx.notify();
}

pub fn toggle(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(screen) = screen(cx) else {
        return;
    };
    let (shown, keep) = {
        let s = screen.read(cx);
        (s.open, s.prefs.keep_open)
    };
    if !shown {
        open(ws, cx);
    } else if !keep {
        close(cx);
    }
}

pub fn turn_nixa_on(cx: &mut App) {
    let Some(view) = switches::shared(cx) else {
        return;
    };
    view.update(cx, |v, cx| {
        if switches::get(&v.state, Switch::Nixa) == Some(true) {
            return;
        }
        if let Some(on) = switches::flip(v, Switch::Nixa) {
            cx.emit(SwitchEvent::Send(switches::tell(Switch::Nixa, on)));
        }
        cx.notify();
    });
}

pub fn on_event(ws: &mut Workspace, event: &ScreenEvent, cx: &mut Context<Workspace>) {
    match event {
        ScreenEvent::Close => close(cx),
        ScreenEvent::Settings => {
            close(cx);
            ws.open_settings(PART, cx);
        }
        ScreenEvent::TurnNixaOn => turn_nixa_on(cx),
        ScreenEvent::Show => {
            open(ws, cx);
            cx.activate(true);
        }
        ScreenEvent::Notice(text) => {
            ws.notice = Some(text.clone());
            cx.notify();
        }
    }
}

pub fn off_now() -> bool {
    paths::data_dir().join(switches::OFF_FILE).exists()
}

pub fn sync(ws: &Workspace, cx: &mut Context<Workspace>) {
    let Some(screen) = screen(cx) else {
        return;
    };
    if !screen.read(cx).wanted() {
        return;
    }
    let now = now_ms();
    let feed = mode::feed_of(&ws.hub.chat.read(cx).snap, off_now(), now);
    let split = input::build(ws);
    let rooms = input::rooms(ws);
    screen.update(cx, |s, _| {
        s.set_feed(feed);
        s.set_rooms(rooms);
        s.set_agents(split.agents, split.deep, now);
    });
}

pub fn overlay(cx: &App) -> Option<AnyElement> {
    let screen = screen(cx).filter(|s| s.read(cx).open)?;
    Some(
        deferred(div().absolute().top_0().left_0().size_full().child(screen))
            .with_priority(PRIORITY)
            .into_any_element(),
    )
}

pub fn run(ws: &mut Workspace, command: Command, cx: &mut Context<Workspace>) {
    let Some(screen) = screen(cx) else {
        return;
    };
    match command {
        Command::Open => open(ws, cx),
        Command::Close => close(cx),
        Command::KeepOpen => screen.update(cx, |s, cx| {
            let on = !s.prefs.keep_open;
            s.set_keep(on, cx)
        }),
        Command::Quiet => screen.update(cx, |s, cx| {
            let on = !s.prefs.quiet;
            s.set_quiet(on, cx)
        }),
        Command::Office => screen.update(cx, |s, cx| {
            let next = match s.prefs.look {
                Look::Universe => Look::Office,
                Look::Office => Look::Universe,
            };
            s.set_look(next, cx)
        }),
        Command::CameraCut => screen.update(cx, |s, cx| {
            let still = !s.prefs.still;
            s.set_still(still, cx)
        }),
        Command::Soap => screen.update(cx, |s, cx| {
            let on = !s.prefs.soap;
            s.set_soap(on, cx)
        }),
    }
}
