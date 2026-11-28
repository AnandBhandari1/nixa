use super::table::{self, Entry, GLOBAL};
use nixa_protocol::paths;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const FILE: &str = "keys.json";
pub const STARTER: &str = "{\n  \"global\": {}\n}\n";

pub fn path() -> PathBuf {
    paths::data_dir().join(FILE)
}

pub fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn read(path: &Path) -> (Vec<Entry>, Vec<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), Vec::new()),
        Err(e) => (Vec::new(), vec![format!("Cannot read {FILE}: {e}")]),
    }
}

pub fn ensure(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, STARTER)?;
    Ok(())
}

pub fn parse(text: &str) -> (Vec<Entry>, Vec<String>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return (out, vec![format!("{FILE} is not valid JSON: {e}")]),
    };
    let Some(sections) = value.as_object() else {
        return (out, vec![format!("{FILE} must hold one JSON object")]);
    };
    for (section, keys) in sections {
        if section != GLOBAL {
            problems.push(format!(
                "The \"{section}\" part is not used. Put keys under \"{GLOBAL}\"."
            ));
            continue;
        }
        let Some(keys) = keys.as_object() else {
            problems.push(format!("\"{GLOBAL}\" must hold key and action pairs"));
            continue;
        };
        for (key, name) in keys {
            let name = match name {
                Value::String(s) => s.trim().to_string(),
                Value::Null => String::new(),
                _ => {
                    problems.push(format!("\"{key}\": the action must be text"));
                    continue;
                }
            };
            let Some(clean) = table::clean_key(key) else {
                problems.push(format!("\"{key}\" is not a key Nixa can read"));
                continue;
            };
            if super::names::action(&name).is_none() {
                problems.push(format!("\"{name}\" for {key} is not a Nixa action"));
                continue;
            }
            out.push(Entry { key: clean, name });
        }
    }
    (out, problems)
}
