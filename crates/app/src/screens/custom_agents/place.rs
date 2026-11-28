use crate::layout;
use crate::workspace::Workspace;
use gpui::{App, WindowHandle};
use nixa_protocol::TermId;

pub fn workspace(cx: &App) -> Option<WindowHandle<Workspace>> {
    cx.windows()
        .into_iter()
        .find_map(|w| w.downcast::<Workspace>())
}

pub fn active_space(cx: &App) -> Option<String> {
    let handle = workspace(cx)?;
    handle.read(cx).ok()?.active_space().map(|s| s.id.clone())
}

pub fn tab_of(ws: &Workspace, id: &TermId) -> Option<String> {
    ws.tabs
        .list
        .iter()
        .find(|t| layout::leaves(&t.root).contains(id))
        .map(|t| t.id.clone())
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
            let Some(tab) = tab_of(ws, &id) else {
                return false;
            };
            ws.select_tab(&tab, cx);
            ws.focus_pane(&tab, &id, cx);
            window.activate_window();
            true
        })
        .unwrap_or(false)
}
