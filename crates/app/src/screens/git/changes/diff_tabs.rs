use super::{
    ChangesEvent,
    diff_view::{self, DiffEvent, DiffView},
    git::base_name,
};
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, FocusHandle, MouseButton, Subscription, div, px};
use nixa_ui::{
    Icon, Theme,
    icon_button::{IconSize, icon_button},
    popover::tooltip,
    tab,
};
use std::collections::HashMap;

struct DiffTab {
    id: u64,
    space: String,
    focus: FocusHandle,
    diff: Entity<DiffView>,
    _events: Subscription,
}

#[derive(Default)]
pub struct DiffTabs {
    list: Vec<DiffTab>,
    selected: HashMap<String, u64>,
    next_id: u64,
}

impl DiffTabs {
    pub fn selected(&self, space: &str) -> Option<u64> {
        self.selected.get(space).copied()
    }

    pub fn show_terminals(&mut self, space: &str) {
        self.selected.remove(space);
    }

    pub fn ids<'a>(&'a self, space: &'a str) -> impl Iterator<Item = u64> + 'a {
        self.list
            .iter()
            .filter(move |tab| tab.space == space)
            .map(|tab| tab.id)
    }

    pub fn content(&self, space: &str) -> Option<(Entity<DiffView>, FocusHandle)> {
        let id = self.selected(space)?;
        let tab = self.list.iter().find(|tab| tab.id == id)?;
        Some((tab.diff.clone(), tab.focus.clone()))
    }

    pub fn retain_spaces(&mut self, spaces: &crate::spaces::Spaces) {
        self.list
            .retain(|tab| crate::spaces::find(spaces, &tab.space).is_some());
        self.selected
            .retain(|space, _| crate::spaces::find(spaces, space).is_some());
    }
}

pub(super) fn open(repo: String, path: String, cx: &mut App) {
    let Some(window) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<Workspace>())
    else {
        return;
    };
    cx.defer(move |cx| {
        window
            .update(cx, |ws, _, cx| open_in_workspace(ws, repo, path, cx))
            .ok();
    });
}

fn open_in_workspace(ws: &mut Workspace, repo: String, path: String, cx: &mut Context<Workspace>) {
    let Some(space) = ws.spaces.active.clone() else {
        return;
    };
    let tabs = &mut ws.git_diffs;
    if let Some(id) = tabs
        .list
        .iter()
        .find(|tab| {
            let diff = tab.diff.read(cx);
            tab.space == space && diff.repo == repo && diff.path == path
        })
        .map(|tab| tab.id)
    {
        select(ws, id, cx);
        return;
    }
    let id = tabs.next_id;
    tabs.next_id += 1;
    let diff = diff_view::new(cx, repo, path);
    let focus = cx.focus_handle();
    let events = cx.subscribe(&diff, move |ws, _, event, cx| match event {
        DiffEvent::Changed => super::refresh_view(&ws.changes, cx),
        DiffEvent::OpenFile { repo, path } => super::host::on_event(
            ws,
            &ChangesEvent::OpenFile {
                repo: repo.clone(),
                path: path.clone(),
            },
            cx,
        ),
        DiffEvent::Notify(note) => {
            super::host::on_event(ws, &ChangesEvent::Notify(note.clone()), cx)
        }
        DiffEvent::Close => close(ws, id, cx),
    });
    tabs.list.push(DiffTab {
        id,
        space: space.clone(),
        focus,
        diff,
        _events: events,
    });
    tabs.selected.insert(space, id);
    ws.refocus = true;
    cx.notify();
}

pub fn select(ws: &mut Workspace, id: u64, cx: &mut Context<Workspace>) {
    let Some(item) = ws.git_diffs.list.iter().find(|tab| tab.id == id) else {
        return;
    };
    ws.git_diffs.selected.insert(item.space.clone(), id);
    diff_view::refresh_entity(&item.diff, cx);
    ws.refocus = true;
    cx.notify();
}

pub fn close(ws: &mut Workspace, id: u64, cx: &mut Context<Workspace>) {
    let Some(position) = ws.git_diffs.list.iter().position(|tab| tab.id == id) else {
        return;
    };
    let space = ws.git_diffs.list.remove(position).space;
    if ws.git_diffs.selected(&space) == Some(id) {
        ws.git_diffs.show_terminals(&space);
        ws.refocus = true;
    }
    cx.notify();
}

pub fn strip(ws: &Workspace, t: &Theme, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let space = ws.spaces.active.as_deref()?;
    if !ws.git_diffs.list.iter().any(|tab| tab.space == space) {
        return None;
    }
    let active = ws.git_diffs.selected(space);
    let tab = tab::tab(t, "git-diff-terminals", "Terminals", None, active.is_none()).on_click(
        cx.listener(|ws, _, _, cx| {
            if let Some(space) = ws.spaces.active.as_deref() {
                ws.git_diffs.show_terminals(space);
                ws.refocus = true;
                cx.notify();
            }
        }),
    );
    let mut strip = tab::tab_bar(t).h(px(38.)).child(tab);
    for item in ws.git_diffs.list.iter().filter(|tab| tab.space == space) {
        let diff = item.diff.read(cx);
        let title = format!("± {}", base_name(&diff.path));
        let tip = diff.path.clone();
        let id = item.id;
        strip = strip.child(
            tab::tab(t, ("git-diff-tab", id), title, None, active == Some(id))
                .tooltip(tooltip(tip))
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |ws, _, _, cx| {
                        cx.stop_propagation();
                        close(ws, id, cx);
                    }),
                )
                .on_click(cx.listener(move |ws, _, _, cx| select(ws, id, cx)))
                .child(
                    icon_button(
                        t,
                        ("git-diff-tab-close", id),
                        Icon::X,
                        IconSize::Small,
                        false,
                    )
                    .tooltip(tooltip("Close diff tab"))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |ws, _, _, cx| close(ws, id, cx))),
                ),
        );
    }
    Some(
        div()
            .id("git-diff-tabs")
            .flex_none()
            .overflow_x_scroll()
            .child(strip)
            .into_any_element(),
    )
}
