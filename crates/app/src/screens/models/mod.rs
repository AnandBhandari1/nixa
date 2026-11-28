pub mod api;
mod board;
pub mod model;
mod parts;
mod table;
#[cfg(test)]
mod tests;

pub use api::{Call, brain};
pub use board::BoardView;
pub use table::ModelsView;

use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Bounds, Entity, Focusable, Global, TitlebarOptions, WeakEntity,
    WindowBounds, WindowOptions, px, size,
};
use serde_json::Value;

pub const TITLE: &str = "Model leaderboard";
pub const BOARD_TITLE: &str = "Model leaderboard · read only";
pub const WIDTH: f32 = 1400.0;
pub const HEIGHT: f32 = 820.0;
pub const BOARD_WIDTH: f32 = 1500.0;
pub const BOARD_HEIGHT: f32 = 850.0;
pub const CHANGED: &str = "models_changed";
pub const AGENTS_PART: &str = "Agents and team";
pub const SHOW_TABLE: &str = "models";
pub const SHOW_BOARD: &str = "models-view";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelsCommand {
    Open,
    View,
}

struct TableWindow(AnyWindowHandle);

impl Global for TableWindow {}

struct BoardWindow(AnyWindowHandle);

impl Global for BoardWindow {}

#[derive(Default)]
struct Views {
    tables: Vec<WeakEntity<ModelsView>>,
    boards: Vec<WeakEntity<BoardView>>,
}

impl Global for Views {}

pub fn commands() -> Vec<(&'static str, &'static str, ModelsCommand)> {
    vec![
        (
            "Model leaderboard",
            "Rank, pin, take out and edit the models Nixa picks",
            ModelsCommand::Open,
        ),
        (
            "Model leaderboard: read only",
            "See the model table and what Nixa would pick, without changing it",
            ModelsCommand::View,
        ),
    ]
}

pub fn run(command: ModelsCommand, cx: &mut App) {
    match command {
        ModelsCommand::Open => open(cx),
        ModelsCommand::View => open_board(cx),
    }
}

pub fn show(what: &str, cx: &mut App) -> bool {
    match what {
        SHOW_TABLE => open(cx),
        SHOW_BOARD => open_board(cx),
        _ => return false,
    }
    true
}

pub fn new_table(call: Call, cx: &mut App) -> Entity<ModelsView> {
    let view = cx.new(|cx| ModelsView::new(call, cx));
    let views = cx.default_global::<Views>();
    views.tables.retain(|v| v.upgrade().is_some());
    views.tables.push(view.downgrade());
    view
}

pub fn new_board(call: Call, cx: &mut App) -> Entity<BoardView> {
    let view = cx.new(|cx| BoardView::new(call, cx));
    let views = cx.default_global::<Views>();
    views.boards.retain(|v| v.upgrade().is_some());
    views.boards.push(view.downgrade());
    view
}

pub fn refresh(cx: &mut App) -> usize {
    let (tables, boards): (Vec<Entity<ModelsView>>, Vec<Entity<BoardView>>) = cx
        .try_global::<Views>()
        .map(|v| {
            (
                v.tables.iter().filter_map(WeakEntity::upgrade).collect(),
                v.boards.iter().filter_map(WeakEntity::upgrade).collect(),
            )
        })
        .unwrap_or_default();
    for view in &tables {
        view.update(cx, |v, cx| {
            if v.idle() {
                v.load(cx);
            }
        });
    }
    for view in &boards {
        view.update(cx, |v, cx| {
            if v.idle() {
                v.load(cx);
            }
        });
    }
    tables.len() + boards.len()
}

pub fn apply(value: &Value, cx: &mut App) -> bool {
    if value.get("type").and_then(Value::as_str) != Some(CHANGED) {
        return false;
    }
    refresh(cx);
    true
}

fn options(title: &'static str, width: f32, height: f32, cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(640.0), px(400.0))),
        ..WindowOptions::default()
    }
}

pub fn open(cx: &mut App) {
    cx.defer(open_table_now);
}

pub fn open_board(cx: &mut App) {
    cx.defer(open_board_now);
}

fn open_table_now(cx: &mut App) {
    let existing = cx
        .try_global::<TableWindow>()
        .and_then(|g| g.0.downcast::<ModelsView>());
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
    let opened = cx.open_window(options(TITLE, WIDTH, HEIGHT, cx), |window, cx| {
        let view = new_table(brain(), cx);
        window.focus(&view.focus_handle(cx));
        view
    });
    if let Ok(handle) = opened {
        cx.set_global(TableWindow(handle.into()));
    }
}

fn open_board_now(cx: &mut App) {
    let existing = cx
        .try_global::<BoardWindow>()
        .and_then(|g| g.0.downcast::<BoardView>());
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
    let opened = cx.open_window(
        options(BOARD_TITLE, BOARD_WIDTH, BOARD_HEIGHT, cx),
        |window, cx| {
            let view = new_board(brain(), cx);
            window.focus(&view.focus_handle(cx));
            view
        },
    );
    if let Ok(handle) = opened {
        cx.set_global(BoardWindow(handle.into()));
    }
}
