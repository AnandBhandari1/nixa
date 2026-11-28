use crate::util::short_hash;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_NODES: usize = 100;
pub const MAX_LINKS: usize = 300;
pub const MAX_TEXT: usize = 4000;
pub const MAX_TITLE: usize = 120;
pub const MAX_LIST: usize = 60;
pub const MAX_LIMIT: i64 = 100;
pub const MAX_RETRY: i64 = 5;
pub const MAX_STEPS: i64 = 500;
pub const DEFAULT_STEPS: i64 = 50;
pub const NAME_RULE: &str = "Workflow name: small letters, numbers and dashes, up to 60.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BoxType {
    Start,
    Stage,
    End,
}

impl BoxType {
    pub const WORDS: [&'static str; 3] = ["start", "stage", "end"];

    fn from_word(word: &str) -> BoxType {
        match word {
            "start" => BoxType::Start,
            "end" => BoxType::End,
            _ => BoxType::Stage,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Gate {
    None,
    Checks,
    DeepBrain,
    User,
}

impl Gate {
    pub const WORDS: [&'static str; 4] = ["none", "checks", "deep-brain", "user"];

    fn from_word(word: &str) -> Gate {
        match word {
            "checks" => Gate::Checks,
            "deep-brain" => Gate::DeepBrain,
            "user" => Gate::User,
            _ => Gate::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    Pass,
    Fail,
    Review,
    Revise,
}

impl LinkKind {
    pub const WORDS: [&'static str; 4] = ["pass", "fail", "review", "revise"];

    fn from_word(word: &str) -> LinkKind {
        match word {
            "fail" => LinkKind::Fail,
            "review" => LinkKind::Review,
            "revise" => LinkKind::Revise,
            _ => LinkKind::Pass,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            LinkKind::Pass => "pass",
            LinkKind::Fail => "fail",
            LinkKind::Review => "review",
            LinkKind::Revise => "revise",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WhoKind {
    #[serde(rename = "")]
    Nobody,
    Agent,
    CustomAgent,
    MapRow,
    Model,
    DeepBrain,
    Nixa,
    User,
}

impl WhoKind {
    pub const WORDS: [&'static str; 8] = ["", "agent", "custom-agent", "map-row", "model", "deep-brain", "nixa", "user"];
    pub const ALL: [WhoKind; 8] = [
        WhoKind::Nobody,
        WhoKind::Agent,
        WhoKind::CustomAgent,
        WhoKind::MapRow,
        WhoKind::Model,
        WhoKind::DeepBrain,
        WhoKind::Nixa,
        WhoKind::User,
    ];

    fn from_word(word: &str) -> WhoKind {
        WhoKind::WORDS.iter().position(|w| *w == word).map(|at| WhoKind::ALL[at]).unwrap_or(WhoKind::Nobody)
    }

    pub fn word(self) -> &'static str {
        WhoKind::ALL.iter().position(|k| *k == self).map(|at| WhoKind::WORDS[at]).unwrap_or("")
    }

    fn has_id(self) -> bool {
        matches!(self, WhoKind::Agent | WhoKind::CustomAgent | WhoKind::MapRow | WhoKind::Model)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Who {
    pub kind: WhoKind,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fallback {
    pub who: Who,
    pub model: String,
    pub effort: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Node {
    #[serde(rename = "type")]
    pub kind: BoxType,
    pub title: String,
    pub who: Who,
    pub model: String,
    pub effort: String,
    pub fallback: Fallback,
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub prompt: String,
    pub gate: Gate,
    pub retry: u32,
    pub limit: u32,
    pub off: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Link {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: LinkKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Spot {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Limits {
    pub max_steps: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Workflow {
    pub version: u32,
    pub name: String,
    pub title: String,
    pub when: String,
    pub nodes: BTreeMap<String, Node>,
    pub links: Vec<Link>,
    pub layout: BTreeMap<String, Spot>,
    pub limits: Limits,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkflowInfo {
    pub name: String,
    pub title: String,
    pub when: String,
    pub nodes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broken: Option<String>,
}

pub fn valid_node_id(id: &str) -> bool {
    let mut chars = id.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    first_ok && id.len() <= 30 && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn text_of(value: Option<&Value>, max: usize) -> String {
    match value {
        Some(Value::String(s)) => s.trim().chars().take(max).collect(),
        _ => String::new(),
    }
}

fn words_text(list: &[&str]) -> String {
    list.iter().filter(|w| !w.is_empty()).cloned().collect::<Vec<_>>().join(", ")
}

fn one_of(list: &[&'static str], value: Option<&Value>, label: &str, fallback: &'static str) -> Result<&'static str, String> {
    match value {
        None | Some(Value::Null) => return Ok(fallback),
        Some(Value::String(s)) if s.is_empty() => return Ok(fallback),
        Some(Value::String(s)) => {
            if let Some(word) = list.iter().find(|w| **w == s.as_str()) {
                return Ok(word);
            }
        }
        _ => {}
    }
    Err(format!("{label} must be one of: {}.", words_text(list)))
}

fn whole(value: Option<&Value>, min: i64, max: i64, label: &str, fallback: i64) -> Result<i64, String> {
    let bad = || format!("{label} must be a whole number from {min} to {max}.");
    match value {
        None | Some(Value::Null) => Ok(fallback),
        Some(Value::String(s)) if s.is_empty() => Ok(fallback),
        Some(Value::Number(n)) => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            if f.fract() == 0.0 && f >= min as f64 && f <= max as f64 { Ok(f as i64) } else { Err(bad()) }
        }
        _ => Err(bad()),
    }
}

fn names(value: Option<&Value>, label: &str) -> Result<Vec<String>, String> {
    let list = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(list)) => list,
        _ => return Err(format!("{label} is a list of names.")),
    };
    if list.iter().any(|v| !v.is_string()) {
        return Err(format!("{label} is a list of names."));
    }
    if list.len() > MAX_LIST {
        return Err(format!("{label} has at most {MAX_LIST} names."));
    }
    let mut out: Vec<String> = Vec::new();
    for item in list.iter().filter_map(Value::as_str) {
        let word = item.trim();
        if !word.is_empty() && !out.iter().any(|w| w == word) {
            out.push(word.to_string());
        }
    }
    Ok(out)
}

fn check_who(raw: Option<&Value>, label: &str, allowed: &[WhoKind]) -> Result<Who, String> {
    let object = raw.and_then(Value::as_object);
    let word = one_of(&WhoKind::WORDS, object.and_then(|o| o.get("kind")), &format!("{label}.kind"), "")?;
    let kind = WhoKind::from_word(word);
    if !allowed.contains(&kind) {
        let words: Vec<&str> = allowed.iter().map(|k| k.word()).collect();
        return Err(format!("{label}.kind must be one of: {}.", words_text(&words)));
    }
    let id = if kind.has_id() { text_of(object.and_then(|o| o.get("id")), 200) } else { String::new() };
    Ok(Who { kind, id })
}

pub fn empty_who() -> Who {
    Who { kind: WhoKind::Nobody, id: String::new() }
}

pub fn check_node(raw: &Value) -> Result<Node, String> {
    let Some(o) = raw.as_object() else {
        return Err("A node is an object with type, title, who, prompt, gate, retry and limit.".into());
    };
    let kind = BoxType::from_word(one_of(&BoxType::WORDS, o.get("type"), "type", "stage")?);
    let title = text_of(o.get("title"), MAX_TITLE);
    if title.is_empty() {
        return Err("A node needs a title.".into());
    }
    let empty = Map::new();
    let fb = o.get("fallback").and_then(Value::as_object).unwrap_or(&empty);
    let who = check_who(o.get("who"), "who", &WhoKind::ALL)?;
    let model = text_of(o.get("model"), 200);
    let effort = text_of(o.get("effort"), 40);
    let fallback = Fallback {
        who: check_who(fb.get("who"), "fallback.who", &[WhoKind::Nobody, WhoKind::Agent, WhoKind::CustomAgent])?,
        model: text_of(fb.get("model"), 200),
        effort: text_of(fb.get("effort"), 40),
    };
    let skills = names(o.get("skills"), "skills")?;
    let tools = names(o.get("tools"), "tools")?;
    let prompt = text_of(o.get("prompt"), MAX_TEXT);
    let gate = Gate::from_word(one_of(&Gate::WORDS, o.get("gate"), "gate", "none")?);
    let retry = whole(o.get("retry"), 0, MAX_RETRY, "retry", 0)? as u32;
    let limit = whole(o.get("limit"), 1, MAX_LIMIT, "limit", 1)? as u32;
    let off = o.get("off") == Some(&Value::Bool(true));
    Ok(Node { kind, title, who, model, effort, fallback, skills, tools, prompt, gate, retry, limit, off })
}

pub fn check_workflow(raw: &Value, saved_name: Option<&str>) -> Result<Workflow, String> {
    let Some(o) = raw.as_object() else {
        return Err("A workflow is { name, title, when, nodes, links }.".into());
    };
    if let Some(version) = o.get("version")
        && version.as_f64() != Some(1.0)
    {
        return Err("Workflow version must be 1.".into());
    }
    let given = text_of(o.get("name"), 80);
    let name = if given.is_empty() { saved_name.unwrap_or_default().to_string() } else { given.clone() };
    if !nixa_store::workflows::valid_name(&name) {
        return Err(NAME_RULE.into());
    }
    if let Some(saved) = saved_name
        && !given.is_empty()
        && given != saved
    {
        return Err(format!("The workflow saved as {saved} has the name \"{given}\" inside. The two must match."));
    }
    let when = text_of(o.get("when"), 400);
    if when.is_empty() {
        return Err("A workflow needs when: one plain line that says when to use it.".into());
    }
    let Some(raw_nodes) = o.get("nodes").and_then(Value::as_object) else {
        return Err("A workflow needs nodes: { \"n1\": { type, title, ... } }.".into());
    };
    if raw_nodes.len() > MAX_NODES {
        return Err(format!("A workflow has at most {MAX_NODES} nodes."));
    }
    let mut nodes = BTreeMap::new();
    for (id, raw_node) in raw_nodes {
        if !valid_node_id(id) {
            return Err(format!("Node id {id}: letters, numbers, dash or underscore, up to 30."));
        }
        let node = check_node(raw_node).map_err(|e| format!("Node {id}: {e}"))?;
        nodes.insert(id.clone(), node);
    }
    let empty = Vec::new();
    let raw_links = match o.get("links") {
        None => &empty,
        Some(Value::Array(list)) => list,
        Some(_) => return Err("links is a list of { id, from, to, kind }.".into()),
    };
    if raw_links.len() > MAX_LINKS {
        return Err(format!("A workflow has at most {MAX_LINKS} links."));
    }
    let mut links = Vec::new();
    for (index, item) in raw_links.iter().enumerate() {
        let Some(l) = item.as_object() else {
            return Err(format!("Link {} is {{ id, from, to, kind }}.", index + 1));
        };
        let id = text_of(l.get("id"), 40);
        links.push(Link {
            id: if id.is_empty() { format!("l{}", index + 1) } else { id },
            from: text_of(l.get("from"), 30),
            to: text_of(l.get("to"), 30),
            kind: LinkKind::from_word(one_of(&LinkKind::WORDS, l.get("kind"), &format!("Link {} kind", index + 1), "pass")?),
        });
    }
    let mut layout = BTreeMap::new();
    if let Some(spots) = o.get("layout").and_then(Value::as_object) {
        for (id, spot) in spots {
            let x = spot.get("x").and_then(Value::as_f64);
            let y = spot.get("y").and_then(Value::as_f64);
            if let (true, Some(x), Some(y)) = (nodes.contains_key(id), x, y)
                && x.is_finite()
                && y.is_finite()
            {
                layout.insert(id.clone(), Spot { x, y });
            }
        }
    }
    let limits = o.get("limits").and_then(Value::as_object);
    let steps_raw = limits.and_then(|l| l.get("max_steps").or_else(|| l.get("maxSteps")));
    let max_steps = whole(steps_raw, 1, MAX_STEPS, "limits.max_steps", DEFAULT_STEPS)? as u32;
    let title = text_of(o.get("title"), MAX_TITLE);
    Ok(Workflow {
        version: 1,
        title: if title.is_empty() { name.clone() } else { title },
        name,
        when,
        nodes,
        links,
        layout,
        limits: Limits { max_steps },
    })
}

fn label(flow: &Workflow, id: &str) -> String {
    format!("\"{}\"", flow.nodes.get(id).map(|n| n.title.as_str()).unwrap_or(id))
}

fn reach(out: &BTreeMap<&str, Vec<&str>>, from: &[&str]) -> BTreeSet<String> {
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut todo: Vec<&str> = from.to_vec();
    while let Some(at) = todo.pop() {
        for to in out.get(at).into_iter().flatten() {
            if found.insert(to.to_string()) {
                todo.push(to);
            }
        }
    }
    found
}

pub fn lint(flow: &Workflow) -> Vec<String> {
    let mut problems = Vec::new();
    let ids: Vec<&str> = flow.nodes.keys().map(String::as_str).collect();
    let starts: Vec<&str> = ids.iter().copied().filter(|id| flow.nodes[*id].kind == BoxType::Start).collect();
    let ends: Vec<&str> = ids.iter().copied().filter(|id| flow.nodes[*id].kind == BoxType::End).collect();
    if starts.len() != 1 {
        problems.push(if starts.is_empty() { "There is no start box." } else { "There must be exactly one start box." }.to_string());
    }
    if ends.is_empty() {
        problems.push("There is no end box.".to_string());
    }
    let mut out: BTreeMap<&str, Vec<&Link>> = ids.iter().map(|id| (*id, Vec::new())).collect();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut ok: Vec<&Link> = Vec::new();
    for link in &flow.links {
        if !seen.insert(link.id.as_str()) {
            problems.push(format!("Two links share the id {}.", link.id));
        }
        let (Some(from), Some(to)) = (flow.nodes.get(&link.from), flow.nodes.get(&link.to)) else {
            problems.push(format!("Link {} joins a box that does not exist.", link.id));
            continue;
        };
        if link.from == link.to {
            problems.push(format!("{} joins itself.", label(flow, &link.from)));
            continue;
        }
        if to.kind == BoxType::Start {
            problems.push(format!("A link goes into the start box ({}).", label(flow, &link.from)));
        }
        if from.kind == BoxType::End {
            problems.push(format!("A link leaves the end box {}.", label(flow, &link.from)));
        }
        let list = out.entry(link.from.as_str()).or_default();
        if list.iter().any(|other| other.kind == link.kind) {
            problems.push(format!("{} has two \"{}\" links. Keep one.", label(flow, &link.from), link.kind.word()));
        }
        list.push(link);
        ok.push(link);
    }
    for (id, node) in &flow.nodes {
        if node.kind == BoxType::Stage && !node.off && node.who.kind == WhoKind::Nobody {
            problems.push(format!("{} does not say who does it.", label(flow, id)));
        }
        if node.kind == BoxType::Stage && node.prompt.is_empty() {
            problems.push(format!("{} has no job text.", label(flow, id)));
        }
        if node.kind != BoxType::End && !out[id.as_str()].iter().any(|l| l.kind == LinkKind::Pass) {
            problems.push(format!("{} has no \"pass\" link to a next box.", label(flow, id)));
        }
    }
    let forward: BTreeMap<&str, Vec<&str>> = out.iter().map(|(id, list)| (*id, list.iter().map(|l| l.to.as_str()).collect())).collect();
    if starts.len() == 1 {
        let mut reached = reach(&forward, &starts);
        reached.insert(starts[0].to_string());
        for id in &ids {
            if !reached.contains(*id) {
                problems.push(format!("{} cannot be reached from the start box.", label(flow, id)));
            }
        }
    }
    let mut back: BTreeMap<&str, Vec<&str>> = ids.iter().map(|id| (*id, Vec::new())).collect();
    for link in &ok {
        back.entry(link.to.as_str()).or_default().push(link.from.as_str());
    }
    let mut to_end = reach(&back, &ends);
    to_end.extend(ends.iter().map(|id| id.to_string()));
    for id in &ids {
        if !to_end.contains(*id) {
            problems.push(format!("{} cannot reach an end box.", label(flow, id)));
        }
    }
    for id in &ids {
        if reach(&forward, &[id]).contains(*id) && flow.nodes[*id].limit < 2 {
            problems.push(format!("{} is inside a loop, so its limit must be 2 or more.", label(flow, id)));
        }
    }
    problems
}

pub fn problems(raw: &Value, saved_name: Option<&str>) -> Vec<String> {
    match check_workflow(raw, saved_name) {
        Ok(flow) => lint(&flow),
        Err(error) => vec![error],
    }
}

pub fn next_node_id(flow: &Workflow) -> String {
    let top = flow
        .nodes
        .keys()
        .filter_map(|id| id.strip_prefix('n').and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    format!("n{}", top + 1)
}

pub fn to_json(flow: &Workflow) -> Value {
    serde_json::to_value(flow).unwrap_or(Value::Null)
}

pub fn flow_hash(flow: &Workflow) -> String {
    short_hash(&to_json(flow).to_string())
}

pub fn parse_saved(name: &str, body: &str) -> Result<Workflow, String> {
    let raw: Value = serde_json::from_str(body).map_err(|e| format!("The saved workflow {name} is not JSON: {e}"))?;
    check_workflow(&raw, Some(name))
}

pub fn info(name: &str, body: &str) -> WorkflowInfo {
    match parse_saved(name, body) {
        Ok(flow) => WorkflowInfo { name: name.to_string(), title: flow.title, when: flow.when, nodes: flow.nodes.len(), broken: None },
        Err(error) => WorkflowInfo { name: name.to_string(), title: name.to_string(), when: String::new(), nodes: 0, broken: Some(error) },
    }
}
