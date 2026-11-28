use super::drop::{Drop, dropdown, spec};
use super::model;
use super::parts::{caption, chip, col, line, look, pair, panel_frame, row};
use super::view::TasksView;
use gpui::prelude::*;
use gpui::{AnyElement, Context, px};
use nixa_ui::{Icon, Theme};

impl TasksView {
    pub fn new_panel(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let d = self.draft.clone();
        let o = &self.options;
        let ready = !d.space.is_empty() && !d.assign.is_empty();
        let idle = self.idle();
        let space = dropdown(
            self,
            t,
            spec(Drop::NSpace, &d.space, model::space_options(o, ""))
                .label("Space")
                .search(),
            cx,
        );
        let assign = dropdown(
            self,
            t,
            spec(Drop::NAssign, &d.assign, model::assignee_options(o, ""))
                .label("Assigned")
                .search()
                .width(320.0),
            cx,
        );
        let mut then = spec(Drop::NThen, &d.then, model::then_options(o))
            .label("Then")
            .search()
            .width(320.0);
        if d.then.is_empty() {
            then = then.display("Usual next step");
        }
        let then = dropdown(self, t, then, cx);
        let group = dropdown(
            self,
            t,
            spec(Drop::NGroup, &d.group, model::group_options(o)).label("Group"),
            cx,
        );
        let body = vec![
            line(
                "A task you add starts in To do. Add and start runs it at once.",
                t.dim,
                11.5,
            )
            .into_any_element(),
            col()
                .w_full()
                .gap(px(3.0))
                .child(caption(t, "Title"))
                .child(self.inputs.new_title.clone())
                .into_any_element(),
            col()
                .w_full()
                .gap(px(3.0))
                .child(caption(t, "Description"))
                .child(self.inputs.new_description.clone())
                .into_any_element(),
            pair(space, assign).into_any_element(),
            pair(then, group).into_any_element(),
            row()
                .w_full()
                .gap(px(6.0))
                .justify_end()
                .child(chip(
                    t,
                    "tasks-n-cancel",
                    "Cancel",
                    "Close without adding",
                    look(Icon::X),
                    cx,
                    |view, _, cx| view.close_panel(cx),
                ))
                .child(chip(
                    t,
                    "tasks-n-add",
                    if self.is_busy("add") {
                        "Adding…"
                    } else {
                        "Add task"
                    },
                    "Add the task. It waits in To do.",
                    look(Icon::Plus).color(t.green).enabled(idle),
                    cx,
                    |view, _, cx| view.add_task(false, cx),
                ))
                .child(chip(
                    t,
                    "tasks-n-add-start",
                    "Add and start",
                    if ready {
                        "Add the task and start its agent now"
                    } else {
                        "Pick a Space and an Assigned agent first"
                    },
                    look(Icon::Play).color(t.green).enabled(idle && ready),
                    cx,
                    |view, _, cx| view.add_task(true, cx),
                ))
                .into_any_element(),
        ];
        panel_frame(t, "New task".into(), false, body, cx).into_any_element()
    }
}
