use nixa_protocol::{Status, TermId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Priority,
    Grouped,
}

impl Sort {
    pub fn word(self) -> &'static str {
        match self {
            Sort::Priority => "priority",
            Sort::Grouped => "grouped",
        }
    }

    pub fn from_word(word: &str) -> Sort {
        if word == "grouped" {
            Sort::Grouped
        } else {
            Sort::Priority
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Priority => "Priority",
            Sort::Grouped => "Grouped by space",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Space,
    All,
}

impl Scope {
    pub fn word(self) -> &'static str {
        match self {
            Scope::Space => "space",
            Scope::All => "all",
        }
    }

    pub fn from_word(word: &str) -> Scope {
        if word == "all" {
            Scope::All
        } else {
            Scope::Space
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    Idle,
    Working,
    Done,
    Blocked,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Working => "working",
            State::Done => "done",
            State::Blocked => "blocked",
        }
    }

    pub fn section(self) -> &'static str {
        match self {
            State::Blocked => "Needs you",
            State::Done => "Done",
            State::Working => "Working",
            State::Idle => "Idle",
        }
    }
}

pub fn state_of(status: Option<Status>, running: bool) -> State {
    if !running {
        return State::Idle;
    }
    match status {
        Some(Status::Blocked | Status::Error) => State::Blocked,
        Some(Status::Done) => State::Done,
        Some(Status::Working) => State::Working,
        Some(Status::Idle) | None => State::Idle,
    }
}

pub fn kid_state(status: Option<Status>, running: bool) -> State {
    match state_of(status, running) {
        State::Done => State::Idle,
        other => other,
    }
}

