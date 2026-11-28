use super::changes::{changed_files, git_changes};
use super::crew::{self, step_agent, title_of};
use super::flow::{self, CHECK_MS, Check, Live, Next, Seen};
use super::model::{By, Run, RunPr, RunState, RunStep, StepState, TeamsFile};
use super::prompt::{Mode, StepAsk, report_path, run_brief, run_summary, step_prompt};
use super::{TEAM_LABEL, changed, get, line, save, say, update, watch};
use crate::app::{App, lock};
use crate::schedules::runner::{self, Job, MEMBER_PREFIX};
use crate::spaces::Space;
use crate::util::{clip, now_ms};
use nixa_store::agents::CustomAgent;
use nixa_store::schedules::ScheduleJob;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct RunStart {
    pub team: String,
    pub task: String,
    pub project: String,
    pub by: By,
    pub space: Option<Space>,
    pub repo: String,
    pub branch: String,
    pub skills: Vec<String>,
    pub mcps: Vec<String>,
    pub brief: String,
    pub flow: String,
    pub pr: Option<RunPr>,
}

fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

pub fn agents(app: &App) -> Vec<CustomAgent> {
    nixa_store::open(&app.db_file).and_then(|c| nixa_store::agents::list_agents(&c)).unwrap_or_default()
}

pub fn teams_file(app: &App) -> Result<TeamsFile, String> {
    crew::load(&app.teams.teams_file).map_err(|e| format!("{e:#}"))
}

fn new_id(app: &App) -> String {
    let n = {
        let mut made = lock(&app.teams.made);
        *made += 1;
        *made
    };
    let at = now_ms();
    loop {
        let hash = crate::util::short_hash(&format!("{at}-{}-{n}-{}", std::process::id(), app.dir.display()));
        let id = format!("run-{}-{}", crate::util::day_of(at), &hash[..6]);
        if get(&app.teams, &id).is_none() {
            return id;
        }
        let mut made = lock(&app.teams.made);
        *made += 1;
    }
}

fn real_dir(path: &Path) -> Option<std::path::PathBuf> {
    crate::project_tools::policy::work_folder(path).ok().filter(|p| p.is_dir())
}

pub fn work_folder(project: &str, repo: &str, space_folder: &str) -> Result<String, String> {
    let project = project.trim();
    let repo = repo.trim();
    let space_folder = space_folder.trim();
    if !project.is_empty() {
        return real_dir(Path::new(project)).map(|p| p.display().to_string()).ok_or_else(|| format!("The project folder cannot be used: {project} is not a folder."));
    }
    if space_folder.is_empty() && repo.is_empty() {
        return Err(String::new());
    }
    if repo.is_empty() {
        return real_dir(Path::new(space_folder)).map(|p| p.display().to_string()).ok_or_else(|| format!("The project folder cannot be used: {space_folder} is not a folder."));
    }
    let base = if space_folder.is_empty() { None } else { Some(real_dir(Path::new(space_folder)).ok_or_else(|| format!("The project folder cannot be used: {space_folder} is not a folder."))?) };
    let wanted = if Path::new(repo).is_absolute() {
        Path::new(repo).to_path_buf()
    } else {
        let Some(base) = &base else {
            return Err(format!("The repo {repo} needs a space or a full folder path."));
        };
        if repo.contains('/') || repo == ".." || repo == "." {
            return Err(format!("The repo {repo} must be a repo name from repos or a full folder path."));
        }
        base.join(repo)
    };
    let real = real_dir(&wanted).ok_or_else(|| format!("The repo folder cannot be used: {} is not a folder.", wanted.display()))?;
    if let Some(base) = &base
        && !real.starts_with(base)
    {
        return Err(format!("The repo {} is not inside the space folder {}.", real.display(), base.display()));
    }
    if !real.join(".git").exists() {
        return Err(format!("{} is not a git repo.", real.display()));
    }
    Ok(real.display().to_string())
}

pub fn locked(app: &App, space: Option<&Space>, folder: &str) -> Option<String> {
    locked_at(app, space.map(|s| s.id.as_str()).unwrap_or_default(), folder)
}

pub fn run_locked(app: &App, run: &Run) -> Option<String> {
    locked_at(app, &run.space, &run.project)
}

