use crate::id::TermId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Working,
    Blocked,
    Done,
    Idle,
    Error,
}

impl Status {
    pub const ALL: [Status; 5] = [
        Status::Working,
        Status::Blocked,
        Status::Done,
        Status::Idle,
        Status::Error,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Working => "working",
            Status::Blocked => "blocked",
            Status::Done => "done",
            Status::Idle => "idle",
            Status::Error => "error",
        }
    }

    pub fn needs_attention(self) -> bool {
        matches!(self, Status::Blocked | Status::Done | Status::Error)
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Status {
    type Err = String;

    fn from_str(s: &str) -> Result<Status, String> {
        Status::ALL
            .into_iter()
            .find(|st| st.as_str() == s)
            .ok_or_else(|| format!("unknown status {s:?}, use working, blocked, done, idle or error"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    #[serde(default)]
    pub agent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermProcs {
    pub id: TermId,
    pub procs: Vec<ProcInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermInfo {
    pub id: TermId,
    pub name: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub parent: Option<TermId>,
    #[serde(default)]
    pub children: Vec<TermId>,
    #[serde(default)]
    pub cmd: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub pid: Option<u32>,
    pub running: bool,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub status: Option<Status>,
    #[serde(default)]
    pub status_note: Option<String>,
    #[serde(default)]
    pub keep: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub title: Option<String>,
    pub rows: u16,
    pub cols: u16,
    pub created_ms: u64,
    #[serde(default)]
    pub seq: u64,
}

pub fn check_start_parent(id: &TermId, parent: Option<&TermId>) -> Result<(), String> {
    if parent.is_some() {
        return Err(format!(
            "Cannot use {id} as a parent: it already has a parent. Only one level of sub-agents is allowed."
        ));
    }
    Ok(())
}
