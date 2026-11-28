use super::store;
use crate::actions::ClosePane;
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, SharedString, Subscription, Window, WindowHandle, div, px,
};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use std::path::PathBuf;

pub const NO_WINDOW: &str = "The main Nixa window is not open.";

pub struct LayoutsView {
    focus: FocusHandle,
    pub input: Entity<TextInput>,
    pub names: Vec<String>,
    pub note: String,
    pub dir: PathBuf,
    _events: Subscription,
}

impl Focusable for LayoutsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn on_input(
    view: &mut LayoutsView,
    _: &Entity<TextInput>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<LayoutsView>,
) {
    match event {
        InputEvent::Submit(name) => view.save(name, cx),
        InputEvent::Cancel => window.remove_window(),
        InputEvent::Changed(_) => {}
    }
}

pub fn new(dir: PathBuf, window: &mut Window, cx: &mut Context<LayoutsView>) -> LayoutsView {
    let input = cx.new(|cx| TextInput::new(cx, "Name for the current layout"));
    let events = cx.subscribe_in(&input, window, on_input);
    LayoutsView {
        focus: cx.focus_handle(),
        input,
        names: store::list(&dir),
        note: String::new(),
        dir,
        _events: events,
    }
}

fn main_window(cx: &App) -> Option<WindowHandle<Workspace>> {
    cx.windows()
        .into_iter()
        .find_map(|w| w.downcast::<Workspace>())
}

fn on_main(cx: &mut App, job: impl FnOnce(&mut Workspace, &mut Context<Workspace>)) -> bool {
    let Some(handle) = main_window(cx) else {
        return false;
    };
    handle.update(cx, |ws, _, cx| job(ws, cx)).is_ok()
}

impl LayoutsView {
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.names = store::list(&self.dir);
        cx.notify();
    }

    pub fn save(&mut self, raw: &str, cx: &mut Context<Self>) {
        let Some(name) = store::clean_name(raw) else {
            self.note = format!(
                "Use 1 to {} letters, digits, spaces, - or _ for the name.",
                store::NAME_MAX
            );
            cx.notify();
            return;
        };
        let saved = main_window(cx).and_then(|h| h.read(cx).ok().map(store::capture));
        let Some(saved) = saved else {
            self.note = NO_WINDOW.to_string();
            cx.notify();
            return;
        };
        self.note = match store::write(&self.dir, &name, &saved) {
            Ok(()) => format!("Saved the layout \"{name}\"."),
            Err(e) => format!("Cannot save the layout: {e:#}"),
        };
        self.input.update(cx, |i, cx| i.set_value("", cx));
        self.reload(cx);
    }

    pub fn use_layout(&mut self, name: &str, cx: &mut Context<Self>) {
        let saved = match store::read(&self.dir, name) {
            Ok(saved) => saved,
            Err(e) => {
                self.note = format!("Cannot read the layout \"{name}\": {e:#}");
                cx.notify();
                return;
            }
        };
        self.note = if on_main(cx, |ws, cx| store::apply(ws, &saved, cx)) {
            format!("Using the layout \"{name}\".")
        } else {
            NO_WINDOW.to_string()
        };
        cx.notify();
    }

    pub fn use_default(&mut self, cx: &mut Context<Self>) {
        self.note = if on_main(cx, store::apply_default) {
            "Using the default widths.".to_string()
        } else {
            NO_WINDOW.to_string()
        };
        cx.notify();
    }

    pub fn delete(&mut self, name: &str, cx: &mut Context<Self>) {
        self.note = match store::delete(&self.dir, name) {
            Ok(()) => format!("Deleted the layout \"{name}\"."),
            Err(e) => format!("Cannot delete the layout: {e:#}"),
        };
        self.reload(cx);
    }
}

fn line(t: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .py(px(6.0))
        .border_b_1()
        .border_color(t.border)
}

fn default_row(t: &Theme, cx: &mut Context<LayoutsView>) -> AnyElement {
    line(t)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child("Default")
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(t.dim)
                        .child("Usual sidebar and panel widths"),
                ),
        )
        .child(
            button(t, "layout-use-default", "Use", ButtonKind::Secondary)
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.use_default(cx))),
        )
        .into_any_element()
}

fn saved_row(t: &Theme, name: &str, cx: &mut Context<LayoutsView>) -> AnyElement {
    let use_name = name.to_string();
    let drop_name = name.to_string();
    line(t)
        .child(div().flex_1().min_w_0().child(name.to_string()))
        .child(
            button(
                t,
                SharedString::from(format!("layout-use-{name}")),
                "Use",
                ButtonKind::Secondary,
            )
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.use_layout(&use_name, cx))),
        )
        .child(
            button(
                t,
                SharedString::from(format!("layout-delete-{name}")),
                "Delete",
                ButtonKind::Ghost,
            )
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.delete(&drop_name, cx))),
        )
        .into_any_element()
}

impl Render for LayoutsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let fg = t.foreground;
        let names = self.names.clone();
        let rows: Vec<AnyElement> = names.iter().map(|n| saved_row(&t, n, cx)).collect();
        let first = default_row(&t, cx);
        div()
            .id("layouts")
            .debug_selector(|| "layouts".into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(16.0))
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.5))
            .on_action(cx.listener(|_, _: &ClosePane, window, _| window.remove_window()))
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, window, _| {
                if e.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(14.0))
                            .text_color(t.bright)
                            .child(super::TITLE),
                    )
                    .child(
                        div()
                            .id("layouts-close")
                            .debug_selector(|| "layouts-close".into())
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .text_color(t.muted)
                            .hover(move |s| s.text_color(fg))
                            .on_click(cx.listener(|_, _: &ClickEvent, window, _| {
                                window.remove_window()
                            }))
                            .child("× close"),
                    ),
            )
            .child(div().text_color(t.muted).child(
                "A layout keeps the sidebar width, the panel width and which side panel is open.",
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().flex_1().min_w_0().child(self.input.clone()))
                    .child(
                        button(&t, "layouts-save", "Save current", ButtonKind::Primary).on_click(
                            cx.listener(|view, _: &ClickEvent, _, cx| {
                                let name = view.input.read(cx).value().to_string();
                                view.save(&name, cx);
                            }),
                        ),
                    ),
            )
            .when(!self.note.is_empty(), |el| {
                el.child(
                    div()
                        .debug_selector(|| "layouts-note".into())
                        .text_color(t.muted)
                        .child(self.note.clone()),
                )
            })
            .child(
                div()
                    .id("layouts-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(first)
                    .children(rows),
            )
    }
}
