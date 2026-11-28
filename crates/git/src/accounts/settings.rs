use serde_json::Value;
use std::collections::BTreeMap;

pub const TRY_OTHERS_KEY: &str = "git.remote.try_other_accounts";
pub const PULL_MODE_KEY: &str = "git.pull.mode";
pub const AUTO_FETCH_KEY: &str = "git.auto_fetch_minutes";

pub const PULL_MODES: &[(&str, &str, &str)] = &[
    ("rebase", "Rebase", "Put your local commits on top of the new remote commits. History stays in one line."),
    ("merge", "Merge", "Join the remote commits with a merge commit."),
    ("ff-only", "Fast-forward only", "Only move ahead when you have no local commits. Stops if both sides have new commits."),
];
pub const FETCH_MINUTES: &[u32] = &[0, 5, 10, 15, 30];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    pub try_other_accounts: bool,
    pub pull_mode: String,
    pub auto_fetch_minutes: u32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            try_other_accounts: true,
            pull_mode: "rebase".into(),
            auto_fetch_minutes: 0,
        }
    }
}

pub fn prefs_from(values: &BTreeMap<String, Value>) -> Prefs {
    let base = Prefs::default();
    let mode = values
        .get(PULL_MODE_KEY)
        .and_then(Value::as_str)
        .filter(|m| PULL_MODES.iter().any(|(id, _, _)| id == m))
        .map(str::to_string)
        .unwrap_or(base.pull_mode);
    let minutes = values
        .get(AUTO_FETCH_KEY)
        .and_then(Value::as_u64)
        .and_then(|m| u32::try_from(m).ok())
        .filter(|m| FETCH_MINUTES.contains(m))
        .unwrap_or(0);
    Prefs {
        try_other_accounts: values.get(TRY_OTHERS_KEY).and_then(Value::as_bool).unwrap_or(true),
        pull_mode: mode,
        auto_fetch_minutes: minutes,
    }
}

pub fn fetch_label(minutes: u32) -> String {
    if minutes == 0 { "Off".into() } else { format!("Every {minutes} min") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prefs_take_safe_defaults() {
        assert_eq!(prefs_from(&BTreeMap::new()), Prefs::default());
        let values = BTreeMap::from([
            (PULL_MODE_KEY.to_string(), json!("merge")),
            (AUTO_FETCH_KEY.to_string(), json!(15)),
            (TRY_OTHERS_KEY.to_string(), json!(false)),
        ]);
        let p = prefs_from(&values);
        assert_eq!(p.pull_mode, "merge");
        assert_eq!(p.auto_fetch_minutes, 15);
        assert!(!p.try_other_accounts);
        let bad = BTreeMap::from([(PULL_MODE_KEY.to_string(), json!("odd")), (AUTO_FETCH_KEY.to_string(), json!(7))]);
        assert_eq!(prefs_from(&bad), Prefs::default());
        assert_eq!(fetch_label(0), "Off");
        assert_eq!(fetch_label(5), "Every 5 min");
    }
}
