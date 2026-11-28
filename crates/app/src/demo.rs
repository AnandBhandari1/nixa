use crate::layout;
use crate::screens::nixa_screen::map::{Agent, St};
use crate::screens::nixa_screen::office::{Room, Rooms};
use crate::screens::nixa_screen::prefs::Look;
use crate::screens::ctxmenu::Target;
use crate::screens::ctxmenu::model::Action;
use crate::workspace::{PanelKind, Workspace};
use gpui::{Context, Window};
use nixa_protocol::TermId;
use serde_json::json;
use std::time::{Duration, Instant};

pub const FLOW_EVERY: Duration = Duration::from_millis(900);
pub const FLOW_ROUNDS: usize = 60;

fn first_tab(ws: &Workspace) -> Option<(String, String, TermId)> {
    let space = ws.spaces.active.clone()?;
    let tab = ws.space_tabs(&space).next()?;
    let term = layout::leaves(&tab.root).first().cloned()?;
    let id = tab.id.clone();
    Some((space, id, term))
}

pub fn show(ws: &mut Workspace, what: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Option<bool> {
    match what {
        "save-agent" => Some(save_agent(ws, window, cx)),
        "rename" => Some(rename(ws, cx)),
        "usage" => {
            usage(ws, cx);
            Some(true)
        }
        "chat-draft" => {
            chat_draft(ws, window, cx);
            Some(true)
        }
        "nixa-screen-office" => {
            crate::screens::nixa_screen::host::open(ws, cx);
            office(cx);
            Some(true)
        }
        "nixa-screen-flow" => {
            crate::screens::nixa_screen::host::open(ws, cx);
            flows(cx);
            Some(true)
        }
        _ => None,
    }
}

pub const CHAT_DRAFT: &str = "Please check the open tasks in this space and tell me which ones are blocked on another task and which ones can start now.\nThen start every open task that can run.";

fn chat_draft(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    ws.set_panel(Some(PanelKind::Nixa), cx);
    let input = ws.hub.chat.read(cx).input.clone();
    input.update(cx, |area, cx| area.set_value(CHAT_DRAFT, cx));
    window.focus(&gpui::Focusable::focus_handle(input.read(cx), cx));
}

fn save_agent(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    let Some((space, tab, term)) = first_tab(ws) else {
        return false;
    };
    let size = window.viewport_size();
    let at = gpui::point(size.width / 3., size.height / 4.);
    let target = Target::Tab {
        space,
        id: Some(tab),
        terminal: Some(term),
    };
    ws.open_ctx(target, at, window, cx);
    let Some(view) = ws.ctx.as_ref().map(|c| c.view()) else {
        return true;
    };
    view.update(cx, |menu, cx| {
        let found = menu
            .entries()
            .into_iter()
            .position(|e| matches!(e.action, Action::SaveAgent { .. }));
        if let Some(index) = found {
            menu.choose(index, cx);
        }
    });
    true
}

fn rename(ws: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    let Some((_, tab, _)) = first_tab(ws) else {
        return false;
    };
    crate::screens::agents::rename_tab(ws, &tab, cx);
    true
}

fn usage(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    ws.set_panel(Some(PanelKind::Nixa), cx);
    let event = json!({"type": "usage", "model": "test-model", "input": 1800, "output": 420, "context_pct": 37.5});
    crate::hub::on_brain_event(ws, &event, cx);
}

fn flows(cx: &mut Context<Workspace>) {
    cx.spawn(async move |_, cx| {
        for round in 0..FLOW_ROUNDS {
            cx.background_executor().timer(FLOW_EVERY).await;
            let wall = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or_default();
            let done = cx.update(|cx| {
                if let Some(screen) = crate::screens::nixa_screen::host::screen(cx) {
                    screen.update(cx, |s, cx| {
                        let other = s.shown.first().map(|a| a.id.clone()).unwrap_or_else(|| "deep".into());
                        let (from, to) = if round % 2 == 0 {
                            ("core".to_string(), other)
                        } else {
                            (other, "core".to_string())
                        };
                        let event = json!({"type": "flow", "from": from, "to": to, "kind": "ask"});
                        if s.on_message(&event, Instant::now(), wall) {
                            s.live.dirty = true;
                        }
                        cx.notify();
                    });
                }
            });
            if done.is_err() {
                break;
            }
        }
    })
    .detach();
}

const DEMO_NAMES: [&str; 6] = ["Maya", "Arlo", "Juno", "Ravi", "Nova", "Otis"];

fn demo_agent(id: &str, program: &str, n: u32, status: St, title: &str) -> Agent {
    Agent {
        id: id.into(),
        title: title.into(),
        program: program.into(),
        mono: program.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(),
        status,
        parent: None,
        seq: n as u64,
        number: Some(n),
        persona: DEMO_NAMES.get(n as usize % DEMO_NAMES.len()).map(|s| s.to_string()),
    }
}

pub fn demo_rooms() -> Rooms {
    Rooms {
        list: vec![
            Room {
                id: "demo-app".into(),
                name: "Nixa app".into(),
                color: 0,
                agents: vec![
                    demo_agent("d-1", "claude", 1, St::Working, "login page #12"),
                    demo_agent("d-2", "codex", 2, St::Working, "api tests"),
                    demo_agent("d-3", "omp", 3, St::Blocked, "db migration"),
                    demo_agent("d-4", "claude", 4, St::Done, "docs"),
                    demo_agent("d-5", "agy", 5, St::Idle, "ui polish"),
                ],
                deep: Vec::new(),
            },
            Room {
                id: "demo-web".into(),
                name: "Website".into(),
                color: 2,
                agents: vec![
                    demo_agent("d-6", "claude", 6, St::Working, "pricing page"),
                    demo_agent("d-7", "codex", 7, St::Idle, "seo"),
                ],
                deep: Vec::new(),
            },
        ],
        loose_deep: vec![demo_agent("d-9", "claude", 9, St::Idle, "Deep brain claude")],
        active: Some("demo-app".into()),
    }
}

const DEMO_TALKS: [(&str, &str, &str); 5] = [
    ("nixa", "d-1", "Please finish the login page and run its tests."),
    ("d-3", "nixa", "The migration fails on the old users table. Can you help?"),
    ("nixa", "deep", "Should we split the migration into two steps?"),
    ("deep", "nixa", "I pick two steps. It is safer and easy to undo."),
    ("d-6", "nixa", "The pricing page is done. Please check it."),
];

fn office(cx: &mut Context<Workspace>) {
    if let Some(screen) = crate::screens::nixa_screen::host::screen(cx) {
        screen.update(cx, |s, cx| {
            s.set_look(Look::Office, cx);
            s.rooms = demo_rooms();
            s.demo_rooms = true;
        });
    }
    cx.spawn(async move |_, cx| {
        for round in 0..FLOW_ROUNDS {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let wall = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or_default();
            let done = cx.update(|cx| {
                if let Some(screen) = crate::screens::nixa_screen::host::screen(cx) {
                    screen.update(cx, |s, cx| {
                        if s.office.now.is_none() && s.office.queue.is_empty() {
                            let (from, to, text) = DEMO_TALKS[round % DEMO_TALKS.len()];
                            let event = json!({"type": "flow", "from": from, "to": to, "kind": "ask", "text": text});
                            s.on_message(&event, Instant::now(), wall);
                        }
                        s.live.dirty = true;
                        cx.notify();
                    });
                }
            });
            if done.is_err() {
                break;
            }
        }
    })
    .detach();
}
