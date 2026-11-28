use super::close::{CLOSE_OPTIONS, CloseFacts, CloseState, KeepHold, close_left_text, close_notes, close_question, close_timeout_text, closes_now, keep_again_notes, keep_open, may_clear};
use super::fix::{FIX_OPTIONS, FIX_WAIT_MS, FIXES_PER_HOUR, FixFacts, fix_notes, fix_now, fix_question, fix_text, same_fix};
use super::hard::DEEP_RELAY_MS;
use super::report::JobKey;
use super::same_work::{SAME_HELD_KEEP_MS, SameWorkInput, anyway, auto_pass, same_work, same_work_key, same_work_line, same_work_reason, task_of};
use super::stall::{RecoverInput, StallKind, can_recover};
use super::standalone::Completion;
use super::{EVENT_KIND, FollowJob, Target};
use crate::app::{App, lock};
use crate::deep::{Ask, AskInput, AskState, Health};
use crate::engine;
use crate::review_flow::done::{DoneEvent, Fallback, Source};
use crate::spaces::Space;
use crate::util::{clip, log, now_ms};
use nixa_protocol::Status;
use nixa_store::tasks::{CommentInput, CommentKind, Task, TaskFilter, TaskStatus};
use regex::Regex;
use serde_json::json;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};

pub const CLOSE_KIND: &str = "close";
pub const FIX_KIND: &str = "fix";
pub const ANYWAY_MS: u64 = 2 * 60 * 1000;
pub const CLOSE_RETRY_MS: u64 = 2 * 60 * 1000;
pub const CLOSE_TRIES: u32 = 5;
pub const MAX_ROUNDS: u32 = 10;
pub const HOLD_TOLD_MS: u64 = 60 * 60 * 1000;
pub const REPORT_LINES: usize = 80;
pub const RETURNS_MAX: usize = 20;
pub const REPORT_COMMENT_CHARS: usize = 1500;
pub const HELD: [TaskStatus; 3] = [TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review];

static TERMINAL_IN: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"\bterminal ({})\b", nixa_protocol::id::TERM_ID_PATTERN)).expect("terminal pattern"));

pub fn number_of(app: &App, terminal: &str) -> Option<u32> {
    lock(&app.announce.numbers).get(terminal).copied()
}

pub fn place_of(app: &App, terminal: &str) -> String {
    match number_of(app, terminal) {
        Some(n) => format!("terminal {n}"),
        None => format!("terminal {terminal}"),
    }
}

fn title_place(app: &App, terminal: &str) -> String {
    let place = place_of(app, terminal);
    format!("T{}", &place[1..])
}

fn space_for(app: &App, terminal: Option<&str>) -> Option<Space> {
    terminal.and_then(|t| app.space_of_terminal(t)).or_else(|| crate::spaces::home(&app.spaces()).cloned())
}

pub fn deep_ready(app: &App, space: &str) -> bool {
    if !crate::deep_service::open_spaces(app).is_ok_and(|spaces| spaces.iter().any(|s| s.id == space)) {
        return false;
    }
    lock(&app.deep).seats.iter().any(|s| s.space == space && s.on && s.joined && s.terminal.is_some() && s.health == Some(Health::Agent))
}

fn ask_deep(app: &Arc<App>, input: AskInput, space: &Space) -> Result<Ask, String> {
    let (made, _) = crate::deep_service::ask(app, input, space)?;
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let app = app.clone();
        handle.spawn(async move { crate::watch::deep_tick(&app).await });
    }
    Ok(made)
}

fn paused(app: &App) -> bool {
    app.dir.join(crate::settings_api::OFF_FILE).exists()
}

fn held_task(app: &App, terminal: &str) -> Option<Task> {
    let conn = nixa_store::open(&app.db_file).ok()?;
    let filter = TaskFilter { terminal: terminal.to_string(), status: HELD.to_vec(), ..TaskFilter::default() };
    nixa_store::tasks::list_tasks(&conn, &filter).ok()?.into_iter().next()
}

fn seen_of(app: &App, terminal: &str) -> Option<nixa_protocol::TermInfo> {
    lock(&app.watch).world.get(terminal).map(|s| s.info.clone())
}

fn working(app: &App, terminal: &str) -> bool {
    seen_of(app, terminal).is_some_and(|i| i.status == Some(Status::Working))
}

fn prompts_of(app: &App, terminal: &str) -> usize {
    lock(&app.prompts.book).by_terminal(terminal).len()
}

fn send_out(app: &App, what: &str, terminal: &str, extra: serde_json::Value) {
    app.send_out(json!({"type": EVENT_KIND, "what": what, "terminal": terminal, "extra": extra}));
}

pub fn relay(app: &Arc<App>, text: &str) -> bool {
    let named = TERMINAL_IN.captures(text).and_then(|c| c[1].parse::<nixa_protocol::TermId>().ok()).map(String::from);
    let space = space_for(app, named.as_deref());
    relay_in_space(app, text, space.as_ref())
}

