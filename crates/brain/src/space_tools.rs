//! nixa_spaces: the spaces (project folders) of the window, so Nixa chat can open, rename, lock or close one without the Settings window.

use crate::app::App;
use crate::tools::{Args, ToolResult, flag, need, schema, spec, text};
use crate::turns::ToolSpec;
use nixa_store::spaces::SpacePatch;
use nixa_store::tasks::TaskStatus;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

pub const NAME: &str = "nixa_spaces";
pub const CHANGED: &str = "spaces_changed";
pub const NAME_MAX: usize = 60;
/// The window paints six space colors and picks the least used one for a new space.
const COLOR_COUNT: i64 = 6;

type Row = nixa_store::spaces::Space;

pub fn specs() -> Vec<ToolSpec> {
    vec![spec(
        NAME,
        "See and change the spaces (project folders) of the window, without the Settings window. action list shows every space with id, name, folder, lock and whether it is the active one. add takes a folder that exists (absolute or starting with ~) and an optional name, and gives back the space; a folder that already is a space is returned as existing. rename takes space and name. lock takes space and on: a locked space refuses new work from Nixa. remove takes space and closes it; it refuses while terminals, open tasks or a team run still use the space. space is an id, a name or a folder path. The window shows the change at once.",
        schema(
            json!({
                "action": {"type": "string", "enum": ["list", "add", "rename", "lock", "remove"]},
                "space": {"type": "string", "description": "Space id, name or folder path. Needed by rename, lock and remove."},
                "folder": {"type": "string", "description": "With add: the project folder, absolute or starting with ~. It must exist."},
                "name": {"type": "string", "minLength": 1, "maxLength": NAME_MAX, "description": "With add or rename: the name the window shows."},
                "on": {"type": "boolean", "description": "With lock: true locks the space for Nixa, false opens it again."},
            }),
            &["action"],
        ),
    )]
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    match text(args, "action").as_deref() {
        Some("list") => list(app),
        Some("add") => add(app, args),
        Some("rename") => rename(app, args),
        Some("lock") => lock(app, args),
        Some("remove") => remove(app, args).await,
        Some(other) => Err(format!("nixa_spaces has no action {other:?}. Use list, add, rename, lock or remove. Nothing was changed.")),
        None => Err("action is needed: list, add, rename, lock or remove. Nothing was changed.".into()),
    }
}

fn open(app: &App) -> Result<nixa_store::Connection, String> {
    nixa_store::open(&app.db_file).map_err(|error| format!("Could not open the Nixa data: {error}"))
}

fn rows(conn: &nixa_store::Connection) -> Result<Vec<Row>, String> {
    nixa_store::spaces::list_spaces(conn).map_err(|error| error.to_string())
}

fn active(conn: &nixa_store::Connection) -> Result<Option<String>, String> {
    nixa_store::spaces::active_space(conn).map_err(|error| error.to_string())
}

fn shown(space: &Row, active: Option<&str>) -> Value {
    json!({
        "id": space.space_id,
        "name": if space.name.trim().is_empty() { space.space_id.clone() } else { space.name.clone() },
        "folder": space.folder,
        "locked": space.locked,
        "active": active == Some(space.space_id.as_str()),
        "position": space.position,
    })
}

fn list(app: &App) -> ToolResult {
    let conn = open(app)?;
    let active = active(&conn)?;
    let spaces: Vec<Value> = rows(&conn)?.iter().map(|s| shown(s, active.as_deref())).collect();
    Ok(json!({ "spaces": spaces, "active": active }).to_string())
}

fn expand(folder: &str) -> Option<PathBuf> {
    let text = folder.trim();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let full = if text == "~" {
        home?
    } else if let Some(rest) = text.strip_prefix("~/") {
        home?.join(rest)
    } else {
        PathBuf::from(text)
    };
    if full.is_absolute() { Some(full) } else { None }
}

/// The next free id of the form s<number>, as the window gives them.
pub fn next_id(spaces: &[Row]) -> String {
    let top = spaces.iter().filter_map(|s| s.space_id.strip_prefix('s').and_then(|n| n.parse::<u64>().ok())).max().unwrap_or(0);
    format!("s{}", top + 1)
}

fn least_used_color(spaces: &[Row]) -> i64 {
    (0..COLOR_COUNT).min_by_key(|c| spaces.iter().filter(|s| s.color == *c).count()).unwrap_or(0)
}

fn default_name(folder: &str) -> String {
    if folder == std::env::var("HOME").unwrap_or_default() {
        return "~".to_string();
    }
    PathBuf::from(folder)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| folder.to_string())
}

/// A space by id, then by name, then by the folder it holds (a sub-folder counts, the deepest one wins).
fn find<'a>(spaces: &'a [Row], key: &str) -> Result<&'a Row, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("space is needed: an id, a name or a folder path.".into());
    }
    let known: Vec<crate::spaces::Space> = crate::spaces::from_store_of(spaces);
    let found = spaces
        .iter()
        .find(|s| s.space_id.eq_ignore_ascii_case(key))
        .or_else(|| spaces.iter().find(|s| s.name.trim().eq_ignore_ascii_case(key)))
        .or_else(|| crate::spaces::find(&known, key).and_then(|s| spaces.iter().find(|r| r.space_id == s.id)));
    found.ok_or_else(|| format!("There is no space {key:?}. Use list to see the spaces."))
}

