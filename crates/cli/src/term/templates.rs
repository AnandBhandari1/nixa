use crate::ctx::{self, CALL_TIMEOUT, Ctx};
use anyhow::{Result, bail};
use nixa_protocol::{CreateSpec, DEFAULT_SEND_TIMEOUT_MS, Reply, Request, TermId, TermInfo};
use nixa_store::agent_catalog;
use nixa_store::agents::{CustomAgent, StartProfile, configure_start_in, start_command_in};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

const READY_QUIET: Duration = Duration::from_millis(1500);
const READY_MAX: Duration = Duration::from_secs(60);
const READY_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub key: String,
    pub name: String,
    pub run: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub key: String,
    pub title: String,
    pub argv: Vec<String>,
    pub rules: Option<String>,
}

fn words(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub fn merge_programs(base: Vec<Program>, text: Option<&str>) -> Vec<Program> {
    let mut list = base;
    let data = text.and_then(|t| serde_json::from_str::<Value>(t).ok());
    let Some(entries) = data.as_ref().and_then(|d| d.get("programs")).and_then(Value::as_object) else {
        return list;
    };
    for (key, entry) in entries {
        let name = entry.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty());
        let run = words(entry.get("run"));
        match list.iter_mut().find(|p| &p.key == key) {
            Some(found) => {
                if let Some(name) = name {
                    found.name = name.to_string();
                }
                if !run.is_empty() {
                    found.run = run;
                }
            }
            None if !run.is_empty() => list.push(Program {
                key: key.clone(),
                name: name.unwrap_or(key).to_string(),
                run,
            }),
            None => {}
        }
    }
    list
}

pub fn programs(dir: &Path) -> Vec<Program> {
    let base = agent_catalog::load(dir)
        .into_iter()
        .map(|a| Program { key: a.key, name: a.name, run: a.run })
        .collect();
    let text = std::fs::read_to_string(dir.join(nixa_store::agents::PROGRAMS_FILE)).ok();
    merge_programs(base, text.as_deref())
}

fn customs(dir: &Path) -> Vec<CustomAgent> {
    let db = dir.join(nixa_store::DB_FILE);
    if !db.is_file() {
        return Vec::new();
    }
    nixa_store::open(&db)
        .and_then(|conn| nixa_store::agents::list_agents(&conn))
        .unwrap_or_default()
}

pub fn profile_argv(dir: &Path, start: &StartProfile) -> Option<Vec<String>> {
    let mut argv: Vec<String> = start_command_in(dir, &start.agent)?.split_whitespace().map(str::to_string).collect();
    if let Some(flag) = nixa_store::agents::model_flag(&start.agent).filter(|_| !start.model.is_empty()) {
        argv.push(flag.into());
        argv.push(start.model.clone());
    }
    argv.extend(nixa_store::agents::effort_words(&start.agent, &start.effort));
    argv.extend(start.args.iter().cloned());
    Some(argv)
}

fn custom_argv(dir: &Path, agent: &CustomAgent) -> Option<Vec<String>> {
    match &agent.start {
        Some(start) => profile_argv(dir, start),
        None => start_command_in(dir, &agent.provider).map(|line| line.split_whitespace().map(str::to_string).collect()),
    }
}

pub fn rules(agent: &CustomAgent) -> String {
    let mut text = format!("You are {}, a custom agent of Nixa.", agent.title);
    if !agent.purpose.trim().is_empty() {
        text.push_str(&format!(" Your purpose: {}", agent.purpose.trim()));
    }
    if !agent.instructions.trim().is_empty() {
        text.push_str(&format!("\n\nYour standing instructions:\n{}", agent.instructions.trim()));
    }
    if let Some(block) = nixa_store::agents::skills_block(agent, &nixa_protocol::paths::data_dir()) {
        text.push_str(&format!("\n\n{block}"));
    }
    text
}

pub fn pick(customs: &[CustomAgent], programs: &[Program], word: &str) -> Option<(Option<CustomAgent>, Option<Program>)> {
    let key = word.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    let custom = customs
        .iter()
        .find(|a| a.id.to_lowercase() == key)
        .or_else(|| customs.iter().find(|a| !a.off && a.title.to_lowercase() == key));
    if let Some(agent) = custom {
        return Some((Some(agent.clone()), None));
    }
    programs
        .iter()
        .find(|p| p.key.to_lowercase() == key)
        .or_else(|| programs.iter().find(|p| p.name.to_lowercase() == key))
        .map(|p| (None, Some(p.clone())))
}

