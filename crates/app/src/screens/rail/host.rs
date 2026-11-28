use super::{
    AGENTS_PART, GENERAL_PART, MEMORY_PART, OnRail, REPEATS_PART, RULES_PART, RailAction,
    SCHEDULES_PART, command_menu_argv, render, state,
};
use crate::screens::{events::Level, git::join::toast};
use crate::workspace::Workspace;
use gpui::{Context, Div, Stateful};
use nixa_ui::theme;
use std::rc::Rc;

pub fn act(ws: &mut Workspace, action: RailAction, cx: &mut Context<Workspace>) {
    match action {
        RailAction::Commands => {
            if let Some(folder) = folder(ws) {
                ws.new_tab_in(command_menu_argv(), Some("Commands".into()), Some(folder), cx);
            }
        }
        RailAction::Tasks => crate::screens::tasks::open(cx),
        RailAction::Panel(kind) => ws.toggle_panel(kind, cx),
        RailAction::Repeats => ws.open_settings(REPEATS_PART, cx),
        RailAction::DeepBrain => {
            if let Some(space) = ws.active_space().map(|s| s.id.clone()) {
                deep_brain(space, cx);
            }
        }
        RailAction::Agents => ws.open_settings(AGENTS_PART, cx),
        RailAction::Workflows => crate::screens::workflows::open(cx),
        RailAction::Automate => ws.open_settings(SCHEDULES_PART, cx),
        RailAction::Rules => ws.open_settings(RULES_PART, cx),
        RailAction::Memory => ws.open_settings(MEMORY_PART, cx),
        RailAction::Settings => ws.open_settings(GENERAL_PART, cx),
    }
}

pub const DEEP_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

pub fn deep_body(space: &str) -> serde_json::Value {
    serde_json::json!({"space": space, "ensure": true})
}

pub fn deep_note(reply: &serde_json::Value) -> String {
    let data = reply.get("data").unwrap_or(reply);
    let space = data.get("space").and_then(serde_json::Value::as_str).unwrap_or("this space");
    if data.get("started").and_then(serde_json::Value::as_bool).unwrap_or(false) {
        format!("Starting the deep brain of space {space}.")
    } else {
        format!("The deep brain of space {space} is already up or starting. Nixa was told.")
    }
}

fn deep_brain(space: String, cx: &mut Context<Workspace>) {
    let job = cx.background_executor().spawn(async move {
        crate::links::brain::http(&nixa_protocol::paths::brain_addr(), "POST", "/deep/start", Some(&deep_body(&space)), DEEP_WAIT).map_err(|e| format!("{e:#}"))
    });
    cx.spawn(async move |_, cx| {
        let out = job.await;
        cx.update(|cx| match out {
            Ok(reply) => toast("Deep brain", &deep_note(&reply), Level::Success, cx),
            Err(why) => toast("Deep brain", &why, Level::Error, cx),
        })
        .ok();
    })
    .detach();
}

fn folder(ws: &Workspace) -> Option<String> {
    ws.active_space()
        .map(|s| s.folder.clone())
        .filter(|f| !f.is_empty() && std::path::Path::new(f).is_dir())
}

pub fn host(ws: &Workspace, cx: &mut Context<Workspace>) -> Stateful<Div> {
    let t = theme(cx).clone();
    let mut s = state(ws.panel, ws.facts.tasks_waiting, folder(ws));
    s.space = ws.active_space().map(|s| s.name.clone());
    let this = cx.entity().downgrade();
    let on: OnRail = Rc::new(move |action, _, app| {
        this.update(app, |ws, cx| act(ws, action, cx)).ok();
    });
    render(&s, &t, on)
}
