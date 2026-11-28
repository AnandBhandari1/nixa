use crate::git_tools::run::read;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub const NO_FOLDER: &str = "(no folder)";

pub fn root_from(common: &str, top: &str) -> Option<String> {
    let common = common.trim();
    let path = Path::new(common);
    if !common.is_empty() && path.file_name().is_some_and(|n| n == ".git") {
        return path.parent().map(|p| p.display().to_string());
    }
    let top = top.trim();
    (!top.is_empty()).then(|| top.to_string())
}

pub async fn project_of(known: &Mutex<HashMap<String, String>>, folder: Option<&str>) -> String {
    let Some(folder) = folder.map(str::trim).filter(|f| !f.is_empty()) else {
        return NO_FOLDER.into();
    };
    if let Some(found) = crate::app::lock(known).get(folder) {
        return found.clone();
    }
    let common = read(folder, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await.unwrap_or_default();
    let top = if root_from(&common, "").is_some() { String::new() } else { read(folder, &["rev-parse", "--show-toplevel"]).await.unwrap_or_default() };
    let Some(root) = root_from(&common, &top) else {
        return folder.to_string();
    };
    crate::app::lock(known).insert(folder.to_string(), root.clone());
    root
}
