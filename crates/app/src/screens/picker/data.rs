use super::rows::{PickAgent, PickSpace, PickTab, Snapshot, Target};
use super::view::PickerEvent;
use crate::workspace::Workspace;
use crate::{layout, terms};
use gpui::{App, Context};
use nixa_protocol::{TermId, TermInfo};
use std::collections::{BTreeMap, BTreeSet};

pub fn is_agent(term: &TermInfo) -> bool {
    term.running && (!terms::is_shell(&term.cmd) || term.status.is_some())
}

pub fn team(terms: &BTreeMap<TermId, TermInfo>, roots: &[TermId]) -> usize {
    let mut seen: BTreeSet<TermId> = roots.iter().cloned().collect();
    let mut queue: Vec<TermId> = roots.to_vec();
    let mut count = 0;
    while let Some(id) = queue.pop() {
        for term in terms.values() {
            if term.running && term.parent.as_ref() == Some(&id) && seen.insert(term.id.clone()) {
                count += 1;
                queue.push(term.id.clone());
            }
        }
    }
    count
}

pub fn snapshot(ws: &Workspace, cx: &App) -> Snapshot {
    let t = nixa_ui::theme(cx);
    let mut snap = Snapshot::default();
    for space in &ws.spaces.spaces {
        let space_active = ws.spaces.active.as_deref() == Some(space.id.as_str());
        let active_tab = ws.tabs.active.get(&space.id);
        let mut tabs = Vec::new();
        for tab in ws.space_tabs(&space.id) {
            let tab_active = active_tab == Some(&tab.id);
            let tab_name = ws.tab_name(tab);
            let leaves = layout::leaves(&tab.root);
            let agents: Vec<&TermInfo> = leaves
                .iter()
                .filter_map(|id| ws.terms.get(id))
                .filter(|term| is_agent(term))
                .collect();
            for term in &agents {
                snap.agents.push(PickAgent {
                    term: term.id.clone(),
                    tab: tab.id.clone(),
                    name: terms::term_name(term),
                    title: term.title.clone().unwrap_or_default(),
                    status: term.status,
                    pinned: ws.hub.agents.read(cx).pins.get(&term.id).map_or_else(
                        || term.labels.get("pinned").is_some_and(|value| !value.is_empty()),
                        |(pinned, _)| *pinned,
                    ),
                    focused: space_active && tab_active && tab.focus == term.id,
                    space_name: space.name.clone(),
                    tab_name: tab_name.clone(),
                });
            }
            let roots: Vec<TermId> = agents.iter().map(|a| a.id.clone()).collect();
            tabs.push(PickTab {
                id: tab.id.clone(),
                name: tab_name,
                active: tab_active,
                status: ws.tab_term(tab).and_then(|term| term.status),
                agents: agents.iter().map(|a| terms::term_name(a)).collect(),
                team: team(&ws.terms, &roots),
            });
        }
        let color = if space.name == "~" {
            t.dim
        } else {
            crate::spaces::color(t, space.color)
        };
        snap.spaces.push(PickSpace {
            id: space.id.clone(),
            name: space.name.clone(),
            color,
            active: space_active,
            tabs,
        });
    }
    snap
}

pub fn apply(ws: &mut Workspace, event: &PickerEvent, cx: &mut Context<Workspace>) {
    match event {
        PickerEvent::Go(Target::Tab(id)) => ws.select_tab(id, cx),
        PickerEvent::Go(Target::Space(id)) => ws.select_space(id, cx),
        PickerEvent::Go(Target::Agent { tab, term }) => {
            ws.select_tab(tab, cx);
            ws.focus_pane(tab, term, cx);
        }
        PickerEvent::Go(Target::NewTabHere(id)) => {
            ws.select_space(id, cx);
            ws.new_tab(Vec::new(), None, cx);
        }
        PickerEvent::Go(Target::NewTab) => ws.new_tab(Vec::new(), None, cx),
        PickerEvent::Go(Target::NewSpace) => ws.pick_space_folder(cx),
        PickerEvent::CloseTab(id) => ws.close_tab(id, cx),
        PickerEvent::Close => {}
    }
}
