use crate::args::{self, Parsed, usage};
use crate::ctx::{self, CALL_TIMEOUT, Ctx};
use crate::{help, http, show};
use anyhow::Result;
use nixa_protocol::{
    CreateSpec, DEFAULT_SEND_TIMEOUT_MS, Key, ReadSource, Reply, Request, Status, TermId, WaitFor,
    WaitOutcome, paths,
};
use std::collections::BTreeMap;
use std::io::{IsTerminal, Read, Write};
use std::path::Path;
use std::time::Duration;

mod attach;
mod put;
mod templates;
mod wait_run;

pub const FLOW_ROUTE: &str = "/universe/flow";
pub const FLOW_TIMEOUT: Duration = Duration::from_millis(80);
pub const EXIT_TIMEOUT: i32 = 124;
pub const EXIT_ENDED: i32 = 3;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::TERM.as_bytes())?;
        return Ok(2);
    };
    let asks_help = matches!(sub.as_str(), "help" | "-h" | "--help")
        || rest.first().is_some_and(|a| a == "-h" || a == "--help");
    if asks_help {
        out.write_all(help::TERM.as_bytes())?;
        return Ok(0);
    }
    match sub.as_str() {
        "ls" | "list" => ls(ctx, rest, out),
        "get" => get(ctx, rest, out),
        "new" => new(ctx, rest, out, false),
        "run" => new(ctx, rest, out, true),
        "attach" => with_parsed(ctx, rest, out, attach::run),
        "put" => with_parsed(ctx, rest, out, put::run),
        "templates" => {
            let (p, ctx) = ctx::command(ctx, rest, &[], false)?;
            p.finish(0, false)?;
            templates::list(&ctx, out)
        }
        "parent" => clear_parent(ctx, rest, out),
        "kill" => simple(ctx, rest, out, |id| Request::Kill { id }, "killed"),
        "rm" | "remove" => simple(ctx, rest, out, |id| Request::Remove { id }, "removed"),
        "read" => read(ctx, rest, out),
        "send" => send(ctx, rest, out),
        "keys" => keys(ctx, rest, out),
        "interrupt" => interrupt(ctx, rest, out),
        "wait" => wait(ctx, rest, out),
        "report" => report(ctx, rest, out),
        "reports" => reports(ctx, rest, out),
        "label" => label(ctx, rest, out),
        "rename" => rename(ctx, rest, out),
        "status" => status(ctx, rest, out),
        "keep" => keep(ctx, rest, out),
        "events" => events(ctx, rest, out),
        other => Err(usage(format!("unknown command nixa term {other}"))),
    }
}

fn with_parsed(
    ctx: &Ctx,
    argv: &[String],
    out: &mut dyn Write,
    act: fn(&Ctx, Parsed, &mut dyn Write) -> Result<i32>,
) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    act(&ctx, p, out)
}

pub fn term_id(s: &str) -> Result<TermId> {
    if s == "." {
        return paths::current_term_id()
            .ok_or_else(|| usage("\".\" means this terminal, but NIXA_TERM_ID is not set"));
    }
    s.parse::<TermId>().map_err(|e| usage(e.to_string()))
}

fn one_id(p: &Parsed, what: &str) -> Result<TermId> {
    match p.pos.first() {
        Some(s) => term_id(s),
        None => Err(usage(format!("nixa term {what} needs a terminal id"))),
    }
}

fn parse_status(s: &str) -> Result<Status> {
    s.parse::<Status>().map_err(usage)
}

fn words(p: &Parsed, skip: usize) -> Vec<String> {
    p.pos.iter().skip(skip).chain(p.rest.iter()).cloned().collect()
}

fn ok_line(ctx: &Ctx, out: &mut dyn Write, reply: Reply, line: String) -> Result<i32> {
    if ctx.json {
        return ctx::print_json(out, &reply);
    }
    match reply {
        Reply::Ok => {
            writeln!(out, "{line}")?;
            Ok(0)
        }
        other => Err(ctx::unexpected(&other)),
    }
}

