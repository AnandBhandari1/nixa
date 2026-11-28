use super::model::{
    Action, Events, Item, Notice, Offer, UNDO_MS, now_ms, row_body, row_meta, row_text, row_tip,
    terminal_of,
};
use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla,
    ScrollHandle, SharedString, Stateful, Window, div, px,
};
use nixa_protocol::Status;
use nixa_ui::popover::{popover, tooltip};
use nixa_ui::theme::status_color;
use nixa_ui::{Theme, theme};
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

pub const WIDTH: f32 = 380.;
pub const HEIGHT: f32 = 480.;
pub const TICK: Duration = Duration::from_secs(30);
pub const ENDED: &str = "That agent has ended.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventsEvent {
    Focus { terminal: String },
    Resume(Offer),
    SetAgentVoice(bool),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventsCommand {
    Show,
    Clear,
    Undo,
    AgentVoiceOn,
    AgentVoiceOff,
}

pub fn commands() -> Vec<(&'static str, &'static str, EventsCommand)> {
    vec![
        ("Show events", "Open the list of agent events and notices", EventsCommand::Show),
        ("Clear events", "Remove every event (Undo for 15 seconds)", EventsCommand::Clear),
        ("Undo clear events", "Put the cleared events back", EventsCommand::Undo),
        ("Agent voice: on", "Nixa says when an agent needs you or is done", EventsCommand::AgentVoiceOn),
        ("Agent voice: off", "Nixa stays quiet about agents", EventsCommand::AgentVoiceOff),
    ]
}

pub struct EventsView {
    pub events: Events,
    pub open: bool,
    pub notice: Option<String>,
    pub numbers: HashMap<String, u32>,
    pub live: BTreeSet<String>,
    pub agent_voice: Option<bool>,
    pub clock: Option<u64>,
    ticking: bool,
    scroll: ScrollHandle,
    focus: FocusHandle,
}

impl EventEmitter<EventsEvent> for EventsView {}

