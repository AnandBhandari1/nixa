use super::words;
use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use anyhow::Result;
use serde_json::json;
use std::io::Write;

const WHO: [&str; 4] = ["user", "nixa", "info", "agent"];
const SOURCES: [&str; 4] = ["memory", "chat", "docs", "notes"];

pub fn history(ctx: &Ctx, sub: &str, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--who", "--limit", "--after"], false)?;
    let who = p.value(&["--who"]);
    let limit = p.value(&["--limit"]).map(|v| args::number::<u32>(&v, "--limit")).transpose()?;
    let after = p.value(&["--after"]).map(|v| args::number::<i64>(&v, "--after")).transpose()?;
    let mut input = match sub {
        "search" => {
            p.finish(usize::MAX, false)?;
            json!({"action": "search", "query": words(&p.pos, &[])?})
        }
        "get" => {
            p.finish(1, false)?;
            let id = p.pos.first().ok_or_else(|| usage("nixa chat history get needs a chat line id"))?;
            json!({"action": "get", "id": args::number::<i64>(id.trim_start_matches('#'), "the chat line id")?})
        }
        _ => {
            p.finish(0, false)?;
            json!({"action": "list"})
        }
    };
    if let Some(who) = who {
        if !WHO.contains(&who.as_str()) {
            return Err(usage(format!("--who must be one of {}", WHO.join(", "))));
        }
        input["who"] = json!(who);
    }
    if let Some(limit) = limit {
        input["limit"] = json!(limit);
    }
    if let Some(after) = after {
        input["after"] = json!(after);
    }
    crate::tool::call(&ctx, "nixa_chat", input, out)
}

pub fn knowledge(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--source", "--project"], false)?;
    let source = p.value(&["--source"]);
    let project = p.value(&["--project"]);
    p.finish(usize::MAX, false)?;
    let mut input = json!({"query": words(&p.pos, &[])?});
    if let Some(source) = source {
        if !SOURCES.contains(&source.as_str()) {
            return Err(usage(format!("--source must be one of {}", SOURCES.join(", "))));
        }
        input["source"] = json!(source);
    }
    if let Some(project) = project {
        input["project"] = json!(project);
    }
    crate::tool::call(&ctx, "search_knowledge", input, out)
}

pub fn memory_search(ctx: &Ctx, pos: &[String], out: &mut dyn Write) -> Result<i32> {
    crate::tool::call(ctx, "search_memory", json!({"query": pos.join(" ").trim()}), out)
}

pub fn memory_forget(ctx: &Ctx, pos: &[String], out: &mut dyn Write) -> Result<i32> {
    let text = words(pos, &[])?;
    crate::tool::call(ctx, "forget", json!({"text": text}), out)
}
