use super::drop::{Drop, dropdown, spec};
use super::model::{self, NOT_CONNECTED};
use super::parts::{Look, chip, col, line, look, pill, row, tint};
use super::place::SPACES_PART;
use super::view::{Panel, TasksView};
use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, FontWeight, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon};

const SPACES_TIP: &str = "Open Nixa settings at Spaces: auto-start, open or locked";
const GROUP_TIP: &str = "Keep project work and Nixa brain gaps apart";

impl TasksView {
    pub fn top_bar(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        row()
            .w_full()
            .flex_none()
            .px(px(14.0))
            .py(px(10.0))
            .gap(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .child(icon(Icon::Tasks).size(px(17.0)).text_color(t.blue))
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.0))
                    .child(
                        row()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(14.0))
                                    .child("Tasks"),
                            )
                            .when(self.waiting > 0, |el| {
                                el.child(
                                    div()
                                        .id("tasks-waiting")
                                        .debug_selector(|| "tasks-waiting".into())
                                        .px(px(6.0))
                                        .py(px(1.0))
                                        .rounded(px(5.0))
                                        .bg(t.yellow)
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(t.background)
                                        .child(model::waiting_text(self.waiting)),
                                )
                            }),
                    ),
            )
            .child(chip(
                t,
                "tasks-new",
                "New task",
                "Add a task by hand",
                look(Icon::Plus)
                    .color(t.green)
                    .on(self.panel == Some(Panel::New)),
                cx,
                |view, window, cx| view.show_new(window, cx),
            ))
            .child(chip(
                t,
                "tasks-spaces",
                "Spaces",
                SPACES_TIP,
                look(Icon::Layers),
                cx,
                |view, _, cx| (view.host.settings)(SPACES_PART, cx),
            ))
            .child(chip(
                t,
                "tasks-refresh",
                "",
                "Load the list again",
                look(Icon::Refresh).enabled(self.connected),
                cx,
                |view, _, cx| view.reload(cx),
            ))
    }

    pub fn filter_bar(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let f = self.filters.clone();
        let o = &self.options;
        let count = model::shown(&self.tasks, &f).len();
        let mut spaces = vec![model::opt("", "All spaces", "")];
        spaces.extend(model::space_options(o, "none"));
        let mut groups = vec![model::opt("", "All groups", "")];
        groups.extend(model::group_options(o));
        let mut sources = vec![model::opt("", "All sources", "")];
        sources.extend(
            o.sources
                .iter()
                .map(|s| model::opt(s, &model::source_label(s), "")),
        );
        let mut people = vec![model::opt("", "Anyone", "")];
        people.extend(model::assignee_options(o, "none"));
        let drops = [
            (
                170.0,
                spec(Drop::FSpace, &f.space, spaces)
                    .search()
                    .tip("Show one space"),
            ),
            (
                150.0,
                spec(
                    Drop::FStatus,
                    &f.status,
                    model::filter_status_options(o, f.done),
                )
                .tip("Show one status"),
            ),
            (
                150.0,
                spec(Drop::FGroup, &f.group, groups).tip(GROUP_TIP),
            ),
            (
                150.0,
                spec(Drop::FSource, &f.source, sources).tip("Show where tasks came from"),
            ),
            (
                170.0,
                spec(Drop::FAssigned, &f.assigned, people)
                    .search()
                    .width(320.0)
                    .tip("Show tasks of one member or agent type"),
            ),
        ];
        let drops: Vec<AnyElement> = drops
            .into_iter()
            .map(|(w, s)| {
                div()
                    .w(px(w))
                    .flex_none()
                    .child(dropdown(self, t, s, cx))
                    .into_any_element()
            })
            .collect();
        let grouped_tip = if self.grouped {
            "Grouped by space. Click for one list."
        } else {
            "One list. Click to group by space."
        };
        let archived_tip = if f.archived {
            "Showing only archived tasks. Click to go back to the list."
        } else {
            "Archived tasks are hidden. Click to see only them."
        };
        let done_tip = if f.done {
            "Showing done and cancelled tasks too. Click to hide them."
        } else {
            "Done and cancelled tasks are hidden. Click to show them."
        };
        row()
            .w_full()
            .flex_none()
            .px(px(14.0))
            .py(px(8.0))
            .gap(px(8.0))
            .flex_wrap()
            .border_b_1()
            .border_color(t.border)
            .children(drops)
            .child(
                div()
                    .w(px(230.0))
                    .flex_none()
                    .child(self.inputs.search.clone()),
            )
            .child(chip(
                t,
                "tasks-group",
                "Group by space",
                grouped_tip,
                Look::default().on(self.grouped),
                cx,
                |view, _, _| view.grouped = !view.grouped,
            ))
            .child(chip(
                t,
                "tasks-archived",
                "Archived",
                archived_tip,
                look(Icon::Archive).on(f.archived),
                cx,
                |view, _, cx| view.toggle_archived(cx),
            ))
            .child(chip(
                t,
                "tasks-done",
                "Show done",
                done_tip,
                Look::default().on(f.done),
                cx,
                |view, _, cx| view.toggle_done(cx),
            ))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(t.dim)
                    .child(model::count_text(count)),
            )
    }

    pub fn notices(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let mut out = col().w_full().flex_none();
        if let Some(undo) = self.undo.clone() {
            let color = if undo.status == "done" {
                t.green
            } else {
                t.yellow
            };
            out = out.child(
                row()
                    .id("tasks-undo")
                    .w_full()
                    .px(px(14.0))
                    .py(px(6.0))
                    .gap(px(8.0))
                    .bg(tint(color, 0x14))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(t.foreground)
                            .child(model::undo_text(&undo)),
                    )
                    .child(chip(
                        t,
                        "tasks-undo-do",
                        if self.is_busy("update") {
                            "Undoing…"
                        } else {
                            "Undo"
                        },
                        &model::undo_tip(&undo),
                        look(Icon::Undo).enabled(self.idle()),
                        cx,
                        |view, _, cx| view.undo_close(cx),
                    ))
                    .child(chip(
                        t,
                        "tasks-undo-close",
                        "",
                        "Hide this message",
                        look(Icon::X),
                        cx,
                        |view, _, _| view.undo = None,
                    )),
            );
        }
        if !self.connected && self.loaded {
            out = out.child(
                div()
                    .id("tasks-offline")
                    .w_full()
                    .px(px(14.0))
                    .py(px(6.0))
                    .bg(t.surface)
                    .text_size(px(12.0))
                    .text_color(t.yellow)
                    .tooltip(tooltip("Trying again every 3 seconds"))
                    .child(NOT_CONNECTED),
            );
        }
        if !self.load_error.is_empty() && self.connected {
            out = out.child(div().w_full().px(px(14.0)).py(px(6.0)).child(line(
                format!("Could not load the tasks: {}", self.load_error),
                t.red,
                12.0,
            )));
        }
        if !self.note.is_empty() {
            out = out.child(
                row()
                    .w_full()
                    .px(px(14.0))
                    .py(px(6.0))
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(line(self.note.clone(), t.green, 12.0)),
                    )
                    .child(chip(
                        t,
                        "tasks-note-close",
                        "",
                        "Hide this message",
                        look(Icon::X),
                        cx,
                        |view, _, _| view.note.clear(),
                    )),
            );
        }
        if !self.error.is_empty() {
            out = out.child(
                row()
                    .w_full()
                    .px(px(14.0))
                    .py(px(6.0))
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("tasks-error")
                            .debug_selector(|| "tasks-error".into())
                            .flex_1()
                            .min_w_0()
                            .child(line(self.error.clone(), t.red, 12.0)),
                    )
                    .child(chip(
                        t,
                        "tasks-error-close",
                        "",
                        "Hide this message",
                        look(Icon::X),
                        cx,
                        |view, _, _| view.error.clear(),
                    )),
            );
        }
        out
    }

    pub fn body(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if !self.loaded {
            let (text, color) = if self.load_error.is_empty() {
                ("Loading the tasks…", t.dim)
            } else if model::offline(&self.load_error) {
                (NOT_CONNECTED, t.yellow)
            } else {
                (
                    "No tasks to show. Press the round arrow to try again.",
                    t.dim,
                )
            };
            return col()
                .id("tasks-loading")
                .debug_selector(|| "tasks-loading".into())
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .child(line(text, color, 12.5))
                .into_any_element();
        }
        let table = self.table_area(t, cx);
        let panel = match self.panel {
            Some(Panel::Detail) => Some(self.detail_panel(t, cx)),
            Some(Panel::New) => Some(self.new_panel(t, cx)),
            None => None,
        };
        row()
            .w_full()
            .flex_1()
            .min_h_0()
            .items_start()
            .child(table)
            .children(panel)
            .into_any_element()
    }
}

pub fn status_color(t: &Theme, status: &str) -> gpui::Hsla {
    match status {
        "inbox" => t.cyan,
        "todo" => t.blue,
        "doing" => t.yellow,
        "blocked" => t.red,
        "review" => t.magenta,
        "done" => t.green,
        "cancelled" => t.dim,
        _ => t.foreground,
    }
}

pub fn group_color(t: &Theme, group: &str) -> gpui::Hsla {
    match group {
        "nixa-gap" => t.cyan,
        _ => t.dim,
    }
}

pub fn status_pill(t: &Theme, status: &str) -> Div {
    let color = status_color(t, status);
    pill(model::status_label(status), color, tint(color, 0x22))
}
