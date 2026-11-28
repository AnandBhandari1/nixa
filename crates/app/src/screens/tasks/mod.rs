pub mod api;
mod description;
mod detail;
pub mod drop;
pub mod model;
mod newtask;
pub mod parts;
pub mod place;
mod table;
#[cfg(test)]
mod tests;
mod top;
pub mod view;
mod work;

pub use api::{Call, Host};
pub use view::TasksView;

use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Bounds, Entity, Focusable, Global, TitlebarOptions, WeakEntity,
    WindowBounds, WindowOptions, px, size,
};
use serde_json::Value;

pub const TITLE: &str = "Tasks";
pub const WIDTH: f32 = 1200.0;
pub const HEIGHT: f32 = 800.0;
pub const CHANGED: &str = "tasks_changed";
pub const FLOWS_CHANGED: &str = "workflows_changed";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TasksCommand {
    Open,
}

struct TasksWindow(AnyWindowHandle);

impl Global for TasksWindow {}

#[derive(Default)]
struct Views(Vec<WeakEntity<TasksView>>);

impl Global for Views {}

pub fn commands() -> Vec<(&'static str, &'static str, TasksCommand)> {
    vec![("Tasks", "Open the Tasks window", TasksCommand::Open)]
}

pub fn run(command: TasksCommand, cx: &mut App) {
    match command {
        TasksCommand::Open => open(cx),
    }
}

pub fn new(call: Call, host: Host, cx: &mut App) -> Entity<TasksView> {
    let offset = if cfg!(test) { 0 } else { place::offset_min() };
    let view = cx.new(|cx| TasksView::new(call, host, offset, cx));
    let views = cx.default_global::<Views>();
    views.0.retain(|v| v.upgrade().is_some());
    views.0.push(view.downgrade());
    view
}

fn views(cx: &App) -> Vec<Entity<TasksView>> {
    cx.try_global::<Views>()
        .map(|v| v.0.iter().filter_map(WeakEntity::upgrade).collect())
        .unwrap_or_default()
}

pub fn apply(value: &Value, cx: &mut App) -> usize {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind != CHANGED && kind != FLOWS_CHANGED {
        return 0;
    }
    let list = views(cx);
    for view in &list {
        view.update(cx, |v, cx| {
            if kind == CHANGED {
                v.changed(value, cx);
            } else {
                v.load_options(cx);
            }
        });
    }
    list.len()
}

pub fn open(cx: &mut App) {
    cx.defer(|cx| open_now("", cx));
}

pub fn show(what: &str, cx: &mut App) {
    let start = what.strip_prefix("tasks:").unwrap_or("").to_string();
    cx.defer(move |cx| open_now(&start, cx));
}

fn start_with(
    view: &mut TasksView,
    start: &str,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<TasksView>,
) {
    if start == "new" {
        view.show_new(window, cx);
    } else if let Ok(id) = start.trim_start_matches('#').parse::<i64>() {
        view.open_task(id, cx);
    }
}

fn open_now(start: &str, cx: &mut App) {
    let existing = cx
        .try_global::<TasksWindow>()
        .and_then(|g| g.0.downcast::<TasksView>());
    if let Some(handle) = existing
        && handle
            .update(cx, |view, window, cx| {
                window.activate_window();
                view.reload(cx);
                start_with(view, start, window, cx);
            })
            .is_ok()
    {
        return;
    }
    let call = api::link_call(place::link(cx));
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(720.0), px(420.0))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = new(call, api::host(), cx);
        window.focus(&view.focus_handle(cx));
        view.update(cx, |v, cx| start_with(v, start, window, cx));
        view
    });
    if let Ok(handle) = opened {
        cx.set_global(TasksWindow(handle.into()));
    }
}
