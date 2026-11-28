use crate::args::usage;
use crate::ctx::Ctx;
use crate::{help, http};
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;
use std::time::Duration;

const TEST_WAIT: Duration = Duration::from_secs(30);
const USES: [&str; 9] = ["questions", "review", "drones", "topic", "idle", "followup", "routing", "screens", "chat"];

#[derive(Debug, PartialEq)]
pub enum Call {
    Status,
    Save(Value),
    Test(Value),
}

fn on_off(word: Option<&String>) -> Result<bool> {
    match word.map(String::as_str) {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => Err(usage("give on or off")),
    }
}

pub fn parse(argv: &[String]) -> Result<Call> {
    let Some((sub, rest)) = argv.split_first() else { return Ok(Call::Status) };
    match sub.as_str() {
        "status" | "show" => Ok(Call::Status),
        "on" => Ok(Call::Save(json!({"on": true}))),
        "off" => Ok(Call::Save(json!({"on": false}))),
        "model" => {
            let name = rest.first().ok_or_else(|| usage("nixa decide model NAME"))?;
            Ok(Call::Save(json!({"model": name})))
        }
        "level" => {
            let level: f64 = rest.first().and_then(|l| l.parse().ok()).ok_or_else(|| usage("nixa decide level NUMBER, like 0.85"))?;
            Ok(Call::Save(json!({"level": level})))
        }
        "use" => {
            let name = rest.first().map(String::as_str).filter(|n| USES.contains(n)).ok_or_else(|| usage("nixa decide use questions|review|drones|topic|idle|followup|routing|screens|chat on|off"))?;
            Ok(Call::Save(json!({name: on_off(rest.get(1))?})))
        }
        "test" => test_body(rest).map(Call::Test),
        other => Err(usage(format!("unknown command nixa decide {other}"))),
    }
}

fn test_body(rest: &[String]) -> Result<Value> {
    let mut body = serde_json::Map::new();
    let mut words = Vec::new();
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| iter.next().cloned().ok_or_else(|| usage(format!("{name} needs a value")));
        match arg.as_str() {
            "--yes-no" => {
                body.insert("yes_no".into(), json!(value("--yes-no")?));
            }
            "--choices" => {
                let list: Vec<String> = value("--choices")?.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
                body.insert("choices".into(), json!(list));
            }
            "--question" => {
                body.insert("question".into(), json!(value("--question")?));
            }
            "--model" => {
                body.insert("model".into(), json!(value("--model")?));
            }
            "--image" => {
                let path = std::path::PathBuf::from(value("--image")?);
                let path = if path.is_absolute() { path } else { std::env::current_dir()?.join(path) };
                body.insert("image".into(), json!(path.display().to_string()));
            }
            "--json" => {}
            flag if flag.starts_with("--") => return Err(usage(format!("nixa decide test does not know {flag}"))),
            word => words.push(word.to_string()),
        }
    }
    if words.is_empty() {
        return Err(usage("nixa decide test TEXT --yes-no STATEMENT or --choices A,B"));
    }
    if !body.contains_key("yes_no") && !body.contains_key("choices") {
        return Err(usage("give --yes-no STATEMENT or --choices A,B"));
    }
    body.insert("text".into(), json!(words.join(" ")));
    Ok(Value::Object(body))
}

fn print_status(reply: &Value, out: &mut dyn Write) -> Result<()> {
    let s = &reply["settings"];
    let flag = |k: &str| if s[k].as_bool().unwrap_or(false) { "on" } else { "off" };
    writeln!(out, "Route: {}", reply["route"].as_str().unwrap_or("?"))?;
    writeln!(out, "Model: {}   sure enough level: {}", s["model"].as_str().unwrap_or("?"), s["level"])?;
    writeln!(out, "Uses: agent questions {}, task review {}, drone steps {}, topic guard {}, close finished terminals {}, close or keep {}, task routing {}, quiet agent check {}, chat route {}", flag("questions"), flag("review"), flag("drones"), flag("topic"), flag("idle"), flag("followup"), flag("routing"), flag("screens"), flag("chat"))?;
    writeln!(out, "OpenCode server: {}", reply["server"].as_str().unwrap_or("starts when needed"))?;
    let t = &reply["today"];
    writeln!(out, "Today: {} calls, {} failed, ${:.4}", t["calls"].as_u64().unwrap_or(0), t["failed"].as_u64().unwrap_or(0), t["cost"].as_f64().unwrap_or(0.0))?;
    Ok(())
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    if matches!(argv.first().map(String::as_str), Some("help" | "-h" | "--help")) {
        out.write_all(help::DECIDE.as_bytes())?;
        return Ok(0);
    }
    let argv: Vec<String> = argv.iter().filter(|a| *a != "--json").cloned().collect();
    let reply = match parse(&argv)? {
        Call::Status => http::window_call("GET", "/decide", None)?,
        Call::Save(body) => http::window_call("POST", "/decide", Some(&body))?,
        Call::Test(body) => {
            let reply = http::window_call_within("POST", "/decide/test", Some(&body), TEST_WAIT)?;
            if !ctx.json {
                writeln!(out, "{} answers: {}", reply["model"].as_str().unwrap_or("?"), reply["answers"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default())?;
                writeln!(out, "Sure enough (level {}): {}   cost ${:.5}", reply["level"], if reply["sure"].as_bool().unwrap_or(false) { "yes" } else { "no" }, reply["cost"].as_f64().unwrap_or(0.0))?;
                return Ok(0);
            }
            reply
        }
    };
    if ctx.json {
        writeln!(out, "{reply}")?;
    } else {
        print_status(&reply, out)?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect()
    }

    #[test]
    fn reads_each_command() {
        assert_eq!(parse(&[]).unwrap(), Call::Status);
        assert_eq!(parse(&words("on")).unwrap(), Call::Save(json!({"on": true})));
        assert_eq!(parse(&words("off")).unwrap(), Call::Save(json!({"on": false})));
        assert_eq!(parse(&words("model gemini-decision-1")).unwrap(), Call::Save(json!({"model": "gemini-decision-1"})));
        assert_eq!(parse(&words("level 0.9")).unwrap(), Call::Save(json!({"level": 0.9})));
        assert_eq!(parse(&words("use review off")).unwrap(), Call::Save(json!({"review": false})));
        assert!(parse(&words("use money on")).is_err());
        assert!(parse(&words("use review maybe")).is_err());
        assert!(parse(&words("level high")).is_err());
        assert!(parse(&words("fly")).is_err());
    }

    #[test]
    fn reads_a_test() {
        let mut argv = words("test Allow edit? 1 Yes 2 No --choices 1,2 --question");
        argv.push("Which option?".into());
        let Call::Test(body) = parse(&argv).unwrap() else { panic!() };
        assert_eq!(body, json!({"text": "Allow edit? 1 Yes 2 No", "choices": ["1", "2"], "question": "Which option?"}));
        assert!(parse(&words("test hello")).is_err());
        assert!(parse(&words("test --yes-no x")).is_err());
        assert!(parse(&words("test hi --rules x")).is_err());
        let Call::Test(body) = parse(&words("test Is it the login page? --yes-no x --image /tmp/shot.png")).unwrap() else { panic!() };
        assert_eq!(body["image"], json!("/tmp/shot.png"));
    }
}