pub fn relay_in_space(app: &Arc<App>, text: &str, space: Option<&Space>) -> bool {
    let Some(turn) = lock(&app.followup.hard).wanted(text, true) else { return false };
    let Some(space) = space else { return false };
    if app.decision.nixa_decides() {
        log("followup", &format!("{} stays with the quick brain: Nixa decides", turn.kind.word()));
        return false;
    }
    if !deep_ready(app, &space.id) {
        if turn.kind == super::hard::HardKind::Check {
            let over = lock(&app.followup.hard).park(text, &space.id, now_ms());
            log("followup", "check saved until a deep brain is up");
            if let Some(over) = over {
                crate::watch::tell_event(app, &over);
            }
            return true;
        }
        log("followup", &format!("{} stays with the quick brain: no deep brain is up", turn.kind.word()));
        return false;
    }
    let input = AskInput {
        question: turn.question.clone(),
        options: turn.options.clone(),
        notes: text.to_string(),
        kind: Some(turn.kind.word().to_string()),
        wait: Some(DEEP_RELAY_MS),
        brain: None,
    };
    match ask_deep(app, input, space) {
        Ok(made) => {
            lock(&app.followup.hard).asked(&made.id, turn.kind, text);
            app.info_in(&space.name, &format!("{} asked the deep brain {} first: {}.", app.name(), made.id, turn.label));
            log("followup", &format!("{} routed to the deep brain as {}", turn.kind.word(), made.id));
            true
        }
        Err(e) => {
            log("followup", &format!("{} not routed: {}", turn.kind.word(), clip(&e, 160)));
            false
        }
    }
}

pub fn send_parked(app: &Arc<App>) {
    let spaces = lock(&app.followup.hard).parked_spaces();
    if spaces.is_empty() {
        return;
    }
    let ready: HashSet<String> = spaces.into_iter().filter(|s| deep_ready(app, s)).collect();
    let (send, late) = lock(&app.followup.hard).unpark(&ready, now_ms());
    let known = app.spaces();
    for parked in send {
        let space = known.iter().find(|s| s.id == parked.space).cloned();
        if !relay_in_space(app, &parked.text, space.as_ref()) {
            let text = lock(&app.followup.hard).release(parked.text);
            crate::watch::tell_event(app, &text);
        }
    }
    for text in late {
        log("followup", "a saved check waited too long for a deep brain, the quick brain takes it");
        crate::watch::tell_event(app, &text);
    }
}

pub fn flush(app: &Arc<App>) {
    let texts = lock(&app.followup.hard).take(paused(app));
    for text in texts {
        crate::watch::tell_event(app, &text);
    }
}

pub fn check_pending(app: &App) -> bool {
    lock(&app.followup.hard).check_pending()
}

pub async fn answered(app: &Arc<App>, ask: &Ask, placed: Option<&str>) -> bool {
    let target = lock(&app.followup.targets).remove(&ask.id);
    match target {
        Some((Target::Close, terminal)) => {
            close_answered(app, ask, &terminal).await;
            true
        }
        Some((Target::Fix(completion), terminal)) => {
            fix_answered(app, ask, &terminal, completion.as_deref()).await;
            true
        }
        None => {
            let mine = lock(&app.followup.hard).answered(ask, placed);
            if mine {
                flush(app);
            }
            mine
        }
    }
}

pub fn job_started(app: &Arc<App>, terminal: &str, request: &str) -> FollowJob {
    let at = now_ms();
    let job = FollowJob {
        terminal: terminal.to_string(), at, sent_at: at,
        revision: app.followup.next_revision.fetch_add(1, Ordering::Relaxed),
        request: request.to_string(), sent: vec![request.to_string()], rounds: 0,
    };
    super::standalone::forget(app, terminal);
    lock(&app.followup.jobs).insert(terminal.to_string(), job.clone());
    lock(&app.followup.fix_asked).remove(terminal);
    lock(&app.followup.stalls).clear(terminal);
    lock(&app.followup.reports).start(&key_of(&job));
    lock(&app.followup.holds).clear(terminal);
    super::patches::save_soon(app, terminal, 0, request);
    job
}

pub fn job_sent(app: &App, terminal: &str, previous: u64, text: &str) -> bool {
    let key = lock(&app.followup.jobs).get_mut(terminal).filter(|job| job.revision == previous).map(|job| {
        job.sent.push(text.to_string());
        job.rounds += 1;
        job.sent_at = now_ms();
        job.revision = app.followup.next_revision.fetch_add(1, Ordering::Relaxed);
        (key_of(job), job.rounds)
    });
    if let Some((key, round)) = key {
        lock(&app.followup.reports).start(&key);
        super::patches::save_soon(app, terminal, round, text);
        return true;
    }
    false
}

pub fn before_send(app: &App, terminal: &str, screen: &str) {
    lock(&app.followup.reports).prepare(terminal, screen);
}

pub fn close_of(app: &App, terminal: &str, outcome: &str) -> (crate::modes::scores::JobClose, crate::modes::journal::ChangeInput) {
    let (checks, short) = {
        let check = lock(&app.review_flow.check);
        match check.outcomes.get(terminal) {
            Some(ran) => (crate::modes::scores::check_state(Some((ran.failed, !ran.none.is_empty()))), ran.none.is_empty().then(|| crate::review_flow::checks::short_checks(ran))),
            None => (crate::modes::scores::Checks::None, None),
        }
    };
    let spent = lock(&app.followup.jobs).get(terminal).is_some_and(|j| j.rounds >= MAX_ROUNDS);
    let close = crate::modes::scores::JobClose { outcome: outcome.to_string(), checks: Some(checks), spent, short };
    let changes = crate::modes::journal::ChangeInput {
        terminal: terminal.to_string(),
        project: crate::modes::folder_of(app, Some(terminal)),
        files: Vec::new(),
        outcome: outcome.to_string(),
        report: reply_of(app, terminal),
    };
    (close, changes)
}

