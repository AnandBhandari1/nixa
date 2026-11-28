use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const BOX_W: f32 = 280.0;
pub const ROW: f32 = 22.0;
pub const HEAD: f32 = 26.0;
pub const GAP_X: f64 = 330.0;
pub const BELOW: f64 = 220.0;
pub const SIDE_STEP: f64 = 160.0;
pub const MAX_UNDO: usize = 60;
pub const MAX_SHOWN: usize = 60;
pub const NEW_PROMPT: &str = "Describe the job of this box.";
pub const NEW_TITLE: &str = "New box";
pub const NEW_TARGET: &str = "+new";

pub const KINDS: &[(&str, &str)] = &[
    ("pass", "Go on to the next box"),
    ("review", "Send the work to a reviewer box"),
    ("fail", "Go here when the checks fail"),
    ("revise", "Go back to fix what a review found"),
];

pub const GATES: &[(&str, &str, &str)] = &[
    ("none", "none", "Go on when the agent says it is done"),
    ("checks", "checks", "The project checks must pass"),
    ("deep-brain", "deep brain", "The deep brain must accept it"),
    ("user", "You", "Nixa decides it for you"),
];

pub const FIXED_WHO: &[(&str, &str, &str)] = &[
    ("deep-brain", "Deep brain", "Decides only, never builds"),
    ("nixa", "Nixa", "Nixa's own step"),
    ("user", "You", "Nixa decides it for you"),
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Who {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fallback {
    pub who: Who,
    pub model: String,
    pub effort: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Node {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub who: Who,
    pub model: String,
    pub effort: String,
    pub fallback: Fallback,
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub prompt: String,
    pub gate: String,
    pub retry: i64,
    pub limit: i64,
    pub off: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Node {
    fn default() -> Self {
        Node {
            kind: "stage".into(),
            title: String::new(),
            who: Who::default(),
            model: String::new(),
            effort: String::new(),
            fallback: Fallback::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            prompt: String::new(),
            gate: "none".into(),
            retry: 0,
            limit: 1,
            off: false,
            extra: Map::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Link {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spot {
    pub x: f64,
    pub y: f64,
}

#[derive(Deserialize)]
struct RawSpot {
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
}

impl Serialize for Spot {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        json!({"x": self.x.round() as i64, "y": self.y.round() as i64}).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Spot {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = RawSpot::deserialize(d)?;
        Ok(Spot { x: raw.x, y: raw.y })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workflow {
    pub version: i64,
    pub name: String,
    pub title: String,
    pub when: String,
    pub nodes: BTreeMap<String, Node>,
    pub links: Vec<Link>,
    pub layout: BTreeMap<String, Spot>,
    pub limits: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Workflow {
    fn default() -> Self {
        Workflow {
            version: 1,
            name: String::new(),
            title: String::new(),
            when: String::new(),
            nodes: BTreeMap::new(),
            links: Vec::new(),
            layout: BTreeMap::new(),
            limits: json!({}),
            extra: Map::new(),
        }
    }
}

impl Workflow {
    pub fn max_steps(&self) -> u64 {
        self.limits
            .get("max_steps")
            .or_else(|| self.limits.get("maxSteps"))
            .and_then(Value::as_u64)
            .unwrap_or(50)
    }

    pub fn set_max_steps(&mut self, steps: u32) {
        let mut limits = self.limits.as_object().cloned().unwrap_or_default();
        limits.remove("maxSteps");
        limits.insert("max_steps".into(), json!(steps));
        self.limits = Value::Object(limits);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Info {
    pub name: String,
    pub title: String,
    pub when: String,
    pub broken: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct CustomAgent {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub role: String,
    pub provider: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct MapRow {
    pub id: String,
    pub job: String,
    pub member: String,
    pub model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ModelEntry {
    pub agent: String,
    pub model: String,
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Named {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Options {
    #[serde(alias = "customAgents")]
    pub custom_agents: Vec<CustomAgent>,
    #[serde(alias = "agentKinds")]
    pub agent_kinds: Vec<String>,
    #[serde(alias = "mapRows")]
    pub map_rows: Vec<MapRow>,
    pub models: Vec<ModelEntry>,
    pub skills: Vec<Named>,
    pub tools: Vec<Named>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct RunInfo {
    #[serde(deserialize_with = "number")]
    pub task: i64,
    #[serde(rename = "box")]
    pub stage: String,
    pub space: String,
    pub state: String,
    pub workflow: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct RunEntry {
    #[serde(rename = "box")]
    pub stage: String,
    pub outcome: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Ask {
    #[serde(rename = "box")]
    pub stage: String,
    pub cause: String,
    pub why: String,
    pub gate: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Run {
    pub workflow: String,
    pub node: String,
    pub state: String,
    pub started: f64,
    pub visits: BTreeMap<String, i64>,
    pub history: Vec<RunEntry>,
    pub ask: Option<Ask>,
    pub line: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Report {
    pub number: u32,
    #[serde(rename = "box")]
    pub box_id: String,
    pub text: String,
}

pub fn parse_reports(value: &Value) -> Vec<Report> {
    list_of(value, "reports")
}

pub const GATE_ANSWERS: &[(&str, &str, &str)] = &[
    ("pass", "Pass", "Accept the work and go on"),
    ("revise", "Revise", "Send it back to be fixed"),
    ("fail", "Fail", "Take the fail link"),
];

pub const NEXT_BOX: &str = "Go to the next box";

pub fn waiting_text(run: &Run) -> Option<String> {
    let ask = run.ask.as_ref().filter(|_| run.state == "waiting")?;
    let why = if ask.why.is_empty() {
        ask.cause.as_str()
    } else {
        ask.why.as_str()
    };
    Some(format!("Waiting at {}: {why}", ask.stage))
}

fn number<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Value::String(s) => s.trim().trim_start_matches('#').parse().unwrap_or(0),
        _ => 0,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Opt {
    pub value: String,
    pub label: String,
    pub hint: String,
}

fn opt(value: impl Into<String>, label: impl Into<String>, hint: impl Into<String>) -> Opt {
    Opt {
        value: value.into(),
        label: label.into(),
        hint: hint.into(),
    }
}

pub fn data(value: &Value) -> &Value {
    match value.get("data") {
        Some(inner) if inner.is_object() && value.get("ok").is_some() => inner,
        _ => value,
    }
}

pub fn text_of(value: &Value) -> String {
    let d = data(value);
    d.get("text")
        .or_else(|| value.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn parse_workflow(value: &Value) -> Option<Workflow> {
    let raw = data(value).get("workflow")?;
    if !raw.is_object() {
        return None;
    }
    serde_json::from_value(raw.clone()).ok()
}

pub fn can_undo(value: &Value) -> bool {
    let d = data(value);
    d.get("canUndo")
        .or_else(|| d.get("can_undo"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn list_of<T: for<'de> Deserialize<'de>>(value: &Value, key: &str) -> Vec<T> {
    data(value)
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_names(value: &Value) -> Vec<Info> {
    let mut list: Vec<Info> = list_of(value, "workflows");
    list.retain(|w| !w.name.is_empty());
    list
}

pub fn parse_runs(value: &Value) -> Vec<RunInfo> {
    list_of(value, "runs")
}

pub fn parse_problems(value: &Value) -> Vec<String> {
    list_of(value, "problems")
}

pub fn parse_options(value: &Value) -> Option<Options> {
    let raw = data(value).get("options")?;
    serde_json::from_value(raw.clone()).ok()
}

pub fn parse_run(value: &Value) -> Option<(Run, Workflow)> {
    let d = data(value);
    let run = d.get("run").filter(|r| r.is_object())?;
    let run: Run = serde_json::from_value(run.clone()).ok()?;
    let flow = parse_workflow(value)?;
    Some((run, flow))
}

pub fn to_text(flow: &Workflow) -> String {
    serde_json::to_string(flow).unwrap_or_default()
}

pub fn to_value(flow: &Workflow) -> Value {
    serde_json::to_value(flow).unwrap_or(Value::Null)
}

pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    let ok = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    !b.is_empty() && b.len() <= 60 && ok(b[0]) && b.iter().all(|c| ok(*c) || *c == b'-')
}

fn suffix_number(id: &str, prefix: char) -> Option<u64> {
    let rest = id.strip_prefix(prefix)?;
    if rest.is_empty() || !rest.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

pub fn next_id<'a>(ids: impl Iterator<Item = &'a str>, prefix: char) -> String {
    let top = ids
        .filter_map(|id| suffix_number(id, prefix))
        .max()
        .unwrap_or(0);
    format!("{prefix}{}", top + 1)
}

pub fn ids(flow: &Workflow) -> Vec<String> {
    let mut list: Vec<String> = flow.nodes.keys().cloned().collect();
    list.sort_by_key(|id| {
        (
            suffix_number(id, 'n').is_none(),
            suffix_number(id, 'n').unwrap_or(0),
            id.clone(),
        )
    });
    list
}

pub fn node(kind: &str, title: &str) -> Node {
    Node {
        kind: kind.into(),
        title: title.into(),
        ..Node::default()
    }
}

pub fn new_stage() -> Node {
    Node {
        who: Who {
            kind: "deep-brain".into(),
            id: String::new(),
        },
        prompt: NEW_PROMPT.into(),
        ..node("stage", NEW_TITLE)
    }
}

pub fn template(name: &str, title: &str, when: &str) -> Workflow {
    let mut nodes = BTreeMap::new();
    nodes.insert("n1".to_string(), node("start", "Start"));
    nodes.insert(
        "n2".to_string(),
        Node {
            title: "Work".into(),
            ..new_stage()
        },
    );
    nodes.insert("n3".to_string(), node("end", "Done"));
    let link = |id: &str, from: &str, to: &str| Link {
        id: id.into(),
        from: from.into(),
        to: to.into(),
        kind: "pass".into(),
        extra: Map::new(),
    };
    Workflow {
        name: name.into(),
        title: if title.is_empty() {
            name.into()
        } else {
            title.into()
        },
        when: when.into(),
        nodes,
        links: vec![link("l1", "n1", "n2"), link("l2", "n2", "n3")],
        limits: json!({"maxSteps": 30}),
        ..Workflow::default()
    }
}

pub fn copy_of(flow: &Workflow, name: &str, old: &str) -> Workflow {
    let mut copy = flow.clone();
    copy.name = name.into();
    let base = if copy.title.is_empty() {
        old
    } else {
        copy.title.as_str()
    };
    copy.title = format!("{base} copy");
    copy
}

pub fn add_box(flow: &mut Workflow, near: Spot) -> String {
    let id = next_id(flow.nodes.keys().map(String::as_str), 'n');
    flow.nodes.insert(id.clone(), new_stage());
    flow.layout.insert(
        id.clone(),
        Spot {
            x: near.x,
            y: near.y + BELOW,
        },
    );
    id
}

pub fn removable(flow: &Workflow, id: &str) -> bool {
    flow.nodes.get(id).is_some_and(|n| n.kind != "start")
}

pub fn remove_box(flow: &mut Workflow, id: &str) -> bool {
    if !removable(flow, id) {
        return false;
    }
    flow.nodes.remove(id);
    flow.layout.remove(id);
    flow.links.retain(|l| l.from != id && l.to != id);
    true
}

pub fn unlink(flow: &mut Workflow, link: &str) {
    flow.links.retain(|l| l.id != link);
}

pub fn connect(flow: &mut Workflow, from: &str, kind: &str, to: &str) -> Option<String> {
    flow.links.retain(|l| !(l.from == from && l.kind == kind));
    let mut made = None;
    let target = if to == NEW_TARGET {
        let id = next_id(flow.nodes.keys().map(String::as_str), 'n');
        flow.nodes.insert(id.clone(), new_stage());
        let p = flow.layout.get(from).copied().unwrap_or_default();
        let used = flow.links.iter().filter(|l| l.from == from).count() as f64;
        flow.layout.insert(
            id.clone(),
            Spot {
                x: p.x + GAP_X,
                y: p.y + used * SIDE_STEP,
            },
        );
        made = Some(id.clone());
        id
    } else {
        to.to_string()
    };
    let id = next_id(flow.links.iter().map(|l| l.id.as_str()), 'l');
    flow.links.push(Link {
        id,
        from: from.into(),
        to: target,
        kind: kind.into(),
        extra: Map::new(),
    });
    made
}

pub fn out_links<'a>(flow: &'a Workflow, id: &'a str) -> impl Iterator<Item = &'a Link> + 'a {
    flow.links.iter().filter(move |l| l.from == id)
}

pub fn rows(flow: &Workflow, id: &str, run: bool, options: &Options) -> f32 {
    let Some(n) = flow.nodes.get(id) else {
        return HEAD + 8.0;
    };
    let mut count = out_links(flow, id).count();
    if n.kind == "stage" {
        count += 8;
        count += usize::from(has_efforts(options, &n.who, &n.model));
        if !n.fallback.who.kind.is_empty() {
            count += 1 + usize::from(has_efforts(options, &n.fallback.who, &n.fallback.model));
        }
        count += usize::from(run);
    }
    if !run && n.kind != "end" {
        count += 1;
    }
    HEAD + 8.0 + count as f32 * ROW
}

pub fn clip(text: &str, n: usize) -> String {
    if text.chars().count() > n {
        let mut out: String = text.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    } else {
        text.to_string()
    }
}

pub fn who_from(value: &str) -> Who {
    let (kind, id) = value.split_once(':').unwrap_or((value, ""));
    Who {
        kind: kind.into(),
        id: id.into(),
    }
}

pub fn who_label(o: &Options, w: &Who) -> String {
    match w.kind.as_str() {
        "" => "choose".into(),
        "agent" => w.id.clone(),
        "custom-agent" => o
            .custom_agents
            .iter()
            .find(|a| a.id == w.id)
            .map(|a| a.title.clone())
            .unwrap_or_else(|| w.id.clone()),
        "map-row" => o
            .map_rows
            .iter()
            .find(|r| r.id == w.id)
            .map(|r| r.job.clone())
            .unwrap_or_else(|| w.id.clone()),
        "model" => w.id.clone(),
        other => FIXED_WHO
            .iter()
            .find(|(k, _, _)| *k == other)
            .map(|(_, label, _)| label.to_string())
            .unwrap_or_else(|| other.to_string()),
    }
}

pub fn kind_of(o: &Options, w: &Who) -> String {
    match w.kind.as_str() {
        "agent" => w.id.clone(),
        "custom-agent" => o
            .custom_agents
            .iter()
            .find(|a| a.id == w.id)
            .map(|a| a.provider.clone())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn models_of<'a>(o: &'a Options, kind: &'a str) -> impl Iterator<Item = &'a ModelEntry> + 'a {
    o.models
        .iter()
        .filter(move |m| kind.is_empty() || m.agent == kind)
}

pub fn efforts_of(o: &Options, kind: &str, model: &str) -> Vec<String> {
    models_of(o, kind)
        .find(|m| m.model == model)
        .map(|m| m.efforts.clone())
        .unwrap_or_default()
}

pub fn has_efforts(o: &Options, who: &Who, model: &str) -> bool {
    !model.is_empty() && !efforts_of(o, &kind_of(o, who), model).is_empty()
}

pub fn no_list(o: &Options, w: &Who) -> bool {
    let kind = kind_of(o, w);
    !kind.is_empty() && models_of(o, &kind).next().is_none()
}

pub fn who_options(o: &Options, fallback: bool) -> Vec<Opt> {
    let mut out: Vec<Opt> = o
        .agent_kinds
        .iter()
        .map(|k| opt(format!("agent:{k}"), k.clone(), "Agent"))
        .collect();
    for a in &o.custom_agents {
        let provider = if a.provider.is_empty() {
            String::new()
        } else {
            format!("{} · ", a.provider)
        };
        out.push(opt(
            format!("custom-agent:{}", a.id),
            format!("{} · {}", a.id, a.title),
            format!("{provider}{}", a.purpose),
        ));
    }
    if fallback {
        out.push(opt(":", "(none)", "No fallback"));
    } else {
        for (kind, label, hint) in FIXED_WHO {
            out.push(opt(format!("{kind}:"), *label, *hint));
        }
    }
    out
}

pub fn model_options(o: &Options, kind: &str) -> Vec<Opt> {
    let mut out = vec![opt("", "(none)", "Use the agent's own model")];
    out.extend(models_of(o, kind).map(|m| opt(m.model.clone(), m.model.clone(), m.agent.clone())));
    out
}

pub fn effort_options(o: &Options, kind: &str, model: &str) -> Vec<Opt> {
    let mut out = vec![opt("", "(none)", "")];
    let mut seen = BTreeSet::new();
    for e in efforts_of(o, kind, model) {
        if seen.insert(e.clone()) {
            out.push(opt(e.clone(), e, ""));
        }
    }
    out
}

pub fn named_options(list: &[Named]) -> Vec<Opt> {
    list.iter()
        .map(|s| opt(s.name.clone(), s.name.clone(), s.description.clone()))
        .collect()
}

pub fn gate_options() -> Vec<Opt> {
    GATES.iter().map(|(v, l, h)| opt(*v, *l, *h)).collect()
}

pub fn gate_label(gate: &str) -> String {
    GATES
        .iter()
        .find(|(v, _, _)| *v == gate)
        .map(|(_, l, _)| l.to_string())
        .unwrap_or_else(|| gate.to_string())
}

pub fn kind_options() -> Vec<Opt> {
    KINDS.iter().map(|(v, h)| opt(*v, *v, *h)).collect()
}

pub fn target_options(flow: &Workflow, from: &str) -> Vec<Opt> {
    let mut out = vec![opt(
        NEW_TARGET,
        "+ New box",
        "Make a new box here and join it",
    )];
    for id in ids(flow) {
        let n = &flow.nodes[&id];
        if id != from && n.kind != "start" {
            out.push(opt(
                id.clone(),
                format!("{id} · {}", n.title),
                n.kind.clone(),
            ));
        }
    }
    out
}

pub fn filter_options<'a>(options: &'a [Opt], query: &str) -> Vec<(usize, &'a Opt)> {
    let q = query.trim().to_lowercase();
    options
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            q.is_empty()
                || format!("{} {}", o.label, o.hint)
                    .to_lowercase()
                    .contains(&q)
        })
        .collect()
}

pub fn filter_names<'a>(names: &'a [Info], query: &str) -> Vec<&'a Info> {
    let q = query.trim().to_lowercase();
    names
        .iter()
        .filter(|n| {
            q.is_empty()
                || format!("{} {} {}", n.name, n.title, n.when)
                    .to_lowercase()
                    .contains(&q)
        })
        .collect()
}

pub fn run_line(n: &Node, run: &Run, id: &str) -> String {
    if n.off {
        return "off, skipped".into();
    }
    let seen = run.visits.get(id).copied().unwrap_or(0);
    let mut out = format!("visits {seen}");
    if let Some(last) = run.history.iter().rev().find(|e| e.stage == id) {
        out.push_str(&format!(" · last {}", last.outcome));
    }
    if run.node == id {
        out.push_str(&format!(" · now {}", run.state));
    }
    out
}

pub fn active(state: &str) -> bool {
    matches!(state, "running" | "paused" | "waiting")
}
