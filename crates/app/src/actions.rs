use crate::pane_moves::{self, Dir};
use crate::screens::agents::{self, AgentsEvent};
use crate::screens::spaces::{self, Form};
use crate::workspace::Workspace;
use crate::links::brain::NOT_CONNECTED;
use gpui::{Action, App, Context, Keystroke, Window, WindowHandle, actions};
use nixa_ui::metrics::SIDEBAR_NARROW;
use serde_json::json;
use std::time::Duration;

pub const KEYS_ENV: &str = "NIXA_APP_KEYS";
const KEYS_WAIT: Duration = Duration::from_millis(4000);
const KEY_GAP: Duration = Duration::from_millis(600);

actions!(
    nixa,
    [
        NewTab,
        SplitRight,
        SplitDown,
        ClosePane,
        CloseTab,
        OpenCommands,
        OpenSettings,
        ToggleSidebar,
        NextTab,
        PrevTab,
        ZoomPane,
        OpenMenu,
        MenuTasks,
        MenuProjectCommands,
        MenuFavourites,
        NextAgent,
        PrevAgent,
        OpenPicker,
        OpenFavourites,
        NewAgentMenu,
        NixaScreen,
        RenameTab,
        RenameSpace,
        NextSpace,
        PrevSpace,
        Listen,
        ToggleNixa,
        NewSpace,
        OpenHelp,
        OpenLayouts,
        FontBigger,
        FontSmaller,
        FontReset,
        Hide,
        HideOthers,
        OpenAbout,
        Quit
    ]
);

#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = nixa, no_json)]
pub struct PickTab {
    pub number: usize,
}

#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = nixa, no_json)]
pub struct PickSpace {
    pub number: usize,
}

#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = nixa, no_json)]
pub struct GoPane {
    pub dir: Dir,
}

#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = nixa, no_json)]
pub struct SwapPane {
    pub dir: Dir,
}

pub fn bind(cx: &mut App) {
    crate::keys::load(cx);
    cx.on_action(|a: &PickTab, cx| later(cx, a.number, |ws, n, _, cx| pick_tab(ws, n, cx)));
    cx.on_action(|a: &PickSpace, cx| later(cx, a.number, |ws, n, _, cx| pick_space(ws, n, cx)));
    cx.on_action(|_: &NextSpace, cx| later(cx, 0, |ws, _, _, cx| step_space(ws, 1, cx)));
    cx.on_action(|_: &PrevSpace, cx| later(cx, 0, |ws, _, _, cx| step_space(ws, -1, cx)));
    cx.on_action(|_: &RenameTab, cx| later(cx, 0, |ws, _, _, cx| rename_tab(ws, cx)));
    cx.on_action(|_: &CloseTab, cx| {
        if main_window(cx).is_none() {
            cx.propagate();
            return;
        }
        later(cx, 0, |ws, _, _, cx| close_active_tab(ws, cx))
    });
    cx.on_action(|_: &NewSpace, cx| later(cx, 0, |ws, _, _, cx| ws.pick_space_folder(cx)));
    cx.on_action(|a: &GoPane, cx| {
        later(cx, pane_moves::index(a.dir), |ws, n, _, cx| {
            pane_moves::go(ws, pane_moves::dir_at(n), cx)
        })
    });
    cx.on_action(|a: &SwapPane, cx| {
        later(cx, pane_moves::index(a.dir), |ws, n, _, cx| {
            pane_moves::swap(ws, pane_moves::dir_at(n), cx)
        })
    });
    cx.on_action(|_: &FontBigger, cx| later(cx, 0, |ws, _, _, cx| crate::font_zoom::step(ws, 1, cx)));
    cx.on_action(|_: &FontSmaller, cx| {
        later(cx, 0, |ws, _, _, cx| crate::font_zoom::step(ws, -1, cx))
    });
    cx.on_action(|_: &FontReset, cx| later(cx, 0, |ws, _, _, cx| crate::font_zoom::reset(ws, cx)));
    cx.on_action(|_: &OpenHelp, cx| crate::keys::help::open(cx));
    cx.on_action(|_: &OpenLayouts, cx| crate::named_layouts::open(cx));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &OpenAbout, cx| crate::screens::settings::open(crate::screens::settings::parts::app::PART, cx));
    cx.on_action(|_: &Listen, cx| later(cx, 0, |ws, _, _, cx| listen(ws, cx)));
    cx.on_action(|_: &ToggleNixa, cx| {
        later(cx, 0, |ws, _, _, cx| crate::screens::tabbar::toggle_nixa(ws, cx))
    });
    cx.on_action(|_: &RenameSpace, cx| {
        later(cx, 0, |ws, _, window, cx| rename_space(ws, window, cx))
    });
    play_keys(cx);
}

fn main_window(cx: &App) -> Option<WindowHandle<Workspace>> {
    match cx.active_window() {
        Some(window) => window.downcast::<Workspace>(),
        None => cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>()),
    }
}

fn later(
    cx: &mut App,
    number: usize,
    run: fn(&mut Workspace, usize, &mut Window, &mut Context<Workspace>),
) {
    let Some(handle) = main_window(cx) else {
        return;
    };
    cx.defer(move |cx| {
        handle.update(cx, |ws, window, cx| run(ws, number, window, cx)).ok();
    });
}

