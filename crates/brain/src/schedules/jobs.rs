use super::runner::{self, Job, Seen, is_team, pauses};
use super::zone::{self, Zone};
use super::{report, when};
use crate::app::{App, lock};
use crate::util::{log, now_ms};
use anyhow::{Result, anyhow, bail};
use nixa_store::schedules::{
    self as store, LastRun, Repeat, RunResult, Schedule, ScheduleInput, ScheduleJob, SchedulePatch,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod repeats;

pub const ENABLED_KEY: &str = "schedules.enabled";
pub const LATE_MS: i64 = 60_000;
pub const LATE_RUN_MS: i64 = 12 * 60 * 60 * 1000;
pub const CHECK_MS: i64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub next: Option<i64>,
    pub disable: bool,
    pub note: Option<String>,
    pub run: bool,
}

pub fn now() -> i64 {
    now_ms() as i64
}

pub fn zone_of(app: &App) -> Zone {
    zone::cached(&app.schedules.zone, now_ms())
}

pub fn offset_minutes(zone: &Zone, at: i64) -> i64 {
    zone::offset_at(zone, at.div_euclid(1000)) / 60
}

pub fn conn(app: &App) -> Result<nixa_store::Connection> {
    nixa_store::open(&app.db_file)
}

pub fn list(app: &App) -> Result<Vec<Schedule>> {
    store::list_schedules(&conn(app)?)
}

pub fn get(app: &App, id: &str) -> Result<Schedule> {
    store::get_schedule(&conn(app)?, id)?.ok_or_else(|| anyhow!("Schedule not found."))
}

pub fn all_on(app: &App) -> bool {
    conn(app)
        .and_then(|c| nixa_store::settings::setting_bool(&c, ENABLED_KEY, true))
        .unwrap_or(true)
}

pub fn notify(app: &App, text: &str) {
    log("schedules", text);
    app.send_out(json!({"type": "schedule_event", "text": text}));
    crate::announce::add_event(
        app,
        crate::announce::AgentEvent {
            time: now_ms(),
            kind: "done".into(),
            text: text.to_string(),
            terminal: String::new(),
            space: String::new(),
            job: String::new(),
            agent_number: None,
            agent: "Nixa".into(),
            offer: None,
        },
    );
}

pub fn changed(app: &App) {
    let mut body = super::snapshot(app);
    body["type"] = json!("schedules");
    app.send_out(body);
}

pub fn claim(app: &App, id: &str) -> bool {
    lock(&app.schedules.busy).insert(id.to_string())
}

pub fn release(app: &App, id: &str) {
    lock(&app.schedules.busy).remove(id);
}

pub fn is_busy(app: &App, id: &str) -> bool {
    lock(&app.schedules.busy).contains(id)
}

pub fn new_run(at: i64, result: RunResult) -> LastRun {
    LastRun {
        at,
        result,
        terminal: String::new(),
        report: String::new(),
        done_at: None,
        open_tasks: None,
        follow_terminal: String::new(),
        team_run: String::new(),
        error: String::new(),
        marker: String::new(),
    }
}

pub fn step(zone: &Zone, s: &Schedule, at: i64) -> Option<Step> {
    let due = s.next_run?;
    if !s.enabled || due > at {
        return None;
    }
    let late = at - due > LATE_MS;
    let missed = if late {
        when::missed_runs(zone, s, due, at)
    } else {
        0
    };
    let once = s.repeat == Repeat::Once;
    let next = if once {
        None
    } else {
        when::schedule_next(zone, s, at)
    };
    if late && at - due > LATE_RUN_MS {
        let after = match next {
            Some(n) => format!("Next run: {}.", when::date_time(zone, n)),
            None => "It does not run again.".into(),
        };
        let note = format!(
            "{} missed its run at {}, more than 12 hours ago. {after}",
            s.name,
            when::clock(zone, due)
        );
        return Some(Step {
            next,
            disable: once,
            note: Some(note),
            run: false,
        });
    }
    let note = late.then(|| {
        let more = if missed > 1 {
            format!(", and {missed} runs were missed")
        } else {
            String::new()
        };
        format!(
            "{} was due at {}{more}. It runs once now.",
            s.name,
            when::clock(zone, due)
        )
    });
    Some(Step {
        next,
        disable: once,
        note,
        run: true,
    })
}