pub fn find(dir: &Path, word: &str) -> Result<Option<Template>> {
    let Some(found) = pick(&customs(dir), &programs(dir), word) else {
        return Ok(None);
    };
    let template = match found {
        (Some(agent), _) => {
            let Some(argv) = custom_argv(dir, &agent) else {
                bail!("custom agent {} ({}) has no start profile, so it cannot start here. Set one with nixa task agents edit {} --agent claude", agent.id, agent.title, agent.id);
            };
            Template { key: agent.id.clone(), title: agent.title.clone(), argv, rules: Some(rules(&agent)) }
        }
        (None, Some(program)) => Template { key: program.key, title: program.name, argv: program.run, rules: None },
        (None, None) => return Ok(None),
    };
    Ok(Some(template))
}

pub fn first_message(template: &Template, task: &str) -> Option<String> {
    let task = task.trim();
    match (&template.rules, task.is_empty()) {
        (Some(rules), true) => Some(rules.clone()),
        (Some(rules), false) => Some(format!("{rules}\n\n{task}")),
        (None, true) => None,
        (None, false) => Some(task.to_string()),
    }
}

pub fn list(ctx: &Ctx, out: &mut dyn Write) -> Result<i32> {
    let dir = nixa_protocol::paths::data_dir();
    let customs = customs(&dir);
    let programs = programs(&dir);
    if ctx.json {
        let customs: Vec<Value> = customs
            .iter()
            .map(|a| json!({"key": a.id, "title": a.title, "purpose": a.purpose, "off": a.off, "run": custom_argv(&dir, a)}))
            .collect();
        let programs: Vec<Value> = programs
            .iter()
            .map(|p| json!({"key": p.key, "name": p.name, "run": p.run, "installed": installed(p)}))
            .collect();
        writeln!(out, "{}", json!({"custom_agents": customs, "programs": programs}))?;
        return Ok(0);
    }
    writeln!(out, "Custom agents (nixa.db):")?;
    if customs.is_empty() {
        writeln!(out, "  none yet. Add one with: nixa task agents add --title NAME --agent claude")?;
    }
    for a in &customs {
        let how = match custom_argv(&dir, a) {
            Some(argv) => argv.join(" "),
            None => "no start profile".to_string(),
        };
        let off = if a.off { "  (off)" } else { "" };
        writeln!(out, "  {:<6} {}{off}  [{how}]", a.id, a.title)?;
    }
    writeln!(out, "Programs (programs.json):")?;
    for p in &programs {
        let mark = if installed(p) { "" } else { "  (not installed)" };
        writeln!(out, "  {:<16} {}  [{}]{mark}", p.key, p.name, p.run.join(" "))?;
    }
    writeln!(out, "Start one with: nixa term run TEMPLATE [TASK...]")?;
    Ok(0)
}

fn installed(p: &Program) -> bool {
    p.run.first().is_some_and(|program| agent_catalog::installed(program))
}

fn wait_ready(ctx: &Ctx, id: &TermId) -> Result<()> {
    let mut client = ctx::connect(ctx, Some(CALL_TIMEOUT))?;
    let began = Instant::now();
    let mut last = (0u64, Instant::now());
    while began.elapsed() < READY_MAX {
        let term = match client.call(&Request::Get { id: id.clone() })? {
            Reply::Term { term } => term,
            other => return Err(ctx::unexpected(&other)),
        };
        if !term.running {
            bail!("{id} ended before it was ready. Read it with: nixa term read {id} --history");
        }
        if term.seq != last.0 {
            last = (term.seq, Instant::now());
        } else if term.seq > 0 && last.1.elapsed() >= READY_QUIET {
            return Ok(());
        }
        sleep(READY_POLL);
    }
    Ok(())
}

