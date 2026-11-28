use crate::app::App;
use crate::engine;
use anyhow::{Result, anyhow, bail};
use nixa_protocol::{CreateSpec, Reply, Request, Status, TermId, TermInfo};
use nixa_store::models::{Prepared, Program};
use nixa_store::models::start::{agent_of_argv, fallover_agents, program_for};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MEMBER_PREFIX: &str = "member:";
pub const TEAM_PREFIX: &str = "team:";
pub const SCHEDULE_LABEL: &str = "nixa.schedule";
pub const SPACE_LABEL: &str = "space";
pub const TAB_LABEL: &str = "tab";
pub const PROGRAMS_FILE: &str = "programs.json";
pub const PAUSE_MARK: &str = "The schedule is paused";
pub const BUILT_IN: [(&str, &str, &[&str]); 13] = [
    ("claude", "Claude Code", &["claude"]),
    ("codex", "Codex", &["codex"]),
    ("gemini", "Gemini", &["gemini"]),
    ("cursor", "Cursor", &["cursor-agent"]),
    ("opencode", "OpenCode", &["opencode"]),
    ("omp", "Oh My Pi", &["omp"]),
    ("antigravity", "Antigravity", &["agy"]),
    ("crush", "Crush", &["crush"]),
    ("omo", "OmO", &["omo"]),
    ("jcode", "Jcode", &["jcode"]),
    ("hermes", "Hermes", &["hermes"]),
    ("dim", "Dim", &["dima"]),
    ("kimi", "Kimi", &["kimi"]),
];
const EXTRA_DIRS: [&str; 6] = [
    ".local/bin",
    ".opencode/bin",
    ".kimi-code/bin",
    ".cargo/bin",
    ".bun/bin",
    ".npm-global/bin",
];
const SYSTEM_DIRS: [&str; 4] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];
const START_POLL_MS: u64 = 1000;
const START_STEADY: u32 = 3;
const START_TRIES: u32 = 25;
const READY_TRIES: u32 = 30;
const READY_SETTLE_MS: u64 = 1500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choice {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub flow: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Repo {
    pub name: String,
    pub folder: String,
    pub branches: Vec<String>,
    pub current: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    Working,
    Waiting,
    Idle,
    Gone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub schedule: String,
    pub name: String,
    pub agent: String,
    pub folder: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Want {
    pub model: String,
    pub effort: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub argv: Vec<String>,
    pub send: String,
    pub label: String,
    pub member: Option<String>,
}

pub fn is_team(agent: &str) -> bool {
    agent.starts_with(TEAM_PREFIX)
}

pub fn is_member(agent: &str) -> bool {
    agent.starts_with(MEMBER_PREFIX)
}

pub fn pause_error(text: &str) -> anyhow::Error {
    anyhow!("{text} {PAUSE_MARK}: pick a new agent.")
}

pub fn pauses(error: &anyhow::Error) -> bool {
    error.to_string().contains(PAUSE_MARK)
}

pub fn programs(text: Option<&str>) -> Vec<(String, String, Vec<String>)> {
    let mut list: Vec<(String, String, Vec<String>)> = BUILT_IN
        .iter()
        .map(|(k, n, r)| {
            (
                k.to_string(),
                n.to_string(),
                r.iter().map(|s| s.to_string()).collect(),
            )
        })
        .collect();
    let data: Value = text
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or(Value::Null);
    let Some(map) = data.get("programs").and_then(Value::as_object) else {
        return list;
    };
    for (key, entry) in map {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        let run: Option<Vec<String>> = entry
            .get("run")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|l| !l.is_empty());
        if let Some(found) = list.iter_mut().find(|p| &p.0 == key) {
            if let Some(name) = name {
                found.1 = name;
            }
            if let Some(run) = run {
                found.2 = run;
            }
            continue;
        }
        if let Some(run) = run {
            list.push((key.clone(), name.unwrap_or_else(|| key.clone()), run));
        }
    }
    list
}

pub fn search_dirs(path_env: Option<&str>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    for part in path_env
        .unwrap_or_default()
        .split(':')
        .filter(|p| !p.is_empty())
    {
        add(PathBuf::from(part));
    }
    if let Some(home) = home {
        for extra in EXTRA_DIRS {
            add(home.join(extra));
        }
    }
    for dir in SYSTEM_DIRS {
        add(PathBuf::from(dir));
    }
    out
}

pub fn default_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    search_dirs(std::env::var("PATH").ok().as_deref(), home.as_deref())
}

