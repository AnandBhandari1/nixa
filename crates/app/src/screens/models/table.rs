use super::api::Call;
use super::model::{self, Field, Row, Table};
use super::parts::{cell, chip, heading, line, text_button, tip_cell, wrap_cell};
use crate::actions::ClosePane;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    MouseButton, MouseUpEvent, ScrollHandle, Task, Window, div, px,
};
use nixa_ui::text_input::TextInput;
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};
use std::time::Duration;

pub const POLL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAct {
    Up,
    Down,
    Pin,
    Out,
    Edit,
}

pub struct ModelsView {
    call: Call,
    pub table: Table,
    pub loaded: bool,
    pub connected: bool,
    pub error: String,
    pub busy: bool,
    pub loading: bool,
    pub filter: String,
    pub drag: Option<String>,
    pub selected: Option<String>,
    seq: u64,
    pub inputs: Vec<(Field, Entity<TextInput>)>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _poll: Task<()>,
}

impl Focusable for ModelsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn input_value(input: &Entity<TextInput>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

impl ModelsView {
    pub fn new(call: Call, cx: &mut Context<Self>) -> Self {
        let inputs = model::EDIT_FIELDS
            .iter()
            .map(|(label, field)| (*field, cx.new(|cx| TextInput::new(cx, *label))))
            .collect();
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let alive = this.update(cx, |view, cx| {
                    if view.idle() {
                        view.load(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut view = ModelsView {
            call,
            table: Table::default(),
            loaded: false,
            connected: false,
            error: String::new(),
            busy: false,
            loading: false,
            filter: String::new(),
            drag: None,
            selected: None,
            seq: 0,
            inputs,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _poll: poll,
        };
        view.load(cx);
        view
    }

    pub fn idle(&self) -> bool {
        !self.busy && !self.loading && self.drag.is_none()
    }

    pub fn row(&self, id: &str) -> Option<&Row> {
        self.table.rows.iter().find(|r| r.id == id)
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.seq += 1;
        let mine = self.seq;
        let call = self.call.clone();
        let job = cx
            .background_executor()
            .spawn(async move { call("list", None) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                view.loading = false;
                view.loaded = true;
                if mine == view.seq {
                    match out {
                        Ok(value) => {
                            view.table = model::parse_table(&value);
                            view.connected = true;
                            view.error.clear();
                        }
                        Err(e) => {
                            view.connected = false;
                            view.error = e;
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn act(&mut self, action: &str, body: Value, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.seq += 1;
        self.busy = true;
        cx.notify();
        let call = self.call.clone();
        let action = action.to_string();
        let job = cx
            .background_executor()
            .spawn(async move { call(&action, Some(body)) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                view.busy = false;
                match out {
                    Ok(value) => {
                        if let Some(rows) = model::reply_rows(&value) {
                            view.table.rows = rows;
                        }
                        view.error.clear();
                    }
                    Err(e) => view.error = e,
                }
                view.load(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn press(&mut self, act: RowAct, id: &str, cx: &mut Context<Self>) {
        let Some(row) = self.row(id).cloned() else {
            return;
        };
        let count = self.table.rows.len();
        match act {
            RowAct::Up => self.act("move", model::move_body(&row.id, model::up_rank(&row)), cx),
            RowAct::Down => self.act(
                "move",
                model::move_body(&row.id, model::down_rank(&row, count)),
                cx,
            ),
            RowAct::Pin => self.act("edit", model::pin_body(&row), cx),
            RowAct::Out => self.act("edit", model::out_body(&row), cx),
            RowAct::Edit => self.edit(&row, cx),
        }
    }

    pub fn start_drag(&mut self, id: &str, cx: &mut Context<Self>) {
        self.drag = Some(id.to_string());
        cx.notify();
    }

    pub fn drop_on(&mut self, id: &str, cx: &mut Context<Self>) {
        let dragged = self.drag.take();
        let body = self
            .row(id)
            .and_then(|row| model::drop_body(dragged.as_deref(), row));
        match body {
            Some(body) => self.act("move", body, cx),
            None => cx.notify(),
        }
    }

    pub fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    pub fn edit(&mut self, row: &Row, cx: &mut Context<Self>) {
        self.selected = Some(row.id.clone());
        for (field, input) in &self.inputs {
            let text = model::field_text(row, *field);
            input.update(cx, |i, cx| i.set_value(text, cx));
        }
        cx.notify();
    }

    pub fn close_edit(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        cx.notify();
    }

    pub fn field(&self, field: Field, cx: &App) -> String {
        self.inputs
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, i)| input_value(i, cx))
            .unwrap_or_default()
    }

    pub fn save_fields(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let body = model::fields_body(
            &id,
            &self.field(Field::Capabilities, cx),
            &self.field(Field::Secondary, cx),
            &self.field(Field::Pins, cx),
            &self.field(Field::PublicNames, cx),
        );
        self.act("edit", body, cx);
    }

    pub fn set_cost(&mut self, cost: &str, cx: &mut Context<Self>) {
        let Some(row) = self
            .selected
            .as_deref()
            .and_then(|id| self.row(id))
            .cloned()
        else {
            return;
        };
        self.act("edit", model::cost_body(&row, cost), cx);
    }

    pub fn set_filter(&mut self, cap: &str, cx: &mut Context<Self>) {
        self.filter = cap.to_string();
        cx.notify();
    }

    pub fn rerank(&mut self, cx: &mut Context<Self>) {
        let body = model::rank_body(&self.filter);
        self.act("rank", body, cx);
    }

    pub fn refresh_public(&mut self, cx: &mut Context<Self>) {
        self.act("refresh", json!({}), cx);
    }

    pub fn refresh_lists(&mut self, cx: &mut Context<Self>) {
        self.act(crate::screens::model_pick::REFRESH_ALL, json!({}), cx);
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.keystroke.key != "escape" {
            return;
        }
        if self.drag.is_some() {
            self.end_drag(cx);
        } else if self.selected.is_some() {
            self.close_edit(cx);
        } else {
            window.remove_window();
        }
    }

    fn header(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .px(px(12.0))
            .py(px(10.0))
            .flex()
            .flex_row()
            .gap(px(10.0))
            .items_center()
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(16.0))
                    .child(super::TITLE),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.muted)
                    .child(model::HINT_TEXT),
            )
            .child(
                text_button(t, "reload".into(), "Reload", "Read the model table again")
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.load(cx))),
            )
            .child(
                text_button(
                    t,
                    "rank".into(),
                    "Nixa: re-rank",
                    "Nixa ranks the rows that are not pinned, for the chosen filter",
                )
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.rerank(cx))),
            )
            .child(
                text_button(
                    t,
                    "public".into(),
                    "Refresh public scores",
                    "Read public scores again, then re-rank",
                )
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.refresh_public(cx))),
            )
            .child(
                text_button(
                    t,
                    "lists".into(),
                    "Refresh model lists",
                    "Read the model list of every installed agent again. Every model picker uses this list",
                )
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.refresh_lists(cx))),
            )
            .child(
                text_button(
                    t,
                    "team".into(),
                    "Agents",
                    "Open Settings at Agents and team",
                )
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                    cx.defer(|cx| crate::screens::settings::open(super::AGENTS_PART, cx));
                })),
            )
            .child(
                text_button(t, "tasks".into(), "Tasks", "Open the Tasks window")
                    .on_click(|_, _, cx| crate::screens::tasks::open(cx)),
            )
            .into_any_element()
    }

    fn chips(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div()
            .px(px(10.0))
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(3.0));
        for cap in std::iter::once("").chain(model::CAPS) {
            let id = format!("cap-{}", if cap.is_empty() { "all" } else { cap });
            let label = if cap.is_empty() { "All" } else { cap };
            row = row.child(chip(t, id, label, self.filter == cap).on_click(
                cx.listener(move |view, _: &ClickEvent, _, cx| view.set_filter(cap, cx)),
            ));
        }
        row.into_any_element()
    }

    fn plans(&self, t: &Theme) -> Option<AnyElement> {
        if self.table.plans.is_empty() {
            return None;
        }
        let header = div()
            .flex()
            .flex_row()
            .font_weight(FontWeight::SEMIBOLD)
            .children(
                model::PLAN_COLUMNS
                    .iter()
                    .map(|(text, width)| cell(*text, *width)),
            );
        let rows = self.table.plans.iter().enumerate().map(|(n, plan)| {
            let cells = model::plan_cells(plan);
            let widths = model::PLAN_COLUMNS;
            div()
                .flex()
                .flex_row()
                .child(tip_cell(
                    format!("plan-{n}"),
                    cells[0].clone(),
                    widths[0].1,
                    model::plan_tip(plan),
                ))
                .children((1..5).map(|i| cell(cells[i].clone(), widths[i].1)))
        });
        Some(
            div()
                .px(px(12.0))
                .py(px(6.0))
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(heading("Plan usage · actual starts"))
                .child(header)
                .children(rows)
                .text_color(t.foreground)
                .into_any_element(),
        )
    }

    fn decisions(&self, t: &Theme) -> Option<AnyElement> {
        let lines = model::decision_lines(&self.table.decisions);
        if lines.is_empty() {
            return None;
        }
        Some(
            div()
                .px(px(12.0))
                .py(px(5.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(heading("Recent model decisions"))
                .children(
                    lines
                        .into_iter()
                        .map(|l| div().truncate().text_color(t.dim).child(l)),
                )
                .into_any_element(),
        )
    }

    fn table_row(&self, t: &Theme, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let id = row.id.clone();
        let rows = &self.table.rows;
        let selected = self.selected.as_deref() == Some(id.as_str());
        let dragging = self.drag.is_some();
        let lifted = self.drag.as_deref() == Some(id.as_str());
        let hover = t.hover;
        let widths = model::TABLE_COLUMNS;
        let key = format!("model-{id}");
        let selector = key.clone();
        let drop_id = id.clone();
        let drag_id = id.clone();
        let mut el = div()
            .id(gpui::SharedString::from(key))
            .debug_selector(move || selector.clone())
            .min_w(px(model::TABLE_WIDTH))
            .flex()
            .flex_row()
            .items_center()
            .py(px(4.0))
            .border_b_1()
            .border_color(t.border)
            .bg(if selected || lifted {
                t.selected
            } else {
                t.background
            })
            .when(dragging && !lifted, |el| el.hover(move |s| s.bg(hover)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |view, _: &MouseUpEvent, _, cx| {
                    if view.drag.is_some() {
                        cx.stop_propagation();
                        view.drop_on(&drop_id, cx);
                    }
                }),
            )
            .child(
                tip_cell(format!("drag-{id}"), "⠿", widths[0].1, model::DRAG_TIP)
                    .cursor_grab()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| view.start_drag(&drag_id, cx)),
                    ),
            )
            .child(cell(row.rank.to_string(), widths[1].1))
            .child(tip_cell(
                format!("name-{id}"),
                model::name(row),
                widths[2].1,
                model::name_tip(row),
            ))
            .child(tip_cell(
                format!("score-{id}"),
                model::scores(row),
                widths[3].1,
                model::scores_tip(row),
            ))
            .child(wrap_cell(row.capabilities.join(", "), widths[4].1))
            .child(cell(row.cost.clone(), widths[5].1))
            .child(cell(model::state_text(row), widths[6].1))
            .child(tip_cell(
                format!("second-{id}"),
                model::secondary_text(row, rows),
                widths[7].1,
                model::secondary_tip(row),
            ))
            .child(tip_cell(
                format!("moved-{id}"),
                model::moved_text(row),
                widths[8].1,
                model::moved_tip(row),
            ));
        let buttons = [
            (
                RowAct::Up,
                "up",
                "↑".to_string(),
                "Move up; your move pins this row",
            ),
            (
                RowAct::Down,
                "down",
                "↓".to_string(),
                "Move down; your move pins this row",
            ),
            (
                RowAct::Pin,
                "pin",
                if row.pinned { "Pinned" } else { "Pin" }.to_string(),
                "Pinned rows keep their place when Nixa re-ranks",
            ),
            (
                RowAct::Out,
                "out",
                if row.disabled { "Restore" } else { "Take out" }.to_string(),
                "A row taken out is never picked",
            ),
            (
                RowAct::Edit,
                "edit",
                "Edit".to_string(),
                "Edit tags, cost, secondary and pins",
            ),
        ];
        for (act, name, label, tip) in buttons {
            let row_id = id.clone();
            el =
                el.child(
                    text_button(t, format!("{name}-{id}"), label, tip)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                            view.press(act, &row_id, cx)
                        })),
                );
        }
        el.into_any_element()
    }

    fn edit_panel(&self, t: &Theme, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let mut costs = div().flex().flex_row().gap(px(8.0));
        for cost in model::COSTS {
            let label = if row.cost == cost {
                format!("✓ {cost}")
            } else {
                cost.to_string()
            };
            costs = costs.child(
                text_button(t, format!("cost-{cost}"), label, "Set the cost tier").on_click(
                    cx.listener(move |view, _: &ClickEvent, _, cx| view.set_cost(cost, cx)),
                ),
            );
        }
        costs = costs
            .child(
                text_button(
                    t,
                    "save-fields".into(),
                    "Save fields",
                    "Save the four fields above",
                )
                .text_color(t.green)
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.save_fields(cx))),
            )
            .child(
                text_button(
                    t,
                    "close-edit".into(),
                    "Close",
                    "Close the edit panel (Esc)",
                )
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.close_edit(cx))),
            );
        let fields = model::EDIT_FIELDS.iter().map(|(label, field)| {
            let input = self
                .inputs
                .iter()
                .find(|(f, _)| f == field)
                .map(|(_, i)| i.clone());
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .items_center()
                .child(cell(*label, 260.0))
                .child(div().flex_1().min_w_0().children(input))
        });
        div()
            .debug_selector(|| "models-edit".into())
            .px(px(12.0))
            .py(px(8.0))
            .flex()
            .flex_col()
            .gap(px(5.0))
            .border_t_1()
            .border_color(t.border)
            .child(heading(format!("Edit {}", row.model)))
            .children(fields)
            .child(costs)
            .into_any_element()
    }
}

