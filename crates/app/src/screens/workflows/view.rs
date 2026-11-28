use super::api::Call;
use super::flow::{self, Info, Opt, Options, Run, RunInfo, Spot, Workflow};
use super::geo::{self, Cam, ZOOM_IN, ZOOM_OUT};
use super::gestures::BoxDrag;
use crate::screens::settings::parts::memory::text_area::{AreaEvent, TextArea};
use gpui::prelude::*;
use gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollWheelEvent, Subscription, Task, Window, px,
};
use nixa_ui::text_input::{InputEvent, TextInput};
use serde_json::json;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

pub const RETRY: Duration = Duration::from_secs(3);
pub const STEP: f64 = 20.0;
pub const BIG_STEP: f64 = 80.0;
pub const DRAG_MIN: f32 = 3.0;
pub const WHEEL_ZOOM: f32 = 0.003;
pub const DIRTY: &str = "Save or revert the open workflow first.";
pub const BAD_NAME: &str = "Name: small letters, numbers and dashes.";

#[derive(Clone, Debug, PartialEq)]
pub enum Purpose {
    Who { id: String, fb: bool },
    Model { id: String, fb: bool },
    Effort { id: String, fb: bool },
    Skills { id: String },
    Tools { id: String },
    Gate { id: String },
    LinkKind { id: String },
    LinkTo { from: String, to: String },
    Target { id: String, kind: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    pub title: String,
    pub purpose: Purpose,
    pub options: Vec<Opt>,
    pub multi: bool,
    pub chosen: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    Prompt(String),
    Details,
    Box(String),
    Reports,
    Copy,
    New,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunView {
    pub task: i64,
    pub run: Run,
    pub reports: Vec<flow::Report>,
}

#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub at: Point<Pixels>,
    pub cam: Cam,
    pub moved: bool,
}

pub struct Inputs {
    pub search: Entity<TextInput>,
    pub filter: Entity<TextInput>,
    pub name: Entity<TextInput>,
    pub title: Entity<TextInput>,
    pub when: Entity<TextInput>,
    pub copy: Entity<TextInput>,
    pub step_limit: Entity<TextInput>,
    pub box_title: Entity<TextInput>,
    pub prompt: Entity<TextArea>,
}

pub struct WorkflowsView {
    pub call: Call,
    pub booted: bool,
    pub link_error: String,
    pub names: Vec<Info>,
    pub name: String,
    pub flow: Option<Workflow>,
    pub saved: String,
    pub can_undo_saved: bool,
    pub deleted: Option<String>,
    pub runs: Vec<RunInfo>,
    pub run_view: Option<RunView>,
    pub confirm_cancel: bool,
    pub options: Options,
    pub problems: Vec<String>,
    pub check_seq: u64,
    pub undo: Vec<String>,
    pub redo: Vec<String>,
    pub selected: Option<String>,
    pub picker: Option<Picker>,
    pub modal: Option<Modal>,
    pub form_error: String,
    pub confirm_delete: bool,
    pub error: String,
    pub note: String,
    pub busy: String,
    pub cam: Option<Cam>,
    pub fitted: bool,
    pub moved: bool,
    pub drag: Option<Drag>,
    pub box_drag: Option<BoxDrag>,
    pub shape: String,
    pub auto: BTreeMap<String, Spot>,
    pub spots: BTreeMap<String, Spot>,
    pub size: Option<(f32, f32)>,
    pub area: Rc<Cell<(f32, f32, f32, f32)>>,
    pub focus: FocusHandle,
    pub canvas_focus: FocusHandle,
    pub focus_next: Option<FocusHandle>,
    pub inputs: Inputs,
    pub start: Option<String>,
    pub reading: bool,
    _subs: Vec<Subscription>,
    _poll: Task<()>,
}

impl Focusable for WorkflowsView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

fn input(cx: &mut Context<WorkflowsView>, placeholder: &'static str) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(cx, placeholder))
}

pub fn value(input: &Entity<TextInput>, cx: &gpui::App) -> String {
    input.read(cx).value().to_string()
}

pub(super) fn set(input: &Entity<TextInput>, text: &str, cx: &mut gpui::App) {
    let text = text.to_string();
    input.update(cx, |i, cx| i.set_value(text, cx));
}

impl WorkflowsView {
    pub fn new(call: Call, cx: &mut Context<Self>) -> Self {
        let inputs = Inputs {
            search: input(cx, "Search workflows"),
            filter: input(cx, "Type to filter"),
            name: input(cx, "Name: small letters, numbers, dashes"),
            title: input(cx, "Title"),
            when: input(cx, "When to use it, one line"),
            copy: input(cx, "Name of the copy: small letters, numbers, dashes"),
            step_limit: input(cx, "Maximum steps"),
            box_title: input(cx, "Box title"),
            prompt: cx
                .new(|cx| TextArea::new(cx, "What this box does. Plain English.").limit(4000)),
        };
        let subs = vec![
            cx.subscribe(&inputs.search, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(
                &inputs.filter,
                |view, _, event: &InputEvent, cx| match event {
                    InputEvent::Cancel => view.close_picker(cx),
                    InputEvent::Submit(_) => view.pick_only(cx),
                    InputEvent::Changed(_) => cx.notify(),
                },
            ),
            cx.subscribe(&inputs.name, Self::form_key),
            cx.subscribe(&inputs.title, Self::form_key),
            cx.subscribe(&inputs.when, Self::form_key),
            cx.subscribe(&inputs.copy, Self::form_key),
            cx.subscribe(&inputs.step_limit, Self::form_key),
            cx.subscribe(&inputs.box_title, Self::form_key),
            cx.subscribe(
                &inputs.prompt,
                |view, _, event: &AreaEvent, cx| match event {
                    AreaEvent::Save => view.use_prompt(cx),
                    AreaEvent::Cancel => view.close_modal(cx),
                    AreaEvent::Changed(_) => {}
                },
            ),
        ];
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(RETRY).await;
                let alive = this.update(cx, |view, cx| {
                    if !view.booted && view.busy.is_empty() {
                        view.boot(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut view = WorkflowsView {
            call,
            booted: false,
            link_error: String::new(),
            names: Vec::new(),
            name: String::new(),
            flow: None,
            saved: String::new(),
            can_undo_saved: false,
            deleted: None,
            runs: Vec::new(),
            run_view: None,
            confirm_cancel: false,
            options: Options::default(),
            problems: Vec::new(),
            check_seq: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            selected: None,
            picker: None,
            modal: None,
            form_error: String::new(),
            confirm_delete: false,
            error: String::new(),
            note: String::new(),
            busy: String::new(),
            cam: None,
            fitted: false,
            moved: false,
            drag: None,
            box_drag: None,
            shape: String::new(),
            auto: BTreeMap::new(),
            spots: BTreeMap::new(),
            size: None,
            area: Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0))),
            focus: cx.focus_handle(),
            canvas_focus: cx.focus_handle(),
            focus_next: None,
            inputs,
            start: None,
            reading: false,
            _subs: subs,
            _poll: poll,
        };
        view.boot(cx);
        view
    }

    fn form_key(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Submit(_) => match self.modal {
                Some(Modal::New) => self.create_flow(cx),
                Some(Modal::Copy) => self.copy_flow(cx),
                Some(Modal::Details | Modal::Box(_)) => self.apply_details(cx),
                _ => {}
            },
            InputEvent::Cancel => self.close_modal(cx),
            InputEvent::Changed(_) => {}
        }
    }

    pub fn job<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Call) -> T + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        let call = self.call.clone();
        let task = cx.background_executor().spawn(async move { work(&call) });
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