fn set_next(app: &App, id: &str, next: Option<i64>) -> Result<()> {
    store::set_next_run(&conn(app)?, id, next).map(|_| ())
}

fn pause(app: &App, id: &str) {
    let zone = zone_of(app);
    let done = conn(app).and_then(|c| {
        store::set_schedule_enabled(&c, id, false, offset_minutes(&zone, now()))?;
        store::set_next_run(&c, id, None)
    });
    if let Err(error) = done {
        log("schedules", &format!("could not pause {id}: {error:#}"));
    }
}

fn save_run(app: &App, id: &str, run: &LastRun) {
    if let Err(error) = conn(app).and_then(|c| store::update_last_run(&c, id, run)) {
        notify(app, &format!("Could not save schedules: {error:#}"));
    }
}

pub fn plan_missing(app: &Arc<App>) {
    let zone = zone_of(app);
    let at = now();
    for s in list(app).unwrap_or_default() {
        if s.enabled
            && s.next_run.is_none()
            && let Some(next) = when::schedule_next(&zone, &s, at)
        {
            let _ = set_next(app, &s.id, Some(next));
        }
    }
}

pub fn tick(app: &Arc<App>) {
    repeats::tick(app);
    let at = now();
    {
        let mut last = lock(&app.schedules.last_check);
        if at - *last >= CHECK_MS {
            *last = at;
            tokio::spawn(watch_runs(app.clone()));
        }
    }
    if !all_on(app) {
        return;
    }
    let zone = zone_of(app);
    let schedules = match list(app) {
        Ok(list) => list,
        Err(error) => {
            log("schedules", &format!("cannot read schedules: {error:#}"));
            return;
        }
    };
    let mut any = false;
    for s in schedules {
        if is_busy(app, &s.id) {
            continue;
        }
        let Some(step) = step(&zone, &s, at) else {
            continue;
        };
        if step.run && automatic_due_held(app, at as u64) {
            continue;
        }
        any = true;
        let saved = conn(app).and_then(|c| {
            if step.disable {
                store::set_schedule_enabled(&c, &s.id, false, offset_minutes(&zone, at))?;
            }
            store::set_next_run(&c, &s.id, step.next)
        });
        if let Err(error) = saved {
            notify(app, &format!("Could not save schedules: {error:#}"));
            continue;
        }
        if let Some(note) = &step.note {
            notify(app, note);
        }
        if step.run && claim(app, &s.id) {
            let app = app.clone();
            tokio::spawn(async move {
                if let Err(error) = run_claimed(&app, &s.id, false).await {
                    notify(&app, &format!("{} did not start: {error:#}", s.name));
                }
                release(&app, &s.id);
                changed(&app);
            });
        }
    }
    if any {
        changed(app);
    }
}

pub(super) fn automatic_due_held(app: &App, now: u64) -> bool {
    crate::term_tools::hold_stops(
        crate::modes::work_now(app).hold.map(|hold| hold.at),
        crate::modes::passed(app),
        "schedule-run",
        now,
    )
}

async fn still_working(run: Option<&LastRun>) -> Option<String> {
    let run = run?;
    for terminal in [&run.terminal, &run.follow_terminal] {
        if !terminal.is_empty()
            && matches!(
                runner::state(terminal).await,
                Ok(Seen::Working | Seen::Waiting)
            )
        {
            return Some(terminal.clone());
        }
    }
    None
}

