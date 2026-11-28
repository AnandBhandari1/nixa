use super::cache::{AgentModels, ModelCache, guess_company, guess_plan, ordered_efforts};
use super::lists::{fixed_for, fixed_in, fixed_models, model_list, parse_models, put_models, run_list_command, run_watched, running_lists, stop_all_lists, stuck_readers};
use super::map::{import_map_md, parse_pick, rows_for_task};
use super::pick::{ModelEdit, ModelRequest, decide_model, edit_model, edit_model_as, ensure_map, launch_result, record_launch};
use super::public::{apply_scores, canonical, public_scores, refresh_due};
use super::rank::{By, capabilities_for, defaults, model_rows, move_model, move_model_as, rank_models};
use super::start::{Program, argv_model, prepare_start};
use super::usage::{leaderboard, plan_usage};
use super::*;
use crate::db::temp_db;
use anyhow::anyhow;
use serde_json::json;
use std::cell::Cell;
use std::time::Duration;

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn list(models: &[&str]) -> AgentModels {
    AgentModels { models: words(models), efforts: [("*".to_string(), words(&["default", "high"]))].into() }
}

fn cache_with(lists: &[(&str, &[&str])]) -> ModelCache {
    let mut cache = ModelCache::default();
    for (agent, models) in lists {
        put_models(&mut cache, agent, &list(models), 1);
    }
    cache
}

fn conn() -> Connection {
    let conn = temp_db("models");
    tables_ready(&conn).unwrap();
    conn
}

fn ids(cache: &ModelCache) -> Vec<String> {
    model_rows(cache).into_iter().map(|r| r.id).collect()
}

#[test]
fn parses_every_agent_list() {
    let codex = r#"note {"models":[{"slug":"gpt-5.5","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]},{"slug":"x","visibility":"hide"}]}"#;
    let parsed = parse_models("codex", codex).unwrap();
    assert_eq!(parsed.models, words(&["gpt-5.5"]));
    assert_eq!(parsed.efforts["gpt-5.5"], words(&["default", "low", "high"]));
    assert!(parse_models("codex", "nothing").is_err());
    let agy = parse_models("agy", "gemini-3-pro-high  Gemini\ngemini-3-pro-low\tx\nclaude-x\nBad Line\n").unwrap();
    assert_eq!(agy.models, words(&["gemini-3-pro", "claude-x"]));
    assert_eq!(agy.efforts["gemini-3-pro"], words(&["high", "low"]));
    assert_eq!(agy.efforts["claude-x"], words(&["default"]));
    let cursor = parse_models("cursor", "Available\ncomposer-2 - Composer\ngpt-5 - GPT\n").unwrap();
    assert_eq!(cursor.models, words(&["composer-2", "gpt-5"]));
    let open = parse_models("opencode", "zai/glm-5\nnot a model\nopencode/big-pickle\nopencode-go/kimi-k3\nollama/qwen3\nopenai/gpt-6\n").unwrap();
    assert_eq!(open.models, words(&["zai/glm-5", "opencode-go/kimi-k3", "ollama/qwen3", "openai/gpt-6"]));
    let omp = "anthropic (2)\n│ model │ name │ ctx │ thinking │\n│ claude-opus-5-5 │ Opus │ 1M │ low, high │\nzai (1)\n│ glm-5 │ GLM │ 200k │ - │\n";
    let parsed = parse_models("omp", omp).unwrap();
    assert_eq!(parsed.models, words(&["anthropic/claude-opus-5-5", "zai/glm-5"]));
    assert_eq!(parsed.efforts["anthropic/claude-opus-5-5"], words(&["default", "low", "high"]));
    assert_eq!(parsed.efforts["zai/glm-5"], words(&["default"]));
    assert!(parse_models("other", "x").unwrap().models.is_empty());
}

#[test]
fn guesses_company_plan_and_defaults() {
    assert_eq!(guess_company("claude", "claude-opus-5-5").as_deref(), Some("Anthropic"));
    assert_eq!(guess_company("codex", "o3").as_deref(), Some("OpenAI"));
    assert_eq!(guess_company("omp", "zai/glm-5").as_deref(), Some("Z.ai"));
    assert_eq!(guess_company("omp", "kimi-code/k2").as_deref(), Some("Moonshot"));
    assert_eq!(guess_company("opencode", "acme/thing"), None);
    assert_eq!(guess_company("agy", "default").as_deref(), Some("Google"));
    assert_eq!(guess_plan("codex", "gpt-5").as_deref(), Some("ChatGPT plan"));
    assert_eq!(guess_plan("opencode", "opencode/big-pickle").as_deref(), Some("Free"));
    assert_eq!(guess_plan("omp", "github-copilot/gpt-5").as_deref(), Some("Copilot plan"));
    assert_eq!(guess_plan("omp", "acme/x"), None);
    let haiku = defaults("claude", "claude-haiku-5-5", &words(&["default", "max"]));
    assert_eq!(haiku.cost, "cheap");
    for cap in ["cheap", "speed", "long-reasoning", "big-context", "vision"] {
        assert!(haiku.capabilities.contains(&cap.to_string()), "{cap}");
    }
    assert_eq!(defaults("claude", "claude-opus-5-5", &[]).cost, "premium");
    assert_eq!(defaults("opencode", "opencode-go/x", &[]).plan, "OpenCode Go plan");
    assert_eq!(defaults("omp", "acme/x", &[]).plan, "Unknown");
    assert_eq!(ordered_efforts(&words(&["max", "zeta", "default", "low", "max"])), words(&["default", "low", "max", "zeta"]));
}

