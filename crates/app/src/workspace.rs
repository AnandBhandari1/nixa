use crate::actions::{
    ClosePane, NixaScreen, NewAgentMenu, NewTab, NextAgent, NextTab, OpenCommands, OpenFavourites,
    OpenPicker, PrevAgent, PrevTab, SplitDown, SplitRight, ToggleSidebar, ZoomPane,
};
use crate::commands::{self, Palette};
use crate::layout::{self, Axis};
use crate::links::engine::{self, EngineMsg};
use crate::screens::events::{self, Offer};
use crate::screens::git::changes::diff_tabs::{self, DiffTabs};
use crate::screens::favourites::{self, view::Favourites, view::FavouritesEvent};
use crate::screens::picker::{self, view::Picker, view::PickerEvent};
use crate::spaces::{self, Space, Spaces};
use crate::tabs::{self, LABEL_SPACE, LABEL_TAB, Tab, Tabs};
use crate::screens::settings;
use crate::{hub, panel, panes, rail, sidebar, statusbar, tabbar, views};
use gpui::prelude::*;
use gpui::{
    Context, DragMoveEvent, Entity, FocusHandle, Focusable, PathPromptOptions, Subscription,
    Window, deferred, div, px,
};
use nixa_protocol::{CreateSpec, Event, TermId, TermInfo, paths};
use nixa_store::Connection;
use nixa_termview::{TermView, TermViewEvent};
use nixa_ui::metrics::{
    PANE_GAP, PANEL_MAX, PANEL_MIN, PANEL_WIDTH, RAIL_WIDTH, SIDEBAR_MAX, SIDEBAR_MIN,
    SIDEBAR_NARROW, SIDEBAR_WIDTH,
};
use nixa_ui::split::{Edge, SplitDrag, size_from_drag};
use nixa_ui::theme;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(500);

pub const SIDEBAR_SPLIT: &str = "sidebar";
pub const PANEL_SPLIT: &str = "panel";
pub const SIDEBAR_WIDTH_KEY: &str = "app.sidebar_width";
pub const SIDEBAR_OPEN_WIDTH_KEY: &str = "app.sidebar_open_width";
pub const PANEL_WIDTH_KEY: &str = "app.panel_width";
pub const PANEL_KEY: &str = "app.panel.";
pub const SHOW_ENV: &str = "NIXA_APP_SHOW";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Files,
    Git,
    Nixa,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineState {
    Starting,
    Up,
    Down(String),
}

pub type TabDone = Box<dyn FnOnce(&mut Workspace, Result<(), String>, &mut Context<Workspace>)>;

enum Then {
    NewTab {
        tab: String,
        space: String,
    },
    Split {
        tab: String,
        target: TermId,
        axis: Axis,
    },
}