fn word(run: &[String]) -> Option<&str> {
    let mut words = run.iter().map(String::as_str);
    let first = words.next()?;
    if first != "env" {
        return Some(first);
    }
    words.find(|w| !w.contains('='))
}

pub fn installed(run: &[String], dirs: &[PathBuf]) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(program) = word(run) else {
        return false;
    };
    let runnable = |p: &Path| {
        std::fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    };
    if program.contains('/') {
        return runnable(Path::new(program));
    }
    dirs.iter().any(|d| runnable(&d.join(program)))
}

pub fn agent_list(dir: &Path, dirs: &[PathBuf]) -> Vec<(String, String, Vec<String>, bool)> {
    let text = std::fs::read_to_string(dir.join(PROGRAMS_FILE)).ok();
    programs(text.as_deref())
        .into_iter()
        .map(|(k, n, mut r)| {
            nixa_store::agents::configure_start(text.as_deref(), &mut r);
            let ok = installed(&r, dirs);
            (k, n, r, ok)
        })
        .collect()
}

pub fn agents(app: &App) -> Vec<Choice> {
    agent_list(&app.dir, &default_dirs())
        .into_iter()
        .filter(|a| a.3)
        .map(|(id, name, _, _)| Choice {
            id,
            name,
            flow: String::new(),
        })
        .collect()
}

pub fn members(app: &App) -> Vec<Choice> {
    let list = nixa_store::open(&app.db_file)
        .and_then(|c| nixa_store::agents::list_agents(&c))
        .unwrap_or_default();
    list.into_iter()
        .filter(|a| !a.off && a.start.is_some())
        .map(|a| Choice {
            id: format!("{MEMBER_PREFIX}{}", a.id),
            name: if a.title.is_empty() {
                a.id.clone()
            } else {
                a.title.clone()
            },
            flow: String::new(),
        })
        .collect()
}

pub fn teams(app: &App) -> Vec<Choice> {
    crate::teams::choices(app)
}

pub fn label_of(agent: &str, lists: &[&[Choice]]) -> String {
    if agent == crate::assistant::AGENT {
        return crate::assistant::AGENT_NAME.into();
    }
    for list in lists {
        if let Some(found) = list.iter().find(|c| c.id == agent) {
            return found.name.clone();
        }
    }
    if let Some(id) = agent.strip_prefix(MEMBER_PREFIX) {
        return format!("{id} (team member not found)");
    }
    if let Some(id) = agent.strip_prefix(TEAM_PREFIX) {
        return format!("{id} (team not found)");
    }
    format!("{agent} (not found)")
}

pub fn model_args(agent: &str, model: &str, effort: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !model.is_empty()
        && matches!(
            agent,
            "claude" | "codex" | "cursor" | "opencode" | "omp" | "gemini" | "crush"
        )
    {
        out.push("--model".to_string());
        out.push(model.to_string());
    }
    if !effort.is_empty() && agent == "codex" {
        out.push("-c".to_string());
        out.push(format!("model_reasoning_effort={effort}"));
    }
    out
}