    pub fn dirty(&self) -> bool {
        self.flow
            .as_ref()
            .is_some_and(|f| flow::to_text(f) != self.saved)
    }

    pub fn editable(&self) -> bool {
        self.flow.is_some() && self.run_view.is_none()
    }

    pub fn boot(&mut self, cx: &mut Context<Self>) {
        if !self.busy.is_empty() {
            return;
        }
        self.busy = "boot".into();
        self.job(
            cx,
            |call| {
                let options = call("options", json!({}));
                let list = options.as_ref().ok().map(|_| call("list", json!({})));
                let runs = options.as_ref().ok().map(|_| call("runs", json!({})));
                (options, list, runs)
            },
            |view, (options, list, runs), cx| {
                view.busy.clear();
                match options {
                    Ok(value) => {
                        view.booted = true;
                        view.link_error.clear();
                        view.options = flow::parse_options(&value).unwrap_or_default();
                    }
                    Err(e) => {
                        view.link_error = e;
                        return;
                    }
                }
                match list {
                    Some(Ok(value)) => view.names = flow::parse_names(&value),
                    Some(Err(e)) => view.error = e,
                    None => {}
                }
                match runs {
                    Some(Ok(value)) => view.runs = flow::parse_runs(&value),
                    Some(Err(e)) => view.error = e,
                    None => {}
                }
                match view.names.first().map(|n| n.name.clone()) {
                    Some(first) if view.name.is_empty() => view.open_flow(&first, false, cx),
                    _ => view.run_pending(cx),
                }
            },
        );
    }

    pub fn load_names(&mut self, cx: &mut Context<Self>) {
        self.job(
            cx,
            |call| call("list", json!({})),
            |view, out, _| match out {
                Ok(value) => view.names = flow::parse_names(&value),
                Err(e) => view.error = e,
            },
        );
    }

    pub fn load_runs(&mut self, cx: &mut Context<Self>) {
        self.job(
            cx,
            |call| call("runs", json!({})),
            |view, out, cx| match out {
                Ok(value) => {
                    view.runs = flow::parse_runs(&value);
                    if let Some(task) = view.run_view.as_ref().map(|r| r.task) {
                        view.open_run(task, true, cx);
                    }
                }
                Err(e) => view.error = e,
            },
        );
    }

