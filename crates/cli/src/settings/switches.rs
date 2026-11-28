use super::{emit, flag_at, on_off, on_word};
use crate::args;
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;

pub const CLEARS: [&str; 2] = ["topic", "hold"];

pub fn or_unknown(text: &str) -> &str {
    if text.is_empty() { "unknown" } else { text }
}

pub fn privacy_body(on: bool) -> Value {
    json!({"type": "set_privacy", "on": on})
}

pub fn clear_body(what: &str) -> Result<Value> {
    if !CLEARS.contains(&what) {
        return Err(args::usage("keyword-clear takes topic or hold"));
    }
    Ok(json!({"type": "keyword_clear", "what": what}))
}

fn post(body: &Value) -> Result<Value> {
    let reply = http::window_call("POST", "/switches", Some(body))?;
    Ok(reply.get("switches").cloned().unwrap_or(Value::Null))
}

pub fn privacy(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(1, false)?;
    let Some(word) = p.pos.first() else {
        let reply = http::window_call("GET", "/switches", None)?;
        let switches = reply.get("switches").cloned().unwrap_or(Value::Null);
        let text = format!("Privacy: {}", on_off(flag_at(&switches, "privacy")));
        return emit(&ctx, out, &switches, &text);
    };
    let on = on_word(word, "privacy")?;
    let switches = post(&privacy_body(on))?;
    let text = format!("Privacy is now {}.", on_off(flag_at(&switches, "privacy")));
    emit(&ctx, out, &switches, &text)
}

pub fn keyword_clear(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(1, false)?;
    let what = p.pos.first().map(String::as_str).unwrap_or("topic");
    let switches = post(&clear_body(what)?)?;
    emit(&ctx, out, &switches, &format!("The {what} keyword is cleared."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies() {
        assert_eq!(privacy_body(true), json!({"type": "set_privacy", "on": true}));
        assert_eq!(clear_body("hold").unwrap(), json!({"type": "keyword_clear", "what": "hold"}));
        assert!(clear_body("all").is_err());
    }
}
