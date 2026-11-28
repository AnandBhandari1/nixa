//! The app boundary for deep brains. Keep routing and space locks the same for tools,
//! HTTP, task picks and internal follow-ups.
use crate::app::{App, lock};
use crate::deep::{Action, Ask, AskInput};
use crate::spaces::Space;
use crate::util::now_ms;

pub fn open_spaces(app: &App) -> Result<Vec<Space>, String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| e.to_string())?;
    let rows = nixa_store::spaces::list_spaces(&conn).map_err(|e| e.to_string())?;
    Ok(rows.into_iter().filter(|s| !s.locked).map(|s| Space {
        id: s.space_id.clone(),
        name: if s.name.trim().is_empty() { s.space_id } else { s.name },
        folder: s.folder,
    }).collect())
}

pub fn ensure_open(app: &App, space: &Space) -> Result<Space, String> {
    open_spaces(app)?.into_iter().find(|s| s.id == space.id)
        .ok_or_else(|| format!("Space {} is locked or closed, so it has no deep brain. Unlock or reopen it first.", space.name))
}

pub fn resolve(app: &App, named: Option<&str>, terminal: Option<&str>) -> Result<Space, String> {
    let spaces = app.spaces();
    let space = if let Some(name) = named.filter(|s| !s.trim().is_empty()) {
        crate::spaces::find(&spaces, name).cloned().ok_or_else(|| format!("No space named {name}. No deep brain request was sent."))?
    } else if let Some(terminal) = terminal {
        app.space_of_terminal(terminal).ok_or_else(|| format!("No space is known for terminal {terminal}. Give its space explicitly."))?
    } else {
        crate::spaces::home(&spaces).cloned().ok_or("No space is known, so no deep brain can take it.")?
    };
    ensure_open(app, &space)
}

pub fn may_start(app: &App) -> bool {
    crate::settings_api::nixa_on(&app.dir)
}

pub fn can_auto_start(app: &App) -> bool {
    app.settings().deep_auto_start.unwrap_or(true) && may_start(app)
}

pub fn ask(app: &App, input: AskInput, space: &Space) -> Result<(Ask, bool), String> {
    let space = ensure_open(app, space)?;
    let can_start = can_auto_start(app);
    let out = lock(&app.deep).ask(input, &space, true, can_start, now_ms())?;
    app.deep_wake.notify_one();
    Ok(out)
}

#[derive(Debug, PartialEq)]
pub enum Ensured {
    Already(String),
    Started(Vec<Action>),
}

pub fn already_text(name: &str, state: &str, space: &Space) -> String {
    format!("Event: Deep brain check. The {name} deep brain of space {} ({}) is already {state}. Nothing new was started. Send it hard decisions with nixa deep ask.", space.name, space.id)
}

pub fn ensure(app: &App, kind: &str, space: &Space) -> Result<Ensured, String> {
    let space = ensure_open(app, space)?;
    let id = crate::deep::brain_id_of(Some(kind), "");
    if let Some((name, state)) = lock(&app.deep).up_state(&id, &space.id, now_ms()) {
        return Ok(Ensured::Already(already_text(&name, state, &space)));
    }
    start(app, kind, &space).map(Ensured::Started)
}

pub fn start(app: &App, kind: &str, space: &Space) -> Result<Vec<Action>, String> {
    let space = ensure_open(app, space)?;
    if !may_start(app) {
        return Err("Nixa is off or work is on hold, so a deep brain was not started.".into());
    }
    let mut deep = lock(&app.deep);
    let role = crate::deep::role_text(&deep.rules_file, &deep.memory_file);
    let out = deep.start(kind, &space, true, &role, now_ms())?;
    app.deep_wake.notify_one();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_store::spaces::{SpacePatch, set_space};

    #[test]
    fn locked_deleted_and_invalid_spaces_never_fall_back_home() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let conn = nixa_store::open(&app.db_file).unwrap();
        let patch = SpacePatch { folder: Some(dir.path().display().to_string()), ..SpacePatch::default() };
        set_space(&conn, "nixa", &patch).unwrap();
        let space = resolve(&app, Some("nixa"), None).unwrap();
        assert!(resolve(&app, Some("missing"), None).unwrap_err().contains("No space named missing"));
        assert!(resolve(&app, None, Some("m9")).unwrap_err().contains("terminal m9"));
        let input = || AskInput { question: "Which agent?".into(), ..AskInput::default() };
        set_space(&conn, "nixa", &SpacePatch { locked: Some(true), ..SpacePatch::default() }).unwrap();
        assert!(open_spaces(&app).unwrap().is_empty());
        assert!(ask(&app, input(), &space).is_err());
        assert!(start(&app, "claude", &space).is_err());
        assert!(resolve(&app, None, None).is_err());
        conn.execute("DELETE FROM spaces", []).unwrap();
        assert!(ask(&app, input(), &space).is_err());
        assert!(lock(&app.deep).asks.is_empty());
        assert!(lock(&app.deep).seats.is_empty());
    }

    #[test]
    fn ensure_starts_once_then_says_it_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let conn = nixa_store::open(&app.db_file).unwrap();
        set_space(&conn, "nixa", &SpacePatch { folder: Some(dir.path().display().to_string()), ..SpacePatch::default() }).unwrap();
        let space = resolve(&app, Some("nixa"), None).unwrap();
        assert!(matches!(ensure(&app, "claude", &space).unwrap(), Ensured::Started(_)));
        let Ensured::Already(text) = ensure(&app, "claude", &space).unwrap() else { panic!("started twice") };
        assert!(text.contains("already starting") && text.contains("Nothing new was started"), "{text}");
    }

    #[test]
    fn auto_start_respects_the_switch_and_off_state() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        assert!(can_auto_start(&app));
        app.save_settings(|s| s.deep_auto_start = Some(false)).unwrap();
        assert!(!can_auto_start(&app));
        assert!(may_start(&app), "explicit starts do not depend on auto start");
        std::fs::write(app.dir.join(crate::settings_api::OFF_FILE), "").unwrap();
        assert!(!may_start(&app));
    }
}
