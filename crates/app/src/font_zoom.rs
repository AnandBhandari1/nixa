use crate::workspace::Workspace;
use gpui::{App, Context, Entity, Global};
use nixa_store::Connection;
use nixa_termview::{DEFAULT_FONT_SIZE, TermView};

pub const SIZE_KEY: &str = "app.terminal_font_size";
pub const MIN_SIZE: f32 = 8.0;
pub const MAX_SIZE: f32 = 32.0;
pub const STEP: f32 = 1.0;

struct FontSize(f32);

impl Global for FontSize {}

pub fn stepped(now: f32, step: isize) -> f32 {
    (now + STEP * step as f32).clamp(MIN_SIZE, MAX_SIZE)
}

pub fn saved(store: Option<&Connection>) -> f32 {
    store
        .and_then(|conn| nixa_store::settings::get_setting(conn, SIZE_KEY).ok().flatten())
        .and_then(|v| v.as_f64())
        .map(|v| (v as f32).clamp(MIN_SIZE, MAX_SIZE))
        .unwrap_or(DEFAULT_FONT_SIZE)
}

pub fn size(store: Option<&Connection>, cx: &mut App) -> f32 {
    if let Some(now) = cx.try_global::<FontSize>() {
        return now.0;
    }
    let now = saved(store);
    cx.set_global(FontSize(now));
    now
}

pub fn apply(store: Option<&Connection>, view: &Entity<TermView>, cx: &mut App) {
    let now = size(store, cx);
    let family = crate::screens::settings::parts::app::terminal_family(store);
    view.update(cx, |v, cx| v.set_font(&family, now, cx));
}

pub fn step(ws: &mut Workspace, step: isize, cx: &mut Context<Workspace>) {
    let now = size(ws.store.as_ref(), cx);
    set(ws, stepped(now, step), cx);
}

pub fn reset(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    set(ws, DEFAULT_FONT_SIZE, cx);
}

fn set(ws: &mut Workspace, next: f32, cx: &mut Context<Workspace>) {
    cx.set_global(FontSize(next));
    let family = crate::screens::settings::parts::app::terminal_family(ws.store.as_ref());
    for view in ws.views.values() {
        view.update(cx, |v, cx| v.set_font(&family, next, cx));
    }
    if let Some(conn) = ws.store.as_ref()
        && let Err(e) = nixa_store::settings::set_setting(conn, SIZE_KEY, &serde_json::json!(next))
    {
        ws.notice = Some(format!("Cannot save the terminal font size: {e:#}"));
    }
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_stay_inside_the_limits() {
        assert_eq!(stepped(14.0, 1), 15.0);
        assert_eq!(stepped(14.0, -1), 13.0);
        assert_eq!(stepped(MAX_SIZE, 1), MAX_SIZE);
        assert_eq!(stepped(MIN_SIZE, -1), MIN_SIZE);
    }

    #[test]
    fn no_store_gives_the_default_size() {
        assert_eq!(saved(None), DEFAULT_FONT_SIZE);
    }
}
