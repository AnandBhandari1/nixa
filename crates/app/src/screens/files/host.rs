use super::{FilesEvent, FilesView, preview_argv};
use crate::workspace::{PanelKind, Workspace};
use crate::{layout, tabs};
use gpui::{AnyElement, Context, Entity, Subscription};
use nixa_protocol::{Reply, Request, TermId, WaitFor, connect, paths};
use std::collections::HashSet;
use std::time::Duration;

pub const QUIT_WAIT: Duration = Duration::from_millis(1500);
pub const CALL: Duration = Duration::from_secs(5);

pub const RUN_IN: &str = "cd -- \"$1\" && shift && exec \"$@\"";

pub fn new(cx: &mut Context<Workspace>) -> (Entity<FilesView>, Subscription) {
    let view = super::new(cx);
    let sub = cx.subscribe(&view, |ws, _, event: &FilesEvent, cx| {
        on_event(ws, event, cx)
    });
    (view, sub)
}

pub fn sync(ws: &Workspace, cx: &mut Context<Workspace>) {
    let root = ws.active_tab()
        .and_then(|tab| ws.terms.get(&tab.focus))
        .map(|term| term.cwd.clone())
        .filter(|folder| !folder.is_empty() && std::path::Path::new(folder).is_dir())
        .or_else(|| ws.active_space().map(|space| space.folder.clone()).filter(|folder| !folder.is_empty()));
    let tabs: HashSet<String> = ws
        .tabs
        .list
        .iter()
        .map(|t| t.id.clone())
        .chain(ws.tabs.pending.iter().cloned())
        .collect();
    let view = ws.files.clone();
    let same = view.read(cx).root == root;
    if !same {
        super::set_root(&view, root, cx);
    }
    super::set_tabs(&view, tabs, cx);
}

pub fn run_argv(folder: &str, argv: &[String]) -> Vec<String> {
    let mut out = vec![
        "sh".to_string(),
        "-c".to_string(),
        RUN_IN.to_string(),
        "run".to_string(),
        folder.to_string(),
    ];
    out.extend(argv.iter().cloned());
    out
}

fn open_tab(
    ws: &mut Workspace,
    argv: Vec<String>,
    name: String,
    cx: &mut Context<Workspace>,
) -> Option<String> {
    let before = ws.tabs.pending.len();
    ws.new_tab(argv, Some(name), cx);
    (ws.tabs.pending.len() > before)
        .then(|| ws.tabs.pending.last().cloned())
        .flatten()
}

fn single_term(ws: &Workspace, tab: &str) -> Option<TermId> {
    let found = tabs::find(&ws.tabs, tab)?;
    match layout::leaves(&found.root).as_slice() {
        [term] => Some(term.clone()),
        _ => None,
    }
}

fn engine_call(req: &Request) -> Option<Reply> {
    let mut client = connect(&paths::socket_path()).ok()?;
    client.set_timeout(Some(QUIT_WAIT + CALL)).ok()?;
    client.call(req).ok()
}

pub fn ask_quit(term: &TermId, keys: &str) -> bool {
    engine_call(&Request::Input {
        id: term.clone(),
        text: keys.to_string(),
    });
    engine_call(&Request::Wait {
        id: term.clone(),
        until: WaitFor::Exit,
        timeout_ms: Some(QUIT_WAIT.as_millis() as u64),
    });
    match engine_call(&Request::Get { id: term.clone() }) {
        Some(Reply::Term { term }) => !term.running,
        Some(_) => true,
        None => false,
    }
}