pub fn member_plan(
    dir: &std::path::Path,
    member: Option<nixa_store::agents::CustomAgent>,
    wanted: &str,
    task: &str,
) -> Result<Plan> {
    let Some(member) = member else {
        return Err(pause_error(&format!(
            "No team member \"{wanted}\" any more."
        )));
    };
    let title = if member.title.is_empty() {
        member.id.clone()
    } else {
        member.title.clone()
    };
    if member.off {
        return Err(pause_error(&format!("{title} is turned off.")));
    }
    let Some(profile) = member
        .start
        .as_ref()
        .and_then(nixa_store::agents::read_profile)
    else {
        return Err(pause_error(&format!("{title} has no start profile.")));
    };
    let Some(line) = nixa_store::agents::start_command_in(dir, &profile.agent) else {
        return Err(pause_error(&format!(
            "{title} uses {}, which Nixa cannot start.",
            profile.agent
        )));
    };
    let mut argv: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    argv.extend(model_args(&profile.agent, &profile.model, &profile.effort));
    argv.extend(profile.args.iter().cloned());
    nixa_store::agents::configure_start_in(dir, &mut argv);
    let send = if member.instructions.trim().is_empty() {
        task.to_string()
    } else {
        format!("{}\n\n{task}", member.instructions.trim())
    };
    Ok(Plan {
        argv,
        send,
        label: title,
        member: Some(member.id),
    })
}

pub fn plan(app: &App, job: &Job) -> Result<Plan> {
    if let Some(id) = job.agent.strip_prefix(MEMBER_PREFIX) {
        let found =
            nixa_store::open(&app.db_file).and_then(|c| nixa_store::agents::get_agent(&c, id))?;
        return member_plan(&app.dir, found, id, &job.prompt);
    }
    let Some((_, name, run, ok)) = program_entry(app, &job.agent) else {
        bail!("No agent \"{}\" in the New menu list.", job.agent);
    };
    if !ok {
        bail!("{name} is not installed.");
    }
    Ok(Plan {
        argv: run,
        send: job.prompt.clone(),
        label: name,
        member: None,
    })
}

pub fn tab_id(schedule: &str, at: u64) -> String {
    format!("sched-{schedule}-{at:x}")
}

pub async fn kill(id: &TermId) {
    let _ = engine::quick(Request::Kill { id: id.clone() }).await;
}

fn agent_up(info: &TermInfo, argv: &[String]) -> bool {
    info.running && !info.cmd.is_empty() && info.cmd == argv
}

async fn fail_start(label: &str, id: &TermId, up: bool) -> anyhow::Error {
    let screen = engine::read(id, Some(40), false).await.unwrap_or_default();
    let last: Vec<&str> = screen
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    let tail = last[last.len().saturating_sub(3)..].join(" / ");
    if up {
        kill(id).await;
    }
    let seen = if tail.is_empty() {
        String::new()
    } else {
        format!(" Its screen says: {tail}")
    };
    anyhow!(
        "{label} did not start in terminal {id} within {} seconds.{seen}",
        START_TRIES as u64 * START_POLL_MS / 1000
    )
}

pub async fn open(app: &App, job: &Job) -> Result<(String, Option<String>)> {
    open_with(app, job, None).await
}

pub async fn open_with(app: &App, job: &Job, want: Option<&Want>) -> Result<(String, Option<String>)> {
    let (plan, prepared) = launch_plan(app, job, want)?;
    open_plan(app, job, plan, prepared.as_ref()).await
}

fn known_agent(app: &App, argv: &[String]) -> bool {
    let Some(agent) = agent_of_argv(argv) else { return false };
    nixa_store::open(&app.db_file)
        .and_then(|c| nixa_store::models::load(&c))
        .map(|cache| nixa_store::models::model_rows(&cache).iter().any(|r| r.agent == agent))
        .unwrap_or(false)
}

fn base_run(app: &App, argv: &[String], named: &str) -> Result<Vec<String>> {
    let Some((agent, _)) = named.split_once("::") else { return Ok(argv.to_vec()) };
    if agent_of_argv(argv).is_none_or(|own| own == agent) {
        return Ok(argv.to_vec());
    }
    let list = crate::models::programs(app);
    program_for(&list, agent)
        .map(|p| p.run.clone())
        .ok_or_else(|| anyhow!("{agent} is not installed, so {named} cannot start."))
}

pub fn choose_model(app: &App, argv: &[String], task: &str, named: Option<&str>) -> Result<Option<Prepared>> {
    if task.trim().is_empty() || argv.is_empty() {
        return Ok(None);
    }
    match crate::models::prepare(app, argv, task, named) {
        Ok(found) => Ok(found),
        Err(error) if known_agent(app, argv) => Err(anyhow!("{error} Nothing was started.")),
        Err(error) => {
            crate::util::log("models", &format!("no model list for this agent, it starts as asked: {error}"));
            Ok(None)
        }
    }
}

