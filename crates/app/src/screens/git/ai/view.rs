use super::menu::{AgentMenu, AgentMenuEvent};
use super::model::{self, Deps, Poll, Request};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClipboardItem, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, Hsla, KeyDownEvent, ScrollHandle, SharedString, Stateful, Subscription, Window, div, px,
};
use nixa_git::ai::report::{BlockKind, Severity};
use nixa_git::ai::{AiError, Cancel, ErrorKind, Tone, cancel, parse_report};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::popover::tooltip;
use nixa_ui::{Theme, text, theme};
use std::sync::atomic::Ordering;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Window,
    Overlay,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiResultEvent {
    Close,
    OpenSettings,
}

pub struct AiResultView {
    pub mode: Mode,
    pub request: Request,
    deps: Deps,
    pub run: u64,
    pub busy: bool,
    pub stopping: bool,
    pub text: String,
    pub error: Option<String>,
    pub missing: bool,
    pub agent: String,
    pub model: String,
    pub copied: bool,
    pub took: u64,
    pub note: Option<String>,
    pub menu: Option<Entity<AgentMenu>>,
    stop_flag: Option<Cancel>,
    began: Instant,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _menu_events: Option<Subscription>,
}

impl EventEmitter<AiResultEvent> for AiResultView {}

impl Focusable for AiResultView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn severity_color(t: &Theme, s: Severity) -> (Hsla, &'static str) {
    match s {
        Severity::Bug => (t.red, "Bug"),
        Severity::Risk => (t.yellow, "Risk"),
        Severity::Nit => (t.dim, "Nit"),
    }
}

fn tone_color(t: &Theme, tone: Tone) -> Hsla {
    match tone {
        Tone::Success => t.green,
        Tone::Danger => t.red,
        Tone::Attention => t.yellow,
    }
}

