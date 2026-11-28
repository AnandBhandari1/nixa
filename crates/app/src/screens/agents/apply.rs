use super::input::PIN_LABEL;
use super::{AgentsEvent, AgentsView, focused, model, rename};
use crate::layout;
use crate::links::engine;
use crate::screens::files::preview;
use crate::tabs::{self, LABEL_SPACE, LABEL_TAB};
use crate::workspace::{EngineState, Workspace};
use gpui::{Context, Entity};
use nixa_protocol::{Request, TermId, connect_default};
use std::time::Duration;

const PIN_TIMEOUT: Duration = Duration::from_secs(10);

pub fn set_pin(id: TermId, on: bool) -> anyhow::Result<()> {
    let mut client = connect_default()?;
    client.set_timeout(Some(PIN_TIMEOUT))?;
    client.call(&Request::LabelSet {
        id,
        key: PIN_LABEL.to_string(),
        value: on.then(|| "1".to_string()),
    })?;
    Ok(())
}

pub fn on_event(ws: &mut Workspace, event: &AgentsEvent, cx: &mut Context<Workspace>) {
    match event {
        AgentsEvent::Focus { tab, term } => {
            ws.select_tab(tab, cx);
            if let Some(term) = term {
                ws.focus_pane(tab, term, cx);
            }
        }
        AgentsEvent::CloseTab { tab } => ws.close_tab(tab, cx),
        AgentsEvent::CloseTerm { term } => ws.close_term(term.clone(), cx),
        AgentsEvent::Pin { term, on } => {
            let (id, on) = (term.clone(), *on);
            let task = cx
                .background_executor()
                .spawn(async move { set_pin(id, on) });
            cx.spawn(async move |this, cx| {
                if let Err(e) = task.await {
                    this.update(cx, |ws, cx| {
                        ws.notice = Some(format!("Cannot pin the agent: {e:#}"));
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }
        AgentsEvent::Adopt { term } => ws.adopt(term.clone(), cx),
        AgentsEvent::PutAway { tab, next } => {
            let space = crate::tabs::find(&ws.tabs, tab).map(|t| t.space.clone());
            if let Some(space) = space
                && ws.tabs.active.get(&space) == Some(tab)
            {
                match next {
                    Some(next) => {
                        ws.tabs.active.insert(space.clone(), next.clone());
                    }
                    None => {
                        ws.tabs.active.remove(&space);
                    }
                }
                if ws.spaces.active.as_deref() == Some(space.as_str()) {
                    ws.refocus = true;
                }
            }
        }
        AgentsEvent::ToggleSidebar => ws.toggle_sidebar(cx),
        AgentsEvent::Notice(text) => ws.notice = Some(text.clone()),
        AgentsEvent::Menu {
            tab,
            space,
            term,
            at,
        } => {
            ws.ctx_wanted = Some((
                crate::screens::ctxmenu::host::agents_target(ws, tab, space, term),
                *at,
            ));
        }
        AgentsEvent::Rename { tab } => rename_tab(ws, tab, cx),
        AgentsEvent::SetName { term, name } => set_name(ws, term, name, cx),
        AgentsEvent::KeepPreview { tab } => {
            ws.files.update(cx, |v, _| {
                if v.previews.preview.as_ref().is_some_and(|p| &p.tab == tab) {
                    preview::keep_preview(&mut v.previews);
                }
            });
        }
        AgentsEvent::PopOut { term } => pop_out(ws, term, cx),
    }
    cx.notify();
}

pub fn rename_tab(ws: &mut Workspace, tab: &str, cx: &mut Context<Workspace>) {
    let Some(found) = tabs::find(&ws.tabs, tab) else {
        return;
    };
    let Some(term) = layout::leaves(&found.root).first().cloned() else {
        return;
    };
    let name = ws
        .terms
        .get(&term)
        .map(|t| t.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| ws.tab_name(found));
    let view = ws.hub.agents.clone();
    view.update(cx, |v, cx| rename::open(v, tab, term, &name, cx));
}

fn set_name(ws: &mut Workspace, term: &TermId, name: &str, cx: &mut Context<Workspace>) {
    let (id, name) = (term.clone(), name.to_string());
    let task = cx
        .background_executor()
        .spawn(async move { engine::rename(id, &name) });
    cx.spawn(async move |this, cx| {
        if let Err(e) = task.await {
            this.update(cx, |ws, cx| {
                ws.notice = Some(format!("Cannot rename the tab: {e:#}"));
                cx.notify();
            })
            .ok();
        }
    })
    .detach();
    ws.notice = None;
}

fn save_layouts(ws: &mut Workspace) {
    if ws.engine != EngineState::Up {
        return;
    }
    if let Err(e) = tabs::persist(&mut ws.tabs, &ws.tabs_path) {
        ws.notice = Some(format!("Cannot save the pane layout: {e}"));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LabelJob {
    Place {
        term: TermId,
        space: String,
        tab: String,
    },
    Unplace {
        term: TermId,
    },
}

fn queue_labels(ws: &mut Workspace, job: LabelJob, cx: &mut Context<Workspace>) {
    let view = ws.hub.agents.clone();
    let idle = view.update(cx, |v, _| {
        v.label_jobs.push_back(job);
        !v.label_busy
    });
    if idle {
        next_labels(ws, cx);
    }
}

fn write_labels(job: &LabelJob) -> anyhow::Result<()> {
    match job {
        LabelJob::Place { term, space, tab } => {
            engine::label(term.clone(), LABEL_SPACE, space)?;
            engine::label(term.clone(), LABEL_TAB, tab)
        }
        LabelJob::Unplace { term } => {
            engine::unlabel(term.clone(), LABEL_TAB)?;
            engine::unlabel(term.clone(), LABEL_SPACE)
        }
    }
}

pub fn labels_done(ws: &mut Workspace, job: LabelJob, done: anyhow::Result<()>, cx: &mut Context<Workspace>) {
    match (job, done) {
        (LabelJob::Unplace { term }, Ok(())) => {
            ws.tabs.hold.insert(term.clone());
            drop_child(ws, &term, cx);
        }
        (LabelJob::Place { .. }, Ok(())) => {}
        (LabelJob::Place { .. }, Err(e)) => {
            ws.notice = Some(format!(
                "The sub-agent is shown, but its pane will not come back after a restart: {e:#}"
            ));
        }
        (LabelJob::Unplace { .. }, Err(e)) => {
            ws.notice = Some(format!("Cannot send the pane back to its parent: {e:#}"));
        }
    }
    cx.notify();
}

fn next_labels(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    let view = ws.hub.agents.clone();
    let job = view.update(cx, |v, _| {
        let job = v.label_jobs.pop_front();
        v.label_busy = job.is_some();
        job
    });
    let Some(job) = job else {
        return;
    };
    let work = job.clone();
    let task = cx
        .background_executor()
        .spawn(async move { write_labels(&work) });
    cx.spawn(async move |this, cx| {
        let done = task.await;
        this.update(cx, |ws, cx| {
            labels_done(ws, job, done, cx);
            next_labels(ws, cx);
        })
        .ok();
    })
    .detach();
}

pub fn pop_out(ws: &mut Workspace, term: &TermId, cx: &mut Context<Workspace>) {
    let Some(parent) = ws.terms.get(term).and_then(|t| t.parent.clone()) else {
        return;
    };
    ws.tabs.hold.remove(term);
    let Some((tab, space)) = tabs::place_beside(&mut ws.tabs, &parent, term) else {
        ws.notice = Some("The parent of this sub-agent has no pane in this window.".into());
        cx.notify();
        return;
    };
    save_layouts(ws);
    if ws.spaces.active.as_deref() != Some(space.as_str()) {
        ws.select_space(&space, cx);
    }
    ws.select_tab(&tab, cx);
    ws.focus_pane(&tab, term, cx);
    let job = LabelJob::Place {
        term: term.clone(),
        space,
        tab,
    };
    queue_labels(ws, job, cx);
    cx.notify();
}

pub fn drop_child(ws: &mut Workspace, term: &TermId, cx: &mut Context<Workspace>) {
    let Some(parent) = ws.terms.get(term).and_then(|t| t.parent.clone()) else {
        return;
    };
    let tab = tabs::tab_of(&ws.tabs, term).map(|t| t.id.clone());
    tabs::drop_leaf(&mut ws.tabs, term);
    save_layouts(ws);
    if let Some(tab) = tab.filter(|t| tabs::find(&ws.tabs, t).is_some_and(|t| layout::contains(&t.root, &parent))) {
        ws.focus_pane(&tab, &parent, cx);
    }
    ws.refocus = true;
    cx.notify();
}

pub fn send_back(ws: &mut Workspace, term: &TermId, cx: &mut Context<Workspace>) {
    if ws.terms.get(term).and_then(|t| t.parent.as_ref()).is_none() {
        return;
    }
    queue_labels(ws, LabelJob::Unplace { term: term.clone() }, cx);
}

pub fn step_agent(
    ws: &mut Workspace,
    view: &Entity<AgentsView>,
    step: isize,
    cx: &mut Context<Workspace>,
) {
    let target = {
        let v = view.read(cx);
        let now = focused(v).map(|a| a.term.clone());
        model::step(&v.view.order, now.as_ref(), step)
    };
    if let Some((tab, term)) = target {
        on_event(
            ws,
            &AgentsEvent::Focus {
                tab,
                term: Some(term),
            },
            cx,
        );
    }
}
