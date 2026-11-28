pub mod branch_ops;
pub mod branches;
pub mod commit_menu;
pub mod commit_ops;
pub mod graph;
pub mod links;
pub mod log;
pub mod repo;
pub mod run;
pub mod stash;
pub mod stash_ops;
#[cfg(test)]
mod tests;
pub mod widgets;
pub mod words;

use gpui::prelude::*;
use gpui::{AnyWindowHandle, App, Bounds, Entity, Focusable, Global, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions, px, size};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const GRAPH_TITLE: &str = "History";
pub const GRAPH_WIDTH: f32 = 900.0;
pub const GRAPH_HEIGHT: f32 = 620.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryCommand {
    Graph,
    Branches,
    Stashes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Then {
    Nothing,
    Commit(String),
    File(String),
}

#[derive(Default)]
struct OpenWindows(HashMap<&'static str, AnyWindowHandle>);

impl Global for OpenWindows {}

#[derive(Default)]
struct Graphs(Vec<WeakEntity<graph::GraphView>>);

impl Global for Graphs {}

struct GraphScope(bool);

impl Global for GraphScope {}

pub fn graph_all(cx: &App) -> bool {
    cx.try_global::<GraphScope>().is_none_or(|g| g.0)
}

pub fn set_graph_all(all: bool, cx: &mut App) {
    cx.set_global(GraphScope(all));
}

pub fn commands() -> Vec<(&'static str, &'static str, HistoryCommand)> {
    vec![
        ("Git history", "Show the commit graph of this repo", HistoryCommand::Graph),
        ("Git branches", "Switch, make, merge and delete branches", HistoryCommand::Branches),
        ("Git stashes", "See, apply and drop stashes", HistoryCommand::Stashes),
    ]
}

pub fn repo_root(folder: Option<&str>) -> Option<PathBuf> {
    let folder = folder.filter(|f| !f.is_empty())?;
    let out = run::read(Path::new(folder), &["rev-parse", "--show-toplevel"]);
    if !out.ok() {
        return None;
    }
    let top = out.line();
    (!top.is_empty()).then(|| PathBuf::from(top))
}

pub fn run(command: HistoryCommand, folder: Option<&str>, cx: &mut App) {
    let repo = repo_root(folder);
    match command {
        HistoryCommand::Graph => open_graph(repo, Then::Nothing, cx),
        HistoryCommand::Branches => branches::open(repo, cx),
        HistoryCommand::Stashes => stash::open(repo, cx),
    }
}

fn form_of(name: &str) -> Option<commit_menu::Form> {
    use commit_menu::Form;
    Some(match name {
        "checkout" => Form::Checkout,
        "branch" => Form::Branch,
        "tag" => Form::Tag,
        "pick" => Form::Pick,
        "revert" => Form::Revert,
        "reset" => Form::Reset,
        "undo" => Form::Undo,
        "reword" => Form::Reword,
        _ => return None,
    })
}

pub fn show(what: &str, folder: Option<&str>, cx: &mut App) -> bool {
    let repo = repo_root(folder);
    let (head, rest) = what.split_once(':').unwrap_or((what, ""));
    match head {
        "git-history" => {
            let then = match rest.split_once(':') {
                Some(("commit", sha)) => Then::Commit(sha.to_string()),
                Some(("file", path)) => Then::File(path.to_string()),
                _ => Then::Nothing,
            };
            open_graph(repo, then, cx);
        }
        "git-branches" => branches::open(repo, cx),
        "git-stash" => stash::open(repo, cx),
        "git-commit" => {
            let Some(root) = repo else { return false };
            let (rev, form) = match rest.split_once(':') {
                Some((form, rev)) => (rev.to_string(), form_of(form)),
                None if form_of(rest).is_some() => ("HEAD".to_string(), form_of(rest)),
                None if !rest.is_empty() => (rest.to_string(), None),
                None => ("HEAD".to_string(), None),
            };
            let Some(sha) = repo::commit_of(&root, &rev) else { return false };
            let line = run::read(&root, &["log", "-1", "--format=%h%x1f%s%x1f%an", &sha]).line();
            let mut parts = line.split('\u{1f}');
            let mut props = commit_menu::Props {
                repo: root.clone(),
                sha,
                short: parts.next().unwrap_or_default().to_string(),
                subject: parts.next().unwrap_or_default().to_string(),
                author: parts.next().unwrap_or_default().to_string(),
                form: None,
            };
            props.form = form;
            commit_menu::open(props, cx);
        }
        _ => return false,
    }
    true
}

fn toast(title: &str, body: &str, level: links::Level, cx: &mut App) {
    use crate::screens::events;
    let level = match level {
        links::Level::Info => events::Level::Info,
        links::Level::Success => events::Level::Success,
        links::Level::Warning => events::Level::Warning,
    };
    super::announce::post(title, body, level, cx);
}

pub fn init(cx: &mut App) {
    links::set_links(links::Links { notify: toast, ..links::Links::default() }, cx);
}

pub fn new_graph(repo: Option<PathBuf>, cx: &mut App) -> Entity<graph::GraphView> {
    let view = cx.new(|cx| graph::GraphView::new(repo, cx));
    let graphs = cx.default_global::<Graphs>();
    graphs.0.retain(|g| g.upgrade().is_some());
    graphs.0.push(view.downgrade());
    view
}

fn with_graph(view: &mut graph::GraphView, then: &Then, cx: &mut gpui::Context<graph::GraphView>) {
    match then {
        Then::Nothing => {}
        Then::Commit(sha) => view.show_commit(sha, cx),
        Then::File(path) => view.show_file_history(path, cx),
    }
}

pub fn open_graph(repo: Option<PathBuf>, then: Then, cx: &mut App) {
    cx.defer(move |cx| {
        let old = cx.try_global::<OpenWindows>().and_then(|w| w.0.get(GRAPH_TITLE).copied()).and_then(|h| h.downcast::<graph::GraphView>());
        if let Some(handle) = old
            && handle
                .update(cx, |v, window, cx| {
                    window.activate_window();
                    v.set_repo(repo.clone(), cx);
                    with_graph(v, &then, cx);
                })
                .is_ok()
        {
            return;
        }
        open_window(GRAPH_TITLE, GRAPH_WIDTH, GRAPH_HEIGHT, cx, move |_, cx| {
            let view = new_graph(repo, cx);
            view.update(cx, |v, cx| with_graph(v, &then, cx));
            view
        })
    });
}

fn min_size(title: &str) -> (f32, f32) {
    match title {
        "Commit" => (280.0, 360.0),
        "Branches" => (300.0, 320.0),
        "Stashes" => (560.0, 400.0),
        _ => (480.0, 300.0),
    }
}

pub fn open_window<V: Render + Focusable>(
    title: &'static str,
    width: f32,
    height: f32,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V> + 'static,
) {
    let old = cx.try_global::<OpenWindows>().and_then(|w| w.0.get(title).copied());
    if let Some(handle) = old {
        handle.update(cx, |_, window, _| window.remove_window()).ok();
    }
    let (min_w, min_h) = min_size(title);
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some(title.into()), appears_transparent: false, traffic_light_position: None }),
        window_min_size: Some(size(px(min_w), px(min_h))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = build(window, cx);
        window.focus(&view.focus_handle(cx));
        view
    });
    if let Ok(handle) = opened {
        cx.default_global::<OpenWindows>().0.insert(title, handle.into());
    }
}

pub fn refresh_all(repo: &Path, cx: &mut App) {
    let repo = repo.to_path_buf();
    cx.defer(move |cx| {
        let same = |r: &Option<PathBuf>| r.as_deref() == Some(repo.as_path());
        let graphs: Vec<Entity<graph::GraphView>> = cx.try_global::<Graphs>().map(|g| g.0.iter().filter_map(WeakEntity::upgrade).collect()).unwrap_or_default();
        for g in graphs {
            g.update(cx, |v, cx| {
                if same(&v.repo) {
                    v.load(true, cx);
                }
            });
        }
        for w in cx.windows() {
            if let Some(h) = w.downcast::<branches::BranchesView>() {
                h.update(cx, |v, _, cx| {
                    if same(&v.repo) && !v.closed {
                        v.load(cx);
                    }
                })
                .ok();
            }
            if let Some(h) = w.downcast::<stash::StashView>() {
                h.update(cx, |v, _, cx| {
                    if same(&v.repo) && !v.closed && v.busy.is_none() {
                        v.load(true, cx);
                    }
                })
                .ok();
            }
        }
    });
}
