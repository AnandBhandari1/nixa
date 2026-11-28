use crate::screens::git::tools::blame::model::clean_path;
use crate::screens::git::tools::errors;
use crate::screens::git::tools::run;
use std::fs;
use std::path::Path;

pub const GONE: &str = "the file is not in the folder. It was deleted on one side. Pick Use current or Use incoming for it in the Changes panel.";

pub fn read_file(root: &Path, file: &str) -> Result<String, String> {
    let p = clean_path(file)?;
    match fs::read(root.join(&p)) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| "it is not a text file.".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(GONE.into()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn mark_resolved(root: &Path, file: &str) -> Result<(), String> {
    let p = clean_path(file)?;
    let out = run::git(root, &["--literal-pathspecs", "add", "-A", "--", &p]);
    if out.ok() {
        return Ok(());
    }
    let said = errors::describe(&out.said());
    Err([
        said.title,
        if said.detail.is_empty() {
            said.hint
        } else {
            said.detail
        },
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(". "))
}

pub fn write_resolved(root: &Path, file: &str, text: &str) -> Result<(), String> {
    let p = clean_path(file)?;
    fs::write(root.join(&p), text).map_err(|e| e.to_string())?;
    mark_resolved(root, &p)
}
