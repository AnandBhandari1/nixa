use crate::db::{bump_number, flag, next_number};
use crate::text::{clean_instructions, clean_line, one_line};
use crate::time::now_ms;
use anyhow::{bail, Result};
use nixa_protocol::Status;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const REVIEWER_ROLE: &str = "reviewer";
pub const TITLE_MAX: usize = 60;
pub const PURPOSE_MAX: usize = 200;
pub const INSTRUCTIONS_MAX: usize = 2000;
pub const SESSION_MAX: usize = 512;
pub const PROFILE_AGENTS: [&str; 13] = [
    "claude", "codex", "omp", "agy", "cursor", "opencode", "gemini", "jcode", "hermes", "omo",
    "crush", "dim", "kimi",
];
pub const RESUME_PROVIDERS: [&str; 13] = [
    "claude", "codex", "cursor", "agy", "omp", "hermes", "dim", "opencode", "crush", "omo",
    "jcode", "gemini", "kimi",
];
pub const START_PROVIDERS: [&str; 13] = RESUME_PROVIDERS;
const COUNTER: &str = "custom_agents";
const CLAUDE_REVIEWER: &str = "A reviewer never runs on Claude: reviewers, testers and checkers use another tool. Pick another agent, or change its start profile first.";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartProfile {
    pub agent: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomAgent {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub instructions: String,
    pub role: String,
    pub job: String,
    pub workflow: String,
    pub start: Option<StartProfile>,
    pub off: bool,
    pub migrated_from: String,
    pub provider: String,
    pub session: String,
    pub space: String,
    pub space_name: String,
    pub folder: String,
    pub terminal: String,
    pub created_at: i64,
    pub last_used: i64,
    pub by: String,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Working,
    Waiting,
    Free,
    Closed,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Waiting => "waiting",
            AgentState::Free => "free",
            AgentState::Closed => "closed",
        }
    }

    pub fn order(self) -> u8 {
        match self {
            AgentState::Free => 0,
            AgentState::Closed => 1,
            AgentState::Waiting => 2,
            AgentState::Working => 3,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AddAgent {
    pub title: String,
    pub purpose: String,
    pub instructions: String,
    pub provider: String,
    pub session: String,
    pub space: String,
    pub space_name: String,
    pub folder: String,
    pub role: String,
    pub job: String,
    pub workflow: String,
    pub start: Option<StartProfile>,
    pub off: bool,
    pub migrated_from: String,
    pub skills: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditAgent {
    pub title: Option<String>,
    pub purpose: Option<String>,
    pub instructions: Option<String>,
    pub session: Option<String>,
    pub role: Option<String>,
    pub job: Option<String>,
    pub workflow: Option<String>,
    pub profile_agent: Option<String>,
    pub profile_model: Option<String>,
    pub profile_effort: Option<String>,
    pub off: Option<bool>,
    pub skills: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentUse {
    pub terminal: Option<String>,
    pub session: Option<String>,
    pub space: Option<String>,
    pub space_name: Option<String>,
    pub folder: Option<String>,
}

static SAFE_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._:/@+-]{1,512}$").expect("valid pattern"));
static LABEL_FORMS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"^claude --resume (\S+)", "claude"),
        (r"^codex resume (\S+)", "codex"),
        (r"^cursor-agent --resume (\S+)", "cursor"),
        (r"^agyd? --conversation (\S+)", "agy"),
        (r"^omp --resume=(\S+)", "omp"),
        (r"^hermes --resume (\S+)", "hermes"),
        (r"^(?:env DIMCODE_UPDATE_CHECK=0 )?dima --resume (\S+)", "dim"),
        (r"^opencode --session (\S+)", "opencode"),
        (r"^crush --session (\S+)", "crush"),
        (r"^omo --session (\S+)", "omo"),
        (r"^jcode (?:--no-update )?--resume (\S+)", "jcode"),
        (r"^gemini --resume (\S+)", "gemini"),
        (r"^kimi --session (\S+)", "kimi"),
    ]
    .into_iter()
    .map(|(p, name)| (Regex::new(p).expect("valid pattern"), name))
    .collect()
});

pub fn safe_session(session: &str) -> bool {
    SAFE_ID.is_match(session)
}

pub const PROGRAMS_FILE: &str = "programs.json";
pub const CHECKS_OFF_KEY: &str = "checksOff";
pub const CHECKS_ON_KEY: &str = "checksOn";

const START_LINES: [(&str, &str, &str); 13] = [
    ("claude", "claude", "claude --dangerously-skip-permissions"),
    (
        "codex",
        "codex",
        "codex --dangerously-bypass-approvals-and-sandbox",
    ),
    ("cursor", "cursor-agent", "cursor-agent --yolo --trust"),
    ("agy", "agy", "agy --dangerously-skip-permissions"),
    ("omp", "omp", "omp --auto-approve"),
    ("hermes", "hermes", "hermes --yolo --accept-hooks"),
    (
        "dim",
        "env DIMCODE_UPDATE_CHECK=0 dima",
        "env DIMCODE_UPDATE_CHECK=0 dima --permission full-access",
    ),
    ("opencode", "opencode", "opencode --auto"),
    ("crush", "crush", "crush -y"),
    ("omo", "omo", "omo --permission-preset full-access"),
    ("jcode", "jcode --no-update", "jcode --no-update"),
    ("gemini", "gemini", "gemini --yolo"),
    ("kimi", "kimi", "kimi --auto"),
];

const RESUME_FORMS: [(&str, &str); 13] = [
    ("claude", "claude --resume "),
    ("codex", "codex resume "),
    ("cursor", "cursor-agent --resume "),
    ("agy", "agy --conversation "),
    ("omp", "omp --resume="),
    ("hermes", "hermes --resume "),
    ("dim", "env DIMCODE_UPDATE_CHECK=0 dima --resume "),
    ("opencode", "opencode --session "),
    ("crush", "crush --session "),
    ("omo", "omo --session "),
    ("jcode", "jcode --no-update --resume "),
    ("gemini", "gemini --resume "),
    ("kimi", "kimi --session "),
];

pub fn checks_off_from(text: Option<&str>, provider: &str) -> bool {
    let data = text.and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok());
    !data
        .as_ref()
        .and_then(|data| data.get(CHECKS_ON_KEY))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|list| list.iter().any(|item| item.as_str() == Some(provider)))
}

pub fn checks_off_in(path: &std::path::Path, provider: &str) -> bool {
    let text = std::fs::read_to_string(path).ok();
    checks_off_from(text.as_deref(), provider)
}

