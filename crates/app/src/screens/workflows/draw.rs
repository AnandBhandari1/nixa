use super::flow::{self, BOX_W, HEAD, ROW};
use super::geo::{self, Curve, ZOOM_IN, ZOOM_OUT};
use super::view::{Modal, WorkflowsView, value};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, MouseButton, PathBuilder, SharedString,
    Stateful, Window, canvas, div, point, px,
};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::popover::tooltip;
use nixa_ui::{Theme, theme};

pub const SIDE_W: f32 = 270.0;
pub const PICKER_W: f32 = 440.0;
pub const MODAL_W: f32 = 520.0;

type Act = Box<dyn Fn(&mut WorkflowsView, &mut Window, &mut Context<WorkflowsView>)>;

pub fn kind_color(t: &Theme, kind: &str) -> Hsla {
    match kind {
        "pass" => t.green,
        "fail" => t.red,
        "review" => t.magenta,
        "revise" => t.yellow,
        _ => t.muted,
    }
}

fn sel(el: Stateful<Div>, id: &str) -> Stateful<Div> {
    let name = id.to_string();
    el.debug_selector(move || name)
}

fn sid(id: &str) -> SharedString {
    SharedString::from(id.to_string())
}

fn stop_down<E: InteractiveElement>(el: E) -> E {
    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn sep(t: &Theme) -> Div {
    div().w(px(1.0)).h(px(18.0)).bg(t.border).flex_none()
}

impl WorkflowsView {
    fn btn(
        &self,
        t: &Theme,
        (id, label, tip): (&str, impl Into<SharedString>, &str),
        color: Option<Hsla>,
        off: bool,
        cx: &mut Context<Self>,
        act: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let el = button(t, sid(id), label, ButtonKind::Secondary)
            .h(px(24.0))
            .px(px(8.0))
            .text_size(px(11.5))
            .tooltip(tooltip(tip.to_string()));
        let el = match color {
            Some(c) if !off => el.text_color(c),
            _ => el,
        };
        let el = sel(disabled(el, off), id);
        if off {
            return el;
        }
        let act: Act = Box::new(act);
        stop_down(el)
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| act(view, window, cx)))
    }

    fn toolbar(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let has = self.flow.is_some();
        let editable = self.editable();
        let dirty = self.dirty();
        let picked = self
            .selected
            .as_ref()
            .and_then(|s| self.flow.as_ref()?.nodes.get(s));
        let start_picked = picked.is_some_and(|n| n.kind == "start");
        let save_label = if self.busy == "save" {
            "Saving…"
        } else if dirty {
            "Save"
        } else {
            "Saved"
        };
        let title = if self.name.is_empty() {
            "No workflow open".to_string()
        } else {
            self.name.clone()
        };
        div()
            .w_full()
            .flex_none()
            .px(px(12.0))
            .py(px(8.0))
            .gap(px(6.0))
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(13.0))
                    .child(title),
            )
            .child(self.btn(
                t,
                (
                    "wf-details",
                    "Details",
                    "Edit the workflow title, when to use it and maximum steps",
                ),
                None,
                !editable,
                cx,
                |v, _, cx| v.workflow_details(cx),
            ))
            .child(sep(t))
            .child(self.btn(
                t,
                ("wf-add", "Add box", "Add a box below the selected one"),
                None,
                !editable,
                cx,
                |v, _, cx| v.add_box(cx),
            ))
            .when(picked.is_some() && editable, |el| {
                el.child(self.btn(
                    t,
                    (
                        "wf-box-rename",
                        "Rename box",
                        "Edit the title of the selected box",
                    ),
                    None,
                    false,
                    cx,
                    |v, _, cx| v.box_details(cx),
                ))
            })
            .when_some(
                picked.filter(|n| editable && n.kind == "stage"),
                |el, node| {
                    el.child(self.btn(
                        t,
                        (
                            "wf-box-toggle",
                            if node.off {
                                "Enable box"
                            } else {
                                "Disable box"
                            },
                            "Enabled boxes run when the workflow reaches them",
                        ),
                        None,
                        false,
                        cx,
                        |v, _, cx| v.toggle_box(cx),
                    ))
                },
            )
            .child(self.btn(
                t,
                (
                    "wf-remove",
                    "Remove box",
                    "Remove the selected box and its links",
                ),
                Some(t.red),
                !editable || self.selected.is_none() || start_picked,
                cx,
                |v, _, cx| v.remove_box(cx),
            ))
            .child(self.btn(
                t,
                ("wf-undo", "Undo", "Undo the last edit (⌘Z)"),
                None,
                self.undo.is_empty() || !editable,
                cx,
                |v, _, cx| v.step_back(false, cx),
            ))
            .child(self.btn(
                t,
                ("wf-redo", "Redo", "Redo (⇧⌘Z)"),
                None,
                self.redo.is_empty() || !editable,
                cx,
                |v, _, cx| v.step_back(true, cx),
            ))
            .child(sep(t))
            .child(self.btn(
                t,
                (
                    "wf-auto",
                    "Auto layout",
                    "Place the boxes again by themselves",
                ),
                None,
                !editable,
                cx,
                |v, _, cx| v.auto_layout(cx),
            ))
            .child(self.btn(
                t,
                ("wf-fit", "Fit", "Show the whole workflow (F)"),
                None,
                !has,
                cx,
                |v, _, cx| {
                    v.fit();
                    cx.notify();
                },
            ))
            .child(self.btn(
                t,
                ("wf-zin", "+", "Zoom in (+)"),
                None,
                !has,
                cx,
                |v, _, cx| {
                    v.zoom(ZOOM_IN);
                    cx.notify();
                },
            ))
            .child(self.btn(
                t,
                ("wf-zout", "−", "Zoom out (-)"),
                None,
                !has,
                cx,
                |v, _, cx| {
                    v.zoom(ZOOM_OUT);
                    cx.notify();
                },
            ))
            .child(div().flex_1())
            .child(self.btn(
                t,
                (
                    "wf-saved-undo",
                    "Put back saved copy",
                    "Put back the copy that was saved before the last save",
                ),
                None,
                !self.can_undo_saved || !editable || !self.busy.is_empty(),
                cx,
                |v, _, cx| v.store_act("undo", cx),
            ))
            .child(self.btn(
                t,
                ("wf-revert", "Revert", "Throw away unsaved edits"),
                None,
                !dirty || !editable,
                cx,
                |v, _, cx| v.revert(cx),
            ))
            .child(self.btn(
                t,
                ("wf-save", save_label, "Check and save this workflow (⌘S)"),
                Some(t.green),
                !editable || !dirty,
                cx,
                |v, _, cx| v.save(cx),
            ))
    }

    fn sidebar(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let q = value(&self.inputs.search, cx);
        let list = flow::filter_names(&self.names, &q);
        let has = self.flow.is_some();
        let in_run = self.run_view.is_some();
        let mut items = div()
            .id("wf-side-list")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.0));
        for n in &list {
            let name = n.name.clone();
            let id = format!("wf-item-{}", n.name);
            let picked = n.name == self.name;
            let hover = t.hover;
            let label = if n.broken.is_some() {
                format!("{} (broken)", n.name)
            } else {
                n.name.clone()
            };
            let row = div()
                .id(sid(&id))
                .w_full()
                .flex_none()
                .px(px(8.0))
                .py(px(6.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .when(picked, |el| el.bg(t.selected))
                .hover(move |s| s.bg(hover))
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(label),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(10.5))
                        .text_color(t.dim)
                        .child(n.title.clone()),
                )
                .on_click(
                    cx.listener(move |view, _: &ClickEvent, _, cx| view.open_from_list(&name, cx)),
                );
            items = items.child(sel(row, &id));
        }
        if list.is_empty() {
            let words = if self.names.is_empty() {
                "No workflows yet."
            } else {
                "Nothing matches."
            };
            items = items.child(div().p(px(8.0)).text_color(t.dim).child(words));
        }
        let delete = if self.confirm_delete {
            self.btn(
                t,
                (
                    "wf-del-yes",
                    "Yes, delete",
                    "Delete the open workflow. Restore deleted puts it back.",
                ),
                Some(t.red),
                !self.busy.is_empty(),
                cx,
                |v, _, cx| v.store_act("delete", cx),
            )
        } else {
            self.btn(
                t,
                ("wf-del", "Delete", "Delete the open workflow"),
                Some(t.red),
                !has || in_run || self.reading || !self.busy.is_empty(),
                cx,
                |v, _, cx| {
                    v.confirm_delete = true;
                    cx.notify();
                },
            )
        };
        div()
            .w(px(SIDE_W))
            .flex_none()
            .h_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(10.0))
            .border_r_1()
            .border_color(t.border)
            .bg(t.chrome)
            .child(self.inputs.search.clone())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(6.0))
                    .child(self.btn(
                        t,
                        ("wf-new", "New", "Make a new workflow"),
                        Some(t.green),
                        false,
                        cx,
                        |v, _, cx| v.open_new(cx),
                    ))
                    .child(self.btn(
                        t,
                        ("wf-copy", "Copy", "Copy the open workflow under a new name"),
                        None,
                        !has || in_run,
                        cx,
                        |v, _, cx| v.open_copy(cx),
                    ))
                    .child(delete)
                    .when(self.confirm_delete, |el| {
                        el.child(self.btn(
                            t,
                            ("wf-del-no", "Keep", "Keep the open workflow"),
                            None,
                            !self.busy.is_empty(),
                            cx,
                            |v, _, cx| {
                                v.confirm_delete = false;
                                cx.notify();
                            },
                        ))
                    })
                    .when_some(self.deleted.as_deref(), |el, name| {
                        el.child(self.btn(
                            t,
                            (
                                "wf-restore-deleted",
                                format!("Restore {name}"),
                                "Put back the deleted workflow",
                            ),
                            None,
                            in_run || self.reading || self.dirty() || !self.busy.is_empty(),
                            cx,
                            |v, _, cx| v.store_act("restore", cx),
                        ))
                    }),
            )
            .child(items)
            .child(self.runs_panel(t, cx))
    }

    fn runs_panel(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let shown = self.shown_runs();
        let rv = self.run_view.as_ref();
        let state = rv.map(|r| r.run.state.clone()).unwrap_or_default();
        let title = if self.name.is_empty() {
            "Runs".to_string()
        } else {
            format!("Runs of {}", self.name)
        };
        let mut list = div()
            .id("wf-runs-list")
            .max_h(px(150.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.0));
        for r in &shown {
            let task = r.task;
            let id = format!("wf-run-{task}");
            let hover = t.hover;
            let row = div()
                .id(sid(&id))
                .w_full()
                .flex_none()
                .px(px(8.0))
                .py(px(5.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .when(rv.is_some_and(|v| v.task == task), |el| el.bg(t.selected))
                .hover(move |s| s.bg(hover))
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.0))
                        .child(format!("#{} · {}", r.task, r.stage)),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(10.5))
                        .text_color(t.dim)
                        .child(format!("{} · {}", r.space, r.state)),
                )
                .on_click(
                    cx.listener(move |view, _: &ClickEvent, _, cx| view.open_run_row(task, cx)),
                );
            list = list.child(sel(row, &id));
        }
        if shown.is_empty() {
            list = list.child(
                div()
                    .px(px(8.0))
                    .text_size(px(11.0))
                    .text_color(t.dim)
                    .child("No active run. Start a run from a task."),
            );
        }
        let mut panel = div()
            .id("wf-runs")
            .w_full()
            .flex_none()
            .max_h(px(260.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .border_t_1()
            .border_color(t.border)
            .pt(px(8.0))
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.muted)
                    .child(title),
            )
            .child(list);
        if let Some(rv) = rv {
            let mut row = div().flex().flex_row().flex_wrap().gap(px(6.0));
            if let Some(text) = flow::waiting_text(&rv.run) {
                panel = panel.child(sel(
                    div()
                        .id("wf-run-ask")
                        .text_size(px(11.0))
                        .text_color(t.yellow)
                        .child(text),
                    "wf-run-ask",
                ));
                let gate = rv.run.ask.as_ref().is_some_and(|a| a.gate);
                if gate {
                    for (word, label, tip) in flow::GATE_ANSWERS {
                        let id = format!("wf-run-answer-{word}");
                        row = row.child(self.btn(
                            t,
                            (&id, *label, tip),
                            Some(kind_color(t, word)),
                            false,
                            cx,
                            move |v, _, cx| v.run_act("answer", word, cx),
                        ));
                    }
                } else {
                    row = row.child(self.btn(
                        t,
                        ("wf-run-retry", "Retry", "Run this box again"),
                        Some(t.green),
                        false,
                        cx,
                        |v, _, cx| v.run_act("retry", "", cx),
                    ));
                    row = row.child(self.btn(
                        t,
                        ("wf-run-next", "Next box", "Go on to the next box"),
                        None,
                        false,
                        cx,
                        |v, _, cx| v.run_act("answer", flow::NEXT_BOX, cx),
                    ));
                }
            }
            if state == "running" {
                row = row.child(self.btn(
                    t,
                    (
                        "wf-run-pause",
                        "Pause",
                        "Finish the current stage, then wait",
                    ),
                    Some(t.blue),
                    false,
                    cx,
                    |v, _, cx| v.run_act("pause", "", cx),
                ));
            }
            if state == "paused" {
                row = row.child(self.btn(
                    t,
                    ("wf-run-resume", "Resume", "Start the next stage"),
                    Some(t.green),
                    false,
                    cx,
                    |v, _, cx| v.run_act("resume", "", cx),
                ));
            }
            if flow::active(&state) {
                row = row.child(if self.confirm_cancel {
                    self.btn(
                        t,
                        (
                            "wf-run-cancel-yes",
                            "Yes, cancel run",
                            "Stop the agent and cancel the task. Changed files stay.",
                        ),
                        Some(t.red),
                        false,
                        cx,
                        |v, _, cx| v.run_act("cancel", "", cx),
                    )
                } else {
                    self.btn(
                        t,
                        (
                            "wf-run-cancel",
                            "Cancel run",
                            "Stop the run now. Asks once.",
                        ),
                        Some(t.red),
                        false,
                        cx,
                        |v, _, cx| {
                            v.confirm_cancel = true;
                            cx.notify();
                        },
                    )
                });
            }
            row = row.child(self.btn(
                t,
                (
                    "wf-run-reports",
                    format!("Box reports ({})", rv.reports.len()),
                    "Read the reports from this run",
                ),
                None,
                rv.reports.is_empty(),
                cx,
                |v, _, cx| {
                    v.modal = Some(Modal::Reports);
                    cx.notify();
                },
            ));
            row = row.child(self.btn(
                t,
                ("wf-run-close", "Back to editing", "Leave the run view"),
                None,
                false,
                cx,
                |v, _, cx| v.close_run(cx),
            ));
            panel = panel.child(row);
        }
        div().w_full().flex_none().child(panel)
    }

    fn field(
        &self,
        t: &Theme,
        z: f32,
        (id, label, shown, tip): (String, &str, String, &str),
        color: Option<Hsla>,
        cx: &mut Context<Self>,
        act: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let hover = t.hover;
        let row = div()
            .id(sid(&id))
            .h(px(ROW * z))
            .w_full()
            .px(px(6.0 * z))
            .gap(px(6.0 * z))
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .rounded(px(4.0 * z))
            .cursor_pointer()
            .tooltip(tooltip(tip.to_string()))
            .hover(move |s| s.bg(hover))
            .child(
                div()
                    .w(px(52.0 * z))
                    .flex_none()
                    .text_size(px(10.0 * z))
                    .text_color(t.dim)
                    .child(label.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(11.0 * z))
                    .text_color(color.unwrap_or(t.foreground))
                    .child(shown),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.0 * z))
                    .text_color(t.dim)
                    .child("▾"),
            );
        let row = sel(row, &id);
        if self.run_view.is_some() {
            return row;
        }
        stop_down(row).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            act(view, cx)
        }))
    }

    fn stepper(
        &self,
        t: &Theme,
        z: f32,
        node: &str,
        limit: bool,
        value: i64,
        cx: &mut Context<Self>,
    ) -> Div {
        let (label, lo, hi) = if limit {
            ("Limit", 1, 100)
        } else {
            ("Retry", 0, 5)
        };
        let key = if limit { "limit" } else { "retry" };
        let run = self.run_view.is_some();
        let step = |sign: &'static str, delta: i64, cx: &mut Context<Self>| {
            let id = format!("wf-{key}-{node}{sign}");
            let hover = t.hover;
            let node = node.to_string();
            let el = div()
                .id(sid(&id))
                .w(px(18.0 * z))
                .h(px(16.0 * z))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0 * z))
                .border_1()
                .border_color(t.border)
                .cursor_pointer()
                .text_size(px(11.0 * z))
                .hover(move |s| s.bg(hover))
                .child(if delta < 0 { "−" } else { "+" });
            let el = sel(el, &id);
            if run {
                return el;
            }
            stop_down(el).on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                view.set_number(&node, limit, (value + delta).clamp(lo, hi), cx);
            }))
        };
        div()
            .h(px(ROW * z))
            .flex_1()
            .px(px(6.0 * z))
            .gap(px(5.0 * z))
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .w(px(34.0 * z))
                    .flex_none()
                    .text_size(px(10.0 * z))
                    .text_color(t.dim)
                    .child(label),
            )
            .child(step("-", -1, cx))
            .child(
                div()
                    .w(px(22.0 * z))
                    .text_size(px(11.0 * z))
                    .flex()
                    .justify_center()
                    .child(value.to_string()),
            )
            .child(step("+", 1, cx))
    }

    fn node_el(&self, t: &Theme, id: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let f = self.flow.as_ref()?;
        let n = f.nodes.get(id)?;
        let cam = self.cam?;
        let p = self.spots.get(id).copied().unwrap_or_default();
        let z = cam.z;
        let in_run = self.run_view.is_some();
        let h = flow::rows(f, id, in_run, &self.options);
        let picked = self.selected.as_deref() == Some(id);
        let color = match n.kind.as_str() {
            "start" => t.cyan,
            "end" => t.magenta,
            _ => t.accent,
        };
        let run = self.run_view.as_ref().map(|r| &r.run);
        let here = run.is_some_and(|r| r.node == id);
        let border = if here {
            t.yellow
        } else if picked {
            t.foreground
        } else {
            color
        };
        let width = if here {
            3.0
        } else if picked {
            2.0
        } else {
            1.0
        };
        let box_id = format!("wf-node-{id}");
        let mut el = div()
            .id(sid(&box_id))
            .absolute()
            .left(px(cam.sx(p.x) - BOX_W / 2.0 * z))
            .top(px(cam.sy(p.y) - h / 2.0 * z))
            .w(px(BOX_W * z))
            .h(px(h * z))
            .py(px(4.0 * z))
            .flex()
            .flex_col()
            .rounded(px(8.0 * z))
            .overflow_hidden()
            .border(px(width))
            .border_color(border)
            .bg(t.background)
            .when(n.off, |el| el.opacity(0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    window.focus(&view.canvas_focus);
                    cx.stop_propagation();
                }),
            );
        let head_id = format!("wf-head-{id}");
        let key = id.to_string();
        let link_key = key.clone();
        let head = div()
            .id(sid(&head_id))
            .h(px(HEAD * z))
            .w_full()
            .px(px(8.0 * z))
            .gap(px(6.0 * z))
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .when(!in_run, |el| el.cursor_grab())
            .tooltip(tooltip(if in_run {
                "Click to select"
            } else {
                "Drag or use the arrow keys to move the box"
            }))
            .child(
                div()
                    .text_size(px(9.5 * z))
                    .text_color(t.dim)
                    .child(format!(
                        "{id} · {}{}",
                        n.kind,
                        if n.off { " · off" } else { "" }
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(12.0 * z))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(n.title.clone()),
            )
            .when(!in_run && n.kind != "end", |el| {
                el.child(
                    div()
                        .id(sid(&format!("wf-link-drag-{id}")))
                        .debug_selector(move || format!("wf-link-drag-{link_key}"))
                        .cursor_crosshair()
                        .px(px(3.0 * z))
                        .text_color(t.accent)
                        .child("↗")
                        .tooltip(tooltip("Drag to a box to connect it"))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener({
                                let id = key.clone();
                                move |v, event, window, cx| {
                                    v.begin_box_drag(&id, true, event, window, cx)
                                }
                            }),
                        ),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event, window, cx| {
                    view.begin_box_drag(&key, false, event, window, cx);
                }),
            );
        el = el.child(sel(head, &head_id));
        if let Some(run) = run
            && n.kind == "stage"
        {
            el = el.child(
                div()
                    .h(px(ROW * z))
                    .w_full()
                    .px(px(8.0 * z))
                    .flex()
                    .items_center()
                    .flex_none()
                    .text_size(px(10.5 * z))
                    .text_color(if here { t.yellow } else { t.dim })
                    .child(flow::run_line(n, run, id)),
            );
        }
        if n.kind == "stage" {
            let o = &self.options;
            let k = id.to_string();
            let none = |set: bool| if set { None } else { Some(t.dim) };
            el = el.child(self.field(t, z, (format!("wf-who-{id}"), "Agent", flow::who_label(o, &n.who), "Pick the agent for this box: claude, codex and the others, a custom agent, the deep brain, Nixa or you"), None, cx, { let k = k.clone(); move |v, cx| v.pick_who(&k, false, cx) }));
            let model = if !n.model.is_empty() {
                n.model.clone()
            } else if flow::no_list(o, &n.who) {
                "no model list for this agent; it uses its default".into()
            } else {
                "(agent's own)".into()
            };
            el = el.child(self.field(
                t,
                z,
                (
                    format!("wf-model-{id}"),
                    "Model",
                    model,
                    "Pick a model of the chosen agent",
                ),
                none(!n.model.is_empty()),
                cx,
                {
                    let k = k.clone();
                    move |v, cx| v.pick_model(&k, false, cx)
                },
            ));
            if flow::has_efforts(o, &n.who, &n.model) {
                let effort = if n.effort.is_empty() {
                    "(none)".into()
                } else {
                    n.effort.clone()
                };
                el = el.child(self.field(
                    t,
                    z,
                    (
                        format!("wf-effort-{id}"),
                        "Effort",
                        effort,
                        "Pick the effort for this model",
                    ),
                    none(!n.effort.is_empty()),
                    cx,
                    {
                        let k = k.clone();
                        move |v, cx| v.pick_effort(&k, false, cx)
                    },
                ));
            }
            let fb = &n.fallback;
            let fb_on = !fb.who.kind.is_empty();
            let fb_label = if fb_on {
                flow::who_label(o, &fb.who)
            } else {
                "(none)".into()
            };
            el = el.child(self.field(
                t,
                z,
                (
                    format!("wf-fb-{id}"),
                    "Fallback",
                    fb_label,
                    "Agent used when the first one fails or cannot start",
                ),
                none(fb_on),
                cx,
                {
                    let k = k.clone();
                    move |v, cx| v.pick_who(&k, true, cx)
                },
            ));
            if fb_on {
                let fm = if !fb.model.is_empty() {
                    fb.model.clone()
                } else if flow::no_list(o, &fb.who) {
                    "no model list for this agent; it uses its default".into()
                } else {
                    "(agent's own)".into()
                };
                el = el.child(self.field(
                    t,
                    z,
                    (
                        format!("wf-fbmodel-{id}"),
                        "Fb model",
                        fm,
                        "Model of the fallback agent",
                    ),
                    none(!fb.model.is_empty()),
                    cx,
                    {
                        let k = k.clone();
                        move |v, cx| v.pick_model(&k, true, cx)
                    },
                ));
                if flow::has_efforts(o, &fb.who, &fb.model) {
                    let fe = if fb.effort.is_empty() {
                        "(none)".into()
                    } else {
                        fb.effort.clone()
                    };
                    el = el.child(self.field(
                        t,
                        z,
                        (
                            format!("wf-fbeffort-{id}"),
                            "Fb effort",
                            fe,
                            "Effort of the fallback model",
                        ),
                        none(!fb.effort.is_empty()),
                        cx,
                        {
                            let k = k.clone();
                            move |v, cx| v.pick_effort(&k, true, cx)
                        },
                    ));
                }
            }
            let list = |items: &[String]| {
                if items.is_empty() {
                    "none".to_string()
                } else {
                    format!("{}: {}", items.len(), items.join(", "))
                }
            };
            el = el.child(self.field(
                t,
                z,
                (
                    format!("wf-skills-{id}"),
                    "Skills",
                    list(&n.skills),
                    "Pick installed skills to attach",
                ),
                none(!n.skills.is_empty()),
                cx,
                {
                    let k = k.clone();
                    move |v, cx| v.pick_list(&k, false, cx)
                },
            ));
            el = el.child(self.field(
                t,
                z,
                (
                    format!("wf-tools-{id}"),
                    "Tools",
                    list(&n.tools),
                    "Pick the Nixa or MCP tools this box may use",
                ),
                none(!n.tools.is_empty()),
                cx,
                {
                    let k = k.clone();
                    move |v, cx| v.pick_list(&k, true, cx)
                },
            ));
            let job = flow::clip(
                if n.prompt.is_empty() {
                    "(write the job)"
                } else {
                    &n.prompt
                },
                60,
            );
            el = el.child(self.field(
                t,
                z,
                (
                    format!("wf-job-{id}"),
                    "Job",
                    job,
                    "Write the job text of this box",
                ),
                none(!n.prompt.is_empty()),
                cx,
                {
                    let k = k.clone();
                    move |v, cx| v.edit_prompt(&k, cx)
                },
            ));
            el = el.child(self.field(
                t,
                z,
                (
                    format!("wf-gate-{id}"),
                    "Gate",
                    flow::gate_label(&n.gate),
                    "Who accepts the work of this box",
                ),
                None,
                cx,
                {
                    let k = k.clone();
                    move |v, cx| v.pick_gate(&k, cx)
                },
            ));
            el = el.child(
                div()
                    .h(px(ROW * z))
                    .w_full()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .child(self.stepper(t, z, id, false, n.retry, cx))
                    .child(self.stepper(t, z, id, true, n.limit, cx)),
            );
        }
        for l in flow::out_links(f, id) {
            let to = f
                .nodes
                .get(&l.to)
                .map(|n| n.title.clone())
                .unwrap_or_else(|| l.to.clone());
            let mut row = div()
                .h(px(ROW * z))
                .w_full()
                .px(px(6.0 * z))
                .gap(px(6.0 * z))
                .flex()
                .flex_row()
                .items_center()
                .flex_none()
                .child(
                    div()
                        .w(px(52.0 * z))
                        .flex_none()
                        .text_size(px(10.0 * z))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(kind_color(t, &l.kind))
                        .child(l.kind.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(11.0 * z))
                        .child(format!("→ {to}")),
                );
            if !in_run {
                let unlink_id = format!("wf-unlink-{}", l.id);
                let link = l.id.clone();
                let hover = t.hover;
                let x = div()
                    .id(sid(&unlink_id))
                    .flex_none()
                    .size(px(16.0 * z))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0 * z))
                    .cursor_pointer()
                    .tooltip(tooltip("Remove this link"))
                    .text_size(px(11.0 * z))
                    .text_color(t.dim)
                    .hover(move |s| s.bg(hover))
                    .child("✕");
                row = row.child(stop_down(sel(x, &unlink_id)).on_click(cx.listener(
                    move |view, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        view.unlink(&link, cx);
                    },
                )));
            }
            el = el.child(row);
        }
        if n.kind != "end" && !in_run {
            let k = id.to_string();
            el = el.child(self.field(
                t,
                z,
                (
                    format!("wf-connect-{id}"),
                    "Next",
                    "connect to next box…".into(),
                    "Pick the link type, then the next box, or make a new box",
                ),
                Some(t.accent),
                cx,
                move |v, cx| v.connect(&k, cx),
            ));
        }
        Some(sel(el, &box_id).into_any_element())
    }

    fn links_layer(&self, t: &Theme, curves: Vec<Curve>) -> AnyElement {
        let colors: Vec<Hsla> = curves.iter().map(|c| kind_color(t, &c.kind)).collect();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                let at = |p: (f32, f32)| point(o.x + px(p.0), o.y + px(p.1));
                for (c, color) in curves.iter().zip(colors.iter()) {
                    let mut line = PathBuilder::stroke(px(1.6));
                    line.move_to(at(c.start));
                    line.cubic_bezier_to(at(c.end), at(c.c1), at(c.c2));
                    if let Ok(path) = line.build() {
                        window.paint_path(path, *color);
                    }
                    let mut head = PathBuilder::fill();
                    let (x, y) = c.tip;
                    head.add_polygon(
                        &[
                            at((x, y)),
                            at((x - c.head, y - c.head * 0.5)),
                            at((x - c.head, y + c.head * 0.5)),
                        ],
                        true,
                    );
                    if let Ok(path) = head.build() {
                        window.paint_path(path, *color);
                    }
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }

    fn canvas_el(&mut self, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let area = self.area.clone();
        let probe = canvas(
            move |bounds, window, _| {
                let now = (
                    f32::from(bounds.origin.x),
                    f32::from(bounds.origin.y),
                    f32::from(bounds.size.width),
                    f32::from(bounds.size.height),
                );
                if area.get() != now {
                    area.set(now);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let mut el = div()
            .id("wf-canvas")
            .flex_1()
            .min_h(px(0.0))
            .relative()
            .overflow_hidden()
            .bg(t.canvas)
            .track_focus(&self.canvas_focus)
            .on_key_down(cx.listener(Self::key))
            .on_scroll_wheel(cx.listener(Self::wheel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::canvas_down))
            .on_mouse_move(cx.listener(Self::canvas_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::canvas_up))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    if view.box_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(probe);
        self.sync_size();
        if self.flow.is_some() {
            self.place();
        }
        let (Some(cam), Some(f)) = (self.cam, self.flow.as_ref()) else {
            let words = if !self.booted {
                if self.link_error.is_empty() {
                    "Connecting to Nixa…".to_string()
                } else {
                    format!("Connecting to Nixa… {}", self.link_error)
                }
            } else if self.names.is_empty() {
                "No workflows yet. Press New.".to_string()
            } else {
                "Loading…".to_string()
            };
            return el.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(t.dim)
                    .child(words),
            );
        };
        let in_run = self.run_view.is_some();
        let curves = geo::curves(f, &self.spots, cam, in_run, &self.options);
        let z = cam.z;
        let labels: Vec<AnyElement> = curves
            .iter()
            .map(|c| {
                let w = (c.kind.len() as f32 * 6.0 + 10.0) * z;
                let h = 14.0 * z;
                div()
                    .absolute()
                    .left(px(c.mid.0 - w / 2.0))
                    .top(px(c.mid.1 - h / 2.0))
                    .w(px(w))
                    .h(px(h))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0 * z))
                    .bg(t.canvas)
                    .text_size(px(9.5 * z))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(kind_color(t, &c.kind))
                    .child(c.kind.clone())
                    .into_any_element()
            })
            .collect();
        let ids = flow::ids(f);
        el = el.child(self.links_layer(t, curves)).children(labels);
        if let Some((start, end)) = self.box_drag.as_ref().and_then(|drag| drag.link_line()) {
            let (ox, oy, _, _) = self.area.get();
            let origin = point(px(ox), px(oy));
            let color = t.accent;
            el = el.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let mut line = PathBuilder::stroke(px(2.0));
                        line.move_to(bounds.origin + (start - origin));
                        line.line_to(bounds.origin + (end - origin));
                        if let Ok(path) = line.build() {
                            window.paint_path(path, color);
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        }
        for id in ids {
            if let Some(node) = self.node_el(t, &id, cx) {
                el = el.child(node);
            }
        }
        el
    }

    fn problems_el(&self, t: &Theme) -> Stateful<Div> {
        let mut el = div()
            .id("wf-problems")
            .w_full()
            .flex_none()
            .max_h(px(96.0))
            .overflow_y_scroll()
            .px(px(12.0))
            .py(px(5.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .border_t_1()
            .border_color(t.border);
        if self.problems.is_empty() {
            let words = if self.flow.is_some() {
                "The checker finds no problems."
            } else {
                "No workflow open."
            };
            el = el.child(div().text_size(px(11.0)).text_color(t.green).child(words));
        }
        for (i, p) in self.problems.iter().enumerate() {
            let id = format!("wf-problem-{i}");
            el = el.child(sel(
                div()
                    .id(sid(&id))
                    .text_size(px(11.0))
                    .text_color(t.red)
                    .child(format!("• {p}")),
                &id,
            ));
        }
        el
    }

    fn picker_el(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let p = self.picker.as_ref()?;
        let all = self.shown_options(cx);
        let shown: Vec<&(usize, flow::Opt)> = all.iter().take(flow::MAX_SHOWN).collect();
        let mut rows = div()
            .id("wf-picker-list")
            .flex()
            .flex_col()
            .gap(px(1.0))
            .min_h(px(0.0))
            .max_h(px(340.0))
            .overflow_y_scroll();
        for (i, (_, o)) in shown.iter().enumerate() {
            let id = format!("wf-opt-{i}");
            let on = p.multi && p.chosen.contains(&o.value);
            let hover = t.hover;
            let v = o.value.clone();
            let multi = p.multi;
            let row = div()
                .id(sid(&id))
                .w_full()
                .flex_none()
                .px(px(8.0))
                .py(px(4.0))
                .gap(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .rounded(px(5.0))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .when(multi, |el| {
                    el.child(
                        div()
                            .w(px(16.0))
                            .flex_none()
                            .text_color(t.green)
                            .child(if on { "✓" } else { "" }),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(div().truncate().text_size(px(12.0)).child(o.label.clone()))
                        .when(!o.hint.is_empty(), |el| {
                            el.child(
                                div()
                                    .truncate()
                                    .text_size(px(10.5))
                                    .text_color(t.dim)
                                    .child(o.hint.clone()),
                            )
                        }),
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if multi {
                        view.toggle_chosen(&v, cx);
                    } else {
                        view.pick(v.clone(), cx);
                    }
                }));
            rows = rows.child(sel(row, &id));
        }
        if shown.is_empty() {
            rows = rows.child(div().p(px(8.0)).text_color(t.dim).child("Nothing matches."));
        }
        let mut panel = div()
            .id("wf-picker-panel")
            .w(px(PICKER_W))
            .max_h(px(520.0))
            .p(px(12.0))
            .gap(px(8.0))
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.0))
                            .child(p.title.clone()),
                    )
                    .child(self.btn(
                        t,
                        ("wf-picker-close", "Close", "Close"),
                        None,
                        false,
                        cx,
                        |v, _, cx| v.close_picker(cx),
                    )),
            )
            .child(self.inputs.filter.clone())
            .child(rows);
        if all.len() > shown.len() {
            panel = panel.child(div().text_size(px(10.5)).text_color(t.dim).child(format!(
                "Showing {} of {}. Type more to narrow the list.",
                shown.len(),
                all.len()
            )));
        }
        if p.multi {
            panel = panel.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .child(self.btn(
                        t,
                        (
                            "wf-picker-done",
                            format!("Done ({})", p.chosen.len()),
                            "Use these",
                        ),
                        Some(t.green),
                        false,
                        cx,
                        |v, _, cx| v.pick_many(cx),
                    ))
                    .child(self.btn(
                        t,
                        ("wf-picker-clear", "Clear", "Clear the choice"),
                        None,
                        false,
                        cx,
                        |v, _, cx| v.clear_chosen(cx),
                    )),
            );
        }
        Some(self.scrim(t, "wf-picker", panel, cx, |v, cx| v.close_picker(cx)))
    }

    fn scrim(
        &self,
        t: &Theme,
        id: &str,
        body: impl IntoElement,
        cx: &mut Context<Self>,
        close: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let el = div()
            .id(sid(id))
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| close(view, cx)))
            .child(body);
        sel(el, id)
    }

    fn modal_el(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let m = self.modal.as_ref()?;
        let (title, body): (String, Div) = match m {
            Modal::Prompt(id) => {
                let title = self
                    .flow
                    .as_ref()
                    .and_then(|f| f.nodes.get(id))
                    .map(|n| n.title.clone())
                    .unwrap_or_default();
                (
                    format!("Job text of {title}"),
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .h(px(180.0))
                                .flex()
                                .flex_col()
                                .child(self.inputs.prompt.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(8.0))
                                .child(self.btn(
                                    t,
                                    ("wf-prompt-ok", "Use this text", "Keep the job text (⌘S)"),
                                    Some(t.green),
                                    false,
                                    cx,
                                    |v, _, cx| v.use_prompt(cx),
                                ))
                                .child(self.btn(
                                    t,
                                    ("wf-prompt-cancel", "Cancel", "Close without a change"),
                                    None,
                                    false,
                                    cx,
                                    |v, _, cx| v.close_modal(cx),
                                )),
                        ),
                )
            }
            Modal::Details | Modal::Box(_) => {
                let mut form = div().flex().flex_col().gap(px(6.0));
                let title = if matches!(m, Modal::Details) {
                    form = form
                        .child("Title")
                        .child(self.inputs.title.clone())
                        .child("When to use it")
                        .child(self.inputs.when.clone())
                        .child("Maximum steps")
                        .child(self.inputs.step_limit.clone());
                    "Workflow details"
                } else {
                    form = form.child(self.inputs.box_title.clone());
                    "Rename box"
                };
                (
                    title.into(),
                    form.child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(self.btn(
                                t,
                                ("wf-details-ok", "Apply", "Apply to the workflow draft"),
                                Some(t.green),
                                false,
                                cx,
                                |v, _, cx| v.apply_details(cx),
                            ))
                            .child(self.btn(
                                t,
                                ("wf-details-cancel", "Cancel", "Close without an edit"),
                                None,
                                false,
                                cx,
                                |v, _, cx| v.close_modal(cx),
                            )),
                    ),
                )
            }
            Modal::Reports => {
                let rv = self.run_view.as_ref()?;
                let list = div()
                    .id("wf-report-list")
                    .debug_selector(|| "wf-report-list".into())
                    .max_h(px(360.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .children(rv.reports.iter().map(|report| {
                        let title = self
                            .flow
                            .as_ref()
                            .and_then(|f| f.nodes.get(&report.box_id))
                            .map_or(report.box_id.as_str(), |node| node.title.as_str());
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .flex_none()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{} · {title}", report.number)),
                            )
                            .child(report.text.clone())
                    }));
                (
                    format!("Box reports for task #{}", rv.task),
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(list)
                        .child(self.btn(
                            t,
                            ("wf-reports-close", "Close", "Return to the run"),
                            None,
                            false,
                            cx,
                            |v, _, cx| v.close_modal(cx),
                        )),
                )
            }
            Modal::Copy => (
                format!("Copy {}", self.name),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(self.inputs.copy.clone())
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.0))
                            .child(self.btn(
                                t,
                                (
                                    "wf-copy-ok",
                                    if self.saving_form() {
                                        "Copying…"
                                    } else {
                                        "Copy"
                                    },
                                    "Save the copy",
                                ),
                                Some(t.green),
                                self.saving_form(),
                                cx,
                                |v, _, cx| v.copy_flow(cx),
                            ))
                            .child(self.btn(
                                t,
                                ("wf-copy-cancel", "Cancel", "Close without a copy"),
                                None,
                                self.saving_form(),
                                cx,
                                |v, _, cx| v.close_modal(cx),
                            )),
                    ),
            ),
            Modal::New => (
                "New workflow".to_string(),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(self.inputs.name.clone())
                    .child(self.inputs.title.clone())
                    .child(self.inputs.when.clone())
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.0))
                            .child(self.btn(
                                t,
                                (
                                    "wf-new-ok",
                                    if self.saving_form() {
                                        "Creating…"
                                    } else {
                                        "Create"
                                    },
                                    "Make it with a start, one box and an end",
                                ),
                                Some(t.green),
                                self.saving_form(),
                                cx,
                                |v, _, cx| v.create_flow(cx),
                            ))
                            .child(self.btn(
                                t,
                                ("wf-new-cancel", "Cancel", "Close without a new workflow"),
                                None,
                                self.saving_form(),
                                cx,
                                |v, _, cx| v.close_modal(cx),
                            )),
                    ),
            ),
        };
        let panel = div()
            .id("wf-modal-panel")
            .w(px(MODAL_W))
            .p(px(14.0))
            .gap(px(8.0))
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(13.0))
                    .child(title),
            )
            .when(!self.form_error.is_empty(), |el| {
                el.child(
                    div()
                        .id("wf-form-error")
                        .debug_selector(|| "wf-form-error".into())
                        .text_color(t.red)
                        .child(self.form_error.clone()),
                )
            })
            .child(body);
        Some(self.scrim(t, "wf-modal", panel, cx, |v, cx| v.close_modal(cx)))
    }
}

impl Render for WorkflowsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.take_focus(window);
        let t = theme(cx).clone();
        let mut editor = div()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .relative()
            .child(self.toolbar(&t, cx));
        if !self.error.is_empty() {
            editor = editor.child(
                div()
                    .id("wf-error")
                    .px(px(12.0))
                    .py(px(5.0))
                    .text_color(t.red)
                    .child(self.error.clone()),
            );
        }
        if !self.note.is_empty() {
            editor = editor.child(
                div()
                    .id("wf-note")
                    .px(px(12.0))
                    .py(px(5.0))
                    .text_color(t.green)
                    .child(self.note.clone()),
            );
        }
        editor = editor
            .child(self.canvas_el(&t, cx))
            .child(self.problems_el(&t));
        let picker = self.picker_el(&t, cx);
        let modal = if picker.is_none() {
            self.modal_el(&t, cx)
        } else {
            None
        };
        div()
            .id("wf-window")
            .key_context("NixaWorkflows")
            .track_focus(&self.focus)
            .size_full()
            .min_h(px(0.0))
            .relative()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.0))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_row()
                    .child(self.sidebar(&t, cx))
                    .child(editor),
            )
            .children(picker)
            .children(modal)
    }
}