fn locked_at(app: &App, space: &str, folder: &str) -> Option<String> {
    let rows = nixa_store::open(&app.db_file).and_then(|c| nixa_store::spaces::list_spaces(&c)).ok()?;
    let folder = folder.trim();
    rows.iter()
        .filter(|r| r.locked)
        .find(|r| (!space.is_empty() && space == r.space_id) || (!folder.is_empty() && !r.folder.trim().is_empty() && crate::spaces::folder_in(folder, &r.folder)))
        .map(|r| {
            let name = if r.name.trim().is_empty() { &r.space_id } else { &r.name };
            format!("Space \"{name}\" is locked. No new work starts there until the user unlocks it.")
        })
}

pub fn create(app: &Arc<App>, input: RunStart) -> Result<Run, String> {
    let task = input.task.trim().to_string();
    if task.is_empty() {
        return Err("Give the team a task.".into());
    }
    let file = teams_file(app)?;
    let wanted = input.team.trim();
    let wanted = wanted.get(..5).filter(|h| h.eq_ignore_ascii_case("team:")).map(|_| wanted[5..].trim()).unwrap_or(wanted);
    let Some(team) = crew::find_team(&file, wanted).cloned() else {
        let all: Vec<String> = file.teams.iter().map(|t| t.name.clone()).collect();
        return Err(format!("No team \"{wanted}\". Teams: {}.", if all.is_empty() { "none yet".to_string() } else { all.join(", ") }));
    };
    if team.steps.is_empty() {
        return Err(format!("The {} has no steps. Add steps to the team first.", team.name));
    }
    let space_folder = input.space.as_ref().map(|s| s.folder.clone()).unwrap_or_default();
    let project = work_folder(&input.project, &input.repo, &space_folder).map_err(|e| if e.is_empty() { format!("No space for the {}: pick a space with a folder.", team.name) } else { e })?;
    let space = input.space.clone().or_else(|| crate::spaces::for_folder(&app.spaces(), &project).cloned());
    let skills = crew::names(&[team.skills.clone(), input.skills.clone()].concat());
    if !skills.is_empty() {
        let dir = super::sources::skill_dir();
        let have = super::sources::skills_in(&dir);
        let missing: Vec<String> = skills.iter().filter(|s| !have.contains(s)).map(|s| format!("\"{s}\"")).collect();
        if !missing.is_empty() {
            return Err(format!("No skill named {} in {}. Check the name.", missing.join(", "), dir.display()));
        }
    }
    let mcps = crew::names(&[team.mcps.clone(), input.mcps.clone()].concat());
    if !mcps.is_empty() {
        let known = super::sources::mcp_names(&project);
        let missing: Vec<String> = mcps.iter().filter(|m| !known.contains(m)).map(|m| format!("\"{m}\"")).collect();
        if !known.is_empty() && !missing.is_empty() {
            return Err(format!("No MCP server named {}. Known: {}.", missing.join(", "), known.join(", ")));
        }
    }
    if let Some(why) = locked(app, space.as_ref(), &project) {
        return Err(why);
    }
    if let Some(going) = super::model::running_in(&lock(&app.teams.runs), &team.id, &project) {
        return Err(format!("The {} already runs in {} (run {}). Stop it first, or pick another space.", team.name, super::model::space_label(going), going.id));
    }
    let all = agents(app);
    let run = Run {
        id: new_id(app),
        team: team.id.clone(),
        team_name: team.name.clone(),
        task: task.clone(),
        project: project.clone(),
        by: input.by,
        space: space.as_ref().map(|s| s.id.clone()).unwrap_or_default(),
        space_name: space.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
        repo: input.repo.trim().to_string(),
        branch: input.branch.trim().to_string(),
        skills: skills.clone(),
        mcps: mcps.clone(),
        brief: input.brief.trim().to_string(),
        flow: input.flow.trim().to_string(),
        pr: input.pr.clone(),
        state: RunState::Running,
        step: 0,
        rounds: 0,
        started_at: now_ms(),
        ended_at: None,
        note: String::new(),
        steps: team
            .steps
            .iter()
            .map(|s| RunStep { agent: s.agent.clone(), name: crew::step_name(&all, &s.agent), note: s.note.clone(), ..RunStep::default() })
            .collect(),
    };
    lock(&app.teams.runs).push(run.clone());
    lock(&app.teams.busy).insert(run.id.clone());
    save(&app.teams);
    changed(app);
    if run.flow.is_empty() {
        let place = if run.space_name.is_empty() { String::new() } else { format!(" in {}", run.space_name) };
        say(app, &format!("Starting the {}{place} with {}: {}", team.name, plural(run.steps.len(), "step"), clip(&task, 80)));
    }
    let names: Vec<String> = run.steps.iter().map(|s| s.name.clone()).collect();
    let place = if run.space_name.is_empty() { project.clone() } else { format!("{} ({project})", run.space_name) };
    let mut text = format!("{}: run {} started in {place}", team.name, run.id);
    if !run.branch.is_empty() {
        text.push_str(&format!(", branch {}", run.branch));
    }
    text.push_str(&format!(". Steps: {}.", names.join(" → ")));
    if !skills.is_empty() {
        text.push_str(&format!(" Skills: {}.", skills.join(", ")));
    }
    if !mcps.is_empty() {
        text.push_str(&format!(" MCP: {}.", mcps.join(", ")));
    }
    line(app, &text);
    Ok(run)
}