async fn unchanged(app: &App, s: &Schedule) -> bool {
    let Some(run) = &s.last_run else { return false };
    if !s.skip_unchanged
        || run.result != RunResult::Started
        || run.done_at.is_none()
        || run.marker.is_empty()
    {
        return false;
    }
    let Some(marker) = runner::marker(&s.folder).await else {
        return false;
    };
    if marker != run.marker {
        return false;
    }
    let zone = zone_of(app);
    let at = now();
    let kept = conn(app).and_then(|c| {
        store::note_skipped(&c, &s.id, at, offset_minutes(&zone, at))?;
        store::set_next_run(&c, &s.id, s.next_run)
    });
    if let Err(error) = kept {
        log("schedules", &format!("could not note a skip: {error:#}"));
    }
    if lock(&app.schedules.quiet).insert(s.id.clone()) {
        notify(
            app,
            &format!(
                "{} skipped: nothing changed since its last run at {}. It stays quiet until something changes.",
                s.name,
                when::clock(&zone, run.at)
            ),
        );
    }
    true
}

pub fn held_event(name: &str, held: &str) -> String {
    format!("Event: The schedule {name} did not run: {} It runs again at its next time. Do not ask the user: when the hold ends, run it now if it is still needed.", crate::util::clip(held, 200))
}

pub async fn run_claimed(app: &Arc<App>, id: &str, manual: bool) -> Result<()> {
    let s = get(app, id)?;
    if !manual && unchanged(app, &s).await {
        return Ok(());
    }
    if !manual && let Some(held) = crate::term_tools::work_hold(app, "schedule-run", &format!("run the schedule {}", s.name), None) {
        crate::util::log("schedules", &format!("{} held: {}", s.name, crate::util::clip(&held, 160)));
        crate::agent::event(app, &held_event(&s.name, &held));
        return Ok(());
    }
    lock(&app.schedules.quiet).remove(id);
    let at = now();
    if let Some(team) = s.agent.strip_prefix(crate::schedules::runner::TEAM_PREFIX) {
        let started = if pr_team(app, team) {
            start_pr_flow(app, team, &s).await
        } else {
            crate::teams::runs::start_for_schedule(app, team, &s.prompt, &s.folder, s.job.as_ref())
        };
        let run = match started {
            Ok(run_id) => LastRun { team_run: run_id, ..new_run(at, RunResult::Started) },
            Err(error) => LastRun { error, ..new_run(at, RunResult::Failed) },
        };
        save_run(app, id, &run);
        if run.error.is_empty() {
            notify(app, &format!("{} started team run {}.", s.name, run.team_run));
        } else {
            notify(app, &format!("{} failed: {}", s.name, run.error));
        }
        return Ok(());
    }
    if s.agent == crate::assistant::AGENT {
        let run = crate::assistant::jobs::run_schedule(app, &s).await;
        save_run(app, id, &run);
        if !run.error.is_empty() {
            notify(app, &format!("{} failed: {}", s.name, run.error));
        }
        return Ok(());
    }
    if let Some(terminal) = still_working(s.last_run.as_ref()).await {
        notify(
            app,
            &format!(
                "{} skipped: terminal {terminal} from the last run is still working.",
                s.name
            ),
        );
        return Ok(());
    }
    let zone = zone_of(app);
    let outcome: Result<(String, String)> = async {
        let report = report::report_path(&zone, &s, at);
        if let Some(dir) = report.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let report = report.display().to_string();
        if let Err(error) = runner::exclude(&s.folder).await {
            log(
                "schedules",
                &format!(
                    "docs/ was not added to the git exclude file of {}: {error:#}",
                    s.folder
                ),
            );
        }
        let job = Job {
            schedule: s.id.clone(),
            name: s.name.clone(),
            agent: s.agent.clone(),
            folder: s.folder.clone(),
            prompt: report::first_message(&s, &report),
        };
        let (terminal, member) = runner::open(app, &job).await?;
        note_member(app, member.as_deref(), &terminal, &s.folder);
        Ok((terminal, report))
    }
    .await;
    let run = match outcome {
        Ok((terminal, report)) => LastRun {
            terminal,
            report,
            ..new_run(at, RunResult::Started)
        },
        Err(error) => {
            if pauses(&error) {
                pause(app, id);
            }
            LastRun {
                error: format!("{error:#}"),
                ..new_run(at, RunResult::Failed)
            }
        }
    };
    save_run(app, id, &run);
    match run.result {
        RunResult::Started => notify(
            app,
            &format!("{} started in terminal {}.", s.name, run.terminal),
        ),
        RunResult::Failed => notify(app, &format!("{} failed: {}", s.name, run.error)),
    }
    Ok(())
}

