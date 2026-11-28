use crate::drones::{DroneStep, DroneTest, DroneTree};
use crate::trees::RuleTree;
use crate::workflows::model::{BoxType, Workflow};
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    pub nodes: BTreeMap<String, String>,
    pub edges: Vec<(String, String, String)>,
}

static NODE: &str = r#"([A-Za-z0-9_]+)\s*(?:\(\["(.*?)"\]\)|\(\("(.*?)"\)\)|\["(.*?)"\]|\{"(.*?)"\}|\("(.*?)"\))?"#;
static EDGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"^\s*{NODE}\s*(?:-->|-\.->|==>)\s*(?:\|"?(.*?)"?\|)?\s*{NODE}\s*;?\s*$"#)).unwrap());
static ALONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"^\s*{NODE}\s*;?\s*$"#)).unwrap());

pub fn mid(id: &str) -> String {
    format!("n_{}", id.replace('-', "__"))
}

pub fn real_id(id: &str) -> String {
    id.strip_prefix("n_").unwrap_or(id).replace("__", "-")
}

fn esc(text: &str) -> String {
    text.trim().replace('"', "#quot;").replace('\n', "<br/>")
}

fn unesc(text: &str) -> String {
    text.replace("#quot;", "\"").replace("<br/>", "\n").replace("<br>", "\n").trim().to_string()
}

fn node(id: &str, shape: Shape, text: &str) -> String {
    let (open, close) = match shape {
        Shape::Box => ("[\"", "\"]"),
        Shape::Ask => ("{\"", "\"}"),
        Shape::End => ("([\"", "\"])"),
        Shape::Wait => ("((\"", "\"))"),
    };
    format!("    {}{open}{}{close}", mid(id), esc(text))
}

