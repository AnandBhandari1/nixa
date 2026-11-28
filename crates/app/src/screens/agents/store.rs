use super::model::{Scope, Sort, Stamp};
use nixa_store::Connection;
use nixa_store::settings::{get_setting, set_setting};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const SORT_KEY: &str = "app.agents.sort";
pub const SCOPE_KEY: &str = "app.agents.scope";
pub const CHANGED_KEY: &str = "app.agents.changed";
pub const AWAY_KEY: &str = "app.tabs.away";

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub sort: Sort,
    pub scope: Scope,
    pub changed: BTreeMap<String, Stamp>,
    pub away: BTreeSet<String>,
}

fn read(conn: Option<&Connection>, key: &str) -> Option<Value> {
    get_setting(conn?, key).ok().flatten()
}

pub fn load(conn: Option<&Connection>) -> Saved {
    let word = |key: &str| {
        read(conn, key)
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    };
    Saved {
        sort: Sort::from_word(&word(SORT_KEY)),
        scope: Scope::from_word(&word(SCOPE_KEY)),
        changed: read(conn, CHANGED_KEY)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        away: read(conn, AWAY_KEY)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
    }
}

pub fn save(conn: Option<&Connection>, key: &str, value: Value) -> anyhow::Result<()> {
    match conn {
        Some(conn) => set_setting(conn, key, &value),
        None => Ok(()),
    }
}

pub fn save_sort(conn: Option<&Connection>, sort: Sort) -> anyhow::Result<()> {
    save(conn, SORT_KEY, json!(sort.word()))
}

pub fn save_scope(conn: Option<&Connection>, scope: Scope) -> anyhow::Result<()> {
    save(conn, SCOPE_KEY, json!(scope.word()))
}

pub fn save_changed(
    conn: Option<&Connection>,
    changed: &BTreeMap<String, Stamp>,
) -> anyhow::Result<()> {
    save(conn, CHANGED_KEY, serde_json::to_value(changed)?)
}

pub fn save_away(conn: Option<&Connection>, away: &BTreeSet<String>) -> anyhow::Result<()> {
    save(conn, AWAY_KEY, serde_json::to_value(away)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_come_back_after_a_restart() {
        let conn = nixa_store::open_memory().unwrap();
        let fresh = load(Some(&conn));
        assert_eq!(fresh.sort, Sort::Priority);
        assert_eq!(fresh.scope, Scope::Space);
        assert!(fresh.changed.is_empty() && fresh.away.is_empty());
        save_sort(Some(&conn), Sort::Grouped).unwrap();
        save_scope(Some(&conn), Scope::All).unwrap();
        let mut changed = BTreeMap::new();
        changed.insert(
            "m3".to_string(),
            Stamp {
                s: "working".into(),
                at: 42,
            },
        );
        save_changed(Some(&conn), &changed).unwrap();
        save_away(Some(&conn), &BTreeSet::from(["t2".to_string()])).unwrap();
        let back = load(Some(&conn));
        assert_eq!(back.sort, Sort::Grouped);
        assert_eq!(back.scope, Scope::All);
        assert_eq!(back.changed, changed);
        assert!(back.away.contains("t2"));
        assert_eq!(load(None).sort, Sort::Priority);
        assert!(save_sort(None, Sort::Grouped).is_ok());
    }
}
