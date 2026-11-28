use super::cache::ModelCache;
use super::pick::{ModelRequest, decide_model};
use super::rank::{ModelRow, available, model_rows};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

pub const LEADERBOARD_CAPABILITIES: [&str; 5] = ["planning", "coding", "ui", "testing", "cheap"];

static PLAN_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bplan\b").expect("plan pattern"));
static NOT_INCLUDED: LazyLock<Regex> = LazyLock::new(|| Regex::new("(?i)unknown|api|metered").expect("plan pattern"));

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlanUsage {
    pub plan: String,
    pub companies: Vec<String>,
    pub included: bool,
    pub jobs: u64,
    pub good: u64,
    pub bad: u64,
    pub available: usize,
    pub models: usize,
    pub last_at: i64,
    pub capabilities: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BoardRow {
    #[serde(flatten)]
    pub row: ModelRow,
    pub secondary_chain: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BoardChoice {
    pub capability: String,
    pub chosen: Option<ModelRow>,
    pub rule: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Leaderboard {
    pub rows: Vec<BoardRow>,
    pub listed: usize,
    pub pinned: usize,
    pub choices: Vec<BoardChoice>,
}

pub fn included_plan(plan: &str) -> bool {
    PLAN_WORD.is_match(plan) && !NOT_INCLUDED.is_match(plan)
}

pub fn plan_usage(rows: &[ModelRow]) -> Vec<PlanUsage> {
    let mut plans: BTreeMap<String, PlanUsage> = BTreeMap::new();
    for row in rows {
        let plan = plans.entry(row.details.plan.clone()).or_insert_with(|| PlanUsage {
            plan: row.details.plan.clone(),
            included: included_plan(&row.details.plan),
            ..PlanUsage::default()
        });
        if !plan.companies.contains(&row.details.company) {
            plan.companies.push(row.details.company.clone());
        }
        let usage = row.details.usage.clone().unwrap_or_default();
        plan.jobs += usage.jobs;
        plan.good += row.details.good;
        plan.bad += row.details.bad;
        plan.models += 1;
        if available(row) {
            plan.available += 1;
        }
        if usage.at > plan.last_at {
            plan.last_at = usage.at;
        }
        for (cap, count) in usage.capabilities {
            *plan.capabilities.entry(cap).or_insert(0) += count;
        }
    }
    plans
        .into_values()
        .map(|mut plan| {
            plan.companies.sort();
            plan
        })
        .collect()
}

fn chain_for(by_id: &HashMap<&str, &ModelRow>, id: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::from([id.to_string()]);
    let mut chain: Vec<String> = Vec::new();
    let mut next = by_id.get(id).and_then(|row| row.details.secondary.clone()).filter(|s| !s.is_empty());
    while let Some(key) = next {
        if !seen.insert(key.clone()) {
            chain.push(format!("{key} (cycle)"));
            break;
        }
        let Some(row) = by_id.get(key.as_str()) else {
            chain.push(format!("{key} (unlisted reference)"));
            break;
        };
        chain.push(key);
        next = row.details.secondary.clone().filter(|s| !s.is_empty());
    }
    chain
}

pub fn leaderboard(cache: &ModelCache) -> Leaderboard {
    let rows = model_rows(cache);
    let by_id: HashMap<&str, &ModelRow> = rows.iter().map(|row| (row.id.as_str(), row)).collect();
    let board: Vec<BoardRow> = rows.iter().map(|row| BoardRow { row: row.clone(), secondary_chain: chain_for(&by_id, &row.id) }).collect();
    let choices = LEADERBOARD_CAPABILITIES
        .iter()
        .map(|cap| {
            let request = ModelRequest { capabilities: Some(vec![cap.to_string()]), ..ModelRequest::default() };
            let decision = decide_model(cache, &request).ok();
            BoardChoice {
                capability: cap.to_string(),
                chosen: decision.as_ref().and_then(|d| d.chosen.clone()),
                rule: decision.map(|d| d.rule).unwrap_or_default(),
            }
        })
        .collect();
    Leaderboard {
        listed: rows.iter().filter(|row| row.listed).count(),
        pinned: rows.iter().filter(|row| row.details.pinned).count(),
        rows: board,
        choices,
    }
}
