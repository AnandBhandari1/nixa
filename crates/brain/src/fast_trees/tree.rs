use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_DEPTH: usize = 12;
pub const MAX_NODES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    Start { next: String },
    Check { key: String, label: String, yes: String, no: String },
    Action { key: String, label: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chart {
    pub start: String,
    pub steps: BTreeMap<String, Step>,
}

#[derive(Debug, Clone)]
pub struct Tree {
    nodes: HashMap<String, Step>,
    start: String,
    limit: usize,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Run {
    pub action: String,
    pub path: Vec<String>,
    pub comparisons: usize,
}

pub fn chart_of(raw: &serde_json::Value) -> Result<Chart, String> {
    serde_json::from_value(raw.clone()).map_err(|e| format!("Invalid decision tree: {e}."))
}

fn visit(chart: &Chart, id: &str, path: &mut Vec<String>, depths: &mut HashMap<String, usize>, limit: usize) -> Result<usize, String> {
    if path.iter().any(|p| p == id) {
        return Err(format!("Decision tree has a cycle at {id}."));
    }
    if let Some(depth) = depths.get(id) {
        return Ok(*depth);
    }
    let node = chart.steps.get(id).ok_or_else(|| format!("Decision tree points to missing node {id}."))?;
    path.push(id.to_string());
    let depth = match node {
        Step::Action { .. } => Ok(0),
        Step::Start { next } => visit(chart, next, path, depths, limit),
        Step::Check { yes, no, .. } => {
            let a = visit(chart, yes, path, depths, limit)?;
            let b = visit(chart, no, path, depths, limit)?;
            Ok(1 + a.max(b))
        }
    };
    path.pop();
    let depth = depth?;
    if depth > limit {
        return Err(format!("Decision tree depth {depth} exceeds limit {limit} at {id}."));
    }
    depths.insert(id.to_string(), depth);
    Ok(depth)
}

fn resolve(chart: &Chart, id: &str) -> String {
    let mut at = id.to_string();
    for _ in 0..=chart.steps.len() {
        match chart.steps.get(&at) {
            Some(Step::Start { next }) => at = next.clone(),
            _ => break,
        }
    }
    at
}

pub fn compile_tree(chart: &Chart, limit: usize) -> Result<Tree, String> {
    if !(1..=MAX_DEPTH).contains(&limit) {
        return Err(format!("Tree depth limit must be 1 to {MAX_DEPTH}."));
    }
    if chart.steps.len() > MAX_NODES {
        return Err(format!("Decision tree has more than {MAX_NODES} nodes."));
    }
    let mut depths = HashMap::new();
    let depth = visit(chart, &chart.start, &mut Vec::new(), &mut depths, limit)?;
    if depths.len() != chart.steps.len() {
        return Err("Decision tree contains unreachable nodes.".into());
    }
    let reached: HashSet<&String> = depths.keys().collect();
    let nodes = chart
        .steps
        .iter()
        .filter(|(id, step)| reached.contains(id) && !matches!(step, Step::Start { .. }))
        .map(|(id, step)| {
            let step = match step {
                Step::Check { key, label, yes, no } => Step::Check { key: key.clone(), label: label.clone(), yes: resolve(chart, yes), no: resolve(chart, no) },
                other => other.clone(),
            };
            (id.clone(), step)
        })
        .collect();
    Ok(Tree { nodes, start: resolve(chart, &chart.start), limit, depth })
}

impl Tree {
    pub fn run(&self, facts: &BTreeMap<String, bool>) -> Result<Run, String> {
        let mut id = self.start.clone();
        let mut path = Vec::new();
        for _ in 0..=self.limit {
            let step = self.nodes.get(&id).ok_or_else(|| format!("Invalid compiled tree node {id}."))?;
            path.push(id.clone());
            match step {
                Step::Action { key, .. } => {
                    let comparisons = path.len() - 1;
                    return Ok(Run { action: key.clone(), path, comparisons });
                }
                Step::Check { key, yes, no, .. } => id = if facts.get(key).copied().unwrap_or(false) { yes.clone() } else { no.clone() },
                Step::Start { .. } => return Err(format!("Invalid compiled tree node {id}.")),
            }
        }
        Err("Compiled decision tree exceeded its depth limit.".into())
    }

    pub fn checks(&self) -> Vec<&str> {
        self.nodes.values().filter_map(|s| if let Step::Check { key, .. } = s { Some(key.as_str()) } else { None }).collect()
    }

    pub fn actions(&self) -> Vec<&str> {
        self.nodes.values().filter_map(|s| if let Step::Action { key, .. } = s { Some(key.as_str()) } else { None }).collect()
    }
}
