use super::{flow, stall};
use crate::app::{App, lock};
use crate::engine;
use crate::review_flow::{auto_checks, checks, done::DoneEvent, facts_with};
use crate::util::{clip_flat, log, now_ms};
use nixa_protocol::{Status, TermInfo};
use nixa_store::tasks::{TaskFilter, list_tasks};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[cfg(test)]
mod tests;

#[derive(Clone, PartialEq, Eq)]
struct Terminal {
    pid: Option<u32>,
    seq: u64,
    cwd: String,
}

#[derive(Clone)]
pub(crate) struct Completion {
    event: DoneEvent,
    revision: u64,
    space: String,
    terminal: Option<Terminal>,
}

struct Entry {
    revision: u64,
    event_at: u64,
    next: Option<(u64, Completion)>,
}

impl Entry {
    fn matches(&self, completion: &Completion) -> bool {
        self.revision == completion.revision && self.event_at == completion.event.at
    }
}

#[derive(Default)]
pub(crate) struct Recovery {
    entries: HashMap<String, Entry>,
    running: bool,
}

fn enabled(app: &App) -> bool {
    crate::settings_api::nixa_on(&app.dir) && auto_checks::on(&app.dir)
}

fn in_scope(app: &App, key: &str, space: &str) -> Option<()> {
    if !enabled(app)
        || lock(&app.deep).owns(key)
        || crate::teams::owns(&app.teams, key)
        || crate::extras::busy(app, key)
        || crate::queue::asked(app, key)
    {
        return None;
    }
    let conn = nixa_store::open(&app.db_file).ok()?;
    let space = nixa_store::spaces::get_space(&conn, space).ok()??;
    if space.locked {
        return None;
    }
    let filter = TaskFilter {
        terminal: key.to_string(),
        status: flow::HELD.to_vec(),
        ..Default::default()
    };
    list_tasks(&conn, &filter).ok()?.is_empty().then_some(())
}

pub(crate) fn queue(app: &App, event: &DoneEvent, now: u64) {
    if !enabled(app) {
        return;
    }
    let key = &event.terminal;
    let Some(space) = app.space_of_terminal(key) else {
        return;
    };
    if in_scope(app, key, &space.id).is_none() {
        return;
    }
    let jobs = lock(&app.followup.jobs);
    let Some(job) = jobs.get(key).filter(|job| event.at >= job.sent_at) else {
        return;
    };
    let revision = job.revision;
    let mut recovery = lock(&app.followup.recovery);
    if recovery
        .entries
        .get(key)
        .is_some_and(|e| e.revision == revision && event.at <= e.event_at)
    {
        return;
    }
    let completion = Completion {
        event: event.clone(),
        revision,
        space: space.id,
        terminal: None,
    };
    recovery.entries.insert(
        key.clone(),
        Entry {
            revision,
            event_at: event.at,
            next: Some((now, completion)),
        },
    );
}

pub(crate) fn forget(app: &App, terminal: &str) {
    lock(&app.followup.recovery).entries.remove(terminal);
}

impl Completion {
    fn key(&self) -> &str {
        &self.event.terminal
    }

    pub(super) fn current(&self, app: &App) -> bool {
        lock(&app.followup.jobs)
            .get(self.key())
            .is_some_and(|j| j.revision == self.revision)
            && lock(&app.followup.recovery)
                .entries
                .get(self.key())
                .is_some_and(|entry| entry.matches(self))
    }

    async fn read(&self, app: &App) -> Option<Terminal> {
        let id = engine::term_id(self.key()).ok()?;
        let info = engine::get(&id).await.ok()?;
        if self.event.fallback.is_some() {
            let screen = engine::read(&id, Some(flow::REPORT_LINES), false)
                .await
                .ok()?;
            if !flow::report_matches(&screen, &self.event.text) {
                return None;
            }
        }
        self.match_terminal(app, info)
    }

    fn match_terminal(&self, app: &App, info: TermInfo) -> Option<Terminal> {
        let status = if self.event.fallback.is_some() {
            None
        } else {
            Some(Status::Done)
        };
        if !info.running
            || info.draft
            || info.status != status
            || info.labels.contains_key(crate::queue::TASK_LABEL)
            || !self.current(app)
            || in_scope(app, self.key(), &self.space).is_none()
        {
            return None;
        }
        let space = app
            .space_of_terminal(self.key())
            .filter(|space| space.id == self.space)?;
        if !crate::spaces::folder_in(&info.cwd, &space.folder) || !Path::new(&info.cwd).is_dir() {
            return None;
        }
        let term = Terminal {
            pid: info.pid,
            seq: info.seq,
            cwd: info.cwd,
        };
        self.terminal
            .as_ref()
            .is_none_or(|before| *before == term)
            .then_some(term)
    }