pub fn job_closed(app: &Arc<App>, terminal: &str, outcome: &str) {
    let (close, changes) = close_of(app, terminal, outcome);
    crate::modes::job_closed(app, terminal, &close, &changes);
    let job = lock(&app.followup.jobs).remove(terminal);
    super::standalone::forget(app, terminal);
    lock(&app.followup.fix_asked).remove(terminal);
    lock(&app.followup.stalls).clear(terminal);
    lock(&app.followup.reports).forget(terminal);
    if outcome == "done"
        && let Some(job) = job
    {
        ask_close(app, terminal, &job.request, None);
    }
}

pub fn reply_of(app: &App, terminal: &str) -> Option<String> {
    lock(&app.followup.replies).get(terminal).cloned()
}

pub fn note_reply(app: &App, terminal: &str, text: &str) {
    lock(&app.followup.replies).insert(terminal.to_string(), text.to_string());
}

fn key_of(job: &FollowJob) -> JobKey {
    JobKey { terminal: job.terminal.clone(), at: job.at, sent: job.sent.last().cloned().unwrap_or_default() }
}

pub fn same_work_held(app: &Arc<App>, text: &str, target: Option<&str>) -> Option<String> {
    let said = lock(&app.brain).last_user.clone();
    if said.is_some_and(|(line, at)| now_ms().saturating_sub(at) < ANYWAY_MS && anyway(&line)) {
        return None;
    }
    let (live, busy): (HashSet<String>, HashSet<String>) = {
        let watch = lock(&app.watch);
        let live = watch.world.iter().filter(|(_, s)| s.info.running).map(|(k, _)| k.clone()).collect();
        let busy = watch.world.iter().filter(|(_, s)| s.info.status == Some(Status::Working)).map(|(k, _)| k.clone()).collect();
        (live, busy)
    };
    let conn = match nixa_store::open(&app.db_file) {
        Ok(conn) => conn,
        Err(e) => {
            log("followup", &format!("same work check skipped: {e:#}"));
            return None;
        }
    };
    let filter = TaskFilter { status: HELD.to_vec(), ..TaskFilter::default() };
    let tasks: Vec<_> = nixa_store::tasks::list_tasks(&conn, &filter).unwrap_or_default().iter().map(task_of).collect();
    let files = nixa_store::claims::file_holders(&conn, now_ms() as i64).unwrap_or_default();
    let hit = same_work(&SameWorkInput { text, target, tasks: &tasks, files: &files, live: &live, working: &busy })?;
    let line = same_work_line(&hit, number_of(app, hit.terminal()));
    if let Some(why) = same_work_pass(app, &hit, busy.contains(hit.terminal())) {
        app.info_for(hit.terminal(), &format!("Same work: {line} {} lets it through: {why}.", app.name()));
        crate::modes::note(app, "auto-yes", &format!("sent task {} work although {} holds it", hit.task(), hit.terminal()), target, Some(why));
        log("followup", &format!("same work let through: task {} in {}: {why}", hit.task(), hit.terminal()));
        return None;
    }
    app.info_for(hit.terminal(), &format!("Same work: {line}"));
    log("followup", &format!("same work held: task {} in {}", hit.task(), hit.terminal()));
    crate::extras::held(app);
    Some(same_work_reason(&line))
}

fn same_work_pass(app: &App, hit: &super::same_work::SameWorkHit, working: bool) -> Option<&'static str> {
    let now = now_ms();
    let key = same_work_key(hit);
    let mut held = lock(&app.followup.same_held);
    held.retain(|_, at| now.saturating_sub(*at) < SAME_HELD_KEEP_MS);
    let why = auto_pass(working, held.get(&key).copied(), now);
    if why.is_some() {
        held.remove(&key);
    } else {
        held.entry(key).or_insert(now);
    }
    why
}

pub fn ask_close(app: &Arc<App>, terminal: &str, request: &str, hold: Option<&KeepHold>) -> Option<String> {
    if let Some(id) = lock(&app.followup.closing).get(terminal) {
        return Some(id.clone());
    }
    let space = space_for(app, Some(terminal))?;
    if !app.decision.deep_mode() || !deep_ready(app, &space.id) || lock(&app.deep).owns(terminal) || crate::extras::busy(app, terminal) || kept(app, terminal) {
        return None;
    }
    let place = title_place(app, terminal);
    let facts = CloseFacts {
        place: place.clone(),
        agent: seen_of(app, terminal).map(|i| crate::prompts::scan::agent_of(&i)),
        space: Some(space.name.clone()),
        request: Some(request.to_string()),
        reply: lock(&app.followup.replies).get(terminal).cloned(),
        task: held_task(app, terminal).map(|t| format!("#{} {}: {}", t.id, t.status.as_str(), t.title)),
    };
    let mut notes = close_notes(&facts);
    if let Some(hold) = hold {
        notes.push('\n');
        notes.push_str(&keep_again_notes(hold));
    }
    let input = AskInput {
        question: close_question(&place),
        options: CLOSE_OPTIONS.iter().map(|s| s.to_string()).collect(),
        notes,
        kind: Some(CLOSE_KIND.into()),
        wait: Some(DEEP_RELAY_MS),
        brain: None,
    };
    match ask_deep(app, input, &space) {
        Ok(made) => {
            lock(&app.followup.closing).insert(terminal.to_string(), made.id.clone());
            lock(&app.followup.targets).insert(made.id.clone(), (Target::Close, terminal.to_string()), made.at);
            app.info_for(terminal, &format!("{} asked the deep brain {} if {} can be closed.", app.name(), made.id, place_of(app, terminal)));
            log("followup", &format!("close asked {} for {terminal}", made.id));
            Some(made.id)
        }
        Err(e) => {
            log("followup", &format!("close not asked for {terminal}: {}", clip(&e, 160)));
            None
        }
    }
}