impl Render for ModelsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let selected = self
            .selected
            .as_deref()
            .and_then(|id| self.row(id))
            .cloned();
        let shown: Vec<Row> = model::shown(&self.table.rows, &self.filter)
            .into_iter()
            .cloned()
            .collect();
        let rows: Vec<AnyElement> = shown.iter().map(|r| self.table_row(&t, r, cx)).collect();
        let header = self.header(&t, cx);
        let chips = self.chips(&t, cx);
        let plans = self.plans(&t);
        let decisions = self.decisions(&t);
        let edit = selected.as_ref().map(|r| self.edit_panel(&t, r, cx));
        let empty = if self.connected {
            model::EMPTY_TEXT
        } else {
            model::CONNECTING_TEXT
        };
        let foot = self
            .table
            .file
            .clone()
            .unwrap_or_else(|| model::FOOT_TEXT.into());
        div()
            .id("models")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.0))
            .on_action(cx.listener(|_, _: &ClosePane, window, _| window.remove_window()))
            .on_key_down(cx.listener(Self::key))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _, cx| view.end_drag(cx)),
            )
            .child(header)
            .child(chips)
            .child(line(&t, model::info_text(self.busy, &self.table)))
            .children(
                model::scores_status(self.table.scores_online, &self.table.scores_note).map(
                    |status| {
                        line(&t, status.to_string())
                            .debug_selector(|| "models-scores-status".into())
                    },
                ),
            )
            .children(plans)
            .when(!self.error.is_empty(), |el| {
                el.child(
                    div()
                        .debug_selector(|| "models-error".into())
                        .px(px(12.0))
                        .py(px(5.0))
                        .text_color(t.red)
                        .child(self.error.clone()),
                )
            })
            .children(decisions)
            .child(
                div()
                    .id("models-scroll")
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .child(
                        div()
                            .min_w(px(model::TABLE_WIDTH))
                            .flex()
                            .flex_row()
                            .py(px(6.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .children(
                                model::TABLE_COLUMNS
                                    .iter()
                                    .map(|(text, width)| cell(*text, *width)),
                            ),
                    )
                    .children(rows)
                    .when(self.table.rows.is_empty(), |el| {
                        el.child(
                            div()
                                .debug_selector(|| "models-empty".into())
                                .p(px(16.0))
                                .child(empty),
                        )
                    }),
            )
            .children(edit)
            .child(line(&t, foot).truncate())
    }
}
