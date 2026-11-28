mod view;

#[cfg(test)]
mod tests;

use crate::links::brain::{BrainMsg, Link, NOT_CONNECTED};
use gpui::{App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Window};
use nixa_store::schedules::{Repeat, Schedule};
use nixa_ui::text_input::TextInput;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::mpsc::Receiver;
use std::time::Duration;

pub const PART: &str = "Schedules";
pub const KINDS: [&str; 2] = ["schedules", "schedule_event"];
pub const PROMPT_CLIP: usize = 220;
pub const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const REPEATS: [(Repeat, &str); 5] = [
    (Repeat::Once, "One time"),
    (Repeat::Hourly, "Hourly"),
    (Repeat::Daily, "Daily"),
    (Repeat::Weekdays, "Weekdays"),
    (Repeat::Weekly, "Weekly"),
];
pub const MEMBER_PREFIX: &str = "member:";
pub const TEAM_PREFIX: &str = "team:";
const POLL: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct View {
    pub when: String,
    pub next: String,
    pub last: String,
    pub failed: bool,
    pub agent_name: String,
    pub follow_name: String,
    pub report_done: bool,
    pub busy: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Item {
    #[serde(flatten)]
    pub schedule: Schedule,
    #[serde(default)]
    pub view: View,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub flow: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SpaceRow {
    pub id: String,
    pub name: String,
    pub folder: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub enabled: bool,
    pub schedules: Vec<Item>,
    pub agents: Vec<Choice>,
    pub members: Vec<Choice>,
    pub teams: Vec<Choice>,
    pub spaces: Vec<SpaceRow>,
    pub error: Option<String>,
    pub zone: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Repo {
    pub name: String,
    pub folder: String,
    pub branches: Vec<String>,
    pub current: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickState {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Picks {
    pub folder: String,
    pub state: PickState,
    pub repos: Vec<Repo>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobPick {
    pub space: String,
    pub repo: String,
    pub branch: String,
    pub skills: Vec<String>,
    pub mcps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub id: Option<String>,
    pub agent: Option<String>,
    pub follow_agent: Option<String>,
    pub repeat: Repeat,
    pub days: Vec<i64>,
    pub enabled: bool,
    pub skip_unchanged: bool,
    pub error: Option<String>,
    pub job: JobPick,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Confirm {
    pub title: String,
    pub detail: String,
    pub action: String,
    pub payload: Value,
    pub label: String,
    pub danger: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub ok: bool,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Values {
    pub name: String,
    pub folder: String,
    pub prompt: String,
    pub time: String,
    pub date: String,
    pub follow_prompt: String,
}

pub struct Inputs {
    pub name: Entity<TextInput>,
    pub folder: Entity<TextInput>,
    pub prompt: Entity<TextInput>,
    pub time: Entity<TextInput>,
    pub date: Entity<TextInput>,
    pub follow_prompt: Entity<TextInput>,
}

pub struct Schedules {
    pub snap: Option<Snapshot>,
    pub load_error: Option<String>,
    pub retry_at: Option<std::time::Instant>,
    pub form: Option<Form>,
    pub confirm: Option<Confirm>,
    pub notice: Option<Notice>,
    pub pending: Option<String>,
    pub picks: Option<Picks>,
    pub inputs: Inputs,
    pub link: Option<Link>,
    pub focus: FocusHandle,
    pub events: Vec<String>,
    rx: Option<Receiver<BrainMsg>>,
}

impl Focusable for Schedules {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Schedules {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        view::render(self, cx)
    }
}

pub fn is_team(agent: &str) -> bool {
    agent.starts_with(TEAM_PREFIX)
}

pub fn is_member(agent: &str) -> bool {
    agent.starts_with(MEMBER_PREFIX)
}

pub fn new(link: Option<Link>, cx: &mut App) -> Entity<Schedules> {
    let view = cx.new(|cx| build(link, cx));
    view.update(cx, |part, cx| {
        reload(part, cx);
        listen(cx);
    });
    view
}

pub fn build(link: Option<Link>, cx: &mut Context<Schedules>) -> Schedules {
    let input =
        |cx: &mut Context<Schedules>, hint: &'static str| cx.new(|cx| TextInput::new(cx, hint));
    let inputs = Inputs {
        name: input(cx, "Nightly review"),
        folder: input(cx, "/absolute/project/folder"),
        prompt: input(cx, "What the agent should do"),
        time: input(cx, "HH:mm"),
        date: input(cx, "YYYY-MM-DD"),
        follow_prompt: input(cx, "Extra rules for the follow-up agent (optional)"),
    };
    let rx = link.as_ref().map(|l| l.subscribe(&KINDS));
    Schedules {
        snap: None,
        load_error: None,
        retry_at: None,
        form: None,
        confirm: None,
        notice: None,
        pending: None,
        picks: None,
        inputs,
        link,
        focus: cx.focus_handle(),
        events: Vec::new(),
        rx,
    }
}

fn listen(cx: &mut Context<Schedules>) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL).await;
            let alive = this.update(cx, drain).is_ok();
            if !alive {
                break;
            }
        }
    })
    .detach();
}

pub fn drain(part: &mut Schedules, cx: &mut Context<Schedules>) {
    let now = std::time::Instant::now();
    if part.snap.is_none() && part.load_error.is_some() && part.retry_at.is_none_or(|at| now >= at) {
        part.retry_at = Some(now + Duration::from_secs(3));
        reload(part, cx);
    }
    let mut msgs = Vec::new();
    if let Some(rx) = &part.rx {
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }
    }
    for msg in msgs {
        match msg {
            BrainMsg::Up(_) => reload(part, cx),
            BrainMsg::Down(why) => {
                part.load_error = Some(why);
                cx.notify();
            }
            BrainMsg::Event(value) => feed(part, &value, cx),
        }
    }
}

pub fn feed(part: &mut Schedules, value: &Value, cx: &mut Context<Schedules>) {
    match value.get("type").and_then(Value::as_str) {
        Some("schedules") => {
            if let Ok(snap) = serde_json::from_value::<Snapshot>(value.clone()) {
                take(part, snap);
                cx.notify();
            }
        }
        Some("schedule_event") => {
            if let Some(text) = value
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                part.events.insert(0, text.to_string());
                part.events.truncate(5);
                cx.notify();
            }
        }
        _ => {}
    }
}

pub fn take(part: &mut Schedules, snap: Snapshot) {
    part.snap = Some(snap);
    part.load_error = None;
}

pub fn reload(part: &mut Schedules, cx: &mut Context<Schedules>) {
    let Some(link) = part.link.clone() else {
        part.load_error = Some(NOT_CONNECTED.into());
        cx.notify();
        return;
    };
    let task = cx
        .background_executor()
        .spawn(async move { link.get("/schedules") });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |part, cx| {
            match result.and_then(|v| serde_json::from_value::<Snapshot>(v).map_err(Into::into)) {
                Ok(snap) => take(part, snap),
                Err(error) => part.load_error = Some(format!("{error:#}")),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn send(
    part: &mut Schedules,
    action: &str,
    payload: Value,
    label: &str,
    cx: &mut Context<Schedules>,
) {
    let Some(link) = part.link.clone() else {
        part.notice = Some(Notice {
            ok: false,
            text: "Nixa is not connected. Nothing was sent.".into(),
        });
        cx.notify();
        return;
    };
    if part.pending.is_some() {
        return;
    }
    part.pending = Some(label.to_string());
    part.notice = None;
    cx.notify();
    let body = json!({"action": action, "payload": payload});
    let task = cx
        .background_executor()
        .spawn(async move { link.post("/schedules", &body) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |part, cx| {
            done(part, result.map_err(|e| format!("{e:#}")), cx);
        })
        .ok();
    })
    .detach();
}

pub fn done(part: &mut Schedules, result: Result<Value, String>, cx: &mut Context<Schedules>) {
    let label = part.pending.take().unwrap_or_else(|| "Change".into());
    match result {
        Ok(value) => {
            if let Ok(snap) = serde_json::from_value::<Snapshot>(value) {
                take(part, snap);
            }
            part.form = None;
            part.confirm = None;
            part.notice = Some(Notice {
                ok: true,
                text: format!("{label}: done."),
            });
        }
        Err(error) => {
            let text = if error.is_empty() {
                format!("{label} failed.")
            } else {
                error
            };
            part.notice = Some(Notice { ok: false, text });
        }
    }
    cx.notify();
}

pub fn values(part: &Schedules, cx: &App) -> Values {
    let v = |e: &Entity<TextInput>| e.read(cx).value().trim().to_string();
    Values {
        name: v(&part.inputs.name),
        folder: v(&part.inputs.folder),
        prompt: v(&part.inputs.prompt),
        time: v(&part.inputs.time),
        date: v(&part.inputs.date),
        follow_prompt: v(&part.inputs.follow_prompt),
    }
}

pub fn flow_of(snap: Option<&Snapshot>, agent: &str) -> String {
    snap.and_then(|s| s.teams.iter().find(|t| t.id == agent))
        .map(|t| t.flow.clone())
        .unwrap_or_default()
}

pub fn is_pr_team(snap: Option<&Snapshot>, agent: &str) -> bool {
    is_team(agent) && flow_of(snap, agent) == "pr"
}

pub fn ready_repos<'a>(picks: Option<&'a Picks>, folder: &str) -> &'a [Repo] {
    match picks {
        Some(p) if p.folder == folder && p.state == PickState::Ready => &p.repos,
        _ => &[],
    }
}

pub fn payload(form: &Form, v: &Values, pr_team: bool, repos: &[Repo]) -> Result<Value, String> {
    let agent = form.agent.clone().unwrap_or_default();
    let team = is_team(&agent);
    let prompt =
        if v.prompt.starts_with('#') && v.prompt[1..].starts_with(|c: char| c.is_ascii_digit()) {
            v.prompt[1..].to_string()
        } else {
            v.prompt.clone()
        };
    if v.name.is_empty()
        || agent.is_empty()
        || v.folder.is_empty()
        || v.time.is_empty()
        || (!team && prompt.is_empty())
    {
        let what = if team {
            "Name, agent, folder and time are needed."
        } else {
            "Name, agent, folder, instructions and time are needed."
        };
        return Err(what.into());
    }
    let mut job = serde_json::Map::new();
    if team {
        let fields = [
            ("space", &form.job.space),
            ("repo", &form.job.repo),
            ("branch", &form.job.branch),
        ]
        .into_iter();
        for (key, value) in fields {
            if !value.is_empty() {
                job.insert(key.to_string(), json!(value));
            }
        }
        for (key, list) in [("skills", &form.job.skills), ("mcps", &form.job.mcps)] {
            if !list.is_empty() {
                job.insert(key.to_string(), json!(list));
            }
        }
    }
    if team && repos.len() > 1 && form.job.repo.is_empty() {
        return Err("Pick the repo.".into());
    }
    if pr_team
        && !prompt.is_empty()
        && !(prompt.len() <= 6 && prompt.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(
            "For the PR team the instructions are one PR number, or empty for every open PR."
                .into(),
        );
    }
    let mut out = json!({
        "name": v.name,
        "agent": agent,
        "folder": v.folder,
        "prompt": prompt,
        "repeat": form.repeat.as_str(),
        "time": v.time,
        "enabled": form.enabled,
    });
    if let Some(id) = &form.id {
        out["id"] = json!(id);
    }
    if !job.is_empty() {
        out["job"] = json!(job);
    }
    if form.repeat == Repeat::Once {
        out["date"] = json!(v.date);
    }
    if form.repeat == Repeat::Weekly {
        let mut days = form.days.clone();
        days.sort();
        out["days"] = json!(days);
    }
    if let Some(follow) = form.follow_agent.as_ref().filter(|_| !team) {
        out["follow_agent"] = json!(follow);
        out["follow_prompt"] = json!(v.follow_prompt);
    }
    if form.skip_unchanged {
        out["skip_unchanged"] = json!(true);
    }
    Ok(out)
}

pub fn open_form(part: &mut Schedules, schedule: Option<&Schedule>, cx: &mut Context<Schedules>) {
    let set = |e: &Entity<TextInput>, text: String, cx: &mut Context<Schedules>| {
        e.update(cx, |i, cx| i.set_value(text, cx))
    };
    let s = schedule.cloned();
    set(
        &part.inputs.name,
        s.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
        cx,
    );
    set(
        &part.inputs.folder,
        s.as_ref().map(|s| s.folder.clone()).unwrap_or_default(),
        cx,
    );
    set(
        &part.inputs.prompt,
        s.as_ref().map(|s| s.prompt.clone()).unwrap_or_default(),
        cx,
    );
    set(
        &part.inputs.time,
        s.as_ref().map(|s| s.time.clone()).unwrap_or_default(),
        cx,
    );
    set(
        &part.inputs.date,
        s.as_ref().map(|s| s.date.clone()).unwrap_or_default(),
        cx,
    );
    set(
        &part.inputs.follow_prompt,
        s.as_ref()
            .map(|s| s.follow_prompt.clone())
            .unwrap_or_default(),
        cx,
    );
    let job = s.as_ref().and_then(|s| s.job.clone()).unwrap_or_default();
    part.form = Some(Form {
        id: s.as_ref().map(|s| s.id.clone()),
        agent: s
            .as_ref()
            .map(|s| s.agent.clone())
            .filter(|a| !a.is_empty()),
        follow_agent: s
            .as_ref()
            .map(|s| s.follow_agent.clone())
            .filter(|a| !a.is_empty()),
        repeat: s.as_ref().map(|s| s.repeat).unwrap_or(Repeat::Daily),
        days: s.as_ref().map(|s| s.days.clone()).unwrap_or_default(),
        enabled: s.as_ref().map(|s| s.enabled).unwrap_or(true),
        skip_unchanged: s.as_ref().is_some_and(|s| s.skip_unchanged),
        error: None,
        job: JobPick {
            space: job.space,
            repo: job.repo,
            branch: job.branch,
            skills: job.skills,
            mcps: job.mcps,
        },
    });
    part.confirm = None;
    if let Some(s) = &s
        && is_team(&s.agent)
        && !s.folder.is_empty()
    {
        request_picks(part, &s.folder.clone(), cx);
    }
    cx.notify();
}

pub fn save_form(part: &mut Schedules, cx: &mut Context<Schedules>) {
    let Some(form) = part.form.clone() else {
        return;
    };
    let v = values(part, cx);
    let agent = form.agent.clone().unwrap_or_default();
    let pr = is_pr_team(part.snap.as_ref(), &agent);
    let repos = ready_repos(part.picks.as_ref(), &v.folder).to_vec();
    match payload(&form, &v, pr, &repos) {
        Ok(schedule) => {
            if let Some(f) = part.form.as_mut() {
                f.error = None;
            }
            let label = if form.id.is_some() {
                "Save schedule"
            } else {
                "Add schedule"
            };
            send(
                part,
                "schedule.save",
                json!({"schedule": schedule}),
                label,
                cx,
            );
        }
        Err(error) => {
            if let Some(f) = part.form.as_mut() {
                f.error = Some(error);
            }
            cx.notify();
        }
    }
}

pub fn settle_job(form: &mut Form, picks: &Picks, folder: &str) {
    if picks.state != PickState::Ready || picks.folder != folder {
        return;
    }
    let found = picks
        .repos
        .iter()
        .find(|r| r.folder == form.job.repo)
        .or_else(|| {
            if picks.repos.len() == 1 {
                picks.repos.first()
            } else {
                picks.repos.iter().find(|r| r.folder == picks.folder)
            }
        })
        .cloned();
    match found {
        Some(repo) => {
            if form.job.branch.is_empty() || !repo.branches.contains(&form.job.branch) {
                form.job.branch = repo.current.clone();
            }
            form.job.repo = repo.folder;
        }
        None => {
            form.job.repo.clear();
            form.job.branch.clear();
        }
    }
}

pub fn request_picks(part: &mut Schedules, folder: &str, cx: &mut Context<Schedules>) {
    if folder.is_empty() {
        part.picks = None;
        return;
    }
    if let Some(p) = &part.picks
        && p.folder == folder
        && !matches!(p.state, PickState::Failed(_))
    {
        let p = p.clone();
        if let Some(form) = part.form.as_mut() {
            settle_job(form, &p, folder);
        }
        return;
    }
    let Some(link) = part.link.clone() else {
        part.picks = Some(Picks {
            folder: folder.to_string(),
            state: PickState::Failed("Nixa is not connected.".into()),
            repos: Vec::new(),
        });
        return;
    };
    part.picks = Some(Picks {
        folder: folder.to_string(),
        state: PickState::Loading,
        repos: Vec::new(),
    });
    let wanted = folder.to_string();
    let body = json!({"folder": wanted});
    let task = cx
        .background_executor()
        .spawn(async move { link.post("/schedules/picks", &body) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |part, cx| {
            take_picks(part, &wanted, result.map_err(|e| format!("{e:#}")));
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn take_picks(part: &mut Schedules, folder: &str, result: Result<Value, String>) {
    if part.picks.as_ref().map(|p| p.folder.as_str()) != Some(folder) {
        return;
    }
    let picks = match result {
        Ok(v) => Picks {
            folder: folder.to_string(),
            state: PickState::Ready,
            repos: serde_json::from_value(v.get("repos").cloned().unwrap_or_default())
                .unwrap_or_default(),
        },
        Err(error) => Picks {
            folder: folder.to_string(),
            state: PickState::Failed(error),
            repos: Vec::new(),
        },
    };
    if let Some(form) = part.form.as_mut() {
        settle_job(form, &picks, folder);
    }
    part.picks = Some(picks);
}

pub fn pick_agent(part: &mut Schedules, id: &str, cx: &mut Context<Schedules>) {
    let Some(form) = part.form.as_mut() else {
        return;
    };
    form.agent = Some(id.to_string());
    if is_team(id) {
        form.follow_agent = None;
        let folder = part.inputs.folder.read(cx).value().trim().to_string();
        request_picks(part, &folder, cx);
    }
    cx.notify();
}

pub fn pick_folder(part: &mut Schedules, space: &SpaceRow, cx: &mut Context<Schedules>) {
    part.inputs
        .folder
        .update(cx, |i, cx| i.set_value(space.folder.clone(), cx));
    let team = part
        .form
        .as_ref()
        .and_then(|f| f.agent.clone())
        .is_some_and(|a| is_team(&a));
    if team {
        if let Some(form) = part.form.as_mut() {
            form.job.space = space.id.clone();
            form.job.repo.clear();
            form.job.branch.clear();
        }
        request_picks(part, &space.folder, cx);
    }
    cx.notify();
}

pub fn ask(
    part: &mut Schedules,
    item: &Item,
    action: &str,
    title: String,
    label: &str,
    danger: bool,
) {
    let s = &item.schedule;
    let what = s.prompt.clone();
    part.confirm = Some(Confirm {
        title,
        detail: format!("{} in {}\n{what}", item.view.agent_name, s.folder),
        action: action.to_string(),
        payload: json!({"id": s.id}),
        label: label.to_string(),
        danger,
    });
}

pub fn ask_work(part: &mut Schedules, item: &Item) {
    let s = &item.schedule;
    let who = if item.view.follow_name.is_empty() {
        item.view.agent_name.clone()
    } else {
        item.view.follow_name.clone()
    };
    let report = s
        .last_run
        .as_ref()
        .map(|r| r.report.clone())
        .unwrap_or_default();
    part.confirm = Some(Confirm {
        title: format!("Work on the tasks of {}?", s.name),
        detail: format!("{who} reads {report} and works on its open tasks."),
        action: "schedule.work".into(),
        payload: json!({"id": s.id}),
        label: "Start".into(),
        danger: false,
    });
}

pub fn known(snap: &Snapshot, id: &str) -> bool {
    snap.members
        .iter()
        .chain(snap.agents.iter())
        .chain(snap.teams.iter())
        .any(|c| c.id == id)
}

pub fn extras(snap: &Snapshot, form: &Form) -> Vec<Choice> {
    let mut out: Vec<Choice> = Vec::new();
    for id in [form.agent.as_deref(), form.follow_agent.as_deref()]
        .into_iter()
        .flatten()
    {
        if id.is_empty() || known(snap, id) || out.iter().any(|c| c.id == id) {
            continue;
        }
        let name = match (id.strip_prefix(MEMBER_PREFIX), id.strip_prefix(TEAM_PREFIX)) {
            (Some(m), _) => format!("{m} (team member not found)"),
            (_, Some(t)) => format!("{t} (team not found)"),
            _ => format!("{id} (not found)"),
        };
        out.push(Choice {
            id: id.to_string(),
            name,
            flow: String::new(),
        });
    }
    out
}

pub fn folders(snap: &Snapshot) -> Vec<SpaceRow> {
    let mut out: Vec<SpaceRow> = Vec::new();
    for s in &snap.spaces {
        if !s.folder.is_empty() && !out.iter().any(|o| o.folder == s.folder) {
            out.push(s.clone());
        }
    }
    out
}

pub fn job_line(s: &Schedule) -> String {
    let Some(job) = &s.job else {
        return String::new();
    };
    let mut parts = Vec::new();
    if !job.repo.is_empty() {
        parts.push(format!(
            "Repo: {}",
            job.repo.rsplit('/').next().unwrap_or(&job.repo)
        ));
    }
    if !job.branch.is_empty() {
        parts.push(format!("Branch: {}", job.branch));
    }
    parts.join(" · ")
}

pub fn summary(item: &Item) -> String {
    let s = &item.schedule;
    let follow = if item.view.follow_name.is_empty() {
        String::new()
    } else {
        format!(" · then {} on the tasks", item.view.follow_name)
    };
    format!(
        "{} · {} · {}{follow}",
        item.view.agent_name, s.folder, item.view.when
    )
}

pub fn clip(text: &str) -> String {
    if text.chars().count() > PROMPT_CLIP {
        format!("{}…", text.chars().take(PROMPT_CLIP).collect::<String>())
    } else {
        text.to_string()
    }
}
