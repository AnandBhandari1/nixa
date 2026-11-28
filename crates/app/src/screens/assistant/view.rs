use super::api::{Call, Host};
use super::model::{self, State};
use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, SharedString, Subscription, Window, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use serde_json::Value;
use std::time::Duration;

pub const REFRESH: Duration = Duration::from_secs(5);

pub struct Inputs {
    pub ask: Entity<TextInput>,
    pub time: Entity<TextInput>,
    pub place: Entity<TextInput>,
    pub topics: Entity<TextInput>,
}

pub struct AssistantView {
    pub call: Call,
    pub host: Host,
    pub state: Option<State>,
    pub inputs: Inputs,
    pub answer: Option<String>,
    pub notice: String,
    pub error: Option<String>,
    pub asking: bool,
    pub saving: bool,
    cancelling: bool,
    pub filled: String,
    pub seq: u64,
    pub focus: FocusHandle,
    pub focus_next: Option<Entity<TextInput>>,
    _subs: Vec<Subscription>,
}

impl Focusable for AssistantView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn value(input: &Entity<TextInput>, cx: &gpui::App) -> String {
    input.read(cx).value().to_string()
}

pub fn set(input: &Entity<TextInput>, text: &str, cx: &mut gpui::App) {
    let text = text.to_string();
    input.update(cx, |i, cx| i.set_value(text, cx));
}