    pub fn changed(&mut self, name: Option<&str>, cx: &mut Context<Self>) {
        if self.busy.is_empty()
            && !self.reading
            && self.run_view.is_none()
            && !self.dirty()
            && !self.name.is_empty()
            && name.is_none_or(|n| n.is_empty() || n == self.name)
        {
            let name = self.name.clone();
            self.open_flow(&name, true, cx);
        }
        self.load_names(cx);
    }

    pub fn reset_canvas(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.selected = None;
        self.box_drag = None;
        self.shape.clear();
        self.fitted = false;
    }

    pub fn open_flow(&mut self, name: &str, quiet: bool, cx: &mut Context<Self>) {
        if name.is_empty() {
            return;
        }
        if self.run_view.is_some() {
            self.run_view = None;
            self.confirm_cancel = false;
        }
        let name = name.to_string();
        let args = json!({"name": name});
        self.reading = true;
        self.job(
            cx,
            move |call| call("read", args),
            move |view, out, cx| match out {
                Ok(value) => {
                    view.reading = false;
                    let Some(flow) = flow::parse_workflow(&value) else {
                        view.error = format!("The brain sent no workflow for {name}.");
                        view.run_pending(cx);
                        return;
                    };
                    view.name = name;
                    view.saved = flow::to_text(&flow);
                    view.flow = Some(flow);
                    view.can_undo_saved = flow::can_undo(&value);
                    view.reset_canvas();
                    view.confirm_delete = false;
                    if !quiet {
                        view.error.clear();
                        view.note.clear();
                    }
                    view.recheck(cx);
                    if let Some(start) = view.start.take() {
                        view.start_with(&start, cx);
                    }
                }
                Err(e) => {
                    view.reading = false;
                    view.error = e;
                    view.run_pending(cx);
                }
            },
        );
    }

    pub fn waits_for_flow(&self) -> bool {
        !self.booted || self.busy == "boot" || self.reading
    }

    pub fn run_pending(&mut self, cx: &mut Context<Self>) {
        if self.flow.is_some() {
            return;
        }
        if let Some(start) = self.start.take() {
            self.start_without_flow(&start, cx);
        }
    }

    pub fn start_without_flow(&mut self, what: &str, cx: &mut Context<Self>) {
        match what.strip_prefix("run:").and_then(|t| t.parse().ok()) {
            Some(task) => self.open_run(task, false, cx),
            None => self.open_new(cx),
        }
    }

    pub fn start_with(&mut self, what: &str, cx: &mut Context<Self>) {
        if self.flow.is_none() {
            if self.waits_for_flow() {
                self.start = Some(what.to_string());
            } else {
                self.start_without_flow(what, cx);
            }
            return;
        }
        let stage = self.flow.as_ref().and_then(|f| {
            flow::ids(f)
                .into_iter()
                .find(|id| f.nodes.get(id).is_some_and(|n| n.kind == "stage"))
        });
        match what {
            "new" => self.open_new(cx),
            "picker" => {
                if let Some(id) = stage {
                    self.selected = Some(id.clone());
                    self.pick_who(&id, false, cx);
                }
            }
            "job" => {
                if let Some(id) = stage {
                    self.edit_prompt(&id, cx);
                }
            }
            other => {
                if let Some(task) = other.strip_prefix("run:").and_then(|t| t.parse().ok()) {
                    self.open_run(task, false, cx);
                }
            }
        }
    }

    pub fn open_run(&mut self, task: i64, quiet: bool, cx: &mut Context<Self>) {
        self.job(
            cx,
            move |call| call("run", json!({"task": task})),
            move |view, out, _| match out {
                Ok(value) => {
                    let Some((run, wf)) = flow::parse_run(&value) else {
                        if !quiet {
                            view.error = "That run has no saved copy.".into();
                        }
                        return;
                    };
                    let same = view.run_view.as_ref().is_some_and(|r| r.task == task);
                    let started = run.started as i64;
                    let line = run.line.clone();
                    if !wf.name.is_empty() {
                        view.name = wf.name.clone();
                    }
                    view.saved = flow::to_text(&wf);
                    view.flow = Some(wf);
                    view.can_undo_saved = false;
                    view.run_view = Some(RunView { task, run, reports: flow::parse_reports(&value) });
                    view.confirm_delete = false;
                    if !same {
                        view.reset_canvas();
                        view.confirm_cancel = false;
                        view.problems.clear();
                    }
                    if !quiet {
                        view.error.clear();
                        let ago = crate::screens::custom_agents::model::ago(
                            nixa_store::time::now_ms(),
                            started,
                        );
                        view.note = if started > 0 {
                            format!(
                                "Task #{task}: this run uses the copy of the workflow from when it started ({ago}). Read only."
                            )
                        } else {
                            format!("Task #{task}: this run uses the copy of the workflow from when it started. Read only.")
                        };
                        if !line.is_empty() {
                            view.note = format!("{} {line}", view.note);
                        }
                    }
                }
                Err(e) => view.error = e,
            },
        );
    }