fn pr_team(app: &App, team: &str) -> bool {
    crate::teams::runs::teams_file(app)
        .ok()
        .and_then(|f| crate::teams::crew::find_team(&f, team).map(|t| t.flow == "pr"))
        .unwrap_or(false)
}

async fn start_pr_flow(app: &Arc<App>, team: &str, s: &Schedule) -> Result<String, String> {
    let job = s.job.clone().unwrap_or_default();
    let space = crate::teams::runs::schedule_space(app, &job, &s.folder);
    let project = if job.repo.trim().is_empty() { s.folder.clone() } else { job.repo.trim().to_string() };
    if let Some(why) = crate::teams::runs::locked(app, space.as_ref(), &project) {
        return Err(why);
    }
    let start = crate::git_tools::pr_flow::FlowStart {
        team: team.to_string(),
        project,
        branch: job.branch.clone(),
        space,
        skills: job.skills.clone(),
        mcps: job.mcps.clone(),
        task: s.prompt.clone(),
        by: crate::teams::model::By::Schedule,
    };
    crate::git_tools::pr_flow::start(app, start).await.map(|flow| flow.id)
}

fn note_member(app: &App, member: Option<&str>, terminal: &str, folder: &str) {
    let Some(member) = member else { return };
    let used = nixa_store::agents::AgentUse {
        terminal: Some(terminal.to_string()),
        folder: Some(folder.to_string()),
        ..Default::default()
    };
    if let Err(error) = conn(app).and_then(|c| nixa_store::agents::use_agent(&c, member, &used)) {
        log(
            "schedules",
            &format!("could not note that {member} started: {error:#}"),
        );
    }
}

pub async fn work(
    app: &Arc<App>,
    id: &str,
    task: Option<String>,
    agent: Option<String>,
) -> Result<String> {
    let s = get(app, id)?;
    work_from(app, &s, task, agent).await
}

async fn work_from(
    app: &Arc<App>,
    s: &Schedule,
    task: Option<String>,
    agent: Option<String>,
) -> Result<String> {
    let run = report_run(s)?;
    let report = &run.report;
    let agent = agent.unwrap_or_else(|| {
        if s.follow_agent.is_empty() { s.agent.clone() } else { s.follow_agent.clone() }
    });
    let job = Job {
        schedule: s.id.clone(),
        name: format!("{} tasks", s.name),
        agent,
        folder: s.folder.clone(),
        prompt: report::work_prompt(report, task.as_deref(), &s.follow_prompt),
    };
    let (terminal, member) = runner::open(app, &job).await?;
    note_member(app, member.as_deref(), &terminal, &s.folder);
    if let Some(mut current) = get(app, &s.id)?.last_run
        && current.at == run.at && current.report == run.report
    {
        current.follow_terminal = terminal.clone();
        save_run(app, &s.id, &current);
    }
    notify(
        app,
        &format!(
            "{}: an agent is working on the report tasks in terminal {terminal}.",
            s.name
        ),
    );
    Ok(terminal)
}

fn report_run(s: &Schedule) -> Result<&LastRun> {
    if is_team(&s.agent) {
        bail!(
            "{} runs a team. Its steps work on the task; there is no report task list.",
            s.name
        );
    }
    s.last_run.as_ref().filter(|run| !run.report.is_empty() && Path::new(&run.report).is_file())
        .ok_or_else(|| anyhow!("{} has no report yet.", s.name))
}

pub(crate) fn queue_report_work(app: &Arc<App>, s: Schedule, task: Option<String>, agent: Option<String>) -> Result<()> {
    report_run(&s)?;
    if !claim(app, &s.id) {
        bail!("{} is busy. Try again in a moment.", s.name);
    }
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(error) = work_from(&app, &s, task, agent).await {
            notify(&app, &format!("{}: the agent did not start: {error:#}", s.name));
        }
        release(&app, &s.id);
        changed(&app);
    });
    Ok(())
}

