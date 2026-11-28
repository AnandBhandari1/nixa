mod api;
mod draw;
pub mod flow;
mod forms;
pub mod geo;
mod gestures;
pub mod layout;
#[cfg(test)]
mod tests;
mod view;

pub use api::{Call, brain, set_link};
pub use view::WorkflowsView;

use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Bounds, Entity, Focusable, Global, TitlebarOptions, WeakEntity,
    WindowBounds, WindowOptions, px, size,
};
use serde_json::Value;

pub const TITLE: &str = "Workflows";
pub const WIDTH: f32 = 1200.0;
pub const HEIGHT: f32 = 800.0;
pub const CHANGED: &str = "workflows_changed";
pub const RUNS_CHANGED: &str = "workflow_runs_changed";
pub const HELLO: &str = "hello";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowsCommand {
    Open,
}

struct WorkflowsWindow(AnyWindowHandle);

impl Global for WorkflowsWindow {}

#[derive(Default)]
struct Views(Vec<WeakEntity<WorkflowsView>>);

impl Global for Views {}

pub fn commands() -> Vec<(&'static str, &'static str, WorkflowsCommand)> {
    vec![(
        "Workflows",
        "Open the Workflows window",
        WorkflowsCommand::Open,
    )]
}

pub fn run(command: WorkflowsCommand, cx: &mut App) {
    match command {
        WorkflowsCommand::Open => open(cx),
    }
}

pub fn new(call: Call, cx: &mut App) -> Entity<WorkflowsView> {
    let view = cx.new(|cx| WorkflowsView::new(call, cx));
    let views = cx.default_global::<Views>();
    views.0.retain(|v| v.upgrade().is_some());
    views.0.push(view.downgrade());
    view
}

fn live(cx: &App) -> Vec<Entity<WorkflowsView>> {
    cx.try_global::<Views>()
        .map(|v| v.0.iter().filter_map(WeakEntity::upgrade).collect())
        .unwrap_or_default()
}

pub fn apply(value: &Value, cx: &mut App) -> bool {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if ![CHANGED, RUNS_CHANGED, HELLO].contains(&kind) {
        return false;
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let views = live(cx);
    for view in &views {
        view.update(cx, |v, cx| match kind {
            CHANGED => v.changed(name.as_deref(), cx),
            RUNS_CHANGED => v.load_runs(cx),
            _ => {
                v.booted = false;
                v.boot(cx);
            }
        });
    }
    !views.is_empty()
}

pub fn open(cx: &mut App) {
    cx.defer(open_now);
}

pub fn show(what: &str, cx: &mut App) {
    let start = what
        .strip_prefix("workflows")
        .unwrap_or(what)
        .trim_start_matches(':')
        .to_string();
    cx.defer(move |cx| {
        open_now(cx);
        if start.is_empty() {
            return;
        }
        for view in live(cx) {
            view.update(cx, |v, cx| v.start_with(&start, cx));
        }
    });
}

fn open_now(cx: &mut App) {
    let existing = cx
        .try_global::<WorkflowsWindow>()
        .and_then(|g| g.0.downcast::<WorkflowsView>());
    if let Some(handle) = existing
        && handle
            .update(cx, |view, window, cx| {
                window.activate_window();
                if !view.booted {
                    view.boot(cx);
                }
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
        window_min_size: Some(size(px(720.0), px(460.0))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = new(brain(), cx);
        window.focus(&view.focus_handle(cx));
        view
    });
    if let Ok(handle) = opened {
        cx.set_global(WorkflowsWindow(handle.into()));
    }
}
