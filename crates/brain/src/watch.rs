use crate::agent;
use crate::app::{App, Seen, lock, status_word};
use crate::deep::{Action, TickInput};
use crate::engine;
use crate::spaces::Space;
use crate::trees::WATCH_TREE;
use crate::util::{clip_flat, day_of, log, now_ms};
use crate::walk::{Facts, Subject, walk_tree};
use nixa_protocol::{CreateSpec, Event, ProcInfo, Reply, Request, Status, TermInfo};
use serde_json::json;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub const DEEP_TICK_MS: u64 = 1500;
pub const SLOW_TICK_MS: u64 = 30_000;
pub const TOLD_MS: u64 = 60_000;
pub const BLOCKED_WAIT_MS: u64 = 4_000;

fn minutes(since: u64, now: u64) -> u64 {
    now.saturating_sub(since) / 60_000
}

fn agent_of(info: &TermInfo) -> String {
    info.cmd.first().map(|c| c.rsplit('/').next().unwrap_or(c).to_string()).unwrap_or_else(|| "shell".into())
}

pub fn subjects(app: &App) -> Vec<Subject> {
    let now = now_ms();
    let spaces = app.spaces();
    let watch = lock(&app.watch);
    let deep = lock(&app.deep);
    let mut list: Vec<&Seen> = watch.world.values().filter(|s| !deep.owns(s.info.id.as_str())).collect();
    list.sort_by(|a, b| a.info.id.cmp(&b.info.id));
    list.iter()
        .map(|seen| {
            let t = &seen.info;
            let job = watch.jobs.get(t.id.as_str()).copied();
            let space = t.labels.get(crate::deep::SPACE_LABEL).cloned().or_else(|| crate::spaces::for_folder(&spaces, &t.cwd).map(|s| s.name.clone())).unwrap_or_default();
            let mut facts = Facts::new();
            facts.insert("subject".into(), json!("terminal"));
            facts.insert("terminal.id".into(), json!(t.id.as_str()));
            facts.insert("terminal.name".into(), json!(t.name));
            facts.insert("terminal.agent".into(), json!(agent_of(t)));
            facts.insert("terminal.space".into(), json!(space));
            facts.insert("terminal.status".into(), json!(t.status.map(|s| s.as_str()).unwrap_or("")));
            facts.insert("terminal.note".into(), json!(t.status_note.clone().unwrap_or_default()));
            facts.insert("terminal.minutesInStatus".into(), json!(minutes(seen.since, now)));
            facts.insert("terminal.running".into(), json!(t.running));
            facts.insert("terminal.exitCode".into(), t.exit_code.map(|c| json!(c)).unwrap_or(serde_json::Value::Null));
            facts.insert("terminal.isChild".into(), json!(t.parent.is_some()));
            facts.insert("terminal.children".into(), json!(t.children.len()));
            facts.insert("terminal.nixaJob".into(), json!(job.is_some()));
            facts.insert("terminal.minutesSinceSent".into(), job.map(|j| json!(minutes(j, now))).unwrap_or(serde_json::Value::Null));
            facts.insert("terminal.draft".into(), json!(t.draft));
            facts.insert("terminal.keep".into(), json!(t.keep));
            Subject { key: format!("terminal {}", t.id), label: format!("terminal {}", t.id), facts }
        })
        .collect()
}

pub struct SpaceFacts<'a> {
    pub work: String,
    pub folder: &'a str,
    pub tasks: Vec<nixa_store::tasks::Task>,
    pub holders: Vec<nixa_store::claims::FileHolder>,
    pub terms: Vec<&'a TermInfo>,
    pub procs: &'a HashMap<String, Vec<ProcInfo>>,
    pub questions: usize,
}

pub fn procs_note(procs: Option<&Vec<ProcInfo>>) -> String {
    let Some(procs) = procs.filter(|p| !p.is_empty()) else { return String::new() };
    let agents: Vec<String> = procs.iter().filter(|p| p.agent).map(|p| format!("{} pid {}", p.name, p.pid)).collect();
    let others = procs.len() - agents.len();
    let mut parts = Vec::new();
    if !agents.is_empty() {
        parts.push(format!("agents: {}", agents.join(", ")));
    }
    if others > 0 {
        parts.push(format!("{others} other processes"));
    }
    format!(" runs {}", parts.join("; "))
}

