use crate::args::{Parsed, usage};
use crate::ctx::{self, Ctx};
use crate::exec::term_cwd;
use crate::help;
use anyhow::{Context, Result};
use nixa_protocol::{EngineError, ErrorCode};
use serde_json::json;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::FILE.as_bytes())?;
        return Ok(2);
    };
    let asks_help = matches!(sub.as_str(), "help" | "-h" | "--help")
        || rest.iter().any(|a| a == "-h" || a == "--help");
    if asks_help {
        out.write_all(help::FILE.as_bytes())?;
        return Ok(0);
    }
    match sub.as_str() {
        "read" | "cat" => read(ctx, rest, out),
        "write" => write(ctx, rest, out),
        "edit" => edit(ctx, rest, out),
        other => Err(usage(format!("unknown command nixa file {other}"))),
    }
}

fn refuse(code: ErrorCode, message: String) -> anyhow::Error {
    EngineError { code, message }.into()
}

fn target(ctx: &Ctx, p: &mut Parsed, what: &str) -> Result<PathBuf> {
    let term = p.value(&["--in"]);
    p.finish(1, false)?;
    let path = p
        .pos
        .first()
        .ok_or_else(|| usage(format!("nixa file {what} needs a PATH")))?;
    let path = Path::new(path);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let base = match term {
        Some(id) => term_cwd(ctx, &id)?,
        None => std::env::current_dir()?,
    };
    Ok(base.join(path))
}

fn load(path: &Path) -> Result<Vec<u8>> {
    match fs::read(path) {
        Ok(b) => Ok(b),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err(refuse(ErrorCode::NotFound, format!("{} does not exist", path.display())))
        }
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

pub fn line_range(v: &str) -> Result<(usize, Option<usize>)> {
    let bad = || usage(format!("--lines needs A:B, A: or :B (from line 1), got {v:?}"));
    let (a, b) = match v.split_once(':') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (v.trim(), v.trim()),
    };
    let from = if a.is_empty() { 1 } else { a.parse::<usize>().map_err(|_| bad())? };
    let to = if b.is_empty() { None } else { Some(b.parse::<usize>().map_err(|_| bad())?) };
    if from == 0 || to.is_some_and(|t| t < from) {
        return Err(bad());
    }
    Ok((from, to))
}

pub fn pick_lines(bytes: &[u8], from: usize, to: Option<usize>) -> &[u8] {
    let mut starts = vec![0];
    starts.extend(bytes.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i + 1));
    let start = starts.get(from - 1).copied().unwrap_or(bytes.len());
    let end = to.and_then(|t| starts.get(t).copied()).unwrap_or(bytes.len());
    &bytes[start.min(end)..end]
}

fn line_count(bytes: &[u8]) -> usize {
    let n = bytes.iter().filter(|b| **b == b'\n').count();
    if bytes.last().is_some_and(|b| *b != b'\n') { n + 1 } else { n }
}

fn read(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--lines", "-n", "--in"], false)?;
    let range = p.value(&["--lines", "-n"]).map(|v| line_range(&v)).transpose()?;
    let path = target(&ctx, &mut p, "read")?;
    let bytes = load(&path)?;
    let (from, to) = range.unwrap_or((1, None));
    let part = pick_lines(&bytes, from, to);
    if ctx.json {
        let line = json!({
            "path": path.display().to_string(),
            "text": String::from_utf8_lossy(part),
            "from": from,
            "to": to,
            "lines": line_count(&bytes),
            "bytes": bytes.len(),
        });
        writeln!(out, "{line}")?;
    } else {
        out.write_all(part)?;
    }
    Ok(0)
}

pub fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.nixa-tmp-{}-{nanos}", std::process::id()))
}

