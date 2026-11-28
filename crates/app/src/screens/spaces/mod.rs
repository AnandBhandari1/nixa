#![allow(dead_code)]
mod checks;
mod draw;
mod forms;
pub mod git;
pub mod order;
#[cfg(test)]
mod tests;

use crate::spaces::{self, Space};
use crate::tabs::LABEL_SPACE;
use nixa_ui::metrics::SIDEBAR_NARROW;
use crate::workspace::Workspace;
use checks::Checks;
use git::{GitInfo, Look, Worktree};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, EventEmitter, Pixels, Point, Subscription, Window};
use nixa_protocol::{Status, TermId};
use nixa_ui::text_input::TextInput;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(500);
const STAMP_EVERY: Duration = Duration::from_secs(2);
const RELOAD_EVERY: Duration = Duration::from_secs(2);
const BORN_WAIT: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub blocked: usize,
    pub done: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Input {
    pub spaces: Vec<Space>,
    pub active: Option<String>,
    pub places: HashMap<String, String>,
    pub counts: HashMap<String, Counts>,
    pub tabs: HashMap<String, usize>,
    pub busy: BTreeMap<TermId, (String, bool)>,
    pub narrow: bool,
    pub window_active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpacesEvent {
    Select(String),
    New(Option<String>),
    Add { folder: String, name: String },
    Rename { id: String, name: String },
    Move { id: String, to: usize },
    Lock { id: String, locked: bool },
    Close(String),
    Menu { id: String, at: Point<Pixels> },
    Notice(String),
    Reload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewSpace,
    NewSpaceInFolder,
    Rename,
    Lock,
    NewWorktree,
    OpenWorktree,
    RemoveWorktree,
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    pub id: String,
    pub block: usize,
    pub over: Option<usize>,
    pub first: usize,
    pub last: usize,
}

pub enum Form {
    None,
    Rename { id: String, input: Entity<TextInput> },
    Branch { id: String, input: Entity<TextInput>, busy: bool },
    Close { id: String, tabs: usize },
    Remove { id: String, busy: bool },
    Open { id: String, list: Vec<Worktree> },
}

impl Form {
    pub fn space(&self) -> Option<&str> {
        match self {
            Form::None => None,
            Form::Rename { id, .. }
            | Form::Branch { id, .. }
            | Form::Close { id, .. }
            | Form::Remove { id, .. }
            | Form::Open { id, .. } => Some(id),
        }
    }
}

pub struct SpacesView {
    pub input: Input,
    pub checks: Checks,
    pub shown: HashSet<String>,
    pub born: HashMap<String, Instant>,
    pub started: bool,
    pub drag: Option<Drag>,
    pub form: Form,
    pub git_on: bool,
    scroll: gpui::ScrollHandle,
    stamp_at: Instant,
    reload_at: Instant,
    form_events: Option<Subscription>,
}

impl EventEmitter<SpacesEvent> for SpacesView {}

pub fn build(cx: &mut Context<SpacesView>) -> SpacesView {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            if this.update(cx, tick).is_err() {
                break;
            }
        }
    })
    .detach();
    let now = Instant::now();
    SpacesView {
        input: Input::default(),
        checks: Checks::default(),
        shown: HashSet::new(),
        born: HashMap::new(),
        started: false,
        drag: None,
        form: Form::None,
        git_on: true,
        scroll: gpui::ScrollHandle::new(),
        stamp_at: now,
        reload_at: now,
        form_events: None,
    }
}

pub fn new(cx: &mut App) -> Entity<SpacesView> {
    cx.new(build)
}

pub fn render(view: &Entity<SpacesView>) -> AnyElement {
    view.clone().into_any_element()
}

pub fn sync(view: &Entity<SpacesView>, input: Input, cx: &mut App) {
    view.update(cx, |v, cx| {
        if v.input != input {
            v.input = input;
            on_input(v, cx);
            cx.notify();
        }
    });
}

pub fn run(view: &Entity<SpacesView>, action: Action, space: Option<&str>, window: &mut Window, cx: &mut App) {
    forms::run(view, action, space, window, cx);
}

pub fn linked(view: &Entity<SpacesView>, id: &str, cx: &App) -> Option<bool> {
    git_of(view.read(cx), id).map(|g| g.linked)
}

pub fn commands() -> Vec<(&'static str, Action)> {
    vec![
        ("New space", Action::NewSpace),
        ("New space in another folder…", Action::NewSpaceInFolder),
        ("Rename this space", Action::Rename),
        ("Lock or open this space for Nixa", Action::Lock),
        ("New worktree from this space", Action::NewWorktree),
        ("Open a worktree of this space", Action::OpenWorktree),
        ("Remove this worktree", Action::RemoveWorktree),
        ("Close this space", Action::Close),
    ]
}