const HELD_FILES_MAX: usize = 8;
const TERMS_MAX: usize = 20;

pub fn facts_lines(f: SpaceFacts) -> String {
    let mut lines = vec![f.work, format!("Folder: {}", f.folder)];
    let tasks: Vec<String> = f
        .tasks
        .iter()
        .map(|t| {
            let at = if t.terminal_id.is_empty() { String::new() } else { format!(" in {}", t.terminal_id) };
            format!("- #{} {} ({}){at}", t.id, clip_flat(&t.title, 80), t.status.as_str())
        })
        .collect();
    lines.push(if tasks.is_empty() { "Tasks held now: none".into() } else { format!("Tasks held now:\n{}", tasks.join("\n")) });
    let files: Vec<String> = f
        .holders
        .iter()
        .take(HELD_FILES_MAX)
        .map(|h| format!("- #{} in {}: {}", h.task, h.terminal, clip_flat(&h.files.join(", "), 120)))
        .collect();
    lines.push(if files.is_empty() { "Files held now: none".into() } else { format!("Files held now:\n{}", files.join("\n")) });
    let terms: Vec<String> = f
        .terms
        .iter()
        .take(TERMS_MAX)
        .map(|t| format!("- {} {} ({}{}){}", t.id, clip_flat(&t.name, 40), status_word(t.status), if t.running { "" } else { ", ended" }, procs_note(f.procs.get(t.id.as_str()))))
        .collect();
    lines.push(if terms.is_empty() { "Terminals: none".into() } else { format!("Terminals:\n{}", terms.join("\n")) });
    lines.push(format!("Open agent questions: {}.", f.questions));
    lines.join("\n")
}

fn facts_text(app: &App, live: &HashMap<String, TermInfo>, procs: &HashMap<String, Vec<ProcInfo>>, space: &Space) -> String {
    let mut terms: Vec<&TermInfo> = live
        .values()
        .filter(|t| t.labels.get(crate::deep::SPACE_LABEL).map(|s| s == &space.id).unwrap_or_else(|| crate::spaces::folder_in(&t.cwd, &space.folder)))
        .filter(|t| t.labels.get(crate::deep::ROLE_LABEL).map(String::as_str) != Some(crate::deep::ROLE_VALUE))
        .collect();
    terms.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: HashSet<String> = terms.iter().map(|t| t.id.to_string()).collect();
    let conn = nixa_store::open(&app.db_file).ok();
    let filter = nixa_store::tasks::TaskFilter {
        archived: nixa_store::tasks::Archived::Hide,
        space: Some(space.id.clone()),
        status: vec![nixa_store::tasks::TaskStatus::Doing, nixa_store::tasks::TaskStatus::Blocked, nixa_store::tasks::TaskStatus::Review],
        ..Default::default()
    };
    let tasks = conn.as_ref().and_then(|c| nixa_store::tasks::list_tasks(c, &filter).ok()).unwrap_or_default();
    let task_ids: HashSet<i64> = tasks.iter().map(|t| t.id).collect();
    let holders = conn
        .as_ref()
        .and_then(|c| nixa_store::claims::file_holders(c, now_ms() as i64).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|h| task_ids.contains(&h.task) || ids.contains(&h.terminal))
        .collect();
    let questions = lock(&app.prompts.book).items.iter().filter(|p| ids.contains(&p.terminal)).count();
    facts_lines(SpaceFacts { work: crate::modes::world_line(app), folder: &space.folder, tasks, holders, terms, procs, questions })
}