pub struct Workspace {
    pub focus: FocusHandle,
    pub spaces: Spaces,
    pub store: Option<Connection>,
    pub views: BTreeMap<TermId, Entity<TermView>>,
    view_events: BTreeMap<TermId, Subscription>,
    socket: PathBuf,
    pub tabs: Tabs,
    pub tabs_path: PathBuf,
    pub terms: BTreeMap<TermId, TermInfo>,
    pub hovered_pane_header: Option<TermId>,
    pub engine: EngineState,
    pub sidebar_w: f32,
    pub sidebar_open_w: f32,
    pub panel_w: f32,
    pub panel: Option<PanelKind>,
    pub palette: Option<Palette>,
    pub picker: Option<(Entity<Picker>, Subscription)>,
    pub favourites: Option<(Entity<Favourites>, Subscription)>,
    pub ctx: Option<crate::screens::ctxmenu::host::Open>,
    pub ctx_wanted: Option<(crate::screens::ctxmenu::Target, gpui::Point<gpui::Pixels>)>,
    show_at_start: Option<String>,
    pub notice: Option<String>,
    pub menu: Option<crate::screens::menu::run::Host>,
    pub hub: hub::Hub,
    pub files: Entity<crate::screens::files::FilesView>,
    _files_events: Subscription,
    pub changes: Entity<crate::screens::git::changes::ChangesView>,
    _changes_events: Subscription,
    pub git_diffs: DiffTabs,
    pub facts: crate::screens::statusbar::host::Facts,
    pub tab_bar: Entity<crate::screens::tabbar::TabBar>,
    pub refocus: bool,
    saved_widths: (f32, f32, f32),
    events: Receiver<EngineMsg>,
    pub spaces_view: Entity<crate::screens::spaces::SpacesView>,
    _spaces_events: Subscription,
    _nixa_screen_events: Subscription,
    tab_done: BTreeMap<String, TabDone>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (store, mut notice) = match nixa_store::open_default() {
            Ok(conn) => (Some(conn), None),
            Err(e) => (
                None,
                Some(format!(
                    "Cannot open the store, so spaces are kept in memory only: {e:#}"
                )),
            ),
        };
        let mut spaces = Spaces::default();
        if let Some(conn) = &store {
            if let Err(e) = spaces::import(conn, &spaces::old_file()) {
                notice = Some(format!("Cannot import the old spaces list: {e:#}"));
            }
            match spaces::load(conn) {
                Ok(list) => spaces = list,
                Err(e) => notice = Some(format!("Cannot read spaces from the store: {e:#}")),
            }
        }
        let fixed = spaces::fix_active(&mut spaces);
        if let Some(conn) = &store
            && let Err(e) = spaces::write_fixed(conn, &spaces, &fixed)
        {
            notice = Some(format!("Cannot save spaces: {e:#}"));
        }
        if let Some(conn) = &store {
            let ids: Vec<String> = spaces.spaces.iter().map(|s| s.id.clone()).collect();
            if let Err(e) = crate::screens::tabbar::panels::prune(conn, &ids) {
                notice = Some(format!("Cannot clean the saved panels: {e:#}"));
            }
        }
        let tabs_path = tabs::layout_path();
        let tabs = Tabs {
            saved: tabs::load_layouts(&tabs_path),
            ..Tabs::default()
        };
        let (sidebar_w, panel_w, sidebar_open_w) = saved_widths(store.as_ref());
        let panel = spaces
            .active
            .as_deref()
            .and_then(|s| saved_panel(store.as_ref(), s));
        let panel = crate::screens::tabbar::panels::at_start(
            panel,
            crate::screens::tabbar::panels::chat_open(store.as_ref()),
        );
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<()>();
        let hub = hub::new(cx, &mut notice, wake_tx.clone());
        crate::screens::first_run::host::attach(store.as_ref(), cx);
        crate::screens::menu::run::switches_for(cx);
        let (files, files_events) = crate::screens::files::host::new(cx);
        let (changes, changes_events) = crate::screens::git::changes::host::new(cx);
        let nixa_screen_events = crate::screens::nixa_screen::host::new(hub.link.clone(), cx);
        cx.observe_global::<nixa_ui::Theme>(|ws: &mut Workspace, cx| ws.theme_changed(cx))
            .detach();
        let tab_bar = crate::screens::tabbar::new(hub.link.clone(), cx);
        cx.observe(&tab_bar, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        window.focus(&focus);
        if !cfg!(test)
            && let Some(why) = crate::links::control::start(window, cx)
        {
            notice = Some(why);
        }
        let events = engine::watch_wake(wake_tx);
        let spaces_view = crate::screens::spaces::new(cx);
        let spaces_events = cx.subscribe(
            &spaces_view,
            |ws, _, event: &crate::screens::spaces::SpacesEvent, cx| {
                crate::screens::spaces::handle(ws, event, cx)
            },
        );
        cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            while wake_rx.next().await.is_some() {
                while wake_rx.try_recv().is_ok() {}
                if this.update(cx, |ws, cx| ws.drain(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this.update(cx, |ws, cx| ws.drain(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            focus,
            spaces,
            store,
            views: BTreeMap::new(),
            view_events: BTreeMap::new(),
            socket: paths::socket_path(),
            tabs,
            tabs_path,
            terms: BTreeMap::new(),
            hovered_pane_header: None,
            engine: EngineState::Starting,
            sidebar_w,
            sidebar_open_w,
            panel_w,
            panel,
            palette: None,
            picker: None,
            favourites: None,
            ctx: None,
            ctx_wanted: None,
            show_at_start: std::env::var(SHOW_ENV).ok().filter(|_| !cfg!(test)),
            notice,
            menu: None,
            hub,
            files,
            _files_events: files_events,
            changes,
            _changes_events: changes_events,
            git_diffs: DiffTabs::default(),
            facts: Default::default(),
            tab_bar,
            refocus: false,
            saved_widths: (sidebar_w, panel_w, sidebar_open_w),
            events,
            spaces_view,
            _spaces_events: spaces_events,
            _nixa_screen_events: nixa_screen_events,
            tab_done: BTreeMap::new(),
        }
    }

    pub fn active_space(&self) -> Option<&Space> {
        spaces::active(&self.spaces)
    }

    pub fn space_tabs<'a>(&'a self, space: &'a str) -> impl Iterator<Item = &'a Tab> + 'a {
        self.tabs.list.iter().filter(move |t| t.space == space)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        let space = self.active_space()?;
        if self.git_diffs.selected(&space.id).is_some() {
            return None;
        }
        let id = self.tabs.active.get(&space.id)?;
        tabs::find(&self.tabs, id)
    }

    pub fn tab_term<'a>(&'a self, tab: &Tab) -> Option<&'a TermInfo> {
        layout::leaves(&tab.root)
            .into_iter()
            .filter_map(|id| self.terms.get(&id))
            .filter(|term| term.running)
            .max_by_key(|t| crate::terms::status_rank(t))
    }

    pub fn tab_name(&self, tab: &Tab) -> String {
        layout::leaves(&tab.root)
            .first()
            .and_then(|id| self.terms.get(id))
            .map(crate::terms::term_name)
            .unwrap_or_else(|| "shell".to_string())
    }

    pub fn loose(&self) -> Vec<&TermInfo> {
        let placed = tabs::placed(&self.tabs);
        self.terms
            .values()
            .filter(|t| t.parent.is_none() && !placed.contains(&t.id))
            .filter(|t| {
                !t.labels
                    .contains_key(crate::screens::menu::runner::LABEL_POPUP)
            })
            .filter(|t| {
                let space_ok = t
                    .labels
                    .get(LABEL_SPACE)
                    .is_some_and(|s| spaces::find(&self.spaces, s).is_some());
                !space_ok || !t.labels.contains_key(LABEL_TAB)
            })
            .collect()
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Ok(msg) = self.events.try_recv() {
            changed |= self.apply(msg);
        }
        if hub::drain(self, cx) {
            cx.notify();
        }
        if self.engine == EngineState::Up && self.adopt_labelled(cx) {
            changed = true;
        }
        if crate::screens::statusbar::host::refresh_facts(self) {
            cx.notify();
        }
        crate::screens::files::host::sync(self, cx);
        crate::screens::git::changes::host::sync(self, cx);
        crate::screens::nixa_screen::host::sync(self, cx);
        self.save_widths();
        if changed {
            if let Some(view) = self.picker.as_ref().map(|p| p.0.clone()) {
                let snap = picker::data::snapshot(self, cx);
                view.update(cx, |p, cx| p.set_snapshot(snap, cx));
            }
            self.persist_layouts();
            cx.notify();
        }
    }

    fn apply(&mut self, msg: EngineMsg) -> bool {
        match msg {
            EngineMsg::Up(terms) => {
                self.engine = EngineState::Up;
                self.terms = terms.into_iter().map(|t| (t.id.clone(), t)).collect();
                self.sync_tabs();
                true
            }
            EngineMsg::Down(why) => {
                if self.engine == EngineState::Down(why.clone()) {
                    return false;
                }
                self.engine = EngineState::Down(why);
                true
            }
            EngineMsg::Event(event) => self.on_event(*event),
        }
    }

    fn update_term(&mut self, id: &TermId, f: impl FnOnce(&mut TermInfo)) -> bool {
        match self.terms.get_mut(id) {
            Some(term) => {
                f(term);
                true
            }
            None => false,
        }
    }

    fn on_event(&mut self, event: Event) -> bool {
        match event {
            Event::Created { term } => {
                self.terms.insert(term.id.clone(), term);
                self.sync_tabs();
                true
            }
            Event::Removed { id } => {
                self.terms.remove(&id);
                self.drop_view(&id);
                self.drop_leaf(&id);
                true
            }
            Event::Status { id, status, note } => self.update_term(&id, |t| {
                t.status = status;
                t.status_note = note;
            }),
            Event::Exit { id, code } => self.update_term(&id, |t| {
                t.running = false;
                t.exit_code = code;
            }),
            Event::Respawned { id, pid } => self.update_term(&id, |t| {
                t.running = true;
                t.pid = pid;
                t.exit_code = None;
            }),
            Event::Label { id, labels } => {
                let changed = self.update_term(&id, |t| t.labels = labels);
                self.sync_tabs();
                changed
            }
            Event::Renamed { id, name } => self.update_term(&id, |t| t.name = name),
            Event::Title { id, title } => self.update_term(&id, |t| t.title = title),
            Event::Resized { id, rows, cols } => self.update_term(&id, |t| {
                t.rows = rows;
                t.cols = cols;
            }),
            Event::Draft { id, active } => self.update_term(&id, |t| t.draft = active),
            _ => false,
        }
    }

    fn sync_tabs(&mut self) {
        let spaces = &self.spaces;
        tabs::sync(&mut self.tabs, &self.terms, |s| {
            spaces::find(spaces, s).is_some()
        });
        crate::hub::push_numbers(self);
    }

    fn persist_layouts(&mut self) {
        if self.engine != EngineState::Up {
            return;
        }
        if let Err(e) = tabs::persist(&mut self.tabs, &self.tabs_path) {
            self.notice = Some(format!("Cannot save the pane layout: {e}"));
        }
    }

    fn save_widths(&mut self) {
        let now = (self.sidebar_w.round(), self.panel_w.round(), self.sidebar_open_w.round());
        if now == self.saved_widths {
            return;
        }
        self.saved_widths = now;
        let result = self.with_store(|conn| write_widths(conn, now));
        if let Err(e) = result {
            self.notice = Some(format!("Cannot save the sidebar and panel width: {e:#}"));
        }
    }

    fn drop_leaf(&mut self, id: &TermId) {
        tabs::drop_leaf(&mut self.tabs, id);
    }

    fn spec(&self, space: &Space, tab: &str, cmd: Vec<String>, name: Option<String>) -> CreateSpec {
        let mut labels = BTreeMap::new();
        labels.insert(LABEL_SPACE.to_string(), space.id.clone());
        let name_slug = nixa_protocol::id::slug(&space.name);
        if !name_slug.is_empty() {
            labels.insert(nixa_protocol::id::SPACE_SLUG_LABEL.into(), name_slug);
        }
        labels.insert(LABEL_TAB.to_string(), tab.to_string());
        CreateSpec {
            cmd,
            cwd: Some(space.folder.clone()),
            name,
            labels,
            env: BTreeMap::from([(
                "PATH".to_string(),
                nixa_protocol::paths::tool_path(
                    &std::env::var("PATH").unwrap_or_default(),
                    nixa_protocol::paths::home_dir().as_deref(),
                ),
            )]),
            ..CreateSpec::default()
        }
    }

    fn engine_down(&mut self, cx: &mut Context<Self>) -> bool {
        if self.engine == EngineState::Up {
            return false;
        }
        self.notice =
            Some("nixa-engine is not running yet. Nixa starts it, see the status bar.".into());
        cx.notify();
        true
    }

    pub fn new_tab(&mut self, cmd: Vec<String>, name: Option<String>, cx: &mut Context<Self>) {
        self.new_tab_in(cmd, name, None, cx);
    }

    pub fn new_tab_in(
        &mut self,
        cmd: Vec<String>,
        name: Option<String>,
        cwd: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.engine_down(cx) {
            return;
        }
        let Some(space) = self.active_space().cloned() else {
            crate::screens::first_run::host::need_space(cx);
            return;
        };
        crate::screens::first_run::host::close(cx);
        let tab = tabs::new_id(&mut self.tabs);
        self.tabs.pending.push(tab.clone());
        let mut spec = self.spec(&space, &tab, cmd, name);
        if let Some(cwd) = cwd.filter(|c| std::path::Path::new(c).is_dir()) {
            spec.cwd = Some(cwd);
        }
        self.spawn_term(
            spec,
            Then::NewTab {
                tab,
                space: space.id,
            },
            cx,
        );
    }

    pub fn resume_offer(&mut self, offer: Offer, cx: &mut Context<Self>) {
        let Some(argv) = events::resume_argv(&offer.command) else {
            self.notice =
                Some("Nixa did not resume it: the resume command is not one it knows.".into());
            cx.notify();
            return;
        };
        if self.engine_down(cx) {
            return;
        }
        let space = self
            .spaces
            .spaces
            .iter()
            .find(|s| s.id == offer.space)
            .cloned()
            .or_else(|| self.active_space().cloned());
        let Some(space) = space else {
            return;
        };
        if self.spaces.active.as_deref() != Some(space.id.as_str()) {
            self.select_space(&space.id, cx);
        }
        let tab = tabs::new_id(&mut self.tabs);
        self.tabs.pending.push(tab.clone());
        let name = Some(offer.name.clone()).filter(|n| !n.trim().is_empty());
        let mut spec = self.spec(&space, &tab, argv, name);
        if std::path::Path::new(&offer.folder).is_dir() {
            spec.cwd = Some(offer.folder.clone());
        }
        self.spawn_term(
            spec,
            Then::NewTab {
                tab,
                space: space.id,
            },
            cx,
        );
    }

    pub fn split(&mut self, axis: Axis, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            self.new_tab(Vec::new(), None, cx);
            return;
        };
        self.split_pane(&tab.id.clone(), &tab.focus.clone(), axis, cx);
    }

