use super::cache::{AgentModels, ModelCache, ModelLaunch, Selection};
use super::lists::put_models;
use super::rank::{USER, By, COSTS, ModelRow, available, capabilities_for, check_capabilities, defaults, model_rows, needs_met, rank_models, set_details, sorted_needs};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const LAUNCH_KEEP: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelRequest {
    pub task: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub named: Option<String>,
    pub agent: Option<String>,
    pub from: Option<String>,
    pub fallover: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ModelDecision {
    pub chosen: Option<ModelRow>,
    pub rule: String,
    pub capabilities: Vec<String>,
    pub chain: Vec<String>,
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelEdit {
    pub secondary: Option<String>,
    pub cost: Option<String>,
    pub disabled: Option<bool>,
    pub pinned: Option<bool>,
    pub pins: Option<Vec<String>>,
    pub public_names: Option<Vec<String>>,
    pub company: Option<String>,
    pub plan: Option<String>,
    pub capabilities: Option<Vec<String>>,
}

fn given(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

pub fn needs_of(input: &ModelRequest) -> Vec<String> {
    match &input.capabilities {
        Some(list) => sorted_needs(list),
        None => capabilities_for(given(&input.task).unwrap_or("coding")),
    }
}

fn decision(chosen: Option<&ModelRow>, rule: &str, needs: &[String], chain: Vec<String>, note: String) -> ModelDecision {
    ModelDecision { chosen: chosen.cloned(), rule: rule.into(), capabilities: needs.to_vec(), chain, note }
}

fn best<'a>(rows: &'a [ModelRow], needs: &[String], agent: Option<&str>) -> Option<(&'a ModelRow, &'static str)> {
    let free: Vec<&ModelRow> = rows.iter().filter(|row| agent.is_none_or(|a| row.agent == a) && available(row)).collect();
    let top = free.iter().map(|row| needs_met(row, needs)).max()?;
    let tier: Vec<&ModelRow> = free.into_iter().filter(|row| needs_met(row, needs) == top).collect();
    let first = tier[0];
    let by_agent = agent.is_some();
    if top == needs.len() {
        if let Some(pinned) = tier.iter().find(|row| row.details.pins.iter().any(|cap| needs.contains(cap))) {
            return Some((pinned, if by_agent { "agent-capability-pin" } else { "capability-pin" }));
        }
        let rule = if by_agent {
            "explicit-agent-rank"
        } else if first.details.pinned {
            "pinned-rank"
        } else {
            "capability-rank"
        };
        return Some((first, rule));
    }
    let rule = match (top > 0, by_agent) {
        (true, true) => "agent-most-needs",
        (true, false) => "most-needs-rank",
        (false, true) => "agent-any-free",
        (false, false) => "any-free-model",
    };
    Some((first, rule))
}

fn no_free_why(rows: &[ModelRow], agent: &str) -> String {
    if rows.iter().any(|row| row.agent == agent) {
        format!("all {agent} models are taken out or no longer listed")
    } else {
        format!("{agent} has no models in the model list")
    }
}

pub fn decide_model(cache: &ModelCache, input: &ModelRequest) -> Result<ModelDecision> {
    let rows = model_rows(cache);
    let needs = needs_of(input);
    let agent = given(&input.agent);
    let of_agent = |row: &ModelRow| agent.is_none_or(|a| row.agent == a);
    if let Some(named) = given(&input.named) {
        let found: Vec<&ModelRow> = rows.iter().filter(|row| (row.id == named || row.model == named) && of_agent(row)).collect();
        if found.len() != 1 {
            bail!("Named model {named} is unknown or ambiguous; use agent::model.");
        }
        return Ok(if available(found[0]) {
            decision(Some(found[0]), "explicit-model", &needs, Vec::new(), String::new())
        } else {
            decision(None, "explicit-model-unavailable", &needs, Vec::new(), String::new())
        });
    }
    let by_id: HashMap<&str, &ModelRow> = rows.iter().map(|row| (row.id.as_str(), row)).collect();
    if let Some(from) = given(&input.from) {
        let mut seen: HashSet<String> = HashSet::from([from.to_string()]);
        let mut path: Vec<String> = Vec::new();
        let mut next = by_id.get(from).and_then(|row| row.details.secondary.as_deref()).and_then(|id| by_id.get(id)).copied();
        while let Some(row) = next {
            if !seen.insert(row.id.clone()) {
                break;
            }
            path.push(row.id.clone());
            if of_agent(row) && available(row) {
                let rule = if agent.is_some() { "agent-secondary" } else { "secondary-chain" };
                return Ok(decision(Some(row), rule, &needs, path, String::new()));
            }
            next = row.details.secondary.as_deref().and_then(|id| by_id.get(id)).copied();
        }
        return Ok(decision(None, "secondary-unavailable", &needs, path, String::new()));
    }
    if let Some((row, rule)) = best(&rows, &needs, agent) {
        return Ok(decision(Some(row), rule, &needs, Vec::new(), String::new()));
    }
    let Some(agent) = agent else {
        return Ok(decision(None, "no-free-model", &needs, Vec::new(), String::new()));
    };
    let mut order: Vec<String> = Vec::new();
    for item in &input.fallover {
        if !order.contains(item) {
            order.push(item.clone());
        }
    }
    let next: Vec<String> = match order.iter().position(|a| a == agent) {
        Some(at) => order[at + 1..].iter().chain(order[..at].iter()).cloned().collect(),
        None => order.clone(),
    }
    .into_iter()
    .filter(|a| a != agent)
    .collect();
    let why = no_free_why(&rows, agent);
    let mut tried: Vec<String> = Vec::new();
    for other in &next {
        tried.push(other.clone());
        if let Some((row, _)) = best(&rows, &needs, Some(other)) {
            let mut chain = vec![agent.to_string()];
            chain.extend(tried.iter().cloned());
            let note = format!("{agent} had no free model ({why}), so {other} took the job with {}.", row.id);
            return Ok(decision(Some(row), "agent-fallover", &needs, chain, note));
        }
    }
    if next.is_empty() {
        return Ok(decision(None, "agent-unavailable", &needs, Vec::new(), format!("No free model: {why}.")));
    }
    let mut chain = vec![agent.to_string()];
    chain.extend(tried.iter().cloned());
    let note = format!("No free model: {why}, and {} had none either.", tried.join(", "));
    Ok(decision(None, "no-free-agent", &needs, chain, note))
}

pub fn ensure_map(cache: &mut ModelCache, at: i64, fixed: &[(&str, AgentModels)]) -> Result<()> {
    for (agent, list) in fixed {
        if !cache.agents.contains_key(*agent) {
            put_models(cache, agent, list, at);
        }
    }
    if cache.ranking.is_none() {
        rank_models(cache, &[], at)?;
    }
    let rows = model_rows(cache);
    let mut plans: Vec<(String, Vec<&ModelRow>)> = Vec::new();
    for row in rows.iter().filter(|row| row.listed) {
        match plans.iter_mut().find(|(plan, _)| *plan == row.details.plan) {
            Some((_, group)) => group.push(row),
            None => plans.push((row.details.plan.clone(), vec![row])),
        }
    }
    let rounds = plans.iter().map(|(_, group)| group.len()).max().unwrap_or(0);
    let mut chain: Vec<&ModelRow> = Vec::new();
    for index in 0..rounds {
        for (_, group) in &plans {
            if let Some(row) = group.get(index) {
                chain.push(row);
            }
        }
    }
    let mut secondaries: HashMap<String, String> = HashMap::new();
    let count = chain.len();
    let mut different: Option<&ModelRow> = None;
    for index in (0..count * 2).rev() {
        let row = chain[index % count];
        let next = chain[(index + 1) % count];
        if row.details.plan != next.details.plan {
            different = Some(next);
        }
        if index < count && (different.is_some() || next.id != row.id) {
            secondaries.insert(row.id.clone(), different.unwrap_or(next).id.clone());
        }
    }
    for row in &rows {
        let saved = cache.agents.get(&row.agent).and_then(|list| list.details.get(&row.model)).cloned();
        if let Some(saved) = &saved
            && saved.secondary.is_some()
            && !saved.secondary_auto
        {
            continue;
        }
        let pick = secondaries
            .get(&row.id)
            .cloned()
            .or_else(|| chain.iter().find(|other| other.id != row.id && other.details.plan != row.details.plan).map(|other| other.id.clone()))
            .or_else(|| chain.iter().find(|other| other.id != row.id).map(|other| other.id.clone()));
        let id = pick.filter(|id| *id != row.id).unwrap_or_default();
        if let Some(saved) = &saved
            && saved.secondary_auto
            && saved.secondary.as_deref() == Some(id.as_str())
        {
            continue;
        }
        let Some(list) = cache.agents.get_mut(&row.agent) else {
            continue;
        };
        let mut details = saved.unwrap_or_else(|| defaults(&row.agent, &row.model, &row.efforts));
        details.secondary = Some(id);
        details.secondary_auto = true;
        list.details.insert(row.model.clone(), details);
    }
    Ok(())
}

fn short(value: &str, min: usize, max: usize, what: &str) -> Result<String> {
    let value = value.trim();
    let count = value.chars().count();
    if count < min || count > max {
        bail!("{what} must be {min} to {max} characters.");
    }
    Ok(value.to_string())
}

pub fn edit_model(cache: &mut ModelCache, id: &str, patch: &ModelEdit, by: By) -> Result<()> {
    edit_model_as(cache, id, patch, by, false)
}

pub fn edit_model_as(cache: &mut ModelCache, id: &str, patch: &ModelEdit, by: By, auto: bool) -> Result<()> {
    if by == By::Nixa && !auto && (patch.pinned.is_some() || patch.pins.is_some()) {
        bail!("Only the user changes pins.");
    }
    let rows = model_rows(cache);
    let Some(row) = rows.iter().find(|row| row.id == id) else {
        bail!("No model {id}. Refresh its agent list first.");
    };
    if let Some(secondary) = given(&patch.secondary)
        && !rows.iter().any(|other| other.id == secondary && other.id != id)
    {
        bail!("Choose another model from the table as secondary.");
    }
    if let Some(cost) = &patch.cost
        && !COSTS.contains(&cost.as_str())
    {
        bail!("Cost must be one of: {}.", COSTS.join(", "));
    }
    let pins = patch.pins.as_deref().map(check_capabilities).transpose()?;
    let capabilities = patch.capabilities.as_deref().map(check_capabilities).transpose()?;
    if capabilities.is_some() && by == By::Nixa && !auto && row.details.capabilities_by == USER {
        bail!("The user set these capabilities by hand; only the user changes them.");
    }
    let public_names = match &patch.public_names {
        Some(names) if names.len() > 10 => bail!("Give at most 10 public names."),
        Some(names) => Some(names.iter().map(|n| short(n, 1, 120, "A public name")).collect::<Result<Vec<_>>>()?),
        None => None,
    };
    let company = patch.company.as_deref().map(|c| short(c, 1, 80, "Company")).transpose()?;
    let plan = patch.plan.as_deref().map(|p| short(p, 1, 80, "Plan")).transpose()?;
    set_details(cache, id, |d| {
        if let Some(secondary) = &patch.secondary {
            d.secondary = Some(secondary.trim().to_string());
            d.secondary_auto = false;
        }
        if let Some(cost) = &patch.cost {
            d.cost = cost.clone();
        }
        if let Some(disabled) = patch.disabled {
            d.disabled = disabled;
        }
        if let Some(pinned) = patch.pinned {
            d.pinned = pinned;
        }
        if let Some(pins) = pins {
            d.pins = pins;
        }
        if let Some(names) = public_names {
            d.public_names = names;
        }
        if let Some(company) = company {
            d.company = company;
        }
        if let Some(plan) = plan {
            d.plan = plan;
        }
        if let Some(capabilities) = capabilities {
            d.capabilities_by = if by == By::User && !capabilities.is_empty() { USER.into() } else { String::new() };
            d.capabilities = capabilities;
        }
    })
}

fn or_keep(new: &str, old: &str) -> String {
    if new.is_empty() { old.to_string() } else { new.to_string() }
}

pub fn record_launch(cache: &mut ModelCache, terminal: &str, launch: &ModelLaunch, at: i64) -> Result<()> {
    if let Some(previous) = cache.launches.get(terminal).cloned()
        && previous.model == launch.model
        && previous.task == launch.task
    {
        let merged = ModelLaunch {
            model: previous.model.clone(),
            effort: or_keep(&launch.effort, &previous.effort),
            task: previous.task.clone(),
            capabilities: if launch.capabilities.is_empty() { previous.capabilities.clone() } else { launch.capabilities.clone() },
            selection: Selection {
                agent: or_keep(&previous.selection.agent, &launch.selection.agent),
                named: or_keep(&previous.selection.named, &launch.selection.named),
            },
            folder: or_keep(&launch.folder, &previous.folder),
            space: or_keep(&launch.space, &previous.space),
            name: or_keep(&launch.name, &previous.name),
            member: or_keep(&launch.member, &previous.member),
            at: previous.at,
            handled: previous.handled,
            result: previous.result.clone(),
        };
        cache.launches.insert(terminal.to_string(), merged);
        return Ok(());
    }
    let needs = if launch.capabilities.is_empty() { capabilities_for(&launch.task) } else { launch.capabilities.clone() };
    if let Some(row) = model_rows(cache).into_iter().find(|row| row.id == launch.model) {
        let mut usage = row.details.usage.clone().unwrap_or_default();
        usage.jobs += 1;
        usage.at = at;
        for cap in &needs {
            *usage.capabilities.entry(cap.clone()).or_insert(0) += 1;
        }
        set_details(cache, &row.id, |d| d.usage = Some(usage))?;
    }
    cache.launches.remove(terminal);
    let mut old: Vec<(String, i64)> = cache.launches.iter().map(|(id, l)| (id.clone(), l.at)).collect();
    old.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let drop = old.len().saturating_sub(LAUNCH_KEEP - 1);
    for (id, _) in old.into_iter().take(drop) {
        cache.launches.remove(&id);
    }
    let mut fresh = launch.clone();
    fresh.capabilities = needs;
    fresh.at = at;
    fresh.handled = false;
    fresh.result = String::new();
    cache.launches.insert(terminal.to_string(), fresh);
    Ok(())
}

pub fn launch_result(cache: &mut ModelCache, terminal: &str, good: bool, at: i64) -> Result<bool> {
    let Some(launch) = cache.launches.get(terminal).cloned() else {
        return Ok(false);
    };
    if !launch.result.is_empty() {
        return Ok(false);
    }
    let Some(row) = model_rows(cache).into_iter().find(|row| row.id == launch.model) else {
        return Ok(false);
    };
    let wins = row.details.good + u64::from(good);
    let losses = row.details.bad + u64::from(!good);
    let mut results = row.details.results.clone();
    let needs = if launch.capabilities.is_empty() { capabilities_for(&launch.task) } else { launch.capabilities.clone() };
    for cap in needs {
        let entry = results.entry(cap).or_default();
        entry.good += u64::from(good);
        entry.bad += u64::from(!good);
    }
    set_details(cache, &row.id, |d| {
        d.good = wins;
        d.bad = losses;
        d.own_score = Some(100.0 * wins as f64 / (wins + losses) as f64);
        d.results = results;
    })?;
    if let Some(saved) = cache.launches.get_mut(terminal) {
        saved.result = if good { "good".into() } else { "bad".into() };
    }
    rank_models(cache, &[], at)?;
    Ok(true)
}