pub async fn deep_tick(app: &Arc<App>) {
    crate::followup::flow::send_parked(app);
    crate::decide::asks::scan(app);
    {
        let deep = lock(&app.deep);
        if deep.seats.iter().all(|s| !s.on) && deep.open_asks().is_empty() {
            return;
        }
    }
    let Ok(terms) = engine::list().await else { return };
    let live: HashMap<String, TermInfo> = terms.into_iter().map(|t| (t.id.to_string(), t)).collect();
    let procs: HashMap<String, Vec<ProcInfo>> = match engine::quick(Request::Procs).await {
        Ok(Reply::Procs { terms }) => terms.into_iter().map(|t| (t.id.to_string(), t.procs)).collect(),
        _ => HashMap::new(),
    };
    let spaces = match crate::deep_service::open_spaces(app) {
        Ok(spaces) => spaces,
        Err(error) => {
            log("deep", &format!("could not read open spaces: {error}"));
            return;
        }
    };
    let home = crate::spaces::home(&spaces).cloned();
    let can_start = crate::deep_service::can_auto_start(app);
    let (actions, retried) = {
        let mut deep = lock(&app.deep);
        if can_start {
            deep.seed_spaces(&spaces);
        }
        if deep.seats.iter().all(|s| !s.on) && deep.open_asks().is_empty() {
            return;
        }
        let now = now_ms();
        let retried = deep.retry_gave_up(&|_: &str| true, now);
        let role = crate::deep::role_text(&deep.rules_file, &deep.memory_file);
        let facts = |space: &Space| facts_text(app, &live, &procs, space);
        let input = TickInput { live: &live, spaces: &spaces, home: home.as_ref(), can_start, facts: &facts, role: &role, now };
        (deep.plan_tick(&input), retried)
    };
    for line in &retried {
        log("deep", line);
        crate::modes::note(app, "auto-yes", line, None, Some("Nixa tries a deep brain seat again 30 minutes after it gave up"));
    }
    run_actions(app, actions).await;
}

fn send_error_line(error: &str) -> String {
    let line = error.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("unknown error");
    clip_flat(line, 200)
}