    pub fn split_pane(&mut self, tab_id: &str, target: &TermId, axis: Axis, cx: &mut Context<Self>) {
        if self.engine_down(cx) {
            return;
        }
        let Some(tab) = tabs::find(&self.tabs, tab_id).filter(|tab| layout::contains(&tab.root, target)) else {
            return;
        };
        let tab = tab.clone();
        let Some(space) = spaces::find(&self.spaces, &tab.space).cloned() else {
            return;
        };
        self.tabs.pending.push(tab.id.clone());
        let spec = self.spec(&space, &tab.id, Vec::new(), None);
        self.spawn_term(
            spec,
            Then::Split {
                tab: tab.id,
                target: target.clone(),
                axis,
            },
            cx,
        );
    }

    fn spawn_term(&mut self, spec: CreateSpec, then: Then, cx: &mut Context<Self>) {
        let task = cx
            .background_executor()
            .spawn(async move { engine::create(spec) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |ws, cx| {
                let tab = then_tab(&then).to_string();
                let outcome = match result {
                    Ok(term) => {
                        ws.on_created(term, then);
                        Ok(())
                    }
                    Err(e) => {
                        ws.unpend(&then);
                        ws.notice = Some(format!("Cannot start a terminal: {e}"));
                        Err(format!("{e:#}"))
                    }
                };
                if let Some(done) = ws.tab_done.remove(&tab) {
                    done(ws, outcome, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn when_tab_made(&mut self, tab: &str, done: TabDone) {
        if self.tabs.pending.iter().any(|p| p == tab) {
            self.tab_done.insert(tab.to_string(), done);
        }
    }

    fn unpend(&mut self, then: &Then) {
        tabs::unpend(&mut self.tabs, then_tab(then));
    }

    fn on_created(&mut self, term: TermInfo, then: Then) {
        self.unpend(&then);
        let id = term.id.clone();
        self.terms.insert(id.clone(), term);
        match then {
            Then::NewTab { tab, space } => {
                tabs::place(&mut self.tabs, id.clone(), &space, &tab);
                if let Some(t) = tabs::find_mut(&mut self.tabs, &tab) {
                    t.focus = id;
                }
                self.git_diffs.show_terminals(&space);
                self.tabs.active.insert(space, tab);
            }
            Then::Split { tab, target, axis } => {
                if let Some(t) = tabs::find_mut(&mut self.tabs, &tab) {
                    if !layout::contains(&t.root, &id) {
                        t.root = if layout::contains(&t.root, &target) {
                            layout::split(t.root.clone(), &target, &id, axis)
                        } else {
                            layout::append(t.root.clone(), &id)
                        };
                    }
                    t.focus = id;
                }
            }
        }
        self.sync_tabs();
        self.persist_layouts();
    }

    pub fn close_pane(&mut self, cx: &mut Context<Self>) {
        if let Some(screen) = crate::screens::nixa_screen::host::screen(cx) {
            let (open, keep) = {
                let s = screen.read(cx);
                (s.open, s.prefs.keep_open)
            };
            if open {
                if !keep {
                    crate::screens::nixa_screen::host::close(cx);
                }
                return;
            }
        }
        if let Some(id) = self.spaces.active.as_deref().and_then(|space| self.git_diffs.selected(space)) {
            diff_tabs::close(self, id, cx);
        } else if let Some(id) = self.active_tab().map(|t| t.focus.clone()) {
            self.close_term(id, cx);
        }
    }

    pub fn close_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let ids = tabs::find(&self.tabs, tab_id)
            .map(|t| layout::leaves(&t.root))
            .unwrap_or_default();
        for id in ids {
            self.close_term(id, cx);
        }
    }

    pub fn close_term(&mut self, id: TermId, cx: &mut Context<Self>) {
        let space = crate::last_tab::space_of(&self.tabs, &id);
        if self.engine != EngineState::Up {
            self.forget_term(&id);
            cx.notify();
            return;
        }
        let task = cx.background_executor().spawn({
            let id = id.clone();
            async move { engine::remove(id) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |ws, cx| {
                match result {
                    Ok(()) => {
                        ws.forget_term(&id);
                        crate::last_tab::refill(ws, space, cx);
                    }
                    Err(e) => ws.notice = Some(format!("Cannot close terminal {id}: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn forget_term(&mut self, id: &TermId) {
        self.drop_leaf(id);
        self.terms.remove(id);
        self.drop_view(id);
        self.persist_layouts();
    }

    fn drop_view(&mut self, id: &TermId) {
        self.views.remove(id);
        self.view_events.remove(id);
    }

    fn sync_views(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let terms = &self.terms;
        self.views.retain(|id, _| terms.contains_key(id));
        self.view_events.retain(|id, _| terms.contains_key(id));
        let Some(tab) = self.active_tab() else {
            return;
        };
        let wanted: Vec<TermId> = layout::leaves(&tab.root)
            .into_iter()
            .filter(|id| self.terms.contains_key(id) && !self.views.contains_key(id))
            .collect();
        if wanted.is_empty() {
            return;
        }
        let palette = views::palette(theme(cx));
        for id in wanted {
            let socket = self.socket.clone();
            let view_id = id.clone();
            let view = cx.new(|cx| TermView::new(socket, view_id, window, cx));
            view.update(cx, |view, cx| view.set_palette(palette, cx));
            crate::font_zoom::apply(self.store.as_ref(), &view, cx);
            let events = cx.subscribe(&view, |ws, _, event: &TermViewEvent, cx| {
                ws.on_view_event(event, cx)
            });
            self.views.insert(id.clone(), view);
            self.view_events.insert(id, events);
        }
    }

    fn sync_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some()
            || self.picker.is_some()
            || self.favourites.is_some()
            || self.ctx.is_some()
        {
            return;
        }
        let refocus = std::mem::take(&mut self.refocus);
        if !refocus
            && let Some(now) = window.focused(cx)
            && now != self.focus
            && !self.views.values().any(|v| v.focus_handle(cx) == now)
        {
            return;
        }
        let diff = self.spaces.active.as_deref().and_then(|space| self.git_diffs.content(space));
        let handle = match diff {
            Some((_, focus)) => focus,
            None => {
                let Some(view) = self.active_tab().and_then(|tab| self.views.get(&tab.focus)) else {
                    if refocus {
                        let focus = self.focus.clone();
                        cx.defer_in(window, move |_, window, _| window.focus(&focus));
                    }
                    return;
                };
                view.focus_handle(cx)
            }
        };
        if handle.is_focused(window) {
            return;
        }
        cx.defer_in(window, move |_, window, _| window.focus(&handle));
    }

    fn on_view_event(&mut self, event: &TermViewEvent, cx: &mut Context<Self>) {
        if let TermViewEvent::Problem(text) = event
            && text.starts_with("Cannot save the pasted image")
        {
            self.notice = Some(text.clone());
            cx.notify();
            return;
        }
        let TermViewEvent::Engine(event) = event else {
            return;
        };
        if matches!(
            **event,
            Event::Title { .. } | Event::Exit { .. } | Event::Respawned { .. }
        ) && self.on_event((**event).clone())
        {
            cx.notify();
        }
    }

    pub fn adopt(&mut self, id: TermId, cx: &mut Context<Self>) {
        let Some(space) = self.active_space().cloned() else {
            return;
        };
        crate::screens::first_run::host::close(cx);
        self.place_in(id, &space.id, true, cx);
    }

    fn adopt_labelled(&mut self, cx: &mut Context<Self>) -> bool {
        let wanted: Vec<(TermId, String)> = self
            .loose()
            .into_iter()
            .filter_map(|t| {
                let space = t.labels.get(tabs::LABEL_BRAIN_SPACE)?;
                spaces::find(&self.spaces, space)?;
                Some((t.id.clone(), space.clone()))
            })
            .collect();
        let any = !wanted.is_empty();
        for (id, space) in wanted {
            self.place_in(id, &space, false, cx);
        }
        any
    }

    fn place_in(&mut self, id: TermId, space_id: &str, activate: bool, cx: &mut Context<Self>) {
        let tab = tabs::new_id(&mut self.tabs);
        tabs::place(&mut self.tabs, id.clone(), space_id, &tab);
        if activate || !self.tabs.active.contains_key(space_id) {
            self.tabs.active.insert(space_id.to_string(), tab.clone());
        }
        self.persist_layouts();
        let space_id = space_id.to_string();
        let task = cx.background_executor().spawn(async move {
            engine::label(id.clone(), LABEL_SPACE, &space_id)?;
            engine::label(id, LABEL_TAB, &tab)
        });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                this.update(cx, |ws, cx| {
                    ws.notice = Some(format!("Cannot move the terminal into a tab: {e}"));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn with_store(
        &self,
        job: impl FnOnce(&Connection) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        match &self.store {
            Some(conn) => job(conn),
            None => Ok(()),
        }
    }

    fn saved(&mut self, result: anyhow::Result<()>) {
        if let Err(e) = result {
            self.notice = Some(format!("Cannot save spaces: {e:#}"));
        }
    }

    fn save_space(&mut self, id: &str) {
        let result = self.with_store(|conn| match spaces::find(&self.spaces, id) {
            Some(space) => spaces::write_space(conn, space),
            None => Ok(()),
        });
        self.saved(result);
    }

    pub fn select_space(&mut self, id: &str, cx: &mut Context<Self>) {
        crate::screens::first_run::host::close(cx);
        if self.spaces.active.as_deref() != Some(id) {
            self.spaces.active = Some(id.to_string());
            self.panel = crate::screens::tabbar::panels::on_switch(
                self.panel,
                saved_panel(self.store.as_ref(), id),
            );
            let result = self.with_store(|conn| spaces::write_active(conn, &self.spaces));
            self.saved(result);
        }
        cx.notify();
    }

    pub fn rename_space(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        if spaces::rename(&mut self.spaces, id, name) {
            self.save_space(id);
        }
        cx.notify();
    }

    pub fn select_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let Some(space) = tabs::find(&self.tabs, tab_id).map(|t| t.space.clone()) else {
            return;
        };
        self.git_diffs.show_terminals(&space);
        self.tabs.active.insert(space.clone(), tab_id.to_string());
        self.refocus = true;
        self.select_space(&space, cx);
    }

    pub fn step_tab(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(space) = self.active_space().map(|s| s.id.clone()) else {
            return;
        };
        let diffs: Vec<_> = self.git_diffs.ids(&space).collect();
        if diffs.is_empty() {
            tabs::step(&mut self.tabs, &space, step);
        } else {
            let terminals: Vec<_> = self.space_tabs(&space).map(|tab| tab.id.clone()).collect();
            let current = self.git_diffs.selected(&space)
                .and_then(|id| diffs.iter().position(|diff| *diff == id).map(|idx| terminals.len() + idx))
                .or_else(|| self.tabs.active.get(&space).and_then(|id| terminals.iter().position(|tab| tab == id)))
                .unwrap_or(0);
            let count = terminals.len() + diffs.len();
            let next = (current as isize + step).rem_euclid(count as isize) as usize;
            if let Some(id) = terminals.get(next) {
                self.select_tab(id, cx);
            } else {
                diff_tabs::select(self, diffs[next - terminals.len()], cx);
            }
        }
        crate::screens::first_run::host::close(cx);
        cx.notify();
    }

    pub fn focus_pane(&mut self, tab_id: &str, id: &TermId, cx: &mut Context<Self>) {
        if let Some(tab) = tabs::find_mut(&mut self.tabs, tab_id)
            && tab.focus != *id
        {
            tab.focus = id.clone();
        }
        crate::screens::first_run::host::close(cx);
        self.refocus = true;
        cx.notify();
    }

    pub fn zoom_focused(&mut self, cx: &mut Context<Self>) {
        if let Some((tab, id)) = self.active_tab().map(|t| (t.id.clone(), t.focus.clone())) {
            panes::zoom(self, &tab, &id, cx);
        }
    }

    pub fn add_space(&mut self, folder: String, cx: &mut Context<Self>) {
        let result = match &self.store {
            Some(conn) => spaces::add_saved(conn, &mut self.spaces, &folder).map(|_| ()),
            None => {
                spaces::add(&mut self.spaces, &folder);
                Ok(())
            }
        };
        let added = result.is_ok();
        self.saved(result);
        if added {
            crate::screens::first_run::host::space_added(self, cx);
        }
        cx.notify();
    }

    pub fn pick_space_folder(&mut self, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("New space".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = answer.await
                && let Some(path) = paths.first()
            {
                let folder = path.display().to_string();
                this.update(cx, |ws, cx| ws.add_space(folder, cx)).ok();
            }
        })
        .detach();
    }

    pub fn toggle_panel(&mut self, kind: PanelKind, cx: &mut Context<Self>) {
        let next = if self.panel == Some(kind) {
            None
        } else {
            Some(kind)
        };
        self.set_panel(next, cx);
    }

    pub fn set_panel(&mut self, kind: Option<PanelKind>, cx: &mut Context<Self>) {
        let before = std::mem::replace(&mut self.panel, kind);
        let space = self.spaces.active.clone();
        let result = self.with_store(|conn| {
            crate::screens::tabbar::panels::save(conn, space.as_deref(), before, kind)
        });
        if let Err(e) = result {
            self.notice = Some(format!("Cannot save the open panel: {e:#}"));
        }
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        (self.sidebar_w, self.sidebar_open_w) = toggled(self.sidebar_w, self.sidebar_open_w);
        cx.notify();
    }

    pub fn menu_command(
        &mut self,
        command: crate::screens::menu::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::screens::menu::{Command, SETTINGS_PART, Tab};
        if command == Command::Open(Tab::Settings) {
            self.open_settings(SETTINGS_PART, cx);
        } else if let Some(host) = crate::screens::menu::run::run_command(self, command, window, cx) {
            let unread = crate::screens::events::unseen(&self.hub.events, cx);
            host.view.update(cx, |view, cx| {
                view.unread = unread;
                cx.notify();
            });
            self.menu = Some(host);
        }
        cx.notify();
    }

    pub fn menu_key(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(command) = crate::screens::menu::key_command(id) {
            self.menu_command(command, window, cx);
        }
    }

    pub fn open_settings(&mut self, part: &str, cx: &mut Context<Self>) {
        settings::open(part, cx);
    }

    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(cx);
        crate::screens::tabbar::close_menu(self, cx);
        let bar: Vec<usize> = self.tab_bar.read(cx).quick.iter().map(|b| b.index).collect();
        let quick = if bar.is_empty() { commands::installed_now() } else { bar };
        let palette: Palette = commands::open_with(Some(quick), window, cx);
        self.palette = Some(palette);
        cx.notify();
    }

    pub fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn close_overlays(&mut self, cx: &mut Context<Self>) {
        self.palette = None;
        self.picker = None;
        self.favourites = None;
        self.ctx = None;
        cx.notify();
    }

    pub fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(cx);
        crate::screens::tabbar::close_menu(self, cx);
        let snap = picker::data::snapshot(self, cx);
        let view = picker::view::new(snap, window, cx);
        let events = cx.subscribe_in(&view, window, |ws, _, event: &PickerEvent, window, cx| {
            ws.close_picker(window, cx);
            picker::data::apply(ws, event, cx);
        });
        self.picker = Some((view, events));
        cx.notify();
    }

    pub fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = None;
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn open_favourites(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(cx);
        crate::screens::tabbar::close_menu(self, cx);
        let folder = favourites::data::active_folder(self);
        let view = favourites::view::new(folder, favourites::store::default_file(), window, cx);
        let events = cx.subscribe_in(
            &view,
            window,
            |ws, _, event: &FavouritesEvent, window, cx| {
                if *event == FavouritesEvent::Close {
                    ws.favourites = None;
                    window.focus(&ws.focus);
                }
                favourites::data::apply(ws, event, cx);
            },
        );
        self.favourites = Some((view, events));
        cx.notify();
    }

    pub fn open_ctx(
        &mut self,
        target: crate::screens::ctxmenu::Target,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_overlays(cx);
        crate::screens::tabbar::close_menu(self, cx);
        crate::screens::ctxmenu::host::open(self, target, at, window, cx);
    }

    pub fn show(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(done) = crate::demo::show(self, what, window, cx) {
            if !done {
                self.show_at_start = Some(what.to_string());
            }
            return;
        }
        match what {
            "picker" => self.open_picker(window, cx),
            "favourites" => self.open_favourites(window, cx),
            "commands" => self.open_palette(window, cx),
            "menu" => self.menu_key("", window, cx),
            "files" => self.set_panel(Some(PanelKind::Files), cx),
            "git" => self.set_panel(Some(PanelKind::Git), cx),
            "nixa" => self.set_panel(Some(PanelKind::Nixa), cx),
            "git:diff" => {
                self.set_panel(Some(PanelKind::Git), cx);
                let view = self.changes.clone();
                crate::screens::git::changes::show_first_diff(&view, cx);
            }
            "custom_agents" => crate::screens::custom_agents::open(cx),
            first if crate::screens::first_run::host::show(first, cx) => {}
            desk if crate::screens::desk_panels::show(desk, cx) => {}
            "models" | "models-view" => {
                crate::screens::models::show(what, cx);
            }
            show if show.starts_with("workflows") => crate::screens::workflows::show(show, cx),
            show if show == "assistant" || show.starts_with("assistant:") => crate::screens::assistant::show(show, cx),
            what if what == "tasks" || what.starts_with("tasks:") => {
                crate::screens::tasks::show(what, cx)
            }
            "nixa-screen" => crate::screens::nixa_screen::host::open(self, cx),
            "events" => {
                let view = self.hub.events.events.clone();
                view.update(cx, |v, cx| v.run(events::EventsCommand::Show, cx));
            }
            "actions" => {
                self.open_ctx(crate::screens::ctxmenu::Target::Default, window.mouse_position(), window, cx);
            }
            spec if spec.starts_with("git-ai:") => {
                if let Err(e) = crate::screens::git::ai::show(&spec["git-ai:".len()..], cx) {
                    self.notice = Some(format!("{SHOW_ENV}: {e}"));
                }
            }
            git if crate::screens::git::tools::show(git, crate::screens::git::join::tools_host(), cx) => {}
            git if crate::screens::git::accounts::show(git, self.active_space().map(|s| s.folder.as_str()), cx) => {}
            part if part.starts_with("git-") => {
                let folder = self.active_space().map(|s| s.folder.clone());
                if !crate::screens::git::history::show(part, folder.as_deref(), cx) {
                    if part.split(':').next() == Some("git-commit") {
                        let repo = crate::screens::git::history::repo_root(folder.as_deref());
                        crate::screens::git::history::commit_menu::open_empty(repo, part, cx);
                    } else {
                        self.notice = Some(format!("{SHOW_ENV}: {part} needs a repo folder."));
                    }
                }
            }
            part if part.starts_with("settings:") => {
                let part = part["settings:".len()..].to_string();
                cx.defer(move |cx| settings::open(&part, cx));
            }
            file if crate::screens::editor::show(file, cx) => {}
            _ => self.notice = Some(format!("{SHOW_ENV}: {what} is not a screen it can open.")),
        }
    }

    pub fn step_theme(&mut self, step: isize, cx: &mut Context<Self>) {
        let list = nixa_ui::theme::themes(cx);
        let now = theme(cx).name.to_string();
        if let Some(name) = nixa_ui::theme::next_name(&list, &now, step)
            && let Err(e) = nixa_ui::theme::choose(cx, &name)
        {
            self.notice = Some(format!("Cannot save the theme choice: {e}"));
        }
        cx.notify();
    }

    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        let palette = views::palette(theme(cx));
        for view in self.views.values() {
            view.update(cx, |view, cx| view.set_palette(palette, cx));
        }
        cx.notify();
    }

    fn drag_split(
        &mut self,
        event: &DragMoveEvent<SplitDrag>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = event.drag(cx).id.clone();
        let at = event.event.position;
        let axis = gpui::Axis::Horizontal;
        match id.as_ref() {
            SIDEBAR_SPLIT => {
                self.sidebar_w = size_from_drag(
                    axis,
                    Edge::End,
                    event.bounds,
                    at,
                    SIDEBAR_MIN,
                    SIDEBAR_MAX,
                );
            }
            PANEL_SPLIT => {
                let edge = RAIL_WIDTH + PANE_GAP;
                self.panel_w = size_from_drag(
                    axis,
                    Edge::Start,
                    event.bounds,
                    at,
                    PANEL_MIN + edge,
                    PANEL_MAX + edge,
                ) - edge;
            }
            _ => return,
        }
        cx.notify();
    }
}

fn then_tab(then: &Then) -> &str {
    match then {
        Then::NewTab { tab, .. } | Then::Split { tab, .. } => tab,
    }
}

pub fn saved_panel(store: Option<&Connection>, space: &str) -> Option<PanelKind> {
    let value =
        nixa_store::settings::get_setting(store?, &format!("{PANEL_KEY}{space}")).ok()??;
    panel::from_word(value.as_str()?)
}

pub fn toggled(sidebar_w: f32, open_w: f32) -> (f32, f32) {
    if sidebar_w < SIDEBAR_NARROW {
        (open_w.max(SIDEBAR_NARROW), open_w)
    } else {
        (SIDEBAR_MIN, sidebar_w)
    }
}

pub fn write_widths(conn: &Connection, widths: (f32, f32, f32)) -> anyhow::Result<()> {
    nixa_store::settings::set_setting(conn, SIDEBAR_WIDTH_KEY, &serde_json::json!(widths.0))?;
    nixa_store::settings::set_setting(conn, PANEL_WIDTH_KEY, &serde_json::json!(widths.1))?;
    nixa_store::settings::set_setting(conn, SIDEBAR_OPEN_WIDTH_KEY, &serde_json::json!(widths.2))
}

pub fn saved_widths(store: Option<&Connection>) -> (f32, f32, f32) {
    let read = |key: &str, default: f32, min: f32, max: f32| {
        store
            .and_then(|conn| nixa_store::settings::get_setting(conn, key).ok().flatten())
            .and_then(|v| v.as_f64())
            .map(|v| (v as f32).clamp(min, max))
            .unwrap_or(default)
    };
    let sidebar = read(SIDEBAR_WIDTH_KEY, SIDEBAR_WIDTH, SIDEBAR_MIN, SIDEBAR_MAX);
    (
        sidebar,
        read(PANEL_WIDTH_KEY, PANEL_WIDTH, PANEL_MIN, PANEL_MAX),
        read(SIDEBAR_OPEN_WIDTH_KEY, sidebar.max(SIDEBAR_NARROW), SIDEBAR_NARROW, SIDEBAR_MAX),
    )
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((target, at)) = self.ctx_wanted.take() {
            self.open_ctx(target, at, window, cx);
        }
        if self.engine == EngineState::Up
            && let Some(what) = self.show_at_start.take()
        {
            self.show(&what, window, cx);
        }
        self.git_diffs.retain_spaces(&self.spaces);
        self.sync_views(window, cx);
        self.sync_focus(window, cx);
        hub::sync(self, cx);
        let spaces_input = crate::screens::spaces::input(self, window);
        crate::screens::spaces::sync(&self.spaces_view, spaces_input, cx);
        crate::screens::first_run::host::sync(self, cx);
        let t = theme(cx).clone();
        let events_open = events::is_open(&self.hub.events, cx);
        let popup = self.hub.events.events.clone();
        let toasts = self.hub.events.toasts.clone();
        // Layout: icon rail, work with its panel on the left, then the sidebar.
        let main = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_row()
            .p(px(PANE_GAP))
            .gap(px(PANE_GAP))
            .bg(t.canvas)
            .when_some(self.panel, |el, kind| {
                el.child(panel::render(self, kind, cx))
            })
            .child(
                div()
                    .debug_selector(|| "work-area".into())
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .children(diff_tabs::strip(self, &t, cx))
                    .child(div().flex_1().min_h_0().child(
                        if let Some((diff, focus)) = self.spaces.active.as_deref().and_then(|space| self.git_diffs.content(space)) {
                            div().id("git-diff-work-area").track_focus(&focus).size_full().child(diff).into_any_element()
                        } else {
                            crate::screens::first_run::host::render(self, cx)
                                .unwrap_or_else(|| panes::render(self, cx))
                        },
                    )),
            );
        div()
            .id("nixa-root")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .bg(t.canvas)
            .text_color(t.foreground)
            .text_size(px(13.))
            .on_key_down(cx.listener(|ws, event: &gpui::KeyDownEvent, window, cx| {
                let key = &event.keystroke;
                let modifiers = key.modifiers;
                if key.key == "tab" && !modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.function
                    && !ws.views.values().any(|view| view.focus_handle(cx).is_focused(window))
                {
                    if modifiers.shift {
                        window.focus_prev();
                    } else {
                        window.focus_next();
                    }
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|ws, _: &NewTab, _, cx| ws.new_tab(Vec::new(), None, cx)))
            .on_action(cx.listener(|ws, _: &SplitRight, _, cx| ws.split(Axis::Row, cx)))
            .on_action(cx.listener(|ws, _: &SplitDown, _, cx| ws.split(Axis::Column, cx)))
            .on_action(cx.listener(|ws, _: &ClosePane, _, cx| ws.close_pane(cx)))
            .on_action(cx.listener(|ws, _: &ToggleSidebar, _, cx| ws.toggle_sidebar(cx)))
            .on_action(cx.listener(|ws, _: &NextTab, _, cx| ws.step_tab(1, cx)))
            .on_action(cx.listener(|ws, _: &PrevTab, _, cx| ws.step_tab(-1, cx)))
            .on_action(cx.listener(|ws, _: &ZoomPane, _, cx| ws.zoom_focused(cx)))
            .on_action(cx.listener(|ws, _: &NextAgent, _, cx| {
                let view = ws.hub.agents.clone();
                crate::screens::agents::step_agent(ws, &view, 1, cx)
            }))
            .on_action(cx.listener(|ws, _: &PrevAgent, _, cx| {
                let view = ws.hub.agents.clone();
                crate::screens::agents::step_agent(ws, &view, -1, cx)
            }))
            .on_action(cx.listener(|ws, _: &OpenPicker, window, cx| {
                if ws.picker.is_some() {
                    ws.close_picker(window, cx)
                } else {
                    ws.open_picker(window, cx)
                }
            }))
            .on_action(
                cx.listener(|ws, _: &OpenFavourites, window, cx| ws.open_favourites(window, cx)),
            )
            .on_action(cx.listener(|ws, _: &NewAgentMenu, window, cx| {
                crate::screens::tabbar::run(ws, crate::screens::tabbar::Action::NewMenu, window, cx)
            }))
            .on_action(cx.listener(|ws, _: &NixaScreen, _, cx| {
                crate::screens::nixa_screen::host::toggle(ws, cx)
            }))
            .on_action(cx.listener(|ws, _: &crate::actions::OpenMenu, window, cx| {
                ws.menu_key("", window, cx)
            }))
            .on_action(
                cx.listener(|ws, _: &crate::actions::MenuTasks, window, cx| {
                    ws.menu_key("tasks", window, cx)
                }),
            )
            .on_action(
                cx.listener(|ws, _: &crate::actions::MenuProjectCommands, window, cx| {
                    ws.menu_key("projectcommands", window, cx)
                }),
            )
            .on_action(
                cx.listener(|ws, _: &crate::actions::MenuFavourites, window, cx| {
                    ws.menu_key("favourites", window, cx)
                }),
            )
            .on_action(cx.listener(|ws, _: &OpenCommands, window, cx| {
                if ws.palette.is_some() {
                    ws.close_palette(window, cx);
                } else {
                    ws.open_palette(window, cx);
                }
            }))
            .on_drag_move(cx.listener(Self::drag_split))
            .child(rail::render(self, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(tabbar::render(self, window, cx))
                    .child(main)
                    .child(statusbar::render(self, cx)),
            )
            .child(sidebar::render(self, cx))
            .child(toasts)
            .when(events_open, |el| {
                el.child(
                    deferred(
                        div()
                            .absolute()
                            .top(px(44.))
                            .right(px(self.sidebar_w + 8.))
                            .child(popup),
                    )
                    .with_priority(2),
                )
            })
            .when(self.palette.is_some(), |el| {
                el.child(commands::render(self, cx))
            })
            .when_some(self.picker.as_ref().map(|p| p.0.clone()), |el, view| {
                el.child(view)
            })
            .when_some(self.favourites.as_ref().map(|f| f.0.clone()), |el, view| {
                el.child(view)
            })
            .children(crate::screens::menu::run::overlay(self.menu.as_ref(), cx))
            .when_some(self.ctx.as_ref().map(|c| c.view()), |el, view| {
                el.child(view)
            })
            .children(crate::screens::nixa_screen::host::overlay(cx))
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    #[test]
    fn the_open_sidebar_width_comes_back_after_a_restart_while_closed() {
        let conn = nixa_store::open_memory().unwrap();
        let (sidebar_w, open_w) = toggled(400.0, 400.0);
        assert_eq!((sidebar_w, open_w), (SIDEBAR_MIN, 400.0));
        write_widths(&conn, (sidebar_w, PANEL_WIDTH, open_w)).unwrap();
        let (sidebar_w, _, open_w) = saved_widths(Some(&conn));
        assert_eq!(sidebar_w, SIDEBAR_MIN);
        assert_eq!(toggled(sidebar_w, open_w).0, 400.0);
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn the_rail_and_panel_sit_left_and_the_sidebar_right(cx: &mut TestAppContext) {
        let dir = std::path::PathBuf::from(format!("/tmp/mo-lay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        unsafe {
            std::env::set_var(paths::ENV_DATA_DIR, &dir);
            std::env::set_var(paths::ENV_ENGINE_SOCKET, dir.join("e.sock"));
            std::env::set_var(nixa_protocol::control::ENV_APP_SOCKET, dir.join("a.sock"));
        }
        cx.update(nixa_ui::init);
        let (ws, vcx) = cx.add_window_view(Workspace::new);
        vcx.simulate_resize(gpui::size(px(1280.), px(800.)));
        ws.update(vcx, |ws, cx| {
            ws.engine = EngineState::Up;
            let mut list = Spaces::default();
            spaces::add(&mut list, "/tmp/shop");
            ws.spaces = list;
            ws.panel = Some(PanelKind::Files);
            cx.notify();
        });
        vcx.run_until_parked();
        vcx.update(|window, app| window.draw(app).clear());
        let mut at = |name: &'static str| vcx.debug_bounds(name).unwrap_or_else(|| panic!("{name} is not drawn"));
        let rail = at("nixa-rail-settings");
        let bar = at("tab-bar");
        let panel = at("panel");
        let work = at("work-area");
        let side = at("sidebar");
        assert!(rail.right() <= bar.left());
        assert!(rail.bottom() <= px(800.));
        assert!(panel.right() <= work.left());
        assert!(work.right() <= side.left());
        assert!(side.right() >= px(1279.));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui::test]
    fn the_ai_commit_message_button_does_not_read_the_busy_workspace(cx: &mut TestAppContext) {
        let dir = std::path::PathBuf::from(format!("/tmp/mo-sug-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        unsafe {
            std::env::set_var(paths::ENV_DATA_DIR, &dir);
            std::env::set_var(paths::ENV_ENGINE_SOCKET, dir.join("e.sock"));
            std::env::set_var(nixa_protocol::control::ENV_APP_SOCKET, dir.join("a.sock"));
        }
        cx.update(nixa_ui::init);
        let (ws, vcx) = cx.add_window_view(Workspace::new);
        let event = crate::screens::git::changes::ChangesEvent::Suggest { repo: dir.display().to_string(), branch: None };
        ws.update(vcx, |ws, cx| crate::screens::git::changes::host::on_event(ws, &event, cx));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