fn ls(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--parent", "--label", "-l"], false)?;
    let parent = p.value(&["--parent"]).map(|s| term_id(&s)).transpose()?;
    let label = p.value(&["--label", "-l"]);
    let all = p.flag(&["--all", "-a"]);
    p.finish(0, false)?;
    let reply = ctx::call(&ctx, &Request::List { parent: parent.clone(), label })?;
    let finished = if all { finished_children(&ctx, &reply, parent.as_ref()) } else { Vec::new() };
    if ctx.json && all {
        let rows: Vec<_> = finished
            .iter()
            .map(|(parent, r)| serde_json::json!({"id": r.child, "parent": parent, "note": r.note}))
            .collect();
        let terms = match &reply {
            Reply::Terms { terms } => serde_json::to_value(terms)?,
            other => return Err(ctx::unexpected(other)),
        };
        writeln!(out, "{}", serde_json::json!({"terms": terms, "finished": rows}))?;
        return Ok(0);
    }
    if ctx.json {
        return ctx::print_json(out, &reply);
    }
    match reply {
        Reply::Terms { terms } if terms.is_empty() && finished.is_empty() => writeln!(out, "no terminals")?,
        Reply::Terms { terms } if terms.is_empty() => {}
        Reply::Terms { terms } => out.write_all(show::term_table(&terms).as_bytes())?,
        other => return Err(ctx::unexpected(&other)),
    }
    if !finished.is_empty() {
        writeln!(out, "\nFinished child terminals (output saved in the parent for 30 minutes):")?;
        let mut rows = vec![vec!["ID".to_string(), "PARENT".to_string(), "NOTE".to_string()]];
        for (parent, r) in &finished {
            rows.push(vec![r.child.to_string(), parent.to_string(), r.note.clone()]);
        }
        out.write_all(show::table(&rows).as_bytes())?;
        writeln!(out, "Read one with: nixa term reports PARENT --output ID")?;
    }
    Ok(0)
}

fn finished_children(ctx: &Ctx, reply: &Reply, only: Option<&TermId>) -> Vec<(TermId, nixa_protocol::ChildReport)> {
    let Reply::Terms { terms } = reply else {
        return Vec::new();
    };
    let mut parents: Vec<TermId> = match only {
        Some(id) => vec![id.clone()],
        None => terms.iter().filter(|t| t.parent.is_none()).map(|t| t.id.clone()).collect(),
    };
    parents.dedup();
    let mut found = Vec::new();
    for parent in parents {
        let Ok(Reply::ChildReports { reports }) = ctx::call(ctx, &Request::ChildReports { id: parent.clone(), reader: None }) else {
            continue;
        };
        found.extend(ended_reports(reports, terms).into_iter().map(|r| (parent.clone(), r)));
    }
    found
}

pub fn ended_reports(reports: Vec<nixa_protocol::ChildReport>, live: &[nixa_protocol::TermInfo]) -> Vec<nixa_protocol::ChildReport> {
    let mut out: Vec<nixa_protocol::ChildReport> = Vec::new();
    for r in reports.into_iter().rev() {
        let alive = live.iter().any(|t| t.id == r.child);
        if r.exited && !alive && !out.iter().any(|o| o.child == r.child) {
            out.push(r);
        }
    }
    out.reverse();
    out
}

fn get(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(1, false)?;
    let id = one_id(&p, "get")?;
    let reply = ctx::call(&ctx, &Request::Get { id })?;
    if ctx.json {
        return ctx::print_json(out, &reply);
    }
    match reply {
        Reply::Term { term } => out.write_all(show::term_detail(&term).as_bytes())?,
        other => return Err(ctx::unexpected(&other)),
    }
    Ok(0)
}

fn clear_parent(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &[], false)?;
    let none = p.flag(&["--none"]);
    p.finish(1, false)?;
    let id = one_id(&p, "parent")?;
    if !none {
        return Err(usage("use nixa term parent ID --none to clear its parent"));
    }
    let reply = ctx::call(&ctx, &Request::ClearParent { id: id.clone() })?;
    ok_line(&ctx, out, reply, format!("{id} has no parent"))
}

