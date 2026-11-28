use super::cache::{CachedModels, ModelCache, ModelDetails, efforts_for, first_effort, guess_company, guess_plan, is_zen_model, model_id, ordered_efforts, split_id};
use anyhow::{Result, bail};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::LazyLock;

pub const CAPABILITIES: [&str; 10] = ["planning", "coding", "refactor", "ui", "testing", "cheap", "big-context", "vision", "long-reasoning", "speed"];
pub const COSTS: [&str; 3] = ["cheap", "standard", "premium"];
pub const USER: &str = "user";
pub const NIXA: &str = "Nixa";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    User,
    Nixa,
}

impl By {
    pub fn as_str(self) -> &'static str {
        match self {
            By::User => USER,
            By::Nixa => NIXA,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelRow {
    pub id: String,
    pub agent: String,
    pub model: String,
    pub efforts: Vec<String>,
    pub rank: usize,
    pub listed: bool,
    #[serde(flatten)]
    pub details: ModelDetails,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub decision: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Combo {
    pub agent: String,
    pub model: String,
    pub effort: String,
    pub company: String,
    pub plan: String,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("model pattern")
}

static CHEAP: LazyLock<Regex> = LazyLock::new(|| re("(?i)haiku|flash|mini|nano"));
static PREMIUM: LazyLock<Regex> = LazyLock::new(|| re("(?i)opus|pro|max"));
static LONG: LazyLock<Regex> = LazyLock::new(|| re("high|max"));
static BIG: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)claude|gpt-(?:4\.1|5|6)|gemini|kimi|deepseek-v[34]|glm-[45]"));
static VISION: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)claude-(?:opus|sonnet|haiku)-[345]|gpt-(?:4o|4\.1|5|6)|gemini|kimi-k(?:2[.-]5|3)"));
static CAP_WORDS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    CAPABILITIES.iter().map(|cap| (*cap, re(&format!(r"(?i)\b{}\b", cap.replace('-', "[ -]"))))).collect()
});
static TASK_HINTS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (re("(?i)design|frontend|interface"), "ui"),
        (re("(?i)test|qa|verify"), "testing"),
        (re("(?i)plan|architect"), "planning"),
        (re("(?i)cheap"), "cheap"),
        (re("(?i)refactoring|restructure"), "refactor"),
        (re(r"(?i)\bfast\b|low latency"), "speed"),
        (re("(?i)screenshot|image|visual|photo|diagram"), "vision"),
        (re(r"(?i)large context|whole (?:repo|codebase)|long document"), "big-context"),
    ]
});

pub fn is_capability(word: &str) -> bool {
    CAPABILITIES.contains(&word)
}

pub fn check_capabilities(list: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for item in list {
        let word = item.trim();
        if !is_capability(word) {
            bail!("Unknown capability {word}. Use one of: {}.", CAPABILITIES.join(", "));
        }
        if !out.iter().any(|w| w == word) {
            out.push(word.to_string());
        }
    }
    Ok(out)
}

pub fn sorted_needs(list: &[String]) -> Vec<String> {
    let mut out = list.to_vec();
    out.sort();
    out.dedup();
    out
}

pub fn defaults(agent: &str, model: &str, efforts: &[String]) -> ModelDetails {
    let cheap = CHEAP.is_match(model);
    let plan = guess_plan(agent, model).unwrap_or_else(|| if model.starts_with("opencode-go/") { "OpenCode Go plan".into() } else { "Unknown".into() });
    let mut capabilities: Vec<String> = ["planning", "coding", "refactor", "ui", "testing"].iter().map(|s| s.to_string()).collect();
    if cheap {
        capabilities.push("cheap".into());
        capabilities.push("speed".into());
    }
    if efforts.iter().any(|e| LONG.is_match(e)) {
        capabilities.push("long-reasoning".into());
    }
    if BIG.is_match(model) {
        capabilities.push("big-context".into());
    }
    if VISION.is_match(model) {
        capabilities.push("vision".into());
    }
    let cost = if cheap { "cheap" } else if PREMIUM.is_match(model) { "premium" } else { "standard" };
    ModelDetails {
        company: guess_company(agent, model).unwrap_or_else(|| "Unknown".into()),
        plan,
        capabilities,
        cost: cost.into(),
        ..ModelDetails::default()
    }
}

