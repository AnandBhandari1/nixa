use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

pub const PER_FOLDER: usize = 500;
pub const TOTAL: usize = 5000;
pub const MAX_DEPTH: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub path: String,
    pub name: String,
    pub dir: bool,
    pub depth: usize,
    pub open: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub entries: HashMap<String, Vec<Entry>>,
    pub open: BTreeSet<String>,
    pub limited: bool,
}

pub fn join(root: &str, path: &str) -> String {
    let root = root.trim_end_matches('/');
    if path.is_empty() {
        if root.is_empty() {
            "/".to_string()
        } else {
            root.to_string()
        }
    } else {
        format!("{root}/{path}")
    }
}

pub fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |at| &path[..at])
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
}

pub fn child(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

pub fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub fn sort(list: &mut [Entry]) {
    list.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

pub fn read_folder(folder: &Path) -> std::io::Result<(Vec<Entry>, bool)> {
    let mut list = Vec::new();
    let mut more = false;
    for item in fs::read_dir(folder)? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        if list.len() >= PER_FOLDER {
            more = true;
            break;
        }
        let dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
        list.push(Entry { name, dir });
    }
    sort(&mut list);
    Ok((list, more))
}

pub fn used_except(tree: &Tree, path: &str) -> usize {
    tree.entries
        .iter()
        .filter(|(p, _)| p.as_str() != path)
        .map(|(_, list)| list.len())
        .sum()
}

pub fn put(tree: &mut Tree, path: &str, mut list: Vec<Entry>, more: bool) {
    let used = used_except(tree, path);
    let room = TOTAL.saturating_sub(used).min(PER_FOLDER);
    if more || list.len() > room {
        tree.limited = true;
    }
    list.truncate(room);
    tree.entries.insert(path.to_string(), list);
}

pub fn load(tree: &mut Tree, root: &str, path: &str) -> std::io::Result<()> {
    let (list, more) = read_folder(Path::new(&join(root, path)))?;
    put(tree, path, list, more);
    Ok(())
}

pub fn toggle(tree: &mut Tree, path: &str) -> bool {
    if tree.open.contains(path) {
        let inner = format!("{path}/");
        let gone: Vec<String> = tree
            .open
            .iter()
            .filter(|p| p.as_str() == path || p.starts_with(&inner))
            .cloned()
            .collect();
        for p in gone {
            tree.open.remove(&p);
            tree.entries.remove(&p);
        }
        false
    } else {
        tree.open.insert(path.to_string());
        true
    }
}

pub fn rows(tree: &Tree) -> Vec<Row> {
    let mut out = Vec::new();
    walk(tree, "", 0, &mut out);
    out
}

fn walk(tree: &Tree, prefix: &str, depth: usize, out: &mut Vec<Row>) {
    if depth > MAX_DEPTH {
        return;
    }
    for entry in tree.entries.get(prefix).map(Vec::as_slice).unwrap_or(&[]) {
        if out.len() >= TOTAL {
            return;
        }
        let path = child(prefix, &entry.name);
        let open = entry.dir && tree.open.contains(&path);
        out.push(Row {
            path: path.clone(),
            name: entry.name.clone(),
            dir: entry.dir,
            depth,
            open,
        });
        if open {
            walk(tree, &path, depth + 1, out);
        }
    }
}

pub fn folders(tree: &Tree) -> Vec<String> {
    std::iter::once(String::new())
        .chain(tree.open.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nixa-files-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn folders_first_then_names_without_case() {
        let dir = temp("sort");
        fs::create_dir_all(dir.join("zeta")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join("B.txt"), "x").unwrap();
        fs::write(dir.join("a.txt"), "x").unwrap();
        fs::write(dir.join(".env"), "x").unwrap();
        let (list, more) = read_folder(&dir).unwrap();
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["zeta", ".env", "a.txt", "B.txt"]);
        assert!(!more);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_big_folder_is_cut_and_marked() {
        let dir = temp("big");
        for n in 0..PER_FOLDER + 5 {
            fs::write(dir.join(format!("f{n:04}")), "").unwrap();
        }
        let mut tree = Tree::default();
        load(&mut tree, &dir.display().to_string(), "").unwrap();
        assert_eq!(tree.entries[""].len(), PER_FOLDER);
        assert!(tree.limited);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_total_cap_holds_over_folders() {
        let mut tree = Tree::default();
        for n in 0..10 {
            let list = (0..PER_FOLDER)
                .map(|i| Entry {
                    name: format!("{n}-{i}"),
                    dir: false,
                })
                .collect();
            put(&mut tree, &format!("d{n}"), list, false);
        }
        let all: usize = tree.entries.values().map(Vec::len).sum();
        assert_eq!(all, TOTAL);
        assert!(!tree.limited);
        put(
            &mut tree,
            "d10",
            vec![Entry {
                name: "x".into(),
                dir: false,
            }],
            false,
        );
        assert!(tree.entries["d10"].is_empty());
        assert!(tree.limited);
    }

    #[test]
    fn closing_a_folder_closes_its_children() {
        let mut tree = Tree::default();
        assert!(toggle(&mut tree, "src"));
        assert!(toggle(&mut tree, "src/app"));
        assert!(toggle(&mut tree, "srcx"));
        tree.entries.insert("src/app".into(), vec![]);
        assert!(!toggle(&mut tree, "src"));
        assert_eq!(
            tree.open.iter().cloned().collect::<Vec<_>>(),
            vec!["srcx".to_string()]
        );
        assert!(!tree.entries.contains_key("src/app"));
    }

    #[test]
    fn rows_nest_open_folders() {
        let mut tree = Tree::default();
        tree.entries.insert(
            "".into(),
            vec![
                Entry {
                    name: "src".into(),
                    dir: true,
                },
                Entry {
                    name: "a.rs".into(),
                    dir: false,
                },
            ],
        );
        tree.entries.insert(
            "src".into(),
            vec![Entry {
                name: "main.rs".into(),
                dir: false,
            }],
        );
        assert_eq!(rows(&tree).len(), 2);
        tree.open.insert("src".into());
        let list = rows(&tree);
        let paths: Vec<(&str, usize)> = list.iter().map(|r| (r.path.as_str(), r.depth)).collect();
        assert_eq!(paths, vec![("src", 0), ("src/main.rs", 1), ("a.rs", 0)]);
        assert!(list[0].open);
    }

    #[test]
    fn relative_paths_stay_inside() {
        assert!(safe_relative("src/main.rs"));
        assert!(!safe_relative("/etc/passwd"));
        assert!(!safe_relative("../x"));
        assert!(!safe_relative("a//b"));
        assert!(!safe_relative("a/./b"));
        assert!(!safe_relative(""));
        assert_eq!(join("/r/", "a/b"), "/r/a/b");
        assert_eq!(join("/r", ""), "/r");
        assert_eq!(parent("a/b/c"), "a/b");
        assert_eq!(parent("a"), "");
        assert_eq!(file_name("a/b/c.rs"), "c.rs");
    }
}
