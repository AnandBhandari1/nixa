use crate::models::{Prepared, Program, prepare_start};
use anyhow::{Result, bail};
use nixa_protocol::{CreateSpec, TermId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const AGENTS_FILE: &str = "agents.json";

const BUILT_IN: [(&str, &str, &str); 10] = [
    ("claude", "Claude Code", "claude"),
    ("codex", "Codex", "codex"),
    ("omp", "Oh My Pi", "omp"),
    ("opencode", "OpenCode", "opencode"),
    ("cursor", "Cursor Agent", "cursor-agent"),
    ("gemini", "Gemini CLI", "gemini"),
    ("agy", "Antigravity", "agy"),
    ("pi", "Pi", "pi"),
    ("amp", "Amp", "amp"),
    ("copilot", "Copilot CLI", "copilot"),
];

const EXTRA_DIRS: [&str; 4] = ["~/.local/bin", "/opt/homebrew/bin", "/usr/local/bin", "~/.cargo/bin"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub key: String,
    pub name: String,
    pub run: Vec<String>,
}

#[derive(Deserialize, Default)]
struct AgentsFile {
    #[serde(default)]
    agents: BTreeMap<String, AgentEntry>,
}

#[derive(Deserialize, Default)]
struct AgentEntry {
    name: Option<String>,
    #[serde(default)]
    run: Vec<String>,
}

fn built_in() -> Vec<Agent> {
    BUILT_IN
        .iter()
        .map(|(key, name, program)| Agent {
            key: key.to_string(),
            name: name.to_string(),
            run: vec![program.to_string()],
        })
        .collect()
}

fn merge(mut list: Vec<Agent>, mine: AgentsFile) -> Vec<Agent> {
    for (key, entry) in mine.agents {
        match list.iter_mut().find(|a| a.key == key) {
            Some(agent) => {
                if let Some(name) = entry.name {
                    agent.name = name;
                }
                if !entry.run.is_empty() {
                    agent.run = entry.run;
                }
            }
            None if !entry.run.is_empty() => list.push(Agent {
                name: entry.name.unwrap_or_else(|| key.clone()),
                key,
                run: entry.run,
            }),
            None => {}
        }
    }
    list
}

pub fn load(data_dir: &Path) -> Vec<Agent> {
    let mine = std::fs::read_to_string(data_dir.join(AGENTS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<AgentsFile>(&text).ok())
        .unwrap_or_default();
    merge(built_in(), mine)
}

fn expand(dir: &str) -> PathBuf {
    match (dir.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(dir),
    }
}

fn is_program(path: &Path) -> bool {
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub fn installed(program: &str) -> bool {
    if program.contains('/') {
        return is_program(&expand(program));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(EXTRA_DIRS.iter().map(|d| expand(d)))
        .any(|dir| is_program(&dir.join(program)))
}

fn find<'a>(list: &'a [Agent], wanted: &str) -> Option<&'a Agent> {
    let wanted = wanted.trim().to_lowercase();
    list.iter()
        .find(|a| a.key.to_lowercase() == wanted)
        .or_else(|| list.iter().find(|a| a.name.to_lowercase() == wanted))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartOptions {
    pub agent: String,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub name: Option<String>,
    pub task: Option<String>,
}

#[derive(Serialize)]
pub struct ListedAgent {
    pub key: String,
    pub name: String,
    pub run: Vec<String>,
    pub installed: bool,
}

pub fn list(data_dir: &Path) -> Vec<ListedAgent> {
    load(data_dir)
        .into_iter()
        .map(|agent| ListedAgent {
            installed: agent.run.first().is_some_and(|program| installed(program)),
            key: agent.key,
            name: agent.name,
            run: agent.run,
        })
        .collect()
}

pub struct StartPlan {
    pub spec: CreateSpec,
    pub name: String,
    pub task: Option<String>,
    pub prepared: Option<Prepared>,
    pub note: String,
}

pub fn plan(data_dir: &Path, args: StartOptions, parent: Option<TermId>) -> Result<StartPlan> {
    let list = load(data_dir);
    let Some(asked) = find(&list, &args.agent) else {
        let keys: Vec<&str> = list.iter().map(|a| a.key.as_str()).collect();
        bail!("No agent \"{}\". Known: {}. Nothing was started.", args.agent, keys.join(", "));
    };
    let named = args.model.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(str::to_string);
    let (agent, moved) = installed_agent(data_dir, &list, asked, named.is_some())?;
    let task = args.task.filter(|t| !t.trim().is_empty());
    let prepared = pick(data_dir, &agent.run, task.as_deref().unwrap_or_default(), named.as_deref())?;
    let agent = prepared.as_ref().and_then(|p| runs_as(&list, &p.argv)).unwrap_or(agent);
    let mut labels = BTreeMap::new();
    labels.insert("agent".to_string(), agent.key.clone());
    let mut cmd = prepared.as_ref().map(|p| p.argv.clone()).unwrap_or_else(|| agent.run.clone());
    crate::agents::configure_start_in(data_dir, &mut cmd);
    let spec = CreateSpec {
        cmd,
        cwd: args.cwd,
        name: Some(args.name.unwrap_or_else(|| agent.name.clone())),
        labels,
        parent,
        ..CreateSpec::default()
    };
    let note = join_notes(&moved, prepared.as_ref().map(|p| p.note.as_str()).unwrap_or_default());
    Ok(StartPlan { spec, name: agent.name.clone(), task, prepared, note })
}

fn join_notes(first: &str, second: &str) -> String {
    [first, second].iter().map(|n| n.trim()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(" ")
}

fn installed_agent<'a>(data_dir: &Path, list: &'a [Agent], asked: &'a Agent, named: bool) -> Result<(&'a Agent, String)> {
    let program = asked.run.first().map(String::as_str).unwrap_or_default();
    if installed(program) {
        return Ok((asked, String::new()));
    }
    let gone = format!("{} ({program}) is not installed on this computer.", asked.name);
    if named {
        bail!("{gone} A named model is never swapped. Nothing was started.");
    }
    let order = crate::open(&data_dir.join(crate::DB_FILE)).ok().and_then(|conn| crate::agent_map::fallover(&conn).ok()).unwrap_or_default();
    match fallover_agent(list, &order, &asked.key) {
        Some(other) => Ok((other, format!("{gone} {} started instead, the next installed agent in the fall over order.", other.name))),
        None => bail!("{gone} No other installed agent is in the fall over order. Nothing was started."),
    }
}

fn fallover_agent<'a>(list: &'a [Agent], order: &[String], skip: &str) -> Option<&'a Agent> {
    crate::agent_map::after(order, skip)
        .iter()
        .filter(|key| key.as_str() != skip)
        .filter_map(|key| list.iter().find(|a| &a.key == key))
        .find(|a| a.run.first().is_some_and(|p| installed(p)))
}

fn runs_as<'a>(list: &'a [Agent], argv: &[String]) -> Option<&'a Agent> {
    let kind = crate::models::start::agent_of_argv(argv)?;
    list.iter()
        .find(|a| a.key == kind)
        .or_else(|| list.iter().find(|a| crate::models::start::agent_of_argv(&a.run) == Some(kind)))
}