fn pairs(items: Vec<String>, name: &str, need_value: bool) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for item in items {
        let (k, v) = match item.split_once('=') {
            Some((k, v)) => (k, v),
            None if !need_value => (item.as_str(), ""),
            None => return Err(usage(format!("{name} needs KEY=VALUE, got {item:?}"))),
        };
        if k.is_empty() {
            return Err(usage(format!("{name} needs a key, got {item:?}")));
        }
        map.insert(k.to_string(), v.to_string());
    }
    Ok(map)
}

fn absolute(dir: &str) -> Result<String> {
    let path = Path::new(dir);
    if path.is_absolute() {
        return Ok(dir.to_string());
    }
    Ok(std::env::current_dir()?.join(path).display().to_string())
}

pub fn after_dash(argv: &[String], rest: &[String]) -> bool {
    argv.len() > rest.len() && argv[argv.len() - rest.len() - 1] == "--"
}

pub fn template_words(argv: &[String], cmd: &[String]) -> Option<(String, String)> {
    let (first, task) = cmd.split_first()?;
    if after_dash(argv, cmd) || task.first().is_some_and(|w| w.starts_with('-')) {
        return None;
    }
    Some((first.clone(), task.join(" ")))
}

fn new(ctx: &Ctx, argv: &[String], out: &mut dyn Write, run_mode: bool) -> Result<i32> {
    let mut with_value = vec![
        "--name", "-n", "--cwd", "-C", "--label", "-l", "--env", "-e", "--parent", "--rows", "--cols",
    ];
    if run_mode {
        with_value.extend(["--timeout", "-t"]);
    }
    let (mut p, ctx) = ctx::command(ctx, argv, &with_value, true)?;
    let name = p.value(&["--name", "-n"]);
    let cwd = match p.value(&["--cwd", "-C"]) {
        Some(dir) => Some(absolute(&dir)?),
        None => std::env::current_dir().ok().map(|d| d.display().to_string()),
    };
    let labels = pairs(p.values(&["--label", "-l"]), "--label", false)?;
    let env = pairs(p.values(&["--env", "-e"]), "--env", true)?;
    let no_parent = p.flag(&["--no-parent"]);
    let given_parent = p.value(&["--parent"]).map(|s| term_id(&s)).transpose()?;
    let rows = p.value(&["--rows"]).map(|v| args::number::<u16>(&v, "--rows")).transpose()?;
    let cols = p.value(&["--cols"]).map(|v| args::number::<u16>(&v, "--cols")).transpose()?;
    let keep = p.flag(&["--keep"]);
    let waits = run_mode && p.flag(&["--wait", "-w"]);
    let timeout_ms = match run_mode {
        true => p.value(&["--timeout", "-t"]).map(|v| args::duration_ms(&v)).transpose()?,
        false => None,
    };
    p.finish(0, true)?;
    let mut cmd = p.rest.clone();
    if run_mode && cmd.is_empty() {
        return Err(usage("nixa term run needs a command or a template, for example: nixa term run -- make test"));
    }
    let data_dir = nixa_protocol::paths::data_dir();
    let template = match template_words(argv, &cmd).filter(|_| run_mode) {
        Some((word, task)) => templates::find(&data_dir, &word)?.map(|t| (t, task)),
        None => None,
    };
    if waits && keep {
        return Err(usage("--keep starts a fresh shell when the program ends, so nixa term run --wait cannot wait. Drop one of them."));
    }
    if timeout_ms.is_some() && !waits {
        return Err(usage("--timeout works with nixa term run --wait"));
    }
    let parent = match given_parent.clone() {
        Some(id) => Some(id),
        None if no_parent || waits => None,
        None => paths::current_term_id(),
    };
    let mut spec = CreateSpec {
        cmd: Vec::new(),
        cwd,
        name,
        labels,
        parent,
        env,
        rows,
        cols,
        keep,
    };
    if let Some((template, task)) = template {
        let (term, sent) = templates::start(&ctx, &template, spec, &task)?;
        if waits {
            return wait_run::run(&ctx, &term, timeout_ms, out);
        }
        if ctx.json {
            writeln!(out, "{}", serde_json::json!({"id": term.id, "template": template.key, "sent": sent}))?;
        } else {
            writeln!(out, "{}", term.id)?;
        }
        return Ok(0);
    }
    nixa_store::agents::configure_start_in(&data_dir, &mut cmd);
    spec.cmd = cmd;
    let reply = ctx::call(&ctx, &Request::Create { spec })?;
    if waits {
        return match reply {
            Reply::Term { term } => wait_run::run(&ctx, &term, timeout_ms, out),
            other => Err(ctx::unexpected(&other)),
        };
    }
    if ctx.json {
        return ctx::print_json(out, &reply);
    }
    match reply {
        Reply::Term { term } => writeln!(out, "{}", term.id)?,
        other => return Err(ctx::unexpected(&other)),
    }
    Ok(0)
}