pub fn checks_off_for(provider: &str) -> bool {
    checks_off_in(
        &nixa_protocol::paths::data_dir().join(PROGRAMS_FILE),
        provider,
    )
}

pub fn start_command_with(provider: &str, checks_off: bool) -> Option<&'static str> {
    START_LINES
        .iter()
        .find(|(p, _, _)| *p == provider)
        .map(|(_, on, off)| if checks_off { *off } else { *on })
}

pub fn start_command(provider: &str) -> Option<&'static str> {
    start_command_with(provider, checks_off_for(provider))
}

pub fn start_command_in(data_dir: &std::path::Path, provider: &str) -> Option<&'static str> {
    start_command_with(
        provider,
        checks_off_in(&data_dir.join(PROGRAMS_FILE), provider),
    )
}

pub fn start_flags(provider: &str) -> &'static str {
    let on = start_command_with(provider, false).unwrap_or_default();
    start_command_with(provider, true)
        .and_then(|off| off.strip_prefix(on))
        .unwrap_or_default()
        .trim()
}

fn provider_of_binary(word: &str) -> Option<&'static str> {
    let name = word.rsplit('/').next()?;
    START_LINES
        .iter()
        .find(|(_, on, _)| {
            on.split_whitespace()
                .find(|word| *word != "env" && !word.contains('='))
                == Some(name)
        })
        .map(|(provider, _, _)| *provider)
        .or(match name {
            "agyd" | "antigravity" => Some("agy"),
            "pi" => Some("omp"),
            _ => None,
        })
}

pub fn configure_start(text: Option<&str>, argv: &mut Vec<String>) {
    let program = if argv.first().is_some_and(|word| word == "env") {
        argv.iter()
            .skip(1)
            .take_while(|word| word.contains('='))
            .count()
            + 1
    } else {
        0
    };
    let Some(provider) = argv.get(program).and_then(|word| provider_of_binary(word)) else {
        return;
    };
    let flags: Vec<&str> = start_flags(provider).split_whitespace().collect();
    if flags.is_empty() {
        return;
    }
    let args = program + 1;
    if argv
        .get(args)
        .is_some_and(|word| matches!(word.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return;
    }
    let allow_all = checks_off_from(text, provider);
    let mut index = args;
    while let Some(word) = argv.get(index) {
        if word == "--" {
            break;
        }
        let (flag, inline) = word
            .split_once('=')
            .map_or((word.as_str(), None), |(flag, value)| (flag, Some(value)));
        let next = argv
            .get(index + 1)
            .map(String::as_str)
            .filter(|value| !value.starts_with('-'));
        let value = inline.or(next);
        let value_len = 1 + usize::from(inline.is_none() && next.is_some());
        let remove = match (provider, flag) {
            ("dim", "--permission") | ("omo", "--permission-preset")
                if allow_all || value == Some("full-access") =>
            {
                value_len
            }
            ("codex", "--ask-for-approval" | "-a" | "--sandbox" | "-s") if allow_all => value_len,
            ("codex", "--config" | "-c")
                if allow_all
                    && value.is_some_and(|v| {
                        v.starts_with("approval_policy=") || v.starts_with("sandbox_mode=")
                    }) =>
            {
                value_len
            }
            ("claude", "--permission-mode") if allow_all || value == Some("bypassPermissions") => {
                value_len
            }
            ("gemini" | "omp", "--approval-mode") if allow_all || value == Some("yolo") => {
                value_len
            }
            ("cursor", "--mode" | "--sandbox") | ("agy", "--mode") if allow_all => value_len,
            ("cursor" | "kimi", "--plan")
            | ("agy" | "gemini", "--sandbox")
            | ("gemini", "-s")
            | ("omp", "--plan-yolo")
            | ("codex", "--full-auto")
                if allow_all =>
            {
                1
            }
            ("crush", "-yy" | "--yolo")
            | ("codex" | "kimi", "--yolo")
            | ("kimi" | "gemini", "-y") => 1,
            _ if flag.starts_with('-')
                && flags.contains(&flag)
                && !matches!(provider, "dim" | "omo") =>
            {
                1
            }
            _ => 0,
        };
        if remove == 0 {
            index += 1;
        } else {
            argv.drain(index..index + remove);
        }
    }
    if allow_all {
        argv.splice(args..args, flags.into_iter().map(str::to_owned));
    }
}

pub fn configure_start_in(dir: &std::path::Path, argv: &mut Vec<String>) {
    let text = std::fs::read_to_string(dir.join(PROGRAMS_FILE)).ok();
    configure_start(text.as_deref(), argv);
}

pub fn skips_agent_checks(argv: &[String]) -> bool {
    let words: Vec<_> = argv
        .iter()
        .flat_map(|s| s.split_whitespace())
        .map(|word| word.trim_matches(['\'', '"']))
        .collect();
    if words.iter().any(|word| {
        [
            "--dangerously-skip-permissions",
            "--dangerously-bypass-approvals-and-sandbox",
            "--yolo",
            "full-access",
            "--accept-hooks",
            "--allow-all",
            "--auto-approve",
        ]
        .iter()
        .any(|flag| word.ends_with(flag))
    }) {
        return true;
    }
    words
        .iter()
        .filter_map(|word| provider_of_binary(word))
        .any(|provider| match provider {
            "crush" => words.iter().any(|word| matches!(*word, "-y" | "-yy")),
            "opencode" | "kimi" => words.contains(&"--auto"),
            _ => false,
        })
}

pub fn resume_command_with(provider: &str, session: &str, checks_off: bool) -> Option<String> {
    if !safe_session(session) || session.starts_with('-') {
        return None;
    }
    let (_, before) = RESUME_FORMS.iter().find(|(p, _)| *p == provider)?;
    let flags = start_flags(provider);
    if checks_off && !flags.is_empty() {
        Some(format!("{before}{session} {flags}"))
    } else {
        Some(format!("{before}{session}"))
    }
}

pub fn resume_command(provider: &str, session: &str) -> Option<String> {
    resume_command_with(provider, session, checks_off_for(provider))
}

pub fn resume_command_in(
    data_dir: &std::path::Path,
    provider: &str,
    session: &str,
) -> Option<String> {
    resume_command_with(
        provider,
        session,
        checks_off_in(&data_dir.join(PROGRAMS_FILE), provider),
    )
}

fn resume_parts(command: &str) -> Option<(&'static str, &str, &str)> {
    let (provider, tail) = RESUME_FORMS
        .iter()
        .find_map(|(provider, prefix)| command.strip_prefix(prefix).map(|tail| (*provider, tail)))
        .or_else(|| {
            command
                .strip_prefix("agyd --conversation ")
                .map(|tail| ("agy", tail))
        })
        .or_else(|| {
            command
                .strip_prefix("dima --resume ")
                .map(|tail| ("dim", tail))
        })
        .or_else(|| {
            command
                .strip_prefix("jcode --resume ")
                .map(|tail| ("jcode", tail))
        })?;
    if tail.ends_with(' ') {
        return None;
    }
    let (id, flags) = tail.split_once(' ').unwrap_or((tail, ""));
    (safe_session(id)
        && !id.starts_with('-')
        && (flags.is_empty()
            || flags == start_flags(provider)
            || (provider == "cursor" && flags == "--yolo")))
        .then_some((provider, id, flags))
}

pub fn safe_resume_command(command: &str) -> bool {
    resume_parts(command).is_some()
}

pub fn resume_skips_checks(command: &str) -> bool {
    resume_parts(command).is_some_and(|(_, _, flags)| !flags.is_empty())
}

pub fn configured_resume_in(dir: &std::path::Path, command: &str) -> Option<String> {
    let (provider, session, _) = resume_parts(command)?;
    resume_command_in(dir, provider, session)
}

pub fn is_store_provider(provider: &str) -> bool {
    START_PROVIDERS.contains(&provider) || RESUME_PROVIDERS.contains(&provider)
}

pub fn session_from_label(label: &str) -> Option<(String, String)> {
    let text = label.trim();
    for (form, provider) in LABEL_FORMS.iter() {
        if let Some(found) = form.captures(text).and_then(|c| c.get(1))
            && safe_session(found.as_str())
        {
            return Some((provider.to_string(), found.as_str().to_string()));
        }
    }
    None
}

pub fn cannot_resume(agent: &CustomAgent) -> Option<String> {
    if agent.provider.is_empty() {
        return Some("no provider was saved".to_string());
    }
    if !RESUME_PROVIDERS.contains(&agent.provider.as_str()) {
        return Some(format!("{} has no resume support in Nixa", agent.provider));
    }
    if agent.session.is_empty() {
        return Some("no session id was saved".to_string());
    }
    if !safe_session(&agent.session) {
        return Some("the saved session id is not valid".to_string());
    }
    None
}

pub fn agent_resume(agent: &CustomAgent) -> Option<String> {
    resume_command(&agent.provider, &agent.session)
}

pub fn agent_resume_in(data_dir: &std::path::Path, agent: &CustomAgent) -> Option<String> {
    resume_command_in(data_dir, &agent.provider, &agent.session)
}

pub fn starts_new(agent: &CustomAgent) -> bool {
    agent.session.is_empty() && (agent.start.is_some() || start_command_with(&agent.provider, false).is_some())
}

pub fn agent_state(live: bool, status: Option<Status>) -> AgentState {
    match (live, status) {
        (false, _) => AgentState::Closed,
        (true, Some(Status::Working)) => AgentState::Working,
        (true, Some(Status::Blocked)) => AgentState::Waiting,
        _ => AgentState::Free,
    }
}

pub fn claude_based(provider: &str, start: Option<&StartProfile>) -> bool {
    let agent = start.map(|s| s.agent.as_str()).unwrap_or(provider);
    agent == "claude" || start.is_some_and(|s| s.model.to_lowercase().contains("claude"))
}

pub fn profile_text(start: &StartProfile) -> String {
    let model = if start.model.is_empty() { "default model" } else { &start.model };
    let effort = if start.effort.is_empty() { "default effort" } else { &start.effort };
    format!("{} · {model} · {effort}", start.agent)
}

pub const PROFILE_EFFORTS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];
pub const PROFILE_ARGS: [&str; 1] = ["--verbose"];

