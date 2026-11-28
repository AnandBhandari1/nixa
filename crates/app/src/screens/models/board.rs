use super::api::Call;
use super::model::{self, Board};
use super::parts::{text_button, wrap_cell};
use crate::actions::ClosePane;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    MouseButton, MouseMoveEvent, ScrollHandle, ScrollWheelEvent, SharedString, Task, Window, div,
    px,
};
use nixa_ui::theme;
use std::time::Duration;

pub const POLL: Duration = Duration::from_secs(3);

pub struct BoardView {
    call: Call,
    pub board: Board,
    pub loaded: bool,
    pub error: String,
    pub loading: bool,
    seq: u64,
    focus: FocusHandle,
    scroll: ScrollHandle,
    dragging_scroll: bool,
    _poll: Task<()>,
}

impl Focusable for BoardView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BoardView {
    pub fn new(call: Call, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this.update(cx, |view, cx| view.load(cx)).is_err() {
                    break;
                }
            }
        });
        let mut view = BoardView {
            call,
            board: Board::default(),
            loaded: false,
            error: String::new(),
            loading: false,
            seq: 0,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            dragging_scroll: false,
            _poll: poll,
        };
        view.load(cx);
        view
    }

    pub fn idle(&self) -> bool {
        !self.loading
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
            .spawn(async move { call("view", None) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                view.loading = false;
                if mine == view.seq {
                    match out {
                        Ok(value) => {
                            view.board = model::parse_board(&value);
                            view.loaded = true;
                            view.error.clear();
                        }
                        Err(e) => view.error = e,
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn scroll_to(&self, x: gpui::Pixels) {
        let mut offset = self.scroll.offset();
        offset.x = x.clamp(-self.scroll.max_offset().width, px(0.0));
        self.scroll.set_offset(offset);
    }

    fn scroll_from_track(&self, x: gpui::Pixels) {
        let bounds = self.scroll.bounds();
        let width = f32::from(bounds.size.width);
        let max = f32::from(self.scroll.max_offset().width);
        if let Some((_, thumb)) = nixa_ui::scroll::thumb(width, max, 0.0)
            && width > thumb
        {
            let fraction = (f32::from(x - bounds.left()) - thumb / 2.0) / (width - thumb);
            self.scroll_to(px(-fraction * max));
        }
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, on_track: bool, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(24.0));
        let horizontal = delta.x != px(0.0);
        if horizontal
            || on_track
            || event.modifiers.shift
            || self.scroll.max_offset().height == px(0.0)
        {
            let step = if horizontal { delta.x } else { delta.y };
            self.scroll_to(self.scroll.offset().x + step);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, _: &mut Context<Self>) {
        if e.keystroke.key == "escape" {
            window.remove_window();
        }
    }
}

impl Render for BoardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let width = model::board_width();
        let rows: Vec<AnyElement> = self
            .board
            .rows
            .iter()
            .map(|row| {
                let key = format!("board-{}", row.id);
                let selector = key.clone();
                div()
                    .id(SharedString::from(key))
                    .debug_selector(move || selector.clone())
                    .min_w(px(width))
                    .flex()
                    .flex_row()
                    .py(px(7.0))
                    .border_b_1()
                    .border_color(t.border)
                    .children(
                        model::board_cells(row)
                            .into_iter()
                            .zip(model::BOARD_COLUMNS)
                            .map(|(text, (_, w))| wrap_cell(text, w)),
                    )
                    .into_any_element()
            })
            .collect();
        let choices = self.board.choices.iter().map(|choice| {
            let key = format!("choice-{}", choice.capability);
            div()
                .debug_selector(move || key.clone())
                .child(model::choice_text(choice))
        });
        let status = self
            .board
            .file
            .as_deref()
            .or_else(|| (!self.loaded && self.error.is_empty()).then_some(model::CONNECTING_TEXT))
            .map(str::to_owned);
        let scores = model::scores_status(self.board.scores_online, &self.board.scores_note)
            .map(str::to_owned);
        let viewport = f32::from(window.viewport_size().width);
        let thumb =
            nixa_ui::scroll::thumb(viewport, width - viewport, self.scroll.offset().x.into());
        div()
            .id("models-board")
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
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                if view.dragging_scroll && event.dragging() {
                    view.scroll_from_track(event.position.x);
                    cx.notify();
                } else {
                    view.dragging_scroll = false;
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.dragging_scroll = false),
            )
            .child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .flex()
                    .flex_row()
                    .gap(px(12.0))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "board-title".into())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(model::board_title(&self.board)),
                    )
                    .child(
                        text_button(
                            &t,
                            "board-reload".into(),
                            "Reload",
                            "Read the table again; this never changes it",
                        )
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.load(cx))),
                    ),
            )
            .child(
                div()
                    .px(px(12.0))
                    .pb(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .children(choices),
            )
            .children(status.map(|status| {
                div()
                    .id("board-status")
                    .debug_selector(|| "board-status".into())
                    .px(px(12.0))
                    .pb(px(8.0))
                    .text_color(t.dim)
                    .child(status)
            }))
            .children(scores.map(|scores| {
                div()
                    .px(px(12.0))
                    .pb(px(8.0))
                    .text_color(t.dim)
                    .child(scores)
            }))
            .when(!self.error.is_empty(), |el| {
                el.child(
                    div()
                        .debug_selector(|| "board-error".into())
                        .px(px(12.0))
                        .py(px(5.0))
                        .text_color(t.red)
                        .child(model::failed_text(&self.error)),
                )
            })
            .child(
                div()
                    .id("board-scroll")
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .on_scroll_wheel(cx.listener(|view, event, _, cx| view.wheel(event, false, cx)))
                    .child(
                        div()
                            .min_w(px(width))
                            .flex()
                            .flex_row()
                            .py(px(7.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .children(
                                model::BOARD_COLUMNS
                                    .iter()
                                    .map(|(title, w)| wrap_cell(*title, *w)),
                            ),
                    )
                    .children(rows)
                    .when(self.board.rows.is_empty(), |el| {
                        el.child(
                            div()
                                .debug_selector(|| "board-empty".into())
                                .p(px(16.0))
                                .child(model::BOARD_EMPTY_TEXT),
                        )
                    }),
            )
            .when_some(thumb, |el, (left, width)| {
                el.child(
                    div()
                        .id("board-horizontal-scroll")
                        .debug_selector(|| "board-horizontal-scroll".into())
                        .relative()
                        .w_full()
                        .h(px(14.0))
                        .flex_none()
                        .bg(t.chrome)
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, event: &gpui::MouseDownEvent, _, cx| {
                                view.dragging_scroll = true;
                                view.scroll_from_track(event.position.x);
                                cx.notify();
                            }),
                        )
                        .on_scroll_wheel(
                            cx.listener(|view, event, _, cx| view.wheel(event, true, cx)),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(3.0))
                                .left(px(left))
                                .w(px(width))
                                .h(px(8.0))
                                .rounded_full()
                                .bg(t.border_strong),
                        ),
                )
            })
    }
}
