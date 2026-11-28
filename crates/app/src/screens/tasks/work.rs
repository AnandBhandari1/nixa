use super::drop::Drop;
use crate::screens::{events::Level, git::join::toast};
use super::model::{self, Task};
use super::view::{Panel, TasksView, area_value, set, set_area, value};
use gpui::{Context, Focusable, Window};
use serde_json::{Value, json};

impl TasksView {
    pub fn report_error(&mut self, error: String, cx: &mut Context<Self>) {
        toast("Tasks", &error, Level::Error, cx);
        self.error = error;
    }

    pub fn apply_task(&mut self, data: &Value, cx: &mut Context<Self>) {
        let Some(raw) = model::get(data, &["task"]) else {
            return;
        };
        let Some(id) = model::parse_task(raw).map(|t| t.id) else {
            return;
        };
        for task in self.tasks.iter_mut().filter(|t| t.id == id) {
            *task = model::merge_task(task, raw);
        }
        let merged = self
            .detail
            .as_ref()
            .filter(|d| d.task.id == id)
            .map(|d| model::merge_task(&d.task, raw));
        if let Some(task) = merged {
            self.fill_text(&task, false, cx);
            if let Some(detail) = self.detail.as_mut() {
                detail.task = task;
            }
        }
    }

    pub fn set_filter(&mut self, which: Drop, picked: &str, cx: &mut Context<Self>) {
        let f = &mut self.filters;
        let slot = match which {
            Drop::FSpace => &mut f.space,
            Drop::FStatus => &mut f.status,
            Drop::FGroup => &mut f.group,
            Drop::FSource => &mut f.source,
            Drop::FAssigned => &mut f.assigned,
            _ => return,
        };
        if slot == picked {
            return;
        }
        *slot = picked.to_string();
        self.reload(cx);
    }

    pub fn toggle_done(&mut self, cx: &mut Context<Self>) {
        self.filters.done = !self.filters.done;
        self.reload(cx);
    }

    pub fn toggle_archived(&mut self, cx: &mut Context<Self>) {
        self.filters.archived = !self.filters.archived;
        self.reload(cx);
    }

    pub fn update_task(&mut self, patch: Value, cx: &mut Context<Self>) {
        self.act("update", patch, cx, |view, data, cx| {
            view.apply_task(&data, cx)
        });
    }

    pub fn set_group(&mut self, task: &Task, group: &str, cx: &mut Context<Self>) {
        if let Some(patch) = model::group_patch(task, group, &self.options.spaces) {
            self.update_task(patch, cx);
        }
    }

    pub fn workflow_act(&mut self, id: i64, what: &str, cx: &mut Context<Self>) {
        self.act(
            "workflow",
            json!({"id": id, "do": what}),
            cx,
            move |view, data, cx| {
                view.confirm_run_cancel = None;
                view.apply_task(&data, cx);
                view.load_detail(id, cx);
            },
        );
    }

    pub fn close_task(&mut self, task: &Task, status: &str, cx: &mut Context<Self>) {
        let undo = model::undo_of(task, status);
        self.act(
            "update",
            json!({"id": task.id, "status": status}),
            cx,
            move |view, data, cx| {
                view.apply_task(&data, cx);
                view.undo = Some(undo);
                view.reload_soon(cx);
            },
        );
    }

    pub fn undo_close(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.undo.clone() else {
            return;
        };
        self.act(
            "update",
            json!({"id": undo.id, "status": undo.back}),
            cx,
            |view, data, cx| {
                view.apply_task(&data, cx);
                view.undo = None;
                view.reload_soon(cx);
            },
        );
    }

    pub fn run_task(&mut self, task: &Task, cx: &mut Context<Self>) {
        if !model::ready_to_run(task) {
            self.open_task(task.id, cx);
            self.report_error(model::needs_pick_text(task.id), cx);
            return;
        }
        self.start(task.id, cx);
    }

    pub fn start(&mut self, id: i64, cx: &mut Context<Self>) {
        self.act("start", json!({"id": id}), cx, |view, data, cx| {
            view.apply_task(&data, cx);
            view.reload_soon(cx);
        });
    }

    pub fn release(&mut self, id: i64, cx: &mut Context<Self>) {
        self.act("release", json!({"id": id}), cx, |view, data, cx| {
            view.apply_task(&data, cx);
            view.reload_soon(cx);
        });
    }

    pub fn continue_task(&mut self, task: &Task, assign: &str, cx: &mut Context<Self>) {
        let mut args = json!({"id": task.id});
        if !assign.is_empty() {
            args["assign"] = json!(assign);
        }
        let id = task.id;
        self.act("continue", args, cx, move |view, data, cx| {
            view.confirm_handover = None;
            view.apply_task(&data, cx);
            let terminal = model::text(&data, &["terminal"]);
            let note = model::continue_note(
                &model::text(&data, &["how"]),
                if terminal.is_empty() { "?" } else { &terminal },
            );
            if !note.is_empty() {
                view.note = format!("#{id}: {note}");
                toast("Tasks", &view.note, Level::Success, cx);
            }
            view.reload_soon(cx);
        });
    }

