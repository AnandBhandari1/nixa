use super::Target;
use super::fix::FIX_WAIT_MS;
use super::hard::DEEP_RELAY_MS;
use crate::util::{log, read_json, write_json};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const TARGETS_FILE: &str = "followup-targets.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Close,
    Fix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub kind: Kind,
    pub terminal: String,
    pub at: u64,
}

pub fn wait_of(kind: Kind) -> u64 {
    match kind {
        Kind::Close => DEEP_RELAY_MS,
        Kind::Fix => FIX_WAIT_MS,
    }
}

fn kind_of(target: &Target) -> Kind {
    match target {
        Target::Close => Kind::Close,
        Target::Fix(_) => Kind::Fix,
    }
}

pub fn fresh(saved: &Saved, now: u64) -> bool {
    now.saturating_sub(saved.at) <= wait_of(saved.kind)
}

pub(crate) struct Targets {
    file: PathBuf,
    items: HashMap<String, (Target, String)>,
    saved: BTreeMap<String, Saved>,
}

impl Targets {
    pub fn open(file: &Path, now: u64) -> Targets {
        let all: BTreeMap<String, Saved> = read_json(file).unwrap_or_default();
        let saved: BTreeMap<String, Saved> = all.into_iter().filter(|(_, s)| fresh(s, now)).collect();
        let items = saved
            .iter()
            .map(|(id, s)| {
                let target = match s.kind {
                    Kind::Close => Target::Close,
                    Kind::Fix => Target::Fix(None),
                };
                (id.clone(), (target, s.terminal.clone()))
            })
            .collect();
        Targets { file: file.to_path_buf(), items, saved }
    }

    fn save(&self) {
        if let Err(e) = write_json(&self.file, &self.saved) {
            log("followup", &format!("could not save {}: {e}", self.file.display()));
        }
    }

    pub fn insert(&mut self, id: String, value: (Target, String), at: u64) {
        let saved = Saved { kind: kind_of(&value.0), terminal: value.1.clone(), at };
        self.saved.insert(id.clone(), saved);
        self.items.insert(id, value);
        self.save();
    }

    pub fn remove(&mut self, id: &str) -> Option<(Target, String)> {
        let found = self.items.remove(id);
        if self.saved.remove(id).is_some() {
            self.save();
        }
        found
    }

    pub fn closing(&self) -> HashMap<String, String> {
        self.saved.iter().filter(|(_, s)| s.kind == Kind::Close).map(|(id, s)| (s.terminal.clone(), id.clone())).collect()
    }

    pub fn fixing(&self) -> HashSet<String> {
        self.saved.values().filter(|s| s.kind == Kind::Fix).map(|s| s.terminal.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_come_back_after_a_restart_until_their_wait_ends() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("deep").join(TARGETS_FILE);
        let mut targets = Targets::open(&file, 0);
        targets.insert("d1".into(), (Target::Close, "m5".into()), 1000);
        targets.insert("d2".into(), (Target::Fix(None), "m6".into()), 1000);
        targets.insert("d3".into(), (Target::Close, "m7".into()), 1000);
        assert!(matches!(targets.remove("d3"), Some((Target::Close, t)) if t == "m7"));
        let again = Targets::open(&file, 2000);
        assert_eq!(again.closing(), HashMap::from([("m5".to_string(), "d1".to_string())]));
        assert_eq!(again.fixing(), HashSet::from(["m6".to_string()]));
        let later = Targets::open(&file, 1000 + DEEP_RELAY_MS + 1);
        assert!(later.closing().is_empty());
        assert_eq!(later.fixing(), HashSet::from(["m6".to_string()]));
        assert!(Targets::open(&file, 1000 + FIX_WAIT_MS + 1).fixing().is_empty());
        let mut gone = Targets::open(&file, 2000);
        assert!(matches!(gone.remove("d1"), Some((Target::Close, t)) if t == "m5"));
        assert!(gone.remove("d1").is_none());
        let left = Targets::open(&file, 2000);
        assert!(left.closing().is_empty());
        assert_eq!(left.fixing(), HashSet::from(["m6".to_string()]));
        std::fs::write(&file, "[1,2]").unwrap();
        assert!(Targets::open(&file, 2000).closing().is_empty());
    }
}