pub fn input(ws: &Workspace, window: &Window) -> Input {
    let mut places = HashMap::new();
    let mut tabs: HashMap<String, usize> = HashMap::new();
    for tab in &ws.tabs.list {
        *tabs.entry(tab.space.clone()).or_default() += 1;
        if places.contains_key(&tab.space) {
            continue;
        }
        let cwd = crate::layout::leaves(&tab.root)
            .first()
            .and_then(|id| ws.terms.get(id))
            .map(|t| t.cwd.clone())
            .filter(|c| !c.is_empty());
        if let Some(cwd) = cwd {
            places.insert(tab.space.clone(), cwd);
        }
    }
    let mut counts: HashMap<String, Counts> = HashMap::new();
    let mut busy = BTreeMap::new();
    for term in ws.terms.values() {
        let Some(space) = term.labels.get(LABEL_SPACE) else {
            continue;
        };
        busy.insert(
            term.id.clone(),
            (space.clone(), term.running && term.status == Some(Status::Working)),
        );
        if !term.running {
            continue;
        }
        let c = counts.entry(space.clone()).or_default();
        match term.status {
            Some(Status::Blocked) => c.blocked += 1,
            Some(Status::Done) => c.done += 1,
            _ => {}
        }
    }
    counts.retain(|_, c| c.blocked + c.done > 0);
    Input {
        spaces: ws.spaces.spaces.clone(),
        active: ws.spaces.active.clone(),
        places,
        counts,
        tabs,
        busy,
        narrow: ws.sidebar_w < SIDEBAR_NARROW,
        window_active: window.is_window_active(),
    }
}

pub fn handle(ws: &mut Workspace, event: &SpacesEvent, cx: &mut Context<Workspace>) {
    match event {
        SpacesEvent::Select(id) => ws.select_space(id, cx),
        SpacesEvent::New(folder) => ws.add_space(folder.clone().unwrap_or_else(spaces::home), cx),
        SpacesEvent::Add { folder, name } => {
            ws.add_space(folder.clone(), cx);
            if let Some(id) = ws.spaces.active.clone() {
                ws.rename_space(&id, name, cx);
            }
        }
        SpacesEvent::Rename { id, name } => ws.rename_space(id, name, cx),
        SpacesEvent::Move { id, to } => {
            if spaces::move_to(&mut ws.spaces, id, *to) {
                let result = store(ws, spaces::write_order);
                saved(ws, result, "Cannot save the space order");
            }
            cx.notify();
        }
        SpacesEvent::Lock { id, locked } => {
            if spaces::set_locked(&mut ws.spaces, id, *locked) {
                let result = store(ws, |conn, _| spaces::write_lock(conn, id, *locked));
                saved(ws, result, "Cannot save the space lock");
            }
            cx.notify();
        }
        SpacesEvent::Close(id) => close(ws, id, cx),
        SpacesEvent::Menu { id, at } => {
            ws.ctx_wanted = Some((crate::screens::ctxmenu::Target::Space { id: id.clone() }, *at));
            cx.notify();
        }
        SpacesEvent::Notice(text) => {
            ws.notice = Some(text.clone());
            cx.notify();
        }
        SpacesEvent::Reload => {
            let mut list = std::mem::take(&mut ws.spaces);
            let result = match &ws.store {
                Some(conn) => spaces::refresh(conn, &mut list),
                None => Ok(false),
            };
            ws.spaces = list;
            match result {
                Ok(true) => cx.notify(),
                Ok(false) => {}
                Err(e) => saved(ws, Err(e), "Cannot read the space locks"),
            }
        }
    }
}

fn store(
    ws: &Workspace,
    job: impl FnOnce(&nixa_store::Connection, &spaces::Spaces) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    match &ws.store {
        Some(conn) => job(conn, &ws.spaces),
        None => Ok(()),
    }
}

fn saved(ws: &mut Workspace, result: anyhow::Result<()>, what: &str) {
    if let Err(e) = result {
        ws.notice = Some(format!("{what}: {e:#}"));
    }
}

fn close(ws: &mut Workspace, id: &str, cx: &mut Context<Workspace>) {
    if spaces::find(&ws.spaces, id).is_none() {
        return;
    }
    let tabs: Vec<String> = ws.space_tabs(id).map(|t| t.id.clone()).collect();
    for tab in tabs {
        ws.close_tab(&tab, cx);
    }
    let was_active = ws.spaces.active.as_deref() == Some(id);
    let fixed = spaces::close(&mut ws.spaces, id);
    ws.tabs.active.remove(id);
    let result = store(ws, |conn, list| {
        spaces::delete(conn, id)?;
        spaces::write_fixed(conn, list, &fixed)?;
        spaces::write_active(conn, list)
    });
    saved(ws, result, "Cannot save spaces");
    if was_active && let Some(next) = ws.spaces.active.clone() {
        ws.panel = crate::workspace::saved_panel(ws.store.as_ref(), &next);
    }
    cx.notify();
}

