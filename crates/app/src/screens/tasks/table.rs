use super::model::{self, COLUMNS, Col, MAX_ROWS, Task};
use super::parts::{chip, col, line, list_end, look, row, tint};
use super::top::{group_color, status_color};
use super::view::{Panel, TasksView};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, SharedString, Stateful, div, px,
};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon};

pub const ID_W: f32 = 58.0;
pub const STATUS_W: f32 = 82.0;
pub const ASSIGNED_W: f32 = 120.0;
pub const ACTIONS_W: f32 = 104.0;

fn cell(c: Col) -> Div {
    match c {
        Col::Id => div().w(px(ID_W)).flex_none(),
        Col::Title => div().flex_1().min_w(px(160.0)),
        Col::Status => div().w(px(STATUS_W)).min_w_0(),
        Col::Assigned => div().w(px(ASSIGNED_W)).min_w_0(),
    }
}

fn mark(needs: bool, t: &Theme, h: f32) -> Div {
    div()
        .w(px(3.0))
        .h(px(h))
        .flex_none()
        .rounded(px(2.0))
        .when(needs, |el| el.bg(t.yellow))
}

fn group_tag(t: &Theme, task: &Task) -> Div {
    let g = model::group_of(task);
    div()
        .flex_none()
        .text_size(px(10.5))
        .text_color(group_color(t, &g))
        .child(model::group_label(&g))
}

fn waits(t: &Theme) -> Div {
    div()
        .flex_none()
        .text_size(px(10.5))
        .text_color(t.yellow)
        .child("waits for you")
}