    pub fn close_run(&mut self, cx: &mut Context<Self>) {
        self.run_view = None;
        self.confirm_cancel = false;
        self.note.clear();
        let name = std::mem::take(&mut self.name);
        self.flow = None;
        let pick = if name.is_empty() {
            self.names.first().map(|n| n.name.clone())
        } else {
            Some(name)
        };
        if let Some(name) = pick {
            self.open_flow(&name, false, cx);
        }
        cx.notify();
    }

    pub fn run_act(&mut self, action: &'static str, text: &str, cx: &mut Context<Self>) {
        let Some(task) = self.run_view.as_ref().map(|r| r.task) else {
            return;
        };
        if !self.busy.is_empty() {
            return;
        }
        self.busy = action.into();
        self.error.clear();
        let args = if text.is_empty() {
            json!({"task": task})
        } else {
            json!({"task": task, "text": text})
        };
        cx.notify();
        self.job(
            cx,
            move |call| call(action, args),
            |view, out, cx| {
                view.busy.clear();
                match out {
                    Ok(value) => {
                        view.note = flow::text_of(&value);
                        view.confirm_cancel = false;
                        view.load_runs(cx);
                    }
                    Err(e) => view.error = e,
                }
            },
        );
    }

    pub fn recheck(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = self.flow.as_ref() else {
            return;
        };
        self.check_seq += 1;
        let seq = self.check_seq;
        let args = json!({"name": self.name, "workflow": flow::to_value(flow)});
        self.job(
            cx,
            move |call| call("check", args),
            move |view, out, _| {
                if seq != view.check_seq {
                    return;
                }
                if let Ok(value) = out {
                    view.problems = flow::parse_problems(&value);
                }
            },
        );
    }

    pub fn edit(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Workflow, &mut Option<String>),
    ) {
        if !self.editable() {
            return;
        }
        let before = self.flow.as_ref().map(flow::to_text);
        self.freeze();
        let Some((before, flow)) = before.zip(self.flow.as_mut()) else {
            return;
        };
        self.undo.push(before);
        if self.undo.len() > flow::MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        f(flow, &mut self.selected);
        self.shape.clear();
        self.note.clear();
        self.recheck(cx);
        cx.notify();
    }

    pub fn step_back(&mut self, redo: bool, cx: &mut Context<Self>) {
        if self.run_view.is_some() {
            return;
        }
        let snap = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        let (Some(snap), Some(flow)) = (snap, self.flow.as_ref()) else {
            return;
        };
        let now = flow::to_text(flow);
        if redo {
            self.undo.push(now);
        } else {
            self.redo.push(now);
        }
        if let Ok(back) = serde_json::from_str::<Workflow>(&snap) {
            if self
                .selected
                .as_ref()
                .is_some_and(|s| !back.nodes.contains_key(s))
            {
                self.selected = None;
            }
            self.flow = Some(back);
        }
        self.shape.clear();
        self.recheck(cx);
        cx.notify();
    }

    pub fn place(&mut self) {
        let Some(flow) = self.flow.as_ref() else {
            self.spots.clear();
            return;
        };
        let run = self.run_view.is_some();
        let shape = geo::shape(flow, run, &self.options);
        if shape != self.shape {
            self.shape = shape;
            self.auto = geo::auto_spots(flow, run, &self.options);
        }
        self.spots = geo::spots(flow, &self.auto);
        if let Some(gesture) = self.box_drag.as_ref()
            && let Some(spot) = gesture.box_spot()
        {
            self.spots.insert(gesture.id.clone(), spot);
        }
    }

    pub fn freeze(&mut self) {
        self.place();
        let spots = self.spots.clone();
        if let Some(flow) = self.flow.as_mut() {
            for (id, p) in spots {
                if flow.nodes.contains_key(&id) {
                    flow.layout.entry(id).or_insert(Spot {
                        x: p.x.round(),
                        y: p.y.round(),
                    });
                }
            }
        }
    }

    pub fn fit(&mut self) {
        let (Some(size), true) = (self.size, self.flow.is_some()) else {
            return;
        };
        self.place();
        let Some(flow) = self.flow.as_ref() else {
            return;
        };
        if let Some(b) = geo::bounds(flow, &self.spots, self.run_view.is_some(), &self.options) {
            self.cam = Some(geo::fit(size, b));
            self.moved = false;
        }
    }

    pub fn zoom(&mut self, f: f32) {
        if let (Some(cam), Some((w, h))) = (self.cam, self.size) {
            self.cam = Some(geo::zoom_at(cam, f, w / 2.0, h / 2.0));
            self.moved = true;
        }
    }

