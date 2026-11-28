pub mod apply;
mod header;
mod input;
pub mod model;
mod peek;
pub mod rename;
mod rows;
mod store;
#[cfg(test)]
mod tests;

pub use apply::{on_event, rename_tab, send_back, step_agent};

use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, EventEmitter, Pixels, Point, Window, div, px};
use nixa_protocol::TermId;
use nixa_store::Connection;
use nixa_termview::TermView;
use nixa_ui::theme;
use model::{Held, Input, Scope, Sort, Stamp, View};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TICK: Duration = Duration::from_secs(30);
pub const PIN_WAIT_MS: u64 = 5_000;

#[derive(Clone, Debug, PartialEq)]
pub enum AgentsEvent {
    Focus {
        tab: String,
        term: Option<TermId>,
    },
    CloseTab {
        tab: String,
    },
    CloseTerm {
        term: TermId,
    },
    Pin {
        term: TermId,
        on: bool,
    },
    Menu {
        tab: Option<String>,
        space: Option<String>,
        term: Option<TermId>,
        at: Point<Pixels>,
    },
    Rename {
        tab: String,
    },
    KeepPreview {
        tab: String,
    },
    SetName {
        term: TermId,
        name: String,
    },
    PopOut {
        term: TermId,
    },
    Adopt {
        term: TermId,
    },
    PutAway {
        tab: String,
        next: Option<String>,
    },
    ToggleSidebar,
    Notice(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SortPriority,
    SortGrouped,
    ScopeSpace,
    ScopeAll,
    NextNeedy,
    NextAgent,
    PrevAgent,
    PinFocused,
    PutAwayFocused,
    BringBackAll,
}

pub struct AgentsView {
    scroll: gpui::ScrollHandle,
    pub input: Input,
    pub view: View,
    pub sort: Sort,
    pub scope: Scope,
    pub changed: BTreeMap<String, Stamp>,
    pub away: BTreeSet<String>,
    pub folded: BTreeSet<String>,
    pub hovered: Option<String>,
    pub held: Option<Held>,
    pub pins: BTreeMap<TermId, (bool, u64)>,
    pub sort_menu: Option<Point<Pixels>>,
    pub away_menu: Option<Point<Pixels>>,
    pub peek: Option<TermId>,
    peek_view: Option<(TermId, Entity<TermView>)>,
    pub renaming: Option<rename::Renaming>,
    pub label_jobs: VecDeque<apply::LabelJob>,
    pub label_busy: bool,
    store: Option<Connection>,
    _keys: gpui::Subscription,
}

impl EventEmitter<AgentsEvent> for AgentsView {}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn build(store: Option<Connection>, cx: &mut Context<AgentsView>) -> AgentsView {
    let saved = store::load(store.as_ref());
    if !cfg!(test) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    let keys = cx.observe_keystrokes(|view, event, _, cx| {
        if event.keystroke.key == "escape" && view.sort_menu.take().is_some() {
            cx.notify();
        }
    });
    AgentsView {
        scroll: gpui::ScrollHandle::new(),
        input: Input::default(),
        view: View::default(),
        sort: saved.sort,
        scope: saved.scope,
        changed: saved.changed,
        away: saved.away,
        folded: BTreeSet::new(),
        hovered: None,
        held: None,
        pins: BTreeMap::new(),
        sort_menu: None,
        away_menu: None,
        peek: None,
        peek_view: None,
        renaming: None,
        label_jobs: VecDeque::new(),
        label_busy: false,
        store,
        _keys: keys,
    }
}

pub fn new(cx: &mut App) -> Entity<AgentsView> {
    let store = if cfg!(test) {
        None
    } else {
        nixa_store::open_default().ok()
    };
    cx.new(|cx| build(store, cx))
}

pub fn render(view: &Entity<AgentsView>) -> AnyElement {
    view.clone().into_any_element()
}

pub fn sync(view: &Entity<AgentsView>, ws: &Workspace, cx: &mut App) {
    let now = now_ms();
    let (away, pins) = {
        let v = view.read(cx);
        let pins = settle_pins(&v.pins, |term| ws.terms.get(term).map(input::pinned), now);
        (v.away.clone(), pins)
    };
    let shown: BTreeMap<TermId, bool> = pins.iter().map(|(t, (on, _))| (t.clone(), *on)).collect();
    let mut input = input::build(ws, &away, &shown);
    let preview = ws.files.read(cx).previews.preview.as_ref().map(|p| p.tab.clone());
    mark_previews(&mut input, preview.as_deref());
    view.update(cx, |v, cx| {
        v.pins = pins;
        set_input(v, input, cx);
    });
}

pub fn mark_previews(input: &mut Input, preview: Option<&str>) {
    for o in input.others.iter_mut() {
        o.preview = preview == Some(o.tab.as_str());
    }
}

pub fn settle_pins(
    pins: &BTreeMap<TermId, (bool, u64)>,
    real: impl Fn(&TermId) -> Option<bool>,
    now: u64,
) -> BTreeMap<TermId, (bool, u64)> {
    pins.iter()
        .filter(|(term, (on, at))| {
            real(term).is_some_and(|r| r != *on) && now.saturating_sub(*at) < PIN_WAIT_MS
        })
        .map(|(t, p)| (t.clone(), *p))
        .collect()
}

pub fn set_input(v: &mut AgentsView, input: Input, cx: &mut Context<AgentsView>) {
    if v.input == input {
        return;
    }
    if input.ready {
        let before = v.away.len();
        v.away
            .retain(|tab| input.tabs.iter().any(|(t, _)| t == tab));
        if v.away.len() != before {
            save_away(v, cx);
        }
    }
    if model::track(&mut v.changed, &input.agents, now_ms())
        && let Err(e) = store::save_changed(v.store.as_ref(), &v.changed)
    {
        cx.emit(AgentsEvent::Notice(format!(
            "Cannot save agent times: {e:#}"
        )));
    }
    v.held = model::hold(v.held.take(), &input.agents);
    if let Some(peek) = &v.peek
        && !input.kids.iter().any(|k| &k.term == peek && k.running)
    {
        v.peek = None;
    }
    let still = v.hovered.as_deref().is_some_and(|key| {
        input.agents.iter().any(|a| a.term.to_string() == key)
            || input.kids.iter().any(|k| format!("sub-{}", k.term) == key)
            || input.others.iter().any(|o| format!("tab-{}", o.tab) == key)
            || input.loose.iter().any(|l| format!("loose-{}", l.term) == key)
    });
    if !still {
        v.hovered = None;
    }
    v.input = input;
    refresh(v);
    cx.notify();
}

pub fn refresh(v: &mut AgentsView) {
    v.view = model::build(&v.input, v.sort, v.scope, v.held.as_ref(), &v.changed);
}

fn save_away(v: &AgentsView, cx: &mut Context<AgentsView>) {
    if let Err(e) = store::save_away(v.store.as_ref(), &v.away) {
        cx.emit(AgentsEvent::Notice(format!(
            "Cannot save the put away tabs: {e:#}"
        )));
    }
}

pub fn set_sort(v: &mut AgentsView, sort: Sort, cx: &mut Context<AgentsView>) {
    v.sort = sort;
    v.sort_menu = None;
    if let Err(e) = store::save_sort(v.store.as_ref(), sort) {
        cx.emit(AgentsEvent::Notice(format!("Cannot save the sort: {e:#}")));
    }
    refresh(v);
    cx.notify();
}

pub fn set_scope(v: &mut AgentsView, scope: Scope, cx: &mut Context<AgentsView>) {
    v.scope = scope;
    if let Err(e) = store::save_scope(v.store.as_ref(), scope) {
        cx.emit(AgentsEvent::Notice(format!(
            "Cannot save the agents scope: {e:#}"
        )));
    }
    refresh(v);
    cx.notify();
}

pub fn toggle_fold(v: &mut AgentsView, id: &str, cx: &mut Context<AgentsView>) {
    if !v.folded.remove(id) {
        v.folded.insert(id.to_string());
    }
    cx.notify();
}

pub fn pin(v: &mut AgentsView, term: &TermId, on: bool, cx: &mut Context<AgentsView>) {
    v.pins.insert(term.clone(), (on, now_ms()));
    v.hovered = None;
    for a in v.input.agents.iter_mut().filter(|a| &a.term == term) {
        a.pinned = on;
    }
    refresh(v);
    cx.emit(AgentsEvent::Pin {
        term: term.clone(),
        on,
    });
    cx.notify();
}

pub fn next_tab(v: &AgentsView, tab: &str) -> Option<String> {
    let tabs = &v.input.tabs;
    let at = tabs.iter().position(|(t, _)| t == tab)?;
    let space = &tabs[at].1;
    let usable = |(t, s): &&(String, String)| s == space && t != tab && !v.away.contains(t);
    tabs[at + 1..]
        .iter()
        .find(usable)
        .or_else(|| tabs[..at].iter().rev().find(usable))
        .map(|(t, _)| t.clone())
}

pub fn put_away(v: &mut AgentsView, tab: &str, cx: &mut Context<AgentsView>) {
    if !v.input.tabs.iter().any(|(t, _)| t == tab) || !v.away.insert(tab.to_string()) {
        return;
    }
    save_away(v, cx);
    cx.emit(AgentsEvent::PutAway {
        tab: tab.to_string(),
        next: next_tab(v, tab),
    });
    cx.notify();
}

pub fn bring_back(v: &mut AgentsView, tab: &str, cx: &mut Context<AgentsView>) {
    v.away_menu = None;
    if !v.away.remove(tab) {
        return;
    }
    save_away(v, cx);
    cx.emit(AgentsEvent::Focus {
        tab: tab.to_string(),
        term: None,
    });
    cx.notify();
}

pub fn focus(target: Option<(String, TermId)>, cx: &mut Context<AgentsView>) {
    if let Some((tab, term)) = target {
        cx.emit(AgentsEvent::Focus {
            tab,
            term: Some(term),
        });
    }
}

pub fn focused(v: &AgentsView) -> Option<&model::Agent> {
    v.input.agents.iter().find(|a| a.focused)
}

pub fn commands() -> Vec<(&'static str, Action)> {
    vec![
        ("Sort agents by priority", Action::SortPriority),
        ("Sort agents grouped by space", Action::SortGrouped),
        ("Show agents in this space", Action::ScopeSpace),
        ("Show agents in all spaces", Action::ScopeAll),
        ("Go to the next agent that needs you", Action::NextNeedy),
        ("Next agent", Action::NextAgent),
        ("Previous agent", Action::PrevAgent),
        ("Pin or unpin this agent", Action::PinFocused),
        ("Put this tab away", Action::PutAwayFocused),
        ("Bring back the put away tabs", Action::BringBackAll),
    ]
}

pub fn run(view: &Entity<AgentsView>, action: Action, cx: &mut App) {
    view.update(cx, |v, cx| match action {
        Action::SortPriority => set_sort(v, Sort::Priority, cx),
        Action::SortGrouped => set_sort(v, Sort::Grouped, cx),
        Action::ScopeSpace => set_scope(v, Scope::Space, cx),
        Action::ScopeAll => set_scope(v, Scope::All, cx),
        Action::NextNeedy => focus(model::next_needy(&v.input), cx),
        Action::NextAgent | Action::PrevAgent => {
            let step = if action == Action::NextAgent { 1 } else { -1 };
            let now = focused(v).map(|a| a.term.clone());
            focus(model::step(&v.view.order, now.as_ref(), step), cx);
        }
        Action::PinFocused => {
            if let Some((term, on)) = focused(v).map(|a| (a.term.clone(), !a.pinned)) {
                pin(v, &term, on, cx);
            }
        }
        Action::PutAwayFocused => {
            let space = v.input.active_space.clone();
            let tab = focused(v)
                .filter(|a| Some(&a.space) == space.as_ref())
                .map(|a| a.tab.clone())
                .or_else(|| {
                    v.input
                        .others
                        .iter()
                        .find(|o| o.active)
                        .map(|o| o.tab.clone())
                });
            if let Some(tab) = tab {
                put_away(v, &tab, cx);
            }
        }
        Action::BringBackAll => {
            let tabs: Vec<String> = v.input.away.iter().map(|a| a.tab.clone()).collect();
            for tab in tabs {
                bring_back(v, &tab, cx);
            }
        }
    });
}

impl gpui::Render for AgentsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let narrow = self.input.narrow;
        let now = now_ms();
        rename::ensure_input(self, window, cx);
        let rows = rows::render(self, &t, now, cx);
        let empty = rows.is_empty();
        let menus = header::menus(self, &t, cx);
        let peek = peek::render(self, &t, window, cx);
        let renaming = rename::render(self, &t, window, cx);
        div()
            .id("agents")
            .w_full()
            .flex_1()
            .min_h(px(90.))
            .flex()
            .flex_col()
            .bg(t.chrome)
            .border_t_1()
            .border_color(t.border)
            .text_size(px(13.))
            .when(!narrow, |el| el.child(header::render(self, &t, cx)))
            .when(empty && !narrow, |el| {
                el.child(
                    div()
                        .px(px(12.))
                        .text_size(px(12.))
                        .text_color(t.dim)
                        .child("No agents yet. Agents show up here when they start or report their status. Press ⌘T, or pick an agent in the top bar."),
                )
            })
            .child(
                nixa_ui::scroll::scroll_area(&t, "agents-list", &self.scroll, div()
                    .px(px(if narrow { 6. } else { 8. }))
                    .when(narrow, |el| el.pt(px(6.)))
                    .pb(px(10.))
                    .flex()
                    .flex_col()
                    .children(rows),
                ).flex_1(),
            )
            .when(narrow, |el| el.child(rows::collapse(&t, cx)))
            .children(menus)
            .children(peek)
            .children(renaming)
    }
}