impl TasksView {
    fn row_base(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.panel == Some(Panel::Detail) && self.selected == Some(task.id);
        let bg: Option<Hsla> = if selected {
            Some(t.selected)
        } else if task.needs_user {
            Some(tint(t.yellow, 0x16))
        } else {
            None
        };
        let hover = t.hover;
        let id = format!("tasks-row-{}", task.id);
        let selector = id.clone();
        let open_id = task.id;
        row()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .w_full()
            .flex_none()
            .px(px(10.0))
            .gap(px(8.0))
            .text_size(px(12.0))
            .when_some(bg, |el, bg| el.bg(bg))
            .border_b_1()
            .border_color(t.border)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(model::row_label(task)))
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.open_task(open_id, cx)))
    }

    fn row_actions(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> Div {
        let closed = model::is_closed(&task.status);
        let idle = self.idle();
        let space = model::space_name(&self.options, task);
        let assigned = model::assigned_name(&self.options, task);
        let id = task.id;
        let mut out = row().w_full().gap(px(4.0)).justify_end();
        if model::runnable(task) {
            let run = task.clone();
            out = out.child(chip(
                t,
                &format!("tasks-row-run-{id}"),
                "",
                &model::run_tip(task, &space, &assigned),
                look(Icon::Play)
                    .color(if model::ready_to_run(task) {
                        t.green
                    } else {
                        t.dim
                    })
                    .enabled(idle),
                cx,
                move |view, _, cx| view.run_task(&run, cx),
            ));
        }
        if !closed {
            let done = task.clone();
            let cancel = task.clone();
            out = out
                .child(chip(
                    t,
                    &format!("tasks-row-done-{id}"),
                    "",
                    &format!("Mark #{id} done. It leaves the list. Undo is shown above."),
                    look(Icon::Check).color(t.green).enabled(idle),
                    cx,
                    move |view, _, cx| view.close_task(&done, "done", cx),
                ))
                .child(chip(
                    t,
                    &format!("tasks-row-cancel-{id}"),
                    "",
                    &format!("Cancel #{id}. It leaves the list. Undo is shown above."),
                    look(Icon::X).color(t.yellow).enabled(idle),
                    cx,
                    move |view, _, cx| view.close_task(&cancel, "cancelled", cx),
                ));
        }
        out
    }

    fn full_row(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> AnyElement {
        let assigned = model::assigned_name(&self.options, task);
        let title = if task.title.is_empty() {
            "(no title)".to_string()
        } else {
            task.title.clone()
        };
        let actions = self.row_actions(t, task, cx);
        self.row_base(t, task, cx)
            .h(px(30.0))
            .child(
                cell(Col::Id).child(
                    row()
                        .gap(px(5.0))
                        .child(mark(task.needs_user, t, 16.0))
                        .child(div().text_color(t.muted).child(format!("#{}", task.id))),
                ),
            )
            .child(
                cell(Col::Title).child(
                    row()
                        .w_full()
                        .gap(px(6.0))
                        .child(group_tag(t, task))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(t.foreground)
                                .child(title),
                        )
                        .when(!task.run_box.is_empty(), |el| {
                            el.child(
                                div()
                                    .flex_none()
                                    .max_w(px(140.0))
                                    .truncate()
                                    .px(px(5.0))
                                    .rounded(px(4.0))
                                    .bg(tint(t.blue, 0x22))
                                    .text_size(px(10.5))
                                    .text_color(t.blue)
                                    .child(task.run_box.clone()),
                            )
                        })
                        .when(task.needs_user, |el| el.child(waits(t))),
                ),
            )
            .child(
                cell(Col::Status)
                    .truncate()
                    .text_color(status_color(t, &task.status))
                    .child(model::status_label(&task.status)),
            )
            .child(
                cell(Col::Assigned)
                    .truncate()
                    .text_color(if assigned.is_empty() {
                        t.dim
                    } else {
                        t.foreground
                    })
                    .child(if assigned.is_empty() {
                        "-".to_string()
                    } else {
                        assigned
                    }),
            )
            .child(div().w(px(ACTIONS_W)).flex_none().child(actions))
            .into_any_element()
    }

    fn compact_row(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> AnyElement {
        let assigned = model::assigned_name(&self.options, task);
        let rest = if assigned.is_empty() {
            "Not assigned".to_string()
        } else {
            assigned
        };
        let title = if task.title.is_empty() {
            "(no title)".to_string()
        } else {
            task.title.clone()
        };
        let actions = self.row_actions(t, task, cx);
        self.row_base(t, task, cx)
            .py(px(6.0))
            .child(mark(task.needs_user, t, 32.0))
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.0))
                    .child(
                        row()
                            .w_full()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(t.muted)
                                    .child(format!("#{}", task.id)),
                            )
                            .child(group_tag(t, task))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(t.foreground)
                                    .child(title),
                            )
                            .when(task.needs_user, |el| el.child(waits(t))),
                    )
                    .child(
                        row()
                            .w_full()
                            .min_w_0()
                            .gap(px(6.0))
                            .text_size(px(11.0))
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(status_color(t, &task.status))
                                    .child(model::status_label(&task.status)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(t.muted)
                                    .child(rest),
                            ),
                    ),
            )
            .child(div().flex_none().child(actions))
            .into_any_element()
    }

    fn title_row(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> AnyElement {
        let title = if task.title.is_empty() {
            "(no title)".to_string()
        } else {
            task.title.clone()
        };
        self.row_base(t, task, cx)
            .h(px(30.0))
            .child(mark(task.needs_user, t, 16.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.foreground)
                    .child(title),
            )
            .into_any_element()
    }

    fn header(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let base = row()
            .w_full()
            .flex_none()
            .px(px(10.0))
            .gap(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .bg(t.surface);
        match self.panel {
            Some(Panel::Detail) => base.h(px(28.0)).child(
                div().flex_1().min_w_0().truncate().text_size(px(11.0)).text_color(t.dim).child("Titles only while a task is open. Close the task to see every column."),
            ),
            Some(Panel::New) => base.h(px(28.0)).child(div().flex_1().min_w_0().truncate().text_size(px(11.0)).text_color(t.dim).child(format!(
                "Short view while the panel is open. Sorted by {}. Close the panel to see every column.",
                self.sort.col.word()
            ))),
            None => {
                let mut out = base.h(px(30.0));
                for c in COLUMNS {
                    let on = self.sort.col == c;
                    let tip = if on { "Click to flip the order".to_string() } else { format!("Sort by {}", c.word()) };
                    let id = format!("tasks-col-{}", c.key());
                    let selector = id.clone();
                    let up = self.sort.up;
                    out = out.child(
                        cell(c).child(
                            row()
                                .id(SharedString::from(id))
                                .debug_selector(move || selector.clone())
                                .w_full()
                                .gap(px(4.0))
                                .cursor_pointer()
                                .tooltip(tooltip(tip))
                                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                    view.sort = model::flip(view.sort, c);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(if on { t.foreground } else { t.muted })
                                        .child(c.label()),
                                )
                                .when(on, |el| el.child(icon(if up { Icon::ArrowUp } else { Icon::ArrowDown }).size(px(11.0)).text_color(t.blue))),
                        ),
                    );
                }
                out.child(div().w(px(ACTIONS_W)).flex_none())
            }
        }
    }

    fn one_row(&self, t: &Theme, task: &Task, cx: &mut Context<Self>) -> AnyElement {
        match self.panel {
            Some(Panel::Detail) => self.title_row(t, task, cx),
            Some(Panel::New) => self.compact_row(t, task, cx),
            None => self.full_row(t, task, cx),
        }
    }

    pub fn table_area(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let list = model::sort_tasks(
            &model::shown(&self.tasks, &self.filters),
            self.sort,
            &self.options,
        );
        let more = list.len().saturating_sub(MAX_ROWS);
        let shown: Vec<Task> = list.iter().take(MAX_ROWS).cloned().collect();
        let mut rows: Vec<AnyElement> = Vec::new();
        if self.grouped {
            for g in model::group_tasks(&shown, &self.options) {
                let gid = super::drop::safe_id(&g.key);
                rows.push(
                    row()
                        .id(SharedString::from(format!(
                            "tasks-group-{}",
                            if gid.is_empty() {
                                "none".to_string()
                            } else {
                                gid
                            }
                        )))
                        .w_full()
                        .flex_none()
                        .h(px(28.0))
                        .px(px(10.0))
                        .gap(px(8.0))
                        .bg(tint(t.blue, 0x10))
                        .border_b_1()
                        .border_color(t.border)
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(12.0))
                                .text_color(if g.key.is_empty() {
                                    t.muted
                                } else {
                                    t.foreground
                                })
                                .child(g.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(t.dim)
                                .child(model::count_text(g.tasks.len())),
                        )
                        .into_any_element(),
                );
                for task in &g.tasks {
                    rows.push(self.one_row(t, task, cx));
                }
            }
        } else {
            for task in &shown {
                rows.push(self.one_row(t, task, cx));
            }
        }
        let filtered = self.filters.any() || !self.search_text(cx).is_empty();
        let empty = model::empty_text(!self.tasks.is_empty(), filtered, self.filters.done);
        let header = self.header(t, cx);
        col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .child(header)
            .child(
                col()
                    .id("tasks-rows")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(more > 0, |el| {
                        el.child(div().px(px(10.0)).py(px(8.0)).child(line(
                            model::more_rows_text(more),
                            t.dim,
                            11.5,
                        )))
                    })
                    .when(list.is_empty(), |el| {
                        el.child(
                            div()
                                .id("tasks-empty")
                                .debug_selector(|| "tasks-empty".into())
                                .px(px(10.0))
                                .py(px(14.0))
                                .child(line(empty, t.dim, 12.0)),
                        )
                    })
                    .child(list_end()),
            )
            .into_any_element()
    }
}