    pub fn auto_layout(&mut self, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        self.edit(cx, |f, _| f.layout.clear());
        self.shape.clear();
        self.fitted = false;
        self.fit();
        cx.notify();
    }

    pub fn add_box(&mut self, cx: &mut Context<Self>) {
        let near = self
            .selected
            .as_ref()
            .and_then(|s| self.spots.get(s))
            .copied()
            .unwrap_or_default();
        self.edit(cx, move |f, selected| {
            let id = flow::add_box(f, near);
            *selected = Some(id);
        });
    }

    pub fn remove_box(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if !self.flow.as_ref().is_some_and(|f| flow::removable(f, &id)) {
            return;
        }
        self.edit(cx, move |f, selected| {
            flow::remove_box(f, &id);
            *selected = None;
        });
    }

    pub fn move_selected(&mut self, dx: f64, dy: f64, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let Some(id) = self.selected.clone() else {
            return;
        };
        if !self
            .flow
            .as_ref()
            .is_some_and(|f| f.nodes.contains_key(&id))
        {
            return;
        }
        self.freeze();
        let spot = self.spots.get(&id).copied().unwrap_or_default();
        if let Some(f) = self.flow.as_mut() {
            let p = f.layout.get(&id).copied().unwrap_or(spot);
            f.layout.insert(
                id,
                Spot {
                    x: (p.x + dx).round(),
                    y: (p.y + dy).round(),
                },
            );
        }
        self.recheck(cx);
        cx.notify();
    }

