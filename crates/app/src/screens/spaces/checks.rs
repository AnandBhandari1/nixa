use super::Input;
use super::git::{GitInfo, Look, Stamp};
use crate::spaces::Space;
use nixa_protocol::TermId;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

pub const FALLBACK: Duration = Duration::from_secs(60);

#[derive(Debug, Default)]
pub struct Checks {
    pub git: HashMap<String, Option<GitInfo>>,
    pub checked: HashMap<String, Instant>,
    pub unsure: HashSet<String>,
    pub pending: BTreeSet<String>,
    pub current: Option<String>,
    pub at: HashMap<String, String>,
    pub busy: BTreeMap<TermId, (String, bool)>,
    pub stamps: HashMap<String, Stamp>,
}

pub fn place_of(input: &Input, space: &Space) -> String {
    input
        .places
        .get(&space.id)
        .filter(|p| !p.is_empty())
        .cloned()
        .unwrap_or_else(|| space.folder.clone())
}

fn same_repo(checks: &Checks, key: &str, keys: &HashSet<String>) -> Vec<String> {
    let mut out = vec![key.to_string()];
    if let Some(Some(info)) = checks.git.get(key) {
        out.extend(
            keys.iter()
                .filter(|k| checks.git.get(*k).cloned().flatten().is_some_and(|g| g.repo == info.repo))
                .cloned(),
        );
    }
    out
}

pub fn due(checks: &mut Checks, input: &Input, now: Instant) -> BTreeSet<String> {
    let at: HashMap<String, String> = input
        .spaces
        .iter()
        .map(|s| (s.id.clone(), place_of(input, s)))
        .collect();
    let keys: HashSet<String> = at.values().filter(|k| !k.is_empty()).cloned().collect();
    let mut due: BTreeSet<String> = keys
        .iter()
        .filter(|k| !checks.git.contains_key(*k))
        .cloned()
        .collect();
    for (id, key) in &at {
        if checks.at.get(id).is_some_and(|old| old != key) && keys.contains(key) {
            due.insert(key.clone());
        }
    }
    let mut ended: Vec<String> = Vec::new();
    for (term, (space, was)) in &checks.busy {
        let now_busy = input.busy.get(term).is_some_and(|(_, b)| *b);
        if *was && !now_busy {
            ended.push(space.clone());
        }
    }
    for space in ended {
        if let Some(key) = at.get(&space) {
            due.extend(same_repo(checks, key, &keys));
        }
    }
    let busy = input.busy.clone();
    checks.at = at;
    checks.busy = busy;
    if input.window_active {
        for key in &keys {
            let asks = checks.git.get(key).is_some_and(Option::is_some) || checks.unsure.contains(key);
            let old = checks.checked.get(key).is_none_or(|t| now.duration_since(*t) > FALLBACK);
            if asks && old {
                due.insert(key.clone());
            }
        }
    }
    due.retain(|k| checks.current.as_deref() != Some(k.as_str()));
    due
}

pub fn stamp_due(checks: &mut Checks, stamp: impl Fn(&GitInfo) -> Stamp) -> BTreeSet<String> {
    let keys: HashSet<String> = checks.git.keys().cloned().collect();
    let mut due = BTreeSet::new();
    let known: Vec<(String, GitInfo)> = checks
        .git
        .iter()
        .filter_map(|(k, g)| Some((k.clone(), g.clone()?)))
        .collect();
    for (key, info) in known {
        let now = stamp(&info);
        if checks.stamps.get(&key).is_some_and(|old| *old != now) {
            due.extend(same_repo(checks, &key, &keys));
        }
        checks.stamps.insert(key, now);
    }
    due
}

pub fn apply(checks: &mut Checks, key: &str, look: Look, stamp: Option<Stamp>, now: Instant) {
    checks.checked.insert(key.to_string(), now);
    checks.unsure.remove(key);
    match look {
        Look::Repo(info) => {
            checks.git.insert(key.to_string(), Some(info));
            if let Some(stamp) = stamp {
                checks.stamps.insert(key.to_string(), stamp);
            }
        }
        Look::NoRepo => {
            checks.git.insert(key.to_string(), None);
            checks.stamps.remove(key);
        }
        Look::Unsure => {
            checks.git.entry(key.to_string()).or_insert(None);
            checks.unsure.insert(key.to_string());
        }
    }
}

