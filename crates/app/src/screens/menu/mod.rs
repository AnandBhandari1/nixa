pub mod footer;
pub mod items;
pub mod nav;
pub mod projects;
pub mod run;
pub mod runner;
pub mod switches;
#[cfg(test)]
mod tests;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    MouseButton, ScrollHandle, SharedString, Window, div, px,
};
use nixa_ui::metrics::RADIUS_POPOVER;
use nixa_ui::popover::{shadow, tooltip};
use nixa_ui::{Theme, icon, theme};
use nav::{Nav, Outcome, View};
use projects::Project;

pub use items::{Action, Item, TABS, Tab};

pub const WIDTH: f32 = 560.;
pub const HEIGHT: f32 = 440.;
pub const HINT: &str = "↑↓ move · enter run · tab / ← → switch tab · esc close";
pub const SETTINGS_PART: &str = "General";

#[derive(Clone, Debug, PartialEq)]
pub enum MenuEvent {
    Run(Action),
    OpenSettings(&'static str),
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Open(Tab),
    Run(Action),
}

pub struct MenuView {
    pub nav: Nav,
    pub projects: Option<Vec<Project>>,
    pub unread: usize,
    pub open: bool,
    pub scroll: ScrollHandle,
    focus: FocusHandle,
}

impl EventEmitter<MenuEvent> for MenuView {}

impl Focusable for MenuView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn build(tab: Tab, cx: &mut Context<MenuView>) -> MenuView {
    let tab = if tab == Tab::Settings {
        Tab::Quick
    } else {
        tab
    };
    MenuView {
        nav: Nav::new(tab),
        projects: None,
        unread: 0,
        open: true,
        scroll: ScrollHandle::new(),
        focus: cx.focus_handle(),
    }
}

pub fn new(tab: Tab, cx: &mut App) -> Entity<MenuView> {
    cx.new(|cx| build(tab, cx))
}

pub fn commands() -> Vec<(&'static str, Command)> {
    vec![
        ("Quick actions menu", Command::Open(Tab::Quick)),
        ("Menu: Create", Command::Open(Tab::Create)),
        ("Menu: Layout", Command::Open(Tab::Layout)),
        ("Menu: Workspace", Command::Open(Tab::Workspace)),
        ("Menu: Settings", Command::Open(Tab::Settings)),
        ("Tasks in a popup terminal", Command::Run(Action::Tasks)),
        ("Quick capture a task", Command::Run(Action::QuickCapture)),
        ("Task board in a new tab", Command::Run(Action::Board)),
        ("Project commands", Command::Run(Action::ProjectCommands)),
        ("Use my layout", Command::Run(Action::MyLayout)),
    ]
}

pub fn key_command(id: &str) -> Option<Command> {
    if id.is_empty() {
        return Some(Command::Open(Tab::Quick));
    }
    items::quick_action(id).map(Command::Run)
}

pub fn rows(view: &MenuView) -> Vec<Item> {
    match view.nav.view {
        View::Projects => items::projects(view.projects.as_deref()),
        View::Menu => items::for_tab(view.nav.tab, view.unread),
    }
}

fn load_projects(cx: &mut Context<MenuView>) {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let data = nixa_protocol::paths::data_dir();
    let task = cx
        .background_executor()
        .spawn(async move { projects::load(&home, &data) });
    cx.spawn(async move |this, cx| {
        let list = task.await;
        this.update(cx, |view, cx| {
            view.projects = Some(list);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn choose(view: &mut MenuView, index: usize, cx: &mut Context<MenuView>) {
    let Some(item) = rows(view).into_iter().nth(index) else {
        return;
    };
    if item.id == "loading" {
        return;
    }
    match item.action {
        Action::Projects => {
            view.nav.view = View::Projects;
            view.nav.at = 0;
            if view.projects.is_none() {
                load_projects(cx);
            }
        }
        Action::Back => {
            view.nav.view = View::Menu;
            view.nav.at = 0;
        }
        action => cx.emit(MenuEvent::Run(action)),
    }
    cx.notify();
}

fn settle(view: &mut MenuView, outcome: Outcome, cx: &mut Context<MenuView>) {
    match outcome {
        Outcome::Moved => {
            view.scroll.scroll_to_item(view.nav.at);
            cx.notify();
        }
        Outcome::Choose(i) => choose(view, i, cx),
        Outcome::Close => cx.emit(MenuEvent::Close),
        Outcome::OpenSettings => cx.emit(MenuEvent::OpenSettings(SETTINGS_PART)),
        Outcome::Ignore => {}
    }
}

fn on_key(
    view: &mut MenuView,
    event: &KeyDownEvent,
    _window: &mut Window,
    cx: &mut Context<MenuView>,
) {
    let count = rows(view).iter().filter(|item| item.id != "loading").count();
    let key = event.keystroke.key.as_str();
    let outcome = nav::key(&mut view.nav, key, event.keystroke.modifiers.shift, count);
    if outcome != Outcome::Ignore {
        cx.stop_propagation();
    }
    settle(view, outcome, cx);
}

fn tab_button(t: &Theme, tab: Tab, on: bool, cx: &mut Context<MenuView>) -> AnyElement {
    let hover = t.hover;
    div()
        .id(("menu-tab", tab.index()))
        .debug_selector(move || format!("menu-tab-{}", tab.index()))
        .h(px(24.))
        .px(px(9.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .cursor_pointer()
        .tooltip(tooltip(format!("{} (Tab / ← →)", tab.title())))
        .map(|el| {
            if on {
                el.bg(t.selected).text_color(t.foreground)
            } else {
                el.text_color(t.muted).hover(move |s| s.bg(hover))
            }
        })
        .on_click(cx.listener(move |view, _, _, cx| {
            let outcome = view.nav.pick_tab(tab);
            settle(view, outcome, cx);
        }))
        .child(tab.title())
        .into_any_element()
}

fn row(t: &Theme, item: &Item, index: usize, on: bool, enabled: bool, cx: &mut Context<MenuView>) -> AnyElement {
    let hover = t.hover;
    div()
        .id(("menu-row", index))
        .debug_selector(move || format!("menu-row-{index}"))
        .h(px(31.))
        .px(px(8.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(9.))
        .rounded(px(7.))
        .when(enabled, |el| el.cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |view, _, _, cx| {
                view.nav.at = index;
                choose(view, index, cx);
            })))
        .when(!item.tip.is_empty(), |el| {
            el.tooltip(tooltip(item.tip.clone()))
        })
        .when(on && enabled, |el| el.bg(t.selected))
        .child(
            icon(item.icon)
                .size(px(14.))
                .flex_none()
                .text_color(t.muted),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.foreground)
                .child(SharedString::from(item.label.clone())),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(t.dim)
                .child(SharedString::from(item.hint.clone())),
        )
        .into_any_element()
}

impl Render for MenuView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        if !self.open {
            return div().id("nixa-menu-closed").into_any_element();
        }
        let list = rows(self);
        let at = self.nav.at.min(list.len().saturating_sub(1));
        let tabs: Vec<AnyElement> = TABS
            .into_iter()
            .map(|tab| tab_button(&t, tab, tab == self.nav.tab, cx))
            .collect();
        let rows: Vec<AnyElement> = list
            .iter()
            .enumerate()
            .map(|(i, item)| row(&t, item, i, i == at, item.id != "loading", cx))
            .collect();
        let hover = t.hover;
        let bright = t.bright;
        let panel = div()
            .id("nixa-menu")
            .debug_selector(|| "nixa-menu".into())
            .key_context("NixaMenu")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(on_key))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(WIDTH))
            .h(px(HEIGHT))
            .max_w_full()
            .flex()
            .flex_col()
            .rounded(px(RADIUS_POPOVER))
            .border_1()
            .border_color(t.border)
            .bg(t.popover)
            .shadow(shadow(&t))
            .overflow_hidden()
            .text_size(px(12.8))
            .text_color(t.foreground)
            .child(
                div()
                    .px(px(8.))
                    .pt(px(8.))
                    .pb(px(2.))
                    .gap(px(4.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .children(tabs)
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("menu-close")
                            .debug_selector(|| "menu-close".into())
                            .size(px(24.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_color(t.muted)
                            .hover(move |s| s.bg(hover).text_color(bright))
                            .tooltip(tooltip("Close (Esc)"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(MenuEvent::Close)))
                            .child("×"),
                    ),
            )
            .child(
                div()
                    .h(px(1.))
                    .mx(px(6.))
                    .my(px(4.))
                    .flex_none()
                    .bg(t.border),
            )
            .child(
                div()
                    .id("nixa-menu-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(5.))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .child(
                div()
                    .px(px(10.))
                    .py(px(5.))
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(t.dim)
                    .child(HINT),
            );
        div()
            .id("nixa-menu-scrim")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(MenuEvent::Close)),
            )
            .child(panel)
            .into_any_element()
    }
}