#[test]
fn capabilities_come_from_task_words() {
    assert_eq!(capabilities_for("fix a bug"), words(&["coding"]));
    assert_eq!(capabilities_for("Design the frontend and test it"), words(&["testing", "ui"]));
    assert_eq!(capabilities_for("cheap job, fast"), words(&["cheap", "speed"]));
    assert_eq!(capabilities_for("read the whole repo screenshot"), words(&["big-context", "vision"]));
    assert_eq!(capabilities_for("long reasoning plan"), words(&["long-reasoning", "planning"]));
}

#[test]
fn picks_follow_the_rules() {
    let mut cache = cache_with(&[("claude", &["claude-opus-5-5", "claude-haiku-5-5"]), ("codex", &["gpt-5.5"])]);
    ensure_map(&mut cache, 1, &fixed_models()).unwrap();
    let named = decide_model(&cache, &ModelRequest { named: Some("gpt-5.5".into()), ..ModelRequest::default() }).unwrap();
    assert_eq!((named.rule.as_str(), named.chosen.unwrap().id.as_str()), ("explicit-model", "codex::gpt-5.5"));
    assert!(decide_model(&cache, &ModelRequest { named: Some("nope".into()), ..ModelRequest::default() }).is_err());
    let cheap = decide_model(&cache, &ModelRequest { capabilities: Some(words(&["cheap"])), ..ModelRequest::default() }).unwrap();
    assert_eq!(cheap.chosen.unwrap().id, "claude::claude-haiku-5-5");
    assert_eq!(cheap.rule, "capability-rank");
    let first = model_rows(&cache)[0].id.clone();
    let any = decide_model(&cache, &ModelRequest { task: Some("fix it".into()), ..ModelRequest::default() }).unwrap();
    assert_eq!(any.chosen.unwrap().id, first);
    edit_model(&mut cache, "codex::gpt-5.5", &ModelEdit { pins: Some(words(&["coding"])), ..ModelEdit::default() }, By::User).unwrap();
    let pinned = decide_model(&cache, &ModelRequest { task: Some("code".into()), ..ModelRequest::default() }).unwrap();
    assert_eq!((pinned.rule.as_str(), pinned.chosen.unwrap().id.as_str()), ("capability-pin", "codex::gpt-5.5"));
    let most = decide_model(&cache, &ModelRequest { capabilities: Some(words(&["cheap", "vision", "planning", "testing"])), agent: Some("codex".into()), ..ModelRequest::default() }).unwrap();
    assert_eq!(most.rule, "agent-most-needs");
    edit_model(&mut cache, "codex::gpt-5.5", &ModelEdit { disabled: Some(true), ..ModelEdit::default() }, By::Nixa).unwrap();
    let off = decide_model(&cache, &ModelRequest { named: Some("codex::gpt-5.5".into()), ..ModelRequest::default() }).unwrap();
    assert_eq!(off.rule, "explicit-model-unavailable");
    let over = decide_model(&cache, &ModelRequest { agent: Some("codex".into()), fallover: words(&["claude", "codex", "omp"]), ..ModelRequest::default() }).unwrap();
    assert_eq!(over.rule, "agent-fallover");
    assert_eq!(over.chain, words(&["codex", "omp", "claude"]));
    assert!(over.note.contains("codex had no free model (all codex models are taken out or no longer listed), so claude took the job"));
    let none = decide_model(&cache, &ModelRequest { agent: Some("omp".into()), fallover: words(&["codex"]), ..ModelRequest::default() }).unwrap();
    assert_eq!(none.rule, "no-free-agent");
    assert!(none.note.contains("omp has no models in the model list"));
    let alone = decide_model(&cache, &ModelRequest { agent: Some("omp".into()), ..ModelRequest::default() }).unwrap();
    assert_eq!((alone.rule.as_str(), alone.chosen.is_none()), ("agent-unavailable", true));
}

#[test]
fn secondaries_cross_plans_and_chain() {
    let mut cache = cache_with(&[("claude", &["claude-opus-5-5", "claude-sonnet-5-5"]), ("codex", &["gpt-5.5"])]);
    ensure_map(&mut cache, 1, &fixed_models()).unwrap();
    let rows = model_rows(&cache);
    for row in &rows {
        let next = row.details.secondary.clone().unwrap();
        assert_ne!(next, row.id);
        let other = rows.iter().find(|r| r.id == next).unwrap();
        assert_ne!(other.details.plan, row.details.plan, "{} -> {}", row.id, next);
    }
    let from = decide_model(&cache, &ModelRequest { from: Some("codex::gpt-5.5".into()), ..ModelRequest::default() }).unwrap();
    assert_eq!(from.rule, "secondary-chain");
    assert_ne!(from.chosen.unwrap().details.plan, "ChatGPT plan");
    edit_model(&mut cache, "codex::gpt-5.5", &ModelEdit { secondary: Some(String::new()), ..ModelEdit::default() }, By::User).unwrap();
    ensure_map(&mut cache, 2, &fixed_models()).unwrap();
    assert_eq!(model_rows(&cache).iter().find(|r| r.id == "codex::gpt-5.5").unwrap().details.secondary.as_deref(), Some(""));
    let gone = decide_model(&cache, &ModelRequest { from: Some("codex::gpt-5.5".into()), ..ModelRequest::default() }).unwrap();
    assert_eq!(gone.rule, "secondary-unavailable");
    assert!(edit_model(&mut cache, "codex::gpt-5.5", &ModelEdit { secondary: Some("codex::gpt-5.5".into()), ..ModelEdit::default() }, By::User).is_err());
    let board = leaderboard(&cache);
    assert_eq!(board.rows.len(), 5);
    assert_eq!(board.choices.len(), 5);
    assert!(board.rows.iter().any(|r| r.row.id == "claude::claude-opus-5-5" && !r.secondary_chain.is_empty()));
}

