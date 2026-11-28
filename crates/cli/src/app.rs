use crate::args::usage;
use crate::ctx::Ctx;
use crate::help;
use anyhow::{Result, anyhow};
use nixa_protocol::app_words;
use nixa_protocol::control::{self, AppCommand, ControlReply, ControlRequest};
use serde_json::{Map, Value, json};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub const ASK_WAIT: Duration = Duration::from_secs(15);
pub const RUN_WAIT: Duration = Duration::from_secs(160);
pub const NOT_OPEN: &str = "The Nixa window is not open, so nothing was run. Open Nixa and try again.";

#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    State,
    Actions,
    Run { command: String, args: Value },
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let json = ctx.json || argv.iter().any(|a| a == "--json");
    let argv: Vec<String> = argv.iter().filter(|a| *a != "--json").cloned().collect();
    let asks_help = argv.first().is_none_or(|a| matches!(a.as_str(), "help" | "-h" | "--help"));
    if asks_help {
        out.write_all(help::APP.as_bytes())?;
        return Ok(if argv.is_empty() { 2 } else { 0 });
    }
    let job = parse(&argv)?;
    let socket = control::app_socket_path();
    match ask(&socket, &job) {
        Ok(reply) => print(&job, reply, json, out),
        Err(e) => fail(json, &format!("{e:#}"), out),
    }
}

pub fn parse(argv: &[String]) -> Result<Job> {
    let Some((first, rest)) = argv.split_first() else {
        return Err(usage("nixa app needs a command, see nixa help app"));
    };
    match first.as_str() {
        "state" if rest.is_empty() => Ok(Job::State),
        "actions" if rest.is_empty() => Ok(Job::Actions),
        "do" => do_job(rest),
        _ => word_job(argv),
    }
}

fn do_job(rest: &[String]) -> Result<Job> {
    let Some((command, given)) = rest.split_first() else {
        return Err(usage("nixa app do needs an action name, see nixa app actions"));
    };
    Ok(Job::Run { command: command.clone(), args: args_of(given)? })
}

pub fn args_of(given: &[String]) -> Result<Value> {
    if let [one] = given
        && one.trim_start().starts_with('{')
    {
        let value: Value = serde_json::from_str(one).map_err(|e| usage(format!("the arguments are not JSON: {e}")))?;
        return match value {
            Value::Object(_) => Ok(value),
            _ => Err(usage("the arguments must be a JSON object")),
        };
    }
    let mut map = Map::new();
    for pair in given {
        let (key, value) = pair
            .split_once('=')
            .filter(|(k, _)| !k.trim().is_empty())
            .ok_or_else(|| usage(format!("{pair:?} is not KEY=VALUE or a JSON object")))?;
        map.insert(key.trim().to_string(), json!(value));
    }
    Ok(if map.is_empty() { Value::Null } else { Value::Object(map) })
}

fn word_job(argv: &[String]) -> Result<Job> {
    let joined = argv.join(" ");
    let (word, rest) = app_words::find(argv).ok_or_else(|| usage(format!("unknown command nixa app {joined}")))?;
    let args = match (word.arg, rest) {
        (None, []) => Value::Null,
        (None, extra) => return Err(usage(format!("nixa app {} takes no more words, got {}", word.say, extra.join(" ")))),
        (Some(key), []) => return Err(usage(format!("nixa app {} needs {}", word.say, key.to_uppercase()))),
        (Some(key), text) => json!({ key: text.join(" ") }),
    };
    Ok(Job::Run { command: word.command.to_string(), args })
}

pub fn request(job: &Job) -> ControlRequest {
    match job {
        Job::State => ControlRequest::State,
        Job::Actions => ControlRequest::Commands,
        Job::Run { command, args } => ControlRequest::Run {
            command: command.clone(),
            args: args.clone(),
            cleared: false,
            token: String::new(),
        },
    }
}

fn ask(socket: &Path, job: &Job) -> Result<ControlReply> {
    if UnixStream::connect(socket).is_err() {
        return Err(anyhow!(NOT_OPEN));
    }
    let wait = if matches!(job, Job::Run { .. }) { RUN_WAIT } else { ASK_WAIT };
    control::call(socket, &request(job), wait)
}

fn fail(json: bool, message: &str, out: &mut dyn Write) -> Result<i32> {
    if json {
        writeln!(out, "{}", json!({"ok": false, "error": message}))?;
    } else {
        eprintln!("nixa: {message}");
    }
    Ok(1)
}

pub fn action_line(c: &AppCommand) -> String {
    let keys: Vec<String> = c.args["properties"]
        .as_object()
        .map(|p| p.keys().map(|k| format!("{k}=")).collect())
        .unwrap_or_default();
    let keys = if keys.is_empty() { String::new() } else { format!(" [{}]", keys.join(" ")) };
    format!("{}{keys}  ({})  {}", c.name, c.permission.as_str(), c.title)
}

