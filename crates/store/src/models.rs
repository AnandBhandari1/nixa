pub mod act;
pub mod cache;
pub mod lists;
pub mod map;
pub mod pick;
pub mod public;
pub mod rank;
pub mod start;
#[cfg(test)]
mod tests;
pub mod usage;

pub use act::{ACTIONS, ModelArgs, act, action_name, list_value, parse_args, view_value};
pub use cache::{AgentModels, CachedModels, ModelCache, ModelDetails, ModelLaunch, PublicRefresh, Selection};
pub use lists::{ListResult, LIST_AGENTS, model_list, refresh_list, run_list_command};
pub use pick::{ModelDecision, ModelEdit, ModelRequest, decide_model};
pub use rank::{By, CAPABILITIES, ModelRow, capabilities_for, model_rows};
pub use start::{Prepared, Program, prepare_start, record_start};
pub use usage::{Leaderboard, PlanUsage, leaderboard, plan_usage};

use crate::settings::{get_setting, set_setting};
use crate::time::now_ms;
use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

pub const SQL: &str = r#"
CREATE TABLE IF NOT EXISTS model_lists (
    agent TEXT PRIMARY KEY,
    at INTEGER NOT NULL DEFAULT 0,
    models TEXT NOT NULL DEFAULT '[]',
    efforts TEXT NOT NULL DEFAULT '{}',
    error TEXT NOT NULL DEFAULT '',
    details TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS model_launches (
    terminal TEXT PRIMARY KEY,
    at INTEGER NOT NULL,
    data TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS model_decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    model TEXT NOT NULL DEFAULT '',
    rule TEXT NOT NULL,
    line TEXT NOT NULL
);
"#;

pub const RANKING_KEY: &str = "models.ranking";
pub const PUBLIC_KEY: &str = "models.public_refresh";
pub const WANTED_KEY: &str = "models.public_wanted";
pub const DECISION_KEEP: i64 = 2000;

fn json_text<T: serde::Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

pub fn read(conn: &Connection) -> Result<ModelCache> {
    let mut cache = ModelCache::default();
    let mut stmt = conn.prepare("SELECT agent, at, models, efforts, error, details FROM model_lists ORDER BY agent")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?))
    })?;
    for row in rows {
        let (agent, at, models, efforts, error, details) = row?;
        let entry = CachedModels {
            at,
            models: serde_json::from_str(&models).unwrap_or_default(),
            efforts: serde_json::from_str(&efforts).unwrap_or_default(),
            error,
            details: serde_json::from_str(&details).unwrap_or_default(),
        };
        cache.agents.insert(agent, entry);
    }
    cache.ranking = get_setting(conn, RANKING_KEY)?.and_then(|v| serde_json::from_value(v).ok());
    cache.public_refresh = get_setting(conn, PUBLIC_KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let mut stmt = conn.prepare("SELECT terminal, data FROM model_launches")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    for row in rows {
        let (terminal, data) = row?;
        if let Ok(launch) = serde_json::from_str::<ModelLaunch>(&data) {
            cache.launches.insert(terminal, launch);
        }
    }
    Ok(cache)
}

fn write(conn: &Connection, before: &ModelCache, after: &ModelCache) -> Result<()> {
    for (agent, entry) in &after.agents {
        if before.agents.get(agent) == Some(entry) {
            continue;
        }
        conn.execute(
            "INSERT INTO model_lists (agent, at, models, efforts, error, details) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(agent) DO UPDATE SET at = excluded.at, models = excluded.models, efforts = excluded.efforts, error = excluded.error, details = excluded.details",
            params![agent, entry.at, json_text(&entry.models)?, json_text(&entry.efforts)?, entry.error, json_text(&entry.details)?],
        )?;
    }
    for agent in before.agents.keys().filter(|a| !after.agents.contains_key(*a)) {
        conn.execute("DELETE FROM model_lists WHERE agent = ?1", params![agent])?;
    }
    if before.ranking != after.ranking {
        set_setting(conn, RANKING_KEY, &serde_json::to_value(&after.ranking)?)?;
    }
    if before.public_refresh != after.public_refresh {
        set_setting(conn, PUBLIC_KEY, &serde_json::to_value(&after.public_refresh)?)?;
    }
    if before.launches != after.launches {
        conn.execute("DELETE FROM model_launches", [])?;
        for (terminal, launch) in &after.launches {
            conn.execute("INSERT INTO model_launches (terminal, at, data) VALUES (?1, ?2, ?3)", params![terminal, launch.at, json_text(launch)?])?;
        }
    }
    Ok(())
}