pub fn join_capabilities(base: &[String], saved: Option<&ModelDetails>) -> Vec<String> {
    if let Some(saved) = saved
        && saved.capabilities_by == USER
    {
        return saved.capabilities.clone();
    }
    let have = |cap: &str| base.iter().any(|c| c == cap) || saved.is_some_and(|s| s.capabilities.iter().any(|c| c == cap));
    CAPABILITIES.iter().filter(|cap| have(cap)).map(|cap| cap.to_string()).collect()
}

fn fill(mut details: ModelDetails, base: &ModelDetails) -> ModelDetails {
    if details.company.is_empty() {
        details.company = base.company.clone();
    }
    if details.plan.is_empty() {
        details.plan = base.plan.clone();
    }
    if details.cost.is_empty() {
        details.cost = base.cost.clone();
    }
    details
}

fn row_of(agent: &str, list: &CachedModels, model: &str) -> ModelRow {
    let efforts = efforts_for(list, model);
    let base = defaults(agent, model, &efforts);
    let saved = list.details.get(model);
    let mut details = saved.cloned().map(|d| fill(d, &base)).unwrap_or_else(|| base.clone());
    details.capabilities = join_capabilities(&base.capabilities, saved);
    ModelRow {
        id: model_id(agent, model),
        agent: agent.to_string(),
        model: model.to_string(),
        efforts: ordered_efforts(&efforts),
        rank: 0,
        listed: list.models.iter().any(|m| m == model),
        details,
        decision: String::new(),
        note: String::new(),
    }
}

pub fn model_rows(cache: &ModelCache) -> Vec<ModelRow> {
    let mut all: Vec<ModelRow> = Vec::new();
    for (agent, list) in &cache.agents {
        let mut names: Vec<&String> = list.models.iter().chain(list.details.keys()).filter(|m| !is_zen_model(agent, m)).collect();
        names.sort();
        names.dedup();
        all.extend(names.into_iter().map(|model| row_of(agent, list, model)));
    }
    let mut positions: HashMap<&str, usize> = HashMap::new();
    for id in cache.ranking.iter().flatten() {
        let next = positions.len();
        positions.entry(id.as_str()).or_insert(next);
    }
    let last = positions.len();
    let place = |row: &ModelRow| positions.get(row.id.as_str()).copied().unwrap_or(last);
    all.sort_by(|a, b| place(a).cmp(&place(b)).then_with(|| a.id.cmp(&b.id)));
    for (index, row) in all.iter_mut().enumerate() {
        row.rank = index + 1;
    }
    all
}

pub fn set_details(cache: &mut ModelCache, id: &str, patch: impl FnOnce(&mut ModelDetails)) -> Result<()> {
    let Some((agent, model)) = split_id(id) else {
        bail!("No model {id}. Refresh its agent list first.");
    };
    let Some(list) = cache.agents.get_mut(agent) else {
        bail!("No model {id}. Refresh its agent list first.");
    };
    if !list.models.iter().any(|m| m == model) && !list.details.contains_key(model) {
        bail!("No model {id}. Refresh its agent list first.");
    }
    let base = defaults(agent, model, &efforts_for(list, model));
    let mut details = list.details.get(model).cloned().map(|d| fill(d, &base)).unwrap_or(base);
    patch(&mut details);
    list.details.insert(model.to_string(), details);
    Ok(())
}

pub fn available(row: &ModelRow) -> bool {
    row.listed && !row.details.disabled
}

pub fn capabilities_for(task: &str) -> Vec<String> {
    let mut found: Vec<String> = CAP_WORDS.iter().filter(|(_, word)| word.is_match(task)).map(|(cap, _)| cap.to_string()).collect();
    for (hint, cap) in TASK_HINTS.iter() {
        if hint.is_match(task) {
            found.push(cap.to_string());
        }
    }
    if found.is_empty() {
        found.push("coding".into());
    }
    sorted_needs(&found)
}