pub async fn run_actions(app: &Arc<App>, actions: Vec<Action>) {
    let mut queue: VecDeque<Action> = actions.into();
    while let Some(action) = queue.pop_front() {
        match action {
            Action::Send { terminal, text, ask, facts } => {
                let space = lock(&app.deep).by_terminal(&terminal).map(|s| s.space.clone());
                if !space.is_some_and(|space| crate::deep_service::resolve(app, Some(&space), None).is_ok()) {
                    lock(&app.deep).sent(&terminal, ask.as_deref(), facts, false);
                    continue;
                }
                let result = match engine::term_id(&terminal) {
                    Ok(id) => engine::send(&id, &text, true, false).await.map(|_| ()).map_err(|e| e.to_string()),
                    Err(e) => Err(format!("bad terminal id: {e}")),
                };
                match result {
                    Ok(()) => {
                        if ask.is_some() {
                            crate::universe::screen_flow(app, crate::universe::NIXA, crate::universe::DEEP, "deep", None);
                        }
                        lock(&app.deep).sent(&terminal, ask.as_deref(), facts, true);
                    }
                    Err(error) => {
                        let error = send_error_line(&error);
                        log("deep", &format!("send to {terminal} failed: {error}"));
                        let more = lock(&app.deep).send_failed(&terminal, ask.as_deref(), &error, now_ms());
                        queue.extend(more);
                    }
                }
            }
            Action::Start { seat, argv, folder, space, old } => {
                if !crate::deep_service::may_start(app) || crate::deep_service::resolve(app, Some(&space), None).is_err() {
                    lock(&app.deep).started(seat, None);
                    continue;
                }
                if let Some(old) = old.and_then(|o| engine::term_id(&o).ok()) {
                    let _ = engine::remove(&old).await;
                }
                let mut spec = CreateSpec { cmd: argv, cwd: Some(folder), name: Some("Deep brain".into()), ..CreateSpec::default() };
                spec.labels.insert(crate::deep::ROLE_LABEL.into(), crate::deep::ROLE_VALUE.into());
                spec.labels.insert(crate::deep::SPACE_LABEL.into(), space);
                let made = engine::create(spec).await.map(|t| t.id.to_string());
                if let Err(error) = &made {
                    log("deep", &format!("could not start a deep brain: {error}"));
                }
                let more = lock(&app.deep).started(seat, made.ok());
                queue.extend(more);
            }
            Action::Stop { terminal } => {
                if let Ok(id) = engine::term_id(&terminal) {
                    let _ = engine::remove(&id).await;
                }
            }
            Action::Notify { line, loud } => {
                app.info(&line);
                app.send_out(json!({"type": "notice", "text": line, "loud": loud}));
            }
            Action::Answered(ask) => {
                let said = ask.pick.as_deref().map(|p| match ask.why.as_deref() { Some(why) => format!("I pick {p}. {why}"), None => format!("I pick {p}.") });
                crate::universe::screen_flow(app, crate::universe::DEEP, crate::universe::NIXA, "deep", said.as_deref());
                if let Some(learn) = ask.learn.as_deref().and_then(crate::deep::learn_of) {
                    let file = lock(&app.deep).memory_file.clone();
                    let _ = crate::deep::append_memory(&file, &format!("{}: {}", learn.0, learn.1), &day_of(now_ms()));
                }
                app.send_out(json!({"type": "deep", "ask": ask}));
                if ask.state == crate::deep::AskState::Answered {
                    crate::follow_watch::note_answer(&ask, now_ms());
                }
                if crate::prompts::auto::answered(app, &ask).await {
                    continue;
                }
                let spaces = app.spaces();
                let live: Vec<String> = app.terminals().into_iter().filter(|s| s.info.running).map(|s| s.info.id.to_string()).collect();
                let placed = nixa_store::open(&app.db_file).map(|conn| crate::deep::apply_answer(&conn, &ask, &spaces, &live)).unwrap_or_default();
                let line = crate::deep::placed_line(&ask, &placed);
                if let Some(line) = &line {
                    app.info(line);
                    app.send_out(json!({"type": "tasks_changed", "why": "deep brain placement"}));
                }
                match crate::workflows::deep_answered(app, &ask).await {
                    Ok(Some(_)) => continue,
                    Ok(None) => {}
                    Err(error) => log("workflow", &format!("the deep brain answer {} was not used: {error}", ask.id)),
                }
                if crate::followup::flow::answered(app, &ask, line.as_deref()).await {
                    continue;
                }
                if ask.kind.as_deref() == Some(crate::extras::model_pick::ASK_KIND) {
                    continue;
                }
                if ask.kind.as_deref() == Some(crate::deep::WORK_KIND) {
                    app.add_chat(crate::chat::Who::Info, &crate::deep::work_line(&ask), crate::chat::Extra::default());
                }
                crate::voice::speech::mark_deep_reply(app);
                let names: Vec<String> = Some(ask.space_name.clone()).filter(|s| !s.trim().is_empty()).into_iter().collect();
                tell_event(app, &with_space_line(&crate::deep::answer_note(&ask, &placed), &names));
            }
        }
    }
}

