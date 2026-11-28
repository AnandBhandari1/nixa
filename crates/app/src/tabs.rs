use crate::layout::{self, Node};
use nixa_protocol::{TermId, TermInfo, paths};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const LABEL_SPACE: &str = "space";
pub const LABEL_TAB: &str = "tab";
pub const LABEL_BRAIN_SPACE: &str = "nixa.space";
pub const LAYOUT_FILE: &str = "tabs.json";

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: String,
    pub space: String,
    pub root: Node,
    pub focus: TermId,
    pub zoom: Option<TermId>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tabs {
    pub list: Vec<Tab>,
    pub active: BTreeMap<String, String>,
    pub pending: Vec<String>,
    pub next: u64,
    pub saved: BTreeMap<String, Node>,
    pub numbers: BTreeMap<TermId, u32>,
    pub hold: BTreeSet<TermId>,
}

fn tab_number(id: &str) -> Option<u64> {
    id.strip_prefix('t')?.parse().ok()
}

pub fn layout_path() -> PathBuf {
    paths::data_dir().join(LAYOUT_FILE)
}

pub fn load_layouts(path: &Path) -> BTreeMap<String, Node> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_layouts(path: &Path, layouts: &BTreeMap<String, Node>) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        paths::ensure_private_dir(dir)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(layouts)? + "\n";
    let written = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| {
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
            f.write_all(text.as_bytes())?;
            f.sync_all()
        })
        .and_then(|_| fs::rename(&tmp, path));
    if written.is_err() {
        fs::remove_file(&tmp).ok();
    }
    written?;
    Ok(())
}

pub fn persist(tabs: &mut Tabs, path: &Path) -> anyhow::Result<bool> {
    let now = layouts(tabs);
    if now == tabs.saved {
        return Ok(false);
    }
    save_layouts(path, &now)?;
    tabs.saved = now;
    Ok(true)
}

pub fn layouts(tabs: &Tabs) -> BTreeMap<String, Node> {
    tabs.list
        .iter()
        .map(|t| (t.id.clone(), t.root.clone()))
        .collect()
}

fn apply_saved(tabs: &mut Tabs, tab_id: &str) {
    let Some(saved) = tabs.saved.get(tab_id).cloned() else {
        return;
    };
    let Some(tab) = find_mut(tabs, tab_id) else {
        return;
    };
    let live: BTreeSet<TermId> = layout::leaves(&tab.root).into_iter().collect();
    let Some(mut root) = layout::prune(saved, &live) else {
        return;
    };
    for id in layout::leaves(&tab.root) {
        if !layout::contains(&root, &id) {
            root = layout::append(root, &id);
        }
    }
    tab.root = root;
}

pub fn new_id(tabs: &mut Tabs) -> String {
    tabs.next += 1;
    format!("t{}", tabs.next)
}

pub fn placed(tabs: &Tabs) -> BTreeSet<TermId> {
    tabs.list
        .iter()
        .flat_map(|t| layout::leaves(&t.root))
        .collect()
}

pub fn find<'a>(tabs: &'a Tabs, id: &str) -> Option<&'a Tab> {
    tabs.list.iter().find(|t| t.id == id)
}

pub fn find_mut<'a>(tabs: &'a mut Tabs, id: &str) -> Option<&'a mut Tab> {
    tabs.list.iter_mut().find(|t| t.id == id)
}

pub fn unpend(tabs: &mut Tabs, tab: &str) {
    if let Some(i) = tabs.pending.iter().position(|p| p == tab) {
        tabs.pending.remove(i);
    }
}

pub fn place(tabs: &mut Tabs, id: TermId, space: &str, tab_id: &str) {
    if let Some(n) = tab_number(tab_id) {
        tabs.next = tabs.next.max(n);
    }
    if let Some(tab) = find_mut(tabs, tab_id) {
        if !layout::contains(&tab.root, &id) {
            tab.root = layout::append(tab.root.clone(), &id);
        }
        return;
    }
    tabs.list.push(Tab {
        id: tab_id.to_string(),
        space: space.to_string(),
        root: Node::Leaf(id.clone()),
        focus: id,
        zoom: None,
    });
    tabs.active
        .entry(space.to_string())
        .or_insert_with(|| tab_id.to_string());
}

