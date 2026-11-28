mod api;
pub mod model;
#[cfg(test)]
mod tests;
mod view;

pub use api::{Call, Host, brain, host};
pub use view::AssistantView;

use gpui::prelude::*;
use gpui::{AnyWindowHandle, App, Bounds, Entity, Focusable, Global, TitlebarOptions, WindowBounds, WindowOptions, px, size};

pub const TITLE: &str = "Personal assistant";
pub const WIDTH: f32 = 560.0;
pub const HEIGHT: f32 = 620.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantCommand {
    Open,
    Briefing,
}

struct AssistantWindow(AnyWindowHandle);

impl Global for AssistantWindow {}

pub fn commands() -> Vec<(&'static str, &'static str, AssistantCommand)> {
    vec![
        ("Personal assistant", "Ask quick questions, set reminders and the morning briefing", AssistantCommand::Open),
        ("Set up morning briefing", "Open the personal assistant at the morning briefing", AssistantCommand::Briefing),
    ]
}

pub fn run(command: AssistantCommand, cx: &mut App) {
    match command {
        AssistantCommand::Open => open(cx),
        AssistantCommand::Briefing => show("assistant:briefing", cx),
    }
}

pub fn new(call: Call, host: Host, cx: &mut App) -> Entity<AssistantView> {
    cx.new(|cx| AssistantView::new(call, host, cx))
}

pub fn open(cx: &mut App) {
    cx.defer(|cx| open_now("", cx));
}

pub fn show(what: &str, cx: &mut App) {
    let start = what.strip_prefix("assistant").unwrap_or(what).trim_start_matches(':').to_string();
    cx.defer(move |cx| open_now(&start, cx));
}

fn open_now(start: &str, cx: &mut App) {
    let existing = cx.try_global::<AssistantWindow>().and_then(|g| g.0.downcast::<AssistantView>());
    if let Some(handle) = existing
        && handle
            .update(cx, |view, window, cx| {
                window.activate_window();
                view.load(cx);
                view.start_with(start, cx);
            })
            .is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some(TITLE.into()), appears_transparent: false, traffic_light_position: None }),
        window_min_size: Some(size(px(420.0), px(360.0))),
        ..WindowOptions::default()
    };
    let start = start.to_string();
    let opened = cx.open_window(options, move |window, cx| {
        let view = new(brain(), host(), cx);
        view.update(cx, |v, cx| v.start_with(&start, cx));
        window.focus(&view.focus_handle(cx));
        view
    });
    if let Ok(handle) = opened {
        cx.set_global(AssistantWindow(handle.into()));
    }
}
