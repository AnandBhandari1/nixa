use anyhow::Result;
use gpui::Hsla;
use nixa_protocol::paths;
use nixa_store::Connection;
use nixa_store::spaces::{self as store, SpacePatch};
use nixa_ui::{Icon, Theme};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const COLOR_COUNT: usize = 6;
pub const SHORT_KEY: &str = "space.short";
const SHAPES: [Icon; 6] = [
    Icon::Circle,
    Icon::Square,
    Icon::Triangle,
    Icon::Diamond,
    Icon::Hexagon,
    Icon::Star,
];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub color: usize,
    pub position: i64,
    pub locked: bool,
    pub short: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spaces {
    pub spaces: Vec<Space>,
    pub active: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fixed {
    pub added: Option<String>,
    pub active: bool,
}

pub fn old_file() -> PathBuf {
    paths::data_dir().join(store::OLD_FILE)
}

pub fn import(conn: &Connection, path: &Path) -> Result<usize> {
    store::import_spaces_json(conn, path)
}

fn shorts(conn: &Connection) -> Result<Map<String, Value>> {
    Ok(nixa_store::settings::get_setting(conn, SHORT_KEY)?
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default())
}

pub fn load(conn: &Connection) -> Result<Spaces> {
    let shorts = shorts(conn)?;
    let spaces = store::list_spaces(conn)?
        .into_iter()
        .map(|s| Space {
            short: shorts
                .get(&s.space_id)
                .and_then(Value::as_str)
                .map(str::to_string),
            id: s.space_id,
            name: s.name,
            folder: s.folder,
            color: usize::try_from(s.color).unwrap_or(0),
            position: s.position,
            locked: s.locked,
        })
        .collect();
    Ok(Spaces {
        spaces,
        active: store::active_space(conn)?,
    })
}

pub fn write_space(conn: &Connection, space: &Space) -> Result<()> {
    let patch = SpacePatch {
        name: Some(space.name.clone()),
        folder: Some(space.folder.clone()),
        color: Some(space.color as i64),
        position: Some(space.position),
        ..SpacePatch::default()
    };
    store::set_space(conn, &space.id, &patch)?;
    Ok(())
}

pub fn write_active(conn: &Connection, spaces: &Spaces) -> Result<()> {
    store::set_active_space(conn, spaces.active.as_deref())
}

pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    store::remove_space(conn, id)?;
    Ok(())
}

pub fn write_fixed(conn: &Connection, spaces: &Spaces, fixed: &Fixed) -> Result<()> {
    if let Some(space) = fixed.added.as_deref().and_then(|id| find(spaces, id)) {
        write_space(conn, space)?;
    }
    if fixed.active || fixed.added.is_some() {
        write_active(conn, spaces)?;
    }
    Ok(())
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

pub fn folder_name(folder: &str) -> String {
    Path::new(folder)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| folder.to_string())
}

pub fn short_folder(folder: &str, home: &str) -> String {
    if folder == home {
        "~".to_string()
    } else if let Some(rest) = folder.strip_prefix(&format!("{home}/")) {
        format!("~/{rest}")
    } else {
        folder.to_string()
    }
}