fn simple(
    ctx: &Ctx,
    argv: &[String],
    out: &mut dyn Write,
    make: fn(TermId) -> Request,
    done: &str,
) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(1, false)?;
    let id = one_id(&p, done)?;
    let reply = ctx::call(&ctx, &make(id.clone()))?;
    ok_line(&ctx, out, reply, format!("{id} {done}"))
}

fn read(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--lines", "-n"], false)?;
    let source = if p.flag(&["--history", "-H"]) {
        ReadSource::History
    } else {
        ReadSource::Screen
    };
    let lines = p
        .value(&["--lines", "-n"])
        .map(|v| args::number::<usize>(&v, "--lines"))
        .transpose()?;
    p.finish(1, false)?;
    let id = one_id(&p, "read")?;
    let reply = ctx::call(&ctx, &Request::Read { id, source, lines })?;
    if ctx.json {
        return ctx::print_json(out, &reply);
    }
    match reply {
        Reply::Text { text, .. } => {
            out.write_all(text.as_bytes())?;
            if !text.is_empty() && !text.ends_with('\n') {
                writeln!(out)?;
            }
        }
        other => return Err(ctx::unexpected(&other)),
    }
    Ok(0)
}

fn read_stdin(command: &str) -> Result<String> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Err(usage(format!("nixa term {command} needs text, or text piped on stdin")));
    }
    let mut text = String::new();
    stdin.read_to_string(&mut text)?;
    Ok(text)
}

fn send(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--timeout", "-t"], false)?;
    let explicit_submit = p.flag(&["--submit", "-s", "--enter"]);
    let no_submit = p.flag(&["--no-submit", "--draft"]);
    let submit = !no_submit || explicit_submit;
    let force = p.flag(&["--force", "-f"]);
    let timeout_ms = p
        .value(&["--timeout", "-t"])
        .map(|v| args::duration_ms(&v))
        .transpose()?;
    p.finish(usize::MAX, true)?;
    let id = one_id(&p, "send")?;
    let parts = words(&p, 1);
    let piped = !std::io::stdin().is_terminal();
    let text = if parts == ["-"] || (parts.is_empty() && (piped || !explicit_submit)) {
        read_stdin("send")?
    } else {
        parts.join(" ")
    };
    let wait = Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_SEND_TIMEOUT_MS));
    let client_timeout = wait.checked_add(CALL_TIMEOUT);
    let req = Request::Send {
        id,
        text,
        submit,
        timeout_ms,
        force,
    };
    let reply = ctx::call_with(&ctx, &req, client_timeout)?;
    if let Reply::Sent { id, .. } = &reply
        && let Some(body) = flow_body(paths::current_term_id().as_ref(), id)
    {
        http::notify_within(FLOW_ROUTE, &body, FLOW_TIMEOUT);
    }
    if ctx.json {
        return ctx::print_json(out, &reply);
    }
    match reply {
        Reply::Sent { id, waited_ms, forced, started, .. } => {
            let mut line = format!("{id} sent");
            if started == Some(false) {
                line.push_str(" (the agent has not confirmed it started)");
            }
            if waited_ms > 0 {
                line.push_str(&format!(" after {} draft wait", show::ms_text(waited_ms)));
            }
            if forced {
                line.push_str(" (forced)");
            }
            writeln!(out, "{line}")?;
        }
        other => return Err(ctx::unexpected(&other)),
    }
    Ok(0)
}

