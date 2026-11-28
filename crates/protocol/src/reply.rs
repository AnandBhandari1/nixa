use crate::event::Event;
use crate::id::TermId;
use crate::screen::Snapshot;
use crate::term::{Status, TermInfo};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    BadRequest,
    DraftBusy,
    Exited,
    Timeout,
    Unsupported,
    Internal,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ErrorCode::NotFound => "not_found",
            ErrorCode::BadRequest => "bad_request",
            ErrorCode::DraftBusy => "draft_busy",
            ErrorCode::Exited => "exited",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::Internal => "internal",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitOutcome {
    Met,
    Timeout,
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitResult {
    pub id: TermId,
    pub outcome: WaitOutcome,
    #[serde(default)]
    pub status: Option<Status>,
    #[serde(default)]
    pub matched: Option<String>,
    pub waited_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildReport {
    pub child: TermId,
    pub status: Option<Status>,
    pub exited: bool,
    pub note: String,
    #[serde(default)]
    pub current_job_result: bool,
    #[serde(default)]
    pub read: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptReport {
    pub id: TermId,
    #[serde(default)]
    pub status_before: Option<Status>,
    #[serde(default)]
    pub status_after: Option<Status>,
    pub running: bool,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub tail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Ok,
    Pong {
        version: u32,
        pid: u32,
    },
    Terms {
        terms: Vec<TermInfo>,
    },
    Term {
        term: TermInfo,
    },
    ChildReports {
        reports: Vec<ChildReport>,
    },
    Procs {
        terms: Vec<crate::term::TermProcs>,
    },
    Text {
        id: TermId,
        text: String,
    },
    Snapshot {
        snapshot: Snapshot,
    },
    Unchanged {
        id: TermId,
        seq: u64,
    },
    Sent {
        id: TermId,
        waited_ms: u64,
        forced: bool,
        #[serde(default)]
        started: Option<bool>,
        #[serde(default)]
        tail: String,
    },
    Interrupted {
        report: InterruptReport,
    },
    Waited {
        result: WaitResult,
    },
    Subscribed,
    Event {
        event: Event,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

impl Reply {
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Reply {
        Reply::Error {
            code,
            message: message.into(),
        }
    }

    pub fn not_found(id: &TermId) -> Reply {
        Reply::error(ErrorCode::NotFound, format!("no terminal {id}"))
    }
}