async fn finish(app: &Arc<App>, s: &Schedule, run: &LastRun) {
    let zone = zone_of(app);
    let report = PathBuf::from(&run.report);
    if !report.exists() {
        let screen = runner::screen(&run.terminal)
            .await
            .unwrap_or_else(|e| format!("(Could not read the screen: {e:#})"));
        let written = report
            .parent()
            .map(std::fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|_| std::fs::write(&report, report::fallback_report(&zone, s, run, &screen)));
        if let Err(error) = written {
            log(
                "schedules",
                &format!("could not write {}: {error}", report.display()),
            );
        }
    }
    let tasks = report::open_tasks(&std::fs::read_to_string(&report).unwrap_or_default());
    let marker = if s.skip_unchanged {
        runner::marker(&s.folder).await
    } else {
        None
    };
    if let Ok(current) = get(app, &s.id)
        && let Some(mut last) = current.last_run
        && last.at == run.at
    {
        last.done_at = Some(now());
        last.open_tasks = Some(tasks);
        if let Some(marker) = marker {
            last.marker = marker;
        }
        save_run(app, &s.id, &last);
    }
    notify(
        app,
        &format!(
            "{} is done. Report: {}. Open tasks: {tasks}.",
            s.name, run.report
        ),
    );
    if tasks > 0
        && !s.follow_agent.is_empty()
        && let Err(error) = work(app, &s.id, None, None).await
    {
        if pauses(&error) {
            pause(app, &s.id);
        }
        notify(
            app,
            &format!("{}: the follow-up agent did not start: {error:#}", s.name),
        );
    }
}

pub async fn watch_runs(app: Arc<App>) {
    for s in list(&app).unwrap_or_default() {
        let Some(run) = s.last_run.clone() else {
            continue;
        };
        if run.result != RunResult::Started
            || run.done_at.is_some()
            || run.terminal.is_empty()
            || run.report.is_empty()
        {
            continue;
        }
        if !claim(&app, &s.id) {
            continue;
        }
        let checked: Result<bool> = async {
            let seen = runner::state(&run.terminal).await?;
            if seen == Seen::Working {
                lock(&app.schedules.seen_working).insert(run.terminal.clone());
            }
            if matches!(seen, Seen::Working | Seen::Waiting) {
                return Ok(false);
            }
            let worked = lock(&app.schedules.seen_working).contains(&run.terminal);
            if seen == Seen::Idle && !worked && !Path::new(&run.report).exists() {
                return Ok(false);
            }
            lock(&app.schedules.seen_working).remove(&run.terminal);
            finish(&app, &s, &run).await;
            Ok(true)
        }
        .await;
        release(&app, &s.id);
        match checked {
            Ok(true) => changed(&app),
            Ok(false) => {}
            Err(error) => log(
                "schedules",
                &format!("{}: could not check the run: {error:#}", s.name),
            ),
        }
    }
}

fn text_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn needed(v: &Value, key: &str, label: &str) -> Result<String> {
    let t = text_of(v, key);
    if t.is_empty() {
        bail!("{label} is required.");
    }
    Ok(t)
}

fn names(v: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in v.and_then(Value::as_array).cloned().unwrap_or_default() {
        if let Some(t) = item.as_str().map(str::trim).filter(|t| !t.is_empty())
            && !out.iter().any(|o| o == t)
        {
            out.push(t.to_string());
        }
    }
    out
}

pub fn read_job(v: Option<&Value>) -> Option<ScheduleJob> {
    let v = v?;
    let job = ScheduleJob {
        space: text_of(v, "space"),
        repo: text_of(v, "repo"),
        branch: text_of(v, "branch"),
        skills: names(v.get("skills")),
        mcps: names(v.get("mcps")),
    };
    (job != ScheduleJob::default()).then_some(job)
}