fn print(job: &Job, reply: ControlReply, json: bool, out: &mut dyn Write) -> Result<i32> {
    match (job, reply) {
        (Job::State, ControlReply::State { state }) => {
            writeln!(out, "{}", serde_json::to_string_pretty(&state)?)?;
        }
        (Job::Actions, ControlReply::Commands { commands }) if json => {
            writeln!(out, "{}", serde_json::to_string(&commands)?)?;
        }
        (Job::Actions, ControlReply::Commands { commands }) => {
            for c in &commands {
                writeln!(out, "{}", action_line(c))?;
            }
        }
        (Job::Run { .. }, ControlReply::Ran { command, focus, asked }) if json => {
            writeln!(out, "{}", json!({"ok": true, "command": command, "focus": focus, "asked": asked}))?;
        }
        (Job::Run { .. }, ControlReply::Ran { command, asked, .. }) => {
            let note = if asked { " (the user said Run)" } else { "" };
            writeln!(out, "ok: {command}{note}")?;
        }
        (_, other) => {
            let shown = serde_json::to_string(&other).unwrap_or_default();
            return fail(json, &format!("unexpected reply from the Nixa window: {shown}"), out);
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn ran(command: &str, args: Value) -> Job {
        Job::Run { command: command.into(), args }
    }

    #[test]
    fn friendly_words_become_window_commands() {
        assert_eq!(parse(&v(&["state"])).unwrap(), Job::State);
        assert_eq!(parse(&v(&["actions"])).unwrap(), Job::Actions);
        assert_eq!(parse(&v(&["tab", "new"])).unwrap(), ran("tab.new", Value::Null));
        assert_eq!(parse(&v(&["pane", "split", "right"])).unwrap(), ran("app.split-right", Value::Null));
        assert_eq!(parse(&v(&["tab", "rename", "build", "logs"])).unwrap(), ran("tab.rename", json!({"name": "build logs"})));
        assert_eq!(parse(&v(&["tab", "go", "t3"])).unwrap(), ran("tab.select", json!({"id": "t3"})));
        assert_eq!(parse(&v(&["events", "clear"])).unwrap(), ran("events.clear-events", Value::Null));
        assert_eq!(parse(&v(&["events"])).unwrap(), ran("events.show-events", Value::Null));
        assert_eq!(parse(&v(&["layout", "rm", "wide"])).unwrap(), ran("layout.delete", json!({"name": "wide"})));
    }

    #[test]
    fn wrong_words_are_usage_errors() {
        for bad in [&["tab", "rename"][..], &["sidebar", "now"], &["pane", "split"], &["nope"], &["state", "x"], &["do"]] {
            let err = parse(&v(bad)).unwrap_err();
            assert!(err.downcast_ref::<crate::args::UsageError>().is_some(), "{bad:?}");
        }
    }

    #[test]
    fn do_takes_json_or_key_value_pairs() {
        assert_eq!(parse(&v(&["do", "panel.hide"])).unwrap(), ran("panel.hide", Value::Null));
        assert_eq!(parse(&v(&["do", "panel.show", "panel=git"])).unwrap(), ran("panel.show", json!({"panel": "git"})));
        let job = parse(&v(&["do", "tab.new", r#"{"argv":["htop"],"name":"top"}"#])).unwrap();
        assert_eq!(job, ran("tab.new", json!({"argv": ["htop"], "name": "top"})));
        assert!(parse(&v(&["do", "x", "[1]"])).is_err());
        assert!(parse(&v(&["do", "x", "{bad"])).is_err());
        assert!(parse(&v(&["do", "x", "=y"])).is_err());
    }

    #[test]
    fn a_cli_run_never_claims_the_brain_yes() {
        let req = request(&ran("app.close-pane", Value::Null));
        assert_eq!(req, ControlRequest::Run { command: "app.close-pane".into(), args: Value::Null, cleared: false, token: String::new() });
    }

    #[test]
    fn a_closed_window_gives_a_clear_answer() {
        let missing = std::env::temp_dir().join(format!("nixa-no-app-{}.sock", std::process::id()));
        let err = ask(&missing, &Job::State).unwrap_err();
        assert_eq!(err.to_string(), NOT_OPEN);
        let mut out = Vec::new();
        assert_eq!(fail(true, NOT_OPEN, &mut out).unwrap(), 1);
        let shown: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(shown, json!({"ok": false, "error": NOT_OPEN}));
    }

    #[test]
    fn action_lines_name_the_keys() {
        let c = AppCommand {
            name: "tab.rename".into(),
            title: "Rename a tab.".into(),
            permission: control::Permission::Write,
            args: json!({"type": "object", "properties": {"name": {}, "id": {}}}),
        };
        let line = action_line(&c);
        assert!(line.starts_with("tab.rename [") && line.contains("name=") && line.ends_with("(write)  Rename a tab."), "{line}");
    }
}