pub fn needs_met(row: &ModelRow, needs: &[String]) -> usize {
    needs.iter().filter(|cap| row.details.capabilities.contains(cap)).count()
}

pub fn model_ability(row: &ModelRow, needs: &[String]) -> f64 {
    let (good, bad) = needs
        .iter()
        .filter_map(|cap| row.details.results.get(cap))
        .fold((0u64, 0u64), |(g, b), r| (g + r.good, b + r.bad));
    let own = if good + bad > 0 { 100.0 * good as f64 / (good + bad) as f64 } else { row.details.own_score.unwrap_or(50.0) };
    row.details.public_score.unwrap_or(50.0) * 0.6 + own * 0.4
}

fn cost_penalty(cost: &str) -> f64 {
    match cost {
        "cheap" => 0.0,
        "premium" => 8.0,
        _ => 3.0,
    }
}

pub fn model_score(row: &ModelRow, needs: &[String]) -> f64 {
    let base = if available(row) { 0.0 } else { -1000.0 };
    base + needs_met(row, needs) as f64 * 100.0 + model_ability(row, needs) - cost_penalty(&row.details.cost)
}

pub fn rank_models(cache: &mut ModelCache, needs: &[String], at: i64) -> Result<()> {
    let rows = model_rows(cache);
    let mut free: Vec<&ModelRow> = rows.iter().filter(|row| !row.details.pinned).collect();
    free.sort_by(|a, b| {
        model_score(b, needs)
            .partial_cmp(&model_score(a, needs))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.rank.cmp(&b.rank))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut next = free.into_iter();
    let sorted: Vec<&ModelRow> = rows.iter().map(|row| if row.details.pinned { row } else { next.next().unwrap_or(row) }).collect();
    cache.ranking = Some(sorted.iter().map(|row| row.id.clone()).collect());
    for (index, row) in sorted.iter().enumerate() {
        if row.id != rows[index].id && !row.details.pinned {
            set_details(cache, &row.id, |d| {
                d.moved_by = NIXA.into();
                d.moved_at = at;
            })?;
        }
    }
    Ok(())
}

pub fn move_model(cache: &mut ModelCache, id: &str, rank: usize, by: By, at: i64) -> Result<()> {
    move_model_as(cache, id, rank, by, false, at)
}

pub fn move_model_as(cache: &mut ModelCache, id: &str, rank: usize, by: By, auto: bool, at: i64) -> Result<()> {
    let rows = model_rows(cache);
    let Some(row) = rows.iter().find(|row| row.id == id) else {
        bail!("No model {id}.");
    };
    if rank < 1 || rank > rows.len() {
        bail!("Rank is outside the table.");
    }
    let free_pins = by == By::Nixa && auto;
    if by == By::Nixa && !free_pins && row.details.pinned {
        bail!("The user pinned this row; only the user can move it.");
    }
    let slots: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].id == id || free_pins || !rows[i].details.pinned).collect();
    let Some(target) = slots.iter().position(|&i| i == rank - 1) else {
        bail!("The user pinned that position.");
    };
    let mut free: Vec<&ModelRow> = slots.iter().map(|&i| &rows[i]).collect();
    if let Some(from) = free.iter().position(|r| r.id == id) {
        let moving = free.remove(from);
        free.insert(target, moving);
    }
    let mut ordered: Vec<&ModelRow> = rows.iter().collect();
    for (k, &slot) in slots.iter().enumerate() {
        ordered[slot] = free[k];
    }
    cache.ranking = Some(ordered.iter().map(|row| row.id.clone()).collect());
    for (index, row) in ordered.iter().enumerate() {
        if row.id != rows[index].id || row.id == id {
            let pin = row.id == id && by == By::User;
            set_details(cache, &row.id, |d| {
                d.moved_by = by.as_str().into();
                d.moved_at = at;
                if pin {
                    d.pinned = true;
                }
            })?;
        }
    }
    Ok(())
}

pub fn row_combo(row: &ModelRow) -> Combo {
    Combo {
        agent: row.agent.clone(),
        model: row.model.clone(),
        effort: first_effort(&row.efforts),
        company: row.details.company.clone(),
        plan: row.details.plan.clone(),
    }
}