#[test]
fn rank_and_move_keep_pins() {
    let mut cache = cache_with(&[("codex", &["a", "b", "c"])]);
    ensure_map(&mut cache, 1, &fixed_models()).unwrap();
    let before = ids(&cache);
    move_model(&mut cache, &before[2], 1, By::User, 5).unwrap();
    let after = ids(&cache);
    assert_eq!(after[0], before[2]);
    let top = model_rows(&cache).into_iter().next().unwrap();
    assert!(top.details.pinned);
    assert_eq!(top.details.moved_by, "user");
    assert!(move_model(&mut cache, &before[2], 2, By::Nixa, 6).is_err());
    assert!(move_model(&mut cache, &before[0], 1, By::Nixa, 6).is_err());
    assert!(move_model(&mut cache, &before[0], 9, By::User, 6).is_err());
    edit_model(&mut cache, &before[0], &ModelEdit { cost: Some("cheap".into()), ..ModelEdit::default() }, By::Nixa).unwrap();
    rank_models(&mut cache, &[], 7).unwrap();
    assert_eq!(ids(&cache)[0], before[2]);
    assert_eq!(ids(&cache)[1], before[0]);
    assert!(edit_model(&mut cache, &before[0], &ModelEdit { pinned: Some(true), ..ModelEdit::default() }, By::Nixa).is_err());
    assert!(edit_model(&mut cache, &before[0], &ModelEdit { cost: Some("free".into()), ..ModelEdit::default() }, By::User).is_err());
    assert!(edit_model(&mut cache, &before[0], &ModelEdit { capabilities: Some(words(&["flying"])), ..ModelEdit::default() }, By::User).is_err());
    edit_model(&mut cache, &before[0], &ModelEdit { capabilities: Some(words(&["ui"])), ..ModelEdit::default() }, By::User).unwrap();
    let row = model_rows(&cache).into_iter().find(|r| r.id == before[0]).unwrap();
    assert_eq!((row.details.capabilities.clone(), row.details.capabilities_by.as_str()), (words(&["ui"]), "user"));
    assert!(edit_model(&mut cache, &before[0], &ModelEdit { capabilities: Some(words(&["coding"])), ..ModelEdit::default() }, By::Nixa).is_err());
    edit_model_as(&mut cache, &before[0], &ModelEdit { pinned: Some(true), pins: Some(words(&["coding"])), ..ModelEdit::default() }, By::Nixa, true).unwrap();
    edit_model_as(&mut cache, &before[0], &ModelEdit { capabilities: Some(words(&["coding"])), ..ModelEdit::default() }, By::Nixa, true).unwrap();
    let row = model_rows(&cache).into_iter().find(|r| r.id == before[0]).unwrap();
    assert!(row.details.pinned);
    assert!(row.details.capabilities.contains(&"coding".to_string()));
    assert_eq!(row.details.capabilities_by, "");
}

#[test]
fn automatic_nixa_moves_pinned_rows() {
    let mut cache = cache_with(&[("codex", &["a", "b", "c"])]);
    ensure_map(&mut cache, 1, &fixed_models()).unwrap();
    let before = ids(&cache);
    move_model(&mut cache, &before[2], 1, By::User, 5).unwrap();
    assert!(move_model_as(&mut cache, &before[2], 2, By::Nixa, false, 6).is_err());
    move_model_as(&mut cache, &before[2], 2, By::Nixa, true, 6).unwrap();
    assert_eq!(ids(&cache)[1], before[2]);
    move_model_as(&mut cache, &before[0], 1, By::Nixa, true, 7).unwrap();
    assert_eq!(ids(&cache)[0], before[0]);
    assert!(move_model_as(&mut cache, &before[0], 9, By::Nixa, true, 8).is_err());
}

#[test]
fn launches_count_usage_and_results() {
    let mut cache = cache_with(&[("codex", &["gpt-5.5"]), ("claude", &["claude-opus-5-5"])]);
    ensure_map(&mut cache, 1, &fixed_models()).unwrap();
    let launch = ModelLaunch { model: "codex::gpt-5.5".into(), task: "write tests".into(), ..ModelLaunch::default() };
    record_launch(&mut cache, "m1", &launch, 10).unwrap();
    record_launch(&mut cache, "m1", &ModelLaunch { folder: "/x".into(), ..launch.clone() }, 20).unwrap();
    let saved = cache.launches["m1"].clone();
    assert_eq!((saved.at, saved.folder.as_str(), saved.capabilities.clone()), (10, "/x", words(&["testing"])));
    let row = model_rows(&cache).into_iter().find(|r| r.id == "codex::gpt-5.5").unwrap();
    assert_eq!(row.details.usage.as_ref().unwrap().jobs, 1);
    assert!(launch_result(&mut cache, "m1", false, 30).unwrap());
    assert!(!launch_result(&mut cache, "m1", true, 31).unwrap());
    let row = model_rows(&cache).into_iter().find(|r| r.id == "codex::gpt-5.5").unwrap();
    assert_eq!((row.details.bad, row.details.own_score), (1, Some(0.0)));
    assert_eq!(row.details.results["testing"].bad, 1);
    let plans = plan_usage(&model_rows(&cache));
    let chatgpt = plans.iter().find(|p| p.plan == "ChatGPT plan").unwrap();
    assert_eq!((chatgpt.jobs, chatgpt.bad, chatgpt.included, chatgpt.last_at), (1, 1, true, 10));
    for n in 0..250 {
        record_launch(&mut cache, &format!("t{n}"), &ModelLaunch { model: "x::y".into(), task: "t".into(), ..ModelLaunch::default() }, 100 + n).unwrap();
    }
    assert_eq!(cache.launches.len(), 200);
    assert!(cache.launches.contains_key("t249"));
}