pub fn close_refusal(app: &Arc<App>, terminal: &str) -> Option<String> {
    let place = place_of(app, terminal);
    if lock(&app.deep).owns(terminal) {
        return Some(end_seat(app, terminal, &place));
    }
    if kept(app, terminal) {
        keep_off_soon(app, terminal, &place);
    }
    let hold = lock(&app.followup.holds).get(terminal, now_ms())?;
    lock(&app.followup.holds).clear(terminal);
    crate::modes::note(app, "auto-yes", &format!("cleared the keep open answer {} for {place} to close it", hold.id), Some(terminal), Some("Nixa closes instead of refusing"));
    log("followup", &format!("close let through for {terminal}: keep open from {} cleared", hold.id));
    None
}

async fn close_answered(app: &Arc<App>, ask: &Ask, terminal: &str) {
    {
        let mut closing = lock(&app.followup.closing);
        if closing.get(terminal) == Some(&ask.id) {
            closing.remove(terminal);
        }
    }
    let name = place_of(app, terminal);
    let answered = ask.state == AskState::Answered;
    let closes = closes_now(ask.pick.as_deref());
    let said = ask.why.clone().filter(|w| !w.trim().is_empty()).or_else(|| ask.pick.clone()).unwrap_or_default();
    if answered && !closes {
        lock(&app.followup.holds).hold(terminal, &ask.id, &said, now_ms());
    }
    if !answered {
        app.info_for(terminal, &format!("{} got no close answer from the deep brain {}. {} decides it.", title_place(app, terminal), ask.id, app.name()));
        crate::modes::note(app, "close", &format!("decide the close of {name}"), Some(terminal), Some("the deep brain gave no answer"));
        lock(&app.followup.holds).clear(terminal);
        tell_brain(app, &close_timeout_text(&name));
        return;
    }
    if !answered || !closes {
        let why = if answered { clip(&said, 160) } else { "the deep brain gave no answer".into() };
        app.info_for(terminal, &format!("{} stays open: {why}.", title_place(app, terminal)));
        return;
    }
    let mut state = close_state(app, terminal).await;
    if state.kept && !state.deep && keep_off(app, terminal, &name).await {
        state = close_state(app, terminal).await;
    }
    if let Some(reason) = keep_open(&state) {
        app.info_for(terminal, &format!("Deep brain {} said close {name}, but {} kept it open: {reason}.", ask.id, app.name()));
        log("followup", &format!("close kept {terminal}: {reason}"));
        if may_clear(&state) {
            close_later(app, terminal, &ask.id, &said);
        } else if state.live {
            tell_brain(app, &close_left_text(&name, &reason));
        }
        return;
    }
    close_now(app, terminal, &ask.id, &said).await;
}

fn kept(app: &App, terminal: &str) -> bool {
    lock(&app.watch).world.get(terminal).is_some_and(|s| s.info.keep)
}

fn keep_cleared(app: &App, terminal: &str, place: &str) {
    if let Some(seen) = lock(&app.watch).world.get_mut(terminal) {
        seen.info.keep = false;
    }
    crate::modes::note(app, "auto-yes", &format!("turned keep off for {place} to close it"), Some(terminal), Some("Nixa closes instead of refusing"));
    log("followup", &format!("keep turned off for {terminal} to close it"));
}

async fn keep_off(app: &Arc<App>, terminal: &str, place: &str) -> bool {
    let Ok(id) = engine::term_id(terminal) else { return false };
    if let Err(e) = engine::quick(nixa_protocol::Request::Keep { id, on: false }).await {
        log("followup", &format!("keep not turned off for {terminal}: {}", clip(&e.to_string(), 160)));
        return false;
    }
    keep_cleared(app, terminal, place);
    true
}

fn keep_off_soon(app: &Arc<App>, terminal: &str, place: &str) {
    keep_cleared(app, terminal, place);
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    let Ok(id) = engine::term_id(terminal) else { return };
    let terminal = terminal.to_string();
    handle.spawn(async move {
        if let Err(e) = engine::quick(nixa_protocol::Request::Keep { id, on: false }).await {
            log("followup", &format!("keep not turned off for {terminal}: {}", clip(&e.to_string(), 160)));
        }
    });
}

fn tell_brain(app: &Arc<App>, text: &str) {
    if crate::agent::has_brain(app) {
        crate::agent::event(app, text);
    } else {
        crate::modes::effects::carry(app, text.to_string());
        log("followup", "no chat brain is up, the event waits for its next turn");
    }
}

