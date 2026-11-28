use super::items::{Action, Tab};
use super::runner::{self, Finish, Run};
use super::switches::{self, SwitchEvent, SwitchesView};
use super::{Command, MenuEvent, MenuView};
use crate::layout::Axis;
use crate::links::brain::{Link, NOT_CONNECTED};
use crate::screens::question::QuestionCommand;
use crate::workspace::{EngineState, PanelKind, Workspace};
use anyhow::Result;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Focusable, Subscription, Window};
use nixa_protocol::paths;
use nixa_ui::metrics::{PANEL_WIDTH, SIDEBAR_WIDTH};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const TASKS: &str = r#"if [ -z "$NIXA_BIN" ]; then echo 'NIXA_BIN is not set in this terminal.'; exit 1; fi
"$NIXA_BIN" task ls"#;

pub const CAPTURE: &str = r#"if [ -z "$NIXA_BIN" ]; then echo 'NIXA_BIN is not set in this terminal.'; exit 1; fi
printf 'New task: '
IFS= read -r line || exit 1
if [ -z "$line" ]; then echo 'Nothing captured.'; exit 0; fi
"$NIXA_BIN" task add "$line""#;

pub const BOARD: &str = r#"while :; do clear; "$NIXA_BIN" task ls; sleep 5; done"#;

pub const COMMAND_MENU: &str = r#"if command -v cmdmenu >/dev/null 2>&1; then
  cmdmenu --root "$PWD"
elif [ -x "$HOME/.local/bin/cmdmenu" ]; then
  "$HOME/.local/bin/cmdmenu" --root "$PWD"
else
  printf '%s\n' 'cmdmenu is not installed on this machine. Install cmdmenu and put it in PATH or ~/.local/bin/cmdmenu.'
  exit 1
fi"#;

pub struct Host {
    pub view: Entity<MenuView>,
    _sub: Subscription,
}

pub fn open(tab: Tab, window: &mut Window, cx: &mut Context<Workspace>) -> Host {
    let view = super::new(tab, cx);
    window.focus(&view.focus_handle(cx));
    let sub = cx.subscribe_in(&view, window, |ws, view, event: &MenuEvent, window, cx| {
        on_event(ws, view, event, window, cx)
    });
    Host { view, _sub: sub }
}

pub fn overlay(host: Option<&Host>, cx: &App) -> Option<AnyElement> {
    host.filter(|h| h.view.read(cx).open)
        .map(|h| h.view.clone().into_any_element())
}

fn close(view: &Entity<MenuView>, ws: &mut Workspace, cx: &mut Context<Workspace>) {
    view.update(cx, |v, cx| {
        v.open = false;
        cx.notify();
    });
    ws.refocus = true;
    cx.notify();
}

fn on_event(
    ws: &mut Workspace,
    view: &Entity<MenuView>,
    event: &MenuEvent,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    close(view, ws, cx);
    match event {
        MenuEvent::Close => {}
        MenuEvent::OpenSettings(part) => ws.open_settings(part, cx),
        MenuEvent::Run(action) => run_action(ws, action.clone(), window, cx),
    }
}

