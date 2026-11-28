pub mod actions;
pub mod git;
pub mod host;
pub mod preview;
pub mod search;
pub mod tone;
pub mod tree;
mod view;
pub mod watch;

#[cfg(test)]
mod tests;

pub use actions::{FileAction, Target};
pub use preview::{Plan, preview_argv};

use gpui::{
    AnyElement, App, AppContext, ClipboardItem, Context, Entity, EventEmitter, FocusHandle,
    Focusable, IntoElement, Pixels, Point, Subscription, UniformListScrollHandle, Window,
};
use nixa_ui::Icon;
use nixa_ui::text_input::{InputEvent, TextInput};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const WATCH_EVERY: Duration = Duration::from_secs(1);
pub const REFRESH_TICKS: u32 = 10;
pub const SEARCH_WAIT: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilesEvent {
    Open {
        file: String,
        keep: bool,
        replace: Option<String>,
    },
    Select(String),
    Terminal {
        file: String,
        keep: bool,
        replace: Option<String>,
    },
    Run {
        name: String,
        folder: String,
        argv: Vec<String>,
    },
    Edit {
        file: String,
    },
    Notify {
        title: String,
        body: Option<String>,
        error: bool,
    },
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Refresh,
    Search,
    Collapse,
}

pub struct Menu {
    pub target: Target,
    pub at: Point<Pixels>,
    pub hi: Option<usize>,
}

pub struct Prompt {
    pub action: FileAction,
    pub target: Target,
    pub input: Entity<TextInput>,
    pub error: Option<String>,
    _events: Subscription,
}

pub struct Confirm {
    pub target: Target,
    pub error: Option<String>,
}

pub struct FilesView {
    pub root: Option<String>,
    pub tree: tree::Tree,
    pub rows: Vec<tree::Row>,
    pub marks: HashMap<String, git::Mark>,
    pub index: Option<PathBuf>,
    pub stamps: watch::Stamps,
    pub saved: HashMap<String, BTreeSet<String>>,
    pub loading: bool,
    pub again: bool,
    pub loaded: bool,
    pub load_seq: u64,
    pub error: Option<String>,
    pub search: Entity<TextInput>,
    pub query: String,
    pub found: search::Found,
    pub searching: bool,
    pub search_error: Option<String>,
    pub search_seq: u64,
    pub menu: Option<Menu>,
    pub prompt: Option<Prompt>,
    pub confirm: Option<Confirm>,
    pub previews: preview::Previews,
    pub tabs: Option<HashSet<String>>,
    pub busy: bool,
    pub home: PathBuf,
    pub ticks: u32,
    pub scroll: UniformListScrollHandle,
    pub focus: FocusHandle,
    _search_events: Subscription,
}

impl EventEmitter<FilesEvent> for FilesView {}

impl Focusable for FilesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn build(cx: &mut Context<FilesView>) -> FilesView {
    let search =
        cx.new(|cx| TextInput::new(cx, "Search names and contents").with_icon(Icon::Search));
    let events = cx.subscribe(
        &search,
        |v: &mut FilesView, _, event: &InputEvent, cx| match event {
            InputEvent::Changed(text) => typed(v, text.clone(), cx),
            InputEvent::Submit(text) => {
                v.query = text.clone();
                start_search(v, cx);
            }
            InputEvent::Cancel => clear_search(v, cx),
        },
    );
    FilesView {
        root: None,
        tree: tree::Tree::default(),
        rows: Vec::new(),
        marks: HashMap::new(),
        index: None,
        stamps: watch::Stamps::new(),
        saved: HashMap::new(),
        loading: false,
        again: false,
        loaded: false,
        load_seq: 0,
        error: None,
        search,
        query: String::new(),
        found: search::Found::default(),
        searching: false,
        search_error: None,
        search_seq: 0,
        menu: None,
        prompt: None,
        confirm: None,
        previews: preview::Previews::default(),
        tabs: None,
        busy: false,
        home: home(),
        ticks: 0,
        scroll: UniformListScrollHandle::new(),
        focus: cx.focus_handle(),
        _search_events: events,
    }
}

pub fn new(cx: &mut App) -> Entity<FilesView> {
    let view = cx.new(build);
    if !cfg!(test) {
        start_watch(&view, cx);
    }
    view
}

