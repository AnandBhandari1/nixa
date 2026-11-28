use super::model::{Action, Level, Notice, TOAST_MS};
use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Div, Entity, EventEmitter, FontWeight, SharedString, Stateful,
    Window, div, px,
};
use nixa_ui::icon_button::{IconSize, icon_button};
use nixa_ui::popover::shadow;
use nixa_ui::toast::{MAX_TOASTS, ToastKind, kind_color, kind_icon, toast_layer, toast_stack};
use nixa_ui::{Icon, Theme, icon, text, theme};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastEvent {
    Act(Action),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub id: u64,
    pub kind: ToastKind,
    pub title: String,
    pub body: String,
    pub action: Option<Action>,
}

#[derive(Default)]
pub struct ToastsView {
    pub items: Vec<Shown>,
    next: u64,
}

impl EventEmitter<ToastEvent> for ToastsView {}

pub fn toast_kind(level: Level) -> ToastKind {
    match level {
        Level::Info => ToastKind::Info,
        Level::Success => ToastKind::Success,
        Level::Warning => ToastKind::Warning,
        Level::Error => ToastKind::Error,
    }
}

impl ToastsView {
    pub fn push(&mut self, notice: &Notice, cx: &mut Context<ToastsView>) -> Option<u64> {
        let title = notice.title.trim().to_string();
        let body = notice.body.trim().to_string();
        if title.is_empty() && body.is_empty() {
            return None;
        }
        self.next += 1;
        let id = self.next;
        let (title, body) = if title.is_empty() { (body, String::new()) } else { (title, body) };
        self.items.push(Shown {
            id,
            kind: toast_kind(notice.level),
            title,
            body,
            action: notice.action.clone(),
        });
        if self.items.len() > MAX_TOASTS {
            self.items.remove(0);
        }
        let wait = Duration::from_millis(notice.timeout_ms.filter(|ms| *ms > 0).unwrap_or(TOAST_MS));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |view, cx| view.dismiss(id, cx)).ok();
        })
        .detach();
        cx.notify();
        Some(id)
    }

    pub fn dismiss(&mut self, id: u64, cx: &mut Context<ToastsView>) {
        let before = self.items.len();
        self.items.retain(|t| t.id != id);
        if before != self.items.len() {
            cx.notify();
        }
    }

    pub fn click(&mut self, id: u64, cx: &mut Context<ToastsView>) {
        let Some(shown) = self.items.iter().find(|t| t.id == id).cloned() else {
            return;
        };
        if let Some(action) = shown.action {
            cx.emit(ToastEvent::Act(action));
        }
        self.dismiss(id, cx);
    }
}

pub fn new(cx: &mut App) -> Entity<ToastsView> {
    cx.new(|_| ToastsView::default())
}

fn card(t: &Theme, shown: &Shown, cx: &mut Context<ToastsView>) -> Stateful<Div> {
    let id = shown.id;
    let color = kind_color(t, shown.kind);
    let clickable = shown.action.is_some();
    let hover = t.hover;
    div()
        .id(("toast", id as usize))
        .debug_selector(move || format!("toast-{id}"))
        .w(px(320.))
        .relative()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(10.))
        .pl(px(14.))
        .pr(px(6.))
        .py(px(10.))
        .rounded(px(10.))
        .bg(t.raised)
        .border_1()
        .border_color(t.border)
        .shadow(shadow(t))
        .overflow_hidden()
        .child(div().absolute().left(px(0.)).top(px(0.)).bottom(px(0.)).w(px(3.)).bg(color))
        .child(icon(kind_icon(shown.kind)).size(px(16.)).mt(px(1.)).text_color(color))
        .child(
            div()
                .id(("toast-body", id as usize))
                .debug_selector(move || format!("toast-body-{id}"))
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .rounded(px(6.))
                .when(clickable, |el| {
                    el.cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.click(id, cx)))
                })
                .child(
                    div()
                        .text_size(px(text::BASE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.bright)
                        .child(SharedString::from(shown.title.clone())),
                )
                .when(!shown.body.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.muted)
                            .child(SharedString::from(shown.body.clone())),
                    )
                }),
        )
        .child(
            icon_button(t, ("toast-close", id as usize), Icon::X, IconSize::Small, false)
                .debug_selector(move || format!("toast-close-{id}"))
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.dismiss(id, cx))),
        )
}

impl Render for ToastsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let items = self.items.clone();
        let cards: Vec<Stateful<Div>> = items.iter().map(|s| card(&t, s, cx)).collect();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(toast_layer(toast_stack().children(cards)))
    }
}
