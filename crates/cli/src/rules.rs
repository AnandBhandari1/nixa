use crate::args;
use crate::ctx::{self, Ctx};
use crate::help;
use crate::http;
use crate::settings::rules as answers;
use crate::task::trees;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::io::Write;

pub const WORDS: &str = "show, standing, answers or trees";

fn wants_help(argv: &[String]) -> bool {
    argv.first().is_some_and(|a| matches!(a.as_str(), "help" | "-h" | "--help"))
}

fn emit(ctx: &Ctx, out: &mut dyn Write, value: &Value, text: &str) -> Result<i32> {
    if ctx.json {
        writeln!(out, "{}", serde_json::to_string(value)?)?;
    } else {
        writeln!(out, "{text}")?;
    }
    Ok(0)
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    if wants_help(argv) {
        out.write_all(help::RULES.as_bytes())?;
        return Ok(0);
    }
    let what = argv.first().map(String::as_str).filter(|w| !w.starts_with('-')).unwrap_or("show");
    let rest = if argv.first().is_some_and(|w| w == what) { &argv[1..] } else { argv };
    match what {
        "show" | "ls" | "list" => show(ctx, rest, out),
        "standing" => standing(ctx, rest, out),
        "answers" | "answer" | "answer-rules" => answers::run(ctx, rest, out),
        "trees" | "tree" => trees::run(ctx, rest, out),
        other => Err(args::usage(format!("nixa rules {other} is not known. Use {WORDS}"))),
    }
}

pub fn standing_lines(reply: &Value) -> String {
    let text = reply.get("text").and_then(Value::as_str).unwrap_or_default();
    let hash = reply.get("hash").and_then(Value::as_str).unwrap_or_default();
    let mut lines = vec![format!("Standing rules (rules.json, hash {hash}):")];
    lines.push(if text.trim().is_empty() { "No standing rules yet.".into() } else { text.to_string() });
    if let Some(problem) = reply.get("problem").and_then(Value::as_str) {
        lines.push(format!("Problem: {problem}"));
    }
    lines.join("\n")
}

fn show(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let standing = http::window_call("GET", "/content/rules", None)?;
    let rules = http::window_call("GET", "/answer-rules", None)?;
    let tree_list = http::get("/trees")?.get("trees").cloned().unwrap_or(Value::Null);
    let tree_lines = match tree_list.as_array() {
        Some(list) if !list.is_empty() => list.iter().map(trees::tree_line).collect::<Vec<_>>().join("\n"),
        _ => "No rule trees.".to_string(),
    };
    let text = [
        standing_lines(&standing),
        answers::list_lines(&rules).join("\n"),
        format!("Rule trees:\n{tree_lines}"),
    ]
    .join("\n\n");
    let all = json!({"standing": standing, "answers": rules, "trees": tree_list});
    emit(&ctx, out, &all, &text)
}

fn standing(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let what = argv.first().map(String::as_str).filter(|w| !w.starts_with('-')).unwrap_or("show");
    let rest = if argv.first().is_some_and(|w| w == what) { &argv[1..] } else { argv };
    let (mut p, ctx) = ctx::command(ctx, rest, &["--file", "--hash"], false)?;
    match what {
        "show" | "read" => {
            p.finish(0, false)?;
            let reply = http::window_call("GET", "/content/rules", None)?;
            emit(&ctx, out, &reply, &standing_lines(&reply))
        }
        "set" | "save" => {
            let file = p.value(&["--file"]).ok_or_else(|| args::usage("rules standing set needs --file F with the complete rules.json text"))?;
            let hash = p.value(&["--hash"]).ok_or_else(|| args::usage("rules standing set needs --hash H from nixa rules standing show"))?;
            p.finish(0, false)?;
            let text = std::fs::read_to_string(&file).map_err(|e| anyhow!("cannot read {file}: {e}"))?;
            let reply = http::window_call("POST", "/content/rules", Some(&json!({"text": text, "expectedHash": hash})))?;
            let saved = reply.get("hash").and_then(Value::as_str).unwrap_or_default();
            emit(&ctx, out, &reply, &format!("Standing rules saved. New hash {saved}."))
        }
        other => Err(args::usage(format!("rules standing takes show or set, not {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_lines_show_text_hash_and_problem() {
        let text = standing_lines(&json!({"text": "{\"rules\": [\"short\"]}", "hash": "ab12"}));
        assert!(text.starts_with("Standing rules (rules.json, hash ab12):"));
        assert!(text.contains("short"));
        let empty = standing_lines(&json!({"text": "", "hash": "x", "problem": "broken"}));
        assert!(empty.contains("No standing rules yet."));
        assert!(empty.ends_with("Problem: broken"));
    }

    #[test]
    fn unknown_words_are_refused() {
        let ctx = Ctx { socket: std::path::PathBuf::from("/nonexistent"), json: false };
        let mut out = Vec::new();
        let err = run(&ctx, &["nope".into()], &mut out).unwrap_err();
        assert!(err.to_string().contains(WORDS), "{err}");
        run(&ctx, &["help".into()], &mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("rules standing set --file F --hash H"));
    }
}
