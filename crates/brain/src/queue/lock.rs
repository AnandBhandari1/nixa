use crate::spaces::{Space, folder_in, for_folder};
use nixa_store::spaces::Space as Row;
use nixa_store::tasks::Task;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locked {
    pub id: String,
    pub name: String,
}

pub fn locked_text(name: &str) -> String {
    format!("Space \"{name}\" is locked. No new work starts there until the user unlocks it.")
}

fn same(a: &str, b: &str) -> bool {
    !a.trim().is_empty() && a.trim().eq_ignore_ascii_case(b.trim())
}

fn absolute(path: &str) -> bool {
    let text = path.trim();
    text.starts_with('/') || text == "~" || text.starts_with("~/")
}

pub fn locked_space(rows: &[Row], id: &str, name: &str, folder: &str) -> Option<Locked> {
    rows.iter()
        .filter(|row| row.locked)
        .find(|row| {
            (!id.is_empty() && row.space_id == id)
                || same(&row.name, name)
                || (absolute(folder)
                    && !row.folder.trim().is_empty()
                    && folder_in(folder, &row.folder))
        })
        .map(|row| Locked {
            id: row.space_id.clone(),
            name: if row.name.trim().is_empty() {
                row.space_id.clone()
            } else {
                row.name.clone()
            },
        })
}

pub fn task_locked(task: &Task, rows: &[Row]) -> Option<Locked> {
    locked_space(rows, &task.space_id, &task.space_name, "")
        .or_else(|| locked_space(rows, "", "", &task.repo))
}

pub fn space_locked(space: &Space, rows: &[Row]) -> bool {
    rows.iter()
        .any(|row| row.locked && row.space_id == space.id)
}

pub fn unlocked(tasks: Vec<Task>, rows: &[Row]) -> Vec<Task> {
    tasks
        .into_iter()
        .filter(|task| task_locked(task, rows).is_none())
        .collect()
}


pub fn folder_mismatch(known: &[Space], space: &Space, folder: &str) -> Option<String> {
    let folder = folder.trim();
    if folder.is_empty() || !absolute(folder) {
        return None;
    }
    let owner = for_folder(known, folder);
    if !space.folder.trim().is_empty() {
        if folder_in(folder, &space.folder) {
            return None;
        }
        let place = match owner {
            Some(other) if other.id != space.id => format!(
                "belongs to space \"{}\" ({}), outside",
                other.name, other.folder
            ),
            _ => "is outside".to_string(),
        };
        return Some(format!(
            "Job folder \"{folder}\" {place} space \"{}\" ({}).",
            space.name, space.folder
        ));
    }
    match owner {
        Some(other) if other.id != space.id => Some(format!(
            "Job folder \"{folder}\" belongs to space \"{}\" ({}), not space \"{}\".",
            other.name, other.folder, space.name
        )),
        _ => None,
    }
}

pub fn folder_owner<'a>(known: &'a [Space], space: &Space, folder: &str) -> Option<&'a Space> {
    for_folder(known, folder.trim()).filter(|other| other.id != space.id)
}

pub fn auto_space(_: &Space) -> bool {
    true
}
