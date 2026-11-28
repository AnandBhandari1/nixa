use super::model::{self, Agent, Away, Input, Kid, Loose, OtherTab};
use crate::layout;
use crate::terms;
use crate::spaces;
use crate::tabs;
use nixa_ui::metrics::SIDEBAR_NARROW;
use crate::workspace::{EngineState, Workspace};
use nixa_protocol::{TermId, TermInfo};
use std::collections::{BTreeMap, BTreeSet};

pub const PIN_LABEL: &str = "pinned";

pub fn is_agent(term: &TermInfo) -> bool {
    term.status.is_some() || !terms::is_shell(&term.cmd)
}

pub fn pinned(term: &TermInfo) -> bool {
    term.labels.get(PIN_LABEL).is_some_and(|v| !v.is_empty())
}

pub fn term_label(term: &TermInfo) -> String {
    model::label(
        &term.name,
        term.title.as_deref(),
        &terms::program(&term.cmd),
    )
}

pub fn build(ws: &Workspace, away: &BTreeSet<String>, pins: &BTreeMap<TermId, bool>) -> Input {
    let active = ws.spaces.active.clone();
    let order: BTreeMap<&str, usize> = ws
        .spaces
        .spaces
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();
    let mut agents: Vec<(usize, Agent)> = Vec::new();
    let mut others = Vec::new();
    let mut away_rows = Vec::new();
    for tab in &ws.tabs.list {
        let Some(space) = spaces::find(&ws.spaces, &tab.space) else {
            continue;
        };
        let here = active.as_deref() == Some(tab.space.as_str());
        let terms: Vec<&TermInfo> = layout::leaves(&tab.root)
            .into_iter()
            .filter_map(|id| ws.terms.get(&id))
            .collect();
        if away.contains(&tab.id) {
            if here {
                away_rows.push(Away {
                    tab: tab.id.clone(),
                    name: ws.tab_name(tab),
                });
            }
            continue;
        }
        let active_tab = ws.tabs.active.get(&tab.space) == Some(&tab.id);
        let mut has_agents = false;
        for term in terms.iter().copied().filter(|t| is_agent(t)) {
                has_agents = true;
                let label = term_label(term);
                let deep = model::is_deep_term(&term.labels, &label);
                agents.push((
                    order.get(tab.space.as_str()).copied().unwrap_or(usize::MAX),
                    Agent {
                        tab: tab.id.clone(),
                        term: term.id.clone(),
                        space: space.id.clone(),
                        space_name: space.name.clone(),
                        color: space.color,
                        deep,
                        label,
                        program: terms::program(&term.cmd),
                        cmd: term.cmd.clone(),
                        status: term.status,
                        running: term.running,
                        pinned: deep || pins.get(&term.id).copied().unwrap_or_else(|| pinned(term)),
                        focused: here && active_tab && tab.focus == term.id,
                        seq: term.seq,
                        number: ws.tabs.numbers.get(&term.id).copied(),
                    },
                ));
        }
        if here && !has_agents {
            others.push(OtherTab {
                tab: tab.id.clone(),
                name: ws.tab_name(tab),
                cmd: terms.first().map(|t| t.cmd.clone()).unwrap_or_default(),
                active: active_tab,
                preview: false,
                zoomed: tabs::zoomed(tab).is_some(),
            });
        }
    }
    agents.sort_by_key(|(i, _)| *i);
    let kids = ws
        .terms
        .values()
        .filter_map(|t| {
            Some(Kid {
                term: t.id.clone(),
                parent: t.parent.clone()?,
                tab: tabs::tab_of(&ws.tabs, &t.id).map(|tab| tab.id.clone()),
                name: term_label(t),
                cmd: t.cmd.clone(),
                status: t.status,
                running: t.running,
            })
        })
        .collect();
    let loose = ws
        .loose()
        .into_iter()
        .map(|t| Loose {
            term: t.id.clone(),
            name: terms::term_name(t),
            cmd: t.cmd.clone(),
        })
        .collect();
    Input {
        active_space: active,
        agents: agents.into_iter().map(|(_, a)| a).collect(),
        kids,
        others,
        away: away_rows,
        loose,
        tabs: ws
            .tabs
            .list
            .iter()
            .map(|t| (t.id.clone(), t.space.clone()))
            .collect(),
        narrow: ws.sidebar_w < SIDEBAR_NARROW,
        ready: ws.engine == EngineState::Up,
    }
}
