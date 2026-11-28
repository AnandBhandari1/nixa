use crate::agents::{AgentState, REVIEWER_ROLE};
use crate::tasks::{set_task_meta, task_meta};
use anyhow::Result;
use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::LazyLock;

pub const HANDOFF_KEY: &str = "handoff:";
pub const MAX_HANDOFFS: u32 = 4;
pub const CUSTOM_PREFIX: &str = "custom:";
pub const PICKUP_ROLE: &str = "code-reviewer";
const SHOWN_FILES: usize = 6;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HandoffState {
    pub steps: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reviewer: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fixer: String,
    #[serde(skip_serializing_if = "is_false")]
    pub recheck: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub escalated: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub fails: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub rounds: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub back: String,
    #[serde(skip_serializing_if = "is_false")]
    pub auto: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSeat {
    pub id: String,
    pub title: String,
    pub role: String,
    pub space: String,
    pub state: AgentState,
    pub terminal: String,
    pub can_resume: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoffMove {
    Done,
    Pass { to: String, why: String, state: HandoffState },
    User { why: String },
}

pub fn handoff_key(task: i64) -> String {
    format!("{HANDOFF_KEY}{task}")
}

pub fn read_handoff_text(raw: &str) -> HandoffState {
    serde_json::from_str(raw).unwrap_or_default()
}

pub fn handoff_state(conn: &Connection, task: i64) -> Result<HandoffState> {
    Ok(task_meta(conn, &handoff_key(task))?.map(|raw| read_handoff_text(&raw)).unwrap_or_default())
}

pub fn set_handoff_state(conn: &Connection, task: i64, state: &HandoffState) -> Result<()> {
    set_task_meta(conn, &handoff_key(task), &serde_json::to_string(state)?)
}

pub fn in_chain(state: &HandoffState) -> bool {
    state.steps > 0
}

pub fn custom_ref(id: &str) -> String {
    format!("{CUSTOM_PREFIX}{id}")
}

pub fn custom_id_of(assigned: &str) -> Option<String> {
    assigned.strip_prefix(CUSTOM_PREFIX).map(str::trim).filter(|id| !id.is_empty()).map(str::to_string)
}

static NO_FINDINGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bNO FINDINGS\b").expect("valid pattern"));
static FINDINGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bFINDINGS:\s*(\d+)\b").expect("valid pattern"));
static NOT_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\.(?:md|mdx|mmd|txt|rst|log|bak|png|jpe?g|gif|webp|ico|pdf|mp3|wav|mp4|mov)$").expect("valid pattern")
});
static DOC_FOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|/)(?:docs?|\.trail|\.claude)/").expect("valid pattern"));

pub fn findings_of(text: &str) -> Option<u32> {
    if NO_FINDINGS.is_match(text) {
        return Some(0);
    }
    FINDINGS.captures_iter(text).last().and_then(|c| c[1].parse().ok())
}

pub fn is_code_file(path: &str) -> bool {
    !NOT_CODE.is_match(path) && !DOC_FOLDER.is_match(path)
}

pub fn named_in(file: &str, text: &str) -> bool {
    if file.is_empty() || text.is_empty() {
        return false;
    }
    if text.contains(file) {
        return true;
    }
    let name = file.rsplit('/').next().unwrap_or("");
    if name.chars().count() < 5 {
        return false;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(at) = text[from..].find(name) {
        let start = from + at;
        let end = start + name.len();
        let before_ok = text[..start].chars().next_back().is_none_or(|c| !(word(c) || c == '.' || c == '-'));
        let after_ok = text[end..].chars().next().is_none_or(|c| !(word(c) || c == '-'));
        if before_ok && after_ok {
            return true;
        }
        from = start + name.len().max(1);
    }
    false
}

pub fn review_files(changed: &[String], named: &[String], said: &str, strict: bool) -> Vec<String> {
    changed
        .iter()
        .filter(|f| is_code_file(f))
        .filter(|f| !strict || named.contains(f) || named_in(f, said))
        .cloned()
        .collect()
}

fn split_id(id: &str) -> (String, u64) {
    let letters: String = id.chars().take_while(|c| !c.is_ascii_digit()).collect();
    let number = id[letters.len()..].parse().unwrap_or(0);
    (letters, number)
}

fn seat_order(a: &ReviewSeat, b: &ReviewSeat) -> Ordering {
    let rank = |s: AgentState| match s {
        AgentState::Free => 0,
        AgentState::Closed => 1,
        AgentState::Working => 2,
        AgentState::Waiting => 3,
    };
    rank(a.state).cmp(&rank(b.state)).then_with(|| split_id(&a.id).cmp(&split_id(&b.id)))
}

pub fn pick_reviewer<'a>(seats: &'a [ReviewSeat], space: &str, terminal: &str, custom: &str, skip: &[String]) -> Option<&'a ReviewSeat> {
    let mut usable: Vec<&ReviewSeat> = seats
        .iter()
        .filter(|s| s.role == REVIEWER_ROLE && s.space == space && s.id != custom && !skip.contains(&s.id))
        .filter(|s| if s.state == AgentState::Closed { s.can_resume } else { terminal.is_empty() || s.terminal != terminal })
        .collect();
    usable.sort_by(|a, b| seat_order(a, b));
    usable.into_iter().next()
}