    pub fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        self.selected = Some(id.to_string());
        self.focus_next = Some(self.canvas_focus.clone());
        cx.notify();
    }

    pub fn open_picker(
        &mut self,
        title: String,
        purpose: Purpose,
        options: Vec<Opt>,
        chosen: Option<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        if self.run_view.is_some() {
            return;
        }
        self.picker = Some(Picker {
            title,
            purpose,
            options,
            multi: chosen.is_some(),
            chosen: chosen.unwrap_or_default(),
        });
        set(&self.inputs.filter, "", cx);
        self.focus_next = Some(self.inputs.filter.focus_handle(cx));
        cx.notify();
    }

    pub fn close_picker(&mut self, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            self.focus_next = Some(self.canvas_focus.clone());
        }
        cx.notify();
    }

    pub fn close_modal(&mut self, cx: &mut Context<Self>) {
        if self.saving_form() {
            return;
        }
        if self.modal.take().is_some() {
            self.form_error.clear();
            self.focus_next = Some(self.canvas_focus.clone());
        }
        cx.notify();
    }

    pub fn shown_options(&self, cx: &gpui::App) -> Vec<(usize, Opt)> {
        let Some(p) = self.picker.as_ref() else {
            return Vec::new();
        };
        let q = value(&self.inputs.filter, cx);
        flow::filter_options(&p.options, &q)
            .into_iter()
            .map(|(i, o)| (i, o.clone()))
            .collect()
    }

    fn pick_only(&mut self, cx: &mut Context<Self>) {
        let shown = self.shown_options(cx);
        if self.picker.as_ref().is_some_and(|p| !p.multi) && shown.len() == 1 {
            let v = shown[0].1.value.clone();
            self.pick(v, cx);
        }
    }

    pub fn toggle_chosen(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Some(p) = self.picker.as_mut() {
            if let Some(at) = p.chosen.iter().position(|c| c == value) {
                p.chosen.remove(at);
            } else {
                p.chosen.push(value.to_string());
            }
        }
        cx.notify();
    }

    pub fn clear_chosen(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.picker.as_mut() {
            p.chosen.clear();
        }
        cx.notify();
    }

    pub fn pick_many(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.picker.take() else {
            return;
        };
        let list = p.chosen.clone();
        match p.purpose {
            Purpose::Skills { id } => self.edit(cx, move |f, _| {
                if let Some(n) = f.nodes.get_mut(&id) {
                    n.skills = list;
                }
            }),
            Purpose::Tools { id } => self.edit(cx, move |f, _| {
                if let Some(n) = f.nodes.get_mut(&id) {
                    n.tools = list;
                }
            }),
            _ => {}
        }
        self.focus_next = Some(self.canvas_focus.clone());
        cx.notify();
    }

    pub fn pick(&mut self, v: String, cx: &mut Context<Self>) {
        let Some(p) = self.picker.take() else {
            return;
        };
        self.focus_next = Some(self.canvas_focus.clone());
        match p.purpose {
            Purpose::Who { id, fb } => {
                let who = flow::who_from(&v);
                let kind = flow::kind_of(&self.options, &who);
                let key = id.clone();
                self.edit(cx, move |f, _| {
                    if let Some(n) = f.nodes.get_mut(&key) {
                        if fb {
                            n.fallback.who = who;
                            n.fallback.model.clear();
                            n.fallback.effort.clear();
                        } else {
                            n.who = who;
                            n.model.clear();
                            n.effort.clear();
                        }
                    }
                });
                if !kind.is_empty() && flow::models_of(&self.options, &kind).next().is_some() {
                    self.pick_model(&id, fb, cx);
                }
            }
            Purpose::Model { id, fb } => {
                let kind = self.kind_for(&id, fb);
                let key = id.clone();
                let model = v.clone();
                self.edit(cx, move |f, _| {
                    if let Some(n) = f.nodes.get_mut(&key) {
                        if fb {
                            n.fallback.model = model;
                            n.fallback.effort.clear();
                        } else {
                            n.model = model;
                            n.effort.clear();
                        }
                    }
                });
                if !v.is_empty() && !flow::efforts_of(&self.options, &kind, &v).is_empty() {
                    self.pick_effort(&id, fb, cx);
                }
            }
            Purpose::Effort { id, fb } => self.edit(cx, move |f, _| {
                if let Some(n) = f.nodes.get_mut(&id) {
                    if fb {
                        n.fallback.effort = v;
                    } else {
                        n.effort = v;
                    }
                }
            }),
            Purpose::Gate { id } => self.edit(cx, move |f, _| {
                if let Some(n) = f.nodes.get_mut(&id) {
                    n.gate = v;
                }
            }),
            Purpose::LinkKind { id } => self.choose_target(&id, &v, cx),
            Purpose::LinkTo { from, to } => self.edit(cx, move |f, _| {
                flow::connect(f, &from, &v, &to);
            }),
            Purpose::Target { id, kind } => self.edit(cx, move |f, selected| {
                if let Some(made) = flow::connect(f, &id, &kind, &v) {
                    *selected = Some(made);
                }
            }),
            Purpose::Skills { .. } | Purpose::Tools { .. } => {}
        }
        cx.notify();
    }

    fn kind_for(&self, id: &str, fb: bool) -> String {
        let Some(n) = self.flow.as_ref().and_then(|f| f.nodes.get(id)) else {
            return String::new();
        };
        let who = if fb { &n.fallback.who } else { &n.who };
        flow::kind_of(&self.options, who)
    }

    pub fn pick_who(&mut self, id: &str, fb: bool, cx: &mut Context<Self>) {
        let title = if fb { "Fallback agent" } else { "Agent" };
        let options = flow::who_options(&self.options, fb);
        self.open_picker(
            title.into(),
            Purpose::Who { id: id.into(), fb },
            options,
            None,
            cx,
        );
    }

    pub fn pick_model(&mut self, id: &str, fb: bool, cx: &mut Context<Self>) {
        let kind = self.kind_for(id, fb);
        let mut title = if fb {
            "Fallback model".to_string()
        } else {
            "Model".to_string()
        };
        if !kind.is_empty() {
            title.push_str(&format!(" of {kind}"));
        }
        let options = flow::model_options(&self.options, &kind);
        self.open_picker(
            title,
            Purpose::Model { id: id.into(), fb },
            options,
            None,
            cx,
        );
    }

    pub fn pick_effort(&mut self, id: &str, fb: bool, cx: &mut Context<Self>) {
        let kind = self.kind_for(id, fb);
        let model = self
            .flow
            .as_ref()
            .and_then(|f| f.nodes.get(id))
            .map(|n| {
                if fb {
                    n.fallback.model.clone()
                } else {
                    n.model.clone()
                }
            })
            .unwrap_or_default();
        let options = flow::effort_options(&self.options, &kind, &model);
        if options.len() <= 1 {
            return;
        }
        self.open_picker(
            "Effort".into(),
            Purpose::Effort { id: id.into(), fb },
            options,
            None,
            cx,
        );
    }

    pub fn pick_list(&mut self, id: &str, tools: bool, cx: &mut Context<Self>) {
        let Some(n) = self.flow.as_ref().and_then(|f| f.nodes.get(id)) else {
            return;
        };
        let (title, source, chosen, purpose) = if tools {
            (
                "Tools to allow",
                &self.options.tools,
                n.tools.clone(),
                Purpose::Tools { id: id.into() },
            )
        } else {
            (
                "Skills to attach",
                &self.options.skills,
                n.skills.clone(),
                Purpose::Skills { id: id.into() },
            )
        };
        let options = flow::named_options(source);
        self.open_picker(title.into(), purpose, options, Some(chosen), cx);
    }

    pub fn pick_gate(&mut self, id: &str, cx: &mut Context<Self>) {
        self.open_picker(
            "Gate: who accepts this box".into(),
            Purpose::Gate { id: id.into() },
            flow::gate_options(),
            None,
            cx,
        );
    }

    pub fn connect(&mut self, id: &str, cx: &mut Context<Self>) {
        self.open_picker(
            "Link type".into(),
            Purpose::LinkKind { id: id.into() },
            flow::kind_options(),
            None,
            cx,
        );
    }

    pub fn choose_target(&mut self, id: &str, kind: &str, cx: &mut Context<Self>) {
        let Some(f) = self.flow.as_ref() else {
            return;
        };
        let options = flow::target_options(f, id);
        self.open_picker(
            format!("Connect with {kind} to"),
            Purpose::Target {
                id: id.into(),
                kind: kind.into(),
            },
            options,
            None,
            cx,
        );
    }

    pub fn unlink(&mut self, link: &str, cx: &mut Context<Self>) {
        let link = link.to_string();
        self.edit(cx, move |f, _| flow::unlink(f, &link));
    }

    pub fn set_number(&mut self, id: &str, limit: bool, n: i64, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.edit(cx, move |f, _| {
            if let Some(node) = f.nodes.get_mut(&id) {
                if limit {
                    node.limit = n.clamp(1, 100);
                } else {
                    node.retry = n.clamp(0, 5);
                }
            }
        });
    }

    pub fn edit_prompt(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.run_view.is_some() {
            return;
        }
        let text = self
            .flow
            .as_ref()
            .and_then(|f| f.nodes.get(id))
            .map(|n| n.prompt.clone())
            .unwrap_or_default();
        self.form_error.clear();
        self.modal = Some(Modal::Prompt(id.to_string()));
        self.inputs.prompt.update(cx, |a, cx| a.set_value(text, cx));
        self.focus_next = Some(self.inputs.prompt.focus_handle(cx));
        cx.notify();
    }

    pub fn use_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::Prompt(id)) = self.modal.clone() else {
            return;
        };
        let text = self.inputs.prompt.read(cx).value().trim().to_string();
        self.modal = None;
        self.focus_next = Some(self.canvas_focus.clone());
        self.edit(cx, move |f, _| {
            if let Some(n) = f.nodes.get_mut(&id) {
                n.prompt = text;
            }
        });
        cx.notify();
    }

    pub fn open_new(&mut self, cx: &mut Context<Self>) {
        if !self.busy.is_empty() {
            return;
        }
        for input in [&self.inputs.name, &self.inputs.title, &self.inputs.when] {
            set(input, "", cx);
        }
        self.form_error.clear();
        self.modal = Some(Modal::New);
        self.focus_next = Some(self.inputs.name.focus_handle(cx));
        cx.notify();
    }

    pub fn open_copy(&mut self, cx: &mut Context<Self>) {
        if !self.editable() || !self.busy.is_empty() {
            return;
        }
        self.form_error.clear();
        set(&self.inputs.copy, &format!("{}-copy", self.name), cx);
        self.modal = Some(Modal::Copy);
        self.focus_next = Some(self.inputs.copy.focus_handle(cx));
        cx.notify();
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        if !self.busy.is_empty() || !self.editable() {
            return;
        }
        let Some(flow) = self.flow.as_ref() else {
            return;
        };
        self.busy = "save".into();
        self.error.clear();
        self.note.clear();
        let args = json!({"name": self.name, "workflow": flow::to_value(flow)});
        cx.notify();
        self.job(
            cx,
            move |call| call("put", args),
            |view, out, cx| {
                view.busy.clear();
                match out {
                    Ok(value) => {
                        if let Some(flow) = flow::parse_workflow(&value) {
                            view.saved = flow::to_text(&flow);
                            view.flow = Some(flow);
                        } else if let Some(f) = view.flow.as_ref() {
                            view.saved = flow::to_text(f);
                        }
                        view.can_undo_saved = flow::can_undo(&value);
                        let text = flow::text_of(&value);
                        view.note = if text.is_empty() {
                            "Saved.".into()
                        } else {
                            text
                        };
                        view.shape.clear();
                    }
                    Err(e) => view.error = e,
                }
                view.recheck(cx);
            },
        );
    }

    pub fn open_from_list(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.busy.is_empty() || self.reading {
            return;
        }
        if self.run_view.is_some() {
            self.flow = None;
            self.note.clear();
            self.open_flow(name, false, cx);
            return;
        }
        if name == self.name {
            return;
        }
        if self.dirty() {
            self.error = DIRTY.into();
            cx.notify();
            return;
        }
        self.open_flow(name, false, cx);
    }

    pub fn open_run_row(&mut self, task: i64, cx: &mut Context<Self>) {
        if !self.busy.is_empty() || self.reading {
            return;
        }
        if self.dirty() && self.run_view.is_none() {
            self.error = DIRTY.into();
            cx.notify();
            return;
        }
        self.open_run(task, false, cx);
    }

    pub fn revert(&mut self, cx: &mut Context<Self>) {
        let name = self.name.clone();
        self.open_flow(&name, false, cx);
    }

    pub fn store_act(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if !self.busy.is_empty() || self.reading || self.run_view.is_some() {
            return;
        }
        let restoring = action == "restore";
        let name = if restoring {
            let Some(name) = self.deleted.as_ref().filter(|_| !self.dirty()) else {
                return;
            };
            name.clone()
        } else {
            self.name.clone()
        };
        if name.is_empty() {
            return;
        }
        self.busy = action.into();
        self.error.clear();
        self.note.clear();
        cx.notify();
        let api_action = if restoring { "undo" } else { action };
        let target = name.clone();
        self.job(
            cx,
            move |call| call(api_action, json!({"name": target})),
            move |view, out, cx| {
                view.busy.clear();
                match out {
                    Ok(value) => {
                        view.note = flow::text_of(&value);
                        view.load_names(cx);
                        if action == "delete" {
                            view.deleted = Some(name.clone());
                            view.names.retain(|entry| entry.name != name);
                            view.flow = None;
                            view.name.clear();
                            view.can_undo_saved = false;
                            view.confirm_delete = false;
                            view.reset_canvas();
                            view.problems.clear();
                            if let Some(first) = view.names.first().map(|n| n.name.clone()) {
                                view.open_flow(&first, true, cx);
                            }
                        } else {
                            if restoring {
                                view.deleted = None;
                            }
                            view.open_flow(&name, true, cx);
                        }
                    }
                    Err(e) => view.error = e,
                }
            },
        );
    }

    pub fn shown_runs(&self) -> Vec<&RunInfo> {
        let task = self.run_view.as_ref().map(|r| r.task);
        self.runs
            .iter()
            .filter(|r| self.name.is_empty() || r.workflow == self.name || Some(r.task) == task)
            .collect()
    }

    pub fn key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = e.keystroke.key.as_str();
        let m = &e.keystroke.modifiers;
        if self.picker.is_some() || self.modal.is_some() {
            if k == "escape" {
                self.close_picker(cx);
                self.close_modal(cx);
                cx.stop_propagation();
            }
            return;
        }
        if m.platform {
            match k {
                "z" if m.shift => self.step_back(true, cx),
                "z" => self.step_back(false, cx),
                "s" => self.save(cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        let step = if m.shift { BIG_STEP } else { STEP };
        match k {
            "left" => self.move_selected(-step, 0.0, cx),
            "right" => self.move_selected(step, 0.0, cx),
            "up" => self.move_selected(0.0, -step, cx),
            "down" => self.move_selected(0.0, step, cx),
            "=" | "+" => self.zoom(ZOOM_IN),
            "-" => self.zoom(ZOOM_OUT),
            "f" => self.fit(),
            "delete" | "backspace" => self.remove_box(cx),
            "escape" => {
                self.box_drag = None;
                self.selected = None;
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub fn canvas_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.canvas_focus);
        if let Some(cam) = self.cam {
            self.drag = Some(Drag {
                at: e.position,
                cam,
                moved: false,
            });
        }
        cx.stop_propagation();
    }

    pub fn canvas_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.box_drag.is_some() {
            self.move_box_drag(e, cx);
            return;
        }
        let Some(d) = self.drag.as_mut() else {
            return;
        };
        if !e.dragging() {
            self.drag = None;
            return;
        }
        let dx = f32::from(e.position.x - d.at.x);
        let dy = f32::from(e.position.y - d.at.y);
        if !d.moved && dx.hypot(dy) < DRAG_MIN {
            return;
        }
        d.moved = true;
        self.moved = true;
        self.cam = Some(Cam {
            z: d.cam.z,
            x: d.cam.x + dx,
            y: d.cam.y + dy,
        });
        cx.notify();
    }

    pub fn canvas_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.box_drag.is_some() {
            self.finish_box_drag(event.position, cx);
            return;
        }
        if let Some(d) = self.drag.take()
            && !d.moved
        {
            self.selected = None;
            cx.notify();
        }
    }

    pub fn wheel(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(cam) = self.cam else {
            return;
        };
        if self.picker.is_some() || self.modal.is_some() {
            return;
        }
        let d = e.delta.pixel_delta(px(16.0));
        let (dx, dy) = (f32::from(d.x), f32::from(d.y));
        self.moved = true;
        if e.modifiers.control || e.modifiers.platform {
            let (ox, oy, _, _) = self.area.get();
            let lx = f32::from(e.position.x) - ox;
            let ly = f32::from(e.position.y) - oy;
            self.cam = Some(geo::zoom_at(cam, (dy * WHEEL_ZOOM).exp(), lx, ly));
        } else {
            self.cam = Some(Cam {
                z: cam.z,
                x: cam.x + dx,
                y: cam.y + dy,
            });
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub fn sync_size(&mut self) {
        let (_, _, w, h) = self.area.get();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let size = Some((w, h));
        if self.size != size {
            self.size = size;
            if self.flow.is_some() && (!self.fitted || !self.moved) {
                self.fit();
                self.fitted = true;
            }
        }
        if self.flow.is_some() && !self.fitted {
            self.fit();
            self.fitted = true;
        }
    }

    pub fn take_focus(&mut self, window: &mut Window) {
        if let Some(handle) = self.focus_next.take() {
            window.focus(&handle);
        }
    }
}