fn done_event(key: &str, info: &nixa_protocol::TermInfo, note: Option<&str>, at: u64) -> crate::review_flow::done::DoneEvent {
    crate::review_flow::done::DoneEvent {
        terminal: key.to_string(),
        agent: info.name.clone(),
        cwd: info.cwd.clone(),
        text: note.unwrap_or_default().to_string(),
        screen: String::new(),
        source: crate::review_flow::done::Source::Watch,
        at,
        fallback: None,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

fn has_token(text: &str, token: &str) -> bool {
    !token.is_empty()
        && text.match_indices(token).any(|(at, _)| {
            text[..at].chars().next_back().is_none_or(|c| !is_word_char(c)) && text[at + token.len()..].chars().next().is_none_or(|c| !is_word_char(c))
        })
}

fn task_numbers(text: &str) -> Vec<i64> {
    let low = text.to_lowercase();
    let mut out = Vec::new();
    for (at, _) in low.match_indices("task #") {
        let digits: String = low[at + 6..].chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse::<i64>()
            && !out.contains(&n)
        {
            out.push(n);
        }
    }
    out
}

pub fn event_spaces(app: &App, text: &str) -> Vec<String> {
    let spaces = app.spaces();
    let mut names: Vec<String> = Vec::new();
    let mut add = |name: String| {
        if !name.trim().is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    };
    let infos: Vec<TermInfo> = lock(&app.watch).world.iter().filter(|(id, _)| has_token(text, id)).map(|(_, seen)| seen.info.clone()).collect();
    for info in infos {
        let found = info.labels.get(crate::deep::SPACE_LABEL).and_then(|id| crate::spaces::find(&spaces, id)).or_else(|| crate::spaces::for_folder(&spaces, &info.cwd));
        if let Some(space) = found {
            add(space.name.clone());
        }
    }
    let numbers = task_numbers(text);
    if !numbers.is_empty()
        && let Ok(conn) = nixa_store::open(&app.db_file)
    {
        for n in numbers {
            if let Ok(Some(task)) = nixa_store::tasks::get_task(&conn, n) {
                add(crate::spaces::name_of(&spaces, &task.space_id).unwrap_or(task.space_name));
            }
        }
    }
    names
}

pub fn with_space_line(text: &str, names: &[String]) -> String {
    if names.is_empty() || text.lines().any(|l| l.starts_with(SPACE_LINE)) {
        return text.to_string();
    }
    format!("{text}\n{SPACE_LINE}{}. Name the space when you tell the user about it.", names.join(", "))
}

pub fn event_text(app: &App, text: &str) -> String {
    with_space_line(text, &event_spaces(app, text))
}

pub const SPACE_LINE: &str = "Space: ";

pub fn tell_event(app: &Arc<App>, text: &str) {
    if crate::followup::flow::relay(app, text) {
        return;
    }
    if !agent::has_brain(app) {
        let names = event_spaces(app, text);
        if names.is_empty() {
            app.info(text);
        } else {
            app.info_in(&names.join(", "), text);
        }
        return;
    }
    agent::event(app, &event_text(app, text));
}

fn told_recently(app: &App, key: &str, now: u64) -> bool {
    let mut watch = lock(&app.watch);
    watch.told.retain(|_, at| now.saturating_sub(*at) < TOLD_MS);
    if watch.told.contains_key(key) {
        return true;
    }
    watch.told.insert(key.to_string(), now);
    false
}

pub const ASKS_HINT: &str = "Read its screen, decide it yourself or with nixa_deep_ask, then answer with nixa_term_send or nixa_term_keys.";

pub const RECOVER_HINT: &str = "Do not wait for the user. Recover it yourself now: read its screen with nixa_term_read, then send one fix with nixa_term_send, start it again, give the work to another agent, ask nixa_deep_ask, or close the job if it is finished. Record what you chose in one short line. Tell the user only as a short side note.";

async fn on_event(app: &Arc<App>, kids: &mut crate::kid_watch::Kids, event: Event) {
    let now = now_ms();
    match event {
        Event::Created { term } => {
            lock(&app.watch).world.insert(term.id.to_string(), Seen { info: term, since: now });
            crate::persona::fill(app).await;
        }
        Event::ParentCleared { id, parent } => {
            let mut watch = lock(&app.watch);
            if let Some(child) = watch.world.get_mut(id.as_str()) {
                child.info.parent = None;
            }
            if let Some(old) = watch.world.get_mut(parent.as_str()) {
                old.info.children.retain(|child| child != &id);
            }
        }
        Event::Status { id, status, note } => {
            let key = id.to_string();
            let (before, info) = {
                let mut watch = lock(&app.watch);
                let Some(seen) = watch.world.get_mut(&key) else { return };
                let before = seen.info.status;
                if before != status {
                    seen.since = now;
                }
                seen.info.status = status;
                seen.info.status_note = note.clone();
                (before, seen.info.clone())
            };
            let parent = info.parent.as_ref().map(ToString::to_string);
            if let Some(why) = crate::kid_watch::on_status(kids, &key, parent.as_deref(), status, now) {
                crate::kid_watch::alert(kids, &key, why).await;
            }
            if lock(&app.deep).owns(&key) {
                if before == Some(Status::Working) && matches!(status, Some(Status::Done) | Some(Status::Idle)) {
                    let actions = lock(&app.deep).finished(&key, Some(&info), now);
                    run_actions(app, actions).await;
                }
                return;
            }
            if before == status {
                return;
            }
            if matches!(status, Some(Status::Idle) | Some(Status::Done)) {
                let app = app.clone();
                let terminal = key.clone();
                tokio::spawn(async move { crate::queue::notes::deliver(&app, &terminal).await });
            }
            let name = clip_flat(&info.name, 40);
            let note = note.map(|n| clip_flat(&n, 300)).filter(|n| !n.is_empty());
            let said = note.as_ref().map(|n| format!(": {}", crate::agent::data_block(n))).unwrap_or_default();
            let text = match status {
                Some(Status::Blocked) if note.as_deref() == Some(crate::kid_watch::MARK_NOTE) => format!("Event: Terminal {id} ({name}) stopped without a result, and its parent terminal was told to check it. This is not a question. Read it with nixa_term_read only if the parent does not act."),
                Some(Status::Blocked) => format!("Event: Terminal {id} ({name}) is blocked{said}. It may ask a question or a permission. {ASKS_HINT}"),
                Some(Status::Error) => format!("Event: Terminal {id} ({name}) reported an error{said}. {RECOVER_HINT}"),
                Some(Status::Done) => format!("Event: Done: terminal {id} ({name}) finished{said}. Check its work with nixa_term_read and tell the user in one short sentence."),
                _ => return,
            };
            if status == Some(Status::Done) {
                crate::universe::screen_flow(app, &key, crate::universe::NIXA, "reply", note.as_deref());
            }
            if status == Some(Status::Blocked) {
                let app = app.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(BLOCKED_WAIT_MS)).await;
                    let still = lock(&app.watch).world.get(&key).is_some_and(|s| s.info.status == Some(Status::Blocked));
                    let asked = !lock(&app.prompts.book).by_terminal(&key).is_empty();
                    if still && !asked && !told_recently(&app, &format!("{key}|blocked"), now_ms()) {
                        tell_event(&app, &text);
                    }
                });
                return;
            }
            if status == Some(Status::Done) && !lock(&app.deep).owns(&key) && crate::review_flow::on_done(app, done_event(&key, &info, note.as_deref(), now)) {
                return;
            }
            if !told_recently(app, &format!("{id}|{}", status_word(status)), now) {
                tell_event(app, &text);
            }
        }
        Event::Exit { id, code } => {
            let key = id.to_string();
            crate::followup::standalone::forget(app, &key);
            crate::kid_watch::gone(kids, &key);
            let (job, name) = {
                let mut watch = lock(&app.watch);
                let job = watch.jobs.remove(&key).is_some();
                let name = match watch.world.get_mut(&key) {
                    Some(seen) => {
                        seen.info.running = false;
                        seen.info.exit_code = code;
                        seen.since = now;
                        clip_flat(&seen.info.name, 40)
                    }
                    None => String::new(),
                };
                (job, name)
            };
            if job && !lock(&app.deep).owns(&key) {
                let code = code.map(|c| c.to_string()).unwrap_or_else(|| "none".into());
                tell_event(app, &format!("Event: The program in terminal {id} ({name}) ended with code {code}. {RECOVER_HINT}"));
            }
        }
        Event::Respawned { id, pid } => {
            if let Some(seen) = lock(&app.watch).world.get_mut(id.as_str()) {
                seen.info.running = true;
                seen.info.pid = pid;
                seen.info.status = None;
                seen.since = now;
            }
        }
        Event::Removed { id } => {
            crate::followup::standalone::forget(app, id.as_str());
            crate::kid_watch::gone(kids, id.as_str());
            let mut watch = lock(&app.watch);
            watch.world.remove(id.as_str());
            watch.jobs.remove(id.as_str());
        }
        Event::Label { id, labels } => {
            if let Some(seen) = lock(&app.watch).world.get_mut(id.as_str()) {
                seen.info.labels = labels;
            }
        }
        Event::Renamed { id, name } => {
            if let Some(seen) = lock(&app.watch).world.get_mut(id.as_str()) {
                seen.info.name = name;
            }
        }
        Event::Title { id, title } => {
            if let Some(seen) = lock(&app.watch).world.get_mut(id.as_str()) {
                seen.info.title = title;
            }
        }
        Event::Draft { id, active } => {
            if let Some(seen) = lock(&app.watch).world.get_mut(id.as_str()) {
                seen.info.draft = active;
            }
        }
        _ => {}
    }
}

