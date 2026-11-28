use super::{ChangesEvent, ChangesView};
use crate::workspace::Workspace;
use gpui::{AnyElement, Context, Entity, Subscription};

pub fn new(cx: &mut Context<Workspace>) -> (Entity<ChangesView>, Subscription) {
    let view = super::new(cx);
    let sub = cx.subscribe(&view, |ws, _, event: &ChangesEvent, cx| on_event(ws, event, cx));
    (view, sub)
}

pub fn sync(ws: &Workspace, cx: &mut Context<Workspace>) {
    let folder = ws.active_space().map(|s| s.folder.clone()).filter(|f| !f.is_empty());
    let view = ws.changes.clone();
    let same = view.read(cx).folder == folder;
    if !same {
        super::set_place(&view, folder.clone(), folder, cx);
    }
}

pub fn on_event(ws: &mut Workspace, event: &ChangesEvent, cx: &mut Context<Workspace>) {
    match event {
        ChangesEvent::Notify(n) => {
            ws.notice = Some(if n.body.is_empty() {
                n.title.clone()
            } else {
                format!("{}: {}", n.title, n.body.replace('\n', " "))
            });
            crate::screens::git::announce::show_note(&ws.hub.events, ws.hub.link.clone(), n, cx);
            cx.notify();
        }
        ChangesEvent::OpenFile { repo, path } => {
            let full = super::menus::full_path(repo, path);
            crate::screens::editor::open(&full, cx);
        }
        ChangesEvent::Open(window) => crate::screens::git::join::open_window(window, cx),
        ChangesEvent::Suggest { repo, branch } => {
            crate::screens::git::join::suggest(ws.changes.clone(), repo.clone(), branch.clone(), ws.hub.link.clone(), cx);
        }
        ChangesEvent::StopSuggest => {
            let view = ws.changes.clone();
            crate::screens::git::join::stop_suggest(&view, cx);
        }
    }
}

pub fn render(ws: &Workspace) -> AnyElement {
    super::render(&ws.changes)
}