pub fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot make folder {}", dir.display()))?;
    }
    let tmp = temp_path(path);
    let result = (|| -> Result<()> {
        let mut f = fs::File::create_new(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        if let Ok(meta) = fs::metadata(path) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("cannot write {}", path.display()))
}

fn write(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--in"], false)?;
    let force = p.flag(&["--force", "-f"]);
    let append = p.flag(&["--append", "-a"]);
    let path = target(&ctx, &mut p, "write")?;
    if path.is_dir() {
        return Err(refuse(ErrorCode::BadRequest, format!("{} is a folder", path.display())));
    }
    let exists = path.exists();
    if exists && !force && !append {
        let msg = format!("{} already exists; add --force to replace it or --append to add to it", path.display());
        return Err(refuse(ErrorCode::BadRequest, msg));
    }
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err(usage("pipe the text in, for example: echo hi | nixa file write PATH"));
    }
    let mut new = Vec::new();
    stdin.lock().read_to_end(&mut new)?;
    let mut bytes = if append && exists { load(&path)? } else { Vec::new() };
    bytes.extend_from_slice(&new);
    save(&path, &bytes)?;
    if ctx.json {
        let line = json!({
            "path": path.display().to_string(),
            "bytes": new.len(),
            "created": !exists,
            "appended": append && exists,
        });
        writeln!(out, "{line}")?;
    } else {
        let verb = if append && exists { "added" } else { "wrote" };
        writeln!(out, "{verb} {} bytes to {}", new.len(), path.display())?;
    }
    Ok(0)
}

pub fn replace(text: &str, old: &str, new: &str, all: bool) -> std::result::Result<(String, usize), String> {
    if old.is_empty() {
        return Err("--old is empty".to_string());
    }
    let count = text.matches(old).count();
    match count {
        0 => Err("the --old text was not found".to_string()),
        1 => Ok((text.replacen(old, new, 1), 1)),
        n if all => Ok((text.replace(old, new), n)),
        n => Err(format!(
            "the --old text was found {n} times; add --all to change all, or give more text so it matches once"
        )),
    }
}

fn edit(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--old", "--new", "--in"], false)?;
    let old = p.value(&["--old"]).ok_or_else(|| usage("nixa file edit needs --old TEXT"))?;
    let new = p.value(&["--new"]).ok_or_else(|| usage("nixa file edit needs --new TEXT"))?;
    let all = p.flag(&["--all"]);
    let path = target(&ctx, &mut p, "edit")?;
    let bytes = load(&path)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| refuse(ErrorCode::BadRequest, format!("{} is not UTF-8 text", path.display())))?;
    let (changed, count) = replace(&text, &old, &new, all)
        .map_err(|m| refuse(ErrorCode::BadRequest, format!("{}: {m}", path.display())))?;
    save(&path, changed.as_bytes())?;
    if ctx.json {
        let line = json!({ "path": path.display().to_string(), "changes": count });
        writeln!(out, "{line}")?;
    } else {
        let word = if count == 1 { "change" } else { "changes" };
        writeln!(out, "{count} {word} in {}", path.display())?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(line_range("2:4").unwrap(), (2, Some(4)));
        assert_eq!(line_range("3:").unwrap(), (3, None));
        assert_eq!(line_range(":2").unwrap(), (1, Some(2)));
        assert_eq!(line_range("5").unwrap(), (5, Some(5)));
        assert!(line_range("0:2").is_err());
        assert!(line_range("4:2").is_err());
        assert!(line_range("a:b").is_err());
        let text = b"one\ntwo\nthree\nfour";
        assert_eq!(pick_lines(text, 2, Some(3)), b"two\nthree\n");
        assert_eq!(pick_lines(text, 4, None), b"four");
        assert_eq!(pick_lines(text, 3, Some(99)), b"three\nfour");
        assert_eq!(pick_lines(text, 9, None), b"");
        assert_eq!(line_count(text), 4);
    }

    #[test]
    fn edit_matching() {
        assert_eq!(replace("a b a", "b", "c", false).unwrap(), ("a c a".to_string(), 1));
        assert!(replace("a b a", "a", "c", false).unwrap_err().contains("2 times"));
        assert_eq!(replace("a b a", "a", "c", true).unwrap(), ("c b c".to_string(), 2));
        assert!(replace("a b a", "z", "c", true).unwrap_err().contains("not found"));
        assert!(replace("a", "", "c", true).is_err());
    }

    #[test]
    fn atomic_save() {
        let dir = std::env::temp_dir().join(format!("nixa-file-test-{}", std::process::id()));
        let path = dir.join("deep/inner/note.txt");
        save(&path, b"first").unwrap();
        save(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        let names: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("note.txt")]);
        assert!(temp_path(&path).parent() == path.parent());
        fs::remove_dir_all(&dir).unwrap();
    }
}