pub async fn refresh_world(app: &App) {
    let Ok(terms) = engine::list().await else { return };
    let now = now_ms();
    let mut watch = lock(&app.watch);
    let old = std::mem::take(&mut watch.world);
    for term in terms {
        let key = term.id.to_string();
        let since = old.get(&key).filter(|s| s.info.status == term.status).map(|s| s.since).unwrap_or(now);
        watch.world.insert(key, Seen { info: term, since });
    }
    let alive: Vec<String> = watch.world.keys().cloned().collect();
    watch.jobs.retain(|k, _| alive.contains(k));
}

pub async fn watch_tick(app: &Arc<App>) {
    let every = app.watch_minutes();
    let now = now_ms();
    if every == 0 || now.saturating_sub(lock(&app.watch).last_at) < every * 60_000 {
        return;
    }
    if agent::busy(app) || !agent::has_brain(app) {
        return;
    }
    let tree = match lock(&app.trees).read(WATCH_TREE) {
        Ok(tree) => tree,
        Err(error) => {
            log("watch", &format!("tree not readable: {error}"));
            lock(&app.watch).last_at = now;
            return;
        }
    };
    lock(&app.watch).last_at = now;
    let found = walk_tree(&tree, &subjects(app));
    let fresh = lock(&app.watch).memory.pick(&found, now);
    if fresh.is_empty() {
        return;
    }
    let lines: Vec<String> = fresh.iter().map(|f| format!("- {}", f.text)).collect();
    let lead = if fresh.iter().any(|f| f.retry) {
        "Retry: these findings are still open after your last watch event, and you have not acted on them yet. Act now: call a tool for each one. Do not only speak.".to_string()
    } else {
        format!(
            "Event: Watch found {} thing{} that need action (rule tree {WATCH_TREE}, \"{}\"). For each one, act: read the terminal, message the agent, or ask the deep brain. Then tell the user one short line per finding.",
            fresh.len(),
            if fresh.len() == 1 { "" } else { "s" },
            tree.title
        )
    };
    tell_event(app, &format!("{lead}\n{}", lines.join("\n")));
}

