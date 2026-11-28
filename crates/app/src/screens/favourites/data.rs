use super::place;
use super::view::FavouritesEvent;
use crate::workspace::Workspace;
use gpui::{App, Context};
use std::path::{Path, PathBuf};

pub fn active_folder(ws: &Workspace) -> Option<PathBuf> {
    let cwd = ws
        .active_tab()
        .and_then(|tab| ws.terms.get(&tab.focus))
        .map(|term| term.cwd.clone())
        .filter(|cwd| !cwd.trim().is_empty());
    cwd.or_else(|| ws.active_space().map(|s| s.folder.clone()))
        .map(PathBuf::from)
}

pub fn edit(path: &Path, cx: &mut App) {
    crate::screens::editor::open(&path.to_string_lossy(), cx);
}

pub fn apply(ws: &mut Workspace, event: &FavouritesEvent, cx: &mut Context<Workspace>) {
    let result = match event {
        FavouritesEvent::OpenUrl(url) => place::open_outside(url),
        FavouritesEvent::EditFile(path) => {
            edit(path, cx);
            Ok(())
        }
        FavouritesEvent::OpenFile(path) => place::open_outside(&path.display().to_string()),
        FavouritesEvent::Notice { text, error } => {
            let level = if *error { crate::screens::events::Level::Error } else { crate::screens::events::Level::Success };
            crate::screens::events::notify(&ws.hub.events, crate::screens::events::Notice::new(level, format!("Favourites: {text}")), cx);
            Ok(())
        }
        FavouritesEvent::Close => Ok(()),
    };
    if let Err(e) = result {
        ws.notice = Some(format!("Favourites: {e:#}"));
    }
    cx.notify();
}
