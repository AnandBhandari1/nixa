use super::{emit, flag_at, full_path, text_at};
use crate::args;
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;

fn count(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

pub fn status_lines(k: &Value) -> Vec<String> {
    let health = k.get("health").cloned().unwrap_or(Value::Null);
    let mut lines = vec![format!(
        "Knowledge index: {} chunks{}.",
        count(k, "chunks"),
        if flag_at(k, "indexing") { ", indexing now" } else { "" }
    )];
    let last = text_at(&health, "last_index_at");
    lines.push(if last.is_empty() {
        "Last index: not yet.".into()
    } else {
        format!("Last index: {last}, {} files, {} removed, {} failed.", count(&health, "last_index_files"), count(&health, "last_index_removed"), count(&health, "last_index_failed"))
    });
    let error = text_at(&health, "last_index_error");
    if !error.is_empty() {
        lines.push(format!("Last error: {error}"));
    }
    lines.extend(folder_lines(&strings(k, "folders")));
    lines
}

pub fn folder_lines(folders: &[String]) -> Vec<String> {
    let mut lines = vec![format!("Extra folders: {}", folders.len())];
    lines.extend(folders.iter().map(|f| format!("  {f}")));
    lines
}

pub fn folders_after(now: &[String], what: &str, given: &[String]) -> Result<Vec<String>> {
    let given: Vec<String> = given.iter().map(|d| full_path(d)).collect();
    if given.is_empty() && what != "set" {
        return Err(args::usage(format!("knowledge folders {what} needs a folder")));
    }
    let mut list = now.to_vec();
    match what {
        "add" => list.extend(given.into_iter().filter(|g| !now.contains(g))),
        "rm" | "remove" => {
            let gone: Vec<&String> = given.iter().filter(|g| !now.contains(g)).collect();
            if let Some(g) = gone.first() {
                return Err(args::usage(format!("{g} is not in the knowledge folders")));
            }
            list.retain(|f| !given.contains(f));
        }
        _ => list = given,
    }
    Ok(list)
}

fn folders(ctx: &Ctx, pos: &[String], out: &mut dyn Write) -> Result<i32> {
    let what = pos.first().map(String::as_str).unwrap_or("ls");
    let reply = http::window_call("GET", "/knowledge/folders", None)?;
    let now = strings(&reply, "folders");
    if matches!(what, "ls" | "list") {
        return emit(ctx, out, &reply, &folder_lines(&now).join("\n"));
    }
    if !matches!(what, "add" | "rm" | "remove" | "set") {
        return Err(args::usage("knowledge folders takes ls, add DIR, rm DIR or set DIR..."));
    }
    let list = folders_after(&now, what, &pos[1..])?;
    let saved = http::window_call("POST", "/knowledge/folders", Some(&json!({"folders": list})))?;
    emit(ctx, out, &saved, &format!("Saved.\n{}", folder_lines(&strings(&saved, "folders")).join("\n")))
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let what = argv.first().map(String::as_str).filter(|w| !w.starts_with('-')).unwrap_or("status");
    let rest = if argv.first().is_some_and(|w| w == what) { &argv[1..] } else { argv };
    let (p, ctx) = ctx::command(ctx, rest, &[], false)?;
    match what {
        "status" | "ls" => {
            p.finish(0, false)?;
            let reply = http::window_call("GET", "/knowledge", None)?;
            let k = reply.get("knowledge").cloned().unwrap_or(Value::Null);
            emit(&ctx, out, &k, &status_lines(&k).join("\n"))
        }
        "index" => {
            p.finish(0, false)?;
            let reply = http::window_call("POST", "/knowledge/index", Some(&json!({})))
                .map_err(|e| anyhow::anyhow!("{e:#}\nA long index keeps running in nixa-brain. Check later with: nixa settings knowledge"))?;
            let r = reply.get("report").cloned().unwrap_or(Value::Null);
            let text = format!("Indexed {} files: {} changed, {} removed, {} failed.", count(&r, "files"), count(&r, "changed"), count(&r, "removed"), count(&r, "failed"));
            emit(&ctx, out, &reply, &text)
        }
        "forget-chat" => {
            p.finish(0, false)?;
            let reply = http::window_call("POST", "/knowledge/forget_chat", Some(&json!({})))?;
            let gone = reply.get("forgot").map(Value::to_string).unwrap_or_else(|| "0".into());
            emit(&ctx, out, &reply, &format!("The chat is taken out of the knowledge index ({gone} removed)."))
        }
        "folders" => {
            p.finish(usize::MAX, false)?;
            folders(&ctx, &p.pos, out)
        }
        other => Err(args::usage(format!("knowledge takes status, index, forget-chat or folders, not {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn folder_changes() {
        let now = list(&["/a", "/b"]);
        assert_eq!(folders_after(&now, "add", &list(&["/c", "/a"])).unwrap(), list(&["/a", "/b", "/c"]));
        assert_eq!(folders_after(&now, "rm", &list(&["/a"])).unwrap(), list(&["/b"]));
        assert!(folders_after(&now, "rm", &list(&["/z"])).is_err());
        assert_eq!(folders_after(&now, "set", &[]).unwrap(), Vec::<String>::new());
        assert!(folders_after(&now, "add", &[]).is_err());
    }

    #[test]
    fn status_text() {
        let k = json!({"chunks": 12, "indexing": false, "health": {"last_index_at": "2026-01-01 10:00", "last_index_files": 4}, "folders": ["/a"]});
        let lines = status_lines(&k);
        assert_eq!(lines[0], "Knowledge index: 12 chunks.");
        assert_eq!(lines[1], "Last index: 2026-01-01 10:00, 4 files, 0 removed, 0 failed.");
        assert_eq!(lines[2], "Extra folders: 1");
    }
}