pub fn flow_body(from: Option<&TermId>, to: &TermId) -> Option<serde_json::Value> {
    let from = from.filter(|f| *f != to)?;
    Some(serde_json::json!({"terminal": from.to_string(), "to": to.to_string()}))
}

fn parse_key(s: &str) -> Result<Key> {
    if let Some(raw) = s.strip_prefix("raw:") {
        return Ok(Key::Raw(raw.to_string()));
    }
    s.parse::<Key>().map_err(usage)
}

fn keys(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(usize::MAX, true)?;
    let id = one_id(&p, "keys")?;
    let names = words(&p, 1);
    if names.is_empty() {
        return Err(usage("nixa term keys needs one or more keys, for example: enter, esc, ctrl+c"));
    }
    let keys = names.iter().map(|k| parse_key(k)).collect::<Result<Vec<_>>>()?;
    let reply = ctx::call(&ctx, &Request::Keys { id: id.clone(), keys })?;
    ok_line(&ctx, out, reply, format!("{id} keys sent"))
}

fn interrupt(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(1, false)?;
    let id = one_id(&p, "interrupt")?;
    let reply = ctx::call(&ctx, &Request::Interrupt { id })?;
    if ctx.json {
        return ctx::print_json(out, &reply);
    }
    match reply {
        Reply::Interrupted { report } => out.write_all(show::interrupt_report(&report).as_bytes())?,
        other => return Err(ctx::unexpected(&other)),
    }
    Ok(0)
}

fn wait(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let with_value = ["--status", "--text", "--regex", "--timeout", "-t"];
    let (mut p, ctx) = ctx::command(ctx, argv, &with_value, false)?;
    let mut picks = Vec::new();
    if p.flag(&["--idle"]) {
        picks.push(WaitFor::Idle);
    }
    if let Some(s) = p.value(&["--status"]) {
        picks.push(WaitFor::Status { status: parse_status(&s)? });
    }
    if let Some(pattern) = p.value(&["--text"]) {
        picks.push(WaitFor::Text { pattern, regex: false });
    }
    if let Some(pattern) = p.value(&["--regex"]) {
        picks.push(WaitFor::Text { pattern, regex: true });
    }
    if p.flag(&["--children"]) {
        picks.push(WaitFor::Children);
    }
    if p.flag(&["--wait-any"]) {
        picks.push(WaitFor::ChildAny);
    }
    if p.flag(&["--wait-all"]) {
        picks.push(WaitFor::ChildAll);
    }
    if p.flag(&["--exit"]) {
        picks.push(WaitFor::Exit);
    }
    if picks.len() > 1 {
        return Err(usage("pick one of --idle, --status, --text, --regex, --children, --wait-any, --wait-all, --exit"));
    }
    let until = picks.pop().unwrap_or(WaitFor::Idle);
    let timeout_ms = p
        .value(&["--timeout", "-t"])
        .map(|v| args::duration_ms(&v))
        .transpose()?;
    p.finish(1, false)?;
    let id = one_id(&p, "wait")?;
    let for_exit = matches!(until, WaitFor::Exit);
    let reply = ctx::call_with(&ctx, &Request::Wait { id, until, timeout_ms }, None)?;
    let result = match &reply {
        Reply::Waited { result } => result.clone(),
        other => return Err(ctx::unexpected(other)),
    };
    let code = match result.outcome {
        WaitOutcome::Met => 0,
        WaitOutcome::Timeout => EXIT_TIMEOUT,
        WaitOutcome::Exited if for_exit => 0,
        WaitOutcome::Exited => EXIT_ENDED,
    };
    if ctx.json {
        ctx::print_json(out, &reply)?;
    } else {
        writeln!(out, "{}", show::wait_result(&result))?;
    }
    Ok(code)
}

