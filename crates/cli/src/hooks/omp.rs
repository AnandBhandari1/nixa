use super::{Action, Setup, place};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

const LINK_NAME: &str = "nixa-status.ts";
const LABEL: &str = "omp";

enum Found {
    Missing,
    Ours,
    Other(String),
}

fn found(link: &Path, target: &Path) -> Result<Found> {
    let meta = match fs::symlink_metadata(link) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Found::Missing),
        Err(e) => return Err(e).with_context(|| format!("cannot look at {}", link.display())),
    };
    if !meta.file_type().is_symlink() {
        return Ok(Found::Other("a file, not a link".to_string()));
    }
    let to = fs::read_link(link)?;
    let same = to == target || fs::canonicalize(link).ok().as_deref() == Some(target);
    Ok(if same { Found::Ours } else { Found::Other(format!("a link to {}", to.display())) })
}

pub fn run(setup: &Setup, dir: &Path, action: Action, out: &mut dyn Write) -> Result<Value> {
    let target = setup.hooks.join("omp").join("nixa-status.ts");
    if !target.is_file() {
        bail!("{} is missing", target.display());
    }
    let ext = dir.join("extensions");
    let link = ext.join(LINK_NAME);
    let now = found(&link, &target)?;
    let say = !setup.json;
    if say {
        writeln!(out, "{LABEL}: {}", link.display())?;
    }
    let state = match &now {
        Found::Missing => "not installed",
        Found::Ours => "installed",
        Found::Other(_) => "other file",
    };
    let other = match &now {
        Found::Other(what) => Some(what.clone()),
        _ => None,
    };
    if action == Action::Status {
        if say {
            match &other {
                Some(what) => writeln!(out, "  other file in place: {what}")?,
                None => writeln!(out, "  {state}")?,
            }
        }
        return Ok(json!({"agent": "omp", "found": true, "file": link, "state": state, "other": other}));
    }
    let mut changes = Vec::new();
    let mut backup = None;
    match (action, &now) {
        (Action::Install, Found::Ours) | (Action::Remove, Found::Missing) => {}
        (Action::Install, Found::Missing) => changes.push(format!("+ link {} -> {}", link.display(), target.display())),
        (Action::Install, Found::Other(what)) => {
            let to = place::backup_path(&link);
            changes.push(format!("~ {} is {what}: it moves to {}", link.display(), to.display()));
            changes.push(format!("+ link {} -> {}", link.display(), target.display()));
            backup = Some(to);
        }
        (Action::Remove, Found::Ours) => changes.push(format!("- link {}", link.display())),
        (Action::Remove, Found::Other(what)) => {
            if say {
                writeln!(out, "  {} is {what}, not ours: left as is", link.display())?;
            }
        }
        (Action::Status, _) => {}
    }
    if say {
        if changes.is_empty() {
            writeln!(out, "  no change")?;
        }
        for line in &changes {
            writeln!(out, "  {line}")?;
        }
    }
    let changed = !changes.is_empty();
    let written = changed && setup.apply;
    if written {
        apply(action, &ext, &link, &target, backup.as_deref())?;
        if say {
            if let Some(b) = &backup {
                writeln!(out, "  backup: {}", b.display())?;
            }
            writeln!(out, "  done")?;
        }
    }
    Ok(json!({
        "agent": "omp",
        "found": true,
        "file": link,
        "changed": changed,
        "written": written,
        "changes": changes,
        "backup": if written { backup } else { None },
    }))
}

fn apply(action: Action, ext: &Path, link: &Path, target: &Path, backup: Option<&Path>) -> Result<()> {
    if action == Action::Remove {
        return fs::remove_file(link).with_context(|| format!("cannot remove {}", link.display()));
    }
    fs::create_dir_all(ext).with_context(|| format!("cannot make {}", ext.display()))?;
    let home = ext.parent().map(place::real_path).unwrap_or_default();
    if !place::real_path(ext).starts_with(&home) {
        bail!("{} leads outside {}, so Nixa does not change it", ext.display(), home.display());
    }
    if let Some(b) = backup {
        fs::rename(link, b).with_context(|| format!("cannot move {} to {}", link.display(), b.display()))?;
    }
    std::os::unix::fs::symlink(target, link).with_context(|| format!("cannot link {}", link.display()))
}
