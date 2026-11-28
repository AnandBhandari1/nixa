//! Collects task and terminal subjects for the drones decision tree.

use super::{DroneSubject, FactMap};
use crate::answer_rules::store::RuleStatus;
use crate::app::{App, lock, status_word};
use crate::queue::{locked_space, unlocked};
use crate::util::{clip_flat, now_ms};
use nixa_protocol::TermInfo;
use nixa_store::claims::file_holders;
use nixa_store::spaces::list_spaces;
use nixa_store::tasks::{Archived, TaskFilter, TaskStatus, list_tasks, task_comments};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

fn minutes(since: u64, now: u64) -> u64 {
    now.saturating_sub(since) / 60_000
}

fn minutes_i64(since: i64, now: u64) -> Option<u64> {
    if since <= 0 {
        return None;
    }
    Some(now.saturating_sub(since as u64) / 60_000)
}

fn agent_of(info: &TermInfo) -> String {
    info.cmd
        .first()
        .map(|c| c.rsplit('/').next().unwrap_or(c).to_string())
        .unwrap_or_else(|| "shell".into())
}

fn looks_like_terminal(holder: &str) -> bool {
    !holder.is_empty()
        && !holder.starts_with(nixa_store::tasks::STARTING_PREFIX)
        && !holder.starts_with(nixa_store::tasks::RUN_PREFIX)
}

