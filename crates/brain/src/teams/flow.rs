use super::changes::Changes;
use super::model::{MAX_ROUNDS, Run, RunState, StepState, Verdict};
use super::prompt::{Mode, SUMMARY_CHARS, step_result};
use crate::util::clip;

pub const CHECK_MS: u64 = 2000;
pub const IDLE_GRACE_MS: u64 = 30_000;
pub const STUCK_MS: u64 = 60_000;
pub const STUCK_AUTO: &str = "Do not wait for the user: list the open prompts with nixa_prompts and answer this one with nixa_answer_prompt (decide true and a short reason), or read the tab with nixa_term_read and send the answer with nixa_term_send. For a folder trust question, trust the run's own work folder. Record what you chose in one short line.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Working,
    Waiting,
    Idle,
    Gone,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub seen_working: bool,
    pub idle_since: Option<u64>,
    pub stuck_since: Option<u64>,
    pub warned: bool,
    pub before: Changes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    Begin(usize, Mode),
    End(RunState, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    Wait,
    Tell(String),
    Finish,
    End(RunState, String),
}

fn plural(count: u32, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

pub fn after_step(run: &mut Run, index: usize, text: &str, files: Vec<String>, now: u64) -> (String, Next) {
    let result = step_result(text);
    let total = run.steps.len();
    let step = &mut run.steps[index];
    step.summary = clip(text.trim(), SUMMARY_CHARS);
    let mut all = std::mem::take(&mut step.files);
    all.extend(files.iter().cloned());
    all.sort();
    all.dedup();
    step.files = all;
    step.result = result;
    step.state = if result == Some(Verdict::Fail) { StepState::Failed } else { StepState::Done };
    step.ended_at = Some(now);
    let name = step.name.clone();
    let changed = if files.is_empty() { String::new() } else { format!(", {} changed file{}", files.len(), if files.len() == 1 { "" } else { "s" }) };
    let line = format!(
        "{}: step {} of {total}, {name} {}{changed}.",
        run.team_name,
        index + 1,
        if result == Some(Verdict::Fail) { "found problems" } else { "is done" }
    );
    if result == Some(Verdict::Fail) {
        if index == 0 {
            return (line, Next::End(RunState::Failed, format!("Step 1 ({name}) failed.")));
        }
        if run.rounds >= MAX_ROUNDS {
            return (line, Next::End(RunState::Failed, format!("The {name} still found problems after {}.", plural(MAX_ROUNDS, "round"))));
        }
        run.rounds += 1;
        return (line, Next::Begin(index - 1, Mode::Fix));
    }
    let next = index + 1;
    if next >= total {
        return (line, Next::End(RunState::Done, String::new()));
    }
    let mode = if run.steps[next].state == StepState::Failed { Mode::Recheck } else { Mode::First };
    (line, Next::Begin(next, mode))
}

fn stuck_auto(place: &str, terminal: &str, asking: bool) -> String {
    let what = if asking { "waits for an answer in its tab" } else { "has not started after 1 minute. Its tab may wait at a question, like the folder trust question" };
    format!("Team: {place}, terminal {terminal}, {what}. {STUCK_AUTO}")
}

fn warn_stuck(live: &mut Live, place: &str, terminal: &str, asking: bool, now: u64) -> Check {
    let since = *live.stuck_since.get_or_insert(now);
    if live.warned || now.saturating_sub(since) < STUCK_MS {
        return Check::Wait;
    }
    live.warned = true;
    Check::Tell(stuck_auto(place, terminal, asking))
}

pub fn check_step(live: &mut Live, run: &Run, index: usize, seen: Seen, report_ready: bool, now: u64) -> Check {
    let name = run.steps.get(index).map(|s| s.name.clone()).unwrap_or_default();
    let terminal = run.steps.get(index).map(|s| s.terminal.clone()).unwrap_or_default();
    let place = format!("{}: step {} ({name})", run.team_name, index + 1);
    match seen {
        Seen::Working => {
            live.seen_working = true;
            live.idle_since = None;
            live.stuck_since = None;
            live.warned = false;
            return Check::Wait;
        }
        Seen::Waiting => {
            live.idle_since = None;
            return warn_stuck(live, &place, &terminal, true, now);
        }
        _ => {}
    }
    if report_ready {
        return Check::Finish;
    }
    if seen == Seen::Gone {
        return Check::End(RunState::Stopped, format!("The tab of step {} ({name}) was closed before it wrote its report. Press Resume to start the step again.", index + 1));
    }
    if !live.seen_working {
        return warn_stuck(live, &place, &terminal, false, now);
    }
    let since = *live.idle_since.get_or_insert(now);
    if now.saturating_sub(since) >= IDLE_GRACE_MS {
        return Check::End(RunState::Stopped, format!("The {name} stopped without writing its report. Check its tab, then press Resume."));
    }
    Check::Wait
}

pub fn resume_mode(run: &Run, index: usize) -> Mode {
    if run.steps.get(index + 1).is_some_and(|s| s.state == StepState::Failed) {
        Mode::Fix
    } else if run.steps.get(index).is_some_and(|s| !s.terminal.is_empty()) {
        Mode::Resume
    } else {
        Mode::First
    }
}

pub fn doing(mode: Mode, rounds: u32) -> String {
    match mode {
        Mode::Fix => format!("is fixing the problems (round {rounds} of {MAX_ROUNDS})"),
        Mode::Recheck => format!("checks again (round {rounds} of {MAX_ROUNDS})"),
        Mode::Resume => "goes on".into(),
        Mode::First => "started".into(),
    }
}