fn add(app: &App, args: &Args) -> ToolResult {
    let folder = need(args, "folder")?;
    let full = expand(&folder).ok_or_else(|| format!("The folder {folder:?} is not absolute. Give the full path, or start it with ~/. Nothing was added."))?;
    if !full.is_dir() {
        return Err(format!("{} is not a folder on this Mac. Nothing was added.", full.display()));
    }
    crate::project_tools::policy::work_folder(&full).map_err(|e| format!("{e} Nothing was added."))?;
    let folder = full.to_string_lossy().to_string();
    let conn = open(app)?;
    let spaces = rows(&conn)?;
    if let Some(found) = nixa_store::spaces::space_for_folder(&conn, &folder).map_err(|error| error.to_string())? {
        let space = spaces.iter().find(|s| s.space_id == found.space_id).ok_or("The space of that folder could not be read. Nothing was added.")?;
        return Ok(json!({ "space": shown(space, None), "existing": true, "note": "That folder already is a space. Nothing was added." }).to_string());
    }
    let id = next_id(&spaces);
    let name = text(args, "name").map(|n| n.chars().take(NAME_MAX).collect::<String>()).unwrap_or_else(|| default_name(&folder));
    let position = spaces.iter().map(|s| s.position + 1).max().unwrap_or(0);
    let patch = SpacePatch { name: Some(name), folder: Some(folder), color: Some(least_used_color(&spaces)), position: Some(position), ..SpacePatch::default() };
    let space = nixa_store::spaces::set_space(&conn, &id, &patch).map_err(|error| format!("The space was not saved: {error}"))?;
    changed(app);
    Ok(json!({
        "space": shown(&space, None),
        "existing": false,
        "note": format!("The space is open in the window. Start agents in it with nixa_agent_start and space {}.", space.space_id),
    })
    .to_string())
}

fn rename(app: &App, args: &Args) -> ToolResult {
    let key = need(args, "space")?;
    let name: String = need(args, "name")?.chars().take(NAME_MAX).collect();
    let conn = open(app)?;
    let id = find(&rows(&conn)?, &key)?.space_id.clone();
    let patch = SpacePatch { name: Some(name), ..SpacePatch::default() };
    let space = nixa_store::spaces::set_space(&conn, &id, &patch).map_err(|error| format!("The space was not renamed: {error}"))?;
    changed(app);
    Ok(json!({ "space": shown(&space, None), "note": "Renamed." }).to_string())
}

fn lock(app: &App, args: &Args) -> ToolResult {
    let key = need(args, "space")?;
    let on = flag(args, "on").ok_or("on is needed with lock: true locks the space for Nixa, false opens it again. Nothing was changed.")?;
    let conn = open(app)?;
    let found = find(&rows(&conn)?, &key)?.clone();
    if !on && found.locked {
        return Err(format!("Space {} is locked by the user. Only the user opens it again, in the window. Nothing was changed.", found.name));
    }
    let id = found.space_id.clone();
    let patch = SpacePatch { locked: Some(on), ..SpacePatch::default() };
    let space = nixa_store::spaces::set_space(&conn, &id, &patch).map_err(|error| format!("The space lock was not saved: {error}"))?;
    changed(app);
    let note = if on { "Locked. Nixa starts no new work in it until it is opened again." } else { "Open again. Nixa may start work in it." };
    Ok(json!({ "space": shown(&space, None), "note": note }).to_string())
}

async fn remove(app: &Arc<App>, args: &Args) -> ToolResult {
    let key = need(args, "space")?;
    let conn = open(app)?;
    let space = find(&rows(&conn)?, &key)?.clone();
    let id = space.space_id.as_str();
    let terminals: Vec<String> = app
        .terminals()
        .iter()
        .filter(|seen| seen.info.running && app.space_of_terminal(seen.info.id.as_str()).is_some_and(|s| s.id == id))
        .map(|seen| seen.info.id.to_string())
        .collect();
    if !terminals.is_empty() {
        return Err(format!(
            "Space {} still has running terminals: {}. Close them with nixa_term_remove first, then remove the space. Nothing was changed.",
            space.name,
            terminals.join(", ")
        ));
    }
    let runs: Vec<String> = crate::teams::list(&app.teams)
        .into_iter()
        .filter(|r| r.state == crate::teams::model::RunState::Running && r.space == id)
        .map(|r| r.team_name)
        .collect();
    if !runs.is_empty() {
        return Err(format!("Space {} still has a running team run: {}. Stop it first. Nothing was changed.", space.name, runs.join(", ")));
    }
    let filter = nixa_store::tasks::TaskFilter {
        space: Some(id.to_string()),
        status: vec![TaskStatus::Inbox, TaskStatus::Todo, TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review],
        ..nixa_store::tasks::TaskFilter::default()
    };
    let tasks: Vec<String> = nixa_store::tasks::list_tasks(&conn, &filter).unwrap_or_default().iter().map(|t| format!("#{}", t.id)).collect();
    if !tasks.is_empty() {
        return Err(format!("Space {} still has open tasks: {}. Close or move them first. Nothing was changed.", space.name, tasks.join(", ")));
    }
    let gone = nixa_store::spaces::remove_space(&conn, id).map_err(|error| format!("The space was not removed: {error}"))?;
    if !gone {
        return Err(format!("Space {} was already gone. Nothing was changed.", space.name));
    }
    let left = rows(&conn)?;
    if active(&conn)?.as_deref() == Some(id) {
        nixa_store::spaces::set_active_space(&conn, left.first().map(|s| s.space_id.as_str())).map_err(|error| format!("The space was removed, but the active space was not moved: {error}"))?;
    }
    changed(app);
    Ok(json!({ "removed": id, "name": space.name, "folder": space.folder, "note": "The space is closed in the window. Its files stay." }).to_string())
}

fn changed(app: &App) {
    app.send_out(json!({ "type": CHANGED }));
}

#[cfg(test)]
mod tests;