pub fn forget(tabs: &mut Tabs, tab_id: &str) {
    let Some(index) = tabs.list.iter().position(|t| t.id == tab_id) else {
        return;
    };
    let tab = tabs.list.remove(index);
    if tabs.active.get(&tab.space).map(String::as_str) != Some(tab_id) {
        return;
    }
    let before = tabs.list[..index]
        .iter()
        .filter(|t| t.space == tab.space)
        .count();
    let same: Vec<String> = tabs
        .list
        .iter()
        .filter(|t| t.space == tab.space)
        .map(|t| t.id.clone())
        .collect();
    match same.get(before.min(same.len().saturating_sub(1))) {
        Some(next) => {
            tabs.active.insert(tab.space, next.clone());
        }
        None => {
            tabs.active.remove(&tab.space);
        }
    }
}

pub fn drop_leaf(tabs: &mut Tabs, id: &TermId) {
    let mut empty = Vec::new();
    for tab in &mut tabs.list {
        if !layout::contains(&tab.root, id) {
            continue;
        }
        let next = layout::neighbour(&tab.root, id);
        match layout::remove(tab.root.clone(), id) {
            Some(root) => {
                if &tab.focus == id {
                    tab.focus = next.unwrap_or_else(|| layout::leaves(&root)[0].clone());
                }
                tab.root = root;
            }
            None => empty.push(tab.id.clone()),
        }
        if tab.zoom.as_ref() == Some(id) {
            tab.zoom = None;
        }
    }
    for tab in empty {
        forget(tabs, &tab);
    }
    tabs.numbers.remove(id);
}

pub fn zoomed(tab: &Tab) -> Option<&TermId> {
    tab.zoom.as_ref().filter(|z| {
        **z == tab.focus && layout::contains(&tab.root, z)
    })
}

pub fn toggle_zoom(tabs: &mut Tabs, tab_id: &str, id: &TermId) -> bool {
    let Some(tab) = find_mut(tabs, tab_id) else {
        return false;
    };
    if zoomed(tab) == Some(id) {
        tab.zoom = None;
        return true;
    }
    if !layout::contains(&tab.root, id) {
        return false;
    }
    tab.zoom = Some(id.clone());
    tab.focus = id.clone();
    true
}

fn settle_zoom(tabs: &mut Tabs) {
    for tab in &mut tabs.list {
        if tab.zoom.is_some() && zoomed(tab).is_none() {
            tab.zoom = None;
        }
    }
}

pub fn resize(
    tabs: &mut Tabs,
    tab_id: &str,
    path: &[usize],
    edge: usize,
    at: f32,
    length: f32,
    gap: f32,
) -> bool {
    find_mut(tabs, tab_id)
        .is_some_and(|tab| layout::resize(&mut tab.root, path, edge, at, length, gap))
}

pub fn assign_numbers(before: &BTreeMap<TermId, u32>, live: &[TermId]) -> BTreeMap<TermId, u32> {
    let mut kept = BTreeMap::new();
    let mut used = BTreeSet::new();
    let mut owners: Vec<&TermId> = live.iter().collect();
    owners.sort_by_key(|id| before.get(*id).copied() != id.own_number().and_then(|n| u32::try_from(n).ok()));
    for id in owners {
        if let Some(n) = id.own_number().and_then(|n| u32::try_from(n).ok())
            && used.insert(n)
        {
            kept.insert(id.clone(), n);
        }
    }
    for id in live {
        if kept.contains_key(id) {
            continue;
        }
        if let Some(n) = before.get(id).copied()
            && n > 0
            && used.insert(n)
        {
            kept.insert(id.clone(), n);
        }
    }
    let mut next = 1;
    for id in live {
        if kept.contains_key(id) {
            continue;
        }
        while used.contains(&next) {
            next += 1;
        }
        kept.insert(id.clone(), next);
        used.insert(next);
    }
    kept
}