pub fn run_command(
    ws: &mut Workspace,
    command: Command,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Option<Host> {
    match command {
        Command::Open(tab) => Some(open(tab, window, cx)),
        Command::Run(action) => {
            run_action(ws, action, window, cx);
            None
        }
    }
}

pub fn workdir(ws: &Workspace) -> String {
    ws.active_tab()
        .and_then(|tab| ws.terms.get(&tab.focus))
        .map(|term| term.cwd.clone())
        .filter(|cwd| !cwd.is_empty())
        .or_else(|| ws.active_space().map(|s| s.folder.clone()))
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

pub fn run_action(
    ws: &mut Workspace,
    action: Action,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let here = Some(workdir(ws));
    match action {
        Action::Tasks => start_runner(ws, runner::script("tasks", TASKS, here), cx),
        Action::QuickCapture => {
            start_runner(ws, runner::script("quick capture", CAPTURE, here), cx)
        }
        Action::ProjectCommands => {
            start_runner(ws, runner::script("commands", COMMAND_MENU, here), cx)
        }
        Action::Board => ws.new_tab(
            vec!["sh".into(), "-c".into(), BOARD.into()],
            Some("Board".into()),
            cx,
        ),
        Action::Files => ws.set_panel(Some(PanelKind::Files), cx),
        Action::Git => ws.set_panel(Some(PanelKind::Git), cx),
        Action::Commands => ws.open_palette(window, cx),
        Action::AgentSettings => ws.open_settings("Agents and team", cx),
        Action::GitAccounts => ws.open_settings("Git accounts", cx),
        Action::Events => {
            let view = ws.hub.events.events.clone();
            view.update(cx, |v, cx| {
                v.run(crate::screens::events::EventsCommand::Show, cx)
            });
            window.focus(&view.focus_handle(cx));
        }
        Action::Favourites => ws.open_favourites(window, cx),
        Action::NewAgent => {
            crate::screens::tabbar::run(ws, crate::screens::tabbar::Action::NewMenu, window, cx)
        }
        Action::CustomAgents => crate::screens::custom_agents::open(cx),
        Action::TasksWindow => crate::screens::tasks::open(cx),
        Action::GitClone => crate::screens::git::accounts::clone_win::open(None, None, cx),
        Action::NewWorktree | Action::OpenWorktree => {
            let step = if action == Action::NewWorktree {
                crate::screens::spaces::Action::NewWorktree
            } else {
                crate::screens::spaces::Action::OpenWorktree
            };
            let view = ws.spaces_view.clone();
            let space = ws.spaces.active.clone();
            crate::screens::spaces::run(&view, step, space.as_deref(), window, cx);
        }
        Action::RenameTab => {
            if let Some(id) = ws.active_tab().map(|t| t.id.clone()) {
                crate::screens::agents::rename_tab(ws, &id, cx);
            }
        }
        Action::Projects | Action::Back => {}
        Action::OpenProject { name, path } => open_project(ws, &name, &path, cx),
        Action::NewTab => ws.new_tab(Vec::new(), None, cx),
        Action::NewSpace | Action::NewSpaceInFolder => ws.pick_space_folder(cx),
        Action::SplitRight => ws.split(Axis::Row, cx),
        Action::SplitDown => ws.split(Axis::Column, cx),
        Action::ZoomPane => ws.zoom_focused(cx),
        Action::MyLayout => {
            ws.sidebar_w = SIDEBAR_WIDTH;
            ws.sidebar_open_w = SIDEBAR_WIDTH;
            ws.panel_w = PANEL_WIDTH;
            cx.notify();
        }
        Action::Layouts => crate::named_layouts::open(cx),
        Action::ToggleSidebar => ws.toggle_sidebar(cx),
        Action::RenameSpace => {
            if let Some(id) = ws.spaces.active.clone() {
                let view = ws.spaces_view.clone();
                crate::screens::spaces::run(
                    &view,
                    crate::screens::spaces::Action::Rename,
                    Some(&id),
                    window,
                    cx,
                );
            }
        }
        Action::CloseTab => {
            if let Some(id) = ws.active_tab().map(|t| t.id.clone()) {
                ws.close_tab(&id, cx);
            }
        }
        Action::CloseSpace => close_space(ws, cx),
    }
}

fn open_project(ws: &mut Workspace, name: &str, path: &str, cx: &mut Context<Workspace>) {
    let path = path.trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    if let Some(id) = ws
        .spaces
        .spaces
        .iter()
        .find(|s| s.folder.trim_end_matches('/') == path)
        .map(|s| s.id.clone())
    {
        ws.select_space(&id, cx);
        return;
    }
    ws.add_space(path.to_string(), cx);
    let named = ws
        .spaces
        .active
        .clone()
        .filter(|_| !name.is_empty())
        .filter(|id| {
            ws.active_space()
                .is_some_and(|s| s.id == *id && s.name != name)
        });
    if let Some(id) = named {
        ws.rename_space(&id, name, cx);
    }
}

fn close_space(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    if let Some(id) = ws.spaces.active.clone() {
        crate::screens::spaces::handle(ws, &crate::screens::spaces::SpacesEvent::Close(id), cx);
    }
}

pub fn start_runner(ws: &mut Workspace, run: Run, cx: &mut Context<Workspace>) {
    if ws.engine != EngineState::Up {
        ws.notice =
            Some("nixa-engine is not running yet. Nixa starts it, see the status bar.".into());
        cx.notify();
        return;
    }
    let socket = paths::socket_path();
    let title = run.title.clone();
    let spec = runner::spec(&run);
    let made = cx.background_executor().spawn({
        let socket = socket.clone();
        async move { runner::create(&socket, spec) }
    });
    cx.spawn(async move |this, cx| {
        let term = match made.await {
            Ok(term) => term,
            Err(e) => {
                this.update(cx, |ws, cx| {
                    ws.notice = Some(format!("{title}: cannot start the terminal: {e:#}"));
                    cx.notify();
                })
                .ok();
                return;
            }
        };
        let closed = Arc::new(AtomicBool::new(false));
        let opened = cx.update(|cx| {
            runner::open_window(
                title.clone(),
                term.id.clone(),
                socket.clone(),
                closed.clone(),
                cx,
            )
        });
        if let Err(e) = opened.and_then(|r| r) {
            let gone = term.id.clone();
            let path = socket.clone();
            std::thread::spawn(move || runner::remove(&path, &gone));
            this.update(cx, |ws, cx| {
                ws.notice = Some(format!("{title}: cannot open the popup window: {e:#}"));
                cx.notify();
            })
            .ok();
            return;
        }
        let id = term.id.clone();
        let name = title.clone();
        let result = cx
            .background_executor()
            .spawn(async move { runner::collect(&socket, &id, &name) })
            .await;
        this.update(cx, |ws, cx| finished(ws, &title, result, &closed, cx))
            .ok();
    })
    .detach();
}

fn finished(
    ws: &mut Workspace,
    title: &str,
    result: Result<Finish>,
    closed: &AtomicBool,
    cx: &mut Context<Workspace>,
) {
    if closed.load(Ordering::SeqCst) {
        return;
    }
    match result {
        Ok(finish) => {
            ws.notice = Some(runner::notice(&finish));
            if let Some(words) = runner::spoken(&finish) {
                say(ws, words, cx);
            }
        }
        Err(e) => ws.notice = Some(format!("{title}: {e:#}")),
    }
    cx.notify();
}

pub fn say_message(words: &str) -> Value {
    json!({"text": words, "agent": "nixa", "kind": "error"})
}

pub fn say(ws: &mut Workspace, words: String, cx: &mut Context<Workspace>) {
    let Some(link) = ws.hub.link.clone() else {
        if let Some(n) = ws.notice.as_mut() {
            n.push_str(&format!(" (not spoken: {NOT_CONNECTED})"));
        }
        return;
    };
    let task = cx
        .background_executor()
        .spawn(async move { link.request("phone_say", say_message(&words)) });
    cx.spawn(async move |this, cx| {
        if let Err(e) = task.await {
            this.update(cx, |ws, cx| {
                if let Some(n) = ws.notice.as_mut() {
                    n.push_str(&format!(" (not spoken: {e:#})"));
                }
                cx.notify();
            })
            .ok();
        }
    })
    .detach();
}

pub fn switches_for(cx: &mut Context<Workspace>) -> Entity<SwitchesView> {
    let store = nixa_store::open_default().ok();
    let view = switches::new(paths::data_dir(), store, cx);
    cx.subscribe(&view, |ws, view, event: &SwitchEvent, cx| {
        on_switch(ws, &view, event, cx)
    })
    .detach();
    cx.set_global(switches::Shared(view.clone()));
    crate::screens::settings::parts::general::install(view.clone(), cx);
    view
}

pub fn on_switch(
    ws: &mut Workspace,
    view: &Entity<SwitchesView>,
    event: &SwitchEvent,
    cx: &mut Context<Workspace>,
) {
    match event {
        SwitchEvent::Send(value) => {
            send_switch(ws.hub.link.clone(), view.clone(), value.clone(), cx)
        }
        SwitchEvent::NixaScreen => crate::screens::nixa_screen::host::open(ws, cx),
        SwitchEvent::ShowCard => {
            ws.set_panel(Some(PanelKind::Nixa), cx);
            ws.hub
                .question
                .update(cx, |q, cx| q.run(QuestionCommand::OpenCard, cx));
        }
    }
}

pub fn send_switch(link: Option<Link>, view: Entity<SwitchesView>, value: Value, cx: &mut App) {
    let Some((kind, fields)) = crate::hub::split_kind(&value) else {
        return;
    };
    let Some(link) = link else {
        switches::set_error(&view, Some(NOT_CONNECTED.to_string()), cx);
        return;
    };
    let task = cx
        .background_executor()
        .spawn(async move { link.request(&kind, fields) });
    cx.spawn(async move |cx| {
        let error = task.await.err().map(|e| format!("{e:#}"));
        cx.update(|cx| switches::set_error(&view, error, cx)).ok();
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::COMMAND_MENU;

    #[test]
    fn command_menu_hint_is_generic() {
        assert!(COMMAND_MENU.contains("cmdmenu --root"));
        assert!(COMMAND_MENU.contains("Install cmdmenu and put it in PATH"));
    }
}