    pub fn open_task(&mut self, id: i64, cx: &mut Context<Self>) {
        let known = self.tasks.iter().find(|t| t.id == id).cloned();
        self.panel = Some(Panel::Detail);
        self.selected = Some(id);
        self.confirm_delete = None;
        self.confirm_handover = None;
        self.confirm_run_cancel = None;
        self.detail_error.clear();
        self.open = None;
        self.detail = known.map(|task| model::Detail {
            task,
            comments: Vec::new(),
            run: None,
        });
        if let Some(task) = self.detail.as_ref().map(|d| d.task.clone()) {
            self.fill_text(&task, true, cx);
        }
        set_area(&self.inputs.comment, "", cx);
        self.load_detail(id, cx);
        cx.notify();
    }

    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.panel = None;
        self.selected = None;
        self.detail = None;
        self.detail_error.clear();
        self.confirm_delete = None;
        self.confirm_handover = None;
        self.confirm_run_cancel = None;
        self.open = None;
        cx.notify();
    }

    pub fn show_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel == Some(Panel::New) {
            self.close_panel(cx);
            return;
        }
        self.close_panel(cx);
        self.panel = Some(Panel::New);
        let active = (self.host.active_space)(cx);
        let space = if self.options.spaces.iter().any(|s| s.id == active) {
            active
        } else {
            String::new()
        };
        self.draft = model::empty_draft(&space);
        set(&self.inputs.new_title, "", cx);
        set_area(&self.inputs.new_description, "", cx);
        window.focus(&self.inputs.new_title.focus_handle(cx));
    }

    pub fn text_changed(&self, cx: &gpui::App) -> bool {
        let Some(task) = self.shown_task() else {
            return false;
        };
        value(&self.inputs.title, cx).trim() != task.title
            || area_value(&self.inputs.description, cx) != task.description
    }

    pub fn undo_text_edit(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.shown_task().cloned() else {
            return;
        };
        set(&self.inputs.title, &task.title, cx);
        set_area(&self.inputs.description, &task.description, cx);
    }

    pub fn save_text(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.shown_task().cloned() else {
            return;
        };
        let title = value(&self.inputs.title, cx).trim().to_string();
        let description = area_value(&self.inputs.description, cx);
        if title.is_empty() {
            self.report_error("A task needs a title.".into(), cx);
            cx.notify();
            return;
        }
        if let Some(patch) = model::text_patch(&task, &title, &description) {
            self.update_task(patch, cx);
        }
    }

    pub fn add_comment(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.shown_task().map(|t| t.id) else {
            return;
        };
        let text = area_value(&self.inputs.comment, cx).trim().to_string();
        if text.is_empty() {
            self.report_error("Type the comment first.".into(), cx);
            cx.notify();
            return;
        }
        let args = json!({"id": id, "kind": self.comment_kind, "text": text});
        self.act("comment", args, cx, move |view, _, cx| {
            set_area(&view.inputs.comment, "", cx);
            view.load_detail(id, cx);
        });
    }

    pub fn add_task(&mut self, start: bool, cx: &mut Context<Self>) {
        let title = value(&self.inputs.new_title, cx).trim().to_string();
        if title.is_empty() {
            self.report_error("Type a title for the new task.".into(), cx);
            cx.notify();
            return;
        }
        let description = area_value(&self.inputs.new_description, cx)
            .trim()
            .to_string();
        let args = model::add_args(&title, &description, &self.draft);
        self.act("add", args, cx, move |view, data, cx| {
            set(&view.inputs.new_title, "", cx);
            set_area(&view.inputs.new_description, "", cx);
            view.draft = model::empty_draft("");
            let task = model::get(&data, &["task"]).and_then(model::parse_task);
            let Some(mut task) = task else {
                view.close_panel(cx);
                return;
            };
            task.comments = 0;
            let id = task.id;
            view.tasks.retain(|t| t.id != id);
            view.tasks.push(task);
            view.open_task(id, cx);
            if start {
                view.start(id, cx);
            }
        });
    }

    pub fn archive_task(&mut self, id: i64, on: bool, cx: &mut Context<Self>) {
        let action = if on { "archive" } else { "unarchive" };
        self.act(action, json!({"id": id}), cx, move |view, _, cx| {
            view.tasks.retain(|t| t.id != id);
            view.close_panel(cx);
        });
    }

    pub fn delete_task(&mut self, id: i64, cx: &mut Context<Self>) {
        self.act(
            "delete",
            json!({"id": id, "confirm": true}),
            cx,
            move |view, _, cx| {
                view.tasks.retain(|t| t.id != id);
                view.close_panel(cx);
            },
        );
    }

    pub fn go_terminal(&mut self, terminal: &str, cx: &mut Context<Self>) {
        if !(self.host.focus)(terminal, cx) {
            self.report_error(format!("Terminal {terminal} is not running any more."), cx);
        }
    }

    pub fn read_task(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.reading {
            return;
        }
        self.reading = true;
        self.error.clear();
        let call = self.call.clone();
        let begin = {
            let call = call.clone();
            cx.background_executor().spawn(async move { call("speak", json!({"id": id})) })
        };
        cx.spawn(async move |this, cx| {
            let begun = begin.await;
            let read = this.update(cx, |view, cx| {
                view.read_id = match begun {
                    Ok(data) => data.get("read_id").and_then(Value::as_u64),
                    Err(error) => {
                        view.report_error(error, cx);
                        None
                    }
                };
                view.reading = view.read_id.is_some();
                cx.notify();
                view.read_id
            }).ok().flatten();
            let Some(read) = read else { return };
            let done = cx.background_executor()
                .spawn(async move { call("speak_wait", json!({"read": read})) }).await;
            this.update(cx, |view, cx| {
                view.reading = false;
                view.read_id = None;
                if let Err(error) = done {
                    view.report_error(error, cx);
                }
                cx.notify();
            }).ok();
        })
        .detach();
    }

    pub fn stop_reading(&mut self, cx: &mut Context<Self>) {
        let Some(read) = self.read_id else { return };
        let call = self.call.clone();
        let stop = cx.background_executor()
            .spawn(async move { call(super::api::STOP_ACTION, json!({"read": read})) });
        cx.spawn(async move |this, cx| {
            if let Err(error) = stop.await {
                this.update(cx, |view, cx| {
                    view.report_error(error, cx);
                    cx.notify();
                }).ok();
            }
        })
        .detach();
    }

    pub fn open_link(&mut self, url: &str, cx: &mut Context<Self>) {
        if model::safe_link(url) {
            (self.host.open_url)(url, cx);
        } else {
            self.report_error(format!("This is not a web or mail link: {url}"), cx);
        }
    }

    pub fn changed(&mut self, note: &Value, cx: &mut Context<Self>) {
        if let Some(n) = note.get("waiting").and_then(Value::as_u64) {
            self.waiting = n as usize;
        }
        self.reload_soon(cx);
        cx.notify();
    }

    pub fn pick(
        &mut self,
        which: Drop,
        picked: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let task = self.shown_task().cloned();
        match which {
            Drop::FSpace | Drop::FStatus | Drop::FGroup | Drop::FSource | Drop::FAssigned => {
                self.set_filter(which, picked, cx)
            }
            Drop::DKind => self.comment_kind = picked.to_string(),
            Drop::DHandover => {
                if let Some(task) = task {
                    self.confirm_handover = Some((task.id, picked.to_string()));
                }
            }
            Drop::NSpace => self.draft.space = picked.to_string(),
            Drop::NAssign => self.draft.assign = picked.to_string(),
            Drop::NThen => self.draft.then = picked.to_string(),
            Drop::NGroup => {
                self.draft.group = picked.to_string();
                if picked == "nixa-gap" {
                    let nixa = model::nixa_space_id(&self.options.spaces);
                    if !nixa.is_empty() {
                        self.draft.space = nixa;
                    }
                }
            }
            _ => {
                if let Some(task) = task {
                    self.pick_field(&task, which, picked, cx);
                }
            }
        }
    }

    fn pick_field(&mut self, task: &Task, which: Drop, picked: &str, cx: &mut Context<Self>) {
        let id = task.id;
        let or_none = |s: &str| {
            if s.is_empty() {
                "none".to_string()
            } else {
                s.to_string()
            }
        };
        match which {
            Drop::DStatus if picked != task.status => {
                self.update_task(json!({"id": id, "status": picked}), cx)
            }
            Drop::DOn if picked != task.blocked_on => {
                self.update_task(json!({"id": id, "status": "blocked", "on": picked}), cx)
            }
            Drop::DSpace if picked != or_none(&task.space_id) => {
                self.update_task(json!({"id": id, "space": picked}), cx)
            }
            Drop::DAssign if picked != or_none(&model::assigned_value(&self.options, task)) => {
                if model::agent_held(task) && picked != "none" {
                    self.confirm_handover = Some((id, picked.to_string()));
                } else {
                    self.update_task(json!({"id": id, "assign": picked}), cx);
                }
            }
            Drop::DThen if picked != model::then_value(&self.options, task) => {
                self.update_task(json!({"id": id, "then": or_none(picked)}), cx)
            }
            Drop::DGroup => self.set_group(task, picked, cx),
            Drop::DWorkflow if picked != or_none(&task.workflow) => {
                self.update_task(json!({"id": id, "workflow": picked}), cx)
            }
            _ => {}
        }
    }
}
