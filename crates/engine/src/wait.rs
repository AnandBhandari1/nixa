use crate::engine::Engine;
use crate::terminal::{Terminal, lock, reports};
use nixa_protocol::{DEFAULT_WAIT_TIMEOUT_MS, ErrorCode, Reply, Status, WaitFor, WaitOutcome, WaitResult};
use regex::{Regex, RegexBuilder};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const MAX_TIMEOUT_MS: u64 = 7 * 24 * 60 * 60 * 1000;

pub fn deadline_after(start: Instant, timeout_ms: u64) -> Instant {
    let limit = Duration::from_millis(timeout_ms.min(MAX_TIMEOUT_MS));
    start.checked_add(limit).unwrap_or(start)
}

enum Matcher {
    Plain(String),
    Pattern(Regex),
}

fn find(m: &Matcher, text: &str) -> Option<String> {
    match m {
        Matcher::Plain(p) => text.contains(p.as_str()).then(|| p.clone()),
        Matcher::Pattern(r) => r.find(text).map(|f| f.as_str().to_string()),
    }
}

fn matcher(until: &WaitFor) -> Result<Option<Matcher>, String> {
    match until {
        WaitFor::Text { pattern, .. } if pattern.is_empty() => Err("empty text pattern".to_string()),
        WaitFor::Text { pattern, regex: false } => Ok(Some(Matcher::Plain(pattern.clone()))),
        WaitFor::Text { pattern, regex: true } => RegexBuilder::new(pattern)
            .multi_line(true)
            .build()
            .map(|r| Some(Matcher::Pattern(r)))
            .map_err(|e| format!("bad regex: {e}")),
        _ => Ok(None),
    }
}

struct Start {
    from: usize,
    exits: u64,
    seq: Option<u64>,
    children: Vec<Arc<Terminal>>,
}

fn child_complete(term: &Terminal) -> bool {
    let inner = lock(term);
    inner.removed || inner.exits > 0 || matches!(inner.status, Some(Status::Done | Status::Error))
}

fn check(
    engine: &Engine,
    term: &Terminal,
    until: &WaitFor,
    m: Option<&Matcher>,
    start: &mut Start,
) -> Option<(WaitOutcome, Option<String>)> {
    let i = lock(term);
    let exited = !i.running || i.exits > start.exits;
    match until {
        WaitFor::Idle => {
            if exited {
                Some((WaitOutcome::Exited, None))
            } else {
                (i.status != Some(Status::Working)).then_some((WaitOutcome::Met, None))
            }
        }
        WaitFor::Status { status } => {
            if i.status == Some(*status) {
                Some((WaitOutcome::Met, None))
            } else {
                exited.then_some((WaitOutcome::Exited, None))
            }
        }
        WaitFor::Text { .. } => {
            if start.seq != Some(i.seq) {
                start.seq = Some(i.seq);
                if let Some(found) = m.and_then(|m| find(m, i.output.since(start.from))) {
                    return Some((WaitOutcome::Met, Some(found)));
                }
            }
            exited.then_some((WaitOutcome::Exited, None))
        }
        WaitFor::Exit => exited.then_some((WaitOutcome::Met, None)),
        WaitFor::Children => {
            let kids = i.children.clone();
            drop(i);
            let all_ended = kids
                .iter()
                .all(|k| engine.get(k).is_none_or(|t| !lock(&t).running));
            all_ended.then_some((WaitOutcome::Met, None))
        }
        WaitFor::ChildAny | WaitFor::ChildAll => {
            drop(i);
            if matches!(until, WaitFor::ChildAny) {
                start.children.iter().find(|child| child_complete(child))
                    .map(|child| (WaitOutcome::Met, Some(child.id.to_string())))
            } else {
                start.children.iter().all(|child| child_complete(child)).then_some((WaitOutcome::Met, None))
            }
        }
    }
}

pub fn run(engine: &Engine, term: &Terminal, until: WaitFor, timeout_ms: Option<u64>) -> Reply {
    let m = match matcher(&until) {
        Ok(m) => m,
        Err(e) => return Reply::error(ErrorCode::BadRequest, e),
    };
    let began = Instant::now();
    let deadline = deadline_after(began, timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS));
    let (mut start, children) = {
        let i = lock(term);
        let start = Start {
            from: i.input_mark.min(i.output.end()),
            exits: i.exits,
            seq: None,
            children: Vec::new(),
        };
        (start, if matches!(until, WaitFor::ChildAny | WaitFor::ChildAll) { i.children.clone() } else { Vec::new() })
    };
    start.children = children.iter().filter_map(|id| engine.get(id)).collect();
    let cross_term = matches!(until, WaitFor::Children | WaitFor::ChildAny | WaitFor::ChildAll);
    let outcome = if matches!(until, WaitFor::ChildAny) && start.children.is_empty() {
        let Some(child) = reports::last_exit(term) else {
            return Reply::error(ErrorCode::BadRequest, "wait-any needs at least one child terminal");
        };
        (WaitOutcome::Met, Some(child.to_string()))
    } else {
        loop {
            let seen = if cross_term { engine.tick() } else { term.tick() };
            if let Some(found) = check(engine, term, &until, m.as_ref(), &mut start) {
                break found;
            }
            if Instant::now() >= deadline {
                break (WaitOutcome::Timeout, None);
            }
            if cross_term {
                engine.wait_change(seen, deadline);
            } else {
                term.wait_change(seen, deadline);
            }
        }
    };
    let status = lock(term).status;
    Reply::Waited {
        result: WaitResult {
            id: term.id.clone(),
            outcome: outcome.0,
            status,
            matched: outcome.1,
            waited_ms: began.elapsed().as_millis() as u64,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_timeouts_do_not_overflow() {
        let now = Instant::now();
        let far = deadline_after(now, u64::MAX);
        assert_eq!(far, now + Duration::from_millis(MAX_TIMEOUT_MS));
        assert_eq!(deadline_after(now, 250), now + Duration::from_millis(250));
    }
}