pub fn launch(app: &Arc<App>, id: &str) {
    let app = app.clone();
    let id = id.to_string();
    tokio::spawn(async move {
        begin_step(&app, &id, 0, Mode::First).await;
        lock(&app.teams.busy).remove(&id);
    });
}

pub fn start(app: &Arc<App>, input: RunStart) -> Result<Run, String> {
    let run = create(app, input)?;
    launch(app, &run.id);
    Ok(run)
}

pub fn schedule_space(app: &App, job: &ScheduleJob, folder: &str) -> Option<Space> {
    let spaces = app.spaces();
    spaces.iter().find(|s| !job.space.is_empty() && s.id == job.space).or_else(|| crate::spaces::for_folder(&spaces, folder)).cloned()
}

pub fn start_for_schedule(app: &Arc<App>, team: &str, task: &str, folder: &str, job: Option<&ScheduleJob>) -> Result<String, String> {
    let job = job.cloned().unwrap_or_default();
    let space = schedule_space(app, &job, folder);
    let project = if job.repo.trim().is_empty() { folder.to_string() } else { job.repo.trim().to_string() };
    let input = RunStart {
        team: team.to_string(),
        task: task.to_string(),
        project,
        by: By::Schedule,
        space,
        repo: job.repo.clone(),
        branch: job.branch.clone(),
        skills: job.skills.clone(),
        mcps: job.mcps.clone(),
        ..RunStart::default()
    };
    start(app, input).map(|r| r.id)
}

pub fn end_run(app: &Arc<App>, id: &str, state: RunState, note: &str) {
    let Some(run) = update(&app.teams, id, |run| {
        run.state = state;
        run.ended_at = Some(now_ms());
        run.note = note.to_string();
        run.clone()
    }) else {
        return;
    };
    lock(&app.teams.live).remove(id);
    changed(app);
    let summary = run_summary(&run);
    line(app, &summary.chat);
    if run.flow.is_empty() {
        say(app, &summary.spoken);
    } else {
        crate::git_tools::pr_flow::ended(app, &run);
    }
}

pub fn seen_of(seen: runner::Seen) -> Seen {
    match seen {
        runner::Seen::Working => Seen::Working,
        runner::Seen::Waiting => Seen::Waiting,
        runner::Seen::Idle => Seen::Idle,
        runner::Seen::Gone => Seen::Gone,
    }
}

async fn alive(terminal: &str) -> bool {
    !terminal.is_empty() && runner::state(terminal).await.map(|s| s != runner::Seen::Gone).unwrap_or(false)
}

fn note_use(app: &App, agent: &str, terminal: &str, run: &Run) {
    let used = nixa_store::agents::AgentUse {
        terminal: Some(terminal.to_string()),
        folder: Some(run.project.clone()),
        space: (!run.space.is_empty()).then(|| run.space.clone()),
        space_name: (!run.space_name.is_empty()).then(|| run.space_name.clone()),
        ..Default::default()
    };
    if let Err(e) = nixa_store::open(&app.db_file).and_then(|c| nixa_store::agents::use_agent(&c, agent, &used)) {
        crate::util::log("teams", &format!("could not note that {agent} started: {e:#}"));
    }
}