pub fn update(conn: &Connection, change: impl FnOnce(&mut ModelCache) -> Result<()>) -> Result<ModelCache> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate).context("the model table is busy; try again")?;
    let before = read(&tx)?;
    let mut cache = before.clone();
    pick::ensure_map(&mut cache, now_ms(), &lists::installed_fixed())?;
    change(&mut cache)?;
    if cache != before {
        write(&tx, &before, &cache)?;
    }
    tx.commit()?;
    Ok(cache)
}

pub fn load(conn: &Connection) -> Result<ModelCache> {
    let cache = read(conn)?;
    let mut ensured = cache.clone();
    pick::ensure_map(&mut ensured, now_ms(), &lists::installed_fixed())?;
    if ensured == cache {
        return Ok(cache);
    }
    update(conn, |_| Ok(()))
}

pub fn record_decision(conn: &Connection, request: &ModelRequest, decision: &ModelDecision) -> Result<()> {
    let model = decision.chosen.as_ref().map(|row| row.id.clone());
    let mut line = json!({"model": model, "rule": decision.rule, "capabilities": decision.capabilities});
    let extra: [(&str, Option<&String>); 3] = [("named", request.named.as_ref()), ("agent", request.agent.as_ref()), ("from", request.from.as_ref())];
    for (key, value) in extra {
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            line[key] = json!(value);
        }
    }
    if !decision.chain.is_empty() {
        line["chain"] = json!(decision.chain);
    }
    if !decision.note.is_empty() {
        line["note"] = json!(decision.note);
    }
    let at = now_ms();
    line["at"] = json!(at);
    conn.execute(
        "INSERT INTO model_decisions (at, model, rule, line) VALUES (?1, ?2, ?3, ?4)",
        params![at, model.unwrap_or_default(), decision.rule, line.to_string()],
    )?;
    conn.execute("DELETE FROM model_decisions WHERE id <= (SELECT MAX(id) FROM model_decisions) - ?1", params![DECISION_KEEP])?;
    Ok(())
}

pub fn decisions(conn: &Connection, limit: usize) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT line FROM model_decisions ORDER BY id DESC LIMIT ?1")?;
    let rows = stmt.query_map(params![limit as i64], |row| row.get::<_, String>(0))?;
    let mut out: Vec<Value> = rows.filter_map(|r| r.ok()).filter_map(|t| serde_json::from_str(&t).ok()).collect();
    out.reverse();
    Ok(out)
}

pub fn choose(conn: &Connection, request: &ModelRequest) -> Result<Option<ModelRow>> {
    let cache = load(conn)?;
    let decision = match decide_model(&cache, request) {
        Ok(decision) => decision,
        Err(error) => {
            let failed = ModelDecision { rule: "invalid-explicit-model".into(), capabilities: pick::needs_of(request), ..ModelDecision::default() };
            record_decision(conn, request, &failed)?;
            return Err(error);
        }
    };
    record_decision(conn, request, &decision)?;
    Ok(decision.chosen.map(|mut row| {
        row.decision = decision.rule.clone();
        row.note = decision.note.clone();
        row
    }))
}

pub fn secondary(conn: &Connection, agent: &str, model: &str) -> Result<Option<rank::Combo>> {
    let rows = model_rows(&load(conn)?);
    let Some(row) = rows.iter().find(|row| row.agent == agent && row.model == model) else {
        return Ok(None);
    };
    let request = ModelRequest { from: Some(row.id.clone()), capabilities: Some(Vec::new()), ..ModelRequest::default() };
    Ok(choose(conn, &request)?.map(|row| rank::row_combo(&row)))
}