async fn close_state(app: &Arc<App>, terminal: &str) -> CloseState {
    let job = lock(&app.followup.jobs).contains_key(terminal) || lock(&app.watch).jobs.contains_key(terminal);
    let mut state = CloseState {
        deep: lock(&app.deep).owns(terminal),
        kept: kept(app, terminal),
        live: true,
        job,
        prompts: prompts_of(app, terminal),
        busy: crate::teams::owns(&app.teams, terminal) || crate::extras::busy(app, terminal),
        working: working(app, terminal),
        task: held_task(app, terminal).map(|t| format!("#{}", t.id)),
    };
    if keep_open(&state).is_none() {
        let id = engine::term_id(terminal).ok();
        state.live = match &id {
            Some(id) => engine::get(id).await.map(|t| t.running).unwrap_or(false),
            None => false,
        };
    }
    state
}

async fn close_now(app: &Arc<App>, terminal: &str, ask_id: &str, said: &str) {
    let name = place_of(app, terminal);
    let space = app.space_of_terminal(terminal).map(|s| s.name).unwrap_or_default();
    if let Some(held) = crate::modes::hold_work(app, "close-job", &format!("close {name}"), Some(terminal)) {
        log("followup", &format!("close of {terminal} held: {}", clip(&held, 160)));
        return;
    }
    let id = engine::term_id(terminal).ok();
    let closed = match &id {
        Some(id) => engine::remove(id).await.is_ok(),
        None => false,
    };
    if closed {
        lock(&app.followup.holds).clear(terminal);
        send_out(app, "closed", terminal, json!({"ask": ask_id}));
        crate::modes::note(app, "close", &format!("close {name}"), Some(terminal), Some(&format!("deep brain {ask_id}: {}", clip(said, 200))));
    }
    app.info_in(&space, &if closed {
        format!("{} closed {name}: deep brain {ask_id} said it needs no more messages.", app.name())
    } else {
        format!("{} could not close {name}.", app.name())
    });
    log("followup", &format!("{} {terminal} after {ask_id}", if closed { "closed" } else { "could not close" }));
}

fn close_later(app: &Arc<App>, terminal: &str, ask_id: &str, said: &str) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    let (app, terminal, ask_id, said) = (app.clone(), terminal.to_string(), ask_id.to_string(), said.to_string());
    handle.spawn(async move {
        let mut reason = String::new();
        for _ in 0..CLOSE_TRIES {
            tokio::time::sleep(std::time::Duration::from_millis(CLOSE_RETRY_MS)).await;
            let state = close_state(&app, &terminal).await;
            match keep_open(&state) {
                None => {
                    close_now(&app, &terminal, &ask_id, &said).await;
                    return;
                }
                Some(why) if !may_clear(&state) => {
                    log("followup", &format!("close retry of {terminal} stopped: {why}"));
                    return;
                }
                Some(why) => reason = why,
            }
        }
        let place = place_of(&app, &terminal);
        crate::modes::note(&app, "close", &format!("close {place} later"), Some(&terminal), Some(&format!("still open after {CLOSE_TRIES} tries: {reason}")));
        tell_brain(&app, &close_left_text(&place, &reason));
    });
}

pub fn may_recover(app: &App, terminal: &str) -> Option<u32> {
    let rounds = lock(&app.followup.jobs).get(terminal).map(|j| j.rounds)?;
    let input = RecoverInput { rounds, max_rounds: MAX_ROUNDS, cap_allows: lock(&app.followup.cap).allows(now_ms()) };
    can_recover(&input).then(|| lock(&app.followup.stalls).take(StallKind::Checks, terminal)).flatten()
}

#[cfg(test)]
pub fn take_stall(app: &App, kind: StallKind, terminal: &str) -> Option<u32> {
    lock(&app.followup.stalls).take(kind, terminal)
}

pub(crate) fn ask_fix(app: &Arc<App>, terminal: &str, checks: &str, completion: Option<Completion>) -> bool {
    let Some(job) = lock(&app.followup.jobs).get(terminal).cloned() else { return false };
    let Some(space) = space_for(app, Some(terminal)) else { return false };
    if completion.as_ref().is_some_and(|done| !done.current(app))
        || !app.decision.deep_mode()
        || !deep_ready(app, &space.id)
        || lock(&app.deep).owns(terminal)
        || lock(&app.followup.fix_asked).contains(terminal)
        || job.rounds >= MAX_ROUNDS
        || !lock(&app.followup.cap).allows(now_ms())
    {
        return false;
    }
    let place = title_place(app, terminal);
    let facts = FixFacts {
        place: place.clone(),
        agent: seen_of(app, terminal).map(|i| crate::prompts::scan::agent_of(&i)),
        space: Some(space.name.clone()),
        request: Some(job.request.clone()),
        reply: lock(&app.followup.replies).get(terminal).cloned(),
        checks: checks.to_string(),
        sent: job.sent.clone(),
    };
    let input = AskInput {
        question: fix_question(&place),
        options: FIX_OPTIONS.iter().map(|s| s.to_string()).collect(),
        notes: fix_notes(&facts),
        kind: Some(FIX_KIND.into()),
        wait: Some(FIX_WAIT_MS),
        brain: None,
    };
    match ask_deep(app, input, &space) {
        Ok(made) => {
            lock(&app.followup.fix_asked).insert(terminal.to_string());
            lock(&app.followup.targets).insert(made.id.clone(), (Target::Fix(completion.map(Box::new)), terminal.to_string()), made.at);
            app.info_for(terminal, &format!("The checks failed on {} and no fix was sent. {} asked the deep brain {} for one fix.", place_of(app, terminal), app.name(), made.id));
            log("followup", &format!("fix asked {} for {terminal}", made.id));
            true
        }
        Err(e) => {
            log("followup", &format!("fix not asked for {terminal}: {}", clip(&e, 160)));
            false
        }
    }
}