pub fn plain_model(model: &str) -> bool {
    model.is_empty()
        || (model.len() <= 80
            && !model.starts_with('-')
            && model.chars().all(|c| c.is_ascii_alphanumeric() || "._:/@-".contains(c)))
}

pub fn read_profile(start: &StartProfile) -> Option<StartProfile> {
    let agent = start.agent.trim().to_lowercase();
    if !PROFILE_AGENTS.contains(&agent.as_str()) {
        return None;
    }
    let effort = start.effort.trim();
    let effort_ok = effort.is_empty() || effort == "default" || PROFILE_EFFORTS.contains(&effort.to_lowercase().as_str());
    let args_ok = start.args.iter().filter(|a| !a.is_empty()).all(|a| PROFILE_ARGS.contains(&a.as_str()));
    if !plain_model(start.model.trim()) || !effort_ok || !args_ok {
        return None;
    }
    Some(StartProfile {
        agent,
        model: start.model.trim().to_string(),
        effort: if effort == "default" { String::new() } else { effort.to_string() },
        args: start.args.iter().filter(|a| !a.is_empty()).cloned().collect(),
    })
}

fn provider_of_profile(start: &StartProfile) -> String {
    if is_store_provider(&start.agent) { start.agent.clone() } else { String::new() }
}

fn read_role(text: &str) -> Result<String> {
    let role = text.trim().to_lowercase();
    match role.as_str() {
        "" | "none" | "off" => Ok(String::new()),
        REVIEWER_ROLE => Ok(role),
        _ => bail!("The role must be one of: {REVIEWER_ROLE}, or none."),
    }
}

fn read_slug(text: &str, max: usize, what: &str) -> Result<String> {
    let word = text.trim().to_lowercase();
    if word.is_empty() || word == "none" {
        return Ok(String::new());
    }
    let ok = word.len() <= max
        && word.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        bail!("{what}");
    }
    Ok(word)
}

fn read_job(text: &str) -> Result<String> {
    read_slug(text, 40, "A job is a short id such as developer or code-reviewer.")
}

fn read_workflow(text: &str) -> Result<String> {
    read_slug(text, 60, "A workflow name is small letters, numbers and dashes, for example build-review-fix.")
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(str::to_string)
        .collect()
}

fn head(word: &str, n: usize) -> &str {
    match word.char_indices().nth(n) {
        Some((at, _)) => &word[..at],
        None => word,
    }
}

pub fn match_score(agent: &CustomAgent, query: &str) -> i64 {
    let asked = words(query);
    if asked.is_empty() {
        return 0;
    }
    let have = format!("{} {} {}", agent.purpose, agent.title, agent.instructions).to_lowercase();
    let own = words(&have);
    let mut score = 0;
    for word in &asked {
        if own.contains(word) {
            score += 2;
        } else if own.iter().any(|item| item.starts_with(head(word, 5)) || word.starts_with(head(item, 5))) {
            score += 1;
        }
    }
    if have.contains(query.to_lowercase().trim()) {
        score += 3;
    }
    score
}

