use crate::tasks::{self, BlockedOn, TaskStatus};
use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, TransactionBehavior};

pub const KEY: &str = "waits_on:";
pub const MAX_WAITS: usize = 10;

fn key(id: i64) -> String {
    format!("{KEY}{id}")
}

pub fn parse(text: &str) -> Vec<i64> {
    let mut ids: Vec<i64> = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|w| w.trim().trim_start_matches('#').parse().ok())
        .filter(|n| *n > 0)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn join(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

pub fn waits_of(conn: &Connection, id: i64) -> Vec<i64> {
    tasks::task_meta(conn, &key(id)).ok().flatten().map(|v| parse(&v)).unwrap_or_default()
}

fn closed(conn: &Connection, id: i64) -> bool {
    tasks::get_task(conn, id).ok().flatten().is_none_or(|t| t.status.is_closed())
}

pub fn open_waits(conn: &Connection, id: i64) -> Vec<i64> {
    waits_of(conn, id).into_iter().filter(|on| !closed(conn, *on)).collect()
}

pub fn all_waits(conn: &Connection) -> Vec<(i64, Vec<i64>)> {
    let plain: String = KEY.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    tasks::task_meta_list(conn, &plain)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| Some((k.strip_prefix(KEY)?.parse().ok()?, parse(&v))))
        .filter(|(_, on): &(i64, Vec<i64>)| !on.is_empty())
        .collect()
}

fn reaches(conn: &Connection, from: i64, target: i64) -> bool {
    let mut seen = vec![from];
    let mut next = vec![from];
    while let Some(id) = next.pop() {
        for on in waits_of(conn, id) {
            if on == target {
                return true;
            }
            if !seen.contains(&on) {
                seen.push(on);
                next.push(on);
            }
        }
    }
    false
}

pub fn set_waits(conn: &Connection, id: i64, on: &[i64]) -> Result<Vec<i64>> {
    if !conn.is_autocommit() {
        return save(conn, id, on);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let saved = save(&tx, id, on)?;
    tx.commit()?;
    Ok(saved)
}

fn save(conn: &Connection, id: i64, on: &[i64]) -> Result<Vec<i64>> {
    let task = tasks::must_task(conn, id)?;
    if task.status.is_closed() {
        bail!("Task #{id} is {}, so it waits on nothing. Nothing was changed.", task.status.as_str());
    }
    let mut ids = on.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        tasks::remove_task_meta(conn, &key(id))?;
        return Ok(ids);
    }
    if ids.len() > MAX_WAITS {
        bail!("A task can wait on at most {MAX_WAITS} tasks. Nothing was changed.");
    }
    for other in &ids {
        if *other == id {
            bail!("Task #{id} cannot wait on itself. Nothing was changed.");
        }
        if !tasks::has_task(conn, *other)? {
            bail!("There is no task #{other}. Nothing was changed.");
        }
        if reaches(conn, *other, id) {
            bail!("Task #{other} already waits on task #{id}, so this would be a loop. Nothing was changed.");
        }
    }
    tasks::set_task_meta(conn, &key(id), &join(&ids))?;
    Ok(ids)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleared {
    Kept,
    ToTodo,
    ToDoing,
}

pub fn clear_waits(conn: &Connection, id: i64) -> Result<Cleared> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let task = tasks::must_task(&tx, id)?;
    save(&tx, id, &[])?;
    let on_task = task.status == TaskStatus::Blocked && task.blocked_on == Some(BlockedOn::Task);
    let moved = match (on_task, task.terminal_id.is_empty()) {
        (true, true) => {
            tasks::set_status(&tx, id, TaskStatus::Todo, None, false)?;
            Cleared::ToTodo
        }
        (true, false) => {
            tasks::set_status(&tx, id, TaskStatus::Doing, None, false)?;
            Cleared::ToDoing
        }
        _ => Cleared::Kept,
    };
    tx.commit()?;
    Ok(moved)
}

pub fn waits_text(ids: &[i64]) -> String {
    ids.iter().map(|n| format!("task #{n}")).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;
    use crate::tasks::{Actor, AuthorKind, TaskInput, add_task, set_status};

    fn add(conn: &Connection, title: &str) -> i64 {
        let input = TaskInput { title: title.into(), ..TaskInput::default() };
        add_task(conn, &input, &Actor::new(AuthorKind::User, "user")).unwrap().id
    }

    #[test]
    fn links_are_saved_checked_and_closed_ones_drop_out() {
        let conn = temp_db("task-waits");
        let (a, b, c) = (add(&conn, "a"), add(&conn, "b"), add(&conn, "c"));
        assert_eq!(parse("#3, 2 2 x 0"), vec![2, 3]);
        assert_eq!(set_waits(&conn, a, &[c, b, b]).unwrap(), vec![b, c]);
        assert_eq!(waits_of(&conn, a), vec![b, c]);
        assert!(set_waits(&conn, a, &[a]).unwrap_err().to_string().contains("itself"));
        assert!(set_waits(&conn, a, &[999]).unwrap_err().to_string().contains("no task #999"));
        assert!(set_waits(&conn, c, &[a]).unwrap_err().to_string().contains("loop"));
        set_status(&conn, b, TaskStatus::Done, None, false).unwrap();
        assert_eq!(open_waits(&conn, a), vec![c]);
        assert_eq!(all_waits(&conn), vec![(a, vec![b, c])]);
        assert!(set_waits(&conn, b, &[c]).is_err(), "a closed task waits on nothing");
        assert_eq!(tasks::next_task(&conn, "").unwrap().map(|t| t.id), Some(c));
        assert!(set_waits(&conn, a, &[]).unwrap().is_empty());
        assert!(waits_of(&conn, a).is_empty());
        assert_eq!(waits_text(&[4, 7]), "task #4, task #7");
    }
}
