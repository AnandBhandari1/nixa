use super::drop::{Drop, dropdown, spec};
use super::model::{self, Comment, Task};
use super::parts::{
    boxed, caption, chip, col, info, line, look, pair, panel_frame, pill, row, tint,
};
use super::top::{status_color, status_pill};
use super::view::{TasksView, area_value};
use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, FontWeight, SharedString, div, px};
use nixa_ui::{Icon, Theme};

const ARCHIVE_TIP: &str = "Hide this task from the list. It stays saved.";
const CANCEL_ARCHIVE_TIP: &str = "Cancel this task and hide it from the list. It stays saved.";

impl TasksView {
    fn summary(&self, t: &Theme, task: &Task) -> Div {
        let assigned = model::assigned_name(&self.options, task);
        let space = model::space_name(&self.options, task);
        col()
            .w_full()
            .gap(px(6.0))
            .when(task.needs_user, |el| {
                el.child(
                    row()
                        .id("tasks-d-waiting")
                        .debug_selector(|| "tasks-d-waiting".into())
                        .w_full()
                        .px(px(10.0))
                        .py(px(6.0))
                        .gap(px(8.0))
                        .rounded(px(6.0))
                        .bg(tint(t.yellow, 0x22))
                        .child(
                            div()
                                .w(px(3.0))
                                .h(px(16.0))
                                .flex_none()
                                .rounded(px(2.0))
                                .bg(t.yellow),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(12.0))
                                .text_color(t.yellow)
                                .child("This task waits for you"),
                        ),
                )
            })
            .child(
                row()
                    .w_full()
                    .gap(px(6.0))
                    .flex_wrap()
                    .text_size(px(11.5))
                    .child(status_pill(t, &task.status))
                    .child(pill(
                        if assigned.is_empty() {
                            "Not assigned".to_string()
                        } else {
                            format!("Assigned: {assigned}")
                        },
                        if assigned.is_empty() {
                            t.dim
                        } else {
                            t.foreground
                        },
                        t.surface,
                    ))
                    .when(!space.is_empty(), |el| {
                        el.child(pill(format!("Space: {space}"), t.muted, t.surface))
                    }),
            )
    }

    fn buttons(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Div {
        let id = task.id;
        let claimed = !task.terminal_id.is_empty();
        let held = model::agent_held(task);
        let closed = model::is_closed(&task.status);
        let idle = self.idle();
        let comments: &[Comment] = self
            .detail
            .as_ref()
            .map(|d| d.comments.as_slice())
            .unwrap_or(&[]);
        let before = model::worked_on(task, comments);
        let ready = !task.space_id.is_empty() && !task.assigned.is_empty() && task.archived_at == 0;
        let go_on = !model::special_holder(task) && (held || before);
        let who = if task.claimed_by.is_empty() {
            task.terminal_id.clone()
        } else {
            task.claimed_by.clone()
        };
        let mut out = row().w_full().gap(px(6.0)).flex_wrap();
        if !closed && !held && !before {
            let tip = if claimed {
                format!("Already claimed by {who}")
            } else {
                "Start the assigned agent now in this task's space".to_string()
            };
            out = out.child(chip(
                t,
                "tasks-d-start",
                if self.is_busy("start") {
                    "Starting…"
                } else {
                    "Run now"
                },
                &tip,
                look(Icon::Play).color(t.green).enabled(idle && !claimed),
                cx,
                move |view, _, cx| view.start(id, cx),
            ));
        }
        if go_on {
            let task_c = task.clone();
            out = out.child(chip(
                t,
                "tasks-d-continue",
                if self.is_busy("continue") {
                    "Continuing…"
                } else {
                    "Continue"
                },
                &model::continue_tip(task, held),
                look(Icon::Play).color(t.green).enabled(idle && ready),
                cx,
                move |view, _, cx| view.continue_task(&task_c, "", cx),
            ));
            if task.archived_at == 0 {
                out = out.child(chip(
                    t,
                    "tasks-d-handover",
                    "Hand over",
                    "Give this task to another agent. The work so far goes with it.",
                    look(Icon::Send)
                        .color(t.blue)
                        .enabled(idle && !task.space_id.is_empty()),
                    cx,
                    move |view, _, _| view.confirm_handover = Some((id, String::new())),
                ));
            }
        }
        if claimed {
            out = out.child(chip(
                t,
                "tasks-d-release",
                if self.is_busy("release") {
                    "Freeing…"
                } else {
                    "Free claim"
                },
                &format!("Take the task away from {who}. It goes back to To do."),
                look(Icon::X).enabled(idle),
                cx,
                move |view, _, cx| view.release(id, cx),
            ));
        }
        if task.status != "done" {
            let task_c = task.clone();
            out = out.child(chip(
                t,
                "tasks-d-done",
                "Mark done",
                "Set the status to Done. It leaves the list. Undo is shown above.",
                look(Icon::Check).color(t.green).enabled(idle),
                cx,
                move |view, _, cx| view.close_task(&task_c, "done", cx),
            ));
        }
        if !closed {
            let task_c = task.clone();
            out = out.child(chip(
                t,
                "tasks-d-cancel",
                "Cancel task",
                "Set the status to Cancelled. It leaves the list. Undo is shown above.",
                look(Icon::X).color(t.yellow).enabled(idle),
                cx,
                move |view, _, cx| view.close_task(&task_c, "cancelled", cx),
            ));
        }
        if claimed {
            let terminal = task.terminal_id.clone();
            out = out.child(chip(
                t,
                "tasks-d-terminal",
                "Go to terminal",
                &format!("Show terminal {}", task.terminal_id),
                look(Icon::Terminal),
                cx,
                move |view, _, cx| view.go_terminal(&terminal, cx),
            ));
        }
        out = if task.archived_at > 0 {
            out.child(chip(
                t,
                "tasks-d-unarchive",
                if self.is_busy("unarchive") {
                    "Bringing back…"
                } else {
                    "Unarchive"
                },
                "Show this task in the list again. Its status stays as it is.",
                look(Icon::Archive).enabled(idle),
                cx,
                move |view, _, cx| view.archive_task(id, false, cx),
            ))
        } else {
            let tip = if closed { ARCHIVE_TIP } else { CANCEL_ARCHIVE_TIP };
            out.child(chip(
                t,
                "tasks-d-archive",
                if self.is_busy("archive") {
                    "Archiving…"
                } else {
                    "Archive"
                },
                tip,
                look(Icon::Archive).enabled(idle),
                cx,
                move |view, _, cx| view.archive_task(id, true, cx),
            ))
        };
        out.child(chip(
            t,
            "tasks-d-delete",
            "Delete",
            "Delete this task for good. Asks first.",
            look(Icon::Trash).color(t.red).enabled(idle),
            cx,
            move |view, _, _| view.confirm_delete = Some(id),
        ))
    }

    fn confirm_box(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Div {
        let id = task.id;
        boxed(t, tint(t.red, 0x88))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.red)
                    .child(format!("Delete #{id} for good?")),
            )
            .child(line(model::delete_text(task), t.foreground, 11.5))
            .child(
                row()
                    .justify_end()
                    .gap(px(6.0))
                    .child(chip(
                        t,
                        "tasks-delete-no",
                        "No",
                        "Keep the task",
                        look(Icon::X),
                        cx,
                        |view, _, _| view.confirm_delete = None,
                    ))
                    .child(chip(
                        t,
                        "tasks-delete-yes",
                        if self.is_busy("delete") {
                            "Deleting…"
                        } else {
                            "Yes"
                        },
                        &format!("Delete #{id}"),
                        look(Icon::Trash).color(t.red).enabled(self.idle()),
                        cx,
                        move |view, _, cx| view.delete_task(id, cx),
                    )),
            )
    }

    fn handover_box(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Div {
        let picked = self
            .confirm_handover
            .as_ref()
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let current = model::assigned_value(&self.options, task);
        let options: Vec<model::Opt> = model::assignee_options(&self.options, "none")
            .into_iter()
            .filter(|o| o.value != "none" && o.value != current)
            .collect();
        let label = options
            .iter()
            .find(|o| o.value == picked)
            .map(|o| o.label.clone())
            .unwrap_or_default();
        let id = task.id;
        let task_c = task.clone();
        let pick_c = picked.clone();
        let drop = spec(Drop::DHandover, &picked, options)
            .label("New agent")
            .display(if label.is_empty() {
                "Pick who takes it over".to_string()
            } else {
                label.clone()
            })
            .search()
            .width(320.0)
            .enabled(self.idle());
        boxed(t, tint(t.blue, 0x88))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.blue)
                    .child(format!("Hand #{id} over to another agent")),
            )
            .child(dropdown(self, t, drop, cx))
            .child(line(model::handover_text(task, &label), t.foreground, 11.5))
            .child(
                row()
                    .justify_end()
                    .gap(px(6.0))
                    .child(chip(
                        t,
                        "tasks-handover-no",
                        "Keep",
                        "Keep the task with the agent it has now",
                        look(Icon::X),
                        cx,
                        |view, _, _| view.confirm_handover = None,
                    ))
                    .child(chip(
                        t,
                        "tasks-handover-yes",
                        if self.is_busy("continue") {
                            "Handing over…"
                        } else {
                            "Hand over"
                        },
                        &if label.is_empty() {
                            "Pick the new agent first".to_string()
                        } else {
                            format!("Give #{id} to {label}")
                        },
                        look(Icon::Send)
                            .color(t.blue)
                            .enabled(self.idle() && !picked.is_empty()),
                        cx,
                        move |view, _, cx| view.continue_task(&task_c, &pick_c, cx),
                    )),
            )
    }

    fn description(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Div {
        let saved = task.description.clone();
        let html = model::is_html(&saved);
        let edited = area_value(&self.inputs.description, cx) != saved;
        let formatted = html && !edited && self.raw_text != Some(task.id);
        let id = task.id;
        let play = if self.read_id.is_some() {
            chip(
                t,
                "tasks-d-desc-stop",
                "Stop",
                "Stop reading aloud",
                look(Icon::X).color(t.red),
                cx,
                |view, _, cx| view.stop_reading(cx),
            )
        } else {
            chip(
                t,
                "tasks-d-desc-play",
                "Play",
                "Read the saved title and description aloud with Nixa's voice",
                look(Icon::Play).color(t.green).enabled(!self.reading),
                cx,
                move |view, _, cx| view.read_task(id, cx),
            )
        };
        let mut head = row()
            .w_full()
            .items_end()
            .gap(px(6.0))
            .child(div().flex_1().min_w_0().child(caption(t, "Description")))
            .child(play);
        if formatted {
            head = head.child(chip(
                t,
                "tasks-d-desc-edit",
                "Edit text",
                "Show the description as plain text to change it",
                Default::default(),
                cx,
                move |view, _, _| view.raw_text = Some(id),
            ));
        } else if html && !edited {
            head = head.child(chip(
                t,
                "tasks-d-desc-formatted",
                "Show formatted",
                "Show the description with its formatting",
                Default::default(),
                cx,
                |view, _, _| view.raw_text = None,
            ));
        }
        let body = if formatted {
            div()
                .id("tasks-d-desc-view")
                .debug_selector(|| "tasks-d-desc-view".into())
                .w_full()
                .child(self.inputs.description_preview.clone())
                .into_any_element()
        } else {
            self.inputs.description.clone().into_any_element()
        };
        col().w_full().gap(px(3.0)).child(head).child(body)
    }

    fn text_fields(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Div {
        let changed = self.text_changed(cx);
        let description = self.description(t, task, cx);
        col()
            .w_full()
            .gap(px(6.0))
            .child(
                col()
                    .w_full()
                    .gap(px(3.0))
                    .child(caption(t, "Title"))
                    .child(self.inputs.title.clone()),
            )
            .child(description)
            .child(
                row()
                    .w_full()
                    .gap(px(6.0))
                    .justify_end()
                    .when(changed, |el| {
                        el.child(chip(
                            t,
                            "tasks-d-text-undo",
                            "Undo",
                            "Put back the saved title and description",
                            look(Icon::X),
                            cx,
                            |view, _, cx| view.undo_text_edit(cx),
                        ))
                    })
                    .child(chip(
                        t,
                        "tasks-d-text-save",
                        if self.is_busy("update") {
                            "Saving…"
                        } else {
                            "Save text"
                        },
                        if changed {
                            "Save the title and description"
                        } else {
                            "Nothing changed"
                        },
                        look(Icon::Check)
                            .color(t.green)
                            .enabled(changed && self.idle()),
                        cx,
                        |view, _, cx| view.save_text(cx),
                    )),
            )
    }

    fn fields(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Div {
        let o = &self.options;
        let space_value = if task.space_id.is_empty() {
            "none".to_string()
        } else {
            task.space_id.clone()
        };
        let assigned = model::assigned_value(&self.options, task);
        let assign_value = if assigned.is_empty() {
            "none".to_string()
        } else {
            assigned.clone()
        };
        let then = model::then_value(&self.options, task);
        let status = spec(Drop::DStatus, &task.status, model::status_options(o))
            .label("Status")
            .color(status_color(t, &task.status));
        let blocked = (task.status == "blocked").then(|| {
            spec(Drop::DOn, &task.blocked_on, model::blocked_options())
                .label("Blocked on")
                .display(if task.blocked_on.is_empty() {
                    "Not said".to_string()
                } else {
                    model::blocked_label(&task.blocked_on)
                })
        });
        let space_name = if task.space_name.is_empty() {
            task.space_id.clone()
        } else {
            task.space_name.clone()
        };
        let space = spec(
            Drop::DSpace,
            &space_value,
            model::with_gone(
                model::space_options(o, "none"),
                &task.space_id,
                &space_name,
                "This space is not open now",
            ),
        )
        .label("Space")
        .search();
        let assign = spec(
            Drop::DAssign,
            &assign_value,
            model::with_gone(
                model::assignee_options(o, "none"),
                &assigned,
                &task.assigned,
                "Not in the team list now",
            ),
        )
        .label("Assigned")
        .search()
        .width(320.0);
        let mut then_spec = spec(
            Drop::DThen,
            &then,
            model::with_gone(
                model::then_options(o),
                &then,
                &task.then_member,
                "Not in the team list now",
            ),
        )
        .label("Then")
        .search()
        .width(320.0);
        if task.then_member.is_empty() {
            then_spec = then_spec.display("Usual next step");
        }
        let group = spec(
            Drop::DGroup,
            &model::group_of(task),
            model::group_options(o),
        )
        .label("Group");
        let flow_value = if task.workflow.is_empty() {
            "none".to_string()
        } else {
            task.workflow.clone()
        };
        let mut flow = spec(
            Drop::DWorkflow,
            &flow_value,
            model::with_gone(
                model::workflow_options(o),
                &task.workflow,
                &task.workflow,
                "Not saved now",
            ),
        )
        .label("Workflow")
        .search()
        .width(320.0);
        if task.workflow.is_empty() {
            flow = flow.display("No workflow");
        }
        let status = dropdown(self, t, status, cx);
        let blocked = blocked
            .map(|s| dropdown(self, t, s, cx))
            .unwrap_or_else(|| div().into_any_element());
        let space = dropdown(self, t, space, cx);
        let assign = dropdown(self, t, assign, cx);
        let then = dropdown(self, t, then_spec, cx);
        let group = dropdown(self, t, group, cx);
        let flow = dropdown(self, t, flow, cx);
        col()
            .w_full()
            .gap(px(8.0))
            .child(pair(status, blocked))
            .child(pair(space, assign))
            .child(pair(then, group))
            .child(pair(flow, div()))
    }

    fn run_box(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Option<Div> {
        let run = self.detail.as_ref()?.run.clone()?;
        let id = task.id;
        let idle = self.idle();
        let active = model::run_active(Some(&run));
        let color = match run.state.as_str() {
            "waiting" => t.yellow,
            "paused" => t.blue,
            "running" => t.green,
            _ => t.dim,
        };
        let busy = self.is_busy("workflow");
        let mut out = boxed(t, t.border).child(
            row()
                .w_full()
                .gap(px(6.0))
                .child(pill(run.state.clone(), color, tint(color, 0x22)).text_size(px(11.0)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.5))
                        .text_color(t.foreground)
                        .whitespace_normal()
                        .child(if run.line.is_empty() {
                            format!("Workflow {}", run.title)
                        } else {
                            run.line.clone()
                        }),
                ),
        );
        if run.state == "waiting" && !run.ask_why.is_empty() {
            out = out.child(line(format!("Waiting: {}", run.ask_why), t.yellow, 11.0));
        }
        if active {
            let mut buttons = row().w_full().gap(px(6.0)).flex_wrap();
            if run.state == "running" {
                buttons = buttons.child(chip(
                    t,
                    "tasks-d-run-pause",
                    if busy { "Pausing…" } else { "Pause" },
                    "Finish the current stage, then wait before the next one",
                    look(Icon::X).enabled(idle),
                    cx,
                    move |view, _, cx| view.workflow_act(id, "pause", cx),
                ));
            }
            if run.state == "paused" {
                buttons = buttons.child(chip(
                    t,
                    "tasks-d-run-resume",
                    if busy { "Resuming…" } else { "Resume" },
                    "Start the next stage of the run",
                    look(Icon::Play).color(t.green).enabled(idle),
                    cx,
                    move |view, _, cx| view.workflow_act(id, "resume", cx),
                ));
            }
            buttons = buttons.child(chip(
                t,
                "tasks-d-run-cancel",
                "Cancel run",
                "Stop the run now. Asks first.",
                look(Icon::X).color(t.red).enabled(idle),
                cx,
                move |view, _, _| view.confirm_run_cancel = Some(id),
            ));
            out = out.child(buttons);
        }
        if self.confirm_run_cancel == Some(id) {
            out = out.child(
                row()
                    .w_full()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.5))
                            .text_color(t.red)
                            .child("Cancel this workflow run? The agent is stopped, the task is cancelled, and the changed files stay."),
                    )
                    .child(chip(t, "tasks-run-cancel-no", "No", "Keep the run", look(Icon::X), cx, |view, _, _| view.confirm_run_cancel = None))
                    .child(chip(
                        t,
                        "tasks-run-cancel-yes",
                        if busy { "Cancelling…" } else { "Yes" },
                        &format!("Cancel the run of #{id}"),
                        look(Icon::X).color(t.red).enabled(idle),
                        cx,
                        move |view, _, cx| view.workflow_act(id, "cancel", cx),
                    )),
            );
        }
        Some(out)
    }

    fn facts(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Div {
        let now = model::now_ms();
        let off = self.offset_min;
        let dash = |s: String| if s.is_empty() { "-".to_string() } else { s };
        let git = model::git_line(task);
        let mut out = col()
            .w_full()
            .gap(px(4.0))
            .child(info(
                t,
                "Group",
                model::group_label(&model::group_of(task)),
                t.foreground,
            ))
            .child(info(t, "Source", model::source_line(task), t.foreground));
        if !task.terminal_id.is_empty() {
            out = out.child(info(
                t,
                "Claimed by",
                model::claim_line(task, now),
                t.yellow,
            ));
        }
        out = out
            .child(info(t, "Made by", model::made_by_line(task), t.foreground))
            .child(info(
                t,
                "Made",
                dash(model::when(task.created_at, now, off)),
                t.foreground,
            ))
            .child(info(
                t,
                "Updated",
                dash(model::when(task.updated_at, now, off)),
                t.foreground,
            ));
        if task.closed_at > 0 {
            out = out.child(info(
                t,
                "Closed",
                model::when(task.closed_at, now, off),
                t.foreground,
            ));
        }
        if !git.is_empty() {
            out = out.child(info(t, "Git", git, t.foreground));
        }
        if !task.pr_url.is_empty() {
            let url = task.pr_url.clone();
            out = out.child(
                row()
                    .w_full()
                    .gap(px(8.0))
                    .child(
                        div()
                            .w(px(96.0))
                            .flex_none()
                            .text_size(px(11.0))
                            .text_color(t.dim)
                            .child("Pull request"),
                    )
                    .child(chip(
                        t,
                        "tasks-d-pr",
                        "Open PR",
                        &task.pr_url,
                        look(Icon::Send),
                        cx,
                        move |view, _, cx| view.open_link(&url, cx),
                    )),
            );
        }
        out
    }

    fn comment_row(&self, t: &Theme, c: &Comment, cx: &mut Context<Self>) -> AnyElement {
        let now = model::now_ms();
        let color = match (c.kind.as_str(), c.author_kind.as_str()) {
            ("error", _) => t.red,
            ("question", _) => t.yellow,
            ("result", _) => t.green,
            (_, "user") => t.blue,
            _ => t.border,
        };
        let safe = super::drop::safe_id(&c.id);
        let links: Vec<AnyElement> = model::link_urls(&c.links)
            .into_iter()
            .enumerate()
            .map(|(i, (label, url))| {
                let u = url.clone();
                chip(
                    t,
                    &format!("tasks-c-{safe}-link-{i}"),
                    &label,
                    &url,
                    look(Icon::Send),
                    cx,
                    move |view, _, cx| view.open_link(&u, cx),
                )
                .into_any_element()
            })
            .collect();
        col()
            .id(SharedString::from(format!("tasks-c-{safe}")))
            .w_full()
            .gap(px(4.0))
            .p(px(8.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(tint(color, 0x77))
            .bg(t.surface)
            .child(
                div()
                    .w_full()
                    .text_size(px(10.5))
                    .text_color(t.dim)
                    .whitespace_normal()
                    .child(model::comment_meta(c, now)),
            )
            .child(
                div()
                    .w_full()
                    .text_size(px(12.0))
                    .text_color(t.foreground)
                    .whitespace_normal()
                    .child(if c.text.is_empty() {
                        "(empty)".to_string()
                    } else {
                        c.text.clone()
                    }),
            )
            .children(
                model::link_notes(&c.links)
                    .into_iter()
                    .map(|n| line(n, t.muted, 10.5)),
            )
            .when(!links.is_empty(), |el| {
                el.child(row().w_full().gap(px(5.0)).flex_wrap().children(links))
            })
            .into_any_element()
    }

    fn comments(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let list: Vec<Comment> = self
            .detail
            .as_ref()
            .map(|d| d.comments.clone())
            .unwrap_or_default();
        let kind = dropdown(
            self,
            t,
            spec(
                Drop::DKind,
                &self.comment_kind,
                model::comment_kind_options(),
            )
            .label("Add a comment"),
            cx,
        );
        let rows: Vec<AnyElement> = list.iter().map(|c| self.comment_row(t, c, cx)).collect();
        col()
            .w_full()
            .gap(px(6.0))
            .child(caption(t, &model::comments_text(list.len())))
            .child(
                boxed(t, t.border)
                    .child(kind)
                    .child(self.inputs.comment.clone())
                    .child(row().w_full().justify_end().child(chip(
                        t,
                        "tasks-c-add",
                        if self.is_busy("comment") {
                            "Adding…"
                        } else {
                            "Add comment"
                        },
                        "Add this comment to the task",
                        look(Icon::Plus).color(t.green).enabled(self.idle()),
                        cx,
                        |view, _, cx| view.add_comment(cx),
                    ))),
            )
            .children(rows)
            .when(list.is_empty(), |el| {
                el.child(line("No comments yet.", t.dim, 11.5))
            })
    }

    pub fn detail_panel(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(task) = self.shown_task().cloned() else {
            let title = self
                .selected
                .map(|id| format!("Task #{id}"))
                .unwrap_or_else(|| "Task".into());
            let body = if self.detail_error.is_empty() {
                line("Loading the task…", t.dim, 12.0)
            } else {
                line(self.detail_error.clone(), t.red, 12.0)
            };
            return panel_frame(t, title, true, vec![body.into_any_element()], cx)
                .into_any_element();
        };
        let mut body: Vec<AnyElement> = Vec::new();
        if !self.detail_error.is_empty() {
            body.push(line(self.detail_error.clone(), t.red, 11.5).into_any_element());
        }
        body.push(self.summary(t, &task).into_any_element());
        body.push(self.buttons(t, &task, cx).into_any_element());
        if self
            .confirm_handover
            .as_ref()
            .is_some_and(|(id, _)| *id == task.id)
        {
            body.push(self.handover_box(t, &task, cx).into_any_element());
        }
        if self.confirm_delete == Some(task.id) {
            body.push(self.confirm_box(t, &task, cx).into_any_element());
        }
        body.push(self.text_fields(t, &task, cx).into_any_element());
        body.push(self.fields(t, &task, cx).into_any_element());
        if let Some(run) = self.run_box(t, &task, cx) {
            body.push(run.into_any_element());
        }
        body.push(self.facts(t, &task, cx).into_any_element());
        body.push(self.comments(t, cx).into_any_element());
        panel_frame(t, model::panel_title(&task), true, body, cx).into_any_element()
    }
}
