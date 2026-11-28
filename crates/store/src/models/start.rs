use super::cache::{ModelLaunch, Selection, model_id};
use super::pick::ModelRequest;
use super::rank::{ModelRow, model_rows};
use anyhow::{Result, bail};
use rusqlite::Connection;
use serde::Serialize;

const BINARIES: [(&str, &str); 9] = [
    ("claude", "claude"),
    ("codex", "codex"),
    ("omp", "omp"),
    ("agy", "agy"),
    ("agyd", "agy"),
    ("cursor-agent", "cursor"),
    ("opencode", "opencode"),
    ("gemini", "gemini"),
    ("kimi", "kimi"),
];
const SHORT_FLAG: [&str; 4] = ["codex", "opencode", "gemini", "kimi"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Program {
    pub key: String,
    pub run: Vec<String>,
    pub installed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Prepared {
    pub argv: Vec<String>,
    pub row: ModelRow,
    pub request: ModelRequest,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedPick {
    pub agent: String,
    pub model: String,
}

fn real_model(model: &str) -> bool {
    !model.is_empty() && model != "default" && model != "config"
}

fn some(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_string())
}

pub fn member_request(saved: &SavedPick, role: &str, task: Option<&str>, agent: Option<&str>, model: Option<&str>) -> ModelRequest {
    let task = Some(task.unwrap_or(role).to_string());
    let agent = agent.filter(|a| !a.is_empty());
    let model = model.filter(|m| !m.is_empty());
    let saved_agent = some(&saved.agent);
    if let Some(model) = model {
        if real_model(model) {
            let agent = match agent {
                Some(a) => Some(a.to_string()),
                None if model.contains("::") => None,
                None => saved_agent,
            };
            return ModelRequest { task, named: Some(model.to_string()), agent, ..ModelRequest::default() };
        }
        return ModelRequest { task, agent: agent.map(str::to_string).or(saved_agent), ..ModelRequest::default() };
    }
    if let Some(agent) = agent {
        return ModelRequest { task, agent: Some(agent.to_string()), ..ModelRequest::default() };
    }
    if real_model(&saved.model) {
        return ModelRequest { task, named: Some(saved.model.clone()), agent: saved_agent, ..ModelRequest::default() };
    }
    ModelRequest { task, agent: saved_agent, ..ModelRequest::default() }
}

pub fn argv_model(argv: &[String]) -> Option<String> {
    if let Some(arg) = argv.iter().find(|a| a.starts_with("--model=") || a.starts_with("-m=")) {
        return arg.split_once('=').map(|(_, v)| v.to_string());
    }
    let index = argv.iter().position(|a| a == "--model" || a == "-m")?;
    argv.get(index + 1).cloned()
}

pub fn agent_of_program(word: &str) -> Option<&'static str> {
    let base = word.rsplit('/').next().unwrap_or(word);
    BINARIES.iter().find(|(b, _)| *b == base).map(|(_, a)| *a)
}

pub fn agent_of_argv(argv: &[String]) -> Option<&'static str> {
    argv.first().and_then(|w| agent_of_program(w))
}

fn plain_run(program: &Program) -> bool {
    !program.run.iter().any(|a| a == "--continue" || a == "--resume" || a == "resume")
}

pub fn program_for<'a>(programs: &'a [Program], agent: &str) -> Option<&'a Program> {
    let mine: Vec<&Program> = programs.iter().filter(|p| p.installed && agent_of_argv(&p.run) == Some(agent)).collect();
    mine.iter()
        .find(|p| p.key == agent && plain_run(p))
        .or_else(|| mine.iter().find(|p| p.run.first() == Some(&p.key) && plain_run(p)))
        .or_else(|| mine.iter().find(|p| plain_run(p)))
        .copied()
}

pub fn fallover_agents(programs: &[Program], order: &[String]) -> Vec<String> {
    order.iter().filter(|agent| program_for(programs, agent).is_some()).cloned().collect()
}

