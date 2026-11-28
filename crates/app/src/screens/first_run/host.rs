use super::{Action, FirstRunEvent, FirstRunView, SHOW};
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Global};

struct Shared(Entity<FirstRunView>);

impl Global for Shared {}

pub fn attach(store: Option<&nixa_store::Connection>, cx: &mut Context<Workspace>) {
    if view(cx).is_some() {
        return;
    }
    let done = cfg!(test) || super::is_done(store);
    let view = super::new(super::live(), nixa_protocol::paths::brain_addr(), done, cx);
    cx.subscribe(&view, |ws, _, event: &FirstRunEvent, cx| handle(ws, event, cx))
        .detach();
    cx.observe(&view, |_, _, cx| cx.notify()).detach();
    cx.set_global(Shared(view));
}

pub fn view(cx: &App) -> Option<Entity<FirstRunView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn names(ws: &Workspace) -> Vec<String> {
    ws.spaces.spaces.iter().map(|s| s.name.clone()).collect()
}

pub fn shown(ws: &Workspace, cx: &App) -> bool {
    view(cx).is_some_and(|v| {
        let v = v.read(cx);
        super::wanted(ws.spaces.spaces.len(), v.asked)
    })
}

pub fn sync(ws: &mut Workspace, cx: &mut App) {
    space_added(ws, cx);
    let Some(view) = view(cx) else {
        return;
    };
    let names = names(ws);
    let on = shown(ws, cx);
    view.update(cx, |v, cx| {
        let was = v.shown;
        if super::set_spaces(v, names, on) {
            if on && !was {
                super::refresh(v, cx);
            }
            cx.notify();
        }
    });
}

pub fn render(ws: &Workspace, cx: &App) -> Option<AnyElement> {
    if !shown(ws, cx) {
        return None;
    }
    view(cx).map(|v| v.into_any_element())
}

pub fn handle(ws: &mut Workspace, event: &FirstRunEvent, cx: &mut Context<Workspace>) {
    match event {
        FirstRunEvent::PickSpace => ws.pick_space_folder(cx),
        FirstRunEvent::Finish => {
            save_done(ws);
            ws.refocus = true;
            cx.notify();
        }
        FirstRunEvent::Settings(part) => {
            let part = *part;
            cx.defer(move |cx| crate::screens::settings::open(part, cx));
        }
    }
}

fn save_done(ws: &mut Workspace) {
    if let Some(conn) = &ws.store
        && let Err(e) = super::mark_done(conn)
    {
        ws.notice = Some(format!("Cannot save that the first run is done: {e:#}"));
    }
}

pub fn space_added(ws: &mut Workspace, cx: &mut App) {
    let Some(view) = view(cx) else {
        return;
    };
    if ws.spaces.spaces.is_empty() || view.read(cx).done {
        return;
    }
    view.update(cx, |v, cx| {
        v.done = true;
        cx.notify();
    });
    save_done(ws);
}

pub fn close(cx: &mut App) {
    if let Some(view) = view(cx) {
        view.update(cx, |v, cx| {
            if super::close(v) {
                cx.notify();
            }
        });
    }
}

pub fn need_space(cx: &mut App) {
    if let Some(view) = view(cx) {
        view.update(cx, |v, cx| {
            if super::ask_for_space(v) {
                cx.notify();
            }
        });
    }
}

pub fn run(action: Action, cx: &mut App) {
    match action {
        Action::Open => {
            if let Some(view) = view(cx) {
                view.update(cx, |v, cx| {
                    v.focus_pending = !v.asked;
                    v.asked = true;
                    super::refresh(v, cx);
                    cx.notify();
                });
            }
        }
    }
}

pub fn show(what: &str, cx: &mut App) -> bool {
    if what != SHOW {
        return false;
    }
    run(Action::Open, cx);
    true
}
