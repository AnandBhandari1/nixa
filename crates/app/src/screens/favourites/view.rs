use super::store::Favourite;
use super::{place, store};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, MouseButton, SharedString, Subscription, Window, div, px,
};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::icon_button::{IconSize, icon_button};
use nixa_ui::metrics::RADIUS_POPOVER;
use nixa_ui::popover::{shadow, tooltip};
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Icon, Theme, theme};
use std::path::PathBuf;

pub const WIDTH: f32 = 420.0;
pub const HEIGHT: f32 = 480.0;

#[derive(Debug, Clone, PartialEq)]
pub enum FavouritesEvent {
    OpenUrl(String),
    EditFile(PathBuf),
    OpenFile(PathBuf),
    Notice { text: String, error: bool },
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AskStep {
    Link,
    Title(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    List,
    Ask(AskStep),
    Confirm(usize),
}

pub struct Favourites {
    scroll: gpui::ScrollHandle,
    pub file: PathBuf,
    pub folder: Option<PathBuf>,
    pub project: Option<PathBuf>,
    pub rows: Vec<Favourite>,
    pub error: Option<String>,
    pub ask_error: Option<String>,
    pub loading: bool,
    pub selected: usize,
    pub mode: Mode,
    pub input: Option<Entity<TextInput>>,
    focus: FocusHandle,
    _input_events: Option<Subscription>,
}

impl EventEmitter<FavouritesEvent> for Favourites {}

impl Focusable for Favourites {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn commands() -> Vec<(&'static str, &'static str)> {
    vec![("Favourites of this project", "")]
}

pub fn new(
    folder: Option<PathBuf>,
    file: PathBuf,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Favourites> {
    cx.new(|cx| {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut view = Favourites {
            scroll: gpui::ScrollHandle::new(),
            file,
            folder,
            project: None,
            rows: Vec::new(),
            error: None,
            ask_error: None,
            loading: true,
            selected: 0,
            mode: Mode::List,
            input: None,
            focus,
            _input_events: None,
        };
        view.reload(cx);
        view
    })
}

impl Favourites {
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        let folder = self.folder.clone();
        let file = self.file.clone();
        let task = cx.background_executor().spawn(async move {
            let root = folder.map(|f| place::project_root(&f));
            let rows = match &root {
                Some(root) => store::load(&file, root),
                None => Ok(Vec::new()),
            };
            (root, rows)
        });
        cx.spawn(async move |this, cx| {
            let (root, rows) = task.await;
            this.update(cx, |view, cx| {
                view.loading = false;
                view.project = root;
                match rows {
                    Ok(rows) => view.rows = rows,
                    Err(e) => {
                        view.rows.clear();
                        view.error = Some(format!("{e:#}"));
                    }
                }
                view.selected = view.selected.min(view.rows.len().saturating_sub(1));
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn refresh_rows(&mut self) {
        let Some(project) = self.project.clone() else {
            return;
        };
        match store::load(&self.file, &project) {
            Ok(rows) => {
                self.rows = rows;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    fn notice(&self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        cx.emit(FavouritesEvent::Notice {
            text: text.into(),
            error,
        });
    }

    pub fn open(&mut self, index: usize, cx: &mut Context<Self>) {
        let (Some(row), Some(project)) = (self.rows.get(index).cloned(), self.project.clone())
        else {
            return;
        };
        self.selected = index;
        if place::is_web(&row.url) {
            cx.emit(FavouritesEvent::OpenUrl(row.url.trim().to_string()));
        } else {
            match place::resolve(&row.url, &project) {
                Ok(path) if place::is_text(&path) => cx.emit(FavouritesEvent::EditFile(path)),
                Ok(path) if path.exists() => cx.emit(FavouritesEvent::OpenFile(path)),
                Ok(path) => self.notice(format!("{} does not exist", path.display()), true, cx),
                Err(e) => self.notice(format!("{e:#}"), true, cx),
            }
        }
        cx.notify();
    }

    pub fn begin_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project.is_none() {
            self.notice("Open a space first", true, cx);
            return;
        }
        self.ask(AskStep::Link, window, cx);
    }

    fn ask(&mut self, step: AskStep, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = match &step {
            AskStep::Link => "https://… or a path under the project".to_string(),
            AskStep::Title(_) => "Title".into(),
        };
        let input = cx.new(|cx| TextInput::new(cx, placeholder));
        let events = cx.subscribe_in(
            &input,
            window,
            |view: &mut Favourites, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submit(text) => view.submit(text.clone(), window, cx),
                InputEvent::Cancel => view.cancel(window, cx),
                InputEvent::Changed(_) => {
                    view.ask_error = None;
                    cx.notify();
                }
            },
        );
        window.focus(&input.focus_handle(cx));
        if let AskStep::Title(url) = &step {
            input.update(cx, |input, cx| {
                input.set_value(url.clone(), cx);
                input.select_value(cx);
            });
        }
        self.mode = Mode::Ask(step);
        self.ask_error = None;
        self.input = Some(input);
        self._input_events = Some(events);
        cx.notify();
    }

    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::List;
        self.input = None;
        self._input_events = None;
        self.ask_error = None;
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn submit(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return self.cancel(window, cx);
        };
        match self.mode.clone() {
            Mode::Ask(AskStep::Link) => {
                if text.trim().is_empty() {
                    return self.cancel(window, cx);
                }
                match place::stored_value(&text, &project) {
                    Ok(url) => self.ask(AskStep::Title(url), window, cx),
                    Err(e) => {
                        self.ask_error = Some(format!("{e:#}"));
                        cx.notify();
                    }
                }
            }
            Mode::Ask(AskStep::Title(url)) => {
                let title = text.trim().to_string();
                self.cancel(window, cx);
                match store::save(&self.file, &project, &title, &url) {
                    Ok(()) => {
                        self.refresh_rows();
                        if let Some(i) = self.rows.iter().position(|f| f.url == url) {
                            self.selected = i;
                        }
                        self.notice("Saved", false, cx);
                    }
                    Err(e) => self.notice(format!("{e:#}"), true, cx),
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn submit_button(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .input
            .as_ref()
            .map(|i| i.read(cx).value().to_string())
            .unwrap_or_default();
        self.submit(text, window, cx);
    }

    pub fn begin_remove(&mut self, cx: &mut Context<Self>) {
        if self.rows.get(self.selected).is_some() {
            self.mode = Mode::Confirm(self.selected);
            cx.notify();
        }
    }

    pub fn confirm_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Mode::Confirm(index) = self.mode else {
            return;
        };
        self.mode = Mode::List;
        window.focus(&self.focus);
        let (Some(row), Some(project)) = (self.rows.get(index).cloned(), self.project.clone())
        else {
            cx.notify();
            return;
        };
        match store::remove(&self.file, &project, &row.url) {
            Ok(_) => {
                self.refresh_rows();
                self.notice("Removed", false, cx);
            }
            Err(e) => self.notice(format!("{e:#}"), true, cx),
        }
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match (&self.mode, key) {
            (Mode::List, "escape") => cx.emit(FavouritesEvent::Close),
            (Mode::List, "down") if !self.rows.is_empty() => {
                self.selected = (self.selected + 1).min(self.rows.len() - 1);
            }
            (Mode::List, "up") => self.selected = self.selected.saturating_sub(1),
            (Mode::List, "enter") => self.open(self.selected, cx),
            (Mode::Confirm(_), "escape") => self.cancel(window, cx),
            (Mode::Confirm(_), "enter") => self.confirm_remove(window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Favourites {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render(self, cx)
    }
}

fn title_bar(t: &Theme, title: &'static str, cx: &mut Context<Favourites>) -> AnyElement {
    div()
        .h(px(40.))
        .flex_none()
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(13.))
                .text_color(t.bright)
                .child(title),
        )
        .child(
            icon_button(t, "fav-close", Icon::X, IconSize::Toolbar, false)
                .debug_selector(|| "fav-close".into())
                .tooltip(tooltip("Close"))
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(FavouritesEvent::Close);
                })),
        )
        .into_any_element()
}

fn ask_body(
    t: &Theme,
    view: &Favourites,
    step: &AskStep,
    cx: &mut Context<Favourites>,
) -> AnyElement {
    let (heading, label, hint) = match step {
        AskStep::Link => (
            "URL or file path",
            "Link",
            "https://… or a path under the project",
        ),
        AskStep::Title(_) => ("Title", "Title", "Enter uses the URL"),
    };
    div()
        .flex_1()
        .min_h_0()
        .p(px(18.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.bright)
                        .child(heading),
                )
                .child(div().text_size(px(11.)).text_color(t.dim).child("Esc")),
        )
        .child(
            div()
                .pt(px(4.))
                .pb(px(4.))
                .text_size(px(11.5))
                .text_color(t.dim)
                .child(label),
        )
        .children(
            view.input
                .clone()
                .map(|input| div().flex().flex_col().child(input)),
        )
        .child(div().text_size(px(11.5)).text_color(t.dim).child(hint))
        .children(view.ask_error.clone().map(|e| {
            div()
                .debug_selector(|| "fav-ask-error".into())
                .text_size(px(11.5))
                .text_color(t.red)
                .child(SharedString::from(e))
        }))
        .child(
            div()
                .pt(px(8.))
                .flex()
                .gap(px(8.))
                .justify_end()
                .child(
                    button(t, "ask-cancel", "Cancel", ButtonKind::Secondary)
                        .debug_selector(|| "ask-cancel".into())
                        .on_click(cx.listener(|view, _, window, cx| view.cancel(window, cx))),
                )
                .child(
                    button(t, "ask-save", "Save", ButtonKind::Primary)
                        .debug_selector(|| "ask-save".into())
                        .on_click(
                            cx.listener(|view, _, window, cx| view.submit_button(window, cx)),
                        ),
                ),
        )
        .into_any_element()
}

fn confirm_body(t: &Theme, row: &Favourite, cx: &mut Context<Favourites>) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .p(px(18.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.bright)
                .child("Remove favourite?"),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(t.foreground)
                .child(SharedString::from(row.title.clone())),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(t.dim)
                .child(SharedString::from(row.url.clone())),
        )
        .child(
            div()
                .pt(px(8.))
                .flex()
                .gap(px(8.))
                .justify_end()
                .child(
                    button(t, "rm-cancel", "Cancel", ButtonKind::Secondary)
                        .debug_selector(|| "rm-cancel".into())
                        .on_click(cx.listener(|view, _, window, cx| view.cancel(window, cx))),
                )
                .child(
                    button(t, "rm-yes", "Remove", ButtonKind::Danger)
                        .debug_selector(|| "rm-yes".into())
                        .on_click(
                            cx.listener(|view, _, window, cx| view.confirm_remove(window, cx)),
                        ),
                ),
        )
        .into_any_element()
}

fn list_body(t: &Theme, view: &Favourites, cx: &mut Context<Favourites>) -> AnyElement {
    let hover = t.hover;
    let no_project = view.project.is_none();
    let rows: Vec<AnyElement> = view
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            div()
                .id(("fav", i))
                .debug_selector(move || format!("fav-{i}"))
                .min_h(px(32.))
                .flex_none()
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(7.))
                .cursor_pointer()
                .tooltip(tooltip(row.url.clone()))
                .when(i == view.selected, |el| el.bg(t.selected))
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |view, _, _, cx| view.open(i, cx)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.bright)
                        .child(SharedString::from(row.title.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(t.dim)
                        .child(SharedString::from(row.url.clone())),
                )
                .into_any_element()
        })
        .collect();
    let note = |text: SharedString, color| {
        div()
            .p(px(8.))
            .text_size(px(12.))
            .text_color(color)
            .child(text)
    };
    let empty = !view.loading && view.rows.is_empty() && view.error.is_none();
    let has_rows = !view.rows.is_empty();
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .px(px(10.))
                .py(px(8.))
                .flex_none()
                .flex()
                .gap(px(8.))
                .child(disabled(
                    button(t, "fav-add", "Add", ButtonKind::Primary)
                        .debug_selector(|| "fav-add".into())
                        .tooltip(tooltip("Add a URL or file path"))
                        .on_click(cx.listener(|view, _, window, cx| view.begin_add(window, cx))),
                    no_project,
                ))
                .when(has_rows, |el| el.child(
                    button(t, "fav-remove", "Remove", ButtonKind::Secondary)
                        .debug_selector(|| "fav-remove".into())
                        .tooltip(tooltip("Remove the selected favourite"))
                        .on_click(cx.listener(|view, _, _, cx| view.begin_remove(cx))),
                )),
        )
        .child(
            div()
                .px(px(12.))
                .pb(px(6.))
                .text_size(px(11.))
                .text_color(t.dim)
                .truncate()
                .child(SharedString::from(
                    view.project
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| {
                            if view.loading {
                                String::new()
                            } else {
                                "No active project".into()
                            }
                        }),
                )),
        )
        .child(
            nixa_ui::scroll::scroll_area(t, "fav-list", &view.scroll, div()
                .px(px(8.))
                .pb(px(8.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .children(view.error.clone().map(|e| note(e.into(), t.red)))
                .when(view.loading, |el| el.child(note("Loading…".into(), t.dim)))
                .when(empty, |el| {
                    el.child(note("No favourites yet".into(), t.dim))
                })
                .children(rows),
            ).flex_1(),
        )
        .when(has_rows, |el| {
            el.child(
                div()
                    .px(px(12.))
                    .pb(px(8.))
                    .text_size(px(11.))
                    .text_color(t.dim)
                    .child("Click a row to open · Remove acts on the highlighted row"),
            )
        })
        .into_any_element()
}

pub fn render(view: &Favourites, cx: &mut Context<Favourites>) -> AnyElement {
    let t = theme(cx).clone();
    let body = match &view.mode {
        Mode::List => list_body(&t, view, cx),
        Mode::Ask(step) => ask_body(&t, view, &step.clone(), cx),
        Mode::Confirm(i) => match view.rows.get(*i) {
            Some(row) => confirm_body(&t, &row.clone(), cx),
            None => list_body(&t, view, cx),
        },
    };
    div()
        .id("favourites-scrim")
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
            cx.listener(|_, _, _, cx| cx.emit(FavouritesEvent::Close)),
        )
        .child(
            div()
                .id("favourites")
                .debug_selector(|| "favourites".into())
                .track_focus(&view.focus)
                .w(px(WIDTH))
                .h(px(match view.mode {
                    Mode::List => (140. + view.rows.len() as f32 * 52.).min(HEIGHT),
                    Mode::Ask(_) => 255.,
                    Mode::Confirm(_) => 190.,
                }))
                .max_w_full()
                .max_h_full()
                .flex()
                .flex_col()
                .rounded(px(RADIUS_POPOVER))
                .border_1()
                .border_color(t.border)
                .bg(t.popover)
                .shadow(shadow(&t))
                .overflow_hidden()
                .text_size(px(13.))
                .text_color(t.foreground)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_key_down(cx.listener(Favourites::key))
                .child(title_bar(&t, "Favourites", cx))
                .child(body),
        )
        .into_any_element()
}