fn to_agent(row: &Row) -> rusqlite::Result<CustomAgent> {
    let start: String = row.get("start")?;
    Ok(CustomAgent {
        id: row.get("id")?,
        title: row.get("title")?,
        purpose: row.get("purpose")?,
        instructions: row.get("instructions")?,
        role: row.get("role")?,
        job: row.get("job")?,
        workflow: row.get("workflow")?,
        start: if start.is_empty() { None } else { serde_json::from_str(&start).ok() },
        off: row.get::<_, i64>("off")? == 1,
        migrated_from: row.get("migrated_from")?,
        provider: row.get("provider")?,
        session: row.get("session")?,
        space: row.get("space")?,
        space_name: row.get("space_name")?,
        folder: row.get("folder")?,
        terminal: row.get("terminal")?,
        created_at: row.get("created_at")?,
        last_used: row.get("last_used")?,
        by: row.get("by")?,
        skills: serde_json::from_str(&row.get::<_, String>("skills")?).unwrap_or_default(),
    })
}

fn start_text(start: &Option<StartProfile>) -> Result<String> {
    Ok(match start {
        Some(s) => serde_json::to_string(s)?,
        None => String::new(),
    })
}

fn save(conn: &Connection, agent: &CustomAgent) -> Result<()> {
    conn.execute(
        "UPDATE custom_agents SET title = ?2, purpose = ?3, instructions = ?4, role = ?5, job = ?6, workflow = ?7, start = ?8, off = ?9,
           migrated_from = ?10, provider = ?11, session = ?12, space = ?13, space_name = ?14, folder = ?15, terminal = ?16, last_used = ?17, skills = ?18
         WHERE id = ?1",
        params![
            agent.id,
            agent.title,
            agent.purpose,
            agent.instructions,
            agent.role,
            agent.job,
            agent.workflow,
            start_text(&agent.start)?,
            flag(agent.off),
            agent.migrated_from,
            agent.provider,
            agent.session,
            agent.space,
            agent.space_name,
            agent.folder,
            agent.terminal,
            agent.last_used,
            serde_json::to_string(&agent.skills)?,
        ],
    )?;
    Ok(())
}

fn query_agents(conn: &Connection, sql: &str, value: Option<&str>) -> Result<Vec<CustomAgent>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = match value {
        Some(v) => stmt.query_map(params![v], to_agent)?.collect::<rusqlite::Result<Vec<_>>>()?,
        None => stmt.query_map([], to_agent)?.collect::<rusqlite::Result<Vec<_>>>()?,
    };
    Ok(rows)
}

pub fn get_agent(conn: &Connection, id: &str) -> Result<Option<CustomAgent>> {
    Ok(conn.query_row("SELECT * FROM custom_agents WHERE id = ?1", params![id], to_agent).optional()?)
}

fn must_agent(conn: &Connection, id: &str) -> Result<CustomAgent> {
    match get_agent(conn, id)? {
        Some(agent) => Ok(agent),
        None => bail!("No saved custom agent {id}."),
    }
}

pub fn list_agents(conn: &Connection) -> Result<Vec<CustomAgent>> {
    query_agents(conn, "SELECT * FROM custom_agents ORDER BY last_used DESC, number", None)
}

pub fn agent_by_terminal(conn: &Connection, terminal: &str) -> Result<Option<CustomAgent>> {
    if terminal.is_empty() {
        return Ok(None);
    }
    Ok(query_agents(conn, "SELECT * FROM custom_agents WHERE terminal = ?1 ORDER BY last_used DESC", Some(terminal))?
        .into_iter()
        .next())
}

fn in_space(agent: &CustomAgent, space: &str) -> bool {
    space.is_empty() || agent.space == space || agent.space_name.eq_ignore_ascii_case(space)
}

pub fn find_agents(conn: &Connection, query: &str, space: &str) -> Result<Vec<CustomAgent>> {
    let mut scored: Vec<(i64, CustomAgent)> = list_agents(conn)?
        .into_iter()
        .map(|a| (match_score(&a, query), a))
        .filter(|(score, a)| *score > 0 && in_space(a, space))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.last_used.cmp(&a.1.last_used)));
    Ok(scored.into_iter().map(|(_, a)| a).collect())
}

pub fn pick_agent(conn: &Connection, reference: &str, space: &str) -> Result<Option<CustomAgent>> {
    let all = list_agents(conn)?;
    let key = reference.trim().to_lowercase();
    let pick = all
        .iter()
        .find(|a| a.id.to_lowercase() == key)
        .or_else(|| all.iter().find(|a| !a.off && a.title.to_lowercase() == key))
        .or_else(|| all.iter().find(|a| !a.off && a.job == key))
        .cloned();
    if pick.is_some() {
        return Ok(pick);
    }
    Ok(find_agents(conn, reference, space)?.into_iter().find(|a| !a.off))
}

pub fn agents_with_role(conn: &Connection, role: &str, space: &str) -> Result<Vec<CustomAgent>> {
    Ok(list_agents(conn)?
        .into_iter()
        .filter(|a| a.role == role && (space.is_empty() || a.space == space))
        .filter(|a| !(role == REVIEWER_ROLE && claude_based(&a.provider, a.start.as_ref())))
        .collect())
}

