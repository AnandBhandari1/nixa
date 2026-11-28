use crate::args::usage;
use crate::ctx::{self, Ctx};
use crate::help;
use anyhow::{Result, anyhow};
use nixa_store::Connection;
use nixa_store::spaces::{self, Space, SpacePatch};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const COLOR_COUNT: i64 = 6;

#[derive(Serialize)]
struct Listed {
    spaces: Vec<Space>,
    active: Option<String>,
}

#[derive(Serialize)]
struct Added {
    space: Space,
    added: bool,
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::SPACE.as_bytes())?;
        return Ok(2);
    };
    let asks_help = matches!(sub.as_str(), "help" | "-h" | "--help")
        || rest.first().is_some_and(|a| a == "-h" || a == "--help");
    if asks_help {
        out.write_all(help::SPACE.as_bytes())?;
        return Ok(0);
    }
    match sub.as_str() {
        "add" | "new" => add(ctx, rest, out),
        "ls" | "list" => list(ctx, rest, out),
        other => Err(usage(format!("unknown command nixa space {other}"))),
    }
}

fn open() -> Result<Connection> {
    nixa_store::open_default().map_err(|e| anyhow!("the space store did not open: {e:#}"))
}

fn print<T: Serialize>(ctx: &Ctx, out: &mut dyn Write, value: &T, text: &str) -> Result<i32> {
    if ctx.json {
        writeln!(out, "{}", serde_json::to_string(value)?)?;
    } else {
        writeln!(out, "{text}")?;
    }
    Ok(0)
}

pub fn folder_of(word: &str) -> Result<PathBuf> {
    if word.trim().is_empty() {
        return Err(usage("nixa space add needs a folder"));
    }
    let path = std::fs::canonicalize(word).map_err(|e| anyhow!("The folder {word} could not be found: {e}"))?;
    if !path.is_dir() {
        return Err(anyhow!("{} is not a folder.", path.display()));
    }
    Ok(path)
}

pub fn next_id(all: &[Space]) -> String {
    let top = all
        .iter()
        .filter_map(|s| s.space_id.strip_prefix('s')?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("s{}", top + 1)
}

pub fn least_used_color(all: &[Space]) -> i64 {
    let uses = |c: i64| all.iter().filter(|s| s.color == c).count();
    (0..COLOR_COUNT).min_by_key(|&c| uses(c)).unwrap_or(0)
}

pub fn default_name(folder: &Path, home: Option<&Path>) -> String {
    if home.is_some_and(|h| h == folder) {
        return "~".to_string();
    }
    folder
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| folder.display().to_string())
}

fn home() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().filter(|h| !h.trim().is_empty())?;
    std::fs::canonicalize(&home).ok().or(Some(PathBuf::from(home)))
}

pub fn add_space(conn: &Connection, folder: &Path, name: Option<&str>) -> Result<(Space, bool)> {
    let text = folder.to_string_lossy().to_string();
    let tx = conn.unchecked_transaction()?;
    if let Some(found) = spaces::space_for_folder(&tx, &text)? {
        return Ok((found, false));
    }
    let all = spaces::list_spaces(&tx)?;
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_name(folder, home().as_deref()));
    let patch = SpacePatch {
        name: Some(name),
        folder: Some(text),
        color: Some(least_used_color(&all)),
        position: Some(all.iter().map(|s| s.position + 1).max().unwrap_or(0)),
        ..SpacePatch::default()
    };
    let space = spaces::set_space(&tx, &next_id(&all), &patch)?;
    if spaces::active_space(&tx)?.is_none() {
        spaces::set_active_space(&tx, Some(&space.space_id))?;
    }
    tx.commit()?;
    Ok((space, true))
}

pub fn space_line(space: &Space, active: Option<&str>) -> String {
    let mut line = format!("{}  {}  {}", space.space_id, space.name, space.folder);
    if active == Some(space.space_id.as_str()) {
        line.push_str("  (active)");
    }
    if space.locked {
        line.push_str("  (locked)");
    }
    line
}

fn add(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--name"], false)?;
    let name = p.value(&["--name"]);
    p.finish(1, false)?;
    let word = p.pos.first().ok_or_else(|| usage("nixa space add needs a folder"))?;
    let folder = folder_of(word)?;
    let conn = open()?;
    let (space, added) = add_space(&conn, &folder, name.as_deref())?;
    let text = if added {
        format!("Added space {}.", space_line(&space, None))
    } else {
        format!("This folder is already the space {}.", space_line(&space, None))
    };
    print(&ctx, out, &Added { space, added }, &text)
}

