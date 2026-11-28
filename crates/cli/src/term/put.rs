use crate::args::{Parsed, usage};
use crate::ctx::{self, Ctx};
use anyhow::{Context, Result, bail};
use nixa_protocol::{Reply, Request};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn target(dir: &Path, file: &Path) -> Result<PathBuf> {
    let name = file
        .file_name()
        .ok_or_else(|| usage(format!("{} has no file name", file.display())))?;
    Ok(dir.join(name))
}

pub fn run(ctx: &Ctx, mut p: Parsed, out: &mut dyn Write) -> Result<i32> {
    let force = p.flag(&["--force", "-f"]);
    p.finish(2, false)?;
    let (Some(id), Some(file)) = (p.pos.first(), p.pos.get(1)) else {
        return Err(usage("nixa term put needs a terminal id and a file, for example: nixa term put nixa-claude-3 notes.txt"));
    };
    let id = super::term_id(id)?;
    let file = PathBuf::from(file);
    if !file.is_file() {
        bail!("{} is not a file", file.display());
    }
    let term = match ctx::call(ctx, &Request::Get { id: id.clone() })? {
        Reply::Term { term } => term,
        other => return Err(ctx::unexpected(&other)),
    };
    let dir = PathBuf::from(&term.cwd);
    if !dir.is_dir() {
        bail!("the folder of {id} ({}) is not on this computer", dir.display());
    }
    let dest = target(&dir, &file)?;
    if dest.exists() && !force {
        bail!("{} already exists. Nothing was copied. Use --force to replace it.", dest.display());
    }
    if dest.canonicalize().ok() == file.canonicalize().ok() && dest.exists() {
        bail!("{} is already in the folder of {id}", dest.display());
    }
    std::fs::copy(&file, &dest).with_context(|| format!("cannot copy to {}", dest.display()))?;
    if ctx.json {
        writeln!(out, "{}", serde_json::json!({"id": id, "path": dest}))?;
    } else {
        writeln!(out, "{}", dest.display())?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_keeps_its_name() {
        let got = target(Path::new("/work"), Path::new("some/dir/a.txt")).unwrap();
        assert_eq!(got, PathBuf::from("/work/a.txt"));
        assert!(target(Path::new("/work"), Path::new("/")).is_err());
    }
}
