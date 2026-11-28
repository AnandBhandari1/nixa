use super::{Dot, OnStatus, Status, StatusAction, branch_of, engine_dot, render};
use crate::links::procs::Kind;
use crate::workspace::{EngineState, Workspace};
use crate::{hub, spaces};
use gpui::{Context, Div, Stateful};
use nixa_ui::theme;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const FACTS_EVERY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub tasks_waiting: usize,
    pub branch: Option<String>,
    pub folder: String,
    pub at: Option<Instant>,
}

pub fn refresh_facts(ws: &mut Workspace) -> bool {
    let folder = ws
        .active_space()
        .map(|s| s.folder.clone())
        .unwrap_or_default();
    let fresh = ws.facts.at.is_some_and(|at| at.elapsed() < FACTS_EVERY);
    if fresh && folder == ws.facts.folder {
        return false;
    }
    let tasks_waiting = ws
        .store
        .as_ref()
        .map(crate::screens::rail::tasks_waiting)
        .unwrap_or(0);
    let branch = if folder.is_empty() {
        None
    } else {
        branch_of(&folder)
    };
    let changed = tasks_waiting != ws.facts.tasks_waiting
        || branch != ws.facts.branch
        || folder != ws.facts.folder;
    ws.facts = Facts {
        tasks_waiting,
        branch,
        folder,
        at: Some(Instant::now()),
    };
    changed
}

pub fn status(ws: &Workspace, cx: &Context<Workspace>) -> Status {
    let t = theme(cx);
    let (up, starting) = match &ws.engine {
        EngineState::Up => (Some(ws.terms.len()), false),
        EngineState::Starting => (None, true),
        EngineState::Down(_) => (None, false),
    };
    let down = hub::status(t, &ws.hub, Kind::Engine).map(|(c, text, _)| (c, text));
    let engine = engine_dot(t, up, starting, down);
    let brain = hub::status(t, &ws.hub, Kind::Brain).map(|(color, process, detail)| {
        let running = ws.hub.procs.as_ref().is_some_and(|procs| {
            matches!(procs.brain, crate::links::procs::State::Running { .. })
        });
        let text = if running {
            let snap = &ws.hub.chat.read(cx).snap;
            let bar = ws.tab_bar.read(cx);
            let state = match snap.state.as_str() {
                "speaking" => "speaking",
                "paused" => "paused",
                "off" => "off",
                "quiet" => "quiet",
                _ if snap.busy => "thinking",
                _ if snap.hearing => "listening",
                _ => "idle",
            };
            let on = if bar.nixa_on { "on" } else { "off" };
            format!("{}: {on}, {state}, {} waiting", snap.name(), bar.waiting)
        } else {
            process
        };
        Dot { color, detail: Some(format!("{text}. {detail}")), text }
    });
    let folder = ws
        .active_space()
        .map(|s| spaces::short_folder(&s.folder, &spaces::home()))
        .unwrap_or_default();
    Status {
        engine: Some(engine),
        brain,
        notice: ws.notice.clone(),
        folder,
        branch: ws.facts.branch.clone(),
        tasks_waiting: ws.facts.tasks_waiting,
        events_unseen: crate::screens::events::unseen(&ws.hub.events, cx),
    }
}

pub fn act(
    ws: &mut Workspace,
    action: StatusAction,
    window: &mut gpui::Window,
    cx: &mut Context<Workspace>,
) {
    match action {
        StatusAction::Detail(text) => ws.notice = Some(text),
        StatusAction::DismissNotice => ws.notice = None,
        StatusAction::Commands => {
            if ws.palette.is_none() {
                ws.open_palette(window, cx);
            }
        }
        StatusAction::Tasks => {
            crate::screens::rail::host::act(ws, crate::screens::rail::RailAction::Tasks, cx)
        }
        StatusAction::Events => {
            let view = ws.hub.events.events.clone();
            view.update(cx, |v, cx| v.toggle(cx));
            window.focus(&gpui::Focusable::focus_handle(view.read(cx), cx));
        }
    }
    cx.notify();
}

pub fn host(ws: &Workspace, cx: &mut Context<Workspace>) -> Stateful<Div> {
    let s = status(ws, cx);
    let t = theme(cx).clone();
    let this = cx.entity().downgrade();
    let on: OnStatus = Rc::new(move |action, window, app| {
        this.update(app, |ws, cx| act(ws, action, window, cx)).ok();
    });
    render(&s, &t, on)
}