impl Focusable for EventsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventsView {
    pub fn empty(cx: &mut Context<EventsView>) -> EventsView {
        EventsView {
            events: Events::default(),
            open: false,
            notice: None,
            numbers: HashMap::new(),
            live: BTreeSet::new(),
            agent_voice: None,
            clock: None,
            ticking: false,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    pub fn now(&self) -> u64 {
        self.clock.unwrap_or_else(now_ms)
    }

    pub fn unseen(&self) -> usize {
        self.events.unseen()
    }

    pub fn add_agent(&mut self, item: Item, cx: &mut Context<EventsView>) {
        if self.events.add_agent(item) {
            cx.notify();
        }
    }

    pub fn post(&mut self, notice: &Notice, cx: &mut Context<EventsView>) -> Option<Item> {
        let now = self.now();
        let item = self.events.post(notice, now);
        cx.notify();
        item
    }

    pub fn load(&mut self, list: Vec<Item>, cx: &mut Context<EventsView>) {
        self.events.load(list);
        cx.notify();
    }

    pub fn set_live(&mut self, live: BTreeSet<String>) {
        self.live = live;
    }

    pub fn show_agent_voice(&mut self, on: bool, cx: &mut Context<EventsView>) {
        self.agent_voice = Some(on);
        cx.notify();
    }

    pub fn set_agent_voice(&mut self, on: bool, cx: &mut Context<EventsView>) {
        cx.emit(EventsEvent::SetAgentVoice(on));
    }

    pub fn show(&mut self, cx: &mut Context<EventsView>) {
        self.open = true;
        self.notice = None;
        self.events.seen = self.now();
        self.tick(cx);
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<EventsView>) {
        if self.open {
            self.open = false;
            self.notice = None;
            cx.emit(EventsEvent::Close);
            cx.notify();
        }
    }

    pub fn toggle(&mut self, cx: &mut Context<EventsView>) {
        if self.open {
            self.close(cx);
        } else {
            self.show(cx);
        }
    }

    fn tick(&mut self, cx: &mut Context<EventsView>) {
        if self.ticking {
            return;
        }
        self.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let open = this
                    .update(cx, |view, cx| {
                        if !view.open {
                            view.ticking = false;
                            return false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn clear(&mut self, cx: &mut Context<EventsView>) {
        self.notice = None;
        if let Some(token) = self.events.clear() {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(UNDO_MS))
                    .await;
                this.update(cx, |view, cx| {
                    if view.events.expire_undo(token) {
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    pub fn undo(&mut self, cx: &mut Context<EventsView>) {
        if self.events.undo_clear() {
            cx.notify();
        }
    }

    pub fn click(&mut self, index: usize, cx: &mut Context<EventsView>) {
        let Some(item) = self.events.items.get(index).cloned() else {
            return;
        };
        if let Some(Action::Resume(offer)) = item.action {
            cx.emit(EventsEvent::Resume(offer));
            self.close(cx);
            return;
        }
        let terminal = terminal_of(&item);
        if terminal.is_empty() {
            return;
        }
        if !self.live.contains(&terminal) {
            self.notice = Some(ENDED.into());
            cx.notify();
            return;
        }
        cx.emit(EventsEvent::Focus { terminal });
        self.close(cx);
    }

    pub fn run(&mut self, command: EventsCommand, cx: &mut Context<EventsView>) {
        match command {
            EventsCommand::Show => self.show(cx),
            EventsCommand::Clear => self.clear(cx),
            EventsCommand::Undo => self.undo(cx),
            EventsCommand::AgentVoiceOn => self.set_agent_voice(true, cx),
            EventsCommand::AgentVoiceOff => self.set_agent_voice(false, cx),
        }
    }
}

impl Render for EventsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render(self, cx)
    }
}

pub fn new(cx: &mut App) -> Entity<EventsView> {
    cx.new(EventsView::empty)
}

pub fn dot_color(t: &Theme, kind: &str) -> Hsla {
    match kind {
        "blocked" | "error" => status_color(t, Status::Blocked),
        "done" => status_color(t, Status::Done),
        "working" => status_color(t, Status::Working),
        _ => t.accent,
    }
}

fn head_button(t: &Theme, id: &'static str, label: &'static str, tip: &'static str, color: Hsla) -> Stateful<Div> {
    let hover = t.hover;
    let bright = t.foreground;
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .h(px(22.))
        .px(px(8.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .text_size(px(11.5))
        .text_color(color)
        .cursor_pointer()
        .tooltip(tooltip(tip))
        .hover(move |s| s.bg(hover).text_color(bright))
        .child(label)
}

fn row(t: &Theme, view: &EventsView, index: usize, item: &Item, now: u64, cx: &mut Context<EventsView>) -> Stateful<Div> {
    let text = row_text(item, &view.numbers);
    let body = row_body(item);
    let meta = row_meta(item, &view.numbers);
    let hover = t.hover;
    div()
        .id(("event-row", index))
        .debug_selector(move || format!("event-row-{index}"))
        .w_full()
        .flex()
        .flex_row()
        .flex_none()
        .px(px(9.))
        .py(px(6.))
        .gap(px(8.))
        .items_start()
        .rounded(px(6.))
        .cursor_pointer()
        .tooltip(tooltip(row_tip(item, &view.numbers)))
        .hover(move |s| s.bg(hover))
        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.click(index, cx)))
        .child(
            div()
                .mt(px(5.))
                .size(px(7.))
                .flex_none()
                .rounded(px(4.))
                .bg(dot_color(t, &item.kind)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(t.foreground)
                        .child(SharedString::from(text)),
                )
                .when(!body.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.muted)
                            .child(SharedString::from(body)),
                    )
                })
                .when(!meta.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.dim)
                            .truncate()
                            .child(SharedString::from(meta)),
                    )
                }),
        )
        .child(
            div()
                .mt(px(2.))
                .flex_none()
                .text_size(px(10.5))
                .text_color(t.dim)
                .child(SharedString::from(super::model::ago(item.time, now))),
        )
}

pub fn render(view: &mut EventsView, cx: &mut Context<EventsView>) -> Stateful<Div> {
    let t = theme(cx).clone();
    let now = view.now();
    let rows: Vec<Stateful<Div>> = view
        .events
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| row(&t, view, i, item, now, cx))
        .collect();
    let empty = rows.is_empty();
    let header = div()
        .flex()
        .flex_row()
        .flex_none()
        .px(px(6.))
        .h(px(24.))
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .flex_1()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child("Events"),
        )
        .child(
            head_button(&t, "events-clear", "Clear", "Remove every event", t.muted)
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.clear(cx))),
        )
        .when(view.events.undo.is_some(), |el| {
            el.child(
                head_button(&t, "events-undo", "Undo", "Put the cleared events back", t.accent)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.undo(cx))),
            )
        })
        .child(
            head_button(&t, "events-close", "×", "Close (Esc)", t.muted)
                .text_size(px(13.))
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.close(cx))),
        );
    popover(&t)
        .id("events")
        .debug_selector(|| "events".into())
        .key_context("Events")
        .track_focus(&view.focus)
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .gap(px(4.))
        .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" {
                view.close(cx);
            }
        }))
        .child(header)
        .when_some(view.notice.clone(), |el, notice| {
            el.child(
                div()
                    .id("events-notice")
                    .debug_selector(|| "events-notice".into())
                    .px(px(9.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(t.muted)
                    .child(SharedString::from(notice)),
            )
        })
        .child(if empty {
            div()
                .id("events-empty")
                .debug_selector(|| "events-empty".into())
                .flex_1()
                .min_h(px(0.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(t.dim)
                .child("No events yet")
        } else {
            div()
                .id("event-list")
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .overflow_y_scroll()
                .track_scroll(&view.scroll)
                .children(rows)
        })
}
