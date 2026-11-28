use super::{item_line, print, words};
use crate::args;
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;
use std::time::{Duration, Instant};

const WAIT_MS: u64 = 180_000;
const POLL: Duration = Duration::from_millis(500);
const TIMEOUT_CODE: i32 = 124;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--timeout"], false)?;
    let no_wait = p.flag(&["--no-wait"]);
    let wait_ms = p.value(&["--timeout"]).map(|v| args::duration_ms(&v)).transpose()?.unwrap_or(WAIT_MS);
    p.finish(usize::MAX, true)?;
    let text = words(&p.pos, &p.rest)?;
    let before = last_id(&http::get("/chat")?);
    http::post("/chat", &body(&text))?;
    if no_wait {
        return print(&ctx, out, &json!({"ok": true, "sent": true}), "Sent.");
    }
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    let mut saw_busy = false;
    loop {
        std::thread::sleep(POLL);
        let reply = http::get("/chat")?;
        let busy = reply.get("busy").and_then(Value::as_bool) == Some(true);
        saw_busy |= busy;
        let replies = new_replies(&reply, before);
        if done(busy, saw_busy, &replies) {
            let lines: Vec<String> = replies.iter().map(item_line).collect();
            let line = if lines.is_empty() { "Nixa ended the turn with no reply.".to_string() } else { lines.join("\n") };
            return print(&ctx, out, &json!({"ok": true, "replies": replies}), &line);
        }
        if Instant::now() >= deadline {
            let line = format!(
                "No reply within {}s. It may still come: nixa chat history --last 3",
                wait_ms / 1000
            );
            print(&ctx, out, &json!({"ok": false, "error": line, "replies": replies}), &line)?;
            return Ok(TIMEOUT_CODE);
        }
    }
}

pub fn body(text: &str) -> Value {
    json!({"text": text, "typed": true})
}

pub fn last_id(reply: &Value) -> u64 {
    reply
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().filter_map(|i| i.get("id").and_then(Value::as_u64)).max())
        .unwrap_or(0)
}

pub fn new_replies(reply: &Value, after: u64) -> Vec<Value> {
    reply
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|i| i.get("id").and_then(Value::as_u64).is_some_and(|id| id > after))
                .filter(|i| i.get("who").and_then(Value::as_str) == Some("nixa"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub fn done(busy: bool, saw_busy: bool, replies: &[Value]) -> bool {
    !busy && (saw_busy || !replies.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_typed_text() {
        assert_eq!(body("hi"), json!({"text": "hi", "typed": true}));
    }

    #[test]
    fn only_new_nixa_items_count() {
        let reply = json!({"items": [
            {"id": 4, "who": "nixa", "text": "old"},
            {"id": 5, "who": "user", "text": "hi"},
            {"id": 6, "who": "info", "text": "note"},
            {"id": 7, "who": "nixa", "text": "new"},
        ]});
        assert_eq!(last_id(&reply), 7);
        assert_eq!(last_id(&json!({"items": []})), 0);
        let got = new_replies(&reply, 4);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["text"], "new");
    }

    #[test]
    fn waits_until_the_turn_ends() {
        let one = vec![json!({"id": 1})];
        assert!(!done(false, false, &[]));
        assert!(!done(true, true, &one));
        assert!(done(false, false, &one));
        assert!(done(false, true, &[]));
    }
}