pub fn kid_status(status: Option<Status>, running: bool) -> Option<Status> {
    match kid_state(status, running) {
        State::Blocked => Some(
            status
                .filter(|s| *s == Status::Error)
                .unwrap_or(Status::Blocked),
        ),
        State::Working => Some(Status::Working),
        _ => Some(Status::Idle),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub tab: String,
    pub term: TermId,
    pub space: String,
    pub space_name: String,
    pub color: usize,
    pub label: String,
    pub program: String,
    pub cmd: Vec<String>,
    pub status: Option<Status>,
    pub running: bool,
    pub pinned: bool,
    pub deep: bool,
    pub focused: bool,
    pub seq: u64,
    pub number: Option<u32>,
}

impl Agent {
    pub fn state(&self) -> State {
        state_of(self.status, self.running)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Kid {
    pub term: TermId,
    pub parent: TermId,
    pub tab: Option<String>,
    pub name: String,
    pub cmd: Vec<String>,
    pub status: Option<Status>,
    pub running: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OtherTab {
    pub tab: String,
    pub name: String,
    pub cmd: Vec<String>,
    pub active: bool,
    pub preview: bool,
    pub zoomed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Loose {
    pub term: TermId,
    pub name: String,
    pub cmd: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Away {
    pub tab: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Input {
    pub active_space: Option<String>,
    pub agents: Vec<Agent>,
    pub kids: Vec<Kid>,
    pub others: Vec<OtherTab>,
    pub away: Vec<Away>,
    pub loose: Vec<Loose>,
    pub tabs: Vec<(String, String)>,
    pub narrow: bool,
    pub ready: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Pass,
    Blank,
    Mid,
    Last,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubRow {
    pub kid: Kid,
    pub lines: Vec<Line>,
}

pub fn tree(kids: &[Kid], parent: &TermId) -> Vec<SubRow> {
    let mut seen = BTreeSet::new();
    seen.insert(parent.clone());
    walk(kids, parent, &[], &mut seen)
}

fn walk(kids: &[Kid], parent: &TermId, lead: &[Line], seen: &mut BTreeSet<TermId>) -> Vec<SubRow> {
    let mine: Vec<&Kid> = kids
        .iter()
        .filter(|k| &k.parent == parent && k.running && !seen.contains(&k.term))
        .collect();
    for k in &mine {
        seen.insert(k.term.clone());
    }
    let mut out = Vec::new();
    for (i, k) in mine.iter().enumerate() {
        let last = i + 1 == mine.len();
        let mut lines = lead.to_vec();
        lines.push(if last { Line::Last } else { Line::Mid });
        out.push(SubRow {
            kid: (*k).clone(),
            lines,
        });
        let mut next = lead.to_vec();
        next.push(if last { Line::Blank } else { Line::Pass });
        out.extend(walk(kids, &k.term, &next, seen));
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub term: TermId,
    pub state: State,
}

pub fn hold(prev: Option<Held>, agents: &[Agent]) -> Option<Held> {
    let cur = agents.iter().find(|a| a.focused)?;
    let same = prev.as_ref().is_some_and(|h| h.term == cur.term);
    if !same || cur.state() != State::Idle {
        return Some(Held {
            term: cur.term.clone(),
            state: cur.state(),
        });
    }
    prev
}

pub fn shown_state(agent: &Agent, held: Option<&Held>) -> State {
    let saw_done = held.is_some_and(|h| {
        h.term == agent.term && h.state == State::Done && agent.state() == State::Idle
    });
    if saw_done { State::Done } else { agent.state() }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Stamp {
    pub s: String,
    pub at: u64,
}

pub const FORGET_AFTER_MS: u64 = 30 * 86_400_000;

pub fn track(changed: &mut BTreeMap<String, Stamp>, agents: &[Agent], now: u64) -> bool {
    let mut dirty = false;
    for a in agents {
        let s = match a.state() {
            State::Done => State::Idle,
            other => other,
        }
        .word();
        let key = a.term.to_string();
        if changed.get(&key).is_some_and(|c| c.s == s) {
            continue;
        }
        changed.insert(
            key,
            Stamp {
                s: s.to_string(),
                at: now,
            },
        );
        dirty = true;
    }
    if dirty {
        let listed: BTreeSet<String> = agents.iter().map(|a| a.term.to_string()).collect();
        changed.retain(|k, c| listed.contains(k) || now.saturating_sub(c.at) <= FORGET_AFTER_MS);
    }
    dirty
}

pub fn duration(ms: u64) -> String {
    let m = ms / 60_000;
    if m < 1 {
        return "<1m".into();
    }
    if m < 60 {
        return format!("{m}m");
    }
    let h = m / 60;
    if h < 24 {
        format!("{h}h")
    } else {
        format!("{}d", h / 24)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub agent: Agent,
    pub urgency: State,
    pub subs: Vec<SubRow>,
    pub since: u64,
    pub name: String,
    pub detail: Option<String>,
    pub full: bool,
    pub compact: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: String,
    pub label: String,
    pub state: Option<State>,
    pub members: Vec<Entry>,
}

impl Group {
    pub fn top(&self) -> Option<State> {
        self.members.iter().map(|e| e.agent.state()).max()
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct View {
    pub groups: Vec<Group>,
    pub flat: Vec<Entry>,
    pub order: Vec<(String, TermId)>,
}

pub const ELSEWHERE: &str = "elsewhere";
pub const PINNED: &str = "pinned";
pub const OTHER_TABS: &str = "other-tabs";

fn digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

pub fn build(
    input: &Input,
    sort: Sort,
    scope: Scope,
    held: Option<&Held>,
    since: &BTreeMap<String, Stamp>,
) -> View {
    let cross = &input.agents;
    let active = input.active_space.as_deref();
    let in_active = |a: &Agent| Some(a.space.as_str()) == active;
    let elsewhere: Vec<&Agent> = match scope {
        Scope::Space => cross
            .iter()
            .filter(|a| !in_active(a) && a.state() == State::Blocked)
            .collect(),
        Scope::All => Vec::new(),
    };
    let mut list: Vec<&Agent> = match scope {
        Scope::All => cross.iter().collect(),
        Scope::Space => cross
            .iter()
            .filter(|a| in_active(a) || (a.pinned && !a.deep))
            .collect(),
    };
    let since_of = |a: &Agent| since.get(&a.term.to_string()).map_or(0, |s| s.at);
    let urgency: BTreeMap<TermId, State> = cross
        .iter()
        .map(|a| {
            let kids = tree(&input.kids, &a.term)
                .into_iter()
                .map(|s| kid_state(s.kid.status, s.kid.running));
            let top = kids.fold(shown_state(a, held), State::max);
            (a.term.clone(), top)
        })
        .collect();
    let urgent = |a: &Agent| urgency.get(&a.term).copied().unwrap_or(State::Idle);
    if sort == Sort::Priority {
        list.sort_by(|x, y| {
            urgent(y)
                .cmp(&urgent(x))
                .then(since_of(y).cmp(&since_of(x)))
                .then(y.seq.cmp(&x.seq))
        });
    }
    let is_elsewhere = |a: &Agent| elsewhere.iter().any(|e| e.term == a.term);
    let mut pinned: Vec<&Agent> = list
        .iter()
        .copied()
        .filter(|a| a.pinned && !is_elsewhere(a))
        .collect();
    pinned.sort_by_key(|a| !a.deep);
    let rest: Vec<&Agent> = list.iter().copied().filter(|a| !a.pinned).collect();
    let labels: Vec<&Agent> = list.iter().chain(elsewhere.iter()).copied().collect();
    let mut details: BTreeMap<TermId, Option<String>> = labels
        .iter()
        .map(|a| {
            let d = (!digits(&a.label)).then(|| a.label.clone());
            (a.term.clone(), d)
        })
        .collect();
    let mut seen: BTreeMap<(String, String), usize> = BTreeMap::new();
    for a in &labels {
        let d = details.get(&a.term).cloned().flatten().unwrap_or_default();
        *seen.entry((a.space_name.clone(), d)).or_default() += 1;
    }
    for a in &labels {
        let clash = seen
            .get(&(a.space_name.clone(), String::new()))
            .is_some_and(|n| *n > 1);
        if let Some(d) = details.get_mut(&a.term)
            && d.is_none()
            && clash
        {
            *d = Some(a.label.clone());
        }
    }
    let priority = sort == Sort::Priority;
    let entry = |a: &Agent, group_full: bool, compact: bool| {
        let detail = details.get(&a.term).cloned().flatten();
        let full = priority || a.pinned || group_full;
        let name = if full {
            if a.space_name.is_empty() {
                a.label.clone()
            } else {
                a.space_name.clone()
            }
        } else {
            match &detail {
                Some(d) => d.clone(),
                None if digits(&a.label) && !a.program.is_empty() => a.program.clone(),
                None => a.label.clone(),
            }
        };
        Entry {
            agent: a.clone(),
            urgency: urgent(a),
            subs: tree(&input.kids, &a.term),
            since: since_of(a),
            detail: if full { detail } else { None },
            name,
            full,
            compact,
        }
    };
    let mut groups: Vec<Group> = Vec::new();
    if !elsewhere.is_empty() {
        groups.push(Group {
            id: ELSEWHERE.into(),
            label: "Needs you elsewhere".into(),
            state: Some(State::Blocked),
            members: elsewhere.iter().map(|a| entry(a, true, true)).collect(),
        });
    }
    if !pinned.is_empty() {
        groups.push(Group {
            id: PINNED.into(),
            label: "Pinned".into(),
            state: None,
            members: pinned.iter().map(|a| entry(a, true, false)).collect(),
        });
    }
    for a in &rest {
        let (id, label, state) = if priority {
            let r = urgent(a);
            (
                format!("sec-{}", r.word()),
                r.section().to_string(),
                (r != State::Idle).then_some(r),
            )
        } else {
            (format!("sp-{}", a.space), a.space_name.clone(), None)
        };
        let e = entry(a, false, priority);
        match groups.last_mut() {
            Some(g) if g.id == id => g.members.push(e),
            _ => groups.push(Group {
                id,
                label,
                state,
                members: vec![e],
            }),
        }
    }
    let order: Vec<(String, TermId)> = groups
        .iter()
        .flat_map(|g| g.members.iter())
        .map(|e| (e.agent.tab.clone(), e.agent.term.clone()))
        .collect();
    let flat: Vec<Entry> = pinned
        .iter()
        .map(|a| entry(a, true, true))
        .chain(rest.iter().map(|a| entry(a, false, true)))
        .collect();
    View {
        groups,
        flat,
        order,
    }
}

pub fn next_needy(input: &Input) -> Option<(String, TermId)> {
    let cross = &input.agents;
    let cur = cross.iter().position(|a| a.focused);
    let needy: Vec<(usize, &Agent)> = cross
        .iter()
        .enumerate()
        .filter(|(_, a)| matches!(a.state(), State::Blocked | State::Done))
        .collect();
    let pick = needy
        .iter()
        .find(|(i, _)| cur.is_none_or(|c| *i > c))
        .or_else(|| needy.first())?;
    Some((pick.1.tab.clone(), pick.1.term.clone()))
}

pub fn step(
    order: &[(String, TermId)],
    focused: Option<&TermId>,
    step: isize,
) -> Option<(String, TermId)> {
    if order.is_empty() {
        return None;
    }
    let len = order.len() as isize;
    let next = match focused.and_then(|f| order.iter().position(|(_, t)| t == f)) {
        Some(at) => (at as isize + step).rem_euclid(len),
        None if step < 0 => len - 1,
        None => 0,
    };
    order.get(next as usize).cloned()
}

pub fn clean_title(title: &str) -> String {
    title
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim()
        .to_string()
}

pub fn label(name: &str, title: Option<&str>, program: &str) -> String {
    if !name.trim().is_empty() {
        return name.trim().to_string();
    }
    let raw = title.map(str::trim).unwrap_or_default();
    let path_like = raw.starts_with('~') || raw.starts_with('/');
    let live = clean_title(raw);
    let telling = !live.is_empty()
        && live != program
        && !path_like
        && !live.contains('@')
        && live.to_lowercase() != program.to_lowercase();
    if telling {
        return live;
    }
    if program.is_empty() {
        "shell".into()
    } else {
        program.to_string()
    }
}

pub const ROLE_LABEL: &str = "nixa.role";
pub const DEEP_ROLE: &str = "deep-brain";

pub fn is_deep(label: &str) -> bool {
    label.to_lowercase().contains("deep brain")
}

pub fn is_deep_term(labels: &BTreeMap<String, String>, label: &str) -> bool {
    match labels.get(ROLE_LABEL) {
        Some(role) => role == DEEP_ROLE,
        None => is_deep(label),
    }
}