pub fn collect(app: &App) -> anyhow::Result<Vec<DroneSubject>> {
    let now = now_ms();
    let spaces_meta = app.spaces();
    let conn = nixa_store::open(&app.db_file)?;
    let rows = list_spaces(&conn)?;
    let holders = file_holders(&conn, now as i64)?;
    let open = list_tasks(
        &conn,
        &TaskFilter {
            archived: Archived::Hide,
            status: vec![
                TaskStatus::Inbox,
                TaskStatus::Todo,
                TaskStatus::Doing,
                TaskStatus::Blocked,
                TaskStatus::Review,
            ],
            ..Default::default()
        },
    )?;

    let mut doing_in: HashMap<String, i64> = HashMap::new();
    for task in &open {
        if task.status == TaskStatus::Doing {
            *doing_in.entry(task.space_id.clone()).or_default() += 1;
        }
    }

    let rules_waiting = crate::answer_rules::store::load(&app.dir)
        .map(|list| {
            list.iter()
                .filter(|r| r.status == RuleStatus::Offer)
                .count()
        })
        .unwrap_or(0);

    let watch = lock(&app.watch);
    let deep = lock(&app.deep);

    let held_terms: HashSet<String> = open
        .iter()
        .filter(|t| {
            matches!(t.status, TaskStatus::Doing | TaskStatus::Blocked) && !t.terminal_id.is_empty()
        })
        .map(|t| t.terminal_id.clone())
        .collect();
    let claimed_terms: HashSet<String> = holders.iter().map(|h| h.terminal.clone()).collect();

    let unlocked_tasks = unlocked(open, &rows);
    let mut out = Vec::with_capacity(unlocked_tasks.len() + watch.world.len());

    for task in &unlocked_tasks {
        let space_row = rows.iter().find(|r| r.space_id == task.space_id);
        let holder = task.terminal_id.as_str();
        let seen = (!holder.is_empty())
            .then(|| watch.world.get(holder))
            .flatten();
        let agent_state = if holder.is_empty() {
            ""
        } else if let Some(seen) = seen {
            if !seen.info.running {
                "closed"
            } else {
                status_word(seen.info.status)
            }
        } else if looks_like_terminal(holder) {
            "closed"
        } else {
            "none"
        };
        let closed_claims = holders
            .iter()
            .filter(|h| {
                h.task == task.id
                    && looks_like_terminal(&h.terminal)
                    && watch.world.get(&h.terminal).is_none_or(|s| !s.info.running)
            })
            .count();
        let space_has_room = space_row
            .is_some_and(|r| doing_in.get(&task.space_id).copied().unwrap_or(0) < r.max_doing);
        let space_label = if !task.space_name.is_empty() {
            task.space_name.clone()
        } else {
            task.space_id.clone()
        };

        let mut facts = FactMap::new();
        facts.insert("subject".into(), json!("task"));
        facts.insert("rules.waiting".into(), json!(rules_waiting));
        facts.insert("task.id".into(), json!(task.id));
        facts.insert("task.title".into(), json!(clip_flat(&task.title, 120)));
        facts.insert("task.status".into(), json!(task.status.as_str()));
        facts.insert("task.assigned".into(), json!(task.assigned));
        facts.insert(
            "task.hasAssignee".into(),
            json!(!task.assigned.trim().is_empty()),
        );
        facts.insert("task.hasAgent".into(), json!(!holder.is_empty()));
        facts.insert("task.agentTerminal".into(), json!(holder));
        facts.insert("task.agentState".into(), json!(agent_state));
        facts.insert(
            "task.agentMinutesInState".into(),
            seen.map(|s| json!(minutes(s.since, now)))
                .unwrap_or(Value::Null),
        );
        facts.insert(
            "task.blockedOn".into(),
            json!(task.blocked_on.map(|b| b.as_str()).unwrap_or("")),
        );
        let parked_id = if task.status == TaskStatus::Blocked {
            let notes = task_comments(&conn, task.id).unwrap_or_default();
            let pairs: Vec<(i64, &str)> = notes.iter().map(|n| (n.id, n.text.as_str())).collect();
            super::parked_note(&pairs)
        } else {
            None
        };
        facts.insert("task.parked".into(), json!(parked_id.is_some()));
        facts.insert(
            "task.parkedMark".into(),
            json!(parked_id
                .map(|id| format!("{}|{}|{id}", task.status.as_str(), task.blocked_on.map(|b| b.as_str()).unwrap_or("")))
                .unwrap_or_default()),
        );
        facts.insert("task.needsUser".into(), json!(task.needs_user));
        facts.insert(
            "task.minutesSinceUpdate".into(),
            minutes_i64(task.updated_at, now)
                .map(|m| json!(m))
                .unwrap_or(Value::Null),
        );
        facts.insert("task.space".into(), json!(space_label));
        facts.insert(
            "task.spaceAutoStart".into(),
            json!(space_row.is_some_and(|r| r.auto_start)),
        );
        facts.insert("task.spaceHasRoom".into(), json!(space_has_room));
        facts.insert("task.closedClaims".into(), json!(closed_claims));

        out.push(DroneSubject {
            kind: "task",
            key: format!("task #{}", task.id),
            facts,
        });
    }

    let mut terms: Vec<&crate::app::Seen> = watch
        .world
        .values()
        .filter(|seen| seen.info.running && !deep.owns(seen.info.id.as_str()))
        .collect();
    terms.sort_by(|a, b| a.info.id.cmp(&b.info.id));

    for seen in terms {
        let t = &seen.info;
        let space_id = t
            .labels
            .get(crate::deep::SPACE_LABEL)
            .map(String::as_str)
            .unwrap_or("");
        let space_name = crate::spaces::for_folder(&spaces_meta, &t.cwd)
            .map(|s| s.name.as_str())
            .unwrap_or("");
        if locked_space(&rows, space_id, space_name, &t.cwd).is_some() {
            continue;
        }
        let job_at = watch.jobs.get(t.id.as_str()).copied();
        let where_label = if !space_name.is_empty() {
            space_name.to_string()
        } else if !space_id.is_empty() {
            space_id.to_string()
        } else {
            String::new()
        };

        let mut facts = FactMap::new();
        facts.insert("subject".into(), json!("terminal"));
        facts.insert("rules.waiting".into(), json!(rules_waiting));
        facts.insert("terminal.id".into(), json!(t.id.as_str()));
        facts.insert("terminal.name".into(), json!(t.name));
        facts.insert("terminal.agent".into(), json!(agent_of(t)));
        facts.insert("terminal.where".into(), json!(where_label));
        facts.insert("terminal.state".into(), json!(status_word(t.status)));
        facts.insert(
            "terminal.minutesInState".into(),
            json!(minutes(seen.since, now)),
        );
        facts.insert(
            "terminal.hasTask".into(),
            json!(held_terms.contains(t.id.as_str())),
        );
        facts.insert(
            "terminal.hasClaim".into(),
            json!(claimed_terms.contains(t.id.as_str())),
        );
        facts.insert("terminal.nixaJob".into(), json!(job_at.is_some()));
        facts.insert(
            "terminal.minutesSinceSent".into(),
            job_at
                .map(|j| json!(minutes(j, now)))
                .unwrap_or(Value::Null),
        );
        facts.insert(
            "terminal.startedAfterSend".into(),
            json!(job_at.is_some_and(|j| seen.since > j)),
        );

        out.push(DroneSubject {
            kind: "terminal",
            key: format!("terminal {}", t.id),
            facts,
        });
    }

    Ok(out)
}