pub fn keep_lists(job: Option<ScheduleJob>, old: Option<&ScheduleJob>, sent: Option<&Value>) -> Option<ScheduleJob> {
    let Some(old) = old else { return job };
    let mut job = job.unwrap_or_default();
    let missing = |key: &str| sent.and_then(|v| v.get(key)).is_none();
    if missing("skills") {
        job.skills = old.skills.clone();
    }
    if missing("mcps") {
        job.mcps = old.mcps.clone();
    }
    (job != ScheduleJob::default()).then_some(job)
}

pub fn folder_of(app: &App, text: &str) -> Result<String> {
    let mut path = text.trim().to_string();
    if let Some(rest) = path.strip_prefix("~/") {
        path = std::env::var("HOME")
            .map(|h| format!("{h}/{rest}"))
            .unwrap_or(path);
    }
    if !path.starts_with('/') {
        let spaces = app.spaces();
        let Some(space) = crate::spaces::find(&spaces, &path) else {
            bail!("Use a full folder path like ~/project, or a space name.")
        };
        path = space.folder.clone();
    }
    let real = crate::project_tools::policy::work_folder(Path::new(&path)).map_err(|e| anyhow!(e))?;
    if !real.is_dir() {
        bail!("Folder must be a folder.");
    }
    Ok(real.display().to_string())
}

pub fn read_input(v: &Value, folder: String) -> Result<ScheduleInput> {
    let name = needed(v, "name", "Name")?;
    let agent = needed(v, "agent", "Agent")?;
    let team = is_team(&agent);
    let prompt = if team {
        text_of(v, "prompt")
    } else {
        needed(v, "prompt", "Instructions")?
    };
    let repeat = Repeat::parse(&text_of(v, "repeat"))
        .ok_or_else(|| anyhow!("Repeat must be one of once, hourly, daily, weekdays, weekly."))?;
    let time = needed(v, "time", "Time")?;
    if !store::valid_time(&time) {
        bail!("Use 24-hour time like 09:30.");
    }
    let mut date = String::new();
    let mut days = Vec::new();
    if repeat == Repeat::Once {
        date = needed(v, "date", "Date")?;
        if !store::valid_date(&date) {
            bail!("Use a real date like 2026-11-09.");
        }
    }
    if repeat == Repeat::Weekly {
        days = v
            .get("days")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_i64)
                    .filter(|d| (0..=6).contains(d))
                    .collect()
            })
            .unwrap_or_default();
        days.sort();
        days.dedup();
        if days.is_empty() {
            bail!("Pick at least one day for a weekly schedule.");
        }
    }
    let follow_agent = if team {
        String::new()
    } else {
        text_of(v, "follow_agent")
    };
    let follow_prompt = if follow_agent.is_empty() {
        String::new()
    } else {
        text_of(v, "follow_prompt")
    };
    Ok(ScheduleInput {
        name,
        agent,
        folder,
        prompt,
        job: if team { read_job(v.get("job")) } else { None },
        repeat,
        time,
        date,
        days,
        enabled: v.get("enabled").and_then(Value::as_bool) == Some(true),
        follow_agent,
        follow_prompt,
        skip_unchanged: v.get("skip_unchanged").and_then(Value::as_bool) == Some(true),
    })
}

pub fn input_next(zone: &Zone, input: &ScheduleInput, at: i64) -> Option<i64> {
    if !input.enabled {
        return None;
    }
    when::next_run(
        zone,
        input.repeat,
        &input.time,
        &input.date,
        &input.days,
        at,
    )
}