pub fn edit(conn: &Connection, id: &str, patch: &ModelEdit, by: By) -> Result<ModelCache> {
    let auto = by == By::Nixa;
    let cache = update(conn, |cache| pick::edit_model_as(cache, id, patch, by, auto))?;
    if auto && user_fields(patch) {
        let chosen = model_rows(&cache).into_iter().find(|row| row.id == id);
        let decision = ModelDecision { chosen, rule: "nixa-automatic-edit".into(), note: "Nixa changed pins or capabilities.".into(), ..ModelDecision::default() };
        record_decision(conn, &ModelRequest::default(), &decision)?;
    }
    Ok(cache)
}

fn user_fields(patch: &ModelEdit) -> bool {
    patch.pinned.is_some() || patch.pins.is_some() || patch.capabilities.is_some()
}

pub fn move_to(conn: &Connection, id: &str, rank: usize, by: By) -> Result<ModelCache> {
    let auto = by == By::Nixa;
    let cache = update(conn, |cache| rank::move_model_as(cache, id, rank, by, auto, now_ms()))?;
    let chosen = model_rows(&cache).into_iter().find(|row| row.id == id);
    let rule = format!("{}-rank-move:{rank}", by.as_str().to_lowercase());
    let note = if auto { "Nixa may move pinned rows.".to_string() } else { String::new() };
    record_decision(conn, &ModelRequest::default(), &ModelDecision { chosen, rule, note, ..ModelDecision::default() })?;
    Ok(cache)
}

pub fn rank(conn: &Connection, needs: &[String]) -> Result<ModelCache> {
    let mut before: Vec<String> = Vec::new();
    let cache = update(conn, |cache| {
        before = model_rows(cache).into_iter().map(|row| row.id).collect();
        rank::rank_models(cache, needs, now_ms())
    })?;
    let sorted = rank::sorted_needs(needs);
    let request = ModelRequest { capabilities: Some(needs.to_vec()), ..ModelRequest::default() };
    for row in model_rows(&cache) {
        if before.get(row.rank - 1) != Some(&row.id) {
            let rule = format!("automatic-rank:{}", row.rank);
            record_decision(conn, &request, &ModelDecision { chosen: Some(row), rule, capabilities: sorted.clone(), ..ModelDecision::default() })?;
        }
    }
    Ok(cache)
}

pub fn record(conn: &Connection, terminal: &str, launch: &ModelLaunch) -> Result<()> {
    update(conn, |cache| pick::record_launch(cache, terminal, launch, now_ms()))?;
    Ok(())
}

pub fn result(conn: &Connection, terminal: &str, good: bool) -> Result<bool> {
    let mut done = false;
    update(conn, |cache| {
        done = pick::launch_result(cache, terminal, good, now_ms())?;
        Ok(())
    })?;
    Ok(done)
}

pub fn launch_of(conn: &Connection, terminal: &str) -> Result<Option<ModelLaunch>> {
    Ok(read(conn)?.launches.remove(terminal))
}

pub fn want_public(conn: &Connection) -> Result<()> {
    set_setting(conn, WANTED_KEY, &json!(true))
}

pub fn take_public_wish(conn: &Connection) -> Result<bool> {
    let wanted = get_setting(conn, WANTED_KEY)?.and_then(|v| v.as_bool()).unwrap_or(false);
    if wanted {
        crate::settings::remove_setting(conn, WANTED_KEY)?;
    }
    Ok(wanted)
}

pub fn agent_rows(conn: &Connection, agent: &str) -> Result<Vec<ModelRow>> {
    Ok(model_rows(&load(conn)?).into_iter().filter(|row| row.agent == agent).collect())
}

pub fn tables_ready(conn: &Connection) -> Result<()> {
    conn.execute_batch(SQL)?;
    Ok(())
}