pub fn start(ctx: &Ctx, template: &Template, mut spec: CreateSpec, task: &str) -> Result<(TermInfo, bool)> {
    let dir = nixa_protocol::paths::data_dir();
    let mut cmd = template.argv.clone();
    configure_start_in(&dir, &mut cmd);
    spec.cmd = cmd;
    if spec.name.is_none() {
        spec.name = Some(template.title.clone());
    }
    spec.labels.entry("template".into()).or_insert_with(|| template.key.clone());
    let term = match ctx::call(ctx, &Request::Create { spec })? {
        Reply::Term { term } => term,
        other => return Err(ctx::unexpected(&other)),
    };
    let Some(text) = first_message(template, task) else {
        return Ok((term, false));
    };
    wait_ready(ctx, &term.id)?;
    let req = Request::Send { id: term.id.clone(), text, submit: true, timeout_ms: None, force: false };
    let wait = Duration::from_millis(DEFAULT_SEND_TIMEOUT_MS).checked_add(CALL_TIMEOUT);
    ctx::call_with(ctx, &req, wait)?;
    if !ctx.json {
        eprintln!("nixa: {} started from template {} and got its task", term.id, template.key);
    }
    Ok((term, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, title: &str, off: bool) -> CustomAgent {
        CustomAgent {
            id: id.into(),
            title: title.into(),
            purpose: "check code".into(),
            instructions: "Be short.".into(),
            role: String::new(),
            job: String::new(),
            workflow: String::new(),
            start: None,
            off,
            migrated_from: String::new(),
            provider: String::new(),
            session: String::new(),
            space: String::new(),
            space_name: String::new(),
            folder: String::new(),
            terminal: String::new(),
            created_at: 0,
            last_used: 0,
            by: String::new(),
            skills: Vec::new(),
        }
    }

    fn prog(key: &str, name: &str) -> Program {
        Program { key: key.into(), name: name.into(), run: vec![key.into()] }
    }

    #[test]
    fn programs_json_overrides_and_adds() {
        let text = r#"{"programs":{"claude":{"run":["claude","--verbose"]},"mine":{"name":"Mine","run":["mine-cli"]},"empty":{}}}"#;
        let list = merge_programs(vec![prog("claude", "Claude Code")], Some(text));
        assert_eq!(list[0].run, vec!["claude", "--verbose"]);
        assert_eq!(list[1].name, "Mine");
        assert_eq!(list.len(), 2);
        assert_eq!(merge_programs(vec![prog("a", "A")], Some("not json")).len(), 1);
    }

    #[test]
    fn custom_agents_win_then_programs() {
        let customs = vec![agent("a1", "Reviewer", false), agent("a2", "Claude", true)];
        let programs = vec![prog("claude", "Claude Code")];
        assert_eq!(pick(&customs, &programs, "A1").unwrap().0.unwrap().id, "a1");
        assert_eq!(pick(&customs, &programs, "reviewer").unwrap().0.unwrap().id, "a1");
        assert_eq!(pick(&customs, &programs, "claude").unwrap().1.unwrap().key, "claude");
        assert_eq!(pick(&customs, &programs, "claude code").unwrap().1.unwrap().key, "claude");
        assert!(pick(&customs, &programs, "sh").is_none());
    }

    #[test]
    fn first_message_joins_rules_and_task() {
        let custom = Template { key: "a1".into(), title: "R".into(), argv: vec![], rules: Some(rules(&agent("a1", "Reviewer", false))) };
        let plain = Template { rules: None, ..custom.clone() };
        let text = first_message(&custom, "fix it").unwrap();
        assert!(text.starts_with("You are Reviewer, a custom agent of Nixa. Your purpose: check code"));
        assert!(text.ends_with("Be short.\n\nfix it"));
        assert_eq!(first_message(&plain, " "), None);
        assert_eq!(first_message(&plain, "go").as_deref(), Some("go"));
    }

    #[test]
    fn profile_adds_model_and_effort() {
        let dir = Path::new("/nonexistent-nixa-dir");
        let start = StartProfile { agent: "codex".into(), model: "m5".into(), effort: "high".into(), args: vec![] };
        let argv = profile_argv(dir, &start).unwrap();
        assert!(argv.windows(2).any(|w| w == ["-m", "m5"]));
        assert!(argv.contains(&"model_reasoning_effort=high".to_string()));
        assert!(profile_argv(dir, &StartProfile { agent: "nope".into(), ..start }).is_none());
    }
}