pub fn add_agent(conn: &Connection, input: &AddAgent, by: &str) -> Result<CustomAgent> {
    let title = clean_line(&input.title, TITLE_MAX);
    let purpose = clean_line(&input.purpose, PURPOSE_MAX);
    if title.is_empty() || purpose.is_empty() {
        bail!("A custom agent needs a title and a purpose.");
    }
    let start = match &input.start {
        Some(s) => match read_profile(s) {
            Some(p) => Some(p),
            None => bail!("A start profile needs an agent: {}, a plain model name, an effort from {} and no extra flags.", PROFILE_AGENTS.join(", "), PROFILE_EFFORTS.join(", ")),
        },
        None => None,
    };
    let provider = match &start {
        Some(s) => provider_of_profile(s),
        None => clean_line(&input.provider, 30).to_lowercase(),
    };
    if !provider.is_empty() && !is_store_provider(&provider) {
        let mut all: Vec<&str> = START_PROVIDERS.to_vec();
        all.sort();
        bail!("The provider must be one of {}, or empty.", all.join(", "));
    }
    let role = read_role(&input.role)?;
    if role == REVIEWER_ROLE && claude_based(&provider, start.as_ref()) {
        bail!("{CLAUDE_REVIEWER}");
    }
    let session = clean_line(&input.session, SESSION_MAX);
    if !session.is_empty() && !safe_session(&session) {
        bail!("The session id has characters Nixa does not accept.");
    }
    let job = read_job(&input.job)?;
    let workflow = read_workflow(&input.workflow)?;
    let all = list_agents(conn)?;
    if let Some(same) = all
        .iter()
        .find(|a| !session.is_empty() && a.provider == provider && a.session == session)
        .or_else(|| all.iter().find(|a| a.title.to_lowercase() == title.to_lowercase()))
    {
        let what = if !session.is_empty() && same.session == session { "session" } else { "title" };
        bail!("A custom agent with this {what} is saved already: {} \"{}\". Edit or forget that one instead.", same.id, same.title);
    }
    let number = next_number(conn, COUNTER)?;
    let at = now_ms();
    let agent = CustomAgent {
        id: format!("a{number}"),
        title,
        purpose,
        instructions: clean_instructions(&input.instructions, INSTRUCTIONS_MAX),
        role,
        job,
        workflow,
        start,
        off: input.off,
        migrated_from: clean_line(&input.migrated_from, 200),
        provider,
        session,
        space: clean_line(&input.space, 120),
        space_name: clean_line(&input.space_name, 120),
        folder: input.folder.trim().to_string(),
        terminal: String::new(),
        created_at: at,
        last_used: at,
        by: clean_line(by, 80),
        skills: read_skills(&input.skills)?,
    };
    conn.execute(
        "INSERT INTO custom_agents (id, number, title, purpose, created_at, last_used) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![agent.id, number, agent.title, agent.purpose, at],
    )?;
    save(conn, &agent)?;
    conn.execute("UPDATE custom_agents SET by = ?2 WHERE id = ?1", params![agent.id, agent.by])?;
    Ok(agent)
}

pub fn import_agent(conn: &Connection, agent: &CustomAgent) -> Result<CustomAgent> {
    let Some(number) = agent.id.strip_prefix('a').and_then(|n| n.parse::<i64>().ok()).filter(|n| *n > 0) else {
        bail!("A custom agent id is a then a number, for example a12.");
    };
    if get_agent(conn, &agent.id)?.is_some() {
        bail!("A custom agent {} is saved already.", agent.id);
    }
    if clean_line(&agent.title, TITLE_MAX).is_empty() || clean_line(&agent.purpose, PURPOSE_MAX).is_empty() {
        bail!("A custom agent needs a title and a purpose.");
    }
    if !agent.session.is_empty() && !safe_session(&agent.session) {
        bail!("The session id has characters Nixa does not accept.");
    }
    let mut clean = agent.clone();
    clean.title = clean_line(&agent.title, TITLE_MAX);
    clean.purpose = clean_line(&agent.purpose, PURPOSE_MAX);
    clean.instructions = clean_instructions(&agent.instructions, INSTRUCTIONS_MAX);
    clean.role = read_role(&agent.role)?;
    clean.job = read_job(&agent.job)?;
    clean.workflow = read_workflow(&agent.workflow)?;
    clean.start = agent.start.as_ref().and_then(read_profile);
    clean.skills = read_skills(&agent.skills)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO custom_agents (id, number, title, purpose, created_at, last_used, by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![clean.id, number, clean.title, clean.purpose, clean.created_at, clean.last_used, clean.by],
    )?;
    save(&tx, &clean)?;
    bump_number(&tx, COUNTER, number)?;
    tx.commit()?;
    Ok(clean)
}

pub fn edit_agent(conn: &Connection, id: &str, change: &EditAgent) -> Result<CustomAgent> {
    let mut agent = must_agent(conn, id)?;
    let title = change.title.as_deref().map(|t| clean_line(t, TITLE_MAX)).unwrap_or_default();
    let purpose = change.purpose.as_deref().map(|t| clean_line(t, PURPOSE_MAX)).unwrap_or_default();
    let session = change.session.as_deref().map(|t| clean_line(t, SESSION_MAX)).unwrap_or_default();
    if !session.is_empty() && !safe_session(&session) {
        bail!("The session id has characters Nixa does not accept.");
    }
    let has_profile = [&change.profile_agent, &change.profile_model, &change.profile_effort]
        .iter()
        .any(|v| v.as_deref().is_some_and(|t| !t.trim().is_empty()));
    let nothing = title.is_empty()
        && purpose.is_empty()
        && change.instructions.is_none()
        && session.is_empty()
        && change.role.is_none()
        && change.job.is_none()
        && change.workflow.is_none()
        && !has_profile
        && change.off.is_none()
        && change.skills.is_none();
    if nothing {
        bail!("Give a new title, purpose, instructions, session, role, job, workflow, start agent, model, effort, skills or on/off.");
    }
    let role = change.role.as_deref().map(read_role).transpose()?;
    let job = change.job.as_deref().map(read_job).transpose()?;
    let workflow = change.workflow.as_deref().map(read_workflow).transpose()?;
    let mut start = agent.start.clone();
    if has_profile {
        let new_agent = change.profile_agent.as_deref().map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty());
        let current = agent.start.clone().unwrap_or_default();
        let fresh = new_agent.as_ref().is_some_and(|a| *a != current.agent);
        let mut next = if fresh { StartProfile::default() } else { current };
        if let Some(a) = new_agent {
            next.agent = a;
        }
        if let Some(m) = change.profile_model.as_deref().filter(|m| !m.trim().is_empty()) {
            next.model = m.trim().to_string();
        }
        if let Some(e) = change.profile_effort.as_deref().filter(|e| !e.trim().is_empty()) {
            next.effort = e.trim().to_string();
        }
        match read_profile(&next) {
            Some(p) => start = Some(p),
            None => bail!("A start profile needs an agent: {}, a plain model name, an effort from {} and no extra flags.", PROFILE_AGENTS.join(", "), PROFILE_EFFORTS.join(", ")),
        }
    }
    let reviewer = role.as_deref() == Some(REVIEWER_ROLE) || (agent.role == REVIEWER_ROLE && role.as_deref() != Some(""));
    if reviewer && claude_based(&agent.provider, start.as_ref()) {
        bail!("{CLAUDE_REVIEWER}");
    }
    if has_profile && let Some(s) = &start {
        let provider = provider_of_profile(s);
        if provider != agent.provider {
            agent.provider = provider;
            agent.session.clear();
            agent.terminal.clear();
        }
    }
    agent.start = start;
    if let Some(off) = change.off {
        agent.off = off;
    }
    if !title.is_empty() {
        agent.title = title;
    }
    if !purpose.is_empty() {
        agent.purpose = purpose;
    }
    if let Some(text) = &change.instructions {
        agent.instructions = clean_instructions(text, INSTRUCTIONS_MAX);
    }
    if let Some(list) = &change.skills {
        agent.skills = read_skills(list)?;
    }
    if !session.is_empty() {
        agent.session = session;
    }
    if let Some(role) = role {
        agent.role = role;
    }
    if let Some(job) = job {
        agent.job = job;
    }
    if let Some(workflow) = workflow {
        agent.workflow = workflow;
    }
    save(conn, &agent)?;
    Ok(agent)
}

