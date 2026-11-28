use super::model::{Opt, SHOWN_OPTIONS, matches};
use super::parts::{caption, line, row};
use super::view::TasksView;
use gpui::prelude::*;
use gpui::{
    AnchoredPositionMode, AnyElement, ClickEvent, Context, Focusable, Hsla, MouseButton,
    MouseDownEvent, SharedString, anchored, deferred, div, point, px,
};
use nixa_ui::popover::{popover, tooltip};
use nixa_ui::{Icon, Theme, icon};
use std::time::{Duration, Instant};

pub const REOPEN_GAP: Duration = Duration::from_millis(350);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    FSpace,
    FStatus,
    FGroup,
    FSource,
    FAssigned,
    DStatus,
    DOn,
    DSpace,
    DAssign,
    DThen,
    DGroup,
    DWorkflow,
    DHandover,
    DKind,
    NSpace,
    NAssign,
    NThen,
    NGroup,
}

impl Drop {
    pub fn key(self) -> &'static str {
        match self {
            Drop::FSpace => "f-space",
            Drop::FStatus => "f-status",
            Drop::FGroup => "f-group",
            Drop::FSource => "f-source",
            Drop::FAssigned => "f-assigned",
            Drop::DStatus => "d-status",
            Drop::DOn => "d-on",
            Drop::DSpace => "d-space",
            Drop::DAssign => "d-assign",
            Drop::DThen => "d-then",
            Drop::DGroup => "d-group",
            Drop::DWorkflow => "d-workflow",
            Drop::DHandover => "d-handover",
            Drop::DKind => "d-comment-kind",
            Drop::NSpace => "n-space",
            Drop::NAssign => "n-assign",
            Drop::NThen => "n-then",
            Drop::NGroup => "n-group",
        }
    }
}

pub fn safe_id(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub fn trigger_id(which: Drop) -> String {
    format!("tasks-dd-{}", which.key())
}

pub fn option_id(which: Drop, value: &str) -> String {
    let value = safe_id(value);
    format!(
        "tasks-dd-{}-{}",
        which.key(),
        if value.is_empty() { "empty" } else { &value }
    )
}

pub struct Spec {
    pub which: Drop,
    pub label: Option<&'static str>,
    pub value: String,
    pub display: Option<String>,
    pub options: Vec<Opt>,
    pub search: bool,
    pub width: f32,
    pub enabled: bool,
    pub tip: String,
    pub color: Option<Hsla>,
    pub empty: &'static str,
}

pub fn spec(which: Drop, value: &str, options: Vec<Opt>) -> Spec {
    Spec {
        which,
        label: None,
        value: value.to_string(),
        display: None,
        options,
        search: false,
        width: 300.0,
        enabled: true,
        tip: String::new(),
        color: None,
        empty: "Nothing to pick.",
    }
}

impl Spec {
    pub fn label(mut self, label: &'static str) -> Spec {
        self.label = Some(label);
        self
    }

    pub fn display(mut self, display: impl Into<String>) -> Spec {
        self.display = Some(display.into());
        self
    }

    pub fn search(mut self) -> Spec {
        self.search = true;
        self
    }

    pub fn width(mut self, width: f32) -> Spec {
        self.width = width;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Spec {
        self.enabled = enabled;
        self
    }

    pub fn tip(mut self, tip: &str) -> Spec {
        self.tip = tip.to_string();
        self
    }

    pub fn color(mut self, color: Hsla) -> Spec {
        self.color = Some(color);
        self
    }
}

fn option_row(
    t: &Theme,
    which: Drop,
    o: &Opt,
    on: bool,
    cx: &mut Context<TasksView>,
) -> AnyElement {
    let hover = t.hover;
    let id = option_id(which, &o.value);
    let selector = id.clone();
    let value = o.value.clone();
    row()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .w_full()
        .px(px(8.0))
        .py(px(5.0))
        .gap(px(8.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            view.open = None;
            view.pick(which, &value, window, cx);
            cx.notify();
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.0))
                        .text_color(t.foreground)
                        .child(o.label.clone()),
                )
                .when(!o.detail.is_empty(), |el| {
                    el.child(
                        div()
                            .truncate()
                            .text_size(px(10.5))
                            .text_color(t.dim)
                            .child(o.detail.clone()),
                    )
                }),
        )
        .child(if on {
            icon(Icon::Check)
                .size(px(12.0))
                .text_color(t.green)
                .into_any_element()
        } else {
            div().w(px(12.0)).into_any_element()
        })
        .into_any_element()
}