fn report(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(usize::MAX, true)?;
    let id = paths::current_term_id()
        .ok_or_else(|| usage("nixa term report runs inside a child terminal"))?;
    let parent = std::env::var(paths::ENV_PARENT_ID).ok()
        .map(|value| value.parse::<TermId>().map_err(|e| usage(e.to_string())))
        .transpose()?;
    let parts = words(&p, 0);
    let text = if parts == ["-"] {
        read_stdin("report")?
    } else {
        parts.join(" ")
    };
    if text.trim().is_empty() {
        return Err(usage("nixa term report needs text or - to read stdin"));
    }
    let reply = ctx::call(&ctx, &Request::ChildReport { id, parent, text })?;
    ok_line(&ctx, out, reply, "Reported to parent".into())
}

fn reports(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--output"], false)?;
    let child = p.value(&["--output"]).map(|value| term_id(&value)).transpose()?;
    p.finish(1, false)?;
    let id = one_id(&p, "reports")?;
    let request = match child {
        Some(child) => Request::ChildOutput { id, child },
        None => Request::ChildReports { reader: nixa_protocol::paths::current_term_id().filter(|me| me == &id), id },
    };
    let reply = ctx::call(&ctx, &request)?;
    if ctx.json {
        return ctx::print_json(out, &reply);
    }
    match reply {
        Reply::ChildReports { reports } => {
            for report in reports {
                writeln!(out, "{}", report.note)?;
            }
        }
        Reply::Text { text, .. } if !text.is_empty() => writeln!(out, "{text}")?,
        Reply::Text { .. } => {}
        other => return Err(ctx::unexpected(&other)),
    }
    Ok(0)
}

fn label(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--rm", "-d"], false)?;
    let removes = p.values(&["--rm", "-d"]);
    p.finish(usize::MAX, false)?;
    let id = one_id(&p, "label")?;
    let sets = pairs(words(&p, 1), "label", false)?;
    let mut client = ctx::connect(&ctx, Some(CALL_TIMEOUT))?;
    for (key, value) in sets {
        client.call(&Request::LabelSet { id: id.clone(), key, value: Some(value) })?;
    }
    for key in removes {
        client.call(&Request::LabelSet { id: id.clone(), key, value: None })?;
    }
    let reply = client.call(&Request::Get { id: id.clone() })?;
    if ctx.json {
        return ctx::print_json(out, &reply);
    }
    match reply {
        Reply::Term { term } if term.labels.is_empty() => writeln!(out, "{id} has no labels")?,
        Reply::Term { term } => writeln!(out, "{id} labels: {}", show::labels_text(&term.labels))?,
        other => return Err(ctx::unexpected(&other)),
    }
    Ok(0)
}

fn rename(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(usize::MAX, true)?;
    let id = one_id(&p, "rename")?;
    let name = words(&p, 1).join(" ");
    if name.is_empty() {
        return Err(usage("nixa term rename needs a new name"));
    }
    let reply = ctx::call(&ctx, &Request::Rename { id: id.clone(), name: name.clone() })?;
    ok_line(&ctx, out, reply, format!("{id} renamed to {name}"))
}

fn status(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(usize::MAX, true)?;
    let mut parts = words(&p, 0);
    let first_is_id = parts.first().is_some_and(|w| w == "." || w.parse::<TermId>().is_ok());
    let id = if first_is_id {
        term_id(&parts.remove(0))?
    } else {
        paths::current_term_id().ok_or_else(|| {
            usage("nixa term status needs a terminal id, or NIXA_TERM_ID set inside a nixa terminal")
        })?
    };
    let Some(state) = parts.first() else {
        return Err(usage("give a status: working, blocked, done, idle, error or clear"));
    };
    let status = if state == "clear" { None } else { Some(parse_status(state)?) };
    let note = parts[1..].join(" ");
    let note = (!note.is_empty()).then_some(note);
    if status.is_none() && note.is_some() {
        return Err(usage("clear takes no note"));
    }
    let line = match status {
        Some(s) => format!("{id} status {s}"),
        None => format!("{id} status cleared"),
    };
    let reply = ctx::call(&ctx, &Request::StatusSet { id, status, note })?;
    ok_line(&ctx, out, reply, line)
}

