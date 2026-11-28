use gpui::{
    AnyWindowHandle, App, Bounds, Context, Entity, Focusable, Global, Render, TitlebarOptions,
    Window, WindowBounds, WindowOptions, px, size,
};
use std::any::TypeId;
use std::collections::HashMap;

pub struct Size {
    pub width: f32,
    pub height: f32,
    pub title: &'static str,
}

#[derive(Default)]
struct Windows(HashMap<TypeId, AnyWindowHandle>);

impl Global for Windows {}

pub fn open_or_reuse<V: Render + Focusable + 'static>(
    size_of: Size,
    cx: &mut App,
    reuse: impl FnOnce(&mut V, &mut Window, &mut Context<V>),
    make: impl FnOnce(&mut Window, &mut App) -> Entity<V> + 'static,
) {
    let key = TypeId::of::<V>();
    let existing = cx
        .try_global::<Windows>()
        .and_then(|w| w.0.get(&key).copied())
        .and_then(|h| h.downcast::<V>());
    if let Some(handle) = existing
        && handle
            .update(cx, |view, window, cx| {
                window.activate_window();
                reuse(view, window, cx);
            })
            .is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(size_of.width), px(size_of.height)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(size_of.title.into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(520.0), px(360.0))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = make(window, cx);
        window.focus(&view.focus_handle(cx));
        view
    });
    if let Ok(handle) = opened {
        cx.default_global::<Windows>().0.insert(key, handle.into());
    }
}
