use super::model::{Node, WhoKind};
use super::run::clip;
use nixa_store::agent_map::AgentMapRow;
use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Custom {
    pub id: String,
    pub title: String,
    pub space: String,
    pub off: bool,
    pub claude: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WhoOptions {
    pub map_rows: Vec<AgentMapRow>,
    pub programs: Vec<String>,
    pub customs: Vec<Custom>,
    pub models: Vec<(String, String)>,
    pub space: String,
    pub fallback: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Pick {
    pub assigned_kind: String,
    pub assigned: String,
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub fallback: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WhoPick {
    Assign(Pick),
    Deep { label: String },
    User { label: String },
    Missing { why: String },
}

pub const KIND_AGENT: &str = "agent";
pub const KIND_MEMBER: &str = "member";

fn effort_of(effort: &str) -> String {
    if effort.is_empty() || effort == "default" { String::new() } else { effort.to_string() }
}

fn model_of(agent: &str, model: &str) -> String {
    if model.is_empty() || model == "default" {
        String::new()
    } else if model.contains("::") {
        model.to_string()
    } else {
        format!("{agent}::{model}")
    }
}

fn agent_assign(agent: &str, model: &str, effort: &str, options: &WhoOptions, label: &str, why: &str, fallback: bool) -> WhoPick {
    if !options.programs.iter().any(|p| p == agent) {
        return WhoPick::Missing { why: format!("{agent} is not installed, so {label} cannot start") };
    }
    WhoPick::Assign(Pick {
        assigned_kind: KIND_AGENT.into(),
        assigned: agent.to_string(),
        label: label.to_string(),
        model: model_of(agent, model),
        effort: effort_of(effort),
        fallback,
        why: why.to_string(),
    })
}

fn custom_assign(id: &str, model: &str, effort: &str, options: &WhoOptions, fallback: bool) -> WhoPick {
    let Some(custom) = options.customs.iter().find(|c| c.id == id) else {
        return WhoPick::Missing { why: format!("the custom agent {id} does not exist any more") };
    };
    if custom.off {
        return WhoPick::Missing { why: format!("the custom agent {id} ({}) is turned off", custom.title) };
    }
    if !options.space.is_empty() && !custom.space.is_empty() && custom.space != options.space {
        return WhoPick::Missing { why: format!("the custom agent {id} ({}) belongs to another space", custom.title) };
    }
    WhoPick::Assign(Pick {
        assigned_kind: KIND_MEMBER.into(),
        assigned: id.to_string(),
        label: format!("custom agent {id} ({})", clip(&custom.title, 80)),
        model: if model == "default" { String::new() } else { model.to_string() },
        effort: effort_of(effort),
        fallback,
        ..Pick::default()
    })
}

fn row_assign(node: &Node, options: &WhoOptions, why: &str) -> WhoPick {
    let id = &node.who.id;
    let Some(at) = options.map_rows.iter().position(|row| &row.id == id) else {
        return WhoPick::Missing { why: format!("the agent map has no row {id}") };
    };
    let row = &options.map_rows[at];
    let number = at + 1;
    if row.off {
        return WhoPick::Missing { why: format!("the agent map row {number} is off") };
    }
    if !row.member.is_empty() {
        let owner = if row.agent.is_empty() { String::new() } else { row.agent.clone() };
        let model = if !node.model.is_empty() && !owner.is_empty() { model_of(&owner, &node.model) } else if !row.agent.is_empty() { model_of(&row.agent, &row.model) } else { String::new() };
        let effort = if node.effort.is_empty() { effort_of(&row.effort) } else { effort_of(&node.effort) };
        return WhoPick::Assign(Pick {
            assigned_kind: KIND_MEMBER.into(),
            assigned: row.member.clone(),
            label: node.title.clone(),
            model,
            effort,
            fallback: false,
            why: why.to_string(),
        });
    }
    if !row.agent.is_empty() && !row.model.is_empty() {
        let model = if node.model.is_empty() { row.model.clone() } else { node.model.clone() };
        let effort = if node.effort.is_empty() { row.effort.clone() } else { node.effort.clone() };
        return agent_assign(&row.agent, &model, &effort, options, &node.title, why, false);
    }
    WhoPick::Missing { why: format!("row {number} ({}) is a command or a tool, not an agent start", clip(&row.job, 80)) }
}

fn main_who(node: &Node, options: &WhoOptions, why: &str) -> WhoPick {
    let label = &node.title;
    match node.who.kind {
        WhoKind::Agent if node.who.id.is_empty() => WhoPick::Missing { why: format!("the box {label} names no agent") },
        WhoKind::Agent => agent_assign(&node.who.id, &node.model, &node.effort, options, label, why, false),
        WhoKind::CustomAgent => custom_assign(&node.who.id, &node.model, &node.effort, options, false),
        WhoKind::DeepBrain => WhoPick::Deep { label: label.clone() },
        WhoKind::User => WhoPick::User { label: label.clone() },
        WhoKind::Nixa => match options.programs.first() {
            Some(agent) => agent_assign(agent, &node.model, &node.effort, options, label, "Nixa gives its own box to the first installed agent", false),
            None => WhoPick::Missing { why: format!("no agent is installed to take the box {label}") },
        },
        WhoKind::MapRow => row_assign(node, options, why),
        WhoKind::Model => {
            let wanted = if node.who.id.is_empty() { node.model.clone() } else { node.who.id.clone() };
            let (agent_part, model_part) = match wanted.split_once("::") {
                Some((a, m)) => (a.to_string(), m.to_string()),
                None => (String::new(), wanted.clone()),
            };
            let owner = if agent_part.is_empty() {
                options.models.iter().find(|(_, m)| *m == model_part).map(|(a, _)| a.clone()).unwrap_or_default()
            } else {
                agent_part
            };
            if owner.is_empty() || model_part.is_empty() {
                let shown = if wanted.is_empty() { "(none)".to_string() } else { wanted };
                return WhoPick::Missing { why: format!("no agent has the model {shown}") };
            }
            agent_assign(&owner, &model_part, &node.effort, options, label, why, false)
        }
        WhoKind::Nobody => WhoPick::Missing { why: format!("the box {label} does not say who does it") },
    }
}

pub fn who_of(node: &Node, options: &WhoOptions, why: &str) -> WhoPick {
    let main = if options.fallback { None } else { Some(main_who(node, options, why)) };
    if let Some(found) = &main
        && !matches!(found, WhoPick::Missing { .. })
    {
        return found.clone();
    }
    let fb = &node.fallback;
    if fb.who.kind == WhoKind::Nobody {
        return main.unwrap_or_else(|| WhoPick::Missing { why: format!("the box {} has no fallback", node.title) });
    }
    let backup = if fb.who.kind == WhoKind::Agent {
        agent_assign(&fb.who.id, &fb.model, &fb.effort, options, &format!("{} (fallback)", node.title), why, true)
    } else {
        custom_assign(&fb.who.id, &fb.model, &fb.effort, options, true)
    };
    match (&backup, main) {
        (WhoPick::Missing { why: back }, Some(WhoPick::Missing { why: first })) => WhoPick::Missing { why: format!("{first}, and the fallback failed: {back}") },
        _ => backup,
    }
}
