use crate::util::{append_line, atomic_write, read_json, write_json};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const TRUST_FILE: &str = "trust.json";
pub const CALLS_FILE: &str = "calls.jsonl";
pub const KEEP_CALLS: usize = 1000;
pub const TRIM_AT: usize = 1200;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Trust {
    pub trusted: bool,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CallRow {
    pub at: u64,
    pub connector: String,
    pub action: String,
    pub tool: String,
    pub permission: String,
    pub reach: String,
    pub input: String,
    pub verdict: String,
    pub approved_by: String,
    pub result: String,
    pub ok: bool,
    pub ms: u64,
}

pub fn trust_all(root: &Path) -> BTreeMap<String, Trust> {
    read_json(&root.join(TRUST_FILE)).unwrap_or_default()
}

pub fn trusted(root: &Path, connector: &str) -> bool {
    trust_all(root).get(connector).is_some_and(|t| t.trusted)
}

pub fn set_trusted(root: &Path, connector: &str, on: bool, now: u64) -> Result<(), String> {
    nixa_protocol::paths::ensure_private_dir(root).map_err(|e| format!("The connectors folder cannot be made: {e}"))?;
    let mut all = trust_all(root);
    all.insert(connector.to_string(), Trust { trusted: on, updated_at: now });
    write_json(&root.join(TRUST_FILE), &all).map_err(|e| format!("The trust file was not saved: {e}"))
}

pub fn add_call(root: &Path, row: &CallRow) -> Result<(), String> {
    nixa_protocol::paths::ensure_private_dir(root).map_err(|e| format!("The connectors folder cannot be made: {e}"))?;
    let path = root.join(CALLS_FILE);
    let line = serde_json::to_string(row).map_err(|e| e.to_string())?;
    append_line(&path, &line).map_err(|e| format!("The call log was not written: {e}"))?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() > TRIM_AT {
        let kept = lines[lines.len() - KEEP_CALLS..].join("\n");
        atomic_write(&path, &format!("{kept}\n")).map_err(|e| format!("The call log was not trimmed: {e}"))?;
    }
    Ok(())
}

pub fn calls(root: &Path, limit: usize, connector: Option<&str>) -> Vec<CallRow> {
    let text = std::fs::read_to_string(root.join(CALLS_FILE)).unwrap_or_default();
    text.lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<CallRow>(l).ok())
        .filter(|r| connector.is_none_or(|c| r.connector == c))
        .take(limit)
        .collect()
}
