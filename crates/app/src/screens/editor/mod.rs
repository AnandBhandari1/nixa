#[cfg(test)]
mod tests;

use gpui::prelude::*;
use gpui::{App, Bounds, Focusable, Global, TitlebarOptions, WindowBounds, WindowHandle, WindowId, WindowOptions, px, size};
use nixa_editor::{Editor, EditorEvent, Options};
use std::collections::HashMap;
use std::path::Path;

pub const WIDTH: f32 = 1000.0;
pub const HEIGHT: f32 = 700.0;
pub const SHOW: &str = "editor:";

#[derive(Default)]
struct OpenEditors(HashMap<String, Vec<WindowHandle<Editor>>>);

impl Global for OpenEditors {}

pub fn init(cx: &mut App) {
    nixa_editor::bind_keys(cx);
    cx.on_action(|_: &crate::actions::OpenCommands, cx| {
        let Some(editor) = cx.active_window().and_then(|window| window.downcast::<Editor>()) else {
            return;
        };
        crate::commands::target_editor(editor, cx);
        cx.defer(|cx| {
            if let Some(workspace) = cx.windows().into_iter().find_map(|window| window.downcast::<crate::workspace::Workspace>()) {
                workspace.update(cx, |workspace, window, cx| {
                    window.activate_window();
                    workspace.open_palette(window, cx);
                }).ok();
            }
        });
    });
}

pub fn title_of(path: &str) -> String {
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if name.is_empty() { "Editor".to_string() } else { name }
}

pub fn split_show(rest: &str) -> (String, Option<String>) {
    match rest.split_once('|') {
        Some((path, then)) => (path.to_string(), Some(then.to_string()).filter(|t| !t.is_empty())),
        None => (rest.to_string(), None),
    }
}

pub fn show(what: &str, cx: &mut App) -> bool {
    let Some(rest) = what.strip_prefix(SHOW) else { return false };
    let (path, then) = split_show(rest);
    if path.is_empty() {
        return false;
    }
    let readonly = then.as_deref() == Some("readonly");
    let then = then.filter(|value| {
        value != "readonly" && (cfg!(test) || !matches!(value.as_str(), "save" | "close" | "conflict"))
    });
    open_with(path, then, readonly, cx);
    true
}

pub fn open(path: &str, cx: &mut App) {
    open_with(path.to_string(), None, false, cx);
}

pub fn open_at(repo: &Path, file: &str, cx: &mut App) -> bool {
    let full = if Path::new(file).is_absolute() { file.to_string() } else { repo.join(file).to_string_lossy().to_string() };
    open(&full, cx);
    true
}

fn drop_window(open: &mut HashMap<String, Vec<WindowHandle<Editor>>>, id: WindowId) {
    for list in open.values_mut() {
        list.retain(|h| h.window_id() != id);
    }
    open.retain(|_, list| !list.is_empty());
}

fn forget_window(id: WindowId, cx: &mut App) {
    if cx.has_global::<OpenEditors>() {
        drop_window(&mut cx.global_mut::<OpenEditors>().0, id);
    }
}

fn rekey(id: WindowId, path: &str, cx: &mut App) {
    if !cx.has_global::<OpenEditors>() {
        return;
    }
    let open = &mut cx.global_mut::<OpenEditors>().0;
    let Some(handle) = open.values().flatten().find(|h| h.window_id() == id).copied() else { return };
    drop_window(open, id);
    open.entry(path.to_string()).or_default().push(handle);
}

fn normalized(path: &str) -> String {
    let expanded = if let (Some(rest), Some(home)) = (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        Path::new(&home).join(rest)
    } else {
        Path::new(path).to_path_buf()
    };
    expanded.canonicalize().unwrap_or(expanded).to_string_lossy().into_owned()
}

fn open_with(path: String, then: Option<String>, readonly: bool, cx: &mut App) {
    let path = normalized(&path);
    cx.defer(move |cx| {
        let key = if readonly { format!("readonly:{path}") } else { path.clone() };
        let found = cx.try_global::<OpenEditors>().and_then(|o| o.0.get(&key).cloned()).unwrap_or_default();
        for handle in found {
            let shown = handle.update(cx, |ed, window, cx| {
                window.activate_window();
                if let Some(what) = &then {
                    nixa_editor::view::show(ed, what, window, cx);
                }
            });
            if shown.is_ok() {
                return;
            }
            forget_window(handle.window_id(), cx);
        }
        let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(title_of(&path).into()), appears_transparent: false, traffic_light_position: None }),
            window_min_size: Some(size(px(420.0), px(300.0))),
            ..WindowOptions::default()
        };
        let opened = cx.open_window(options, move |window, cx| {
            let view = cx.new(|cx| Editor::new(Some(&path), Options { readonly }, window, cx));
            let closing = view.clone();
            let id = window.window_handle().window_id();
            window.on_window_should_close(cx, move |_, cx| {
                let close = closing.update(cx, |ed, cx| ed.request_close(cx));
                if close {
                    forget_window(id, cx);
                }
                close
            });
            window
                .subscribe(&view, cx, move |_, event: &EditorEvent, window, cx| match event {
                    EditorEvent::Close => {
                        forget_window(id, cx);
                        window.remove_window();
                    }
                    EditorEvent::Opened(p) => {
                        let path = normalized(p);
                        let key = if readonly { format!("readonly:{path}") } else { path };
                        rekey(id, &key, cx);
                        window.set_window_title(&title_of(p));
                    }
                    EditorEvent::Saved(p) => window.set_window_title(&title_of(p)),
                    EditorEvent::Edited => {}
                })
                .detach();
            if let Some(what) = then {
                view.update(cx, |ed, cx| nixa_editor::view::show(ed, &what, window, cx));
            }
            window.focus(&view.focus_handle(cx));
            view
        });
        if let Ok(handle) = opened {
            cx.default_global::<OpenEditors>().0.entry(key).or_default().push(handle);
        }
    });
}