fn next_id(spaces: &Spaces) -> String {
    let top = spaces
        .spaces
        .iter()
        .filter_map(|s| s.id.strip_prefix('s')?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("s{}", top + 1)
}

fn least_used_color(spaces: &Spaces) -> usize {
    let uses = |c: usize| spaces.spaces.iter().filter(|s| s.color == c).count();
    (0..COLOR_COUNT).min_by_key(|&c| uses(c)).unwrap_or(0)
}

pub fn add(spaces: &mut Spaces, folder: &str) -> String {
    let id = next_id(spaces);
    let color = least_used_color(spaces);
    let name = if folder == home() {
        "~".to_string()
    } else {
        folder_name(folder)
    };
    let position = spaces
        .spaces
        .iter()
        .map(|s| s.position + 1)
        .max()
        .unwrap_or(0);
    spaces.spaces.push(Space {
        id: id.clone(),
        name,
        folder: folder.to_string(),
        color,
        position,
        ..Space::default()
    });
    spaces.active = Some(id.clone());
    id
}

pub fn merge_saved(conn: &Connection, spaces: &mut Spaces) -> Result<bool> {
    let mut changed = false;
    for space in load(conn)?.spaces {
        if find(spaces, &space.id).is_none() {
            spaces.spaces.push(space);
            changed = true;
        }
    }
    Ok(changed)
}

pub fn add_saved(conn: &Connection, spaces: &mut Spaces, folder: &str) -> Result<String> {
    let mut next = spaces.clone();
    let tx = conn.unchecked_transaction()?;
    merge_saved(&tx, &mut next)?;
    let known = store::space_for_folder(&tx, folder)?
        .map(|row| row.space_id)
        .filter(|id| find(&next, id).is_some());
    let id = match known {
        Some(id) => {
            next.active = Some(id.clone());
            id
        }
        None => {
            let id = add(&mut next, folder);
            if let Some(space) = find(&next, &id) {
                write_space(&tx, space)?;
            }
            id
        }
    };
    write_active(&tx, &next)?;
    tx.commit()?;
    *spaces = next;
    Ok(id)
}

pub fn remove(spaces: &mut Spaces, id: &str) {
    let index = spaces.spaces.iter().position(|s| s.id == id);
    spaces.spaces.retain(|s| s.id != id);
    if spaces.active.as_deref() == Some(id) {
        let pick = index.map(|i| i.min(spaces.spaces.len().saturating_sub(1)));
        spaces.active = pick
            .and_then(|i| spaces.spaces.get(i))
            .map(|s| s.id.clone());
    }
}

pub fn rename(spaces: &mut Spaces, id: &str, name: &str) -> bool {
    let name = name.trim();
    match spaces.spaces.iter_mut().find(|s| s.id == id) {
        Some(space) if !name.is_empty() && space.name != name => {
            space.name = name.to_string();
            true
        }
        _ => false,
    }
}

pub fn fix_active(spaces: &mut Spaces) -> Fixed {
    let mut fixed = Fixed::default();
    let active_ok = spaces
        .active
        .as_ref()
        .is_some_and(|a| spaces.spaces.iter().any(|s| &s.id == a));
    if !active_ok {
        spaces.active = spaces.spaces.first().map(|s| s.id.clone());
        fixed.active = true;
    }
    fixed
}

pub fn active(spaces: &Spaces) -> Option<&Space> {
    let id = spaces.active.as_ref()?;
    spaces.spaces.iter().find(|s| &s.id == id)
}

pub fn find<'a>(spaces: &'a Spaces, id: &str) -> Option<&'a Space> {
    spaces.spaces.iter().find(|s| s.id == id)
}

pub fn move_to(spaces: &mut Spaces, id: &str, to: usize) -> bool {
    let Some(from) = spaces.spaces.iter().position(|s| s.id == id) else {
        return false;
    };
    let space = spaces.spaces.remove(from);
    let to = to.min(spaces.spaces.len());
    spaces.spaces.insert(to, space);
    let mut changed = from != to;
    for (i, space) in spaces.spaces.iter_mut().enumerate() {
        let position = i as i64;
        if space.position != position {
            space.position = position;
            changed = true;
        }
    }
    changed
}

pub fn write_order(conn: &Connection, spaces: &Spaces) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for space in &spaces.spaces {
        let patch = SpacePatch {
            position: Some(space.position),
            ..SpacePatch::default()
        };
        store::set_space(&tx, &space.id, &patch)?;
    }
    tx.commit()?;
    Ok(())
}

pub fn set_locked(spaces: &mut Spaces, id: &str, locked: bool) -> bool {
    match spaces.spaces.iter_mut().find(|s| s.id == id) {
        Some(space) if space.locked != locked => {
            space.locked = locked;
            true
        }
        _ => false,
    }
}

pub fn write_lock(conn: &Connection, id: &str, locked: bool) -> Result<()> {
    let patch = SpacePatch {
        locked: Some(locked),
        ..SpacePatch::default()
    };
    store::set_space(conn, id, &patch)?;
    Ok(())
}