fn programs(list: &[Agent]) -> Vec<Program> {
    list.iter()
        .map(|a| Program { key: a.key.clone(), run: a.run.clone(), installed: a.run.first().is_some_and(|p| installed(p)) })
        .collect()
}

fn known_agent(conn: &crate::Connection, argv: &[String]) -> bool {
    let Some(agent) = crate::models::start::agent_of_argv(argv) else { return false };
    crate::models::load(conn).map(|cache| crate::models::model_rows(&cache).iter().any(|r| r.agent == agent)).unwrap_or(false)
}

fn pick(data_dir: &Path, argv: &[String], task: &str, named: Option<&str>) -> Result<Option<Prepared>> {
    let Ok(conn) = crate::open(&data_dir.join(crate::DB_FILE)) else { return Ok(None) };
    let order = crate::agent_map::fallover(&conn).unwrap_or_default();
    let list = programs(&load(data_dir));
    match prepare_start(&conn, argv, task, named, Some(&list), &order) {
        Ok(found) => Ok(found),
        Err(error) if named.is_some() || known_agent(&conn, argv) => bail!("{error:#} Nothing was started."),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_overrides_and_adds() {
        let mine: AgentsFile = serde_json::from_str(
            r#"{"agents":{"claude":{"run":["claude","--verbose"]},"mine":{"name":"Mine","run":["mine-cli"]},"empty":{}}}"#,
        )
        .unwrap();
        let list = merge(built_in(), mine);
        let claude = find(&list, "Claude Code").unwrap();
        assert_eq!(claude.run, vec!["claude", "--verbose"]);
        assert_eq!(find(&list, "MINE").unwrap().name, "Mine");
        assert!(find(&list, "empty").is_none());
        assert_eq!(list[0].key, "claude");
    }

    #[test]
    fn a_failed_over_start_is_named_after_the_agent_that_runs() {
        let list = built_in();
        let codex = vec!["codex".to_string(), "-m".to_string(), "gpt-5".to_string()];
        assert_eq!(runs_as(&list, &codex).map(|a| a.key.as_str()), Some("codex"));
        let claude = vec!["claude".to_string(), "--model".to_string(), "x".to_string()];
        assert_eq!(runs_as(&list, &claude).map(|a| a.name.as_str()), Some("Claude Code"));
        let cursor = vec!["cursor-agent".to_string()];
        assert_eq!(runs_as(&list, &cursor).map(|a| a.key.as_str()), Some("cursor"));
        assert!(runs_as(&list, &["/bin/sh".to_string()]).is_none());
    }

    #[test]
    fn a_missing_agent_falls_over_to_the_next_installed_one() {
        let list = vec![
            Agent { key: "gone".into(), name: "Gone".into(), run: vec!["nixa-no-such-program-x".into()] },
            Agent { key: "shell".into(), name: "Shell".into(), run: vec!["/bin/sh".into()] },
        ];
        let order = vec!["gone".to_string(), "shell".to_string()];
        assert_eq!(fallover_agent(&list, &order, "gone").map(|a| a.key.as_str()), Some("shell"));
        assert!(fallover_agent(&list, &order[..1], "x").is_none());
        let named = installed_agent(Path::new("."), &list, &list[0], true).unwrap_err().to_string();
        assert!(named.contains("never swapped"), "{named}");
        assert_eq!(join_notes("a.", " b. "), "a. b.");
    }

    #[test]
    fn installed_finds_sh() {
        assert!(installed("/bin/sh"));
        assert!(installed("sh"));
        assert!(!installed("nixa-no-such-program-x"));
    }
}