pub fn render(view: &Entity<FilesView>) -> AnyElement {
    view.clone().into_any_element()
}

pub fn start_watch(view: &Entity<FilesView>, cx: &mut App) {
    let weak = view.downgrade();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(WATCH_EVERY).await;
            let Some(view) = weak.upgrade() else { break };
            if view.update(cx, tick).is_err() {
                break;
            }
        }
    })
    .detach();
}

pub fn tick(v: &mut FilesView, cx: &mut Context<FilesView>) {
    v.ticks = v.ticks.wrapping_add(1);
    let Some(root) = v.root.clone() else { return };
    if v.loading {
        return;
    }
    if v.ticks.is_multiple_of(REFRESH_TICKS) {
        refresh(v, cx);
        return;
    }
    let folders = tree::folders(&v.tree);
    let index = v.index.clone();
    let seq = v.load_seq;
    let job = cx
        .background_executor()
        .spawn(async move { watch::stamps(&root, &folders, index.as_deref()) });
    cx.spawn(async move |this, cx| {
        let now = job.await;
        this.update(cx, |v, cx| {
            if v.load_seq == seq && !v.loading && watch::changed(&v.stamps, &now) {
                refresh(v, cx);
            }
        })
        .ok();
    })
    .detach();
}

pub fn set_root(view: &Entity<FilesView>, root: Option<String>, cx: &mut App) {
    view.update(cx, |v, cx| change_root(v, root, cx));
}

pub fn change_root(v: &mut FilesView, root: Option<String>, cx: &mut Context<FilesView>) {
    if v.root == root {
        return;
    }
    if let Some(old) = v.root.take() {
        v.saved.insert(old, v.tree.open.clone());
    }
    v.tree = tree::Tree {
        open: root
            .as_ref()
            .and_then(|r| v.saved.get(r).cloned())
            .unwrap_or_default(),
        ..tree::Tree::default()
    };
    v.root = root;
    v.rows.clear();
    v.marks.clear();
    v.index = None;
    v.stamps.clear();
    v.error = None;
    v.loaded = false;
    v.loading = false;
    v.again = false;
    v.menu = None;
    v.prompt = None;
    v.confirm = None;
    v.load_seq += 1;
    v.search.update(cx, |input, cx| input.set_value("", cx));
    reset_search(v);
    refresh(v, cx);
    cx.notify();
}

pub fn set_tabs(view: &Entity<FilesView>, tabs: HashSet<String>, cx: &mut App) {
    view.update(cx, |v, _| {
        v.tabs = Some(tabs);
    });
}

pub fn opened(view: &Entity<FilesView>, file: &str, keep: bool, tab: &str, cx: &mut App) {
    view.update(cx, |v, _| {
        if let Some(tabs) = v.tabs.as_mut() {
            tabs.insert(tab.to_string());
        }
        preview::opened(&mut v.previews, file, keep, tab);
    });
}

pub fn keep_tab(view: &Entity<FilesView>, tab: &str, cx: &mut App) {
    view.update(cx, |v, _| preview::keep_tab(&mut v.previews, tab));
}

pub fn commands() -> Vec<(&'static str, Command)> {
    vec![
        ("Files: refresh the list", Command::Refresh),
        ("Files: search names and contents", Command::Search),
        ("Files: close all folders", Command::Collapse),
    ]
}

pub fn run(view: &Entity<FilesView>, command: Command, window: &mut Window, cx: &mut App) {
    view.update(cx, |v, cx| match command {
        Command::Refresh => refresh(v, cx),
        Command::Search => {
            let focus = v.search.focus_handle(cx);
            window.focus(&focus);
        }
        Command::Collapse => {
            v.tree.open.clear();
            v.tree.entries.retain(|k, _| k.is_empty());
            v.rows = tree::rows(&v.tree);
            cx.notify();
        }
    });
}

struct Loaded {
    tree: tree::Tree,
    marks: HashMap<String, git::Mark>,
    index: Option<PathBuf>,
    stamps: watch::Stamps,
    error: Option<String>,
}