pub fn refresh(conn: &Connection, spaces: &mut Spaces) -> Result<bool> {
    let saved = store::list_spaces(conn)?;
    let shorts = shorts(conn)?;
    let mut changed = false;
    // A space the brain added (nixa_spaces) or removed since the window last looked.
    if saved.iter().any(|row| find(spaces, &row.space_id).is_none()) {
        for space in load(conn)?.spaces {
            if find(spaces, &space.id).is_none() {
                spaces.spaces.push(space);
                changed = true;
            }
        }
    }
    let gone: Vec<String> = spaces
        .spaces
        .iter()
        .filter(|space| !saved.iter().any(|row| row.space_id == space.id))
        .map(|space| space.id.clone())
        .collect();
    for id in gone {
        remove(spaces, &id);
        changed = true;
    }
    if changed {
        fix_active(spaces);
        let mut sorted = spaces.spaces.clone();
        sorted.sort_by_key(|space| space.position);
        if sorted != spaces.spaces {
            spaces.spaces = sorted;
        }
    }
    for space in &mut spaces.spaces {
        if let Some(row) = saved.iter().find(|r| r.space_id == space.id)
            && row.locked != space.locked
        {
            space.locked = row.locked;
            changed = true;
        }
        let short = shorts
            .get(&space.id)
            .and_then(Value::as_str)
            .map(str::to_string);
        if short != space.short {
            space.short = short;
            changed = true;
        }
    }
    Ok(changed)
}

pub fn close(spaces: &mut Spaces, id: &str) -> Fixed {
    remove(spaces, id);
    fix_active(spaces)
}

/// The ids of the spaces the data folder holds now.
pub fn saved_ids(conn: &Connection) -> Result<Vec<String>> {
    Ok(store::list_spaces(conn)?.into_iter().map(|s| s.space_id).collect())
}

pub fn color(t: &Theme, index: usize) -> Hsla {
    let colors = [t.cyan, t.yellow, t.blue, t.magenta, t.green, t.red];
    colors[index % colors.len()]
}

pub fn shape(id: &str) -> Icon {
    let mut hash: i32 = 0;
    for c in id.chars() {
        hash = (hash << 5).wrapping_sub(hash).wrapping_add(c as i32);
    }
    SHAPES[(hash.unsigned_abs() as usize) % SHAPES.len()]
}

