use super::cache::ModelCache;
use super::rank::{model_rows, rank_models, set_details};
use crate::time::{DAY_MS, HOUR_MS};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

pub const PUBLIC_SCORES_URL: &str = "https://raw.githubusercontent.com/SWE-bench/swe-bench.github.io/master/data/leaderboards.json";
pub const PUBLIC_SOURCE: &str = "SWE-bench Verified (best reported agent run)";
pub const PUBLIC_MAX_AGE_MS: i64 = 7 * DAY_MS;
pub const PUBLIC_RETRY_MS: i64 = HOUR_MS;
pub const PUBLIC_MAX_BYTES: usize = 30_000_000;

pub fn canonical(name: &str) -> String {
    let last = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    let mut words: Vec<String> = Vec::new();
    let mut digits: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut flush = |current: &mut String| {
        if current.is_empty() {
            return;
        }
        if current.starts_with(|c: char| c.is_ascii_digit()) {
            digits.push(std::mem::take(current));
        } else {
            words.push(std::mem::take(current));
        }
    };
    for c in last.chars() {
        let same = current.chars().next().is_some_and(|first| first.is_ascii_digit() == c.is_ascii_digit());
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if !current.is_empty() && !same {
                flush(&mut current);
            }
            current.push(c);
        } else {
            flush(&mut current);
        }
    }
    flush(&mut current);
    words.sort();
    format!("{}|{}", words.join(" "), digits.join("."))
}

fn skipped_name(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    lower.is_empty() || matches!(lower.as_str(), "multiple" | "unknown" | "undisclosed")
}

pub fn public_scores(raw: &Value) -> HashMap<String, f64> {
    let mut out: HashMap<String, f64> = HashMap::new();
    let boards = raw.get("leaderboards").and_then(Value::as_array);
    let verified = boards.and_then(|list| list.iter().find(|b| b.get("name").and_then(Value::as_str) == Some("Verified")));
    for result in verified.and_then(|b| b.get("results")).and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = result.get("model_display").and_then(Value::as_str) else {
            continue;
        };
        let Some(score) = result.get("resolved").and_then(Value::as_f64) else {
            continue;
        };
        if skipped_name(name) || !score.is_finite() || !(0.0..=100.0).contains(&score) {
            continue;
        }
        let key = canonical(name);
        let best = out.get(&key).copied().unwrap_or(0.0).max(score);
        out.insert(key, best);
    }
    out
}

pub fn refresh_due(cache: &ModelCache, now: i64, force: bool) -> bool {
    let recent = &cache.public_refresh;
    if force || recent.attempt_at == 0 {
        return true;
    }
    let failed = recent.attempt_at != recent.at;
    now - recent.attempt_at >= if failed { PUBLIC_RETRY_MS } else { PUBLIC_MAX_AGE_MS }
}

pub fn mark_attempt(cache: &mut ModelCache, now: i64) {
    cache.public_refresh.attempt_at = now;
}

pub fn apply_scores(cache: &mut ModelCache, scores: &HashMap<String, f64>, now: i64) -> Result<usize> {
    let mut changed = 0;
    for row in model_rows(cache) {
        let found = std::iter::once(&row.model).chain(row.details.public_names.iter()).map(|n| canonical(n)).find_map(|n| scores.get(&n).copied());
        if let Some(score) = found {
            set_details(cache, &row.id, |d| {
                d.public_score = Some(score);
                d.public_source = PUBLIC_SOURCE.into();
                d.public_at = now;
            })?;
            changed += 1;
        }
    }
    cache.public_refresh.at = now;
    cache.public_refresh.attempt_at = now;
    rank_models(cache, &[], now)?;
    Ok(changed)
}