pub fn renumber(tabs: &mut Tabs, terms: &BTreeMap<TermId, TermInfo>) {
    let mut live: Vec<TermId> = Vec::new();
    let shown = tabs.list.iter().flat_map(|t| layout::leaves(&t.root));
    let still_running = tabs
        .numbers
        .keys()
        .filter(|id| terms.get(*id).is_some_and(|t| t.running))
        .cloned();
    for id in shown.chain(still_running) {
        if !live.contains(&id) {
            live.push(id);
        }
    }
    tabs.numbers = assign_numbers(&tabs.numbers, &live);
}

pub fn sync(tabs: &mut Tabs, terms: &BTreeMap<TermId, TermInfo>, space_ok: impl Fn(&str) -> bool) {
    let gone: Vec<TermId> = placed(tabs)
        .into_iter()
        .filter(|id| !terms.contains_key(id))
        .collect();
    for id in gone {
        drop_leaf(tabs, &id);
    }
    tabs.hold.retain(|id| terms.contains_key(id));
    let now = placed(tabs);
    let held = &tabs.hold;
    let adds: Vec<(TermId, String, String)> = terms
        .values()
        .filter(|t| !now.contains(&t.id) && !held.contains(&t.id))
        .filter_map(|t| {
            Some((
                t.id.clone(),
                t.labels.get(LABEL_SPACE)?.clone(),
                t.labels.get(LABEL_TAB)?.clone(),
            ))
        })
        .filter(|(_, space, tab)| space_ok(space) && !tabs.pending.contains(tab))
        .collect();
    let mut touched = BTreeSet::new();
    for (id, space, tab) in adds {
        place(tabs, id, &space, &tab);
        touched.insert(tab);
    }
    for tab in touched {
        apply_saved(tabs, &tab);
    }
    settle_zoom(tabs);
    renumber(tabs, terms);
}

pub fn tab_of<'a>(tabs: &'a Tabs, id: &TermId) -> Option<&'a Tab> {
    tabs.list.iter().find(|t| layout::contains(&t.root, id))
}

pub fn place_beside(tabs: &mut Tabs, parent: &TermId, id: &TermId) -> Option<(String, String)> {
    if let Some(tab) = tab_of(tabs, id) {
        return Some((tab.id.clone(), tab.space.clone()));
    }
    let tab_id = tab_of(tabs, parent)?.id.clone();
    let tab = find_mut(tabs, &tab_id)?;
    tab.root = layout::split(tab.root.clone(), parent, id, layout::Axis::Row);
    tab.focus = id.clone();
    tab.zoom = None;
    Some((tab.id.clone(), tab.space.clone()))
}