fn short_words(name: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            prev_lower = false;
            continue;
        }
        if prev_lower && c.is_ascii_uppercase() && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        prev_lower = c.is_ascii_lowercase();
        word.push(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

pub fn auto_short(name: &str) -> String {
    let words = short_words(name);
    let Some(last) = words.last() else {
        let head: String = name.chars().take(3).collect();
        return if head.is_empty() {
            "?".to_string()
        } else {
            head
        };
    };
    if words.len() > 1
        && last.len() <= 3
        && last.chars().any(|c| c.is_ascii_uppercase())
        && *last == last.to_uppercase()
    {
        return last.clone();
    }
    if words.len() >= 3 {
        return words
            .iter()
            .take(3)
            .filter_map(|w| w.chars().next())
            .collect::<String>()
            .to_uppercase();
    }
    words[0].chars().take(3).collect::<String>().to_uppercase()
}

pub fn short_name(space: &Space) -> String {
    match space
        .short
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(saved) => saved.chars().take(3).collect::<String>().to_uppercase(),
        None => auto_short(&space.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn memory() -> Connection {
        nixa_store::open_memory().unwrap()
    }

    #[test]
    fn add_gives_new_ids_and_activates() {
        let mut s = Spaces::default();
        assert_eq!(add(&mut s, "/a/one"), "s1");
        assert_eq!(add(&mut s, "/a/two"), "s2");
        assert_eq!(s.active.as_deref(), Some("s2"));
        assert_eq!(s.spaces[1].name, "two");
        assert_eq!(s.spaces[1].position, 1);
        remove(&mut s, "s1");
        assert_eq!(add(&mut s, "/a/three"), "s3");
        assert_eq!(s.spaces[1].position, 2);
    }

    #[test]
    fn remove_picks_a_neighbour() {
        let mut s = Spaces::default();
        add(&mut s, "/x/a");
        add(&mut s, "/x/b");
        remove(&mut s, "s2");
        assert_eq!(s.active.as_deref(), Some("s1"));
        remove(&mut s, "s1");
        assert_eq!(s.active, None);
        let fixed = fix_active(&mut s);
        assert_eq!(fixed.added, None);
        assert!(s.spaces.is_empty());
        assert_eq!(s.active, None);
        add(&mut s, "/home");
        s.active = Some("gone".into());
        let fixed = fix_active(&mut s);
        assert!(fixed.active);
        assert_eq!(active(&s).unwrap().folder, "/home");
        assert_eq!(fix_active(&mut s), Fixed::default());
    }

    #[test]
    fn rename_a_space() {
        let mut s = Spaces::default();
        add(&mut s, "/x/a");
        assert!(rename(&mut s, "s1", "  alpha "));
        assert_eq!(s.spaces[0].name, "alpha");
        assert!(!rename(&mut s, "s1", "   "));
        assert!(!rename(&mut s, "s9", "beta"));
    }

    #[test]
    fn new_spaces_take_the_least_used_color_and_home_is_tilde() {
        let mut s = Spaces::default();
        add(&mut s, "/a/one");
        add(&mut s, "/a/two");
        assert_eq!(s.spaces[0].color, 0);
        assert_eq!(s.spaces[1].color, 1);
        s.spaces[1].color = 0;
        add(&mut s, &home());
        assert_eq!(s.spaces[2].color, 1);
        assert_eq!(s.spaces[2].name, "~");
    }

    #[test]
    fn move_to_reorders_and_renumbers() {
        let mut s = Spaces::default();
        for f in ["/a", "/b", "/c", "/d"] {
            add(&mut s, f);
        }
        assert!(move_to(&mut s, "s1", 2));
        let ids: Vec<&str> = s.spaces.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["s2", "s3", "s1", "s4"]);
        assert_eq!(
            s.spaces.iter().map(|x| x.position).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert!(move_to(&mut s, "s4", 0));
        assert_eq!(s.spaces[0].id, "s4");
        assert!(!move_to(&mut s, "s4", 0));
        assert!(!move_to(&mut s, "s9", 0));
        assert!(move_to(&mut s, "s4", 99));
        assert_eq!(s.spaces[3].id, "s4");
    }

    #[test]
    fn short_names() {
        assert_eq!(auto_short("nixa-app"), "NIX");
        assert_eq!(auto_short("nixa"), "NIX");
        assert_eq!(auto_short("myWebAgent"), "MWA");
        assert_eq!(auto_short("shop web app"), "SWA");
        assert_eq!(auto_short("web-admin"), "WEB");
        assert_eq!(auto_short("~"), "~");
        assert_eq!(auto_short(""), "?");
        assert_eq!(auto_short("nixa-UI"), "UI");
        let mut s = Spaces::default();
        add(&mut s, "/x/alpha");
        assert_eq!(short_name(&s.spaces[0]), "ALP");
        s.spaces[0].short = Some(" zz9x ".into());
        assert_eq!(short_name(&s.spaces[0]), "ZZ9");
        s.spaces[0].short = Some("  ".into());
        assert_eq!(short_name(&s.spaces[0]), "ALP");
    }

    #[test]
    fn shapes_are_stable() {
        assert_eq!(shape("s1"), shape("s1"));
        let all: std::collections::HashSet<Icon> =
            (1..40).map(|i| shape(&format!("s{i}"))).collect();
        assert!(all.len() >= 4);
        let t = nixa_ui::theme::nixa_dark();
        assert_eq!(color(&t, 0), t.cyan);
        assert_eq!(color(&t, COLOR_COUNT + 1), t.yellow);
    }

    #[test]
    fn locks_order_and_shorts_are_stored() {
        let conn = memory();
        let mut s = Spaces::default();
        for f in ["/a", "/b", "/c"] {
            let id = add(&mut s, f);
            write_space(&conn, find(&s, &id).unwrap()).unwrap();
        }
        assert!(set_locked(&mut s, "s2", true));
        assert!(!set_locked(&mut s, "s2", true));
        write_lock(&conn, "s2", true).unwrap();
        move_to(&mut s, "s3", 0);
        write_order(&conn, &s).unwrap();
        nixa_store::settings::set_setting(
            &conn,
            SHORT_KEY,
            &serde_json::json!({"s1": "ONE", "s9": "X"}),
        )
        .unwrap();
        let back = load(&conn).unwrap();
        let ids: Vec<&str> = back.spaces.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["s3", "s1", "s2"]);
        assert!(find(&back, "s2").unwrap().locked);
        assert_eq!(find(&back, "s1").unwrap().short.as_deref(), Some("ONE"));
        write_lock(&conn, "s2", false).unwrap();
        write_lock(&conn, "s3", true).unwrap();
        assert!(refresh(&conn, &mut s).unwrap());
        assert!(!find(&s, "s2").unwrap().locked);
        assert!(find(&s, "s3").unwrap().locked);
        assert!(!refresh(&conn, &mut s).unwrap());
        nixa_store::settings::set_setting(&conn, SHORT_KEY, &serde_json::json!({"s1": "NEW"}))
            .unwrap();
        assert!(refresh(&conn, &mut s).unwrap());
        assert_eq!(find(&s, "s1").unwrap().short.as_deref(), Some("NEW"));
        assert!(!refresh(&conn, &mut s).unwrap());
        let fixed = close(&mut s, "s3");
        assert_eq!(fixed, Fixed::default());
        assert_eq!(s.spaces.len(), 2);
        close(&mut s, "s1");
        close(&mut s, "s2");
        assert!(s.spaces.is_empty());
        assert_eq!(s.active, None);
        assert!(s.spaces.iter().all(|x| x.name != "~"));
    }

    #[test]
    fn a_space_made_outside_the_window_is_kept() {
        let conn = memory();
        let mut s = load(&conn).unwrap();
        let outside = SpacePatch {
            name: Some("shop".into()),
            folder: Some("/w/shop".into()),
            ..SpacePatch::default()
        };
        store::set_space(&conn, "s1", &outside).unwrap();
        assert_eq!(add_saved(&conn, &mut s, "/w/farm").unwrap(), "s2");
        assert_eq!(store::get_space(&conn, "s1").unwrap().unwrap().folder, "/w/shop");
        assert_eq!(store::get_space(&conn, "s2").unwrap().unwrap().folder, "/w/farm");
        let ids: Vec<&str> = s.spaces.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["s1", "s2"]);
        assert_eq!(s.active.as_deref(), Some("s2"));
        assert_eq!(add_saved(&conn, &mut s, "/w/shop").unwrap(), "s1");
        assert_eq!(s.spaces.len(), 2);
        assert_eq!(store::active_space(&conn).unwrap().as_deref(), Some("s1"));
        assert!(!merge_saved(&conn, &mut s).unwrap());
    }

    #[test]
    fn refresh_takes_in_spaces_the_brain_added_and_out_the_ones_it_removed() {
        let conn = memory();
        let mut s = load(&conn).unwrap();
        assert_eq!(add_saved(&conn, &mut s, "/w/shop").unwrap(), "s1");
        // The brain adds one behind the window (nixa_spaces add).
        let added = SpacePatch {
            name: Some("farm".into()),
            folder: Some("/w/farm".into()),
            position: Some(1),
            ..SpacePatch::default()
        };
        store::set_space(&conn, "s7", &added).unwrap();
        assert!(refresh(&conn, &mut s).unwrap());
        let ids: Vec<&str> = s.spaces.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["s1", "s7"]);
        assert_eq!(find(&s, "s7").unwrap().folder, "/w/farm");
        assert!(!refresh(&conn, &mut s).unwrap());
        // The brain removes the active one behind the window (nixa_spaces remove).
        store::remove_space(&conn, "s1").unwrap();
        assert!(refresh(&conn, &mut s).unwrap());
        let ids: Vec<&str> = s.spaces.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["s7"]);
        assert_eq!(s.active.as_deref(), Some("s7"), "the active space moves to what is left");
    }

    #[test]
    fn a_failed_add_changes_nothing() {
        let conn = memory();
        let mut s = load(&conn).unwrap();
        assert_eq!(add_saved(&conn, &mut s, "/w/shop").unwrap(), "s1");
        let outside = SpacePatch {
            name: Some("farm".into()),
            folder: Some("/w/farm".into()),
            ..SpacePatch::default()
        };
        store::set_space(&conn, "s2", &outside).unwrap();
        let before = s.clone();
        let saved = load(&conn).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER no_new BEFORE INSERT ON spaces BEGIN SELECT RAISE(ABORT, 'full'); END;",
        )
        .unwrap();
        assert!(add_saved(&conn, &mut s, "/w/barn").is_err());
        assert_eq!(s, before);
        assert_eq!(load(&conn).unwrap(), saved);
        conn.execute_batch("DROP TRIGGER no_new;").unwrap();
        conn.execute_batch(
            "CREATE TRIGGER no_active BEFORE INSERT ON settings BEGIN SELECT RAISE(ABORT, 'full'); END;
             CREATE TRIGGER no_active_set BEFORE UPDATE ON settings BEGIN SELECT RAISE(ABORT, 'full'); END;",
        )
        .unwrap();
        assert!(add_saved(&conn, &mut s, "/w/barn").is_err());
        assert_eq!(s, before);
        assert_eq!(load(&conn).unwrap(), saved);
        assert!(add_saved(&conn, &mut s, "/w/farm").is_err());
        assert_eq!(s, before);
    }

    #[test]
    fn short_folder_uses_tilde() {
        assert_eq!(short_folder("/Users/a", "/Users/a"), "~");
        assert_eq!(short_folder("/Users/a/p", "/Users/a"), "~/p");
        assert_eq!(short_folder("/Users/ab", "/Users/a"), "/Users/ab");
        assert_eq!(folder_name("/"), "/");
    }

    #[test]
    fn the_store_keeps_every_change() {
        let conn = memory();
        let mut s = load(&conn).unwrap();
        assert_eq!(s, Spaces::default());
        assert_eq!(fix_active(&mut s).added, None);
        assert!(s.spaces.is_empty());
        let fixed = Fixed { added: Some(add(&mut s, "/home/me")), active: true };
        write_fixed(&conn, &s, &fixed).unwrap();
        assert_eq!(load(&conn).unwrap(), s);
        add(&mut s, "/p/two");
        write_space(&conn, find(&s, "s2").unwrap()).unwrap();
        write_active(&conn, &s).unwrap();
        rename(&mut s, "s2", "second");
        s.spaces[1].color = 3;
        write_space(&conn, find(&s, "s2").unwrap()).unwrap();
        let back = load(&conn).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.active.as_deref(), Some("s2"));
        assert_eq!(back.spaces[1].name, "second");
        remove(&mut s, "s2");
        delete(&conn, "s2").unwrap();
        write_active(&conn, &s).unwrap();
        assert_eq!(load(&conn).unwrap(), s);
        assert_eq!(s.active.as_deref(), Some("s1"));
    }

    #[test]
    fn the_old_file_is_imported_once() {
        let conn = memory();
        let dir = std::env::temp_dir().join(format!("nixa-app-import-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(store::OLD_FILE);
        assert_eq!(import(&conn, &path).unwrap(), 0);
        fs::write(
            &path,
            r#"{"spaces":[{"id":"s1","name":"one","folder":"/a/one","color":0},{"id":"s4","name":"four","folder":"/a/four","color":3}],"active":"s4"}"#,
        )
        .unwrap();
        assert_eq!(import(&conn, &path).unwrap(), 2);
        assert!(!path.exists());
        assert!(path.with_extension("json.imported").exists());
        let mut s = load(&conn).unwrap();
        assert_eq!(s.spaces.len(), 2);
        assert_eq!(s.active.as_deref(), Some("s4"));
        assert_eq!(s.spaces[1].color, 3);
        assert_eq!(add(&mut s, "/a/five"), "s5");
        assert_eq!(s.spaces[2].position, 2);
        fs::write(&path, "{\"spaces\": [").unwrap();
        assert!(import(&conn, &path).is_err());
        assert!(path.exists());
        fs::remove_dir_all(&dir).ok();
    }
}