/// The brain changed the spaces (nixa_spaces): close the ones it removed, then merge the rest from the data folder.
pub fn sync_from_brain(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let gone: Vec<String> = match &ws.store {
        Some(conn) => spaces::saved_ids(conn)
            .map(|saved| ws.spaces.spaces.iter().filter(|s| !saved.contains(&s.id)).map(|s| s.id.clone()).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    for id in gone {
        close(ws, &id, cx);
    }
    handle(ws, &SpacesEvent::Reload, cx);
    cx.notify();
}

pub fn place(v: &SpacesView, space: &Space) -> String {
    checks::place_of(&v.input, space)
}

pub fn git_of(v: &SpacesView, id: &str) -> Option<GitInfo> {
    let space = v.input.spaces.iter().find(|s| s.id == id)?;
    v.checks.git.get(&place(v, space)).cloned().flatten()
}

fn on_input(v: &mut SpacesView, cx: &mut Context<SpacesView>) {
    let now = Instant::now();
    let ids: HashSet<String> = v.input.spaces.iter().map(|s| s.id.clone()).collect();
    if !v.started && !ids.is_empty() {
        v.shown = ids.clone();
        v.started = true;
    }
    for id in &ids {
        if !v.shown.contains(id) {
            v.born.entry(id.clone()).or_insert(now);
        }
    }
    v.shown.retain(|id| ids.contains(id));
    v.born.retain(|id, _| ids.contains(id));
    if v.form.space().is_some_and(|id| !ids.contains(id)) {
        set_form(v, Form::None, None);
    }
    checks::prune(&mut v.checks, &v.input);
    let due = checks::due(&mut v.checks, &v.input, now);
    queue(v, due, cx);
    reveal(v, now);
}

fn reveal(v: &mut SpacesView, now: Instant) -> bool {
    let mut changed = false;
    let waiting: Vec<Space> = v
        .input
        .spaces
        .iter()
        .filter(|s| !v.shown.contains(&s.id))
        .cloned()
        .collect();
    for space in waiting {
        let known = v.checks.git.contains_key(&place(v, &space));
        let old = v.born.get(&space.id).is_none_or(|t| now.duration_since(*t) >= BORN_WAIT);
        if known || old || !v.git_on {
            v.shown.insert(space.id.clone());
            changed = true;
        }
    }
    changed
}

fn tick(v: &mut SpacesView, cx: &mut Context<SpacesView>) {
    let now = Instant::now();
    let mut changed = reveal(v, now);
    if v.input.window_active && now.duration_since(v.stamp_at) >= STAMP_EVERY {
        v.stamp_at = now;
        let woke = checks::stamp_due(&mut v.checks, git::stamp);
        queue(v, woke, cx);
    }
    let due = checks::due(&mut v.checks, &v.input, now);
    if !due.is_empty() {
        queue(v, due, cx);
        changed = true;
    }
    if now.duration_since(v.reload_at) >= RELOAD_EVERY {
        v.reload_at = now;
        cx.emit(SpacesEvent::Reload);
    }
    if changed {
        cx.notify();
    }
}

fn queue(v: &mut SpacesView, keys: impl IntoIterator<Item = String>, cx: &mut Context<SpacesView>) {
    if !v.git_on {
        return;
    }
    v.checks.pending.extend(keys);
    pump(v, cx);
}

fn pump(v: &mut SpacesView, cx: &mut Context<SpacesView>) {
    if v.checks.current.is_some() {
        return;
    }
    let Some(key) = v.checks.pending.pop_first() else {
        return;
    };
    v.checks.current = Some(key.clone());
    let task = cx.background_executor().spawn(async move {
        let look = git::look(&key);
        let stamp = match &look {
            Look::Repo(info) => Some(git::stamp(info)),
            _ => None,
        };
        (key, look, stamp)
    });
    cx.spawn(async move |this, cx| {
        let (key, look, stamp) = task.await;
        this.update(cx, |v, cx| {
            v.checks.current = None;
            checks::apply(&mut v.checks, &key, look, stamp, Instant::now());
            reveal(v, Instant::now());
            pump(v, cx);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn set_form(v: &mut SpacesView, form: Form, events: Option<Subscription>) {
    v.form = form;
    v.form_events = events;
}