fn list(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let conn = open()?;
    let all = spaces::list_spaces(&conn)?;
    let active = spaces::active_space(&conn)?;
    let text = if all.is_empty() {
        "No spaces yet. Add one with: nixa space add FOLDER".to_string()
    } else {
        all.iter().map(|s| space_line(s, active.as_deref())).collect::<Vec<_>>().join("\n")
    };
    print(&ctx, out, &Listed { spaces: all, active }, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nixa-cli-space-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
    }

    #[test]
    fn add_gives_ids_colors_and_keeps_one_per_folder() {
        let conn = nixa_store::open_memory().unwrap();
        let root = temp("add");
        let shop = root.join("shop");
        let farm = root.join("farm");
        std::fs::create_dir_all(&shop).unwrap();
        std::fs::create_dir_all(&farm).unwrap();
        let (one, added) = add_space(&conn, &shop, None).unwrap();
        assert!(added);
        assert_eq!((one.space_id.as_str(), one.name.as_str(), one.color, one.position), ("s1", "shop", 0, 0));
        assert_eq!(spaces::active_space(&conn).unwrap().as_deref(), Some("s1"));
        let (two, _) = add_space(&conn, &farm, Some(" Farm work ")).unwrap();
        assert_eq!((two.space_id.as_str(), two.name.as_str(), two.color, two.position), ("s2", "Farm work", 1, 1));
        assert_eq!(spaces::active_space(&conn).unwrap().as_deref(), Some("s1"));
        let (again, added) = add_space(&conn, &shop, Some("other")).unwrap();
        assert!(!added);
        assert_eq!(again.space_id, "s1");
        assert_eq!(spaces::list_spaces(&conn).unwrap().len(), 2);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_folder_with_spaces_keeps_its_name() {
        let conn = nixa_store::open_memory().unwrap();
        let root = temp("blank");
        std::fs::create_dir_all(root.join("shop")).unwrap();
        std::fs::create_dir_all(root.join("shop ")).unwrap();
        let word = format!("{}/shop ", root.display());
        let folder = folder_of(&word).unwrap();
        assert_eq!(folder, root.join("shop "));
        let (one, _) = add_space(&conn, &folder, None).unwrap();
        assert_eq!(one.folder, word);
        assert_eq!(one.name, "shop");
        let (two, added) = add_space(&conn, &folder_of(&format!("{}/shop", root.display())).unwrap(), None).unwrap();
        assert!(added);
        assert_eq!(two.space_id, "s2");
        assert_eq!(spaces::get_space(&conn, "s1").unwrap().unwrap().folder, word);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_space_saved_by_its_link_is_not_added_again() {
        let conn = nixa_store::open_memory().unwrap();
        let root = temp("link");
        std::fs::create_dir_all(root.join("proj")).unwrap();
        let link = root.join("proj-link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(root.join("proj"), &link).unwrap();
        let saved = link.to_string_lossy().to_string();
        spaces::set_space(&conn, "s1", &SpacePatch { name: Some("proj".into()), folder: Some(saved.clone()), ..SpacePatch::default() }).unwrap();
        let folder = folder_of(&saved).unwrap();
        assert_eq!(folder, root.join("proj"));
        let (found, added) = add_space(&conn, &folder, None).unwrap();
        assert!(!added);
        assert_eq!(found.space_id, "s1");
        assert_eq!(found.folder, saved);
        assert_eq!(spaces::list_spaces(&conn).unwrap().len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_root_folder_is_added_once() {
        let conn = nixa_store::open_memory().unwrap();
        let (one, added) = add_space(&conn, Path::new("/"), None).unwrap();
        assert!(added);
        assert_eq!((one.space_id.as_str(), one.name.as_str()), ("s1", "/"));
        let (again, added) = add_space(&conn, Path::new("/"), None).unwrap();
        assert!(!added);
        assert_eq!(again.space_id, "s1");
        assert_eq!(spaces::list_spaces(&conn).unwrap().len(), 1);
    }

    #[test]
    fn names_ids_and_lines() {
        assert_eq!(default_name(Path::new("/w/shop"), Some(Path::new("/w"))), "shop");
        assert_eq!(default_name(Path::new("/w"), Some(Path::new("/w"))), "~");
        assert_eq!(default_name(Path::new("/"), None), "/");
        assert_eq!(next_id(&[]), "s1");
        assert!(folder_of(" ").is_err());
        assert!(folder_of("/no/such/folder/for/nixa").is_err());
        let conn = nixa_store::open_memory().unwrap();
        let made = spaces::set_space(&conn, "s9", &SpacePatch { name: Some("shop".into()), folder: Some("/w/shop".into()), locked: Some(true), ..SpacePatch::default() }).unwrap();
        assert_eq!(next_id(std::slice::from_ref(&made)), "s10");
        assert_eq!(space_line(&made, Some("s9")), "s9  shop  /w/shop  (active)  (locked)");
    }
}
