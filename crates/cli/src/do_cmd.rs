use crate::args::usage;
use crate::ctx::Ctx;
use crate::{help, http};
use anyhow::{Result, bail};
use serde_json::{Map, Value, json};
use std::io::Write;
use std::time::Duration;

const RUN_WAIT: Duration = Duration::from_secs(180);

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let sub = argv.first().map(String::as_str).unwrap_or("list");
    if matches!(sub, "help" | "-h" | "--help") {
        out.write_all(help::DO.as_bytes())?;
        return Ok(0);
    }
    let rest = if argv.is_empty() { &[][..] } else { &argv[1..] };
    let (args, ctx) = crate::ctx::command(ctx, rest, &[], false)?;
    match sub {
        "list" => list(&ctx, args.pos.first().map(String::as_str), out),
        "show" => {
            let name = args.pos.first().cloned().ok_or_else(|| usage("nixa do show NAME"))?;
            show(&ctx, &name, out)
        }
        name => {
            let input = input_of(&args.pos)?;
            call(&ctx, name, input, out)
        }
    }
}

pub fn value_of(text: &str) -> Value {
    let t = text.trim();
    if (t.starts_with('{') || t.starts_with('['))
        && let Ok(v) = serde_json::from_str::<Value>(t)
    {
        return v;
    }
    match t {
        "true" => return json!(true),
        "false" => return json!(false),
        _ => {}
    }
    if let Ok(n) = t.parse::<i64>() {
        return json!(n);
    }
    if let Ok(n) = t.parse::<f64>()
        && n.is_finite()
        && t.chars().any(|c| c.is_ascii_digit())
    {
        return json!(n);
    }
    json!(text)
}

fn put(map: &mut Map<String, Value>, key: &str, value: Value) -> Result<()> {
    match key.split_once('.') {
        None => {
            map.insert(key.to_string(), value);
            Ok(())
        }
        Some((head, tail)) => {
            let inner = map.entry(head.to_string()).or_insert_with(|| json!({}));
            let Some(inner) = inner.as_object_mut() else {
                bail!("{head} is already a value, not an object");
            };
            put(inner, tail, value)
        }
    }
}

pub fn input_of(pos: &[String]) -> Result<Value> {
    if let [only] = pos
        && only.trim().starts_with('{')
    {
        let v: Value = serde_json::from_str(only).map_err(|e| usage(format!("the input must be a JSON object: {e}")))?;
        if !v.is_object() {
            return Err(usage("the input must be a JSON object"));
        }
        return Ok(v);
    }
    let mut map = Map::new();
    for pair in pos {
        let (key, value) = pair.split_once('=').ok_or_else(|| usage(format!("{pair} is not key=value")))?;
        let key = key.trim();
        if key.is_empty() || key.split('.').any(str::is_empty) {
            return Err(usage(format!("{pair} has an empty key")));
        }
        put(&mut map, key, value_of(value))?;
    }
    Ok(Value::Object(map))
}

fn actions(word: Option<&str>) -> Result<Vec<Value>> {
    let path = match word {
        Some(w) => format!("/actions?word={}", encode(w)),
        None => "/actions".to_string(),
    };
    let reply = http::window_call("GET", &path, None)?;
    Ok(reply.get("actions").and_then(Value::as_array).cloned().unwrap_or_default())
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn list(ctx: &Ctx, word: Option<&str>, out: &mut dyn Write) -> Result<i32> {
    let all = actions(word)?;
    if ctx.json {
        writeln!(out, "{}", Value::Array(all))?;
        return Ok(0);
    }
    for a in &all {
        let name = a["name"].as_str().unwrap_or("");
        let kind = a["kind"].as_str().unwrap_or("");
        writeln!(out, "{name:<32} {kind:<6} {}", a["help"].as_str().unwrap_or(""))?;
    }
    if all.is_empty() {
        writeln!(out, "No actions match.")?;
    }
    writeln!(out, "\nRun one: nixa do NAME key=value. Details: nixa do show NAME.")?;
    Ok(0)
}

fn show(ctx: &Ctx, name: &str, out: &mut dyn Write) -> Result<i32> {
    let all = actions(None)?;
    let Some(a) = all.iter().find(|a| a["name"].as_str() == Some(name)) else {
        bail!("No Nixa action named {name}. Run nixa do list.");
    };
    if ctx.json {
        writeln!(out, "{a}")?;
        return Ok(0);
    }
    writeln!(
        out,
        "{name} ({})\n{} {}\n\n{}\n\nInput:\n{}",
        a["kind"].as_str().unwrap_or(""),
        a["method"].as_str().unwrap_or(""),
        a["path"].as_str().unwrap_or(""),
        a["help"].as_str().unwrap_or(""),
        serde_json::to_string_pretty(&a["inputs"])?
    )?;
    Ok(0)
}

fn call(ctx: &Ctx, name: &str, input: Value, out: &mut dyn Write) -> Result<i32> {
    let reply = http::window_call_within("POST", "/actions/run", Some(&json!({"name": name, "input": input})), RUN_WAIT)?;
    if ctx.json {
        writeln!(out, "{reply}")?;
    } else {
        writeln!(out, "{}", serde_json::to_string_pretty(&reply["reply"])?)?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_become_typed_nested_input() {
        let pos: Vec<String> = ["repeat.text=check in", "repeat.every=15", "repeat.enabled=false", "id=rep-1", "list=[1,2]"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            input_of(&pos).unwrap(),
            json!({"repeat": {"text": "check in", "every": 15, "enabled": false}, "id": "rep-1", "list": [1, 2]})
        );
    }

    #[test]
    fn one_json_object_is_the_whole_input() {
        assert_eq!(input_of(&["{\"id\": \"a\"}".to_string()]).unwrap(), json!({"id": "a"}));
        assert!(input_of(&["noequals".to_string()]).is_err());
        assert!(input_of(&["a=1".to_string(), "a.b=2".to_string()]).is_err());
    }
}
