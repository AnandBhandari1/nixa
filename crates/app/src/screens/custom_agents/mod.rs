mod api;
pub mod model;
mod place;
#[cfg(test)]
mod tests;
mod view;

pub use api::{Call, brain};
pub use view::{CustomAgentsView, Mode};

use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Bounds, Entity, Focusable, Global, TitlebarOptions, WeakEntity,
    WindowBounds, WindowOptions, px, size,
};
use serde_json::Value;

pub const TITLE: &str = "Custom agents";
pub const WIDTH: f32 = 1000.0;
pub const HEIGHT: f32 = 700.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomAgentsCommand {
    Open,
}

pub const CHANGED: &str = "custom_agents_changed";
pub const SHOW: &str = "custom_agents_show";

struct CustomAgentsWindow(AnyWindowHandle);

impl Global for CustomAgentsWindow {}

#[derive(Default)]
struct Views(Vec<WeakEntity<CustomAgentsView>>);

impl Global for Views {}

pub fn commands() -> Vec<(&'static str, &'static str, CustomAgentsCommand)> {
    vec![(
        "Custom agents",
        "Open the Custom agents window",
        CustomAgentsCommand::Open,
    )]
}

pub fn run(command: CustomAgentsCommand, cx: &mut App) {
    match command {
        CustomAgentsCommand::Open => open(cx),
    }
}

pub fn new(mode: Mode, call: Call, cx: &mut App) -> Entity<CustomAgentsView> {
    let view = cx.new(|cx| CustomAgentsView::new(mode, call, cx));
    let weak = view.downgrade();
    let views = cx.default_global::<Views>();
    views.0.retain(|v| v.upgrade().is_some());
    views.0.push(weak);
    view
}

pub fn refresh(cx: &mut App) -> usize {
    let views: Vec<Entity<CustomAgentsView>> = cx
        .try_global::<Views>()
        .map(|v| v.0.iter().filter_map(WeakEntity::upgrade).collect())
        .unwrap_or_default();
    for view in &views {
        view.update(cx, |v, cx| {
            if v.idle() {
                v.load(cx);
            }
        });
    }
    views.len()
}

pub fn apply(value: &Value, cx: &mut App) -> Option<String> {
    match value.get("type").and_then(Value::as_str)? {
        CHANGED => {
            refresh(cx);
            None
        }
        SHOW => value
            .get("terminal")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

pub fn open(cx: &mut App) {
    cx.defer(open_now);
}

fn open_now(cx: &mut App) {
    let existing = cx
        .try_global::<CustomAgentsWindow>()
        .and_then(|g| g.0.downcast::<CustomAgentsView>());
    if let Some(handle) = existing
        && handle
            .update(cx, |view, window, cx| {
                window.activate_window();
                view.load(cx);
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
        window_min_size: Some(size(px(560.0), px(380.0))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = new(Mode::Window, brain(), cx);
        window.focus(&view.focus_handle(cx));
        view
    });
    if let Ok(handle) = opened {
        cx.set_global(CustomAgentsWindow(handle.into()));
    }
}