pub fn close_preview(ws: &mut Workspace, tab: &str, cx: &mut Context<Workspace>) {
    let Some(term) = single_term(ws, tab) else {
        return;
    };
    let Some(info) = ws.terms.get(&term) else {
        ws.close_tab(tab, cx);
        return;
    };
    if !info.running {
        ws.close_tab(tab, cx);
        return;
    }
    let Some(keys) = super::preview::quit_keys(&info.cmd) else {
        return;
    };
    let name = info.name.clone();
    let task = cx.background_executor().spawn({
        let term = term.clone();
        async move { ask_quit(&term, keys) }
    });
    let tab = tab.to_string();
    cx.spawn(async move |this, cx| {
        let quit = task.await;
        this.update(cx, |ws, cx| {
            if tabs::find(&ws.tabs, &tab).is_none() {
                return;
            }
            if quit {
                if single_term(ws, &tab).as_ref() == Some(&term) {
                    ws.close_tab(&tab, cx);
                }
            } else {
                ws.notice = Some(format!(
                    "{name} did not close. It may have unsaved changes, so its tab stays open."
                ));
                cx.notify();
            }
        })
        .ok();
    })
    .detach();
}

pub fn open_in_terminal(
    ws: &mut Workspace,
    file: &str,
    keep: bool,
    replace: Option<&str>,
    cx: &mut Context<Workspace>,
) {
    let name = super::tree::file_name(file).to_string();
    let Some(tab) = open_tab(ws, preview_argv(file), name.clone(), cx) else {
        report(ws, format!("Could not start a terminal for {name}"), ws.notice.clone(), true, cx);
        return;
    };
    let view = ws.files.clone();
    if let Some(old) = replace {
        super::keep_tab(&view, old, cx);
    }
    super::opened(&view, file, keep, &tab, cx);
    let old = replace.map(str::to_string);
    ws.when_tab_made(&tab, Box::new(move |ws, outcome, cx| match outcome {
        Ok(()) => {
            report(ws, format!("Opened {name} in a terminal"), None, false, cx);
            if let Some(old) = old {
                close_preview(ws, &old, cx);
            }
        }
        Err(reason) => {
            sync(ws, cx);
            report(ws, format!("Could not open {name} in a terminal"), Some(reason), true, cx);
        }
    }));
}

pub fn run_in_terminal(ws: &mut Workspace, name: &str, folder: &str, argv: &[String], cx: &mut Context<Workspace>) {
    let Some(tab) = open_tab(ws, run_argv(folder, argv), name.to_string(), cx) else {
        report(ws, format!("Could not start {name}"), ws.notice.clone(), true, cx);
        return;
    };
    let name = name.to_string();
    ws.when_tab_made(&tab, Box::new(move |ws, outcome, cx| match outcome {
        Ok(()) => report(ws, format!("Started {name}"), None, false, cx),
        Err(reason) => report(ws, format!("Could not start {name}"), Some(reason), true, cx),
    }));
}

fn report(ws: &mut Workspace, title: String, body: Option<String>, error: bool, cx: &mut Context<Workspace>) {
    use crate::screens::events::model::{Level, Notice};
    let mut notice = Notice::new(if error { Level::Error } else { Level::Success }, title);
    notice.body = body.unwrap_or_default();
    ws.hub.events.toasts.update(cx, |toasts, cx| toasts.push(&notice, cx));
    cx.notify();
}

pub fn on_event(ws: &mut Workspace, event: &FilesEvent, cx: &mut Context<Workspace>) {
    match event {
        FilesEvent::Open { file, .. } => crate::screens::editor::open(file, cx),
        FilesEvent::Terminal {
            file,
            keep,
            replace,
        } => open_in_terminal(ws, file, *keep, replace.as_deref(), cx),
        FilesEvent::Select(tab) => ws.select_tab(tab, cx),
        FilesEvent::Run { name, folder, argv } => run_in_terminal(ws, name, folder, argv, cx),
        FilesEvent::Edit { file } => {
            crate::screens::editor::open(file, cx);
            report(ws, format!("Opening {} in the editor", super::tree::file_name(file)), None, false, cx);
        }
        FilesEvent::Notify {
            title,
            body,
            error,
        } => report(ws, title.clone(), body.clone(), *error, cx),
        FilesEvent::Close => {
            if ws.panel == Some(PanelKind::Files) {
                ws.set_panel(None, cx);
            }
        }
    }
}

pub fn render(ws: &Workspace) -> AnyElement {
    super::render(&ws.files)
}