pub fn step(tabs: &mut Tabs, space: &str, step: isize) {
    let ids: Vec<String> = tabs
        .list
        .iter()
        .filter(|t| t.space == space)
        .map(|t| t.id.clone())
        .collect();
    if ids.is_empty() {
        return;
    }
    let now = tabs
        .active
        .get(space)
        .and_then(|a| ids.iter().position(|i| i == a))
        .unwrap_or(0) as isize;
    let next = (now + step).rem_euclid(ids.len() as isize) as usize;
    tabs.active.insert(space.to_string(), ids[next].clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Axis;

    fn term(n: u64, space: &str, tab: &str) -> TermInfo {
        let mut labels = BTreeMap::new();
        labels.insert(LABEL_SPACE.to_string(), space.to_string());
        labels.insert(LABEL_TAB.to_string(), tab.to_string());
        TermInfo {
            id: TermId::from_number(n),
            name: String::new(),
            labels,
            parent: None,
            children: Vec::new(),
            cmd: Vec::new(),
            cwd: "/".into(),
            pid: None,
            running: true,
            exit_code: None,
            status: None,
            status_note: None,
            keep: false,
            draft: false,
            title: None,
            rows: 24,
            cols: 80,
            created_ms: 0,
            seq: 0,
        }
    }

    fn terms(list: Vec<TermInfo>) -> BTreeMap<TermId, TermInfo> {
        list.into_iter().map(|t| (t.id.clone(), t)).collect()
    }

    #[test]
    fn a_labelled_child_joins_its_tab_and_a_bare_child_does_not() {
        let mut tabs = Tabs::default();
        let mut kid = term(2, "s1", "t1");
        kid.parent = Some(TermId::from_number(1));
        let mut bare = term(3, "s1", "t1");
        bare.parent = Some(TermId::from_number(1));
        bare.labels.clear();
        let all = terms(vec![term(1, "s1", "t1"), kid, bare]);
        sync(&mut tabs, &all, |_| true);
        assert_eq!(tabs.list.len(), 1);
        assert_eq!(
            layout::leaves(&tabs.list[0].root),
            vec![TermId::from_number(1), TermId::from_number(2)]
        );
    }

    #[test]
    fn a_held_terminal_is_not_placed_from_its_labels() {
        let mut tabs = Tabs::default();
        let all = terms(vec![term(1, "s1", "t1"), term(2, "s1", "t1")]);
        tabs.hold.insert(TermId::from_number(2));
        sync(&mut tabs, &all, |_| true);
        assert_eq!(layout::leaves(&tabs.list[0].root), vec![TermId::from_number(1)]);
        tabs.hold.clear();
        sync(&mut tabs, &all, |_| true);
        assert_eq!(layout::leaves(&tabs.list[0].root).len(), 2);
        tabs.hold.insert(TermId::from_number(9));
        sync(&mut tabs, &all, |_| true);
        assert!(tabs.hold.is_empty());
    }

    #[test]
    fn place_beside_splits_the_parent_tab_once() {
        let mut tabs = Tabs::default();
        sync(&mut tabs, &terms(vec![term(1, "s1", "t1")]), |_| true);
        let (parent, kid) = (TermId::from_number(1), TermId::from_number(7));
        assert_eq!(
            place_beside(&mut tabs, &parent, &kid),
            Some(("t1".to_string(), "s1".to_string()))
        );
        assert_eq!(layout::leaves(&tabs.list[0].root), vec![parent.clone(), kid.clone()]);
        assert_eq!(tabs.list[0].focus, kid);
        assert!(place_beside(&mut tabs, &parent, &kid).is_some());
        assert_eq!(layout::leaves(&tabs.list[0].root).len(), 2);
        assert_eq!(place_beside(&mut tabs, &TermId::from_number(9), &TermId::from_number(8)), None);
        drop_leaf(&mut tabs, &kid);
        assert_eq!(layout::leaves(&tabs.list[0].root), vec![parent]);
    }

    #[test]
    fn labels_rebuild_tabs() {
        let mut tabs = Tabs::default();
        let all = terms(vec![
            term(1, "s1", "t1"),
            term(2, "s1", "t1"),
            term(3, "s1", "t4"),
            term(4, "s9", "t2"),
        ]);
        sync(&mut tabs, &all, |s| s == "s1");
        assert_eq!(tabs.list.len(), 2);
        assert_eq!(layout::leaves(&tabs.list[0].root).len(), 2);
        assert_eq!(tabs.active.get("s1").map(String::as_str), Some("t1"));
        assert_eq!(new_id(&mut tabs), "t5");
    }

    #[test]
    fn saved_layout_brings_back_a_column_split() {
        let mut before = Tabs::default();
        let all = terms(vec![term(1, "s1", "t1"), term(2, "s1", "t1")]);
        sync(&mut before, &all, |_| true);
        let t1 = find_mut(&mut before, "t1").unwrap();
        t1.root = layout::column(vec![
            Node::Leaf(TermId::from_number(1)),
            Node::Leaf(TermId::from_number(2)),
        ]);
        let dir = std::env::temp_dir().join(format!("nixa-app-tabs-{}", std::process::id()));
        let path = dir.join(LAYOUT_FILE);
        save_layouts(&path, &layouts(&before)).unwrap();
        let mut after = Tabs {
            saved: load_layouts(&path),
            ..Tabs::default()
        };
        sync(&mut after, &all, |_| true);
        assert_eq!(
            find(&after, "t1").unwrap().root,
            find(&before, "t1").unwrap().root
        );
        let more = terms(vec![
            term(1, "s1", "t1"),
            term(2, "s1", "t1"),
            term(3, "s1", "t1"),
        ]);
        sync(&mut after, &more, |_| true);
        assert_eq!(
            layout::leaves(&find(&after, "t1").unwrap().root),
            vec![
                TermId::from_number(1),
                TermId::from_number(2),
                TermId::from_number(3)
            ]
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pending_tabs_wait_for_the_reply() {
        let mut tabs = Tabs::default();
        tabs.pending.push("t1".into());
        sync(&mut tabs, &terms(vec![term(1, "s1", "t1")]), |_| true);
        assert!(tabs.list.is_empty());
        unpend(&mut tabs, "t1");
        sync(&mut tabs, &terms(vec![term(1, "s1", "t1")]), |_| true);
        assert_eq!(tabs.list.len(), 1);
    }

    #[test]
    fn gone_terminals_close_tabs_and_pick_a_neighbour() {
        let mut tabs = Tabs::default();
        let all = terms(vec![
            term(1, "s1", "t1"),
            term(2, "s1", "t2"),
            term(3, "s1", "t3"),
        ]);
        sync(&mut tabs, &all, |_| true);
        tabs.active.insert("s1".into(), "t2".into());
        let t1 = find_mut(&mut tabs, "t1").unwrap();
        t1.root = layout::split(
            t1.root.clone(),
            &TermId::from_number(1),
            &TermId::from_number(9),
            Axis::Column,
        );
        t1.focus = TermId::from_number(9);
        drop_leaf(&mut tabs, &TermId::from_number(9));
        assert_eq!(find(&tabs, "t1").unwrap().focus, TermId::from_number(1));
        sync(
            &mut tabs,
            &terms(vec![term(1, "s1", "t1"), term(3, "s1", "t3")]),
            |_| true,
        );
        assert_eq!(tabs.list.len(), 2);
        assert_eq!(tabs.active.get("s1").map(String::as_str), Some("t3"));
        step(&mut tabs, "s1", 1);
        assert_eq!(tabs.active.get("s1").map(String::as_str), Some("t1"));
        step(&mut tabs, "s1", -1);
        assert_eq!(tabs.active.get("s1").map(String::as_str), Some("t3"));
        forget(&mut tabs, "t3");
        forget(&mut tabs, "t1");
        assert!(tabs.active.is_empty());
    }

    fn id(n: u64) -> TermId {
        TermId::from_number(n)
    }

    #[test]
    fn zoom_follows_focus_and_leaves() {
        let mut tabs = Tabs::default();
        let all = terms(vec![
            term(1, "s1", "t1"),
            term(2, "s1", "t1"),
            term(3, "s1", "t2"),
        ]);
        sync(&mut tabs, &all, |_| true);
        assert!(toggle_zoom(&mut tabs, "t2", &id(3)));
        assert_eq!(zoomed(find(&tabs, "t2").unwrap()), Some(&id(3)));
        assert!(toggle_zoom(&mut tabs, "t2", &id(3)));
        assert_eq!(zoomed(find(&tabs, "t2").unwrap()), None);
        assert!(!toggle_zoom(&mut tabs, "t1", &id(9)));
        assert!(toggle_zoom(&mut tabs, "t1", &id(1)));
        let t1 = find(&tabs, "t1").unwrap();
        assert_eq!(zoomed(t1), Some(&id(1)));
        assert_eq!(t1.focus, id(1));
        assert!(toggle_zoom(&mut tabs, "t1", &id(1)));
        assert_eq!(zoomed(find(&tabs, "t1").unwrap()), None);
        assert!(toggle_zoom(&mut tabs, "t1", &id(2)));
        find_mut(&mut tabs, "t1").unwrap().focus = id(1);
        sync(&mut tabs, &all, |_| true);
        assert_eq!(find(&tabs, "t1").unwrap().zoom, None);
        assert!(toggle_zoom(&mut tabs, "t1", &id(2)));
        drop_leaf(&mut tabs, &id(2));
        let t1 = find(&tabs, "t1").unwrap();
        assert_eq!(t1.zoom, None);
        assert_eq!(t1.root, Node::Leaf(id(1)));
    }

    #[test]
    fn readable_ids_show_their_own_number() {
        let a: TermId = "nixa-claude-357".parse().unwrap();
        let b: TermId = "nixa-sh-358".parse().unwrap();
        let c: TermId = "other-codex-357".parse().unwrap();
        let before: BTreeMap<TermId, u32> = [(a.clone(), 3), (id(1), 357)].into_iter().collect();
        let after = assign_numbers(&before, &[id(1), a.clone(), b.clone(), c.clone()]);
        assert_eq!(after.get(&a), Some(&357));
        assert_eq!(after.get(&b), Some(&358));
        assert_eq!(after.get(&id(1)), Some(&1));
        assert_eq!(after.get(&c), Some(&2));
    }

    #[test]
    fn a_clashing_id_number_stays_with_the_terminal_that_had_it() {
        let old: TermId = "nixa-sh-482913".parse().unwrap();
        let new: TermId = "other-claude-482913".parse().unwrap();
        let before: BTreeMap<TermId, u32> = [(old.clone(), 482913)].into_iter().collect();
        let after = assign_numbers(&before, &[new.clone(), old.clone()]);
        assert_eq!(after.get(&old), Some(&482913));
        assert_eq!(after.get(&new), Some(&1));
        let again = assign_numbers(&after, &[new.clone(), old.clone()]);
        assert_eq!(again, after);
    }

    #[test]
    fn numbers_stay_with_their_terminal() {
        let before: BTreeMap<TermId, u32> =
            [(id(2), 2), (id(5), 2), (id(7), 0)].into_iter().collect();
        let after = assign_numbers(&before, &[id(1), id(2), id(5), id(7)]);
        assert_eq!(after.get(&id(2)), Some(&2));
        assert_eq!(after.get(&id(1)), Some(&1));
        assert_eq!(after.get(&id(5)), Some(&3));
        assert_eq!(after.get(&id(7)), Some(&4));
        let mut tabs = Tabs::default();
        let mut all = terms(vec![
            term(1, "s1", "t1"),
            term(2, "s1", "t1"),
            term(3, "s1", "t2"),
        ]);
        sync(&mut tabs, &all, |_| true);
        assert_eq!(tabs.numbers.get(&id(1)), Some(&1));
        assert_eq!(tabs.numbers.get(&id(3)), Some(&3));
        drop_leaf(&mut tabs, &id(1));
        all.remove(&id(1));
        all.insert(id(4), term(4, "s1", "t3"));
        sync(&mut tabs, &all, |_| true);
        assert_eq!(tabs.numbers.get(&id(4)), Some(&1));
        assert_eq!(tabs.numbers.get(&id(2)), Some(&2));
        let mut loose = term(5, "s1", "t5");
        loose.labels.clear();
        all.insert(id(5), loose);
        tabs.numbers.insert(id(5), 7);
        sync(&mut tabs, &all, |_| true);
        assert_eq!(tabs.numbers.get(&id(5)), Some(&7));
        all.get_mut(&id(5)).unwrap().running = false;
        sync(&mut tabs, &all, |_| true);
        assert_eq!(tabs.numbers.get(&id(5)), None);
        assert_eq!(tabs.numbers.get(&id(3)), Some(&3));
    }

    #[test]
    fn resize_is_saved_and_comes_back() {
        let mut tabs = Tabs::default();
        let all = terms(vec![term(1, "s1", "t1"), term(2, "s1", "t1")]);
        sync(&mut tabs, &all, |_| true);
        assert!(resize(&mut tabs, "t1", &[], 0, 304.0, 1008.0, 8.0));
        assert!(!resize(&mut tabs, "t9", &[], 0, 304.0, 1008.0, 8.0));
        let dir = std::env::temp_dir().join(format!("nixa-app-resize-{}", std::process::id()));
        let path = dir.join(LAYOUT_FILE);
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(persist(&mut tabs, &path).unwrap());
        assert!(!persist(&mut tabs, &path).unwrap());
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty());
        let mut after = Tabs {
            saved: load_layouts(&path),
            ..Tabs::default()
        };
        sync(&mut after, &all, |_| true);
        let Node::Split { sizes, .. } = &find(&after, "t1").unwrap().root else {
            panic!("not a split");
        };
        assert_eq!(sizes.len(), 2);
        assert!((sizes[0] - 0.3).abs() < 1e-4);
        fs::remove_dir_all(&dir).ok();
    }
}
