pub mod store;
#[cfg(test)]
mod tests;
pub mod view;

use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Bounds, Focusable, Global, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};
use view::LayoutsView;

pub const TITLE: &str = "Layouts";
pub const WIDTH: f32 = 520.0;
pub const HEIGHT: f32 = 520.0;

struct LayoutsWindow(AnyWindowHandle);

impl Global for LayoutsWindow {}

pub fn open(cx: &mut App) {
    cx.defer(open_now);
}

fn open_now(cx: &mut App) {
    let existing = cx
        .try_global::<LayoutsWindow>()
        .and_then(|g| g.0.downcast::<LayoutsView>());
    if let Some(handle) = existing
        && handle
            .update(cx, |view, window, cx| {
                window.activate_window();
                view.reload(cx);
            })
            .is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(400.0), px(300.0))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| view::new(store::folder(), window, cx));
        let input = view.read(cx).input.clone();
        window.focus(&input.focus_handle(cx));
        view
    });
    if let Ok(handle) = opened {
        cx.set_global(LayoutsWindow(handle.into()));
    }
}
