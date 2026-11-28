use crate::layout;
use crate::links::brain::Link;
use crate::workspace::Workspace;
use gpui::{App, WindowHandle};
use nixa_protocol::TermId;
use std::sync::OnceLock;

pub const SPACES_PART: &str = crate::screens::settings::parts::spaces::PART;

pub fn workspace(cx: &App) -> Option<WindowHandle<Workspace>> {
    cx.windows()
        .into_iter()
        .find_map(|w| w.downcast::<Workspace>())
}

pub fn link(cx: &App) -> Option<Link> {
    let handle = workspace(cx)?;
    handle.read(cx).ok()?.hub.link.clone()
}

pub fn active_space(cx: &App) -> String {
    workspace(cx)
        .and_then(|h| {
            h.read(cx)
                .ok()
                .and_then(|ws| ws.active_space().map(|s| s.id.clone()))
        })
        .unwrap_or_default()
}

pub fn settings(part: &str, cx: &mut App) {
    let part = part.to_string();
    cx.defer(move |cx| crate::screens::settings::open(&part, cx));
}

pub fn focus(terminal: &str, cx: &mut App) -> bool {
    let Ok(id) = terminal.trim().parse::<TermId>() else {
        return false;
    };
    let Some(handle) = workspace(cx) else {
        return false;
    };
    handle
        .update(cx, |ws, window, cx| {
            let tab = ws
                .tabs
                .list
                .iter()
                .find(|t| layout::leaves(&t.root).contains(&id))
                .map(|t| t.id.clone());
            let Some(tab) = tab else {
                return false;
            };
            ws.select_tab(&tab, cx);
            ws.focus_pane(&tab, &id, cx);
            window.activate_window();
            true
        })
        .unwrap_or(false)
}

pub fn offset_min() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| i64::from(crate::screens::tabbar::local_offset_min()))
}