fn set_model(argv: &[String], agent: &str, value: &str) -> Vec<String> {
    let mut next = argv.to_vec();
    if let Some(at) = next.iter().position(|a| a.starts_with("--model=") || a.starts_with("-m=")) {
        let flag = next[at].split_once('=').map(|(f, _)| f.to_string()).unwrap_or_default();
        next[at] = format!("{flag}={value}");
    } else if let Some(at) = next.iter().position(|a| a == "--model" || a == "-m") {
        if at + 1 < next.len() {
            next[at + 1] = value.to_string();
        } else {
            next.push(value.to_string());
        }
    } else {
        let flag = if SHORT_FLAG.contains(&agent) { "-m" } else { "--model" };
        let at = 1.min(next.len());
        next.insert(at, value.to_string());
        next.insert(at, flag.to_string());
    }
    next
}

pub fn prepare_start(conn: &Connection, argv: &[String], task: &str, named: Option<&str>, programs: Option<&[Program]>, order: &[String]) -> Result<Option<Prepared>> {
    let Some(agent) = agent_of_argv(argv) else {
        return Ok(None);
    };
    let named = named.filter(|n| !n.is_empty());
    let raw = if named.is_some() { None } else { argv_model(argv) };
    let asked = named.map(str::to_string).or_else(|| raw.clone().filter(|r| real_model(r)));
    let rows = if asked.is_some() { model_rows(&super::load(conn)?) } else { Vec::new() };
    let row = asked.as_deref().map(|asked| {
        let exact: Vec<&ModelRow> = rows.iter().filter(|r| r.agent == agent && (r.model == asked || r.id == asked)).collect();
        let found: Vec<&ModelRow> = if exact.is_empty() {
            rows.iter()
                .filter(|r| r.agent == agent && r.efforts.iter().any(|e| if agent == "omp" { format!("{}:{e}", r.model) } else { format!("{}-{e}", r.model) } == asked))
                .collect()
        } else {
            exact
        };
        found
    });
    if let Some(found) = &row
        && found.len() > 1
    {
        bail!("Named model {} is ambiguous; use its exact model ID.", asked.clone().unwrap_or_default());
    }
    let row: Option<ModelRow> = row.and_then(|f| f.first().map(|r| (*r).clone()));
    let request = ModelRequest {
        task: Some(task.to_string()),
        agent: Some(agent.to_string()),
        named: row.as_ref().map(|r| r.id.clone()).or_else(|| asked.clone()),
        ..ModelRequest::default()
    };
    let mut chosen = super::choose(conn, &request)?;
    let mut program: Option<Vec<String>> = None;
    if chosen.is_none()
        && asked.is_none()
        && let Some(programs) = programs
    {
        let fallover = fallover_agents(programs, order);
        if fallover.iter().any(|other| other != agent) {
            chosen = super::choose(conn, &ModelRequest { fallover, ..request.clone() })?;
            if let Some(pick) = chosen.clone()
                && pick.agent != agent
            {
                match program_for(programs, &pick.agent) {
                    Some(found) => program = Some(found.run.clone()),
                    None => chosen = None,
                }
            }
        }
    }
    let Some(chosen) = chosen.filter(|c| c.agent == agent || program.is_some()) else {
        let what = asked.map(|a| format!(" ({a})")).unwrap_or_default();
        bail!("No available model for {agent}{what}, and no other installed agent has a free model.");
    };
    let base = program.unwrap_or_else(|| argv.to_vec());
    let value = match (&row, &raw) {
        (Some(r), Some(raw)) if r.id == chosen.id => raw.clone(),
        _ => chosen.model.clone(),
    };
    let argv = set_model(&base, &chosen.agent, &value);
    let request = if chosen.agent == agent { request } else { ModelRequest { agent: Some(chosen.agent.clone()), ..request } };
    let note = chosen.note.clone();
    Ok(Some(Prepared { argv, row: chosen, request, note }))
}

pub fn record_start(conn: &Connection, prepared: &Prepared, terminal: &str, task: &str, folder: &str, space: &str, name: &str) -> Result<()> {
    if terminal.is_empty() {
        return Ok(());
    }
    let launch = ModelLaunch {
        model: model_id(&prepared.row.agent, &prepared.row.model),
        task: task.to_string(),
        selection: Selection {
            agent: prepared.request.agent.clone().unwrap_or_default(),
            named: prepared.request.named.clone().unwrap_or_default(),
        },
        folder: folder.to_string(),
        space: space.to_string(),
        name: name.to_string(),
        ..ModelLaunch::default()
    };
    super::record(conn, terminal, &launch)
}