fn save(app: &App, payload: &Value) -> Result<()> {
    let v = payload.get("schedule").cloned().unwrap_or(Value::Null);
    if !v.is_object() {
        bail!("Give the schedule to save.");
    }
    let prior = v
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|id| get(app, id))
        .transpose()?;
    let folder = folder_of(app, &needed(&v, "folder", "Folder")?)?;
    let mut input = read_input(&v, folder)?;
    if let Some(old) = &prior
        && is_team(&input.agent)
    {
        input.job = keep_lists(input.job.take(), old.job.as_ref(), v.get("job"));
    }
    let zone = zone_of(app);
    let at = now();
    let next = input_next(&zone, &input, at);
    if input.enabled && next.is_none() {
        bail!("That date and time has already passed.");
    }
    let c = conn(app)?;
    let offset = offset_minutes(&zone, at);
    let id = match prior {
        None => store::add_schedule(&c, &input, offset)?.id,
        Some(old) => {
            let patch = SchedulePatch {
                name: Some(input.name.clone()),
                agent: Some(input.agent.clone()),
                folder: Some(input.folder.clone()),
                prompt: Some(input.prompt.clone()),
                job: Some(input.job.clone()),
                repeat: Some(input.repeat),
                time: Some(input.time.clone()),
                date: Some(input.date.clone()),
                days: Some(input.days.clone()),
                enabled: Some(input.enabled),
                follow_agent: Some(input.follow_agent.clone()),
                follow_prompt: Some(input.follow_prompt.clone()),
                skip_unchanged: Some(input.skip_unchanged),
            };
            store::update_schedule(&c, &old.id, &patch, offset)?.id
        }
    };
    store::set_next_run(&c, &id, next)?;
    Ok(())
}

fn id_of(payload: &Value) -> Result<String> {
    payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Schedule not found."))
}

pub fn command(app: &Arc<App>, action: &str, payload: &Value) -> Result<()> {
    if let Some(done) = repeats::command(app, action, payload) {
        return done;
    }
    let zone = zone_of(app);
    let at = now();
    match action {
        "scheduler.set" => {
            let on = payload.get("enabled").and_then(Value::as_bool) == Some(true);
            let c = conn(app)?;
            nixa_store::settings::set_setting_bool(&c, ENABLED_KEY, on)?;
            for s in store::list_schedules(&c)? {
                let next = if s.enabled {
                    when::schedule_next(&zone, &s, at)
                } else {
                    None
                };
                store::set_next_run(&c, &s.id, next)?;
            }
        }
        "schedule.save" => save(app, payload)?,
        "schedule.toggle" => {
            let s = get(app, &id_of(payload)?)?;
            let on = payload.get("enabled").and_then(Value::as_bool) == Some(true);
            let next = if on {
                when::schedule_next(&zone, &s, at)
            } else {
                None
            };
            if on && next.is_none() {
                bail!("That date and time has already passed. Edit the schedule first.");
            }
            let c = conn(app)?;
            store::set_schedule_enabled(&c, &s.id, on, offset_minutes(&zone, at))?;
            store::set_next_run(&c, &s.id, next)?;
        }
        "schedule.skip" => {
            let s = get(app, &id_of(payload)?)?;
            let Some(due) = s.next_run else {
                bail!("Nothing to skip.")
            };
            let next = when::schedule_next(&zone, &s, due.max(at));
            let c = conn(app)?;
            if next.is_none() {
                store::set_schedule_enabled(&c, &s.id, false, offset_minutes(&zone, at))?;
            }
            store::set_next_run(&c, &s.id, next)?;
        }
        "schedule.delete" => {
            let id = id_of(payload)?;
            if !store::remove_schedule(&conn(app)?, &id)? {
                bail!("Schedule not found.");
            }
        }
        "schedule.run" => {
            let s = get(app, &id_of(payload)?)?;
            if !claim(app, &s.id) {
                bail!("{} is busy. Try again in a moment.", s.name);
            }
            let app = app.clone();
            tokio::spawn(async move {
                if let Err(error) = run_claimed(&app, &s.id, true).await {
                    notify(&app, &format!("{} did not start: {error:#}", s.name));
                }
                release(&app, &s.id);
                changed(&app);
            });
        }
        "schedule.work" => {
            let s = get(app, &id_of(payload)?)?;
            let task = Some(text_of(payload, "task")).filter(|t| !t.is_empty());
            let agent = Some(text_of(payload, "agent")).filter(|t| !t.is_empty());
            queue_report_work(app, s, task, agent)?;
        }
        other => bail!("Unknown action {other}."),
    }
    changed(app);
    Ok(())
}
