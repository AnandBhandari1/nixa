use crate::named_layouts::store as layouts;
use crate::screens::agents::{self, AgentsEvent};
use crate::screens::spaces::{self as space_screen, SpacesEvent};
use crate::workspace::{PanelKind, Workspace};
use crate::{layout, spaces, tabs};
use gpui::Context;
use nixa_protocol::control::{AppCommand, Permission};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum More {
    TabRename,
    SpaceRename,
    SpaceLock,
    SpaceOpen,
    SpaceClose,
    LayoutSave,
    LayoutUse,
    LayoutDelete,
    ChatShow,
    ChatHide,
}

fn text(about: &str) -> Value {
    json!({ "type": "string", "description": about })
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn one(name: &str, title: &str, permission: Permission, args: Value, more: More) -> (AppCommand, More) {
    let info = AppCommand { name: name.into(), title: title.into(), permission, args };
    (info, more)
}

pub fn entries() -> Vec<(AppCommand, More)> {
    let space = || text("Space id from the state. Default: the shown space");
    let named = |about: &str| schema(json!({ "name": text(about) }), &["name"]);
    vec![
        one(
            "tab.rename",
            "Rename a tab.",
            Permission::Write,
            schema(json!({ "name": text("New tab name"), "id": text("Tab id from the state. Default: the active tab") }), &["name"]),
            More::TabRename,
        ),
        one(
            "space.rename",
            "Rename a space.",
            Permission::Write,
            schema(json!({ "name": text("New space name"), "id": space() }), &["name"]),
            More::SpaceRename,
        ),
        one("space.lock", "Lock a space so Nixa does not place work in it.", Permission::Write, schema(json!({ "id": space() }), &[]), More::SpaceLock),
        one("space.open", "Open a locked space for Nixa again.", Permission::Write, schema(json!({ "id": space() }), &[]), More::SpaceOpen),
        one("space.close", "Close a space and end its tabs.", Permission::Destructive, schema(json!({ "id": space() }), &[]), More::SpaceClose),
        one("layout.save", "Save the sidebar and panel widths as a named layout.", Permission::Write, named("Layout name"), More::LayoutSave),
        one("layout.use", "Use a saved layout.", Permission::Write, named("Layout name"), More::LayoutUse),
        one("layout.delete", "Delete a saved layout.", Permission::Destructive, named("Layout name"), More::LayoutDelete),
        one("chat.show", "Show the Nixa chat panel.", Permission::Read, schema(json!({}), &[]), More::ChatShow),
        one("chat.hide", "Hide the Nixa chat panel.", Permission::Read, schema(json!({}), &[]), More::ChatHide),
    ]
}

fn given(args: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("Argument \"{key}\" must be a string. Nothing was run.")),
    }
}

fn needed(args: &Map<String, Value>, key: &str) -> Result<String, String> {
    given(args, key)?.ok_or_else(|| format!("Missing required argument \"{key}\". Nothing was run."))
}

fn space_id(ws: &Workspace, args: &Map<String, Value>) -> Result<String, String> {
    let id = match given(args, "id")? {
        Some(id) => id,
        None => ws.spaces.active.clone().ok_or("No space is shown. Nothing was run.")?,
    };
    match spaces::find(&ws.spaces, &id) {
        Some(_) => Ok(id),
        None => Err(format!("There is no space {id}. Nothing was run.")),
    }
}

fn layout_name(args: &Map<String, Value>) -> Result<String, String> {
    let raw = needed(args, "name")?;
    layouts::clean_name(&raw).ok_or_else(|| {
        format!("Use 1 to {} letters, digits, spaces, - or _ for the layout name. Nothing was run.", layouts::NAME_MAX)
    })
}

fn rename_tab(ws: &mut Workspace, args: &Map<String, Value>, cx: &mut Context<Workspace>) -> Result<(), String> {
    let name = needed(args, "name")?;
    let id = match given(args, "id")? {
        Some(id) => id,
        None => ws.active_tab().map(|t| t.id.clone()).ok_or("No tab is active. Nothing was run.")?,
    };
    let tab = tabs::find(&ws.tabs, &id).ok_or_else(|| format!("There is no tab {id}. Nothing was run."))?;
    let term = layout::leaves(&tab.root).first().cloned().ok_or("That tab has no terminal. Nothing was run.")?;
    agents::on_event(ws, &AgentsEvent::SetName { term, name }, cx);
    Ok(())
}