    pub(crate) async fn can_send_fix(&self, app: &App) -> bool {
        app.decision.deep_mode() && self.read(app).await.is_some()
    }
}

struct Running<'a>(&'a App);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        lock(&self.0.followup.recovery).running = false;
    }
}

pub(crate) fn tick(app: &Arc<App>) {
    let now = now_ms();
    let completion = {
        let mut recovery = lock(&app.followup.recovery);
        if recovery.running {
            return;
        }
        let Some(entry) = recovery
            .entries
            .values_mut()
            .filter(|e| e.next.as_ref().is_some_and(|(due, _)| *due <= now))
            .min_by_key(|e| e.next.as_ref().map(|(due, _)| *due))
        else {
            return;
        };
        let (_, completion) = entry
            .next
            .take()
            .expect("filtered for entries with a pending check");
        recovery.running = true;
        completion
    };
    let app = Arc::clone(app);
    tokio::spawn(async move {
        let _running = Running(&app);
        check(&app, completion).await;
    });
}

fn again(app: &App, completion: Completion, delay: u64) {
    let mut recovery = lock(&app.followup.recovery);
    if let Some(entry) = recovery.entries.get_mut(completion.key())
        && entry.matches(&completion)
    {
        entry.next = Some((now_ms().saturating_add(delay), completion));
    }
}

async fn check(app: &Arc<App>, mut completion: Completion) {
    if !completion.current(app) {
        return;
    }
    let Some(terminal) = completion.read(app).await else {
        return;
    };
    let folder = terminal.cwd.clone();
    completion.terminal = Some(terminal);
    let (facts, outcome) = facts_with(&app.dir, &folder, true, None).await;
    if completion.read(app).await.is_none() {
        return;
    }
    finish(app, completion, facts.unread, outcome);
}

fn finish(
    app: &Arc<App>,
    completion: Completion,
    unread: bool,
    outcome: Option<checks::ChecksOutcome>,
) {
    if !completion.current(app) {
        return;
    }
    let key = completion.key();
    let place = flow::place_of(app, key);
    if unread
        || outcome
            .as_ref()
            .is_some_and(|o| o.results.iter().any(|r| r.timed_out))
    {
        let reason = if unread {
            "project changes could not be read"
        } else {
            "project checks timed out"
        };
        let next = stall::plan_review(&mut lock(&app.followup.stalls), key);
        if let Some((attempt, delay)) = next {
            app.info(&format!(
                "{place}: {reason}. Nixa will retry in {} seconds.",
                delay / 1000
            ));
            log(
                "followup",
                &format!("retry check of {key} ({attempt}) in {delay} ms"),
            );
            again(app, completion, delay);
        } else {
            flow::hold_job(app, key, &format!("{reason} after three retries"));
        }
        return;
    }
    let Some(outcome) = outcome.filter(|o| !o.none.is_empty() || !o.results.is_empty()) else {
        return;
    };
    if !outcome.none.is_empty() {
        app.info(&format!("{place}: Nixa found no project checks to run."));
        return;
    }
    let summary = clip_flat(&checks::short_checks(&outcome), 300);
    let score = crate::modes::scores::check_state(Some((outcome.failed, false)));
    crate::modes::checked(app, key, score, Some(&summary));
    if !outcome.failed {
        app.info(&format!("{place}: project checks passed. {summary}"));
        return;
    }
    if app.decision.deep_mode()
        && flow::ask_fix(app, key, &summary, Some(completion.clone()))
    {
        return;
    }
    if let Some(attempt) = flow::may_recover(app, key) {
        let delay =
            stall::review_delay(attempt).expect("failed checks use the first two retry delays");
        app.info(&format!(
            "{place}: project checks failed. Nixa will retry in {} seconds.",
            delay / 1000
        ));
        again(app, completion, delay);
    } else {
        flow::hold_job(app, key, &format!("checks failed: {summary}"));
    }
}