pub fn set_agent_off(conn: &Connection, id: &str, off: bool) -> Result<CustomAgent> {
    let mut agent = must_agent(conn, id)?;
    agent.off = off;
    save(conn, &agent)?;
    Ok(agent)
}

pub fn use_agent(conn: &Connection, id: &str, used: &AgentUse) -> Result<CustomAgent> {
    let mut agent = must_agent(conn, id)?;
    if let Some(terminal) = &used.terminal {
        agent.terminal = terminal.trim().to_string();
    }
    if let Some(session) = used.session.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !safe_session(session) {
            bail!("The session id has characters Nixa does not accept.");
        }
        agent.session = session.to_string();
    }
    if let Some(space) = used.space.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        agent.space = space.to_string();
    }
    if let Some(name) = used.space_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        agent.space_name = name.to_string();
    }
    if let Some(folder) = used.folder.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        agent.folder = folder.to_string();
    }
    agent.last_used = now_ms();
    save(conn, &agent)?;
    Ok(agent)
}

pub fn model_flag(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" | "cursor" | "omp" | "opencode" | "kimi" | "agy" => Some("--model"),
        "codex" | "gemini" => Some("-m"),
        _ => None,
    }
}

pub fn effort_words(agent: &str, effort: &str) -> Vec<String> {
    let effort = effort.trim();
    if effort.is_empty() {
        return Vec::new();
    }
    match agent {
        "codex" => vec!["-c".into(), format!("model_reasoning_effort={effort}")],
        "claude" | "agy" => vec!["--effort".into(), effort.to_string()],
        _ => Vec::new(),
    }
}

pub const SKILLS_MAX: usize = 8;
pub const SKILLS_DIR: &str = "skills";
pub const SKILL_FILE: &str = "SKILL.md";

pub fn good_skill(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.contains("..")
}

pub fn read_skills(list: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for name in list.iter().flat_map(|s| s.split(',')).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        if !good_skill(&name) {
            bail!("The skill name {name:?} is not valid. Use letters, numbers, - _ and . only.");
        }
        if !out.contains(&name) {
            out.push(name);
        }
    }
    if out.len() > SKILLS_MAX {
        bail!("A custom agent can have at most {SKILLS_MAX} skills.");
    }
    Ok(out)
}

pub fn skill_file(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join(SKILLS_DIR).join(name).join(SKILL_FILE)
}

pub fn skills_block(agent: &CustomAgent, data_dir: &Path) -> Option<String> {
    if agent.skills.is_empty() {
        return None;
    }
    let lines: Vec<String> = agent
        .skills
        .iter()
        .map(|name| {
            let file = skill_file(data_dir, name);
            if file.is_file() {
                format!("- {name}: {}", file.display())
            } else {
                format!("- {name}: not installed in Nixa yet, skip it")
            }
        })
        .collect();
    Some(format!(
        "Your skills: before you start, read each skill file below and follow it for this job. Your standing instructions and the task come first when they differ.\n{}",
        lines.join("\n")
    ))
}

pub fn forget_agent(conn: &Connection, id: &str) -> Result<CustomAgent> {
    let agent = must_agent(conn, id)?;
    conn.execute("DELETE FROM custom_agents WHERE id = ?1", params![id])?;
    Ok(agent)
}