pub fn pickup_move(reviewer: &ReviewSeat, files: &[String], state: &HandoffState, back: &str) -> Option<HandoffMove> {
    if !state.reviewer.is_empty() || state.steps >= MAX_HANDOFFS || files.is_empty() {
        return None;
    }
    let shown = if files.len() <= SHOWN_FILES {
        files.join(", ")
    } else {
        format!("{} and {} more", files[..SHOWN_FILES].join(", "), files.len() - SHOWN_FILES)
    };
    let plural = if files.len() == 1 { "" } else { "s" };
    let to = custom_ref(&reviewer.id);
    let mut next = state.clone();
    next.steps += 1;
    next.reviewer = to.clone();
    if !back.is_empty() {
        next.back = back.to_string();
    }
    next.auto = true;
    next.recheck = false;
    next.fails = 0;
    Some(HandoffMove::Pass {
        why: format!(
            "Review pickup: {} changed code file{plural} ({shown}) of this task get one review by {} ({}), the review agent of this space.",
            files.len(),
            reviewer.title,
            reviewer.id
        ),
        to,
        state: next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;
    use crate::tasks::{add_task, Actor, AuthorKind, TaskInput};

    fn seat(id: &str, state: AgentState, terminal: &str, can_resume: bool) -> ReviewSeat {
        ReviewSeat { id: id.into(), title: format!("R {id}"), role: REVIEWER_ROLE.into(), space: "s1".into(), state, terminal: terminal.into(), can_resume }
    }

    #[test]
    fn findings_and_files() {
        assert_eq!(findings_of("all fine. NO FINDINGS"), Some(0));
        assert_eq!(findings_of("FINDINGS: 2 then FINDINGS: 3"), Some(3));
        assert_eq!(findings_of("nothing said"), None);
        assert!(is_code_file("src/a.rs"));
        assert!(!is_code_file("docs/a.rs"));
        assert!(!is_code_file("README.md"));
        assert!(named_in("crates/store/src/claims.rs", "I changed claims.rs today"));
        assert!(!named_in("src/claims.rs", "see myclaims.rs"));
        let changed = vec!["src/a.rs".to_string(), "src/notes.md".to_string(), "src/bbbbb.rs".to_string()];
        assert_eq!(review_files(&changed, &[], "", false), vec!["src/a.rs".to_string(), "src/bbbbb.rs".to_string()]);
        assert_eq!(review_files(&changed, &["src/a.rs".to_string()], "", true), vec!["src/a.rs".to_string()]);
    }

    #[test]
    fn picks_free_reviewer_first() {
        let seats = vec![
            seat("a12", AgentState::Free, "m5", true),
            seat("a3", AgentState::Free, "m6", true),
            seat("a1", AgentState::Closed, "", false),
            seat("a2", AgentState::Working, "m1", true),
        ];
        assert_eq!(pick_reviewer(&seats, "s1", "m1", "", &[]).unwrap().id, "a3");
        assert_eq!(pick_reviewer(&seats, "s1", "m6", "", &["a12".to_string()]).unwrap().id, "a2");
        assert!(pick_reviewer(&seats, "s2", "", "", &[]).is_none());
    }

    #[test]
    fn pickup_and_state_store() {
        let conn = temp_db("review");
        let task = add_task(&conn, &TaskInput { title: "t".into(), ..TaskInput::default() }, &Actor::new(AuthorKind::User, "Sam")).unwrap();
        let state = handoff_state(&conn, task.id).unwrap();
        assert_eq!(state, HandoffState::default());
        let reviewer = seat("a3", AgentState::Free, "m6", true);
        let files = vec!["src/a.rs".to_string()];
        let Some(HandoffMove::Pass { to, why, state: next }) = pickup_move(&reviewer, &files, &state, "custom:a1") else {
            panic!("expected a pass");
        };
        assert_eq!(to, "custom:a3");
        assert!(why.starts_with("Review pickup: 1 changed code file (src/a.rs)"));
        assert_eq!(next.steps, 1);
        assert!(next.auto);
        set_handoff_state(&conn, task.id, &next).unwrap();
        let saved = handoff_state(&conn, task.id).unwrap();
        assert_eq!(saved, next);
        assert!(pickup_move(&reviewer, &files, &saved, "").is_none());
        assert_eq!(custom_id_of("custom:a3").as_deref(), Some("a3"));
        assert_eq!(custom_id_of("member"), None);
    }
}
