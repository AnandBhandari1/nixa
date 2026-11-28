use super::table::{self, Entry};
use super::{KeyTable, file, names};
use crate::actions::ClosePane;
use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, ClickEvent, Context, FocusHandle, Focusable,
    FontWeight, Global, KeyDownEvent, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    px, size,
};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::{Theme, theme};

pub const TITLE: &str = "Keys";
pub const WIDTH: f32 = 640.0;
pub const HEIGHT: f32 = 680.0;

struct HelpWindow(AnyWindowHandle);

impl Global for HelpWindow {}

pub struct HelpView {
    focus: FocusHandle,
    pub note: String,
}

impl Focusable for HelpView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn new(cx: &mut Context<HelpView>) -> HelpView {
    cx.observe_global::<KeyTable>(|_, cx| cx.notify()).detach();
    HelpView {
        focus: cx.focus_handle(),
        note: String::new(),
    }
}

pub fn open(cx: &mut App) {
    cx.defer(open_now);
}

fn open_now(cx: &mut App) {
    let existing = cx
        .try_global::<HelpWindow>()
        .and_then(|g| g.0.downcast::<HelpView>());
    if let Some(handle) = existing
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(420.0), px(320.0))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = cx.new(new);
        window.focus(&view.focus_handle(cx));
        view
    });
    if let Ok(handle) = opened {
        cx.set_global(HelpWindow(handle.into()));
    }
}

fn open_file(view: &mut HelpView, cx: &mut Context<HelpView>) {
    let path = file::path();
    match file::ensure(&path) {
        Ok(()) => {
            view.note = format!("Opened {}. Saved changes apply at once.", path.display());
            cx.open_with_system(&path);
        }
        Err(e) => view.note = format!("Cannot make {}: {e:#}", path.display()),
    }
    cx.notify();
}

fn row(t: &Theme, entry: &Entry) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .py(px(5.0))
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .w(px(150.0))
                .flex_none()
                .font_family(nixa_ui::text::MONO_FONT)
                .text_color(t.bright)
                .child(table::shown(&entry.key)),
        )
        .child(div().flex_1().min_w_0().child(names::label(&entry.name)))
        .child(
            div()
                .flex_none()
                .text_size(px(11.0))
                .text_color(t.dim)
                .child(entry.name.clone()),
        )
        .into_any_element()
}

fn group(t: &Theme, title: &'static str, list: &[Entry]) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .pt(px(12.0))
        .child(
            div()
                .pb(px(4.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.bright)
                .child(title),
        )
        .children(list.iter().map(|e| row(t, e)))
        .into_any_element()
}

fn close_button(t: &Theme, cx: &mut Context<HelpView>) -> impl IntoElement + use<> {
    let fg = t.foreground;
    div()
        .id("keys-help-close")
        .debug_selector(|| "keys-help-close".into())
        .flex_none()
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_color(t.muted)
        .hover(move |s| s.text_color(fg))
        .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.remove_window()))
        .child("× close")
}

impl Render for HelpView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let keys = cx.try_global::<KeyTable>().cloned().unwrap_or_default();
        let groups: Vec<AnyElement> = table::groups(&keys.entries)
            .iter()
            .map(|(title, list)| group(&t, title, list))
            .collect();
        let problems: Vec<AnyElement> = keys
            .problems
            .iter()
            .map(|p| {
                div()
                    .text_color(t.red)
                    .child(p.clone())
                    .into_any_element()
            })
            .collect();
        div()
            .id("keys-help")
            .debug_selector(|| "keys-help".into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
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
                    .gap(px(10.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(14.0))
                            .text_color(t.bright)
                            .child(TITLE),
                    )
                    .child(
                        button(&t, "keys-help-file", "Open keys file", ButtonKind::Secondary)
                            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| open_file(view, cx))),
                    )
                    .child(close_button(&t, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(16.0))
                    .pt(px(10.0))
                    .child(div().text_color(t.muted).child(format!(
                        "Change keys in {}. Write \"key\": \"action\" pairs under \"global\". Use \"none\" to turn a key off.",
                        file::path().display()
                    )))
                    .when(!self.note.is_empty(), |el| {
                        el.child(div().text_color(t.muted).child(self.note.clone()))
                    })
                    .children(problems),
            )
            .child(
                div()
                    .id("keys-help-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.0))
                    .pb(px(16.0))
                    .children(groups),
            )
    }
}