fn sync_trees(app: &App) {
    let report = lock(&app.trees).sync();
    for name in report.changed.iter().chain(report.removed.iter()) {
        let note = crate::trees::change_note(&mut lock(&app.trees), name, "on disk", crate::trees::NOTE_MAX);
        let mut brain = lock(&app.brain);
        brain.carry = crate::trees::carry_note(&brain.carry, name, &note);
    }
    for (name, problem) in report.problems {
        app.info(&format!("Rule tree {name} is broken and was not loaded: {problem}"));
    }
    if !report.changed.is_empty() || !report.removed.is_empty() {
        app.send_out(json!({"type": "trees_changed"}));
    }
}

pub async fn run(app: Arc<App>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    engine::watch_events(tx);
    refresh_world(&app).await;
    crate::persona::fill(&app).await;
    lock(&app.trees).sync();
    let mut deep_timer = tokio::time::interval(Duration::from_millis(DEEP_TICK_MS));
    let mut slow_timer = tokio::time::interval(Duration::from_millis(SLOW_TICK_MS));
    let mut kids = crate::kid_watch::Kids::default();
    let mut warned: HashSet<String> = HashSet::new();
    let mut idle = crate::idle_watch::Idle::default();
    loop {
        tokio::select! {
            Some(event) = rx.recv() => on_event(&app, &mut kids, event).await,
            _ = app.deep_wake.notified() => deep_tick(&app).await,
            _ = deep_timer.tick() => deep_tick(&app).await,
            _ = slow_timer.tick() => {
                refresh_world(&app).await;
                crate::persona::fill(&app).await;
                sync_trees(&app);
                watch_tick(&app).await;
                crate::drones::tick(&app);
                crate::kid_watch::tick(&app, &mut kids, now_ms()).await;
                crate::context_watch::tick(&app, &mut warned).await;
                crate::idle_watch::tick(&app, &mut idle, now_ms()).await;
                crate::follow_watch::tick(&app, now_ms()).await;
                crate::leave::tick(&app, now_ms()).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_brains::{Cmd, Handle, Kind};

    #[test]
    fn an_event_names_the_space_of_its_terminal_and_task() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let conn = nixa_store::open(&app.db_file).unwrap();
        nixa_store::spaces::set_space(&conn, "shop", &nixa_store::spaces::SpacePatch { name: Some("shop".into()), folder: Some("/work/shop".into()), ..Default::default() }).unwrap();
        let info: nixa_protocol::TermInfo = serde_json::from_value(json!({"id": "m7", "name": "agent", "cwd": "/work/shop/app", "running": true, "rows": 24, "cols": 80, "created_ms": 0})).unwrap();
        lock(&app.watch).world.insert("m7".into(), crate::app::Seen { info, since: 0 });
        let input = nixa_store::tasks::TaskInput { title: "Fix".into(), space_id: "boss".into(), space_name: "boss app".into(), ..Default::default() };
        let task = nixa_store::tasks::add_task(&conn, &input, &nixa_store::tasks::Actor::new(nixa_store::tasks::AuthorKind::User, "Sam")).unwrap();
        assert_eq!(event_spaces(&app, "Event: terminal m7 is done."), vec!["shop".to_string()]);
        assert!(event_spaces(&app, "Event: terminal m77 is done.").is_empty());
        assert!(event_spaces(&app, "Event: terminal m8 links issue #1.").is_empty());
        let both = event_text(&app, &format!("Event: terminal m7 finished task #{}.", task.id));
        assert!(both.ends_with("\nSpace: shop, boss app. Name the space when you tell the user about it."), "{both}");
        assert_eq!(event_text(&app, "Event: the chat brain switch did not happen."), "Event: the chat brain switch did not happen.");
        crate::settings_api::set_privacy(&app, false).ok();
        tell_event(&app, "Event: terminal m7 is done.");
        let last = lock(&app.chat).list().last().cloned().unwrap();
        assert_eq!((last.text.as_str(), last.space.as_deref()), ("shop: Event: terminal m7 is done.", Some("shop")));
        let line = crate::prompt::context_line(&app);
        assert!(line.contains("m7 agent (") && line.contains(", space shop)"), "{line}");
    }

    #[test]
    fn a_send_error_is_logged_as_one_short_line() {
        assert_eq!(send_error_line("\n  draft_busy: a person is typing in m3\nmore detail"), "draft_busy: a person is typing in m3");
        assert_eq!(send_error_line(""), "unknown error");
        assert!(send_error_line(&"x".repeat(500)).chars().count() <= 200);
    }

    #[test]
    fn error_and_exit_events_tell_the_brain_to_recover() {
        let auto = RECOVER_HINT;
        assert!(auto.to_lowercase().contains("do not wait for the user"));
        assert!(auto.contains("nixa_deep_ask"));
        assert!(!auto.contains("Tell the user in one short sentence."));
    }

    #[tokio::test]
    async fn tell_event_sends_to_chosen_cli_brain() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        agent::pick(&app, "opencode").unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        lock(&app.cli.live).run = Some(Handle { id: 1, kind: Kind::Opencode, tx });

        let answer = "Deep brain decision: continue the assigned task.";
        tell_event(&app, answer);

        assert!(matches!(rx.try_recv(), Ok(Cmd::Tell(text)) if text.starts_with(answer)));
    }
}