async fn open_step(app: &Arc<App>, run: &Run, index: usize, agent: &CustomAgent, mode: Mode, swapped: bool, report: &Path) -> Result<String, String> {
    if let Some(dir) = report.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    }
    let _ = runner::exclude(&run.project).await;
    let step = &run.steps[index];
    let reuse = mode != Mode::First && !swapped && alive(&step.terminal).await;
    let file = teams_file(app).unwrap_or_default();
    let purpose = crew::find_team(&file, &run.team).map(|t| t.purpose.clone()).unwrap_or_default();
    let dir = super::sources::skill_dir();
    let text = step_prompt(&StepAsk { run, index, purpose: &purpose, note: &step.note, mode, fresh: !reuse, report, skill_dir: &dir });
    if reuse {
        let id = crate::engine::term_id(&step.terminal).map_err(|e| format!("{e:#}"))?;
        crate::engine::send(&id, &text, true, false).await.map_err(|e| format!("{e:#}"))?;
        return Ok(step.terminal.clone());
    }
    let job = Job {
        schedule: run.id.clone(),
        name: format!("{} step {}", run.team_name, index + 1),
        agent: format!("{MEMBER_PREFIX}{}", agent.id),
        folder: run.project.clone(),
        prompt: text,
    };
    let (terminal, member) = runner::open(app, &job).await.map_err(|e| format!("{e:#}"))?;
    if let Ok(id) = crate::engine::term_id(&terminal) {
        let _ = crate::engine::label(&id, TEAM_LABEL, Some(&run.id)).await;
        let _ = crate::engine::label(&id, crate::custom_agents::CUSTOM_LABEL, Some(&agent.id)).await;
    }
    if let Some(member) = member {
        note_use(app, &member, &terminal, run);
    }
    Ok(terminal)
}

pub async fn begin_step(app: &Arc<App>, id: &str, index: usize, mode: Mode) {
    let Some(run) = get(&app.teams, id) else { return };
    if run.state != RunState::Running || index >= run.steps.len() {
        return;
    }
    if let Some(why) = run_locked(app, &run) {
        return end_run(app, id, RunState::Stopped, &format!("Step {} did not start. {why} Press Resume after unlocking it.", index + 1));
    }
    let file = match teams_file(app) {
        Ok(file) => file,
        Err(e) => return end_run(app, id, RunState::Failed, &format!("Step {}: {e}", index + 1)),
    };
    let all = agents(app);
    let pick = step_agent(&file, &all, &run.steps[index].agent);
    let Some(agent) = pick.agent else {
        return end_run(app, id, RunState::Failed, &format!("Step {}: {}", index + 1, pick.note));
    };
    if !pick.note.is_empty() {
        line(app, &format!("{}: {}", run.team_name, pick.note));
    }
    let swapped = run.steps[index].agent != agent.id;
    let Some(run) = update(&app.teams, id, |run| {
        let rounds = run.rounds;
        let step = &mut run.steps[index];
        step.agent = agent.id.clone();
        step.name = title_of(&agent);
        step.round = rounds;
        run.clone()
    }) else {
        return;
    };
    let report = report_path(&run.project, &run.id, index, &agent.id, run.rounds);
    let before = git_changes(&run.project).await;
    let opened = open_step(app, &run, index, &agent, mode, swapped, &report).await;
    let terminal = match opened {
        Ok(terminal) => terminal,
        Err(e) => {
            if get(&app.teams, id).is_some_and(|r| r.state == RunState::Running) {
                end_run(app, id, RunState::Failed, &format!("Step {} ({}) did not start: {e}", index + 1, title_of(&agent)));
            }
            return;
        }
    };
    let Some(Some(run)) = update(&app.teams, id, |run| {
        if run.state != RunState::Running {
            return None;
        }
        let step = &mut run.steps[index];
        step.state = StepState::Working;
        step.terminal = terminal.clone();
        step.started_at = Some(now_ms());
        step.report = report.display().to_string();
        step.ended_at = None;
        step.result = None;
        run.step = index;
        Some(run.clone())
    }) else {
        return;
    };
    lock(&app.teams.live).insert(id.to_string(), Live { before, ..Live::default() });
    changed(app);
    line(app, &format!("{}: step {} of {}, {} {} in terminal {terminal}.", run.team_name, index + 1, run.steps.len(), title_of(&agent), flow::doing(mode, run.rounds)));
}