pub fn prune(checks: &mut Checks, input: &Input) {
    let keys: HashSet<String> = input.spaces.iter().map(|s| place_of(input, s)).collect();
    checks.git.retain(|k, _| keys.contains(k));
    checks.checked.retain(|k, _| keys.contains(k));
    checks.stamps.retain(|k, _| keys.contains(k));
    checks.unsure.retain(|k| keys.contains(k));
    checks.pending.retain(|k| keys.contains(k));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(folders: &[(&str, &str)]) -> Input {
        Input {
            spaces: folders
                .iter()
                .map(|(id, f)| Space {
                    id: id.to_string(),
                    folder: f.to_string(),
                    ..Space::default()
                })
                .collect(),
            ..Input::default()
        }
    }

    fn info(repo: &str) -> GitInfo {
        GitInfo {
            name: "r".into(),
            branch: Some("main".into()),
            ahead: 0,
            behind: 0,
            repo: repo.into(),
            top: String::new(),
            git_dir: String::new(),
            linked: false,
        }
    }

    #[test]
    fn new_places_moves_and_turn_ends_are_due() {
        let mut c = Checks::default();
        let now = Instant::now();
        let mut i = input(&[("s1", "/a"), ("s2", "/b"), ("s3", "/c")]);
        let first: Vec<String> = due(&mut c, &i, now).into_iter().collect();
        assert_eq!(first, ["/a", "/b", "/c"]);
        apply(&mut c, "/a", Look::Repo(info("/r/.git")), None, now);
        apply(&mut c, "/b", Look::NoRepo, None, now);
        apply(&mut c, "/c", Look::Repo(info("/r/.git")), None, now);
        assert!(due(&mut c, &i, now).is_empty());
        i.places.insert("s2".into(), "/b/sub".into());
        assert_eq!(due(&mut c, &i, now).into_iter().collect::<Vec<_>>(), ["/b/sub"]);
        apply(&mut c, "/b/sub", Look::NoRepo, None, now);
        i.busy.insert(TermId::from_number(1), ("s1".into(), true));
        assert!(due(&mut c, &i, now).is_empty());
        i.busy.insert(TermId::from_number(1), ("s1".into(), false));
        assert_eq!(due(&mut c, &i, now).into_iter().collect::<Vec<_>>(), ["/a", "/c"]);
        i.busy.insert(TermId::from_number(2), ("s3".into(), true));
        assert!(due(&mut c, &i, now).is_empty());
        i.busy.remove(&TermId::from_number(2));
        assert_eq!(due(&mut c, &i, now).into_iter().collect::<Vec<_>>(), ["/a", "/c"]);
    }

    #[test]
    fn the_fallback_waits_for_a_window_in_front() {
        let mut c = Checks::default();
        let then = Instant::now();
        let mut i = input(&[("s1", "/a"), ("s2", "/b")]);
        due(&mut c, &i, then);
        apply(&mut c, "/a", Look::Repo(info("/r/.git")), None, then);
        apply(&mut c, "/b", Look::NoRepo, None, then);
        let later = then + FALLBACK + Duration::from_secs(1);
        assert!(due(&mut c, &i, later).is_empty());
        i.window_active = true;
        assert_eq!(due(&mut c, &i, later).into_iter().collect::<Vec<_>>(), ["/a"]);
        apply(&mut c, "/b", Look::Unsure, None, then);
        assert_eq!(due(&mut c, &i, later).len(), 2);
        c.current = Some("/a".into());
        assert_eq!(due(&mut c, &i, later).into_iter().collect::<Vec<_>>(), ["/b"]);
    }

    #[test]
    fn a_changed_stamp_wakes_the_whole_repo() {
        let mut c = Checks::default();
        let now = Instant::now();
        apply(&mut c, "/a", Look::Repo(info("/r/.git")), Some(vec![None]), now);
        apply(&mut c, "/c", Look::Repo(info("/r/.git")), Some(vec![None]), now);
        apply(&mut c, "/x", Look::Repo(info("/x/.git")), Some(vec![None]), now);
        assert!(stamp_due(&mut c, |_| vec![None]).is_empty());
        let woke = stamp_due(&mut c, |g| if g.repo == "/r/.git" { vec![Some(std::time::SystemTime::now())] } else { vec![None] });
        assert_eq!(woke.into_iter().collect::<Vec<_>>(), ["/a", "/c"]);
        let i = input(&[("s1", "/a")]);
        prune(&mut c, &i);
        assert_eq!(c.git.len(), 1);
    }
}
