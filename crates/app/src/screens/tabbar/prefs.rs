use serde_json::{Map, Value};
use std::io;
use std::path::{Path, PathBuf};

const FILE: &str = "quick_models.json";

pub fn file() -> PathBuf {
    nixa_protocol::paths::data_dir().join(FILE)
}

fn read_map(path: &Path) -> io::Result<Map<String, Value>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(error),
    };
    let value: Value = serde_json::from_str(&text).map_err(io::Error::other)?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| io::Error::other("the quick model file is not an object"))
}

pub fn model_for(path: &Path, agent: &str) -> Option<String> {
    read_map(path)
        .ok()?
        .get(agent)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
pub fn set_model(path: &Path, agent: &str, model: Option<&str>) -> io::Result<()> {
    let mut map = read_map(path)?;
    match model.map(str::trim).filter(|model| !model.is_empty()) {
        Some(model) => map.insert(agent.to_string(), Value::String(model.to_string())),
        None => map.remove(agent),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(map)).map_err(io::Error::other)?;
    let temp = path.with_extension(format!("json.{}.{}.tmp", std::process::id(), nixa_store::time::now_ms()));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}