fn end_seat(app: &Arc<App>, terminal: &str, place: &str) -> String {
    let left = lock(&app.deep).leave(terminal);
    let seat = match left {
        Ok(seat) => seat,
        Err(e) => {
            log("followup", &format!("deep brain seat {terminal} not ended: {}", clip(&e, 160)));
            return deep_seat_refusal(place, &app.name());
        }
    };
    crate::modes::note(app, "auto-yes", &format!("ended deep brain seat {} in {place} to close it", seat.name), Some(terminal), Some("Nixa closes instead of refusing"));
    close_seat_soon(app, terminal);
    deep_seat_ended_text(place, &seat.name)
}

fn close_seat_soon(app: &Arc<App>, terminal: &str) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    let Ok(id) = engine::term_id(terminal) else { return };
    let (app, terminal) = (app.clone(), terminal.to_string());
    handle.spawn(async move {
        let closed = engine::remove(&id).await.is_ok();
        if closed {
            lock(&app.followup.holds).clear(&terminal);
            crate::modes::note(&app, "close", &format!("closed {} after its deep brain seat ended", place_of(&app, &terminal)), Some(terminal.as_str()), None);
        }
        log("followup", &format!("{} deep brain terminal {terminal}", if closed { "closed" } else { "could not close" }));
    });
}

pub fn deep_seat_ended_text(place: &str, seat: &str) -> String {
    format!("{place} was the deep brain seat {seat}. Nixa ended the seat and closes the terminal now. Do not ask the user. When you need deep answers again, add a fresh seat with nixa_deep_ask start, and go on.")
}

pub fn deep_seat_refusal(place: &str, name: &str) -> String {
    format!("{place} is {name}'s deep brain seat, so it is not closed from here. Nothing was closed. Do not ask the user: keep asking that seat with nixa_deep_ask, or add a fresh one with nixa_deep_ask start, and go on.")
}

pub fn hand_on_text(place: &str, reason: &str) -> String {
    format!("Event: {place} stopped: {reason}. Do not wait for the user and do not tell them it waits. Decide the next step now: read its screen with nixa_term_read, then send one new fix, give the work to another agent, ask nixa_deep_ask, or close the job. Record what you chose in one short line.")
}

pub fn hold_job(app: &Arc<App>, terminal: &str, reason: &str) {
    let reason = reason.trim().trim_end_matches(['.', '!', '?']).to_string();
    let text = format!("{} stopped: {reason}. {} decides the next step.", title_place(app, terminal), app.name());
    let now = now_ms();
    let again = {
        let mut told = lock(&app.followup.told_holds);
        let again = told.get(terminal).is_some_and(|(said, at)| now.saturating_sub(*at) < HOLD_TOLD_MS && same_fix(std::slice::from_ref(said), &reason));
        if !again {
            told.insert(terminal.to_string(), (reason.clone(), now));
        }
        again
    };
    if again {
        app.info_for(terminal, &format!("Still: {text}"));
        return;
    }
    app.info_for(terminal, &text);
    send_out(app, "handed_on", terminal, json!({"reason": reason}));
    crate::modes::note(app, "hold", &format!("{} goes back to {} to decide", place_of(app, terminal), app.name()), Some(terminal), Some(&reason));
    if !crate::agent::has_brain(app) {
        log("followup", &format!("{terminal} stopped with no chat brain up: {reason}"));
    }
    tell_brain(app, &hand_on_text(&place_of(app, terminal), &reason));
}

fn hold_fix(app: &Arc<App>, terminal: &str, reason: &str, id: &str) {
    hold_job(app, terminal, &format!("checks failed: {reason}"));
    job_closed(app, terminal, "checks failed");
    log("followup", &format!("fix held for {terminal} after {id}: {reason}"));
}

