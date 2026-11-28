use super::map::{Agent, St};
use super::office::{Room, Rooms};
use crate::layout;
use crate::screens::agents::model;
use crate::terms;
use crate::workspace::Workspace;
use nixa_protocol::{Status, TermId, TermInfo};
use std::collections::{BTreeMap, BTreeSet};

pub fn st(status: Option<Status>) -> St {
    match status {
        Some(Status::Working) => St::Working,
        Some(Status::Blocked) | Some(Status::Error) => St::Blocked,
        Some(Status::Done) => St::Done,
        Some(Status::Idle) | None => St::Idle,
    }
}

pub fn is_agent(term: &TermInfo) -> bool {
    term.status.is_some() || !terms::is_shell(&term.cmd)
}

pub const PERSONA_LABEL: &str = "nixa.name";

pub fn label_of(term: &TermInfo) -> String {
    model::label(
        &term.name,
        term.title.as_deref(),
        &terms::program(&term.cmd),
    )
}

pub fn agent_of(term: &TermInfo, numbers: &BTreeMap<TermId, u32>) -> Agent {
    Agent {
        id: term.id.as_str().to_string(),
        title: label_of(term),
        program: terms::program(&term.cmd),
        mono: terms::monogram(&term.cmd),
        status: st(term.status),
        parent: term.parent.as_ref().map(|p| p.as_str().to_string()),
        seq: term.seq,
        number: numbers.get(&term.id).copied(),
        persona: term
            .labels
            .get(PERSONA_LABEL)
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty()),
    }
}

pub struct Split {
    pub agents: Vec<Agent>,
    pub deep: Vec<Agent>,
}

pub fn split(
    terms: &BTreeMap<TermId, TermInfo>,
    here: &BTreeSet<TermId>,
    placed: &BTreeSet<TermId>,
    numbers: &BTreeMap<TermId, u32>,
) -> Split {
    let live: BTreeMap<&TermId, &TermInfo> = terms.iter().filter(|(_, t)| t.running).collect();
    let mut keep: BTreeSet<&TermId> = live
        .values()
        .filter(|t| {
            is_agent(t) && (t.status.is_some() || t.parent.is_some() || here.contains(&t.id))
        })
        .map(|t| &t.id)
        .collect();
    let base: Vec<&TermId> = keep.iter().copied().collect();
    for id in base {
        let mut seen = BTreeSet::new();
        let mut parent = live.get(id).and_then(|t| t.parent.as_ref());
        while let Some(p) = parent {
            if !seen.insert(p) {
                break;
            }
            let Some((key, term)) = live.get_key_value(p) else {
                break;
            };
            keep.insert(key);
            parent = term.parent.as_ref();
        }
    }
    let mut agents = Vec::new();
    let mut deep_here = Vec::new();
    let mut deep_loose = Vec::new();
    for id in keep {
        let Some(term) = live.get(id) else {
            continue;
        };
        let agent = agent_of(term, numbers);
        if model::is_deep_term(&term.labels, &agent.title) {
            if here.contains(id) {
                deep_here.push(agent);
            } else if !placed.contains(id) {
                deep_loose.push(agent);
            }
        } else {
            agents.push(agent);
        }
    }
    Split {
        agents,
        deep: if deep_here.is_empty() {
            deep_loose
        } else {
            deep_here
        },
    }
}

pub fn build(ws: &Workspace) -> Split {
    let active = ws.spaces.active.as_deref();
    let mut here = BTreeSet::new();
    let mut placed = BTreeSet::new();
    for tab in &ws.tabs.list {
        for id in layout::leaves(&tab.root) {
            if Some(tab.space.as_str()) == active {
                here.insert(id.clone());
            }
            placed.insert(id);
        }
    }
    split(&ws.terms, &here, &placed, &ws.tabs.numbers)
}

pub fn space_of_terms(ws: &Workspace) -> BTreeMap<TermId, String> {
    let mut out = BTreeMap::new();
    for tab in &ws.tabs.list {
        for id in layout::leaves(&tab.root) {
            out.insert(id, tab.space.clone());
        }
    }
    out
}

pub fn home_space(
    id: &TermId,
    terms: &BTreeMap<TermId, TermInfo>,
    places: &BTreeMap<TermId, String>,
) -> Option<String> {
    let mut seen = BTreeSet::new();
    let mut at = Some(id);
    while let Some(now) = at {
        if !seen.insert(now) {
            break;
        }
        if let Some(space) = places.get(now) {
            return Some(space.clone());
        }
        at = terms.get(now).and_then(|t| t.parent.as_ref());
    }
    None
}

pub fn rooms_from(
    terms: &BTreeMap<TermId, TermInfo>,
    places: &BTreeMap<TermId, String>,
    spaces: &[(String, String, usize)],
    active: Option<&str>,
    numbers: &BTreeMap<TermId, u32>,
) -> Rooms {
    let mut list: Vec<Room> = spaces
        .iter()
        .map(|(id, name, color)| Room {
            id: id.clone(),
            name: name.clone(),
            color: *color,
            ..Room::default()
        })
        .collect();
    let mut loose_deep = Vec::new();
    for term in terms.values().filter(|t| t.running && is_agent(t)) {
        let placed = places.contains_key(&term.id);
        if !(placed || term.status.is_some() || term.parent.is_some()) {
            continue;
        }
        let agent = agent_of(term, numbers);
        let deep = model::is_deep_term(&term.labels, &agent.title);
        let space = home_space(&term.id, terms, places)
            .filter(|s| list.iter().any(|r| &r.id == s))
            .or_else(|| (!deep).then(|| active.map(str::to_string)).flatten());
        match (space, deep) {
            (Some(s), _) => {
                if let Some(room) = list.iter_mut().find(|r| r.id == s) {
                    if deep {
                        room.deep.push(agent);
                    } else {
                        room.agents.push(agent);
                    }
                }
            }
            (None, true) => loose_deep.push(agent),
            (None, false) => {}
        }
    }
    for room in &mut list {
        room.agents.sort_by(|a, b| a.seq.cmp(&b.seq).then_with(|| a.id.cmp(&b.id)));
    }
    Rooms {
        list,
        loose_deep,
        active: active.map(str::to_string),
    }
}

pub fn rooms(ws: &Workspace) -> Rooms {
    let spaces: Vec<(String, String, usize)> = ws
        .spaces
        .spaces
        .iter()
        .map(|s| (s.id.clone(), s.name.clone(), s.color))
        .collect();
    rooms_from(
        &ws.terms,
        &space_of_terms(ws),
        &spaces,
        ws.spaces.active.as_deref(),
        &ws.tabs.numbers,
    )
}
