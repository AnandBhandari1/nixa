use super::api::{Call, Host};
use super::drop::Drop;
use super::model::{self, Detail, Draft, Filters, Options, Sort, Task, Undo};
use crate::actions::ClosePane;
use crate::screens::settings::parts::memory::text_area::{AreaEvent, TextArea};
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, KeyDownEvent, Subscription, Task as GpuiTask,
    Window, div, px,
};
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::theme;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub const RELOAD_GAP: Duration = Duration::from_millis(300);
pub const RETRY: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Detail,
    New,
}

pub struct Inputs {
    pub search: Entity<TextInput>,
    pub pick: Entity<TextInput>,
    pub title: Entity<TextInput>,
    pub description: Entity<TextArea>,
    pub description_preview: Entity<TextArea>,
    pub comment: Entity<TextArea>,
    pub new_title: Entity<TextInput>,
    pub new_description: Entity<TextArea>,
}

pub struct TasksView {
    pub call: Call,
    pub host: Host,
    pub connected: bool,
    pub loaded: bool,
    pub loading: bool,
    pub load_error: String,
    pub error: String,
    pub note: String,
    pub busy: String,
    pub options: Options,
    pub tasks: Vec<Task>,
    pub waiting: usize,
    pub filters: Filters,
    pub grouped: bool,
    pub sort: Sort,
    pub panel: Option<Panel>,
    pub selected: Option<i64>,
    pub detail: Option<Detail>,
    pub detail_error: String,
    pub text_base: (String, String),
    pub raw_text: Option<i64>,
    pub reading: bool,
    pub read_id: Option<u64>,
    pub comment_kind: String,
    pub confirm_delete: Option<i64>,
    pub confirm_handover: Option<(i64, String)>,
    pub confirm_run_cancel: Option<i64>,
    pub draft: Draft,
    pub open: Option<Drop>,
    pub just_closed: Option<(Drop, Instant)>,
    pub undo: Option<Undo>,
    pub inputs: Inputs,
    pub load_seq: u64,
    pub reload_gen: u64,
    pub title_shown: String,
    pub offset_min: i64,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
    _poll: GpuiTask<()>,
}

fn input(cx: &mut Context<TasksView>, placeholder: &'static str) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(cx, placeholder))
}

fn area(cx: &mut Context<TasksView>, placeholder: &'static str, rows: (u16, u16)) -> Entity<TextArea> {
    cx.new(|cx| TextArea::new(cx, placeholder).auto_rows(rows.0, rows.1))
}