async fn fix_answered(app: &Arc<App>, ask: &Ask, terminal: &str, completion: Option<&Completion>) {
    let name = place_of(app, terminal);
    let job = lock(&app.followup.jobs).get(terminal).cloned();
    let Some(job) = job.filter(|j| j.at <= ask.at) else {
        app.info_for(terminal, &format!("Deep brain {} answered the fix for {name} after its job ended. {} sent nothing.", ask.id, app.name()));
        return;
    };
    if working(app, terminal) || prompts_of(app, terminal) > 0 {
        app.info_for(terminal, &format!("Deep brain {} gave a fix for {name}, but its agent is busy again. {} checks it when it finishes.", ask.id, app.name()));
        return;
    }
    if let Some(completion) = completion
        && !completion.can_send_fix(app).await
    {
        app.info_for(terminal, &format!("{name}: the job or its recovery settings changed while the deep brain was deciding."));
        return;
    }
    let answered = ask.state == AskState::Answered;
    let send = answered && fix_now(ask.pick.as_deref());
    let text = if send { fix_text(ask.why.as_deref()) } else { None };
    let said = ask.why.clone().filter(|w| !w.is_empty()).or_else(|| ask.pick.clone()).unwrap_or_default();
    let stop = if !answered {
        Some("the deep brain gave no fix in time".to_string())
    } else if !send {
        Some(format!("deep brain {} held it: {}", ask.id, clip(&said, 200)))
    } else if text.is_none() {
        Some(format!("deep brain {} gave no fix text", ask.id))
    } else if job.rounds >= MAX_ROUNDS {
        Some("used all rounds".to_string())
    } else if text.as_deref().is_some_and(|t| same_fix(&job.sent, t)) {
        Some("the deep brain's fix is the same as an earlier one".to_string())
    } else if !lock(&app.followup.cap).allows(now_ms()) {
        Some(format!("{FIXES_PER_HOUR} fixes in the last hour"))
    } else {
        None
    };
    let (None, Some(text)) = (stop.clone(), text) else {
        hold_fix(app, terminal, stop.as_deref().unwrap_or("no fix text"), &ask.id);
        return;
    };
    let id = engine::term_id(terminal);
    if let (Some(completion), Ok(id)) = (completion, &id) {
        let Ok(screen) = engine::read(id, Some(REPORT_LINES), false).await else {
            if completion.current(app) {
                hold_fix(app, terminal, "the terminal screen could not be read before sending the fix", &ask.id);
            }
            return;
        };
        if !completion.can_send_fix(app).await {
            return;
        }
        before_send(app, terminal, &screen);
    }
    if let Some(held) = crate::modes::hold_work(app, "fix", &text, Some(terminal)) {
        log("followup", &format!("fix from {} for {terminal} held: {}", ask.id, clip(&held, 160)));
        return;
    }
    let sent = match id {
        Ok(id) => engine::send_now(&id, &text).await.is_ok(),
        Err(_) => false,
    };
    if !sent {
        if completion.is_none_or(|done| done.current(app)) {
            hold_fix(app, terminal, &format!("the fix from deep brain {} could not be sent", ask.id), &ask.id);
        }
        return;
    }
    lock(&app.followup.cap).note(now_ms());
    if !job_sent(app, terminal, job.revision, &text) {
        return;
    }
    crate::modes::note(app, "fix", &clip(&text, 300), Some(terminal), Some(&format!("deep brain {}", ask.id)));
    lock(&app.watch).jobs.insert(terminal.to_string(), now_ms());
    app.info_for(terminal, &format!("{} sent the fix from deep brain {} to {name}.", app.name(), ask.id));
    send_out(app, "fix_sent", terminal, json!({"ask": ask.id}));
    log("followup", &format!("fix from {} sent to {terminal}", ask.id));
}

pub async fn returned(app: &Arc<App>, terminal: &str) -> Option<(String, String)> {
    let job = lock(&app.followup.jobs).get(terminal).cloned()?;
    let id = engine::term_id(terminal).ok()?;
    let screen = engine::read(&id, Some(REPORT_LINES), false).await.ok()?;
    let current = lock(&app.followup.jobs).get(terminal).cloned()?;
    if current.revision != job.revision {
        return None;
    }
    let report = lock(&app.followup.reports).observe(&key_of(&job), &screen)?;
    note_reply(app, terminal, &report);
    Some((report, screen))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handed {
    Review(i64),
    Queued,
    Skipped,
}

pub fn hand_report(app: &Arc<App>, terminal: &str, report: &str, screen: &str) -> Handed {
    let Some(id) = doing_task(app, terminal) else {
        return if queue_done(app, terminal, report, screen) { Handed::Queued } else { Handed::Skipped };
    };
    match to_review(app, id, terminal, report) {
        Ok(true) => Handed::Review(id),
        Ok(false) => {
            log("followup", &format!("task #{id} of {terminal} changed before the review move, so it was skipped"));
            Handed::Skipped
        }
        Err(e) => {
            log("followup", &format!("task #{id} of {terminal} not moved to review: {}", clip(&e, 160)));
            if queue_done(app, terminal, report, screen) { Handed::Queued } else { Handed::Skipped }
        }
    }
}

fn doing_task(app: &App, terminal: &str) -> Option<i64> {
    let conn = nixa_store::open(&app.db_file).ok()?;
    let filter = TaskFilter { terminal: terminal.to_string(), status: vec![TaskStatus::Doing], ..TaskFilter::default() };
    nixa_store::tasks::list_tasks(&conn, &filter).ok()?.into_iter().next().map(|t| t.id)
}

fn to_review(app: &Arc<App>, id: i64, terminal: &str, report: &str) -> Result<bool, String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| format!("{e:#}"))?;
    let text = format!("The agent in {} returned this report with no status: {}", place_of(app, terminal), clip(report, REPORT_COMMENT_CHARS));
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
    let moved = review_if_doing(&conn, id, terminal, &text);
    let end = if matches!(moved, Ok(true)) { "COMMIT" } else { "ROLLBACK" };
    conn.execute_batch(end).map_err(|e| e.to_string())?;
    if !moved.map_err(|e| format!("{e:#}"))? {
        return Ok(false);
    }
    drop(conn);
    crate::queue::on_change(app, id, "status:review", "nixa");
    crate::queue::changed(app, "status:review", Some(id));
    log("followup", &format!("{terminal} came back with no status: task #{id} moved to review"));
    Ok(true)
}

pub fn review_if_doing(conn: &nixa_store::Connection, id: i64, terminal: &str, text: &str) -> anyhow::Result<bool> {
    let task = nixa_store::tasks::must_task(conn, id)?;
    if task.status != TaskStatus::Doing || task.terminal_id != terminal {
        return Ok(false);
    }
    nixa_store::tasks::set_status(conn, id, TaskStatus::Review, None, false)?;
    nixa_store::tasks::add_comment(conn, id, &CommentInput::new(CommentKind::Result, text), &crate::queue::nixa())?;
    Ok(true)
}

