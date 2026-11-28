#![allow(dead_code)]

pub mod menu;
pub mod model;
#[cfg(test)]
mod tests;
pub mod view;
pub mod voice;

pub use menu::AgentMenu;
pub use model::{Deps, Request};
pub use view::{AiResultView, Mode};

use gpui::prelude::*;
use gpui::{App, Bounds, Entity, Focusable, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use std::path::PathBuf;

pub const WIDTH: f32 = 680.0;
pub const HEIGHT: f32 = 620.0;
pub const SETTINGS_PART: &str = "Git accounts";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitAiCommand {
    ReviewStaged,
    ReviewLastCommit,
    ExplainLastCommit,
}

pub fn commands() -> Vec<(&'static str, &'static str, GitAiCommand)> {
    vec![
        ("Git: AI review of staged changes", "Ask the AI agent to review what is staged", GitAiCommand::ReviewStaged),
        ("Git: AI review of the last commit", "Ask the AI agent to review the last commit", GitAiCommand::ReviewLastCommit),
        ("Git: AI explains the last commit", "Ask the AI agent to explain the last commit", GitAiCommand::ExplainLastCommit),
    ]
}

pub fn request_for(command: GitAiCommand, repo: PathBuf) -> Request {
    match command {
        GitAiCommand::ReviewStaged => Request::review_staged(repo),
        GitAiCommand::ReviewLastCommit => Request::review_commit(repo, "HEAD"),
        GitAiCommand::ExplainLastCommit => Request::explain(repo, "HEAD"),
    }
}

pub fn run(command: GitAiCommand, repo: Option<PathBuf>, cx: &mut App) {
    let request = match repo {
        Some(repo) => request_for(command, repo),
        None => Request {
            kind: request_for(command, PathBuf::new()).kind,
            ..Request::default()
        },
    };
    open(request, cx);
}

pub fn new(mode: Mode, request: Request, deps: Deps, cx: &mut App) -> Entity<AiResultView> {
    cx.new(|cx| AiResultView::new(mode, request, deps, cx))
}

pub fn new_menu(deps: Deps, cx: &mut App) -> Entity<AgentMenu> {
    cx.new(|cx| AgentMenu::new(deps, cx))
}

pub fn open_settings(cx: &mut App) {
    cx.defer(|cx| crate::screens::settings::open(SETTINGS_PART, cx));
}

pub fn show(spec: &str, cx: &mut App) -> Result<(), String> {
    let request = model::parse_show(spec)
        .ok_or_else(|| format!("{spec} is not review[@sha]:<repo>, explain[@sha]:<repo> or conflict@<file>:<repo>"))?;
    open(request, cx);
    Ok(())
}

pub fn open(request: Request, cx: &mut App) {
    cx.defer(move |cx| open_now(request, cx));
}

fn open_now(request: Request, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(model::heading(request.kind).into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(460.0), px(360.0))),
        ..WindowOptions::default()
    };
    cx.open_window(options, |window, cx| {
        let view = new(Mode::Window, request, model::real(), cx);
        window.focus(&view.focus_handle(cx));
        view
    })
    .ok();
}
