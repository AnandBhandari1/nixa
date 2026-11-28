use super::keys;
use super::model::{self, Action, Checkout, Entry, Facts, Target};
use super::run;
use crate::links::brain::Link;
use gpui::prelude::*;
use gpui::{
    App, Context, Corner, ElementId, Entity, EventEmitter, FocusHandle, KeyDownEvent, MouseButton,
    Pixels, Point, SharedString, Task, Window, div, px,
};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::menu::{MenuItem, MenuKey, menu_key, menu_row};
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_protocol::TermId;
use gpui::{Focusable, Subscription};
use nixa_ui::popover::{float_at, popover, tooltip};
use nixa_ui::section::separator;
use nixa_ui::{Theme, theme};
use std::path::PathBuf;

pub const WIDTH: f32 = 220.;
pub const MAX_HEIGHT: f32 = 440.;
pub const FORM_WIDTH: f32 = 300.;

#[derive(Debug, Clone, PartialEq)]
pub enum CtxEvent {
    Run(Action),
    Open { path: String, folder: bool },
    Saved,
    Notice { title: String, body: String },
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtxCommand {
    Actions,
}

pub fn commands() -> Vec<(&'static str, &'static str, CtxCommand)> {
    vec![(
        "Show the actions menu",
        "Open the right-click menu with new space, keybinds, themes and quick actions",
        CtxCommand::Actions,
    )]
}

pub struct SaveForm {
    pub terminal: TermId,
    pub title: Entity<TextInput>,
    pub purpose: Entity<TextInput>,
    pub error: String,
    pub focused: bool,
    _events: Vec<Subscription>,
}

pub struct CtxMenu {
    pub form: Option<SaveForm>,
    pub target: Target,
    pub facts: Facts,
    pub position: Point<Pixels>,
    pub at: Option<usize>,
    pub git: Option<Checkout>,
    pub sending: bool,
    pub link: Option<Link>,
    pub focus: FocusHandle,
    probe: Option<Task<()>>,
    work: Option<Task<()>>,
}

impl EventEmitter<CtxEvent> for CtxMenu {}

pub fn new(
    target: Target,
    facts: Facts,
    position: Point<Pixels>,
    link: Option<Link>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<CtxMenu> {
    cx.new(|cx| build(target, facts, position, link, window, cx))
}

pub fn build(
    target: Target,
    facts: Facts,
    position: Point<Pixels>,
    link: Option<Link>,
    window: &mut Window,
    cx: &mut Context<CtxMenu>,
) -> CtxMenu {
    let focus = cx.focus_handle();
    window.focus(&focus);
    let mut menu = CtxMenu {
        form: None,
        target,
        facts,
        position,
        at: None,
        git: None,
        sending: false,
        link,
        focus,
        probe: None,
        work: None,
    };
    menu.at = (!menu.entries().is_empty()).then_some(0);
    menu.probe = probe_git(&menu, cx);
    menu
}

fn probe_git(menu: &CtxMenu, cx: &mut Context<CtxMenu>) -> Option<Task<()>> {
    let Target::Space { id } = &menu.target else {
        return None;
    };
    let space = menu.facts.spaces.iter().find(|s| &s.id == id)?;
    let place = model::space_place(&space.folder, [space.place.as_str()]);
    if place.is_empty() {
        return None;
    }
    let folder = PathBuf::from(place);
    let task = cx
        .background_executor()
        .spawn(async move { run::checkout(&folder) });
    Some(cx.spawn(async move |this, cx| {
        let git = task.await;
        this.update(cx, |menu, cx| {
            menu.git = git;
            cx.notify();
        })
        .ok();
    }))
}

impl CtxMenu {
    pub fn entries(&self) -> Vec<Entry> {
        model::entries(&self.target, &self.facts, self.git)
    }

    pub fn menu_items(&self, window: Option<&Window>) -> Vec<MenuItem> {
        self.entries()
            .into_iter()
            .map(|entry| {
                let saving = matches!(entry.action, Action::SaveAgent { .. });
                let label = if saving && self.sending {
                    run::SAVING.to_string()
                } else {
                    entry.label.clone()
                };
                let mut item = MenuItem::new(label)
                    .icon(entry.icon)
                    .checked(entry.checked)
                    .disabled(self.sending);
                if entry.separated {
                    item = item.separated();
                }
                if let Some(keys) = window.and_then(|w| keys::shortcut(w, &entry.action)) {
                    item = item.shortcut(keys);
                }
                item
            })
            .collect()
    }

    pub fn key(&mut self, key: &str, cx: &mut Context<CtxMenu>) -> bool {
        if self.form.is_some() {
            return false;
        }
        let items = self.menu_items(None);
        match menu_key(&items, self.at, key) {
            MenuKey::Move(index) => {
                self.at = Some(index);
                cx.notify();
            }
            MenuKey::Choose(index) => self.choose(index, cx),
            MenuKey::Close => cx.emit(CtxEvent::Close),
            MenuKey::Ignore => return false,
        }
        true
    }

    pub fn choose(&mut self, index: usize, cx: &mut Context<CtxMenu>) {
        if self.sending {
            return;
        }
        let Some(entry) = self.entries().into_iter().nth(index) else {
            return;
        };
        self.at = Some(index);
        match entry.action {
            Action::SaveAgent {
                terminal, title, ..
            } => self.open_form(terminal, title, cx),
            Action::LinkOpen { path } => {
                let folder = run::is_dir(&path);
                cx.emit(CtxEvent::Open { path, folder });
                cx.emit(CtxEvent::Close);
            }
            Action::LinkReveal { path } => {
                cx.background_executor()
                    .spawn(async move { run::reveal(&path).ok() })
                    .detach();
                cx.emit(CtxEvent::Close);
            }
            Action::RightClicks {
                terminal,
                to_program,
            } => {
                cx.background_executor()
                    .spawn(async move { run::set_right_clicks(terminal, to_program).ok() })
                    .detach();
                cx.emit(CtxEvent::Close);
            }
            action => {
                cx.emit(CtxEvent::Run(action));
                cx.emit(CtxEvent::Close);
            }
        }
    }

    pub fn open_form(&mut self, terminal: TermId, title: String, cx: &mut Context<CtxMenu>) {
        if self.link.is_none() {
            cx.emit(CtxEvent::Notice {
                title: run::SAVE_TITLE.to_string(),
                body: run::NO_BRAIN.to_string(),
            });
            return;
        }
        let title_input = cx.new(|cx| TextInput::new(cx, "Title"));
        title_input.update(cx, |i, cx| i.set_value(title, cx));
        let purpose = cx.new(|cx| TextInput::new(cx, "What is this agent for?"));
        let on_input = |menu: &mut CtxMenu, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<CtxMenu>| match event {
            InputEvent::Submit(_) => menu.submit_form(cx),
            InputEvent::Cancel => cx.emit(CtxEvent::Close),
            InputEvent::Changed(_) => {
                if let Some(form) = menu.form.as_mut() {
                    form.error.clear();
                }
            }
        };
        let events = vec![
            cx.subscribe(&title_input, on_input),
            cx.subscribe(&purpose, on_input),
        ];
        self.form = Some(SaveForm {
            terminal,
            title: title_input,
            purpose,
            error: String::new(),
            focused: false,
            _events: events,
        });
        cx.notify();
    }

    pub fn submit_form(&mut self, cx: &mut Context<CtxMenu>) {
        if self.sending {
            return;
        }
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let title = form.title.read(cx).value().to_string();
        let purpose = form.purpose.read(cx).value().to_string();
        if let Some(missing) = run::save_missing(&title, &purpose) {
            form.error = missing.to_string();
            cx.notify();
            return;
        }
        let Some(link) = self.link.clone() else {
            form.error = run::NO_BRAIN.to_string();
            cx.notify();
            return;
        };
        let body = run::save_body(&form.terminal, &title, &purpose);
        self.sending = true;
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { run::save_agent(&link, &body) });
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |menu, cx| {
                menu.sending = false;
                match result {
                    Ok(text) => {
                        cx.emit(CtxEvent::Notice {
                            title: run::SAVE_TITLE.to_string(),
                            body: text,
                        });
                        cx.emit(CtxEvent::Saved);
                        cx.emit(CtxEvent::Close);
                    }
                    Err(e) => {
                        if let Some(form) = menu.form.as_mut() {
                            form.error = format!("{e:#}");
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

fn form_view(menu: &mut CtxMenu, t: &Theme, window: &mut Window, cx: &mut Context<CtxMenu>) -> Option<gpui::AnyElement> {
    let sending = menu.sending;
    let form = menu.form.as_mut()?;
    if !form.focused {
        form.focused = true;
        window.focus(&form.title.focus_handle(cx));
    }
    let field = |id: &'static str, label: &'static str, input: Entity<TextInput>| {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(div().text_size(px(11.)).text_color(t.dim).child(label))
            .child(
                div()
                    .h(px(26.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(px(5.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.raised)
                    .child(input),
            )
    };
    let error = form.error.clone();
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(8.))
            .py(px(6.))
            .child(field("ctx-save-title", "Title", form.title.clone()))
            .child(field("ctx-save-purpose", "Purpose", form.purpose.clone()))
            .when(!error.is_empty(), |el| {
                el.child(
                    div()
                        .id("ctx-save-error")
                        .debug_selector(|| "ctx-save-error".into())
                        .text_size(px(11.))
                        .text_color(t.red)
                        .child(SharedString::from(error)),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.))
                    .child(
                        button(t, "ctx-save-cancel", "Cancel", ButtonKind::Ghost)
                            .debug_selector(|| "ctx-save-cancel".into())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(CtxEvent::Close))),
                    )
                    .child(
                        button(
                            t,
                            "ctx-save-ok",
                            if sending { run::SAVING } else { "Save" },
                            ButtonKind::Primary,
                        )
                        .debug_selector(|| "ctx-save-ok".into())
                        .on_click(cx.listener(|menu, _, _, cx| menu.submit_form(cx))),
                    ),
            )
            .into_any_element(),
    )
}

fn header(t: &Theme, title: String, cx: &mut Context<CtxMenu>) -> impl IntoElement {
    let hover = t.hover;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .pb(px(2.))
        .text_size(px(11.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_color(t.muted)
                .child(title),
        )
        .child(
            div()
                .id("ctx-close")
                .debug_selector(|| "ctx-close".into())
                .flex_none()
                .px(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .text_color(t.dim)
                .hover(move |s| s.bg(hover))
                .tooltip(tooltip("Close"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(CtxEvent::Close)))
                .child("× close"),
        )
}

impl Render for CtxMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let items = self.menu_items(Some(window));
        let form = form_view(self, &t, window, cx);
        let mut panel = popover(&t)
            .id("ctx-menu")
            .debug_selector(|| "ctx-menu".into())
            .track_focus(&self.focus)
            .w(px(WIDTH))
            .max_h(px(MAX_HEIGHT))
            .overflow_y_scroll()
            .on_key_down(cx.listener(|menu, event: &KeyDownEvent, _, cx| {
                if menu.key(event.keystroke.key.as_str(), cx) {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(CtxEvent::Close)))
            .child(header(
                &t,
                if form.is_some() {
                    run::SAVE_TITLE.to_string()
                } else {
                    model::title(&self.target)
                },
                cx,
            ));
        if let Some(form) = form {
            panel = panel.w(px(FORM_WIDTH)).child(form);
            return div().child(float_at(self.position, Corner::TopLeft, panel));
        }
        for (index, item) in items.iter().enumerate() {
            if item.separator_before && index > 0 {
                panel = panel.child(separator(&t));
            }
            let selector = format!("ctx-row-{index}");
            let id = ElementId::Name(SharedString::from(selector.clone()));
            let mut row = menu_row(&t, id, item, self.at == Some(index))
                .debug_selector(move || selector.clone())
                .tooltip(tooltip(item.label.clone()));
            if !item.disabled {
                row = row.on_click(cx.listener(move |menu, _, _, cx| {
                    cx.stop_propagation();
                    menu.choose(index, cx);
                }));
            }
            panel = panel.child(row);
        }
        div().child(float_at(self.position, Corner::TopLeft, panel))
    }
}