#[test]
fn public_scores_match_names() {
    assert_eq!(canonical("Claude 4.5 Opus"), canonical("anthropic/claude-opus-4-5"));
    assert_ne!(canonical("claude-opus-4-5"), canonical("claude-opus-4-1"));
    let raw = json!({"leaderboards": [{"name": "Lite", "results": []}, {"name": "Verified", "results": [
        {"model_display": "GPT 5.5", "resolved": 70.0},
        {"model_display": "gpt-5.5", "resolved": 72.5},
        {"model_display": "unknown", "resolved": 99.0},
        {"model_display": "Bad", "resolved": 120.0},
    ]}]});
    let scores = public_scores(&raw);
    assert_eq!(scores.len(), 1);
    let mut cache = cache_with(&[("codex", &["gpt-5.5"])]);
    ensure_map(&mut cache, 1, &fixed_models()).unwrap();
    assert!(refresh_due(&cache, 5, false));
    assert_eq!(apply_scores(&mut cache, &scores, 50).unwrap(), 1);
    assert_eq!(model_rows(&cache)[0].details.public_score, Some(72.5));
    assert!(!refresh_due(&cache, 60, false));
    assert!(refresh_due(&cache, 60, true));
    cache.public_refresh.attempt_at = 100;
    assert!(!refresh_due(&cache, 100 + 59 * 60_000, false));
    assert!(refresh_due(&cache, 100 + 61 * 60_000, false));
}

