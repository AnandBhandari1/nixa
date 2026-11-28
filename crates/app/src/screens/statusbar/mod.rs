pub mod host;
#[cfg(test)]
mod tests;

use gpui::prelude::*;
use gpui::{App, Div, Hsla, SharedString, Stateful, Window, div, px};
use nixa_ui::metrics::STATUS_BAR_HEIGHT;
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon};
use std::fs;
use std::path::Path;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct Dot {
    pub color: Hsla,
    pub text: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub engine: Option<Dot>,
    pub brain: Option<Dot>,
    pub notice: Option<String>,
    pub folder: String,
    pub branch: Option<String>,
    pub tasks_waiting: usize,
    pub events_unseen: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusAction {
    Detail(String),
    DismissNotice,
    Commands,
    Tasks,
    Events,
}

pub type OnStatus = Rc<dyn Fn(StatusAction, &mut Window, &mut App)>;

pub fn events_tip(unseen: usize) -> String {
    format!("Event history: {unseen} new")
}

pub fn engine_dot(
    t: &Theme,
    up: Option<usize>,
    starting: bool,
    down: Option<(Hsla, String)>,
) -> Dot {
    let (color, text) = match (up, starting) {
        (Some(n), _) => (
            t.green,
            format!("Engine: on, {n} terminal{}", if n == 1 { "" } else { "s" }),
        ),
        (None, true) => (t.dim, "Engine: connecting".to_string()),
        (None, false) => down.unwrap_or((t.red, "Engine: not running".to_string())),
    };
    Dot {
        color,
        text,
        detail: None,
    }
}

pub fn branch_of(folder: &str) -> Option<String> {
    let mut dir = Path::new(folder);
    loop {
        let dot = dir.join(".git");
        if dot.is_dir() {
            return head_branch(&dot);
        }
        if dot.is_file() {
            let text = fs::read_to_string(&dot).ok()?;
            let path = text.trim().strip_prefix("gitdir:")?.trim();
            let git = if Path::new(path).is_absolute() {
                Path::new(path).to_path_buf()
            } else {
                dir.join(path)
            };
            return head_branch(&git);
        }
        dir = dir.parent()?;
    }
}

fn head_branch(git: &Path) -> Option<String> {
    let head = fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(r) => Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string()),
        None => {
            let short: String = head.chars().take(7).collect();
            (short.len() == 7 && short.chars().all(|c| c.is_ascii_hexdigit())).then_some(short)
        }
    }
}

fn dot(d: &Dot) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(div().size(px(7.)).rounded_full().bg(d.color))
        .child(SharedString::from(d.text.clone()))
}

fn clickable(el: Stateful<Div>, on: &OnStatus, action: StatusAction) -> Stateful<Div> {
    let on = on.clone();
    el.cursor_pointer()
        .on_click(move |_, window, cx| on(action.clone(), window, cx))
}

pub fn render(s: &Status, t: &Theme, on: OnStatus) -> Stateful<Div> {
    let hover = t.hover;
    div()
        .id("workspace-status")
        .h(px(STATUS_BAR_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(12.))
        .bg(t.chrome)
        .border_t_1()
        .border_color(t.border)
        .text_size(px(11.5))
        .text_color(t.dim)
        .when_some(s.engine.clone(), |el, d| {
            let tip = d.detail.clone().unwrap_or_else(|| d.text.clone());
            let item = dot(&d)
                .id("status-engine")
                .debug_selector(|| "status-engine".into())
                .tooltip(tooltip(tip));
            el.child(match d.detail.clone() {
                Some(detail) => clickable(item, &on, StatusAction::Detail(detail)),
                None => item,
            })
        })
        .when_some(s.brain.clone(), |el, d| {
            let tip = d.detail.clone().unwrap_or_else(|| d.text.clone());
            let item = dot(&d)
                .id("status-brain")
                .debug_selector(|| "status-brain".into())
                .tooltip(tooltip(tip));
            el.child(match d.detail.clone() {
                Some(detail) => clickable(item, &on, StatusAction::Detail(detail)),
                None => item,
            })
        })
        .when(s.tasks_waiting > 0, |el| {
            let n = s.tasks_waiting;
            let item = div()
                .id("status-tasks")
                .debug_selector(|| "status-tasks".into())
                .flex()
                .items_center()
                .gap(px(5.))
                .text_color(t.yellow)
                .tooltip(tooltip(
                    "Tasks waiting for you. Click to open the task list.",
                ))
                .child(icon(Icon::Tasks).size(px(12.)).text_color(t.yellow))
                .child(SharedString::from(format!("{n} waiting")));
            el.child(clickable(item, &on, StatusAction::Tasks))
        })
        .when_some(s.notice.clone(), |el, notice| {
            let item = div()
                .id("status-notice")
                .debug_selector(|| "status-notice".into())
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .text_color(t.yellow)
                .tooltip(tooltip("Click to hide this note"))
                .child(div().truncate().child(SharedString::from(notice)))
                .child(div().flex_none().text_color(t.dim).child("×"));
            el.child(clickable(item, &on, StatusAction::DismissNotice))
        })
        .child(div().flex_1())
        .child({
            let n = s.events_unseen;
            let item = div()
                .id("status-events")
                .debug_selector(|| "status-events".into())
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .text_color(if n > 0 { t.foreground } else { t.dim })
                .tooltip(tooltip(events_tip(n)))
                .child(icon(Icon::Bell).size(px(12.)))
                .child(SharedString::from(if n > 0 {
                    format!("Events {n}")
                } else {
                    "Events".to_string()
                }));
            clickable(item, &on, StatusAction::Events)
        })
        .when_some(s.branch.clone(), |el, branch| {
            el.child(
                div()
                    .id("status-branch")
                    .debug_selector(|| "status-branch".into())
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .child(icon(Icon::GitBranch).size(px(11.)).text_color(t.dim))
                    .child(SharedString::from(branch)),
            )
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(SharedString::from(s.folder.clone())),
        )
        .child(clickable(
            div()
                .id("status-commands")
                .debug_selector(|| "status-commands".into())
                .flex_none()
                .px(px(4.))
                .rounded(px(4.))
                .hover(move |st| st.bg(hover))
                .tooltip(tooltip("Open the command list"))
                .child("⌘K Commands"),
            &on,
            StatusAction::Commands,
        ))
}