pub fn value(input: &Entity<TextInput>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

pub fn area_value(area: &Entity<TextArea>, cx: &App) -> String {
    area.read(cx).value().to_string()
}

pub fn set(input: &Entity<TextInput>, text: &str, cx: &mut App) {
    let text = text.to_string();
    input.update(cx, |i, cx| i.set_value(text, cx));
}

pub fn set_area(area: &Entity<TextArea>, text: &str, cx: &mut App) {
    let text = text.to_string();
    area.update(cx, |i, cx| i.set_value(text, cx));
}

const SEARCH_HINT: &str = "Search title or text";

impl TasksView {
    pub fn new(call: Call, host: Host, offset_min: i64, cx: &mut Context<Self>) -> Self {
        let inputs = Inputs {
            search: input(cx, SEARCH_HINT),
            pick: input(cx, "Search"),
            title: input(cx, "Task title"),
            description: area(cx, "What needs to be done", (3, 10)),
            description_preview: cx.new(|cx| TextArea::new(cx, "").preview().auto_rows(1, 10)),
            comment: area(cx, "Write a comment", (2, 8)),
            new_title: input(cx, "What is the task?"),
            new_description: area(cx, "More detail (optional)", (3, 10)),
        };
        let subs = vec![
            cx.subscribe(&inputs.search, |view, _, e: &InputEvent, cx| match e {
                InputEvent::Changed(_) => view.reload_soon(cx),
                InputEvent::Submit(_) => view.reload(cx),
                InputEvent::Cancel => {}
            }),
            cx.subscribe(&inputs.pick, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&inputs.title, |view, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Submit(_)) {
                    view.save_text(cx);
                }
                cx.notify();
            }),
            cx.subscribe(&inputs.new_title, |view, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Submit(_)) {
                    view.add_task(false, cx);
                }
                cx.notify();
            }),
            cx.subscribe(&inputs.description, |view, _, e: &AreaEvent, cx| {
                if matches!(e, AreaEvent::Save) {
                    view.save_text(cx);
                }
                cx.notify();
            }),
            cx.subscribe(&inputs.comment, |view, _, e: &AreaEvent, cx| {
                if matches!(e, AreaEvent::Save) {
                    view.add_comment(cx);
                }
                cx.notify();
            }),
            cx.subscribe(&inputs.new_description, |view, _, e: &AreaEvent, cx| {
                if matches!(e, AreaEvent::Save) {
                    view.add_task(false, cx);
                }
                cx.notify();
            }),
        ];
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(RETRY).await;
                let alive = this.update(cx, |view, cx| {
                    if !view.connected && !view.loading {
                        view.reload(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut view = TasksView {
            call,
            host,
            connected: false,
            loaded: false,
            loading: false,
            load_error: String::new(),
            error: String::new(),
            note: String::new(),
            busy: String::new(),
            options: model::parse_options(&Value::Null),
            tasks: Vec::new(),
            waiting: 0,
            filters: Filters::default(),
            grouped: true,
            sort: Sort::default(),
            panel: None,
            selected: None,
            detail: None,
            detail_error: String::new(),
            text_base: (String::new(), String::new()),
            raw_text: None,
            reading: false,
            read_id: None,
            comment_kind: "note".into(),
            confirm_delete: None,
            confirm_handover: None,
            confirm_run_cancel: None,
            draft: model::empty_draft(""),
            open: None,
            just_closed: None,
            undo: None,
            inputs,
            load_seq: 0,
            reload_gen: 0,
            title_shown: String::new(),
            offset_min,
            focus: cx.focus_handle(),
            _subs: subs,
            _poll: poll,
        };
        view.reload(cx);
        view
    }

    pub fn search_text(&self, cx: &App) -> String {
        value(&self.inputs.search, cx).trim().to_string()
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load_seq += 1;
        let seq = self.load_seq;
        self.loading = true;
        let call = self.call.clone();
        let args = model::list_args(&self.filters, &self.search_text(cx));
        let job = cx.background_executor().spawn(async move {
            let options = call("options", json!({}));
            let listed = options
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|_| call("list", args));
            let waiting = listed
                .as_ref()
                .ok()
                .and_then(|_| call("list", json!({"status": "blocked,review"})).ok());
            (options, listed, waiting)
        });
        cx.spawn(async move |this, cx| {
            let (options, listed, waiting) = job.await;
            this.update(cx, |view, cx| {
                if seq != view.load_seq {
                    return;
                }
                view.loading = false;
                match (options, listed) {
                    (Ok(options), Ok(listed)) => {
                        view.options = model::parse_options(&options);
                        view.tasks = model::parse_tasks(&listed);
                        view.loaded = true;
                        view.connected = true;
                        view.load_error.clear();
                        if let Some(waiting) = waiting {
                            view.waiting = model::waiting_count(&model::parse_tasks(&waiting));
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        view.connected = !model::offline(&e);
                        view.load_error = e;
                    }
                }
                if view.panel == Some(Panel::Detail)
                    && let Some(id) = view.selected
                {
                    view.load_detail(id, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn reload_soon(&mut self, cx: &mut Context<Self>) {
        self.reload_gen += 1;
        let gen_now = self.reload_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RELOAD_GAP).await;
            this.update(cx, |view, cx| {
                if view.reload_gen == gen_now {
                    view.reload(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn load_options(&mut self, cx: &mut Context<Self>) {
        let call = self.call.clone();
        let job = cx
            .background_executor()
            .spawn(async move { call("options", json!({})) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                if let Ok(v) = out {
                    view.options.workflows = model::parse_flows(&v);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn load_detail(&mut self, id: i64, cx: &mut Context<Self>) {
        let call = self.call.clone();
        let job = cx
            .background_executor()
            .spawn(async move { call("show", json!({"id": id})) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                if view.selected != Some(id) {
                    return;
                }
                match out.map(|v| model::parse_detail(&v)) {
                    Ok(Some(detail)) => {
                        let fresh = view.detail.as_ref().is_none_or(|d| d.task.id != id);
                        view.fill_text(&detail.task, fresh, cx);
                        view.detail = Some(detail);
                        view.detail_error.clear();
                    }
                    Ok(None) => {}
                    Err(e) => view.detail_error = e,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn fill_text(&mut self, task: &Task, fresh: bool, cx: &mut Context<Self>) {
        let (base_title, base_description) = self.text_base.clone();
        if fresh || value(&self.inputs.title, cx) == base_title {
            set(&self.inputs.title, &task.title, cx);
        }
        if fresh || area_value(&self.inputs.description, cx) == base_description {
            set_area(&self.inputs.description, &task.description, cx);
        }
        if fresh || task.description != base_description {
            let (text, spans) = if model::is_html(&task.description) {
                super::description::parse(&task.description)
            } else {
                (String::new(), Vec::new())
            };
            self.inputs.description_preview.update(cx, |preview, cx| preview.set_preview(text, spans, cx));
        }
        self.text_base = (task.title.clone(), task.description.clone());
    }

    pub fn act(
        &mut self,
        action: &'static str,
        args: Value,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut TasksView, Value, &mut Context<TasksView>) + 'static,
    ) {
        if !self.busy.is_empty() {
            return;
        }
        self.busy = action.to_string();
        self.error.clear();
        self.note.clear();
        let call = self.call.clone();
        let job = cx
            .background_executor()
            .spawn(async move { call(action, args) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                view.busy.clear();
                match out {
                    Ok(data) => done(view, data, cx),
                    Err(e) => view.report_error(e, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn is_busy(&self, action: &str) -> bool {
        self.busy == action
    }

    pub fn idle(&self) -> bool {
        self.busy.is_empty()
    }

    pub fn shown_task(&self) -> Option<&Task> {
        self.detail.as_ref().map(|d| &d.task)
    }

    fn escape(&mut self, cx: &mut Context<Self>) {
        if self.open.is_some() {
            self.open = None;
        } else if self.confirm_delete.is_some()
            || self.confirm_handover.is_some()
            || self.confirm_run_cancel.is_some()
        {
            self.confirm_delete = None;
            self.confirm_handover = None;
            self.confirm_run_cancel = None;
        } else if self.panel.is_some() {
            self.close_panel(cx);
        }
        cx.notify();
    }
}

impl Focusable for TasksView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TasksView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let title = model::window_title(self.waiting);
        if self.title_shown != title {
            window.set_window_title(&title);
            self.title_shown = title;
        }
        let top = self.top_bar(&t, cx);
        let filters = self.loaded.then(|| self.filter_bar(&t, cx));
        let notices = self.notices(&t, cx);
        let body = self.body(&t, cx);
        div()
            .id("nixa-tasks")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.5))
            .on_action(cx.listener(|_, _: &ClosePane, window, _| window.remove_window()))
            .on_key_down(cx.listener(|view, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    view.escape(cx);
                }
            }))
            .child(top)
            .children(filters)
            .child(notices)
            .child(body)
    }
}