pub async fn finish_step(app: &Arc<App>, id: &str, index: usize, text: &str) {
    let Some(run) = get(&app.teams, id) else { return };
    let after = git_changes(&run.project).await;
    if get(&app.teams, id).is_none_or(|r| r.state != RunState::Running) {
        return;
    }
    let before = lock(&app.teams.live).remove(id).map(|l| l.before);
    let files = before.map(|b| changed_files(&b, &after)).unwrap_or_default();
    let Some((text, next)) = update(&app.teams, id, |run| flow::after_step(run, index, text, files, now_ms())) else { return };
    changed(app);
    line(app, &text);
    match next {
        Next::Begin(at, mode) => match step_held(app, &run, at, mode) {
            Some(held) => end_run(app, id, RunState::Stopped, &format!("Step {} waits. {held} Press Resume to go on.", at + 1)),
            None => begin_step(app, id, at, mode).await,
        },
        Next::End(state, note) => end_run(app, id, state, &note),
    }
}

pub fn step_action(mode: Mode) -> &'static str {
    if mode == Mode::Fix { "fix" } else { "team-step" }
}

fn step_held(app: &Arc<App>, run: &Run, index: usize, mode: Mode) -> Option<String> {
    crate::modes::hold_work(app, step_action(mode), &format!("step {} of the {}", index + 1, run.team_name), None)
}

pub async fn check(app: &Arc<App>, id: &str) {
    let Some(run) = get(&app.teams, id) else { return };
    let index = run.step;
    let Some(step) = run.steps.get(index).cloned() else { return };
    if step.state != StepState::Working || step.terminal.is_empty() || step.report.is_empty() {
        return;
    }
    let seen = match runner::state(&step.terminal).await {
        Ok(seen) => seen_of(seen),
        Err(e) => {
            line(app, &format!("{}: could not check step {}: {e:#}", run.team_name, index + 1));
            return;
        }
    };
    let Some(run) = get(&app.teams, id).filter(|r| r.state == RunState::Running && r.step == index) else { return };
    let ready = Path::new(&step.report).is_file();
    let decision = {
        let mut live = lock(&app.teams.live);
        let state = live.entry(id.to_string()).or_default();
        flow::check_step(state, &run, index, seen, ready, now_ms())
    };
    match decision {
        Check::Wait => {}
        Check::Tell(text) => crate::watch::tell_event(app, &text),
        Check::Finish => match std::fs::read_to_string(&step.report) {
            Ok(text) => finish_step(app, id, index, &text).await,
            Err(e) => line(app, &format!("{}: could not read the report of step {}: {e}", run.team_name, index + 1)),
        },
        Check::End(state, note) => end_run(app, id, state, &note),
    }
}

pub fn stop(app: &Arc<App>, id: &str) -> Result<Run, String> {
    let run = get(&app.teams, id).ok_or_else(|| format!("No team run \"{}\". Use the list of runs.", id.trim()))?;
    if run.state != RunState::Running {
        return Err(format!("The {} run is not running. It is {}.", run.team_name, run.state.word()));
    }
    update(&app.teams, &run.id, |run| {
        let at = run.step;
        if let Some(step) = run.steps.get_mut(at)
            && step.state == StepState::Working
        {
            step.state = StepState::Waiting;
        }
    });
    end_run(app, &run.id, RunState::Stopped, "Stopped by the user. The tabs stay open.");
    get(&app.teams, &run.id).ok_or_else(|| "The run is gone.".to_string())
}

pub fn resume_check(app: &Arc<App>, id: &str) -> Result<Run, String> {
    let run = get(&app.teams, id).ok_or_else(|| format!("No team run \"{}\". Use the list of runs.", id.trim()))?;
    if run.state != RunState::Cut && run.state != RunState::Stopped {
        return Err(format!("Only a cut off or stopped run can go on. The {} run is {}.", run.team_name, run.state.word()));
    }
    if run.steps.is_empty() {
        return Err(format!("The {} run has no steps.", run.team_name));
    }
    if let Some(why) = run_locked(app, &run) {
        return Err(why);
    }
    if !lock(&app.teams.busy).insert(run.id.clone()) {
        return Err(format!("The {} run is busy. Try again in a moment.", run.team_name));
    }
    let index = run.step.min(run.steps.len() - 1);
    let run = update(&app.teams, &run.id, |run| {
        run.state = RunState::Running;
        run.ended_at = None;
        run.note = String::new();
        run.step = index;
        run.clone()
    })
    .ok_or_else(|| "The run is gone.".to_string())?;
    changed(app);
    say(app, &format!("Going on with the {} at step {} of {}.", run.team_name, index + 1, run.steps.len()));
    Ok(run)
}

