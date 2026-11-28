use super::{emit, flag_at, on_off, on_word, secret, text_at};
use crate::args;
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::Result;
use serde_json::{Map, Value, json};
use std::io::Write;

pub const PROVIDERS: [&str; 2] = ["searxng", "exa"];
pub const KEY_PROVIDER: &str = "exa";

pub fn line(web: &Value) -> String {
    let searx = web.get("searxng").cloned().unwrap_or(Value::Null);
    let provider = text_at(web, "provider");
    let detail = if provider == KEY_PROVIDER {
        format!("Exa key: {}", if flag_at(web, "exa_key") { "saved" } else { "none" })
    } else {
        format!(
            "SearXNG {}, {}",
            if flag_at(&searx, "installed") { "installed" } else { "not installed" },
            if flag_at(&searx, "running") { "running" } else { "not running" }
        )
    };
    format!("Web search: {} (provider {}, {detail})", on_off(flag_at(web, "on")), super::switches::or_unknown(provider))
}

pub fn provider_of(word: &str) -> Result<String> {
    let word = word.trim().to_lowercase();
    if PROVIDERS.contains(&word.as_str()) {
        Ok(word)
    } else {
        Err(args::usage("the provider is searxng or exa"))
    }
}

pub fn set_body(on: Option<bool>, provider: Option<&str>) -> Result<Value> {
    let mut body = Map::new();
    body.insert("type".into(), json!("set_web"));
    if let Some(on) = on {
        body.insert("on".into(), json!(on));
    }
    if let Some(p) = provider {
        body.insert("provider".into(), json!(provider_of(p)?));
    }
    Ok(Value::Object(body))
}

pub fn key_provider(word: &str) -> Result<()> {
    if provider_of(word)? != KEY_PROVIDER {
        return Err(args::usage("only exa takes a key. Local SearXNG needs no key"));
    }
    Ok(())
}

pub fn key_body(key: &str) -> Value {
    json!({"type": "web_key", "key": key})
}

fn send(ctx: &Ctx, out: &mut dyn Write, body: &Value, done: &str) -> Result<i32> {
    let reply = http::window_call("POST", "/web", Some(body))?;
    let web = reply.get("web").cloned().unwrap_or(Value::Null);
    emit(ctx, out, &web, &format!("{done}\n{}", line(&web)))
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--provider"], false)?;
    let provider = p.value(&["--provider"]);
    let remove = p.flag(&["--remove"]);
    if p.pos.first().is_some_and(|w| w == "key") {
        p.finish(2, false)?;
        let name = p.pos.get(1).ok_or_else(|| args::usage("web key needs a provider: nixa settings web key exa"))?;
        key_provider(name)?;
        if remove {
            return send(&ctx, out, &json!({"type": "web_key_remove"}), "The Exa key is removed.");
        }
        let key = secret::read_hidden("Exa key (hidden): ")?;
        return send(&ctx, out, &key_body(&key), "The Exa key is saved in the keychain.");
    }
    if remove {
        return Err(args::usage("--remove goes with: web key PROVIDER --remove"));
    }
    p.finish(1, false)?;
    let on = p.pos.first().map(|w| on_word(w, "web")).transpose()?;
    if on.is_none() && provider.is_none() {
        let reply = http::window_call("GET", "/web", None)?;
        let web = reply.get("web").cloned().unwrap_or(Value::Null);
        return emit(&ctx, out, &web, &line(&web));
    }
    send(&ctx, out, &set_body(on, provider.as_deref())?, "Saved.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_bodies() {
        assert_eq!(set_body(Some(true), None).unwrap(), json!({"type": "set_web", "on": true}));
        assert_eq!(set_body(None, Some("EXA")).unwrap(), json!({"type": "set_web", "provider": "exa"}));
        assert!(set_body(None, Some("google")).is_err());
        assert!(key_provider("exa").is_ok());
        assert!(key_provider("searxng").is_err());
        assert_eq!(key_body("k"), json!({"type": "web_key", "key": "k"}));
    }

    #[test]
    fn web_line() {
        let web = json!({"on": true, "provider": "exa", "exa_key": true});
        assert_eq!(line(&web), "Web search: on (provider exa, Exa key: saved)");
        let local = json!({"on": false, "provider": "searxng", "searxng": {"installed": false, "running": false}});
        assert_eq!(line(&local), "Web search: off (provider searxng, SearXNG not installed, not running)");
    }
}
