use crate::id::TermId;
use crate::term::{Status, TermInfo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Created {
        term: TermInfo,
    },
    Status {
        id: TermId,
        #[serde(default)]
        status: Option<Status>,
        #[serde(default)]
        note: Option<String>,
    },
    Exit {
        id: TermId,
        #[serde(default)]
        code: Option<i32>,
    },
    Respawned {
        id: TermId,
        #[serde(default)]
        pid: Option<u32>,
    },
    Removed {
        id: TermId,
    },
    ParentCleared {
        id: TermId,
        parent: TermId,
    },
    Label {
        id: TermId,
        labels: BTreeMap<String, String>,
    },
    Renamed {
        id: TermId,
        name: String,
    },
    Title {
        id: TermId,
        #[serde(default)]
        title: Option<String>,
    },
    Resized {
        id: TermId,
        rows: u16,
        cols: u16,
    },
    Draft {
        id: TermId,
        active: bool,
    },
    OutputChanged {
        id: TermId,
        seq: u64,
    },
    ChildNote {
        id: TermId,
        child: TermId,
        #[serde(default)]
        status: Option<Status>,
        #[serde(default)]
        exited: bool,
        note: String,
    },
}

impl Event {
    pub fn term_id(&self) -> &TermId {
        match self {
            Event::Created { term } => &term.id,
            Event::Status { id, .. }
            | Event::Exit { id, .. }
            | Event::Respawned { id, .. }
            | Event::Removed { id }
            | Event::ParentCleared { id, .. }
            | Event::Label { id, .. }
            | Event::Renamed { id, .. }
            | Event::Title { id, .. }
            | Event::Resized { id, .. }
            | Event::Draft { id, .. }
            | Event::OutputChanged { id, .. }
            | Event::ChildNote { id, .. } => id,
        }
    }

    pub fn is_output(&self) -> bool {
        matches!(self, Event::OutputChanged { .. })
    }
}