fn chip(t: &Theme, id: &'static str, label: String) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_none()
        .px(px(7.0))
        .py(px(2.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(t.border)
        .bg(t.raised)
        .text_size(px(text::SM))
        .text_color(t.muted)
        .child(label)
}

fn named(id: &'static str, el: Stateful<Div>) -> Stateful<Div> {
    el.debug_selector(move || id.into())
}

impl AiResultView {
    pub fn new(mode: Mode, request: Request, deps: Deps, cx: &mut Context<Self>) -> AiResultView {
        cx.on_release(|view: &mut AiResultView, _| view.cancel_job()).detach();
        let mut view = AiResultView {
            mode,
            request,
            deps,
            run: 0,
            busy: false,
            stopping: false,
            text: String::new(),
            error: None,
            missing: false,
            agent: String::new(),
            model: String::new(),
            copied: false,
            took: 0,
            note: None,
            menu: None,
            stop_flag: None,
            began: Instant::now(),
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _menu_events: None,
        };
        view.start(cx);
        view
    }

    fn cancel_job(&mut self) {
        if let Some(flag) = &self.stop_flag {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.run += 1;
        let run = self.run;
        let flag = cancel();
        self.busy = true;
        self.stopping = false;
        self.text.clear();
        self.error = None;
        self.missing = false;
        self.copied = false;
        self.note = None;
        self.took = 0;
        self.began = Instant::now();
        self.stop_flag = Some(flag.clone());
        let deps = self.deps.clone();
        let request = self.request.clone();
        let (tx, labels) = std::sync::mpsc::channel();
        let rx = model::off_thread(move || {
            if request.repo.is_none() {
                return Err(AiError::new(ErrorKind::Nothing, model::NO_REPO, None));
            }
            let settings = (deps.load)();
            let _ = tx.send(model::agent_label(&settings));
            let opts = nixa_git::ai::Opts {
                cancel: flag,
                ..Default::default()
            };
            (deps.ask)(&request, &settings, &opts)
        });
        cx.spawn(async move |this, cx| {
            loop {
                if let Ok((agent, model)) = labels.try_recv() {
                    let alive = this
                        .update(cx, |v, cx| {
                            if v.run == run {
                                v.agent = agent;
                                v.model = model;
                                cx.notify();
                            }
                        })
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
                match model::poll(&rx) {
                    Poll::Ready(result) => {
                        this.update(cx, |v, cx| v.finish(run, result, cx)).ok();
                        break;
                    }
                    Poll::Waiting => cx.background_executor().timer(model::POLL).await,
                    Poll::Gone => {
                        this.update(cx, |v, cx| {
                            v.finish(run, Err(AiError::new(ErrorKind::Failed, "The AI job ended without an answer.", None)), cx)
                        })
                        .ok();
                        break;
                    }
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn finish(&mut self, run: u64, result: Result<String, AiError>, cx: &mut Context<Self>) {
        if run != self.run {
            return;
        }
        match result {
            Ok(text) => self.text = text,
            Err(e) => {
                self.missing = e.kind == ErrorKind::Missing;
                self.error = Some(model::error_text(&e));
            }
        }
        self.busy = false;
        self.stopping = false;
        self.stop_flag = None;
        self.took = model::took_secs(self.began.elapsed().as_millis());
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.busy || self.stopping {
            return;
        }
        self.cancel_job();
        self.stopping = true;
        cx.notify();
    }

    pub fn copy(&mut self, cx: &mut Context<Self>) {
        if self.text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.text.clone()));
        self.copied = true;
        cx.notify();
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_job();
        cx.emit(AiResultEvent::Close);
        if self.mode == Mode::Window {
            window.remove_window();
        }
    }

    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self._menu_events = None;
        cx.emit(AiResultEvent::OpenSettings);
        if self.mode == Mode::Window {
            super::open_settings(cx);
        }
        cx.notify();
    }

    pub fn change_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.close_menu(window, cx);
            return;
        }
        let deps = self.deps.clone();
        let menu = cx.new(|cx| AgentMenu::new(deps, cx));
        let events = cx.subscribe_in(&menu, window, |view, _, event: &AgentMenuEvent, window, cx| match event {
            AgentMenuEvent::Picked(id) => {
                let label = nixa_git::ai::agent_by_id(id).map(|a| a.label).unwrap_or(id.as_str()).to_string();
                if !view.busy {
                    view.agent = label.clone();
                    view.model.clear();
                }
                view.note = Some(format!("Press Retry to ask {label}."));
                view.close_menu(window, cx);
            }
            AgentMenuEvent::Settings => view.open_settings(cx),
            AgentMenuEvent::Close => view.close_menu(window, cx),
        });
        window.focus(&menu.focus_handle(cx));
        self.menu = Some(menu);
        self._menu_events = Some(events);
        cx.notify();
    }

    pub fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self._menu_events = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.keystroke.key == "escape" {
            if self.menu.is_some() {
                self.close_menu(window, cx);
            } else {
                self.close(window, cx);
            }
            cx.stop_propagation();
        }
    }

    fn answer(&self, t: &Theme) -> AnyElement {
        let report = parse_report(&self.text);
        let verdict = report.verdict.clone().map(|v| {
            let tone = tone_color(t, v.tone);
            div()
                .id("ai-result-verdict")
                .debug_selector(|| "ai-result-verdict".into())
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(7.0))
                .border_1()
                .border_color(tone)
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tone)
                .child(format!("Verdict: {}", v.text))
        });
        let mut parts: Vec<AnyElement> = Vec::new();
        for section in &report.sections {
            if let Some(label) = &section.label {
                parts.push(
                    div()
                        .pt(px(4.0))
                        .text_size(px(text::BASE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.bright)
                        .child(label.clone())
                        .into_any_element(),
                );
            }
            for block in &section.blocks {
                let row = match block.kind {
                    BlockKind::Paragraph => div().text_color(t.foreground).child(block.text.clone()),
                    BlockKind::Bullet => {
                        let mark = block.severity.map(|s| {
                            let (color, word) = severity_color(t, s);
                            nixa_ui::chip::chip(word, color)
                        });
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(6.0))
                            .items_start()
                            .child(div().flex_none().text_color(t.dim).child("•"))
                            .when_some(mark, |el, m| el.child(m))
                            .child(div().flex_1().min_w_0().text_color(t.foreground).child(block.text.clone()))
                    }
                };
                parts.push(row.into_any_element());
            }
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .when_some(verdict, |el, v| el.child(v))
            .children(parts)
            .into_any_element()
    }

    fn body(&self, t: &Theme) -> AnyElement {
        if self.busy {
            return div()
                .id("ai-result-waiting")
                .debug_selector(|| "ai-result-waiting".into())
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().text_color(t.accent).child("◌"))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(t.muted)
                        .child(model::waiting_text(&self.agent, self.stopping)),
                )
                .into_any_element();
        }
        if let Some(error) = &self.error {
            return div()
                .id("ai-result-error")
                .debug_selector(|| "ai-result-error".into())
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(div().text_color(t.red).child(error.clone()))
                .child(text::muted(t, model::error_hint(self.missing)))
                .into_any_element();
        }
        self.answer(t)
    }

    fn buttons(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        if self.busy {
            let label = if self.stopping { "Stopping…" } else { "Stop" };
            out.push(
                disabled(named("ai-result-stop", button(t, "ai-result-stop", label, ButtonKind::Danger)), self.stopping)
                    .on_click(cx.listener(|v, _, _, cx| v.stop(cx)))
                    .into_any_element(),
            );
        }
        if !self.busy && !self.text.is_empty() {
            let label = if self.copied { "Copied" } else { "Copy" };
            out.push(
                named("ai-result-copy", button(t, "ai-result-copy", label, ButtonKind::Secondary))
                    .on_click(cx.listener(|v, _, _, cx| v.copy(cx)))
                    .into_any_element(),
            );
        }
        if !self.busy {
            out.push(
                named("ai-result-retry", button(t, "ai-result-retry", "Retry", ButtonKind::Secondary))
                    .on_click(cx.listener(|v, _, _, cx| v.start(cx)))
                    .into_any_element(),
            );
        }
        out.push(
            named("ai-result-change", button(t, "ai-result-change", "Change agent", ButtonKind::Secondary))
                .on_click(cx.listener(|v, _, window, cx| v.change_agent(window, cx)))
                .into_any_element(),
        );
        out.push(
            named("ai-result-close", button(t, "ai-result-close", "Close", ButtonKind::Primary))
                .on_click(cx.listener(|v, _, window, cx| v.close(window, cx)))
                .into_any_element(),
        );
        out
    }
}