impl AssistantView {
    pub fn new(call: Call, host: Host, cx: &mut Context<Self>) -> Self {
        let inputs = Inputs {
            ask: cx.new(|cx| TextInput::new(cx, model::PLACEHOLDER)),
            time: cx.new(|cx| TextInput::new(cx, model::TIME_HINT)),
            place: cx.new(|cx| TextInput::new(cx, model::PLACE_HINT)),
            topics: cx.new(|cx| TextInput::new(cx, model::TOPICS_HINT)),
        };
        let subs = vec![
            cx.subscribe(&inputs.ask, |view, _, event: &InputEvent, cx| {
                if let InputEvent::Submit(_) = event {
                    view.ask(cx);
                }
            }),
            cx.subscribe(&inputs.time, Self::form_key),
            cx.subscribe(&inputs.place, Self::form_key),
            cx.subscribe(&inputs.topics, Self::form_key),
        ];
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                let alive = this.update(cx, |view, cx| {
                    if !view.asking && !view.saving && !view.cancelling {
                        view.load(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        poll.detach();
        let mut view = AssistantView {
            call,
            host,
            state: None,
            inputs,
            answer: None,
            notice: model::NOTICE.into(),
            error: None,
            asking: false,
            saving: false,
            cancelling: false,
            filled: String::new(),
            seq: 0,
            focus: cx.focus_handle(),
            focus_next: None,
            _subs: subs,
        };
        view.focus_next = Some(view.inputs.ask.clone());
        view.load(cx);
        view
    }

    fn form_key(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        if let InputEvent::Submit(_) = event {
            self.save_briefing(cx);
        }
    }

    pub fn job(&mut self, body: Value, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, Result<Value, String>, &mut Context<Self>) + 'static) {
        let call = self.call.clone();
        let task = cx.background_executor().spawn(async move { call(body) });
        cx.spawn(async move |this, cx| {
            let out = task.await;
            this.update(cx, |view, cx| {
                then(view, out, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn take(&mut self, out: &Value, cx: &mut Context<Self>) {
        let Some(state) = out.get("view").and_then(model::parse) else { return };
        if let Some(b) = &state.briefing
            && self.filled != b.id
        {
            self.filled = b.id.clone();
            set(&self.inputs.time, &b.time, cx);
            set(&self.inputs.place, &b.place, cx);
            set(&self.inputs.topics, &b.topics.join(", "), cx);
        }
        self.state = Some(state);
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.seq += 1;
        let seq = self.seq;
        self.job(model::view_body(), cx, move |view, out, cx| {
            if view.seq != seq {
                return;
            }
            match out {
                Ok(out) => {
                    view.error = None;
                    view.take(&out, cx);
                }
                Err(why) => view.error = Some(why),
            }
        });
    }

    pub fn ask(&mut self, cx: &mut Context<Self>) {
        let question = value(&self.inputs.ask, cx);
        if question.trim().is_empty() || self.asking || self.cancelling {
            return;
        }
        self.asking = true;
        self.answer = None;
        self.notice = model::ASKING.into();
        self.seq += 1;
        self.job(model::ask_body(&question), cx, |view, out, cx| {
            view.asking = false;
            match out {
                Ok(out) => {
                    view.error = None;
                    view.answer = out.get("answer").and_then(Value::as_str).map(str::to_string);
                    view.notice = out.get("notice").and_then(Value::as_str).unwrap_or(model::NOTICE).to_string();
                    set(&view.inputs.ask, "", cx);
                    view.take(&out, cx);
                }
                Err(why) => {
                    view.notice = if why.starts_with("cannot reach nixa-brain at ") {
                        "Could not connect to Nixa."
                    } else {
                        "Nixa could not complete the request."
                    }.into();
                    view.error = Some(why);
                }
            }
        });
        cx.notify();
    }

    pub fn save_briefing(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.cancelling {
            return;
        }
        let enabled = self.state.as_ref().and_then(|s| s.briefing.as_ref()).is_none_or(|b| b.enabled);
        let body = model::briefing_body(&value(&self.inputs.time, cx), &value(&self.inputs.place, cx), &value(&self.inputs.topics, cx), enabled);
        self.saved_job(body, cx);
    }

    pub fn toggle_briefing(&mut self, cx: &mut Context<Self>) {
        let Some(on) = self.state.as_ref().and_then(|s| s.briefing.as_ref()).map(|b| b.enabled) else { return };
        if self.saving || self.cancelling {
            return;
        }
        self.saved_job(model::on_body(!on), cx);
    }

    fn saved_job(&mut self, body: Value, cx: &mut Context<Self>) {
        self.saving = true;
        self.seq += 1;
        self.notice = model::SAVING.into();
        self.job(body, cx, |view, out, cx| {
            view.saving = false;
            match out {
                Ok(out) => {
                    view.error = None;
                    view.notice = out.get("text").and_then(Value::as_str).unwrap_or(model::NOTICE).to_string();
                    view.filled.clear();
                    view.take(&out, cx);
                }
                Err(why) => {
                    view.notice = "The briefing was not saved.".into();
                    view.error = Some(why);
                }
            }
        });
        cx.notify();
    }

    fn cancel_reminder(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.cancelling || self.saving || self.asking {
            return;
        }
        self.cancelling = true;
        self.seq += 1;
        self.error = None;
        self.notice = "Cancelling reminder…".into();
        self.job(model::cancel_body(id), cx, |view, out, cx| {
            view.cancelling = false;
            match out {
                Ok(out) => {
                    view.notice = out.get("text").and_then(Value::as_str).unwrap_or("Reminder cancelled.").into();
                    view.take(&out, cx);
                }
                Err(why) => {
                    view.notice = "Could not cancel the reminder.".into();
                    view.error = Some(why);
                }
            }
        });
        cx.notify();
    }

    pub fn start_with(&mut self, what: &str, cx: &mut Context<Self>) {
        self.focus_next = Some(if what == "briefing" { self.inputs.place.clone() } else { self.inputs.ask.clone() });
        cx.notify();
    }

    fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = self.focus_next.take() {
            window.focus(&input.focus_handle(cx));
        }
    }
}

fn heading(t: &Theme, text: &'static str) -> AnyElement {
    div().pt(px(6.)).font_weight(FontWeight::SEMIBOLD).text_color(t.bright).child(text).into_any_element()
}

fn small(t: &Theme, text: impl Into<SharedString>) -> AnyElement {
    div().text_size(px(12.)).text_color(t.muted).child(text.into()).into_any_element()
}

impl AssistantView {
    fn ask_part(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let label = if self.asking { model::ASKING } else { "Ask Nixa" };
        let mut part = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .items_center()
                    .child(div().flex_1().min_w_0().child(self.inputs.ask.clone()))
                    .child(
                        disabled(button(t, "assistant-ask", label, ButtonKind::Primary), self.asking)
                            .debug_selector(|| "assistant-ask".into())
                            .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.ask(cx))),
                    ),
            );
        if let Some(answer) = &self.answer {
            part = part.child(
                div()
                    .id("assistant-answer")
                    .debug_selector(|| "assistant-answer".into())
                    .p(px(10.))
                    .rounded(px(6.))
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.border)
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .children(answer.lines().map(|l| div().child(l.to_string()))),
            );
        }
        part.child(div().id("assistant-notice").debug_selector(|| "assistant-notice".into()).text_size(px(12.)).text_color(t.muted).child(self.notice.clone()))
            .into_any_element()
    }

    fn briefing_part(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let briefing = self.state.as_ref().and_then(|s| s.briefing.clone());
        let line = if self.state.is_none() { model::LOADING.to_string() } else { model::briefing_line(briefing.as_ref()) };
        let mut buttons = div().flex().flex_row().gap(px(8.)).child(
            disabled(button(t, "assistant-save-briefing", if briefing.is_some() { "Save briefing" } else { "Set up morning briefing" }, ButtonKind::Secondary), self.saving)
                .debug_selector(|| "assistant-save-briefing".into())
                .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.save_briefing(cx))),
        );
        if let Some(b) = &briefing {
            buttons = buttons.child(
                disabled(button(t, "assistant-briefing-on", if b.enabled { "Pause" } else { "Turn on" }, ButtonKind::Ghost), self.saving)
                    .debug_selector(|| "assistant-briefing-on".into())
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.toggle_briefing(cx))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(heading(t, "Morning briefing"))
            .child(small(t, model::BRIEFING_HINT))
            .child(div().id("assistant-briefing-line").debug_selector(|| "assistant-briefing-line".into()).child(line))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .child(div().w(px(72.)).flex_none().child(self.inputs.time.clone()))
                    .child(div().flex_1().min_w_0().child(self.inputs.place.clone()))
                    .child(div().flex_1().min_w_0().child(self.inputs.topics.clone())),
            )
            .child(buttons)
            .into_any_element()
    }

    fn reminders_part(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let list = self.state.as_ref().map(|s| s.reminders.as_slice()).unwrap_or_default();
        let busy = self.cancelling || self.saving || self.asking;
        let mut part = div().flex().flex_col().gap(px(4.)).child(heading(t, "Reminders"));
        if list.is_empty() {
            return part.child(small(t, model::NO_REMINDERS)).into_any_element();
        }
        for r in list {
            let id = format!("assistant-reminder-{}", r.task);
            let selector = id.clone();
            let cancel = format!("assistant-reminder-cancel-{}", r.task);
            let schedule = r.id.clone();
            let line = model::reminder_line(r);
            part = part.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .id(SharedString::from(id))
                            .debug_selector(move || selector.clone())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .tooltip(tooltip(line.clone()))
                            .child(line),
                    )
                    .when(!schedule.is_empty(), |row| row.child(
                        disabled(button(t, SharedString::from(cancel.clone()), "Cancel reminder", ButtonKind::Secondary), busy)
                            .debug_selector(move || cancel.clone())
                            .tooltip(tooltip("Cancel this reminder and close its task"))
                            .when(!busy, |button| {
                                button.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.cancel_reminder(&schedule, cx)))
                            }),
                    )),
            );
        }
        part.into_any_element()
    }

    fn footer(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let sources = self.state.as_ref().map(|s| model::sources_line(&s.sources)).unwrap_or_default();
        let zone = self.state.as_ref().map(|s| s.zone.clone()).filter(|z| !z.is_empty()).map(|z| format!("Time zone: {z}.")).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .child(
                        button(t, "assistant-tasks", "Tasks", ButtonKind::Secondary)
                            .debug_selector(|| "assistant-tasks".into())
                            .on_click(cx.listener(|v, _: &ClickEvent, _, cx| (v.host.tasks)(cx))),
                    )
                    .child(
                        button(t, "assistant-schedules", "Schedules", ButtonKind::Secondary)
                            .debug_selector(|| "assistant-schedules".into())
                            .on_click(cx.listener(|v, _: &ClickEvent, _, cx| (v.host.schedules)(cx))),
                    ),
            )
            .child(small(t, format!("{sources} {zone}").trim().to_string()))
            .child(small(t, model::EXAMPLES))
            .into_any_element()
    }
}

impl Render for AssistantView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.take_focus(window, cx);
        let t = theme(cx).clone();
        let error = self.error.clone();
        div()
            .id("assistant")
            .debug_selector(|| "assistant".into())
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(18.))
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(13.))
            .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(19.)).child(super::TITLE))
            .child(div().child(model::INTRO))
            .child(self.ask_part(&t, cx))
            .when_some(error, |el, e| el.child(div().id("assistant-error").debug_selector(|| "assistant-error".into()).text_color(t.red).text_size(px(12.)).child(e)))
            .child(self.briefing_part(&t, cx))
            .child(self.reminders_part(&t, cx))
            .child(self.footer(&t, cx))
    }
}
