use super::model::{Action, Side, TO_PROGRAM, Worktree};
use super::view::{CtxEvent, CtxMenu};
use super::{Facts, RIGHT_CLICK_LABEL, SpaceFact, TabFact, Target, TermFact};
use crate::layout::{self, Axis};
use crate::screens::{agents, spaces};
use crate::workspace::Workspace;
use crate::{panes, terms};
use gpui::{Context, Entity, Pixels, Point, Subscription, Window};
use nixa_protocol::TermId;

pub struct Open {
    pub view: Entity<CtxMenu>,
    _events: Subscription,
}

impl Open {
    pub fn view(&self) -> Entity<CtxMenu> {
        self.view.clone()
    }
}

pub fn facts(ws: &Workspace, cx: &gpui::App) -> Facts {
    let away = &ws.hub.agents.read(cx).away;
    let spaces_view = ws.spaces_view.read(cx);
    let spaces = ws
        .spaces
        .spaces
        .iter()
        .map(|space| SpaceFact {
            id: space.id.clone(),
            name: space.name.clone(),
            folder: space.folder.clone(),
            place: spaces::place(spaces_view, space),
            tabs: ws
                .space_tabs(&space.id)
                .map(|tab| {
                    let lead = layout::leaves(&tab.root)
                        .first()
                        .and_then(|id| ws.terms.get(id));
                    TabFact {
                        id: tab.id.clone(),
                        name: ws.tab_name(tab),
                        named: lead.is_some_and(|t| !t.name.trim().is_empty()),
                        program: lead
                            .filter(|t| !terms::is_shell(&t.cmd))
                            .map(|t| terms::program(&t.cmd)),
                        title: lead.and_then(|t| t.title.clone()),
                        settled: away.contains(&tab.id),
                    }
                })
                .collect(),
        })
        .collect();
    let terms = ws
        .terms
        .values()
        .map(|t| TermFact {
            id: t.id.clone(),
            name: terms::term_name(t),
            to_program: to_program(ws, &t.id),
            parent: t.parent.clone(),
        })
        .collect();
    Facts {
        spaces,
        terms,
        themes: nixa_ui::theme::themes(cx)
            .into_iter()
            .map(|(name, _)| name.to_string())
            .collect(),
        theme: Some(nixa_ui::theme(cx).name.to_string()),
    }
}

pub fn to_program(ws: &Workspace, id: &TermId) -> bool {
    ws.terms
        .get(id)
        .and_then(|t| t.labels.get(RIGHT_CLICK_LABEL))
        .is_some_and(|v| v == TO_PROGRAM)
}

pub fn open(
    ws: &mut Workspace,
    target: Target,
    at: Point<Pixels>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if let Target::Pane { terminal, .. } = &target
        && to_program(ws, terminal)
    {
        return;
    }
    let facts = facts(ws, cx);
    let view = super::new(target, facts, at, ws.hub.link.clone(), window, cx);
    let events = cx.subscribe_in(&view, window, |ws, _, event: &CtxEvent, window, cx| {
        on_event(ws, event, window, cx)
    });
    ws.ctx = Some(Open {
        view,
        _events: events,
    });
    cx.notify();
}

pub fn close(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if ws.ctx.take().is_some() {
        window.focus(&ws.focus);
        cx.notify();
    }
}

fn on_event(
    ws: &mut Workspace,
    event: &CtxEvent,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    match event {
        CtxEvent::Close => close(ws, window, cx),
        CtxEvent::Notice { title, body } => {
            crate::screens::events::notify(
                &ws.hub.events,
                crate::screens::events::Notice::new(
                    crate::screens::events::Level::Info,
                    title.clone(),
                )
                .body(body.clone()),
                cx,
            );
        }
        CtxEvent::Saved => {
            crate::screens::custom_agents::refresh(cx);
        }
        CtxEvent::Open { path, folder } => {
            if *folder {
                ws.new_tab_in(Vec::new(), None, Some(path.clone()), cx);
            } else {
                crate::screens::editor::open(path, cx);
            }
        }
        CtxEvent::Run(action) => run(ws, action.clone(), window, cx),
    }
    cx.notify();
}