#[test]
fn fixed_lists_only_for_installed_agents() {
    let dir = std::env::temp_dir().join(format!("nixa-store-fixed-{}-{}", std::process::id(), crate::time::now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(fixed_in(std::slice::from_ref(&dir)).is_empty());
    let program = dir.join("jcode");
    std::fs::write(&program, "#!/bin/sh\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("hermes"), "not a program").unwrap();
    let found: Vec<&str> = fixed_in(std::slice::from_ref(&dir)).into_iter().map(|(agent, _)| agent).collect();
    assert_eq!(found, vec!["jcode"]);
    let mut cache = ModelCache::default();
    ensure_map(&mut cache, 1, &fixed_in(std::slice::from_ref(&dir))).unwrap();
    assert_eq!(cache.agents.keys().cloned().collect::<Vec<_>>(), vec!["jcode".to_string()]);
    let mut empty = ModelCache::default();
    ensure_map(&mut empty, 1, &[]).unwrap();
    assert!(model_rows(&empty).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn database_round_trip_and_log() {
    let conn = conn();
    let cache = load(&conn).unwrap();
    assert!(cache.agents.contains_key("claude"));
    assert_eq!(read(&conn).unwrap(), cache);
    let chosen = choose(&conn, &ModelRequest { task: Some("plan the work".into()), ..ModelRequest::default() }).unwrap().unwrap();
    assert_eq!(chosen.agent, "claude");
    assert!(!chosen.decision.is_empty());
    assert!(choose(&conn, &ModelRequest { named: Some("nope".into()), ..ModelRequest::default() }).is_err());
    let lines = decisions(&conn, 10).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["rule"], "invalid-explicit-model");
    assert_eq!(lines[1]["named"], "nope");
    let top = model_rows(&load(&conn).unwrap())[2].id.clone();
    move_to(&conn, &top, 1, By::User).unwrap();
    assert_eq!(model_rows(&read(&conn).unwrap())[0].id, top);
    record(&conn, "m5", &ModelLaunch { model: top.clone(), task: "ui work".into(), ..ModelLaunch::default() }).unwrap();
    assert!(result(&conn, "m5", true).unwrap());
    assert_eq!(launch_of(&conn, "m5").unwrap().unwrap().result, "good");
    want_public(&conn).unwrap();
    assert!(take_public_wish(&conn).unwrap());
    assert!(!take_public_wish(&conn).unwrap());
    let value = list_value(&conn).unwrap();
    assert_eq!(value["rows"].as_array().unwrap().len(), 5);
    assert!(value["rows"][0]["company"].is_string());
    assert!(value["plans"].is_array());
    let view = view_value(&conn).unwrap();
    assert_eq!(view["choices"].as_array().unwrap().len(), 5);
}

#[test]
fn tool_args_are_strict() {
    assert!(parse_args(&json!({"action": "view", "id": "x"})).is_err());
    assert!(parse_args(&json!({"action": "fly"})).is_err());
    assert!(parse_args(&json!({"other": 1})).is_err());
    assert!(parse_args(&json!({"capabilities": ["nope"]})).is_err());
    assert!(parse_args(&json!({"action": "move", "rank": 0})).is_err());
    assert_eq!(action_name(&parse_args(&Value::Null).unwrap()), "list");
    let conn = conn();
    let args = parse_args(&json!({"action": "edit", "id": "claude::claude-opus-5-5", "pinned": true})).unwrap();
    crate::settings::set_setting_text(&conn, "decision.mode", "manual").unwrap();
    assert!(act(&conn, &args, By::User).is_ok());
    assert!(act(&conn, &args, By::Nixa).is_ok());
    assert!(decisions(&conn, 10).unwrap().iter().any(|d| d["rule"] == "nixa-automatic-edit"));
    let choose = parse_args(&json!({"action": "choose", "agent": "codex"})).unwrap();
    let out = act(&conn, &choose, By::Nixa).unwrap();
    assert_eq!(out["chosen"]["agent"], "claude");
    assert_eq!(out["chosen"]["decision"], "agent-fallover");
    assert!(act(&conn, &parse_args(&json!({"action": "move"})).unwrap(), By::Nixa).is_err());
    assert!(act(&conn, &parse_args(&json!({"action": "refresh"})).unwrap(), By::Nixa).is_err());
}

#[test]
fn lists_use_saved_then_refresh() {
    let conn = conn();
    let calls = Cell::new(0);
    let ok = |_: &[&str]| -> anyhow::Result<String> {
        calls.set(calls.get() + 1);
        Ok("composer-2 - Composer\n".into())
    };
    let fail = |_: &[&str]| -> anyhow::Result<String> { Err(anyhow!("boom")) };
    let claude = model_list(&conn, "claude", false, 10, &fail).unwrap();
    assert_eq!(claude.models, fixed_for("claude").unwrap().models);
    assert!(model_list(&conn, "cursor", false, 10, &fail).is_err());
    assert!(read(&conn).unwrap().agents["cursor"].error.contains("cursor-agent models failed: boom"));
    let fresh = model_list(&conn, "cursor", false, 10, &ok).unwrap();
    assert_eq!((fresh.models.clone(), fresh.stale), (words(&["composer-2"]), false));
    let saved = model_list(&conn, "cursor", false, 20, &ok).unwrap();
    assert_eq!((saved.stale, calls.get()), (false, 1));
    let old = model_list(&conn, "cursor", false, 10 + 2 * crate::time::DAY_MS, &ok).unwrap();
    assert_eq!((old.stale, calls.get()), (true, 1));
    let kept = model_list(&conn, "cursor", true, 30, &fail).unwrap();
    assert!(kept.stale && kept.error.contains("boom"));
    assert!(model_list(&conn, "nobody", false, 10, &ok).is_err());
    assert!(model_list(&conn, "crush", false, 10, &ok).is_err());
}

#[test]
fn list_command_runs_with_time_limit() {
    let _one = one_list_test();
    let dir = std::env::temp_dir().join(format!("nixa-model-cmd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("lister");
    std::fs::write(&script, "#!/bin/sh\nif [ \"$1\" = slow ]; then sleep 5; fi\nif [ \"$1\" = bad ]; then echo nope >&2; exit 3; fi\necho one\necho two\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = script.to_string_lossy().to_string();
    assert_eq!(run_list_command(&[&path, "ok"], Duration::from_secs(5)).unwrap(), "one\ntwo\n");
    assert_eq!(run_list_command(&[&path, "bad"], Duration::from_secs(5)).unwrap_err().to_string(), "nope");
    assert!(run_list_command(&[&path, "slow"], Duration::from_millis(300)).unwrap_err().to_string().contains("timed out"));
    assert!(run_list_command(&["nixa-no-such-lister"], Duration::from_secs(1)).is_err());
}

static LISTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn one_list_test() -> std::sync::MutexGuard<'static, ()> {
    LISTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn lister_script(name: &str, body: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("nixa-model-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("lister");
    std::fs::write(&script, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = script.to_string_lossy().to_string();
    (dir, path)
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("/bin/kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

fn gone_soon(pid: &str) -> bool {
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < until {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn a_list_command_leaves_no_helper_running() {
    let _one = one_list_test();
    let body = "#!/bin/sh\nsleep 30 &\necho $! > \"$(dirname \"$0\")/helper.pid\"\nif [ \"$1\" = slow ]; then sleep 30; fi\necho one\n";
    let (dir, path) = lister_script("helper", body);
    let pid_file = dir.join("helper.pid");
    let started = std::time::Instant::now();
    assert_eq!(run_list_command(&[&path, "ok"], Duration::from_secs(5)).unwrap(), "one
");
    assert!(started.elapsed() < Duration::from_secs(5), "a helper holding the output kept the list open");
    let helper = std::fs::read_to_string(&pid_file).unwrap().trim().to_string();
    assert!(gone_soon(&helper), "the helper {helper} outlived its list command");
    std::fs::remove_file(&pid_file).unwrap();
    assert!(run_list_command(&[&path, "slow"], Duration::from_millis(400)).unwrap_err().to_string().contains("timed out"));
    let helper = std::fs::read_to_string(&pid_file).unwrap().trim().to_string();
    assert!(gone_soon(&helper), "the helper {helper} outlived the time limit");
}

const SESSION_LISTER: &str = r#"#!/bin/sh
if [ "$1" != plain ]; then
/usr/bin/perl -MPOSIX -e 'POSIX::setsid(); open(my $f, ">>", $ARGV[0]); print $f "$$\n"; close($f); sleep 30' "$(dirname "$0")/session.pids" &
sleep 0.3
fi
if [ "$1" = slow ]; then sleep 30; fi
echo one
"#;

const DOUBLE_FORK_LISTER: &str = r#"#!/bin/sh
/usr/bin/perl -MPOSIX -e '
my $file = $ARGV[0];
if (fork() == 0) {
    POSIX::setsid();
    if (fork() == 0) {
        open(my $f, ">>", $file); print $f "$$\n"; close($f);
        sleep 30; exit 0;
    }
    select(undef, undef, undef, 0.5); exit 0;
}
select(undef, undef, undef, 0.5);
' "$(dirname "$0")/session.pids" &
sleep 1
echo one
"#;

fn pids_in(file: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(file).map(|t| t.lines().map(str::to_string).collect()).unwrap_or_default()
}

fn kill_all(pids: &[String]) {
    for pid in pids {
        let _ = std::process::Command::new("/bin/kill").args(["-KILL", pid]).status();
    }
}

fn readers_end(path: &str) {
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while stuck_readers(path) > 0 && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(stuck_readers(path), 0, "the readers did not end");
}

#[test]
fn a_helper_in_its_own_session_is_found_and_stopped() {
    let _one = one_list_test();
    let (dir, path) = lister_script("session", SESSION_LISTER);
    let pid_file = dir.join("session.pids");
    let _ = std::fs::remove_file(&pid_file);
    let gate = std::sync::Arc::new(std::sync::Barrier::new(8));
    let calls: Vec<_> = (0..8)
        .map(|_| {
            let (gate, path) = (gate.clone(), path.clone());
            std::thread::spawn(move || {
                gate.wait();
                run_list_command(&[&path, "ok"], Duration::from_secs(5)).map_err(|e| e.to_string())
            })
        })
        .collect();
    let results: Vec<Result<String, String>> = calls.into_iter().map(|c| c.join().unwrap()).collect();
    let helpers = pids_in(&pid_file);
    let all_gone = helpers.iter().all(|pid| gone_soon(pid));
    kill_all(&helpers);
    assert_eq!(results.iter().filter(|r| r.as_deref() == Ok("one\n")).count(), 1, "{results:?}");
    assert_eq!(results.iter().filter(|r| r.as_ref().is_err_and(|e| e.contains("is already running"))).count(), 7, "{results:?}");
    assert_eq!(helpers.len(), 1, "more than one helper started");
    assert!(all_gone, "the session helper outlived its list");
    readers_end(&path);
    assert_eq!(run_list_command(&[&path, "plain"], Duration::from_secs(5)).unwrap(), "one\n");
    std::fs::remove_file(&pid_file).unwrap();
    assert!(run_list_command(&[&path, "slow"], Duration::from_millis(600)).unwrap_err().to_string().contains("timed out"));
    let helpers = pids_in(&pid_file);
    let all_gone = helpers.iter().all(|pid| gone_soon(pid));
    kill_all(&helpers);
    assert!(all_gone, "the session helper outlived the time limit");
}

#[test]
fn a_double_forked_helper_in_a_new_session_is_stopped() {
    let _one = one_list_test();
    let (dir, path) = lister_script("double", DOUBLE_FORK_LISTER);
    let pid_file = dir.join("session.pids");
    let _ = std::fs::remove_file(&pid_file);
    assert_eq!(run_list_command(&[&path], Duration::from_secs(10)).unwrap(), "one\n");
    let helpers = pids_in(&pid_file);
    let all_gone = helpers.iter().all(|pid| gone_soon(pid));
    kill_all(&helpers);
    assert_eq!(helpers.len(), 1, "the helper did not start");
    assert!(all_gone, "the double-forked helper outlived its list");
    readers_end(&path);
}

#[test]
fn an_unseen_helper_still_cannot_hold_the_list_open() {
    let _one = one_list_test();
    let (dir, path) = lister_script("unseen", SESSION_LISTER);
    let pid_file = dir.join("session.pids");
    let _ = std::fs::remove_file(&pid_file);
    let started = std::time::Instant::now();
    let first = run_watched(&[&path, "ok"], Duration::from_secs(5), false).unwrap_err().to_string();
    assert!(first.contains("kept its output open"), "{first}");
    assert!(started.elapsed() < Duration::from_secs(1) + super::lists::OUTPUT_GRACE + Duration::from_secs(1));
    assert_eq!(stuck_readers(&path), 2);
    let again = run_watched(&[&path, "ok"], Duration::from_secs(5), false).unwrap_err().to_string();
    assert!(again.contains("is already running"), "{again}");
    std::thread::sleep(Duration::from_millis(400));
    let helpers = pids_in(&pid_file);
    kill_all(&helpers);
    assert_eq!(helpers.len(), 1, "a refused call started a helper");
    readers_end(&path);
    assert_eq!(run_list_command(&[&path, "plain"], Duration::from_secs(5)).unwrap(), "one\n");
}

fn seen(start: u64) -> super::lists::Seen {
    super::lists::Seen { start, handle: None }
}

#[test]
fn the_process_tree_follows_children_but_not_reused_pids() {
    use super::lists::{Tree, alive_in, grow_tree};
    let mut tree = Tree::from([(10, seen(100))]);
    grow_tree(&mut tree, &[(10, 1, 100), (11, 10, 101), (12, 11, 102), (13, 1, 103)]);
    let mut kept: Vec<u32> = tree.keys().copied().collect();
    kept.sort();
    assert_eq!(kept, vec![10, 11, 12]);
    let later = [(12, 1, 102), (14, 12, 105), (11, 1, 900), (15, 11, 901)];
    grow_tree(&mut tree, &later);
    assert!(tree.contains_key(&14), "a child of a known helper is kept after it moves to pid 1");
    assert!(!tree.contains_key(&15), "a child of a reused pid is not ours");
    let alive = alive_in(&tree, &later);
    assert!(alive.contains(&12) && alive.contains(&14) && !alive.contains(&11));
    let reused_root = [(10, 1, 2000), (99, 10, 2001)];
    grow_tree(&mut tree, &reused_root);
    assert!(!tree.contains_key(&99), "a child of a reused root pid is not ours");
    assert!(!alive_in(&tree, &reused_root).contains(&10), "a reused root pid is never stopped");
}

#[test]
fn a_root_missed_at_spawn_is_never_adopted_later() {
    use super::lists::{Tree, grow_tree, look};
    let mut late = Tree::new();
    assert!(grow_tree(&mut late, &[(10, 1, 2000), (99, 10, 2001)]).is_empty());
    assert!(late.is_empty(), "an empty tree adopts nothing");
    assert!(look(&mut late).is_empty());
    assert!(late.is_empty(), "a later look on an empty tree adopts nothing");
}

#[cfg(target_os = "macos")]
#[test]
fn the_macos_process_reader_reads_exact_start_times() {
    use super::lists::native;
    assert_eq!(std::mem::size_of::<native::BsdInfo>(), 136);
    let me = std::process::id();
    let (ppid, start) = native::info(me).unwrap();
    assert_eq!(ppid, std::os::unix::process::parent_id());
    assert!(start > 1_600_000_000_000_000, "start time is in microseconds");
    assert_eq!(native::info(me).unwrap().1, start, "the start time does not change");
    let mut child = std::process::Command::new("/bin/sleep").arg("5").spawn().unwrap();
    let kids = native::children(me);
    let (child_parent, child_start) = native::info(child.id()).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(kids.contains(&child.id()));
    assert_eq!(child_parent, me);
    assert!(child_start >= start);
    assert!(native::info(child.id()).is_none_or(|(_, s)| s != child_start), "a reaped child keeps no identity");
}

#[test]
fn a_group_is_signalled_only_while_it_is_still_ours() {
    use super::lists::{Tree, group_owned, may_signal_group};
    assert!(!group_owned(false, 500, &[]), "a reaped root whose group is empty gets no group signal");
    assert!(!group_owned(false, 500, &[600]), "a member of another group does not count");
    assert!(group_owned(false, 500, &[500]));
    assert!(group_owned(true, 500, &[]), "a root not yet reaped holds its group");
    assert!(!group_owned(true, 1, &[1]));
    assert!(!may_signal_group(500, &Tree::new(), false));
    let me = std::process::id();
    if let (Some((_, start)), Some(mine)) = (super::lists::native::info(me), super::lists::native::group_of(me)) {
        let tree = Tree::from([(me, seen(start))]);
        assert!(may_signal_group(mine, &tree, false), "a checked live member holds the group");
        assert!(!may_signal_group(mine.wrapping_add(7), &tree, false));
        let stale = Tree::from([(me, seen(start + 1))]);
        assert!(!may_signal_group(mine, &stale, false), "a member with the wrong start time does not count");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn an_ended_root_is_seen_without_being_reaped() {
    use super::lists::native;
    let mut running = std::process::Command::new("/bin/sleep").arg("5").spawn().unwrap();
    assert_eq!(native::exited(running.id()), Some(false));
    running.kill().unwrap();
    running.wait().unwrap();
    let mut done = std::process::Command::new("/bin/sh").args(["-c", "exit 3"]).spawn().unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while native::exited(done.id()) != Some(true) && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(native::exited(done.id()), Some(true), "the ended child is seen");
    assert_eq!(native::exited(done.id()), Some(true), "looking does not reap it");
    assert_eq!(done.wait().unwrap().code(), Some(3), "the real exit code is kept");
}

const GRACE_LISTER: &str = r#"#!/bin/sh
/usr/bin/perl -MPOSIX -e '
my $file = $ARGV[0];
sub note { open(my $f, ">>", $file); print $f "$_[0]\n"; close($f); }
POSIX::setsid();
note($$);
$SIG{TERM} = sub {
    if (fork() == 0) { $SIG{TERM} = "DEFAULT"; POSIX::setsid(); note($$); sleep 30; exit 0; }
};
while (1) { sleep 1; }
' "$(dirname "$0")/session.pids" &
sleep 0.3
echo one
"#;

#[test]
fn a_child_born_during_the_stop_wait_is_stopped_too() {
    let _one = one_list_test();
    let (dir, path) = lister_script("grace", GRACE_LISTER);
    let pid_file = dir.join("session.pids");
    let _ = std::fs::remove_file(&pid_file);
    assert_eq!(run_list_command(&[&path], Duration::from_secs(5)).unwrap(), "one\n");
    let pids = pids_in(&pid_file);
    let all_gone = pids.iter().all(|pid| gone_soon(pid));
    kill_all(&pids);
    assert_eq!(pids.len(), 2, "the helper did not start a child on TERM: {pids:?}");
    assert!(all_gone, "a helper or its grace child outlived the list: {pids:?}");
    readers_end(&path);
}

#[test]
fn stop_all_lists_ends_a_running_list() {
    let _one = one_list_test();
    let (_dir, path) = lister_script("stop", "#!/bin/sh\nsleep 30\necho late\n");
    let worker = std::thread::spawn(move || run_list_command(&[&path], Duration::from_secs(20)));
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while running_lists() == 0 && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    let started = std::time::Instant::now();
    stop_all_lists();
    let done = worker.join().unwrap();
    assert!(done.is_err());
    assert!(started.elapsed() < Duration::from_secs(5), "the stopped list kept running");
}

#[test]
fn start_sets_model_and_falls_over() {
    let conn = conn();
    update(&conn, |cache| {
        put_models(cache, "codex", &list(&["gpt-5.5", "gpt-5.5-mini"]), 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(argv_model(&words(&["codex", "-m=gpt"])).as_deref(), Some("gpt"));
    assert_eq!(argv_model(&words(&["claude", "--model", "x"])).as_deref(), Some("x"));
    let codex = prepare_start(&conn, &words(&["codex", "--yolo"]), "write code", None, None, &[]).unwrap().unwrap();
    assert_eq!(codex.argv[..2], words(&["codex", "-m"])[..]);
    assert_eq!(codex.row.agent, "codex");
    let effort = prepare_start(&conn, &words(&["codex", "--model=gpt-5.5-high"]), "x", None, None, &[]).unwrap().unwrap();
    assert_eq!(effort.argv, words(&["codex", "--model=gpt-5.5-high"]));
    assert!(prepare_start(&conn, &words(&["ls"]), "x", None, None, &[]).unwrap().is_none());
    update(&conn, |cache| {
        for model in ["gpt-5.5", "gpt-5.5-mini"] {
            super::rank::set_details(cache, &format!("codex::{model}"), |d| d.disabled = true)?;
        }
        Ok(())
    })
    .unwrap();
    let programs = vec![
        Program { key: "codex".into(), run: words(&["codex"]), installed: true },
        Program { key: "claude".into(), run: words(&["claude", "--dangerously-skip-permissions"]), installed: true },
    ];
    let over = prepare_start(&conn, &words(&["codex", "--yolo"]), "write code", None, Some(&programs), &words(&["codex", "claude"])).unwrap().unwrap();
    assert_eq!(over.argv[..3], words(&["claude", "--model", &over.row.model])[..]);
    assert_eq!(over.request.agent.as_deref(), Some("claude"));
    assert!(over.note.contains("so claude took the job"));
    assert!(prepare_start(&conn, &words(&["codex"]), "x", None, None, &[]).is_err());
    record_start(&conn, &over, "m9", "write code", "/tmp", "s1", "Builder").unwrap();
    let launch = launch_of(&conn, "m9").unwrap().unwrap();
    assert_eq!((launch.selection.agent.as_str(), launch.space.as_str()), ("claude", "s1"));
}

#[test]
fn agent_map_text_helpers() {
    let pick = parse_pick("Code Reviewer (backup) with agy gemini-3.1-pro high; more").unwrap();
    assert_eq!((pick.member.as_str(), pick.backup, pick.agent.as_str(), pick.model.as_str(), pick.effort.as_str()), ("Code Reviewer", true, "agy", "gemini-3.1-pro", "high"));
    let short = parse_pick("Builder with codex gpt-5.5 high.").unwrap();
    assert_eq!((short.model.as_str(), short.effort.as_str()), ("gpt-5.5", ""));
    let plain = parse_pick("Nixa Master").unwrap();
    assert_eq!((plain.member.as_str(), plain.agent.as_str()), ("Nixa Master", ""));
    assert!(parse_pick("nobody").is_none());
    let md = "# Map\n## Build\n1. Write code: Builder with codex gpt-5.5 high. Backup: Fixer. Tab, worktree. Keep it small.\n2. Review code: Code Reviewer with agy gemini-3-pro\n   on every change.\n## Rules\n- Never on Claude.\n  Really.\n## Start commands\n1. ignored: x\n";
    let map = import_map_md(md);
    assert_eq!(map.rows.len(), 2);
    let first = &map.rows[0];
    assert_eq!((first.id.as_str(), first.job.as_str(), first.member.as_str(), first.fallback.as_str(), first.place.as_str(), first.notes.as_str()), ("m1", "Write code", "Builder", "Fixer", "Tab, worktree", "Keep it small"));
    assert_eq!(map.rows[1].model, "gemini-3-pro");
    assert!(map.rows[1].pick.ends_with("on every change"));
    assert_eq!(map.criteria.last().unwrap(), "Never on Claude. Really.");
    let found = rows_for_task(&map, "please review the code change", 5);
    assert_eq!(found[0].row.id, "m2");
    let conn = conn();
    let seeded = super::map::seed_from_md(&conn, md, "2026-11-22").unwrap().unwrap();
    assert_eq!(seeded.rows.len(), 2);
    assert!(super::map::seed_from_md(&conn, md, "2026-11-22").unwrap().is_none());
}

#[test]
fn a_live_model_list_is_saved_in_the_shared_cache() {
    let conn = temp_db("live-list");
    let list = AgentModels { models: vec!["gpt-a".into(), "gpt-b".into()], efforts: [("gpt-a".to_string(), vec!["low".to_string()])].into() };
    assert!(super::lists::save_live(&conn, "codex", &list, 5).unwrap());
    let saved = load(&conn).unwrap();
    assert_eq!(saved.agents["codex"].models, list.models);
    assert_eq!(saved.agents["codex"].at, 5);
    assert!(!super::lists::save_live(&conn, "claude", &list, 6).unwrap(), "a fixed list is not replaced");
    assert!(!super::lists::save_live(&conn, "nobody", &list, 6).unwrap());
    assert!(!super::lists::save_live(&conn, "codex", &AgentModels::default(), 7).unwrap(), "an empty answer keeps the saved list");
}