pub fn nth(ids: &[String], number: usize) -> Option<String> {
    number.checked_sub(1).and_then(|i| ids.get(i)).cloned()
}

pub fn stepped(ids: &[String], now: Option<&str>, step: isize) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let at = now.and_then(|n| ids.iter().position(|i| i == n));
    let next = match at {
        Some(at) => (at as isize + step).rem_euclid(ids.len() as isize) as usize,
        None if step < 0 => ids.len() - 1,
        None => 0,
    };
    ids.get(next).cloned()
}

fn space_ids(ws: &Workspace) -> Vec<String> {
    ws.spaces.spaces.iter().map(|s| s.id.clone()).collect()
}

fn go_to_space(ws: &mut Workspace, id: &str, cx: &mut Context<Workspace>) {
    ws.select_space(id, cx);
    let first = ws.space_tabs(id).next().map(|t| t.id.clone());
    if ws.active_tab().is_none()
        && let Some(first) = first
    {
        ws.select_tab(&first, cx);
    }
    ws.refocus = true;
}

pub fn pick_tab(ws: &mut Workspace, number: usize, cx: &mut Context<Workspace>) {
    let Some(space) = ws.active_space().map(|s| s.id.clone()) else {
        return;
    };
    let ids: Vec<String> = ws.space_tabs(&space).map(|t| t.id.clone()).collect();
    if let Some(id) = nth(&ids, number) {
        ws.select_tab(&id, cx);
    }
}

pub fn pick_space(ws: &mut Workspace, number: usize, cx: &mut Context<Workspace>) {
    if let Some(id) = nth(&space_ids(ws), number) {
        go_to_space(ws, &id, cx);
    }
}

pub fn step_space(ws: &mut Workspace, step: isize, cx: &mut Context<Workspace>) {
    let now = ws.spaces.active.clone();
    if let Some(id) = stepped(&space_ids(ws), now.as_deref(), step) {
        go_to_space(ws, &id, cx);
    }
}

pub fn rename_tab(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(tab) = ws.active_tab().map(|t| t.id.clone()) else {
        return;
    };
    agents::on_event(ws, &AgentsEvent::Rename { tab }, cx);
}

fn screen_takes_close(cx: &mut Context<Workspace>) -> bool {
    let Some(screen) = crate::screens::nixa_screen::host::screen(cx) else {
        return false;
    };
    let (open, keep) = {
        let s = screen.read(cx);
        (s.open, s.prefs.keep_open)
    };
    if open && !keep {
        crate::screens::nixa_screen::host::close(cx);
    }
    open
}

pub fn close_active_tab(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    if screen_takes_close(cx) {
        return;
    }
    if let Some(tab) = ws.active_tab().map(|t| t.id.clone()) {
        ws.close_tab(&tab, cx);
    }
}

pub fn hold_kind(holding: bool) -> &'static str {
    if holding { "hold_end" } else { "hold_start" }
}

pub fn listen(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let holding = ws.hub.chat.read(cx).snap.key_listen == "on";
    let sent = match &ws.hub.link {
        Some(link) => link.send(hold_kind(holding), json!({})),
        None => Err(anyhow::anyhow!(NOT_CONNECTED)),
    };
    if let Err(e) = sent {
        ws.notice = Some(format!("Nixa cannot listen now: {e:#}"));
        cx.notify();
    }
}

pub fn rename_space(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(id) = ws.spaces.active.clone() else {
        return;
    };
    if ws.sidebar_w < SIDEBAR_NARROW {
        ws.toggle_sidebar(cx);
    }
    let view = ws.spaces_view.clone();
    spaces::run(&view, spaces::Action::Rename, Some(&id), window, cx);
    cx.notify();
}

pub fn key_list(text: &str) -> Vec<Keystroke> {
    text.split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .filter_map(|k| Keystroke::parse(k).ok())
        .collect()
}

fn state_line(ws: &Workspace, cx: &App) -> String {
    let tab = ws.active_tab().map(|t| t.id.clone()).unwrap_or_default();
    let space = ws.active_space();
    let renaming = match &ws.spaces_view.read(cx).form {
        Form::Rename { id, .. } => id.clone(),
        _ => String::new(),
    };
    format!(
        "space={} name={} tab={} rename_box={} notice={}",
        space.map(|s| s.id.as_str()).unwrap_or(""),
        space.map(|s| s.name.as_str()).unwrap_or(""),
        tab,
        renaming,
        ws.notice.as_deref().unwrap_or("")
    )
}

fn play_keys(cx: &mut App) {
    let Ok(text) = std::env::var(KEYS_ENV) else {
        return;
    };
    let keys = key_list(&text);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(KEYS_WAIT).await;
        for key in keys {
            let shown = key.unparse();
            let line = cx
                .update(|cx| {
                    let handle = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>())?;
                    let any: gpui::AnyWindowHandle = handle.into();
                    any.update(cx, |_, window, cx| window.dispatch_keystroke(key, cx))
                        .ok()?;
                    Some(handle)
                })
                .ok()
                .flatten();
            cx.background_executor().timer(KEY_GAP).await;
            if let Some(handle) = line {
                let state = cx
                    .update(|cx| handle.read(cx).map(|ws| state_line(ws, cx)).ok())
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                eprintln!("nixa-app keys: {shown} -> {state}");
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests;
