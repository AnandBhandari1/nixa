use crate::id::TermId;
use crate::keys::Key;
use crate::term::Status;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateSpec {
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    pub name: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub parent: Option<TermId>,
    pub env: BTreeMap<String, String>,
    pub rows: Option<u16>,
    pub cols: Option<u16>,
    pub keep: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadSource {
    #[default]
    Screen,
    History,
}

/// Engine state the chat brain checked before archiving an ended terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndedSnapshot {
    pub space_label: Option<String>,
    pub window_space_label: Option<String>,
    pub cwd: String,
    pub seq: u64,
    pub exit_code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitFor {
    Idle,
    Status {
        status: Status,
    },
    Text {
        pattern: String,
        #[serde(default)]
        regex: bool,
    },
    Children,
    ChildAny,
    ChildAll,
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Ping,
    List {
        #[serde(default)]
        parent: Option<TermId>,
        #[serde(default)]
        label: Option<String>,
    },
    Get {
        id: TermId,
    },
    Procs,
    ChildReports {
        id: TermId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reader: Option<TermId>,
    },
    ChildReport {
        id: TermId,
        #[serde(default)]
        parent: Option<TermId>,
        text: String,
    },
    ChildOutput {
        id: TermId,
        child: TermId,
    },
    ClearParent {
        id: TermId,
    },
    Create {
        #[serde(flatten)]
        spec: CreateSpec,
    },
    Kill {
        id: TermId,
    },
    Remove {
        id: TermId,
    },
    RemoveEnded {
        id: TermId,
        expected: EndedSnapshot,
    },
    Read {
        id: TermId,
        #[serde(default)]
        source: ReadSource,
        #[serde(default)]
        lines: Option<usize>,
    },
    Snapshot {
        id: TermId,
        #[serde(default)]
        scroll: usize,
        #[serde(default)]
        since_seq: Option<u64>,
    },
    Send {
        id: TermId,
        text: String,
        #[serde(default)]
        submit: bool,
        #[serde(default)]
        timeout_ms: Option<u64>,
        #[serde(default)]
        force: bool,
    },
    Input {
        id: TermId,
        text: String,
    },
    Keys {
        id: TermId,
        keys: Vec<Key>,
    },
    Interrupt {
        id: TermId,
    },
    Resize {
        id: TermId,
        rows: u16,
        cols: u16,
    },
    LabelSet {
        id: TermId,
        key: String,
        #[serde(default)]
        value: Option<String>,
    },
    Rename {
        id: TermId,
        name: String,
    },
    StatusSet {
        id: TermId,
        #[serde(default)]
        status: Option<Status>,
        #[serde(default)]
        note: Option<String>,
    },
    Wait {
        id: TermId,
        until: WaitFor,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    Keep {
        id: TermId,
        on: bool,
    },
    Subscribe {
        #[serde(default)]
        ids: Vec<TermId>,
        #[serde(default)]
        output: bool,
    },
    Shutdown,
}

impl Request {
    pub fn term_id(&self) -> Option<&TermId> {
        match self {
            Request::Get { id }
            | Request::ChildReports { id, .. }
            | Request::ChildReport { id, .. }
            | Request::ChildOutput { id, .. }
            | Request::ClearParent { id }
            | Request::Kill { id }
            | Request::Remove { id }
            | Request::RemoveEnded { id, .. }
            | Request::Read { id, .. }
            | Request::Snapshot { id, .. }
            | Request::Send { id, .. }
            | Request::Input { id, .. }
            | Request::Keys { id, .. }
            | Request::Interrupt { id }
            | Request::Resize { id, .. }
            | Request::LabelSet { id, .. }
            | Request::Rename { id, .. }
            | Request::StatusSet { id, .. }
            | Request::Wait { id, .. }
            | Request::Keep { id, .. } => Some(id),
            Request::Ping
            | Request::Procs
            | Request::List { .. }
            | Request::Create { .. }
            | Request::Subscribe { .. }
            | Request::Shutdown => None,
        }
    }
}