pub fn agent_line(agent: &CustomAgent, state: AgentState) -> String {
    let instructions = if agent.instructions.is_empty() {
        String::new()
    } else {
        format!(". Instructions: {}", crate::text::clip(&one_line(&agent.instructions), 200))
    };
    let mut line = format!("{} \"{}\" ({}{})", agent.id, agent.title, one_line(&agent.purpose), instructions);
    for (name, value) in [("job", &agent.job), ("workflow", &agent.workflow), ("role", &agent.role)] {
        if !value.is_empty() {
            line.push_str(&format!(" · {name} {value}"));
        }
    }
    if !agent.skills.is_empty() {
        line.push_str(&format!(" · skills {}", agent.skills.join(", ")));
    }
    if agent.off {
        line.push_str(" · OFF");
    }
    let starts = match &agent.start {
        Some(s) => format!("starts as {}", profile_text(s)),
        None if agent.provider.is_empty() => "no provider".to_string(),
        None => agent.provider.clone(),
    };
    let space = if agent.space.is_empty() {
        "no space".to_string()
    } else {
        format!("space {}", if agent.space_name.is_empty() { &agent.space } else { &agent.space_name })
    };
    line.push_str(&format!(" · {starts} · {space} · {}", state.as_str()));
    line
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_profile_refuses_flags_in_model_effort_and_args() {
        let base = super::StartProfile { agent: "claude".into(), ..Default::default() };
        assert!(super::read_profile(&base).is_some());
        assert!(super::read_profile(&super::StartProfile { model: "--foo".into(), ..base.clone() }).is_none());
        assert!(super::read_profile(&super::StartProfile { model: "opus 5".into(), ..base.clone() }).is_none());
        assert!(super::read_profile(&super::StartProfile { effort: "--yolo".into(), ..base.clone() }).is_none());
        assert!(super::read_profile(&super::StartProfile { args: vec!["--dangerously-skip-permissions".into()], ..base.clone() }).is_none());
        assert!(super::read_profile(&super::StartProfile { model: "claude-opus-5-5".into(), effort: "high".into(), ..base }).is_some());
    }

    use super::*;
    use crate::db::temp_db;

    fn reviewer_input(title: &str, agent: &str) -> AddAgent {
        AddAgent {
            title: title.into(),
            purpose: "Reviews Rust code for bugs".into(),
            role: "reviewer".into(),
            space: "s1".into(),
            start: Some(StartProfile { agent: agent.into(), ..StartProfile::default() }),
            ..AddAgent::default()
        }
    }

    #[test]
    fn add_find_pick() {
        let conn = temp_db("agents-add");
        let builder = add_agent(
            &conn,
            &AddAgent { title: "Builder".into(), purpose: "Builds Rust crates".into(), provider: "codex".into(), job: "developer".into(), space: "s1".into(), ..AddAgent::default() },
            "Sam",
        )
        .unwrap();
        assert_eq!(builder.id, "a1");
        assert!(add_agent(&conn, &AddAgent { title: "builder".into(), purpose: "x".into(), ..AddAgent::default() }, "Sam").is_err());
        assert!(add_agent(&conn, &AddAgent { title: "No purpose".into(), ..AddAgent::default() }, "Sam").is_err());
        assert!(add_agent(&conn, &AddAgent { title: "X".into(), purpose: "y".into(), provider: "nope".into(), ..AddAgent::default() }, "Sam").is_err());
        let review = add_agent(&conn, &reviewer_input("Checker", "codex"), "Sam").unwrap();
        assert_eq!(review.id, "a2");
        assert_eq!(review.provider, "codex");
        assert!(add_agent(&conn, &reviewer_input("Claude checker", "claude"), "Sam").is_err());
        let found = find_agents(&conn, "rust crates", "").unwrap();
        assert_eq!(found[0].id, "a1");
        assert_eq!(pick_agent(&conn, "A2", "").unwrap().unwrap().id, "a2");
        assert_eq!(pick_agent(&conn, "developer", "").unwrap().unwrap().id, "a1");
        assert_eq!(pick_agent(&conn, "reviews bugs", "s1").unwrap().unwrap().id, "a2");
        assert!(pick_agent(&conn, "zzz", "").unwrap().is_none());
        assert_eq!(agents_with_role(&conn, REVIEWER_ROLE, "s1").unwrap().len(), 1);
        assert!(agents_with_role(&conn, REVIEWER_ROLE, "s2").unwrap().is_empty());
    }

    #[test]
    fn only_checks_on_overrides_the_allow_all_default() {
        let dir = std::env::temp_dir().join(format!(
            "nixa-agent-checks-{}-{}",
            std::process::id(),
            crate::time::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(PROGRAMS_FILE);
        let assert_allow_all = || {
            for (provider, on, off) in START_LINES {
                assert_eq!(start_command_in(&dir, provider), Some(off), "{provider}");
                let resume = resume_command_in(&dir, provider, "s-1").unwrap();
                assert!(safe_resume_command(&resume), "{resume}");
                assert_eq!(resume_skips_checks(&resume), on != off, "{resume}");
            }
        };
        assert_allow_all();
        let legacy = r#"{"checksOff":["claude","codex","cursor","agy","omp","hermes","dim","opencode","crush","omo","jcode","gemini"]}"#;
        std::fs::write(&path, legacy).unwrap();
        assert_allow_all();
        std::fs::write(
            &path,
            r#"{"programs": {}, "checksOff": ["codex", "claude", "omp"], "checksOn": ["codex", "hermes", "omp"]}"#,
        )
        .unwrap();
        assert_eq!(start_command_in(&dir, "codex"), Some("codex"));
        assert_eq!(
            start_command_in(&dir, "claude"),
            Some("claude --dangerously-skip-permissions")
        );
        assert_eq!(start_command_in(&dir, "omp"), Some("omp"));
        assert_eq!(
            resume_command_in(&dir, "hermes", "h1").as_deref(),
            Some("hermes --resume h1")
        );
        assert_eq!(
            resume_command_in(&dir, "claude", "c1").as_deref(),
            Some("claude --resume c1 --dangerously-skip-permissions")
        );
        assert_eq!(
            configured_resume_in(&dir, "omp --resume=s/1 --auto-approve").as_deref(),
            Some("omp --resume=s/1")
        );
        assert_eq!(
            configured_resume_in(&dir, "cursor-agent --resume c1 --yolo").as_deref(),
            Some("cursor-agent --resume c1 --yolo --trust")
        );
        assert_eq!(
            configured_resume_in(&dir, "jcode --resume j1").as_deref(),
            Some("jcode --no-update --resume j1")
        );
        assert!(!safe_resume_command("claude --resume a --yolo"));
        assert!(!safe_resume_command("claude --resume --help"));
        assert!(resume_command_with("claude", "bad id;", true).is_none());
        std::fs::write(&path, "{nope").unwrap();
        assert!(checks_off_in(&path, "codex"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn launch_defaults_replace_ask_options_and_keep_other_arguments() {
        let cases: &[(&[&str], &[&str])] = &[
            (
                &["claude", "--permission-mode", "plan", "--model", "test"],
                &[
                    "claude",
                    "--dangerously-skip-permissions",
                    "--model",
                    "test",
                ],
            ),
            (
                &[
                    "codex",
                    "--ask-for-approval=on-request",
                    "-s",
                    "read-only",
                    "-c",
                    "model=test",
                ],
                &[
                    "codex",
                    "--dangerously-bypass-approvals-and-sandbox",
                    "-c",
                    "model=test",
                ],
            ),
            (
                &["cursor-agent", "--plan", "--mode=ask"],
                &["cursor-agent", "--yolo", "--trust"],
            ),
            (
                &[
                    "omp",
                    "--approval-mode=always-ask",
                    "--",
                    "--approval-mode=always-ask",
                ],
                &["omp", "--auto-approve", "--", "--approval-mode=always-ask"],
            ),
            (
                &["omp", "--approval-mode", "always-ask"],
                &["omp", "--auto-approve"],
            ),
            (
                &[
                    "env",
                    "DIMCODE_UPDATE_CHECK=0",
                    "dima",
                    "--permission=read-only",
                ],
                &[
                    "env",
                    "DIMCODE_UPDATE_CHECK=0",
                    "dima",
                    "--permission",
                    "full-access",
                ],
            ),
            (&["jcode", "--no-update"], &["jcode", "--no-update"]),
            (&["omp", "--help"], &["omp", "--help"]),
        ];
        for (raw, expected) in cases {
            let mut argv: Vec<String> = raw.iter().map(|s| (*s).to_owned()).collect();
            configure_start(None, &mut argv);
            assert_eq!(&argv, expected);
            configure_start(None, &mut argv);
            assert_eq!(&argv, expected);
        }
        let mut argv = vec![
            "omp".into(),
            "--auto-approve".into(),
            "--approval-mode=always-ask".into(),
        ];
        configure_start(Some(r#"{"checksOff":["omp"],"checksOn":["omp"]}"#), &mut argv);
        assert_eq!(argv, ["omp", "--approval-mode=always-ask"]);
    }

    #[test]
    fn start_flags_carry_model_and_effort() {
        assert_eq!(model_flag("agy"), Some("--model"));
        assert_eq!(effort_words("agy", "high"), vec!["--effort", "high"]);
        assert_eq!(effort_words("claude", "low"), vec!["--effort", "low"]);
        assert_eq!(effort_words("codex", "xhigh"), vec!["-c", "model_reasoning_effort=xhigh"]);
        assert!(effort_words("opencode", "high").is_empty());
        assert!(effort_words("claude", " ").is_empty());
    }

    #[test]
    fn skills_are_saved_edited_and_shown() {
        let conn = temp_db("agents-skills");
        assert!(read_skills(&["../x".into()]).is_err());
        assert!(read_skills(&["a,b,c,d,e,f,g,h,i".into()]).is_err());
        assert_eq!(read_skills(&["TDD, research,tdd".into()]).unwrap(), vec!["tdd", "research"]);
        let agent = add_agent(&conn, &AddAgent { title: "Researcher".into(), purpose: "Looks things up".into(), provider: "claude".into(), skills: vec!["research".into()], ..AddAgent::default() }, "Nixa").unwrap();
        assert_eq!(get_agent(&conn, &agent.id).unwrap().unwrap().skills, vec!["research"]);
        let edited = edit_agent(&conn, &agent.id, &EditAgent { skills: Some(vec!["research,tdd".into()]), ..EditAgent::default() }).unwrap();
        assert_eq!(edited.skills, vec!["research", "tdd"]);
        assert!(agent_line(&edited, AgentState::Closed).contains("skills research, tdd"));
        let dir = std::env::temp_dir().join(format!("nixa-skills-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("skills/research")).unwrap();
        std::fs::write(dir.join("skills/research/SKILL.md"), "x").unwrap();
        let block = skills_block(&edited, &dir).unwrap();
        assert!(block.contains(&skill_file(&dir, "research").display().to_string()));
        assert!(block.contains("- tdd: not installed in Nixa yet, skip it"));
        let cleared = edit_agent(&conn, &agent.id, &EditAgent { skills: Some(vec![String::new()]), ..EditAgent::default() }).unwrap();
        assert!(cleared.skills.is_empty() && skills_block(&cleared, &dir).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn use_edit_state_forget() {
        let conn = temp_db("agents-use");
        let agent = add_agent(&conn, &AddAgent { title: "Helper".into(), purpose: "Helps".into(), provider: "claude".into(), ..AddAgent::default() }, "Nixa").unwrap();
        assert_eq!(cannot_resume(&agent).as_deref(), Some("no session id was saved"));
        assert!(starts_new(&agent));
        let used = use_agent(&conn, &agent.id, &AgentUse { terminal: Some("m4".into()), session: Some("abc-123".into()), ..AgentUse::default() }).unwrap();
        assert_eq!(used.terminal, "m4");
        assert_eq!(agent_resume(&used), resume_command("claude", "abc-123"));
        assert_eq!(resume_command_with("claude", "abc-123", false).as_deref(), Some("claude --resume abc-123"));
        assert_eq!(agent_by_terminal(&conn, "m4").unwrap().unwrap().id, agent.id);
        assert!(use_agent(&conn, &agent.id, &AgentUse { session: Some("bad id;".into()), ..AgentUse::default() }).is_err());
        let off = set_agent_off(&conn, &agent.id, true).unwrap();
        assert!(off.off);
        assert!(pick_agent(&conn, "helper", "").unwrap().is_none());
        assert_eq!(pick_agent(&conn, "a1", "").unwrap().unwrap().id, "a1");
        let change = EditAgent { profile_agent: Some("codex".into()), profile_model: Some("gpt".into()), instructions: Some("line one\n\n\n\nline two".into()), ..EditAgent::default() };
        let edited = edit_agent(&conn, &agent.id, &change).unwrap();
        assert_eq!(edited.provider, "codex");
        assert!(edited.session.is_empty());
        assert!(edited.terminal.is_empty());
        assert_eq!(edited.start.as_ref().unwrap().model, "gpt");
        assert_eq!(edited.instructions, "line one\n\nline two");
        assert!(edit_agent(&conn, &agent.id, &EditAgent::default()).is_err());
        assert!(edit_agent(&conn, &agent.id, &EditAgent { job: Some("Bad Job".into()), ..EditAgent::default() }).is_err());
        let reviewer = edit_agent(&conn, &agent.id, &EditAgent { role: Some("reviewer".into()), ..EditAgent::default() }).unwrap();
        assert_eq!(reviewer.role, "reviewer");
        assert!(edit_agent(&conn, &agent.id, &EditAgent { profile_agent: Some("claude".into()), ..EditAgent::default() }).is_err());
        assert!(agent_line(&reviewer, AgentState::Closed).contains("role reviewer · OFF · starts as codex · gpt · default effort"));
        forget_agent(&conn, &agent.id).unwrap();
        assert!(get_agent(&conn, &agent.id).unwrap().is_none());
        let next = add_agent(&conn, &AddAgent { title: "Next".into(), purpose: "p".into(), ..AddAgent::default() }, "Nixa").unwrap();
        assert_eq!(next.id, "a2");
        let old = CustomAgent { id: "a12".into(), title: "Old".into(), purpose: "Old one".into(), ..next.clone() };
        let imported = import_agent(&conn, &old).unwrap();
        assert_eq!(imported.id, "a12");
        assert!(import_agent(&conn, &old).is_err());
        assert!(import_agent(&conn, &CustomAgent { id: "x".into(), ..old.clone() }).is_err());
        let after = add_agent(&conn, &AddAgent { title: "After".into(), purpose: "p".into(), ..AddAgent::default() }, "Nixa").unwrap();
        assert_eq!(after.id, "a13");
    }

    #[test]
    fn labels_and_state() {
        assert_eq!(session_from_label("codex resume s-1"), Some(("codex".to_string(), "s-1".to_string())));
        assert_eq!(session_from_label("agy --conversation c9"), Some(("agy".to_string(), "c9".to_string())));
        assert_eq!(session_from_label("vim"), None);
        assert_eq!(agent_state(false, Some(Status::Working)), AgentState::Closed);
        assert_eq!(agent_state(true, Some(Status::Working)), AgentState::Working);
        assert_eq!(agent_state(true, Some(Status::Blocked)), AgentState::Waiting);
        assert_eq!(agent_state(true, None), AgentState::Free);
        assert!(claude_based("codex", Some(&StartProfile { agent: "omp".into(), model: "claude-opus".into(), ..StartProfile::default() })));
    }
}
