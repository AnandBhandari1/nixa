use super::git::GitInfo;
use crate::spaces::Space;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tree {
    Mid,
    Last,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub index: usize,
    pub id: String,
    pub tree: Option<Tree>,
    pub block: usize,
    pub pos: usize,
    pub start: usize,
    pub end: usize,
    pub first: usize,
    pub last: usize,
    pub git: Option<GitInfo>,
}

pub fn ordered(
    spaces: &[Space],
    place: impl Fn(&Space) -> String,
    git: &HashMap<String, Option<GitInfo>>,
    shown: &HashSet<String>,
) -> Vec<Row> {
    let list: Vec<(usize, &Space, Option<GitInfo>)> = spaces
        .iter()
        .enumerate()
        .filter(|(_, s)| shown.contains(&s.id))
        .map(|(i, s)| (i, s, git.get(&place(s)).cloned().flatten()))
        .collect();
    let mains: HashSet<String> = list
        .iter()
        .filter_map(|(_, _, g)| g.as_ref().filter(|g| !g.linked).map(|g| g.repo.clone()))
        .collect();
    let nested = |g: &Option<GitInfo>| g.as_ref().is_some_and(|g| g.linked && mains.contains(&g.repo));
    let mut out: Vec<Row> = Vec::new();
    let mut placed: HashSet<String> = HashSet::new();
    let mut block = 0;
    for (i, space, g) in &list {
        if nested(g) {
            continue;
        }
        let start = out.len();
        out.push(row(*i, space, None, block, out.len(), g.clone()));
        if let Some(main) = g.as_ref().filter(|g| !g.linked)
            && placed.insert(main.repo.clone())
        {
            let kids: Vec<&(usize, &Space, Option<GitInfo>)> = list
                .iter()
                .filter(|(_, _, k)| nested(k) && k.as_ref().is_some_and(|k| k.repo == main.repo))
                .collect();
            for (n, (ki, ks, kg)) in kids.iter().enumerate() {
                let tree = if n + 1 == kids.len() { Tree::Last } else { Tree::Mid };
                out.push(row(*ki, ks, Some(tree), block, out.len(), kg.clone()));
            }
        }
        let end = out.len() - 1;
        let first = out[start..].iter().map(|r| r.index).min().unwrap_or(*i);
        let last = out[start..].iter().map(|r| r.index).max().unwrap_or(*i);
        for r in &mut out[start..] {
            r.start = start;
            r.end = end;
            r.first = first;
            r.last = last;
        }
        block += 1;
    }
    out
}

fn row(index: usize, space: &Space, tree: Option<Tree>, block: usize, pos: usize, git: Option<GitInfo>) -> Row {
    Row {
        index,
        id: space.id.clone(),
        tree,
        block,
        pos,
        start: pos,
        end: pos,
        first: index,
        last: index,
        git,
    }
}

pub fn drop_index(spaces: &[Space], id: &str, down: bool, first: usize, last: usize) -> Option<usize> {
    let anchor = spaces.get(if down { last } else { first })?.id.clone();
    if anchor == id {
        return None;
    }
    let rest: Vec<&Space> = spaces.iter().filter(|s| s.id != id).collect();
    let at = rest.iter().position(|s| s.id == anchor)?;
    Some(if down { at + 1 } else { at })
}

pub fn tail_text(git: &GitInfo, tree: bool, name: &str) -> String {
    let raw = git.branch.clone().unwrap_or_else(|| "detached".to_string());
    let same = |a: &str| a.eq_ignore_ascii_case(name);
    let branch = if same(&raw) || raw == "master" || raw == "main" { String::new() } else { raw };
    let repo = if tree || same(&git.name) { String::new() } else { git.name.clone() };
    [repo, branch]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(id: &str, folder: &str) -> Space {
        Space {
            id: id.into(),
            name: id.into(),
            folder: folder.into(),
            ..Space::default()
        }
    }

    fn info(repo: &str, linked: bool, branch: &str) -> Option<GitInfo> {
        Some(GitInfo {
            name: "proj".into(),
            branch: Some(branch.into()),
            ahead: 0,
            behind: 0,
            repo: repo.into(),
            top: String::new(),
            git_dir: String::new(),
            linked,
        })
    }

    #[test]
    fn worktrees_sit_under_their_main_checkout() {
        let spaces = vec![
            space("s1", "/wt1"),
            space("s2", "/other"),
            space("s3", "/main"),
            space("s4", "/wt2"),
            space("s5", "/lonely-wt"),
        ];
        let mut git = HashMap::new();
        git.insert("/wt1".to_string(), info("/main/.git", true, "a"));
        git.insert("/other".to_string(), None);
        git.insert("/main".to_string(), info("/main/.git", false, "main"));
        git.insert("/wt2".to_string(), info("/main/.git", true, "b"));
        git.insert("/lonely-wt".to_string(), info("/gone/.git", true, "c"));
        let shown: HashSet<String> = spaces.iter().map(|s| s.id.clone()).collect();
        let rows = ordered(&spaces, |s| s.folder.clone(), &git, &shown);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["s2", "s3", "s1", "s4", "s5"]);
        assert_eq!(rows[2].tree, Some(Tree::Mid));
        assert_eq!(rows[3].tree, Some(Tree::Last));
        assert_eq!(rows[4].tree, None);
        assert_eq!((rows[1].block, rows[2].block, rows[3].block), (1, 1, 1));
        assert_eq!((rows[1].first, rows[1].last), (0, 3));
        assert_eq!((rows[1].start, rows[1].end), (1, 3));
        assert_eq!(rows[4].block, 2);
        let mut fewer = shown.clone();
        fewer.remove("s3");
        let rows = ordered(&spaces, |s| s.folder.clone(), &git, &fewer);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.tree.is_none()));
    }

    #[test]
    fn drops_go_before_or_after_the_block() {
        let spaces: Vec<Space> = ["s1", "s2", "s3", "s4"].iter().map(|i| space(i, "/")).collect();
        assert_eq!(drop_index(&spaces, "s1", true, 2, 3), Some(3));
        assert_eq!(drop_index(&spaces, "s4", false, 0, 1), Some(0));
        assert_eq!(drop_index(&spaces, "s4", false, 1, 2), Some(1));
        assert_eq!(drop_index(&spaces, "s2", false, 1, 1), None);
        assert_eq!(drop_index(&spaces, "s2", true, 9, 9), None);
    }

    #[test]
    fn the_tail_says_only_what_is_unusual() {
        let g = info("/r/.git", false, "main").unwrap();
        assert_eq!(tail_text(&g, false, "proj"), "");
        assert_eq!(tail_text(&g, false, "web"), "proj");
        let spike = GitInfo { branch: Some("spike".into()), ..g.clone() };
        assert_eq!(tail_text(&spike, false, "web"), "proj \u{b7} spike");
        assert_eq!(tail_text(&spike, true, "Spike"), "");
        let none = GitInfo { branch: None, ..g };
        assert_eq!(tail_text(&none, true, "x"), "detached");
    }
}