fn keep(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(2, false)?;
    let id = one_id(&p, "keep")?;
    let on = match p.pos.get(1).map(String::as_str) {
        None | Some("on") => true,
        Some("off") => false,
        Some(other) => return Err(usage(format!("keep takes on or off, got {other:?}"))),
    };
    let reply = ctx::call(&ctx, &Request::Keep { id: id.clone(), on })?;
    ok_line(&ctx, out, reply, format!("{id} keep {}", if on { "on" } else { "off" }))
}

fn events(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--count", "-c"], false)?;
    let output = p.flag(&["--output", "-o"]);
    let count = p
        .value(&["--count", "-c"])
        .map(|v| args::number::<usize>(&v, "--count"))
        .transpose()?;
    if count == Some(0) {
        return Err(usage("--count needs a number above 0"));
    }
    p.finish(usize::MAX, false)?;
    let ids = p.pos.iter().map(|s| term_id(s)).collect::<Result<Vec<_>>>()?;
    let client = ctx::connect(&ctx, None)?;
    let json = ctx.json;
    let mut seen = 0usize;
    let mut write_err = None;
    client.subscribe(ids, output, |event| {
        let line = if json {
            serde_json::to_string(&event).unwrap_or_default()
        } else {
            show::event_line(&event)
        };
        if let Err(e) = writeln!(out, "{line}").and_then(|_| out.flush()) {
            write_err = Some(e);
            return false;
        }
        seen += 1;
        count.is_none_or(|c| seen < c)
    })?;
    match write_err {
        Some(e) => Err(e.into()),
        None => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn templates_only_without_dash_or_flags() {
        let argv = strs(&["--wait", "reviewer", "fix", "the", "bug"]);
        let cmd = strs(&["reviewer", "fix", "the", "bug"]);
        assert_eq!(template_words(&argv, &cmd), Some(("reviewer".into(), "fix the bug".into())));
        let argv = strs(&["--", "claude", "hi"]);
        assert_eq!(template_words(&argv, &argv[1..]), None);
        let argv = strs(&["claude", "--model", "x"]);
        assert_eq!(template_words(&argv, &argv), None);
        assert!(after_dash(&strs(&["-n", "a", "--", "sh"]), &strs(&["sh"])));
        assert!(!after_dash(&strs(&["sh"]), &strs(&["sh"])));
    }

    #[test]
    fn only_ended_children_not_live_are_finished() {
        let report = |n: u64, exited: bool| nixa_protocol::ChildReport {
            child: TermId::from_number(n),
            status: None,
            exited,
            note: format!("m{n} note"),
            current_job_result: false, read: false,
        };
        let live: Vec<nixa_protocol::TermInfo> = vec![serde_json::from_value(serde_json::json!({
            "id": "m3", "name": "x", "cwd": "/", "running": true, "rows": 1, "cols": 2, "created_ms": 0
        })).unwrap()];
        let got = ended_reports(vec![report(2, false), report(2, true), report(2, true), report(3, true), report(4, true)], &live);
        let ids: Vec<String> = got.iter().map(|r| r.child.to_string()).collect();
        assert_eq!(ids, vec!["m2", "m4"]);
    }

    #[test]
    fn a_send_from_a_terminal_names_both_ends() {
        let m1 = TermId::from_number(1);
        let m2 = TermId::from_number(2);
        let body = flow_body(Some(&m1), &m2).unwrap();
        assert_eq!(body["terminal"], m1.to_string());
        assert_eq!(body["to"], m2.to_string());
        assert!(flow_body(None, &m2).is_none());
        assert!(flow_body(Some(&m2), &m2).is_none());
    }
}