fn program_entry(app: &App, agent: &str) -> Option<(String, String, Vec<String>, bool)> {
    agent_list(&app.dir, &default_dirs())
        .into_iter()
        .find(|a| a.0 == agent || a.1 == agent)
}

pub fn fallover_runs(programs: &[Program], order: &[String], skip: &str) -> Vec<Program> {
    fallover_agents(programs, order)
        .iter()
        .filter(|agent| agent.as_str() != skip)
        .filter_map(|agent| program_for(programs, agent).cloned())
        .collect()
}

fn fallover_of(app: &App, job: &Job, key: &str, argv: Vec<String>) -> Plan {
    let label = program_entry(app, key).map(|e| e.1).unwrap_or_else(|| key.to_string());
    Plan { argv, send: job.prompt.clone(), label, member: None }
}

#[derive(Debug)]
pub enum Tried {
    Ready(Box<Prepared>),
    Unknown,
    Refused(String),
}

pub fn fallover_choice(name: &str, tried: Vec<(Program, Tried)>) -> Result<(Program, Option<Prepared>)> {
    let mut unknown: Option<Program> = None;
    let mut refused: Vec<String> = Vec::new();
    for (program, result) in tried {
        match result {
            Tried::Ready(prepared) => return Ok((program, Some(*prepared))),
            Tried::Unknown => {
                unknown.get_or_insert(program);
            }
            Tried::Refused(why) => refused.push(format!("{}: {why}", program.key)),
        }
    }
    if let Some(program) = unknown {
        return Ok((program, None));
    }
    if refused.is_empty() {
        bail!("{name} is not installed.");
    }
    bail!("{name} is not installed, and no fallover agent has an available model ({}). Nothing was started.", refused.join("; "))
}

fn fallover_plan(app: &App, job: &Job, name: &str, run: &[String]) -> Result<(Plan, Option<Prepared>)> {
    let order = nixa_store::open(&app.db_file)
        .and_then(|c| nixa_store::agent_map::fallover(&c))
        .unwrap_or_default();
    let skip = agent_of_argv(run).unwrap_or_default();
    let tried: Vec<(Program, Tried)> = fallover_runs(&crate::models::programs(app), &order, skip)
        .into_iter()
        .map(|other| {
            let result = match choose_model(app, &other.run, &job.prompt, None) {
                Ok(Some(prepared)) => Tried::Ready(Box::new(prepared)),
                Ok(None) => Tried::Unknown,
                Err(error) => Tried::Refused(format!("{error:#}")),
            };
            (other, result)
        })
        .collect();
    let (program, prepared) = fallover_choice(name, tried)?;
    let argv = prepared.as_ref().map(|p| p.argv.clone()).unwrap_or_else(|| program.run.clone());
    Ok((fallover_of(app, job, &program.key, argv), prepared))
}

pub fn effort_note(agent: &str, effort: &str) -> Option<String> {
    (!effort.is_empty() && agent != "codex").then(|| format!("Work with {effort} effort."))
}

pub fn with_effort(send: &str, agent: &str, effort: &str) -> String {
    match effort_note(agent, effort) {
        Some(note) => format!("{send}\n\n{note}"),
        None => send.to_string(),
    }
}