pub fn run(ws: &mut Workspace, action: Action, window: &mut Window, cx: &mut Context<Workspace>) {
    let spaces_view = ws.spaces_view.clone();
    let agents_view = ws.hub.agents.clone();
    match action {
        Action::Split {
            tab,
            terminal,
            side,
        } => {
            ws.focus_pane(&tab, &terminal, cx);
            let axis = match side {
                Side::Right => Axis::Row,
                Side::Down => Axis::Column,
            };
            ws.split(axis, cx);
        }
        Action::Zoom { tab, terminal } => panes::zoom(ws, &tab, &terminal, cx),
        Action::ClosePane { terminal, .. } => ws.close_term(terminal, cx),
        Action::NewTab { space } => {
            ws.select_space(&space, cx);
            ws.new_tab(Vec::new(), None, cx);
        }
        Action::SettleTab { id } => agents_view.update(cx, |v, cx| agents::put_away(v, &id, cx)),
        Action::UnsettleTab { id } => {
            agents_view.update(cx, |v, cx| agents::bring_back(v, &id, cx))
        }
        Action::CloseTab { id } => ws.close_tab(&id, cx),
        Action::RenameSpace { id } => {
            spaces::run(&spaces_view, spaces::Action::Rename, Some(&id), window, cx)
        }
        Action::Worktree { space, step } => {
            let action = match step {
                Worktree::New => spaces::Action::NewWorktree,
                Worktree::Open => spaces::Action::OpenWorktree,
                Worktree::Remove => spaces::Action::RemoveWorktree,
            };
            spaces::run(&spaces_view, action, Some(&space), window, cx);
        }
        Action::CloseSpace { id } => {
            spaces::run(&spaces_view, spaces::Action::Close, Some(&id), window, cx)
        }
        Action::NewSpace => {
            let active = ws.spaces.active.clone();
            spaces::run(
                &spaces_view,
                spaces::Action::NewSpace,
                active.as_deref(),
                window,
                cx,
            );
        }
        Action::Theme { name } => {
            if let Err(e) = nixa_ui::theme::choose(cx, &name) {
                ws.notice = Some(format!("Cannot save the theme choice: {e}"));
            }
        }
        Action::QuickActions => ws.menu_key("", window, cx),
        Action::Keybinds => crate::keys::help::open(cx),
        Action::RenameTab { id } => agents::rename_tab(ws, &id, cx),
        Action::SendBack { terminal } => agents::send_back(ws, &terminal, cx),
        Action::LinkOpen { .. }
        | Action::LinkReveal { .. }
        | Action::RightClicks { .. }
        | Action::SaveAgent { .. } => {}
    }
    cx.notify();
}

pub fn agents_target(
    ws: &Workspace,
    tab: &Option<String>,
    space: &Option<String>,
    term: &Option<TermId>,
) -> Target {
    let space = space
        .clone()
        .or_else(|| ws.spaces.active.clone())
        .unwrap_or_default();
    match tab {
        Some(id) => Target::Tab {
            space,
            id: Some(id.clone()),
            terminal: term.clone(),
        },
        None if !space.is_empty() => Target::Settled { space },
        None => Target::Default,
    }
}

#[cfg(test)]
#[gpui::test]
fn opening_a_file_link_creates_an_editor_window(cx: &mut gpui::TestAppContext) {
    let name = format!("nixa-context-file-link-{}.txt", std::process::id());
    let file = std::env::temp_dir().join(name);
    std::fs::write(&file, "Sample notes").unwrap();
    let path = file.canonicalize().unwrap().to_string_lossy().into_owned();
    cx.update(nixa_ui::init);
    cx.update(crate::screens::editor::init);
    let (ws, vcx) = cx.add_window_view(Workspace::new);
    vcx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_ctx(
                Target::Link { path: path.clone() },
                gpui::point(gpui::px(200.), gpui::px(150.)),
                window,
                cx,
            );
        });
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let opened = vcx.update(|_, cx| {
        cx.windows()
            .into_iter()
            .filter_map(|window| window.downcast::<nixa_editor::Editor>())
            .filter_map(|window| window.read(cx).ok()?.path().map(str::to_owned))
            .collect::<Vec<_>>()
    });
    std::fs::remove_file(file).unwrap();
    assert_eq!(opened, [path]);
}