impl Render for AiResultView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let agent_chip = (!self.agent.is_empty()).then(|| {
            let hover = t.hover;
            chip(&t, "ai-result-agent", model::chip_text(&self.agent, &self.model))
                .text_color(t.accent)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .tooltip(tooltip("Change the agent in AI settings"))
                .on_click(cx.listener(|v, _, _, cx| v.open_settings(cx)))
        });
        let meta = div()
            .flex_none()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .child(chip(&t, "ai-result-target", model::target_text(&self.request)))
            .when_some(agent_chip, |el, c| el.child(c))
            .when(!self.busy && self.took > 0, |el| el.child(text::caption(&t, format!("{} s", self.took))));
        let body = self.body(&t);
        let buttons = self.buttons(&t, cx);
        let note = self.note.clone();
        let menu = self.menu.clone();
        let scroll = self.scroll.clone();
        div()
            .id("ai-result-view")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.8))
            .child(
                div()
                    .flex_none()
                    .px(px(16.0))
                    .pt(px(14.0))
                    .pb(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .border_b_1()
                    .border_color(t.border)
                    .child(text::heading(&t, SharedString::from(model::heading(self.request.kind))))
                    .child(meta),
            )
            .child(
                div().flex_1().min_h_0().p(px(14.0)).flex().flex_col().child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.surface)
                        .child(nixa_ui::scroll::scroll_area(
                            &t,
                            "ai-result-body",
                            &scroll,
                            div().p(px(12.0)).child(body),
                        ).flex_1()),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(16.0))
                    .py(px(12.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .border_t_1()
                    .border_color(t.border)
                    .bg(t.chrome)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(text::SM))
                            .text_color(t.muted)
                            .children(note),
                    )
                    .children(buttons),
            )
            .when_some(menu, |el, m| {
                el.child(div().absolute().right(px(16.0)).bottom(px(58.0)).child(m))
            })
    }
}
