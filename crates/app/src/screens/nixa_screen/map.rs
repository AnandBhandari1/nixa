use super::draw::{PI, TAU};
use gpui::{Hsla, rgb};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const MAX_AGENTS: usize = 24;
pub const FLOW_MS: u64 = 1400;
pub const PULSE_MS: u64 = 600;
pub const FLOW_LIFE_MS: u64 = FLOW_MS + PULSE_MS;
pub const DEEP_CARRY_MS: u64 = 90_000;
pub const NOVA_MS: u64 = 1600;
pub const WARP_MS: u64 = 900;
pub const AGENT_RADIUS: f32 = 18.0;
pub const KID_RADIUS: f32 = 12.0;
pub const NIXA: &str = "nixa";
pub const DEEP: &str = "deep";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum St {
    Blocked,
    Working,
    Done,
    Idle,
}

impl St {
    pub fn rank(self) -> u8 {
        match self {
            St::Blocked => 0,
            St::Working => 1,
            St::Done => 2,
            St::Idle => 3,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            St::Blocked => "blocked",
            St::Working => "working",
            St::Done => "done",
            St::Idle => "idle",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub id: String,
    pub title: String,
    pub program: String,
    pub mono: String,
    pub status: St,
    pub parent: Option<String>,
    pub seq: u64,
    pub number: Option<u32>,
    pub persona: Option<String>,
}

impl Agent {
    pub fn name(&self) -> String {
        format!("agent {}", self.shown_number())
    }

    pub fn shown_number(&self) -> String {
        self.number
            .map(|n| n.to_string())
            .unwrap_or_else(|| self.id.clone())
    }

    pub fn label(&self) -> String {
        let title = self.title.split(" · ").next().unwrap_or("").trim();
        let base = if title.is_empty() {
            self.name()
        } else {
            format!("{} · {title}", self.shown_number())
        };
        match &self.persona {
            Some(name) => format!("{name} · {base}"),
            None => base,
        }
    }

    pub fn task(&self) -> Option<String> {
        let at = self.title.find('#')?;
        let digits: String = self.title[at + 1..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        (!digits.is_empty()).then_some(digits)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Claude,
    Codex,
    Gemini,
    Other,
}

pub fn deep_kind(agent: &Agent) -> Kind {
    let text = format!("{} {}", agent.title, agent.program).to_lowercase();
    let hits = [
        ("claude", Kind::Claude),
        ("codex", Kind::Codex),
        ("antigravity", Kind::Gemini),
        ("gemini", Kind::Gemini),
        ("agy", Kind::Gemini),
    ];
    hits.iter()
        .filter_map(|(word, kind)| text.find(word).map(|at| (at, *kind)))
        .min_by_key(|(at, _)| *at)
        .map(|(_, kind)| kind)
        .unwrap_or(Kind::Other)
}

pub fn deep_color(kind: Kind) -> Hsla {
    match kind {
        Kind::Claude => rgb(0xd97757).into(),
        Kind::Codex => rgb(0xffffff).into(),
        Kind::Gemini => rgb(0x3186ff).into(),
        Kind::Other => rgb(0x7d8794).into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Brain {
    pub agent: Agent,
    pub kind: Kind,
    pub color: Hsla,
    pub spot: Spot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub agent: Agent,
    pub parent: Option<String>,
    pub radius: f32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    pub nixa: Spot,
    pub brains: Vec<Brain>,
    pub agents: Vec<Placed>,
    pub ax: f32,
    pub ay: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub x: f32,
    pub y: f32,
    pub r: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Flow {
    pub from: String,
    pub to: String,
    pub at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Burst {
    Warp,
    Nova,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: Burst,
    pub id: String,
    pub at: u64,
}

pub fn pick_shown(agents: &[Agent]) -> Vec<Agent> {
    let mut picked: BTreeMap<String, Agent> = if agents.len() <= MAX_AGENTS {
        agents.iter().map(|a| (a.id.clone(), a.clone())).collect()
    } else {
        let mut sorted = agents.to_vec();
        sorted.sort_by_key(|a| a.status.rank());
        sorted
            .into_iter()
            .take(MAX_AGENTS)
            .map(|a| (a.id.clone(), a))
            .collect()
    };
    let all: HashMap<&str, &Agent> = agents.iter().map(|a| (a.id.as_str(), a)).collect();
    let ids: Vec<String> = picked.keys().cloned().collect();
    for id in ids {
        let mut seen = BTreeSet::new();
        let mut parent = picked.get(&id).and_then(|a| a.parent.clone());
        while let Some(p) = parent {
            if !seen.insert(p.clone()) || picked.contains_key(&p) {
                break;
            }
            let Some(found) = all.get(p.as_str()) else {
                break;
            };
            picked.insert(p.clone(), (*found).clone());
            parent = found.parent.clone();
        }
    }
    let mut out: Vec<Agent> = picked.into_values().collect();
    out.sort_by(|a, b| a.seq.cmp(&b.seq).then_with(|| a.id.cmp(&b.id)));
    out
}

fn orbit_of(radius: f32, largest: f32, kids: usize) -> f32 {
    (radius + largest + 24.0).max(kids as f32 * (largest * 2.0 + 24.0) / TAU)
}

struct Tree<'a> {
    parents: HashMap<&'a str, &'a str>,
    children: HashMap<&'a str, Vec<&'a Agent>>,
    sizes: HashMap<&'a str, f32>,
}

fn measure<'a>(tree: &mut Tree<'a>, agent: &'a Agent, depth: usize) -> f32 {
    let kids = tree
        .children
        .get(agent.id.as_str())
        .cloned()
        .unwrap_or_default();
    let radius = if tree.parents.contains_key(agent.id.as_str()) {
        KID_RADIUS
    } else {
        AGENT_RADIUS
    };
    let mut largest: f32 = 24.0;
    if depth < 64 {
        for kid in &kids {
            largest = largest.max(measure(tree, kid, depth + 1));
        }
    }
    let extent = if kids.is_empty() {
        radius + 22.0
    } else {
        orbit_of(radius, largest, kids.len()) + largest
    };
    tree.sizes.insert(agent.id.as_str(), extent);
    extent
}

struct Place<'a> {
    w: f32,
    h: f32,
    nixa: Spot,
    tree: &'a Tree<'a>,
    out: HashMap<String, Placed>,
}

fn place_children(p: &mut Place, id: &str, (x, y, radius): (f32, f32, f32), depth: usize) {
    if depth > 64 {
        return;
    }
    let kids = p.tree.children.get(id).cloned().unwrap_or_default();
    if kids.is_empty() {
        return;
    }
    let largest = kids
        .iter()
        .map(|a| p.tree.sizes.get(a.id.as_str()).copied().unwrap_or(24.0))
        .fold(24.0_f32, f32::max);
    let orbit = orbit_of(radius, largest, kids.len());
    let full =
        x - orbit > 47.5 && x + orbit < p.w - 47.5 && y - orbit > 23.5 && y + orbit < p.h - 47.5;
    let phase = if full {
        (y - p.nixa.y).atan2(x - p.nixa.x)
    } else {
        (p.h / 2.0 - y).atan2(p.w / 2.0 - x)
    };
    let n = kids.len() as f32;
    for (i, agent) in kids.iter().enumerate() {
        let i = i as f32;
        let angle = phase
            + if full {
                TAU * i / n
            } else {
                PI * ((i + 0.5) / n - 0.5)
            };
        let node = Placed {
            agent: (*agent).clone(),
            parent: Some(id.to_string()),
            radius: KID_RADIUS,
            x: x + orbit * angle.cos(),
            y: y + orbit * angle.sin(),
        };
        let at = (node.x, node.y, node.radius);
        p.out.insert(agent.id.clone(), node);
        place_children(p, &agent.id, at, depth + 1);
    }
}

pub fn layout(w: f32, h: f32, agents: &[Agent], deep: &[Agent]) -> Layout {
    let min = w.min(h);
    let size = (min * 0.34).floor().max(140.0);
    let deep_size = (size * 0.42).floor().max(56.0);
    let nixa = Spot {
        x: w / 2.0,
        y: h * 0.56,
        size,
    };
    let deep_y = (deep_size / 2.0 + 18.0).max(nixa.y - min * 0.43);
    let spacing = (deep_size * 1.55).min(w / (deep.len() as f32 + 1.0));
    let brains: Vec<Brain> = deep
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let kind = deep_kind(agent);
            Brain {
                agent: agent.clone(),
                kind,
                color: deep_color(kind),
                spot: Spot {
                    x: w / 2.0 + (i as f32 - (deep.len() as f32 - 1.0) / 2.0) * spacing,
                    y: deep_y,
                    size: deep_size,
                },
            }
        })
        .collect();
    let by_id: HashMap<&str, &Agent> = agents.iter().map(|a| (a.id.as_str(), a)).collect();
    let brain_ids: BTreeSet<&str> = brains.iter().map(|b| b.agent.id.as_str()).collect();
    let mut tree = Tree {
        parents: HashMap::new(),
        children: HashMap::new(),
        sizes: HashMap::new(),
    };
    for agent in agents {
        let mut visited = BTreeSet::from([agent.id.as_str()]);
        let mut parent = agent.parent.as_deref();
        while let Some(p) = parent.filter(|p| by_id.contains_key(p)) {
            if !visited.insert(p) {
                break;
            }
            parent = by_id.get(p).and_then(|a| a.parent.as_deref());
        }
        let cycle = parent.is_some_and(|p| visited.contains(p));
        if let Some(direct) = agent.parent.as_deref()
            && !cycle
            && (by_id.contains_key(direct) || brain_ids.contains(direct))
        {
            tree.parents.insert(agent.id.as_str(), direct);
        }
    }
    for agent in agents {
        if let Some(parent) = tree.parents.get(agent.id.as_str()).copied() {
            tree.children.entry(parent).or_default().push(agent);
        }
    }
    let roots: Vec<&Agent> = agents
        .iter()
        .filter(|a| !tree.parents.contains_key(a.id.as_str()))
        .collect();
    for root in &roots {
        measure(&mut tree, root, 0);
    }
    for brain in &brains {
        for kid in tree
            .children
            .get(brain.agent.id.as_str())
            .cloned()
            .unwrap_or_default()
        {
            measure(&mut tree, kid, 0);
        }
    }
    let margin = roots
        .iter()
        .filter(|a| tree.children.contains_key(a.id.as_str()))
        .filter_map(|a| tree.sizes.get(a.id.as_str()).copied())
        .fold(0.0_f32, f32::max);
    let ax = size.max((w * 0.42).min(w / 2.0 - margin.max(110.0)));
    let ay = (size * 0.8)
        .max(h * 0.38)
        .min((size * 0.8).max(h * 0.44 - margin));
    let mut place = Place {
        w,
        h,
        nixa,
        tree: &tree,
        out: HashMap::new(),
    };
    let n = roots.len() as f32;
    for (i, agent) in roots.iter().enumerate() {
        let turn = (45.0 + 270.0 * (i as f32 + 0.5) / n).to_radians();
        let extent = if tree.children.contains_key(agent.id.as_str()) {
            tree.sizes
                .get(agent.id.as_str())
                .copied()
                .unwrap_or(AGENT_RADIUS)
        } else {
            AGENT_RADIUS
        };
        let x = (nixa.x + ax * turn.sin()).min(w - extent).max(extent);
        let y = (nixa.y - ay * turn.cos())
            .min(h - extent - 14.0)
            .max(extent);
        place.out.insert(
            agent.id.clone(),
            Placed {
                agent: (*agent).clone(),
                parent: None,
                radius: AGENT_RADIUS,
                x,
                y,
            },
        );
        place_children(&mut place, &agent.id, (x, y, AGENT_RADIUS), 0);
    }
    for brain in &brains {
        let s = brain.spot;
        place_children(&mut place, &brain.agent.id, (s.x, s.y, s.size / 2.0), 0);
    }
    let mut out = place.out;
    let placed = agents.iter().filter_map(|a| out.remove(&a.id)).collect();
    Layout {
        w,
        h,
        nixa,
        brains,
        agents: placed,
        ax,
        ay,
    }
}

pub fn node_of(layout: &Layout, who: &str) -> Option<Node> {
    if who == NIXA {
        return Some(Node {
            x: layout.nixa.x,
            y: layout.nixa.y,
            r: layout.nixa.size / 2.0 * 1.02,
        });
    }
    let brain = if who == DEEP {
        layout
            .brains
            .iter()
            .find(|b| b.agent.status == St::Working)
            .or_else(|| layout.brains.first())
    } else {
        layout.brains.iter().find(|b| b.agent.id == who)
    };
    if let Some(b) = brain {
        return Some(Node {
            x: b.spot.x,
            y: b.spot.y,
            r: b.spot.size / 2.0 * 1.02,
        });
    }
    layout
        .agents
        .iter()
        .find(|p| p.agent.id == who)
        .map(|p| Node {
            x: p.x,
            y: p.y,
            r: p.radius,
        })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub to: Node,
}

pub fn endpoints(layout: &Layout, from: &str, to: &str) -> Option<Line> {
    let a = node_of(layout, from)?;
    let b = node_of(layout, to)?;
    let angle = (b.y - a.y).atan2(b.x - a.x);
    Some(Line {
        x1: a.x + a.r * angle.cos(),
        y1: a.y + a.r * angle.sin(),
        x2: b.x - b.r * angle.cos(),
        y2: b.y - b.r * angle.sin(),
        to: b,
    })
}

pub type Seen = BTreeMap<String, St>;

pub fn agent_flows(
    before: Option<&Seen>,
    agents: &[Agent],
    now: u64,
    deep_answered_at: u64,
) -> (Seen, Vec<Flow>) {
    let seen: Seen = agents.iter().map(|a| (a.id.clone(), a.status)).collect();
    let Some(before) = before else {
        return (seen, Vec::new());
    };
    let giver = if deep_answered_at > 0 && now.saturating_sub(deep_answered_at) < DEEP_CARRY_MS {
        DEEP
    } else {
        NIXA
    };
    let mut flows = Vec::new();
    for a in agents {
        let was = before.get(&a.id).copied();
        if was == Some(a.status) {
            continue;
        }
        let owner = a.parent.clone().unwrap_or_else(|| giver.to_string());
        if was.is_none() || (a.status == St::Working && was != Some(St::Working)) {
            flows.push(Flow {
                from: owner,
                to: a.id.clone(),
                at: now,
            });
        } else if was == Some(St::Working) || a.status == St::Blocked {
            flows.push(Flow {
                from: a.id.clone(),
                to: a.parent.clone().unwrap_or_else(|| NIXA.to_string()),
                at: now,
            });
        }
    }
    (seen, flows)
}

pub fn agent_events(before: Option<&Seen>, agents: &[Agent], now: u64) -> Vec<Event> {
    let Some(before) = before else {
        return Vec::new();
    };
    agents
        .iter()
        .filter_map(|a| match before.get(&a.id) {
            None => Some(Event {
                kind: Burst::Warp,
                id: a.id.clone(),
                at: now,
            }),
            Some(St::Working) if a.status == St::Done => Some(Event {
                kind: Burst::Nova,
                id: a.id.clone(),
                at: now,
            }),
            _ => None,
        })
        .collect()
}

pub fn deep_flow(was: Option<bool>, working: bool, now: u64) -> Option<Flow> {
    let was = was?;
    if was == working {
        return None;
    }
    Some(if working {
        Flow {
            from: NIXA.into(),
            to: DEEP.into(),
            at: now,
        }
    } else {
        Flow {
            from: DEEP.into(),
            to: NIXA.into(),
            at: now,
        }
    })
}

pub fn live_flows(flows: &mut Vec<Flow>, now: u64) {
    flows.retain(|f| now.saturating_sub(f.at) < FLOW_LIFE_MS && f.at <= now + FLOW_LIFE_MS);
}

pub fn live_events(events: &mut Vec<Event>, now: u64) {
    events.retain(|e| now.saturating_sub(e.at) < NOVA_MS);
}
