use gpui::Hsla;
use nixa_protocol::{Status, TermId};

#[derive(Debug, Clone, PartialEq)]
pub struct PickTab {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub status: Option<Status>,
    pub agents: Vec<String>,
    pub team: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickSpace {
    pub id: String,
    pub name: String,
    pub color: Hsla,
    pub active: bool,
    pub tabs: Vec<PickTab>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickAgent {
    pub term: TermId,
    pub tab: String,
    pub name: String,
    pub title: String,
    pub status: Option<Status>,
    pub pinned: bool,
    pub focused: bool,
    pub space_name: String,
    pub tab_name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub spaces: Vec<PickSpace>,
    pub agents: Vec<PickAgent>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Head(usize),
    Tab(usize, usize),
    Quiet(usize),
    NewTabHere(usize),
    Agents,
    Agent(usize),
    NewTab,
    NewSpace,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Tab(String),
    Space(String),
    Agent { tab: String, term: TermId },
    NewTabHere(String),
    NewTab,
    NewSpace,
}

pub fn rank(status: Option<Status>) -> u8 {
    match status {
        Some(Status::Blocked) => 5,
        Some(Status::Error) => 4,
        Some(Status::Working) => 3,
        Some(Status::Done) => 2,
        Some(Status::Idle) => 1,
        None => 0,
    }
}

pub fn space_status(space: &PickSpace) -> Option<Status> {
    space
        .tabs
        .iter()
        .map(|t| t.status)
        .max_by_key(|s| rank(*s))
        .flatten()
}

pub fn busy(status: Option<Status>) -> bool {
    status.is_some()
}

pub fn matches(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

fn tab_text(space: &PickSpace, tab: &PickTab) -> String {
    let mut parts = vec![space.name.clone(), tab.name.clone()];
    parts.extend(tab.agents.iter().cloned());
    parts.join(" ")
}

fn agent_text(agent: &PickAgent) -> String {
    [
        agent.name.as_str(),
        agent.title.as_str(),
        agent.space_name.as_str(),
        agent.tab_name.as_str(),
        &agent.term.to_string(),
    ]
    .join(" ")
}

pub fn rows(snap: &Snapshot, filter: &str, show_quiet: bool) -> Vec<Row> {
    let groups: Vec<(usize, Vec<usize>)> = snap
        .spaces
        .iter()
        .enumerate()
        .map(|(si, space)| {
            let tabs = space
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, tab)| matches(&tab_text(space, tab), filter))
                .map(|(ti, _)| ti)
                .collect();
            (si, tabs)
        })
        .filter(|(si, tabs): &(usize, Vec<usize>)| {
            let space = &snap.spaces[*si];
            !tabs.is_empty() || (space.tabs.is_empty() && matches(&space.name, filter))
        })
        .collect();
    let (mut loud, quiet): (Vec<_>, Vec<_>) = groups
        .into_iter()
        .partition(|(si, _)| {
            let space = &snap.spaces[*si];
            space.tabs.is_empty() || busy(space_status(space))
        });
    loud.sort_by_key(|(si, _)| std::cmp::Reverse(rank(space_status(&snap.spaces[*si]))));
    let mut out = Vec::new();
    let add = |out: &mut Vec<Row>, (si, tabs): &(usize, Vec<usize>)| {
        out.push(Row::Head(*si));
        out.extend(tabs.iter().map(|ti| Row::Tab(*si, *ti)));
        if snap.spaces[*si].tabs.is_empty() {
            out.push(Row::NewTabHere(*si));
        }
    };
    for group in &loud {
        add(&mut out, group);
    }
    if !quiet.is_empty() {
        out.push(Row::Quiet(quiet.len()));
        if show_quiet || !filter.trim().is_empty() {
            for group in &quiet {
                add(&mut out, group);
            }
        }
    }
    let agents: Vec<usize> = snap
        .agents
        .iter()
        .enumerate()
        .filter(|(_, a)| matches(&agent_text(a), filter))
        .map(|(i, _)| i)
        .collect();
    if !agents.is_empty() {
        out.push(Row::Agents);
        out.extend(agents.into_iter().map(Row::Agent));
    }
    out.push(Row::NewTab);
    out.push(Row::NewSpace);
    out
}

pub fn pickable(rows: &[Row]) -> Vec<Row> {
    rows.iter()
        .filter(|r| !matches!(r, Row::Agents))
        .cloned()
        .collect()
}

pub fn start(snap: &Snapshot, picks: &[Row]) -> usize {
    let here = picks.iter().position(|r| match r {
        Row::Tab(si, ti) => snap.spaces[*si].active && snap.spaces[*si].tabs[*ti].active,
        _ => false,
    });
    let head = || {
        picks
            .iter()
            .position(|r| matches!(r, Row::Head(si) if snap.spaces[*si].active))
    };
    here.or_else(head).unwrap_or(0)
}

pub fn target(snap: &Snapshot, row: &Row) -> Option<Target> {
    match row {
        Row::Head(si) => snap.spaces.get(*si).map(|s| Target::Space(s.id.clone())),
        Row::Tab(si, ti) => snap
            .spaces
            .get(*si)
            .and_then(|s| s.tabs.get(*ti))
            .map(|t| Target::Tab(t.id.clone())),
        Row::Agent(i) => snap.agents.get(*i).map(|a| Target::Agent {
            tab: a.tab.clone(),
            term: a.term.clone(),
        }),
        Row::NewTabHere(si) => snap.spaces.get(*si).map(|s| Target::NewTabHere(s.id.clone())),
        Row::NewTab => Some(Target::NewTab),
        Row::NewSpace => Some(Target::NewSpace),
        Row::Quiet(_) | Row::Agents => None,
    }
}

pub fn quiet_label(count: usize, open: bool) -> String {
    format!(
        "earlier: {count} quiet space{} {}",
        if count > 1 { "s" } else { "" },
        if open { "▴" } else { "▾" }
    )
}