fn lock(ws: &mut Workspace, args: &Map<String, Value>, locked: bool, cx: &mut Context<Workspace>) -> Result<(), String> {
    let id = space_id(ws, args)?;
    space_screen::handle(ws, &SpacesEvent::Lock { id, locked }, cx);
    Ok(())
}

pub fn run(ws: &mut Workspace, more: More, args: &Map<String, Value>, cx: &mut Context<Workspace>) -> Result<(), String> {
    match more {
        More::TabRename => rename_tab(ws, args, cx)?,
        More::SpaceRename => {
            let name = needed(args, "name")?;
            let id = space_id(ws, args)?;
            space_screen::handle(ws, &SpacesEvent::Rename { id, name }, cx);
        }
        More::SpaceLock => lock(ws, args, true, cx)?,
        More::SpaceOpen => lock(ws, args, false, cx)?,
        More::SpaceClose => {
            let id = space_id(ws, args)?;
            space_screen::handle(ws, &SpacesEvent::Close(id), cx);
        }
        More::LayoutSave => {
            let name = layout_name(args)?;
            layouts::write(&layouts::folder(), &name, &layouts::capture(ws))
                .map_err(|e| format!("Cannot save the layout \"{name}\": {e:#}"))?;
        }
        More::LayoutUse => {
            let name = layout_name(args)?;
            let saved = layouts::read(&layouts::folder(), &name)
                .map_err(|_| format!("There is no saved layout \"{name}\". Nothing was run."))?;
            layouts::apply(ws, &saved, cx);
        }
        More::LayoutDelete => {
            let name = layout_name(args)?;
            layouts::delete(&layouts::folder(), &name)
                .map_err(|_| format!("There is no saved layout \"{name}\". Nothing was run."))?;
        }
        More::ChatShow => ws.set_panel(Some(PanelKind::Nixa), cx),
        More::ChatHide => {
            if ws.panel == Some(PanelKind::Nixa) {
                ws.set_panel(None, cx);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_has_a_unique_name_and_an_object_schema() {
        let list = entries();
        for (i, (a, _)) in list.iter().enumerate() {
            assert_eq!(a.args["type"], "object", "{}", a.name);
            assert!(list[i + 1..].iter().all(|(b, _)| b.name != a.name));
        }
        let close = list.iter().find(|(e, _)| e.name == "space.close").unwrap();
        assert_eq!(close.0.permission, Permission::Destructive);
    }

    #[test]
    fn names_and_strings_are_checked() {
        let mut args = Map::new();
        assert!(needed(&args, "name").unwrap_err().contains("Missing"));
        args.insert("name".into(), json!(3));
        assert!(given(&args, "name").unwrap_err().contains("must be a string"));
        args.insert("name".into(), json!("  "));
        assert_eq!(given(&args, "name").unwrap(), None);
        args.insert("name".into(), json!("a/b"));
        assert!(layout_name(&args).unwrap_err().contains("letters"));
        args.insert("name".into(), json!(" wide "));
        assert_eq!(layout_name(&args).unwrap(), "wide");
    }
}

#[cfg(test)]
mod ui_tests {
    use crate::links::control::answer;
    use crate::workspace::{EngineState, PanelKind, Workspace};
    use gpui::{Entity, TestAppContext, VisualTestContext, px, size};
    use nixa_protocol::control::{self, ControlReply, ControlRequest};
    use nixa_protocol::{app_words, paths};
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::sync::Once;

    fn data_dir() -> PathBuf {
        PathBuf::from(format!("/tmp/mo-cm-{}", std::process::id()))
    }

    fn setup(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        static ENV: Once = Once::new();
        ENV.call_once(|| unsafe {
            std::env::set_var(paths::ENV_DATA_DIR, data_dir());
            std::env::set_var(paths::ENV_ENGINE_SOCKET, data_dir().join("e.sock"));
            std::env::set_var(control::ENV_APP_SOCKET, data_dir().join("a.sock"));
        });
        cx.update(nixa_ui::init);
        let (ws, cx) = cx.add_window_view(Workspace::new);
        cx.simulate_resize(size(px(1280.), px(800.)));
        ws.update(cx, |ws, cx| {
            ws.engine = EngineState::Up;
            if ws.spaces.spaces.is_empty() {
                crate::spaces::add(&mut ws.spaces, &crate::spaces::home());
            }
            cx.notify();
        });
        cx.run_until_parked();
        (ws, cx)
    }

    fn ask(ws: &Entity<Workspace>, cx: &mut VisualTestContext, req: ControlRequest) -> ControlReply {
        let reply = ws.update_in(cx, |ws, window, cx| answer(ws, req, window, cx));
        cx.run_until_parked();
        reply
    }

    fn run(ws: &Entity<Workspace>, cx: &mut VisualTestContext, command: &str, args: Value) -> ControlReply {
        let req = ControlRequest::Run { command: command.into(), args, cleared: false, token: String::new() };
        ask(ws, cx, req)
    }

    #[gpui::test]
    fn every_cli_word_names_a_real_window_command(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx);
        let ControlReply::Commands { commands } = ask(&ws, cx, ControlRequest::Commands) else {
            panic!("no command list");
        };
        for word in app_words::WORDS {
            let found = commands.iter().find(|c| c.name == word.command);
            let found = found.unwrap_or_else(|| panic!("{} names {} which the window does not have", word.say, word.command));
            if let Some(arg) = word.arg {
                assert!(found.args["properties"].get(arg).is_some(), "{} has no {arg}", word.command);
            }
        }
    }

    #[gpui::test]
    fn title_double_click_is_listed_and_runs(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx);
        let ControlReply::Commands { commands } = ask(&ws, cx, ControlRequest::Commands) else {
            panic!("no command list");
        };
        assert!(commands.iter().any(|c| c.name == "app.title-double-click"));
        assert!(matches!(run(&ws, cx, "app.title-double-click", Value::Null), ControlReply::Ran { .. }));
    }

    #[gpui::test]
    fn chat_space_and_layout_commands_change_the_window(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx);
        assert!(matches!(run(&ws, cx, "chat.show", Value::Null), ControlReply::Ran { .. }));
        assert_eq!(ws.read_with(cx, |ws, _| ws.panel), Some(PanelKind::Nixa));
        run(&ws, cx, "chat.hide", Value::Null);
        assert_eq!(ws.read_with(cx, |ws, _| ws.panel), None);
        run(&ws, cx, "space.rename", json!({"name": "shop"}));
        let name = ws.read_with(cx, |ws, _| ws.active_space().map(|s| s.name.clone()));
        assert_eq!(name.as_deref(), Some("shop"));
        run(&ws, cx, "space.lock", Value::Null);
        assert!(ws.read_with(cx, |ws, _| ws.active_space().is_some_and(|s| s.locked)));
        run(&ws, cx, "space.open", Value::Null);
        assert!(ws.read_with(cx, |ws, _| ws.active_space().is_some_and(|s| !s.locked)));
        assert!(matches!(run(&ws, cx, "layout.save", json!({"name": "wide"})), ControlReply::Ran { .. }));
        assert!(matches!(run(&ws, cx, "layout.use", json!({"name": "wide"})), ControlReply::Ran { .. }));
        assert!(matches!(run(&ws, cx, "layout.delete", json!({"name": "wide"})), ControlReply::Ran { .. }));
        let ControlReply::Error { message } = run(&ws, cx, "layout.use", json!({"name": "wide"})) else {
            panic!("a missing layout must fail");
        };
        assert!(message.contains("no saved layout"), "{message}");
        let ControlReply::Error { message } = run(&ws, cx, "space.rename", json!({"name": "x", "nope": 1})) else {
            panic!("an unknown argument must fail");
        };
        assert!(message.contains("Unknown argument"), "{message}");
        std::fs::remove_dir_all(data_dir()).ok();
    }
}