pub fn launch_plan(app: &App, job: &Job, want: Option<&Want>) -> Result<(Plan, Option<Prepared>)> {
    if is_member(&job.agent) {
        return Ok((plan(app, job)?, None));
    }
    let Some((_, name, run, installed)) = program_entry(app, &job.agent) else {
        bail!("No agent \"{}\" in the New menu list.", job.agent);
    };
    let named = want.map(|w| w.model.as_str()).filter(|m| !m.is_empty());
    let base = match named {
        Some(named) => base_run(app, &run, named),
        None => Ok(run.clone()),
    };
    let moved = base.as_ref().is_ok_and(|argv| *argv != run);
    let (mut plan, prepared, named) = if !installed && !moved {
        let (plan, prepared) = fallover_plan(app, job, &name, &run)?;
        (plan, prepared, None)
    } else {
        let base = base?;
        let prepared = choose_model(app, &base, &job.prompt, named)?;
        let argv = prepared.as_ref().map(|p| p.argv.clone()).unwrap_or(base);
        (Plan { argv, send: job.prompt.clone(), label: name, member: None }, prepared, named)
    };
    if let Some(want) = want {
        let (agent, model) = match named.and_then(|n| n.split_once("::")) {
            Some((agent, model)) => (agent, model),
            None => (agent_of_argv(&plan.argv).unwrap_or_default(), named.unwrap_or_default()),
        };
        let model = if prepared.is_some() { "" } else { model };
        plan.argv.extend(model_args(agent, model, &want.effort));
        plan.send = with_effort(&plan.send, agent, &want.effort);
    }
    nixa_store::agents::configure_start_in(&app.dir, &mut plan.argv);
    Ok((plan, prepared))
}

pub async fn open_plan(app: &App, job: &Job, plan: Plan, prepared: Option<&Prepared>) -> Result<(String, Option<String>)> {
    let spaces = app.spaces();
    let space = crate::spaces::for_folder(&spaces, &job.folder).cloned();
    let mut spec = CreateSpec {
        cmd: plan.argv.clone(),
        cwd: Some(job.folder.clone()),
        name: Some(job.name.clone()),
        keep: true,
        ..CreateSpec::default()
    };
    spec.labels
        .insert(SCHEDULE_LABEL.into(), job.schedule.clone());
    if let Some(space) = &space {
        spec.labels.insert(nixa_protocol::id::SPACE_SLUG_LABEL.into(), space.name.clone());
        spec.labels.insert(SPACE_LABEL.into(), space.id.clone());
        spec.labels
            .insert(crate::deep::SPACE_LABEL.into(), space.id.clone());
        spec.labels.insert(
            TAB_LABEL.into(),
            tab_id(&job.schedule, crate::util::now_ms()),
        );
    }
    let term = engine::create(spec).await?;
    let id = term.id.clone();
    let mut steady = 0;
    let mut up = false;
    for _ in 0..START_TRIES {
        tokio::time::sleep(Duration::from_millis(START_POLL_MS)).await;
        let info = engine::get(&id).await?;
        if !agent_up(&info, &plan.argv) {
            return Err(fail_start(&plan.label, &id, false).await);
        }
        steady += 1;
        if steady >= START_STEADY {
            up = true;
            break;
        }
    }
    if !up {
        return Err(fail_start(&plan.label, &id, true).await);
    }
    for _ in 0..READY_TRIES {
        let info = engine::get(&id).await?;
        if !agent_up(&info, &plan.argv) {
            return Err(fail_start(&plan.label, &id, false).await);
        }
        let drew = engine::read(&id, Some(5), false)
            .await
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if info.status.is_some() || drew {
            break;
        }
        tokio::time::sleep(Duration::from_millis(START_POLL_MS)).await;
    }
    tokio::time::sleep(Duration::from_millis(READY_SETTLE_MS)).await;
    if !agent_up(&engine::get(&id).await?, &plan.argv) {
        return Err(fail_start(&plan.label, &id, false).await);
    }
    if let Err(error) = engine::send(&id, &plan.send, true, false).await {
        kill(&id).await;
        bail!(
            "{} started in terminal {id}, but Nixa could not send it the task: {error}.",
            plan.label
        );
    }
    if let Some(prepared) = prepared {
        let place = space.as_ref().map(|s| s.id.clone()).unwrap_or_default();
        if let Err(e) = crate::models::record(app, prepared, id.as_str(), &job.prompt, &job.folder, &place, &job.name) {
            crate::util::log("models", &format!("the model start of {id} was not saved: {e}"));
        }
    }
    Ok((id.to_string(), plan.member))
}