pub fn still_fresh(app: &App, event: &DoneEvent) -> bool {
    let Some(fallback) = &event.fallback else { return true };
    let job = lock(&app.followup.jobs).get(&event.terminal).cloned();
    let same_job = job.is_some_and(|j| j.at == fallback.job_at && j.sent.last().map(String::as_str).unwrap_or_default() == fallback.sent);
    let same_report = reply_of(app, &event.terminal).is_some_and(|r| r.trim() == event.text.trim());
    let on_screen = lock(&app.followup.queued).get(&event.terminal).is_some_and(|r| r == event.text.trim());
    same_job && same_report && on_screen && !working(app, &event.terminal) && prompts_of(app, &event.terminal) == 0
}

fn queue_done(app: &App, terminal: &str, report: &str, screen: &str) -> bool {
    let Some(job) = lock(&app.followup.jobs).get(terminal).cloned() else { return false };
    let info = seen_of(app, terminal);
    let event = DoneEvent {
        terminal: terminal.to_string(),
        agent: info.as_ref().map(|i| i.name.clone()).unwrap_or_default(),
        cwd: info.map(|i| i.cwd).unwrap_or_default(),
        text: report.to_string(),
        screen: screen.to_string(),
        source: Source::Watch,
        at: now_ms(),
        fallback: Some(Fallback { job_at: job.at, sent: job.sent.last().cloned().unwrap_or_default() }),
    };
    let queued = crate::review_flow::on_done(app, event);
    if queued {
        lock(&app.followup.queued).insert(terminal.to_string(), report.trim().to_string());
    }
    queued
}

pub async fn recheck_queued(app: &Arc<App>) {
    let list: Vec<(String, String)> = lock(&app.followup.queued).iter().map(|(t, r)| (t.clone(), r.clone())).collect();
    for (terminal, report) in list {
        if !lock(&app.review_flow.done).has(&terminal) {
            lock(&app.followup.queued).remove(&terminal);
            continue;
        }
        if !screen_still_shows(&terminal, &report).await {
            lock(&app.followup.queued).remove(&terminal);
            log("followup", &format!("{terminal} changed after its report, so its done fallback is dropped"));
        }
    }
}

async fn screen_still_shows(terminal: &str, report: &str) -> bool {
    let Ok(id) = engine::term_id(terminal) else { return false };
    let Ok(screen) = engine::read(&id, Some(REPORT_LINES), false).await else { return false };
    report_matches(&screen, report)
}

pub fn report_matches(screen: &str, report: &str) -> bool {
    super::report::input_empty(screen) && super::report::follow_up_report(screen).is_some_and(|r| r.trim() == report.trim())
}

pub fn checked(app: &App, terminal: &str, screen: &str) {
    let Some(job) = lock(&app.followup.jobs).get(terminal).cloned() else { return };
    lock(&app.followup.reports).checked(&key_of(&job), screen);
}

pub fn handed(app: &App, terminal: &str, report: &str) {
    let Some(job) = lock(&app.followup.jobs).get(terminal).cloned() else { return };
    lock(&app.followup.reports).mark(&key_of(&job), report);
}

pub fn take_returns(app: &App) -> Vec<(String, String)> {
    lock(&app.followup.returns).drain(..).collect()
}

fn quiet_jobs(app: &App) -> Vec<String> {
    let jobs: Vec<String> = {
        let mut list: Vec<String> = lock(&app.followup.jobs).keys().cloned().collect();
        list.sort();
        list
    };
    let world = lock(&app.watch).world.clone();
    let deep = lock(&app.deep);
    jobs.into_iter()
        .filter(|t| world.get(t).is_some_and(|s| s.info.running && s.info.status.is_none()) && !deep.owns(t))
        .collect()
}

pub async fn tick(app: &Arc<App>) {
    if paused(app) {
        return;
    }
    flush(app);
    let (jobs, live): (HashSet<String>, HashSet<String>) = {
        let jobs = lock(&app.followup.jobs).keys().cloned().collect();
        let live = lock(&app.watch).world.iter().filter(|(_, s)| s.info.running).map(|(k, _)| k.clone()).collect();
        (jobs, live)
    };
    lock(&app.followup.reports).keep(&jobs, &live);
    let quiet = quiet_jobs(app);
    if quiet.is_empty() {
        return;
    }
    let next = {
        let mut cursor = lock(&app.followup.cursor);
        let pick = quiet[*cursor % quiet.len()].clone();
        *cursor = cursor.wrapping_add(1);
        pick
    };
    if prompts_of(app, &next) > 0 {
        return;
    }
    if let Some((report, screen)) = returned(app, &next).await {
        handed(app, &next, &report);
        let handed_to = hand_report(app, &next, &report, &screen);
        log("followup", &format!("completion fallback for {next}: {handed_to:?}"));
        {
            let mut returns = lock(&app.followup.returns);
            returns.push_back((next.clone(), report.clone()));
            while returns.len() > RETURNS_MAX {
                returns.pop_front();
            }
        }
        send_out(app, "returned", &next, json!({"report": clip(&report, 900)}));
        log("followup", &format!("{next} came back with no status"));
    }
}