pub async fn resume_go(app: &Arc<App>, id: &str) {
    let Some(run) = get(&app.teams, id) else { return };
    let index = run.step;
    let step = run.steps[index].clone();
    let working = |app: &Arc<App>, seen_working: bool, before| {
        update(&app.teams, id, |run| run.steps[index].state = StepState::Working);
        lock(&app.teams.live).insert(id.to_string(), Live { seen_working, before, ..Live::default() });
        changed(app);
    };
    if !step.report.is_empty() && Path::new(&step.report).is_file() {
        let before = git_changes(&run.project).await;
        working(app, true, before);
        return;
    }
    let status = if step.terminal.is_empty() { runner::Seen::Gone } else { runner::state(&step.terminal).await.unwrap_or(runner::Seen::Gone) };
    if !step.report.is_empty() && matches!(status, runner::Seen::Working | runner::Seen::Waiting) {
        let before = git_changes(&run.project).await;
        working(app, status == runner::Seen::Working, before);
        line(app, &format!("{}: step {} of {}, {} is still working in terminal {}.", run.team_name, index + 1, run.steps.len(), step.name, step.terminal));
        return;
    }
    begin_step(app, id, index, flow::resume_mode(&run, index)).await;
}

pub fn resume(app: &Arc<App>, id: &str) -> Result<Run, String> {
    let run = resume_check(app, id)?;
    let app = app.clone();
    let id = run.id.clone();
    tokio::spawn(async move {
        resume_go(&app, &id).await;
        lock(&app.teams.busy).remove(&id);
    });
    Ok(run)
}

pub fn tick(app: &Arc<App>) {
    let at = now_ms();
    {
        let mut last = lock(&app.teams.last_check);
        if at.saturating_sub(*last) < CHECK_MS {
            return;
        }
        *last = at;
    }
    let ids: Vec<String> = lock(&app.teams.runs).iter().filter(|r| r.state == RunState::Running).map(|r| r.id.clone()).collect();
    for id in ids {
        if !lock(&app.teams.busy).insert(id.clone()) {
            continue;
        }
        let app = app.clone();
        tokio::spawn(async move {
            check(&app, &id).await;
            lock(&app.teams.busy).remove(&id);
        });
    }
}

struct Watched {
    terminal: String,
    agent: String,
    name: String,
    tab: String,
    working: bool,
}

fn watched(app: &App) -> Vec<Watched> {
    let all = agents(app);
    app.terminals()
        .into_iter()
        .filter(|s| s.info.running)
        .filter_map(|s| {
            let info = s.info;
            let agent_id = info.labels.get(crate::custom_agents::CUSTOM_LABEL).cloned().unwrap_or_default();
            let team = info.labels.contains_key(TEAM_LABEL);
            if agent_id.is_empty() && !team {
                return None;
            }
            let name = all.iter().find(|a| a.id == agent_id).map(title_of).unwrap_or_else(|| if info.name.is_empty() { info.id.to_string() } else { info.name.clone() });
            Some(Watched {
                terminal: info.id.to_string(),
                agent: agent_id,
                name,
                tab: info.name.clone(),
                working: info.status == Some(nixa_protocol::Status::Working),
            })
        })
        .collect()
}

pub fn watch_tick(app: &Arc<App>) {
    {
        let mut busy = lock(&app.teams.watching);
        if *busy {
            return;
        }
        *busy = true;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let list = watched(&app);
        let alive: HashSet<String> = list.iter().map(|w| w.terminal.clone()).collect();
        watch::keep(&mut lock(&app.teams.watch), &alive);
        for item in list {
            let now = now_ms();
            if !watch::due(&lock(&app.teams.watch), &item.terminal, now, watch::READ_MS) {
                continue;
            }
            let screen = if item.working { runner::screen(&item.terminal).await.unwrap_or_default() } else { String::new() };
            let look = watch::Look { terminal: &item.terminal, number: None, agent: &item.agent, name: &item.name, tab: &item.tab, working: item.working, screen: &screen };
            let note = watch::check(&mut lock(&app.teams.watch), &look, now_ms(), watch::QUIET_MS);
            if let Some(note) = note {
                crate::watch::tell_event(&app, &note);
            }
        }
        *lock(&app.teams.watching) = false;
    });
}

pub fn briefs(runs: &[Run], most: usize) -> Vec<Value> {
    runs.iter().rev().take(most).map(run_brief).collect()
}