fn load_all(root: &str, open: BTreeSet<String>) -> Loaded {
    let mut tree = tree::Tree {
        open,
        ..tree::Tree::default()
    };
    let error = match tree::load(&mut tree, root, "") {
        Ok(()) => None,
        Err(e) => Some(format!("Cannot list this folder: {e}")),
    };
    for path in tree.open.clone() {
        if !tree.open.contains(tree::parent(&path)) && !tree::parent(&path).is_empty() {
            tree.open.remove(&path);
            continue;
        }
        if tree::load(&mut tree, root, &path).is_err() {
            tree.open.remove(&path);
        }
    }
    let marks = if error.is_none() {
        git::marks(root)
    } else {
        HashMap::new()
    };
    let index = git::index_path(root);
    let stamps = watch::stamps(root, &tree::folders(&tree), index.as_deref());
    Loaded {
        tree,
        marks,
        index,
        stamps,
        error,
    }
}

pub fn refresh(v: &mut FilesView, cx: &mut Context<FilesView>) {
    let Some(root) = v.root.clone() else {
        cx.notify();
        return;
    };
    if v.loading {
        v.again = true;
        return;
    }
    v.loading = true;
    v.again = false;
    v.load_seq += 1;
    let seq = v.load_seq;
    let open = v.tree.open.clone();
    let job = cx
        .background_executor()
        .spawn(async move { load_all(&root, open) });
    cx.spawn(async move |this, cx| {
        let done = job.await;
        this.update(cx, |v, cx| {
            if v.load_seq != seq {
                return;
            }
            v.loading = false;
            v.loaded = true;
            v.tree = done.tree;
            v.marks = done.marks;
            v.index = done.index;
            v.stamps = done.stamps;
            v.error = done.error;
            v.rows = tree::rows(&v.tree);
            if v.again {
                refresh(v, cx);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn toggle_folder(v: &mut FilesView, path: &str, cx: &mut Context<FilesView>) {
    let opened = tree::toggle(&mut v.tree, path);
    v.rows = tree::rows(&v.tree);
    cx.notify();
    if !opened || v.tree.entries.contains_key(path) {
        return;
    }
    let Some(root) = v.root.clone() else { return };
    let seq = v.load_seq;
    let path = path.to_string();
    let full = tree::join(&root, &path);
    let job = cx
        .background_executor()
        .spawn(async move { tree::read_folder(Path::new(&full)) });
    cx.spawn(async move |this, cx| {
        let result = job.await;
        this.update(cx, |v, cx| {
            if v.load_seq != seq || !v.tree.open.contains(&path) {
                return;
            }
            match result {
                Ok((list, more)) => {
                    tree::put(&mut v.tree, &path, list, more);
                    let folder = tree::join(v.root.as_deref().unwrap_or(""), &path);
                    v.stamps.insert(path.clone(), watch::stamp(&folder));
                }
                Err(e) => v.error = Some(format!("Cannot open {path}: {e}")),
            }
            v.rows = tree::rows(&v.tree);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn open_file(v: &mut FilesView, path: &str, keep: bool, cx: &mut Context<FilesView>) {
    open_as(v, path, keep, false, cx);
}

pub fn open_in_terminal(v: &mut FilesView, path: &str, cx: &mut Context<FilesView>) {
    open_as(v, path, true, true, cx);
}

fn open_as(
    v: &mut FilesView,
    path: &str,
    keep: bool,
    terminal: bool,
    cx: &mut Context<FilesView>,
) {
    let Some(root) = v.root.as_deref() else {
        return;
    };
    if !tree::safe_relative(path) {
        return;
    }
    let file = tree::join(root, path);
    if !terminal {
        cx.emit(FilesEvent::Open {
            file,
            keep,
            replace: None,
        });
        return;
    }
    let tabs = &v.tabs;
    let alive = |tab: &str| tabs.as_ref().is_none_or(|t| t.contains(tab));
    match preview::plan(&mut v.previews, &file, keep, &alive) {
        Plan::Select(tab) => cx.emit(FilesEvent::Select(tab)),
        Plan::Open { keep, replace } => cx.emit(FilesEvent::Terminal {
            file,
            keep,
            replace,
        }),
    }
}

pub fn target_of(v: &FilesView, path: &str, dir: bool) -> Option<Target> {
    Some(Target {
        root: v.root.clone()?,
        path: path.to_string(),
        dir,
    })
}

pub fn open_menu(
    v: &mut FilesView,
    target: Target,
    at: Point<Pixels>,
    window: &mut Window,
    cx: &mut Context<FilesView>,
) {
    v.menu = Some(Menu {
        target,
        at,
        hi: None,
    });
    window.focus(&v.focus);
    cx.notify();
}

pub fn close_menu(v: &mut FilesView, cx: &mut Context<FilesView>) {
    if v.menu.take().is_some() {
        cx.notify();
    }
}

pub fn pick(v: &mut FilesView, index: usize, window: &mut Window, cx: &mut Context<FilesView>) {
    let Some(menu) = v.menu.take() else { return };
    let Some(action) = actions::menu_items(&menu.target).get(index).copied() else {
        return;
    };
    choose(v, action, menu.target, window, cx);
    cx.notify();
}

pub fn notify(
    cx: &mut Context<FilesView>,
    title: impl Into<String>,
    body: Option<String>,
    error: bool,
) {
    cx.emit(FilesEvent::Notify {
        title: title.into(),
        body,
        error,
    });
}

pub fn choose(
    v: &mut FilesView,
    action: FileAction,
    target: Target,
    window: &mut Window,
    cx: &mut Context<FilesView>,
) {
    if let Err(e) = actions::check(&target) {
        notify(cx, "File action failed", Some(e.to_string()), true);
        return;
    }
    let full = target.full();
    match action {
        FileAction::NewFile | FileAction::NewFolder | FileAction::Rename => {
            open_prompt(v, action, target, window, cx)
        }
        FileAction::Delete => {
            v.confirm = Some(Confirm {
                target,
                error: None,
            });
            window.focus(&v.focus);
        }
        FileAction::Reveal => {
            launch_open(cx, &full, true);
        }
        FileAction::Open => {
            launch_open(cx, &full, false);
        }
        FileAction::Copy => {
            cx.write_to_clipboard(ClipboardItem::new_string(full));
            notify(cx, actions::done_title(action, None, &target), None, false);
        }
        FileAction::Run => match actions::run_argv(&target) {
            Ok((name, folder, argv)) => {
                cx.emit(FilesEvent::Run { name, folder, argv });
            }
            Err(e) => notify(cx, "File action failed", Some(e.to_string()), true),
        },
        FileAction::Edit => {
            if target.dir {
                notify(
                    cx,
                    "File action failed",
                    Some("Only files open in the editor".into()),
                    true,
                );
            } else {
                cx.emit(FilesEvent::Edit { file: full });
            }
        }
        FileAction::Terminal => {
            if target.dir {
                notify(
                    cx,
                    "File action failed",
                    Some("Only files open in a terminal".into()),
                    true,
                );
            } else {
                open_in_terminal(v, &target.path, cx);
            }
        }
        FileAction::Gitignore | FileAction::Exclude => act(v, action, target, String::new(), cx),
    }
}

fn launch_open(cx: &mut Context<FilesView>, file: &str, reveal: bool) {
    let file = file.to_owned();
    let job = cx.background_executor().spawn(async move {
        let mut command = std::process::Command::new("/usr/bin/open");
        if reveal {
            command.arg("-R");
        }
        command.arg("--").arg(file).output()
    });
    cx.spawn(async move |this, cx| {
        let output = job.await;
        this.update(cx, |_, cx| match output {
            Ok(output) if output.status.success() => notify(cx, if reveal { "Revealed in Finder" } else { "Opened" }, None, false),
            Ok(output) => notify(cx, "File action failed", Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()), true),
            Err(error) => notify(cx, "File action failed", Some(error.to_string()), true),
        }).ok();
    }).detach();
}

pub fn open_prompt(
    v: &mut FilesView,
    action: FileAction,
    target: Target,
    window: &mut Window,
    cx: &mut Context<FilesView>,
) {
    let input = cx.new(|cx| TextInput::new(cx, "Name"));
    if action == FileAction::Rename {
        let name = tree::file_name(&target.path).to_string();
        input.update(cx, |input, cx| {
            input.set_value(name, cx);
            input.select_value(cx);
        });
    }
    let events = cx.subscribe(
        &input,
        |v: &mut FilesView, _, event: &InputEvent, cx| match event {
            InputEvent::Submit(text) => submit(v, text.clone(), cx),
            InputEvent::Cancel => close_dialogs(v, cx),
            InputEvent::Changed(_) => {
                if let Some(p) = v.prompt.as_mut()
                    && p.error.take().is_some()
                {
                    cx.notify();
                }
            }
        },
    );
    window.focus(&input.focus_handle(cx));
    v.prompt = Some(Prompt {
        action,
        target,
        input,
        error: None,
        _events: events,
    });
    cx.notify();
}

pub fn submit(v: &mut FilesView, text: String, cx: &mut Context<FilesView>) {
    let Some(p) = v.prompt.as_ref() else { return };
    if text.is_empty() {
        return;
    }
    let (action, target) = (p.action, p.target.clone());
    act(v, action, target, text, cx);
}

pub fn close_dialogs(v: &mut FilesView, cx: &mut Context<FilesView>) {
    v.prompt = None;
    v.confirm = None;
    v.menu = None;
    cx.notify();
}

pub fn act(
    v: &mut FilesView,
    action: FileAction,
    target: Target,
    name: String,
    cx: &mut Context<FilesView>,
) {
    if v.busy {
        return;
    }
    v.busy = true;
    let home = v.home.clone();
    let job_target = target.clone();
    let job = cx
        .background_executor()
        .spawn(async move { actions::run(action, &job_target, &name, &home) });
    cx.spawn(async move |this, cx| {
        let result = job.await;
        this.update(cx, |v, cx| {
            v.busy = false;
            match result {
                Ok(done) => {
                    if let actions::Done::Renamed(to) = &done {
                        preview::renamed(&mut v.previews, &target.full(), to);
                    }
                    notify(
                        cx,
                        actions::done_title(action, Some(&done), &target),
                        None,
                        false,
                    );
                    v.prompt = None;
                    v.confirm = None;
                    refresh(v, cx);
                    if !v.query.trim().is_empty() {
                        start_search(v, cx);
                    }
                }
                Err(e) => {
                    let text = e.to_string();
                    if let Some(p) = v.prompt.as_mut() {
                        p.error = Some(text);
                    } else if let Some(c) = v.confirm.as_mut() {
                        c.error = Some(text);
                    } else {
                        notify(cx, "File action failed", Some(text), true);
                    }
                }
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn confirm_delete(v: &mut FilesView, cx: &mut Context<FilesView>) {
    if let Some(target) = v.confirm.as_ref().map(|c| c.target.clone()) {
        act(v, FileAction::Delete, target, String::new(), cx);
    }
}

fn reset_search(v: &mut FilesView) {
    v.query.clear();
    v.found = search::Found::default();
    v.searching = false;
    v.search_error = None;
    v.search_seq += 1;
}

pub fn clear_search(v: &mut FilesView, cx: &mut Context<FilesView>) {
    v.search.update(cx, |input, cx| input.set_value("", cx));
    reset_search(v);
    cx.notify();
}

pub fn typed(v: &mut FilesView, text: String, cx: &mut Context<FilesView>) {
    v.query = text;
    v.search_error = None;
    v.search_seq += 1;
    if v.query.trim().is_empty() {
        v.found = search::Found::default();
        v.searching = false;
        cx.notify();
        return;
    }
    let seq = v.search_seq;
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(SEARCH_WAIT).await;
        this.update(cx, |v, cx| {
            if v.search_seq == seq {
                start_search(v, cx);
            }
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn start_search(v: &mut FilesView, cx: &mut Context<FilesView>) {
    v.search_seq += 1;
    let query = v.query.clone();
    let Some(root) = v.root.clone().filter(|_| !query.trim().is_empty()) else {
        v.found = search::Found::default();
        v.searching = false;
        cx.notify();
        return;
    };
    v.searching = true;
    v.search_error = None;
    let seq = v.search_seq;
    let job = cx
        .background_executor()
        .spawn(async move { search::search(&root, &query) });
    cx.spawn(async move |this, cx| {
        let result = job.await;
        this.update(cx, |v, cx| {
            if v.search_seq != seq {
                return;
            }
            v.searching = false;
            match result {
                Ok(found) => v.found = found,
                Err(e) => {
                    v.found = search::Found::default();
                    v.search_error = Some(e.to_string());
                }
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}