pub fn dropdown(view: &TasksView, t: &Theme, s: Spec, cx: &mut Context<TasksView>) -> AnyElement {
    let which = s.which;
    let open = view.open == Some(which) && s.enabled;
    let current = s.options.iter().find(|o| o.value == s.value);
    let shown = s
        .display
        .clone()
        .or_else(|| current.map(|o| o.label.clone()))
        .unwrap_or_else(|| {
            if s.value.is_empty() {
                "Pick one".to_string()
            } else {
                s.value.clone()
            }
        });
    let picked = !s.value.is_empty() || current.is_some();
    let hover = t.hover;
    let id = trigger_id(which);
    let selector = id.clone();
    let search = s.search;
    let tip = if s.tip.is_empty() {
        shown.clone()
    } else {
        s.tip.clone()
    };
    let mut trigger = row()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .w_full()
        .h(px(29.0))
        .px(px(8.0))
        .gap(px(6.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(if open { t.blue } else { t.border })
        .bg(if s.enabled { t.background } else { t.surface })
        .tooltip(tooltip(tip))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.0))
                .text_color(if picked {
                    s.color.unwrap_or(t.foreground)
                } else {
                    t.dim
                })
                .child(shown),
        )
        .child(icon(Icon::ChevronDown).size(px(12.0)).text_color(t.dim));
    if s.enabled {
        trigger = trigger
            .cursor_pointer()
            .hover(move |st| st.bg(hover))
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if view.open == Some(which) {
                    view.open = None;
                } else if view
                    .just_closed
                    .is_some_and(|(w, at)| w == which && at.elapsed() < REOPEN_GAP)
                {
                    view.just_closed = None;
                } else {
                    view.open = Some(which);
                    if search {
                        let pick = view.inputs.pick.clone();
                        pick.update(cx, |i, cx| i.set_value("", cx));
                        window.focus(&pick.focus_handle(cx));
                    }
                }
                cx.notify();
            }));
    }
    let mut holder = div().relative().w_full().flex().flex_col().gap(px(3.0));
    if let Some(label) = s.label {
        holder = holder.child(caption(t, label));
    }
    holder = holder.child(trigger);
    if open {
        let typed = if search {
            view.inputs.pick.read(cx).value().trim().to_string()
        } else {
            String::new()
        };
        let found = matches(&s.options, &typed);
        let rows: Vec<AnyElement> = found
            .iter()
            .take(SHOWN_OPTIONS)
            .map(|o| option_row(t, which, o, o.value == s.value, cx))
            .collect();
        let more = found.len().saturating_sub(SHOWN_OPTIONS);
        let empty = found.is_empty();
        let list = div()
            .id(SharedString::from(format!("tasks-dd-list-{}", which.key())))
            .w_full()
            .max_h(px(300.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .children(rows)
            .when(more > 0, |el| {
                el.child(line(
                    format!("{more} more. Type to narrow the list."),
                    t.dim,
                    11.0,
                ))
            })
            .when(empty, |el| el.child(line(s.empty, t.dim, 11.0)));
        let panel = popover(t)
            .w(px(s.width))
            .p(px(6.0))
            .flex()
            .flex_col()
            .gap(px(5.0))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(move |view, _: &MouseDownEvent, _, cx| {
                if view.open == Some(which) {
                    view.open = None;
                    view.just_closed = Some((which, Instant::now()));
                    cx.notify();
                }
            }))
            .when(search, |el| {
                el.child(div().w_full().child(view.inputs.pick.clone()))
            })
            .child(list);
        let top = if s.label.is_some() { 50.0 } else { 32.0 };
        holder = holder.child(
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Local)
                    .position(point(px(0.0), px(top)))
                    .snap_to_window_with_margin(px(8.0))
                    .child(panel),
            )
            .with_priority(1),
        );
    }
    holder.into_any_element()
}