fn edge(from: &str, word: &str, to: &str) -> String {
    if word.is_empty() {
        format!("    {} --> {}", mid(from), mid(to))
    } else {
        format!("    {} -->|\"{}\"| {}", mid(from), esc(word), mid(to))
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Box,
    Ask,
    End,
    Wait,
}

fn head(kind: &str, name: &str, title: &str) -> Vec<String> {
    vec![format!("%% nixa {kind} {name}: {}", title.replace('\n', " ")), "flowchart TD".to_string()]
}

const OFF_STYLE: &str = "    classDef off stroke-dasharray: 5 5,opacity:0.5";

pub fn tree_mmd(tree: &RuleTree) -> String {
    let mut lines = head("tree", &tree.name, &tree.title);
    for (id, b) in &tree.boxes {
        let shape = if b.report {
            Shape::End
        } else if b.check.is_some() || b.next.as_ref().is_some_and(|n| n.len() > 1) {
            Shape::Ask
        } else if b.next.as_ref().is_none_or(|n| n.is_empty()) {
            Shape::End
        } else {
            Shape::Box
        };
        lines.push(node(id, shape, &b.text));
    }
    for (id, b) in &tree.boxes {
        for (word, to) in b.next.iter().flatten() {
            lines.push(edge(id, word, to));
        }
    }
    lines.join("\n") + "\n"
}

fn test_text(test: &DroneTest) -> String {
    let not = if test.not == Some(true) { "not " } else { "" };
    if let Some(question) = &test.ask {
        return format!("{not}ask: {question}");
    }
    let what = if let Some(v) = &test.is {
        format!("is {not}{v}")
    } else if let Some(list) = &test.within {
        format!("is {not}one of {}", list.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "))
    } else if let Some(n) = test.over {
        format!("{not}over {n}")
    } else if let Some(n) = test.under {
        format!("{not}under {n}")
    } else {
        format!("{not}set")
    };
    format!("{} {what}", test.fact)
}

pub fn drone_text(step: &DroneStep) -> String {
    if let Some(label) = step.label() {
        return label.to_string();
    }
    match step {
        DroneStep::If { test, .. } => test_text(test),
        DroneStep::Switch { fact, .. } => format!("Which {fact}?"),
        DroneStep::All { tests, .. } => format!("All: {}", tests.iter().map(test_text).collect::<Vec<_>>().join(" and ")),
        DroneStep::Any { tests, .. } => format!("Any: {}", tests.iter().map(test_text).collect::<Vec<_>>().join(" or ")),
        DroneStep::Wait { .. } => "Wait".to_string(),
        DroneStep::Report { text, .. } => text.clone(),
    }
}

pub fn drone_mmd(tree: &DroneTree) -> String {
    let mut lines = head("drones", "drones", "Drone checks over tasks and terminals");
    let mut off = Vec::new();
    for (id, step) in &tree.steps {
        let shape = match step {
            DroneStep::Wait { .. } => Shape::Wait,
            DroneStep::Report { .. } => Shape::End,
            _ => Shape::Ask,
        };
        lines.push(node(id, shape, &drone_text(step)));
        if step.is_off() {
            off.push(mid(id));
        }
    }
    for (id, step) in &tree.steps {
        match step {
            DroneStep::If { then, otherwise, .. } | DroneStep::All { then, otherwise, .. } | DroneStep::Any { then, otherwise, .. } => {
                lines.push(edge(id, "yes", then));
                lines.push(edge(id, "no", otherwise));
            }
            DroneStep::Switch { cases, default, .. } => {
                for (word, to) in cases {
                    lines.push(edge(id, if word.is_empty() { "empty" } else { word }, to));
                }
                lines.push(edge(id, "other", default));
            }
            DroneStep::Wait { .. } | DroneStep::Report { .. } => {}
        }
    }
    if !off.is_empty() {
        lines.push(OFF_STYLE.to_string());
        lines.push(format!("    class {} off", off.join(",")));
    }
    lines.join("\n") + "\n"
}

pub fn workflow_mmd(flow: &Workflow) -> String {
    let mut lines = head("workflow", &flow.name, &flow.title);
    let mut off = Vec::new();
    for (id, n) in &flow.nodes {
        let shape = match n.kind {
            BoxType::Start | BoxType::End => Shape::End,
            BoxType::Stage => Shape::Box,
        };
        lines.push(node(id, shape, &n.title));
        if n.off {
            off.push(mid(id));
        }
    }
    for l in &flow.links {
        lines.push(edge(&l.from, l.kind.word(), &l.to));
    }
    if !off.is_empty() {
        lines.push(OFF_STYLE.to_string());
        lines.push(format!("    class {} off", off.join(",")));
    }
    lines.join("\n") + "\n"
}

fn label_of(caps: &regex::Captures, first: usize) -> Option<String> {
    (first..first + 5).find_map(|i| caps.get(i)).map(|m| unesc(m.as_str()))
}

fn keep(nodes: &mut BTreeMap<String, String>, id: &str, label: Option<String>) {
    let id = real_id(id);
    match label {
        Some(text) => {
            nodes.insert(id, text);
        }
        None => {
            nodes.entry(id.clone()).or_insert(id);
        }
    }
}

pub fn parse(source: &str) -> Result<Parsed, String> {
    let mut out = Parsed::default();
    let mut seen_chart = false;
    for (n, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") || line.starts_with("classDef ") || line.starts_with("class ") || line.starts_with("style ") || line.starts_with("linkStyle ") {
            continue;
        }
        if line.starts_with("flowchart") || line.starts_with("graph") {
            seen_chart = true;
            continue;
        }
        if let Some(c) = EDGE.captures(line) {
            keep(&mut out.nodes, &c[1], label_of(&c, 2));
            let word = c.get(7).map(|m| unesc(m.as_str())).unwrap_or_default();
            keep(&mut out.nodes, &c[8], label_of(&c, 9));
            out.edges.push((real_id(&c[1]), word, real_id(&c[8])));
            continue;
        }
        if let Some(c) = ALONE.captures(line) {
            keep(&mut out.nodes, &c[1], label_of(&c, 2));
            continue;
        }
        return Err(format!("Line {} is not a box or an arrow Nixa can read: {line}", n + 1));
    }
    if !seen_chart {
        return Err("This is not a Mermaid flowchart. The first line must be flowchart TD.".into());
    }
    Ok(out)
}