pub fn seen_of(info: &TermInfo) -> Seen {
    if !info.running {
        return Seen::Gone;
    }
    match info.status {
        Some(Status::Working) => Seen::Working,
        Some(Status::Blocked) => Seen::Waiting,
        _ => Seen::Idle,
    }
}

pub async fn state(terminal: &str) -> Result<Seen> {
    let id = engine::term_id(terminal)?;
    match engine::quick(Request::Get { id: id.clone() }).await {
        Ok(Reply::Term { term }) => Ok(seen_of(&term)),
        Ok(other) => bail!("Cannot see terminal {terminal}: unexpected reply {other:?}"),
        Err(error) => {
            let list = engine::list()
                .await
                .map_err(|_| anyhow!("Cannot see terminal {terminal}: {error}"))?;
            if list.iter().any(|t| t.id == id) {
                bail!("Cannot see terminal {terminal}: {error}");
            }
            Ok(Seen::Gone)
        }
    }
}

pub async fn screen(terminal: &str) -> Result<String> {
    let id = engine::term_id(terminal)?;
    engine::read(&id, Some(200), true).await
}

pub async fn git(folder: &str, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .output()
        .await
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

pub async fn exclude(folder: &str) -> Result<()> {
    let Some(common) = git(folder, &["rev-parse", "--git-common-dir"]).await else {
        return Ok(());
    };
    if git(
        folder,
        &["check-ignore", "-q", "docs/automations/report.md"],
    )
    .await
    .is_some()
    {
        return Ok(());
    }
    let file = Path::new(folder)
        .join(common.trim())
        .join("info")
        .join("exclude");
    let top = git(folder, &["rev-parse", "--show-toplevel"]).await.ok_or_else(|| anyhow!("{folder} is not a git work tree."))?;
    let top = std::fs::canonicalize(top.trim())?;
    let common_real = std::fs::canonicalize(Path::new(folder).join(common.trim()))?;
    if !common_real.starts_with(&top) && common_real.file_name().is_none_or(|n| n != ".git") {
        bail!("The git folder {} is outside {}.", common_real.display(), top.display());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if let Ok(meta) = file.symlink_metadata()
        && (meta.file_type().is_symlink() || !meta.is_file())
    {
        bail!("{} is a link or not a plain file.", file.display());
    }
    let before = std::fs::read_to_string(&file).unwrap_or_default();
    let line = if before.is_empty() || before.ends_with('\n') {
        "docs/\n"
    } else {
        "\ndocs/\n"
    };
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)?
        .write_all(line.as_bytes())?;
    Ok(())
}

pub async fn marker(folder: &str) -> Option<String> {
    let head = git(folder, &["rev-parse", "HEAD"]).await?;
    let status = git(folder, &["status", "--porcelain"]).await?;
    let refs = git(
        folder,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
            "refs/remotes",
        ],
    )
    .await
    .unwrap_or_default();
    Some(crate::util::short_hash(
        &[head, status, refs].join("\n--\n"),
    ))
}

pub async fn repo_of(folder: &Path) -> Option<Repo> {
    let path = folder.display().to_string();
    let top = git(&path, &["rev-parse", "--show-toplevel"]).await?;
    if Path::new(top.trim()) != folder {
        return None;
    }
    let branches = git(
        &path,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )
    .await
    .unwrap_or_default();
    let current = git(&path, &["branch", "--show-current"])
        .await
        .unwrap_or_default();
    Some(Repo {
        name: folder
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone()),
        folder: path,
        branches: branches
            .lines()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(str::to_string)
            .collect(),
        current: current.trim().to_string(),
    })
}

pub async fn repos(folder: &str) -> Vec<Repo> {
    let root = PathBuf::from(folder);
    let mut out = Vec::new();
    if let Some(repo) = repo_of(&root).await {
        out.push(repo);
    }
    let mut kids: Vec<PathBuf> = std::fs::read_dir(&root)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.join(".git").exists())
                .collect()
        })
        .unwrap_or_default();
    kids.sort();
    for kid in kids.into_iter().take(40) {
        if let Some(repo) = repo_of(&kid).await {
            out.push(repo);
        }
    }
    out
}
