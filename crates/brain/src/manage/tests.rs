use super::{diagnostics, repeats, schedules, settings, usage};
use crate::extras::usage::DayUsage;
use crate::provider::Model;
use nixa_store::models::{ModelDetails, ModelRow, PlanUsage};
use nixa_store::schedules::{Repeat, ScheduleInput};
use serde_json::json;

fn model(id: &str, efforts: &[&str]) -> Model {
    Model { id: id.into(), name: id.to_uppercase(), description: String::new(), efforts: efforts.iter().map(|e| e.to_string()).collect(), max_tokens: None }
}

fn brains() -> Vec<(String, String)> {
    vec![("saved:0a1b2c3d".into(), "Home server".into()), ("codex".into(), "Codex".into())]
}

#[tokio::test]
async fn automatic_settings_are_verified_and_journaled_without_a_yes_card() {
    let dir = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(dir.path()).unwrap();
    for (key, value) in [("git_voice", "off"), ("agent_voice", "off"), ("headphones", "on")] {
        let call = crate::turns::ToolCall {
            id: key.into(), name: settings::NAME.into(),
            args: json!({"action": "set", "key": key, "value": value}).as_object().unwrap().clone(),
        };
        let (text, error) = tokio::time::timeout(std::time::Duration::from_secs(2), crate::tools::call(&app, &call)).await.unwrap();
        assert!(!error, "{text}");
        let result: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(matches!(result["status"].as_str(), Some("verified" | "unchanged")), "{result}");
        assert_eq!(settings::raw(&settings::read(&app), key), value);
    }
    let conn = crate::modes::store(&app).unwrap();
    let rows = crate::modes::journal::last_for(&conn, None, 20).unwrap();
    let actions: Vec<_> = rows.iter().filter(|row| row.kind == "tool").collect();
    assert_eq!(actions.len(), 3);
    assert!(actions.iter().all(|row| row.outcome.as_deref() == Some("completed")));
}

#[tokio::test]
async fn user_stop_and_off_block_automatic_dispatch_and_are_journaled() {
    let dir = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(dir.path()).unwrap();
    let before = settings::read(&app).git_voice;
    let call = crate::turns::ToolCall {
        id: "change".into(), name: settings::NAME.into(),
        args: json!({"action": "set", "key": "git_voice", "value": if before { "off" } else { "on" }}).as_object().unwrap().clone(),
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    crate::app::lock(&app.brain).cancel = Some(cancel);
    let (text, error) = crate::tools::call(&app, &call).await;
    assert!(error && text.contains("stopped by the user"), "{text}");
    assert_eq!(settings::read(&app).git_voice, before);
    crate::app::lock(&app.brain).cancel = None;
    crate::settings_api::set_nixa(&app, false).unwrap();
    let (text, error) = crate::tools::call(&app, &call).await;
    assert!(error && text.contains("Nixa is off"), "{text}");
    assert_eq!(settings::read(&app).git_voice, before);
    let conn = crate::modes::store(&app).unwrap();
    let rows = crate::modes::journal::last_for(&conn, None, 20).unwrap();
    assert_eq!(rows.iter().filter(|row| row.kind == "tool" && row.outcome.as_deref() == Some("failed or stopped")).count(), 2);
}

#[test]
fn setting_values_must_be_listed_choices() {
    let models = vec![model("m-small", &["low", "high"]), model("m-big", &[])];
    assert_eq!(settings::check_value("mic", " ON ", &[], &[], None).unwrap(), "on");
    assert_eq!(settings::check_value("listen", "hotkey", &[], &[], None).unwrap(), "hotkey");
    assert!(settings::check_value("work", "Review", &[], &[], None).is_err());
    assert_eq!(settings::check_value("brain", "codex", &brains(), &[], None).unwrap(), "codex");
    assert_eq!(settings::check_value("brain", "home server", &brains(), &[], None).unwrap(), "saved:0a1b2c3d");
    assert_eq!(settings::check_value("model", "M-BIG", &[], &models, None).unwrap(), "m-big");
    assert_eq!(settings::check_value("effort", "", &[], &models, Some("m-small")).unwrap(), "");
    assert_eq!(settings::check_value("effort", "default", &[], &models, Some("m-small")).unwrap(), "");
    assert_eq!(settings::check_value("effort", "HIGH", &[], &models, Some("m-small")).unwrap(), "high");
    let bad = settings::check_value("voice", "loud", &[], &[], None).unwrap_err();
    assert!(bad.contains("on, off") && bad.contains("Nothing changed"), "{bad}");
    assert!(settings::check_value("effort", "max", &[], &models, Some("m-big")).unwrap_err().contains("empty for default"));
    assert!(settings::check_value("model", "m-big", &[], &[], None).unwrap_err().contains("could not be read"));
    assert!(settings::check_value("brain", "other", &brains(), &[], None).is_err());
    assert!(settings::check_value("colour", "red", &[], &[], None).is_err());
}

#[tokio::test]
async fn chat_refresh_models_reloads_the_selected_provider_without_changing_selection() {
    use crate::app::{App, lock};
    use crate::brains::{Checked, choice_of};
    use crate::provider::Kind;

    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let args = json!({"action": "refresh_models"});
    let call = crate::turns::ToolCall {
        id: "models".into(),
        name: settings::NAME.into(),
        args: crate::turns::args_of(&args),
    };
    let (text, error) = crate::tools::call(&app, &call).await;
    assert!(error && text.contains("No brain is picked"), "{text}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = axum::Router::new().route("/v1/models", axum::routing::get(|| async {
        axum::Json(json!({"data": [{"id": "selected"}, {"id": "new"}]}))
    }));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let brain = app.brains.add(Checked {
        name: "Local".into(), kind: Kind::Openai, base_url: format!("http://{addr}/v1"),
        key_env: None, model: "selected".into(),
    }).unwrap();
    let choice = choice_of(&brain.id);
    app.save_settings(|s| s.choice = Some(choice.clone())).unwrap();
    lock(&app.brain).models = vec![model("stale", &[])];

    let (text, error) = crate::tools::call(&app, &call).await;
    assert!(!error, "{text}");
    let result: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result["status"], "refreshed");
    assert_eq!(result["modelCount"], 2);
    let now = settings::read(&app);
    assert_eq!(now.brain.as_deref(), Some(choice.as_str()));
    assert_eq!(now.model.as_deref(), Some("selected"));
    let models = &lock(&app.brain).models;
    assert!(models.iter().any(|m| m.id == "new"));
    assert!(!models.iter().any(|m| m.id == "stale"));
    server.abort();
}

#[test]
fn quiet_minutes_have_limits() {
    assert_eq!(settings::check_minutes(Some(0.0)).unwrap(), 0.0);
    assert_eq!(settings::check_minutes(Some(30.0)).unwrap(), 30.0);
    assert!(settings::check_minutes(None).is_err());
    assert!(settings::check_minutes(Some(-1.0)).is_err());
    assert!(settings::check_minutes(Some(f64::NAN)).is_err());
    assert!(settings::check_minutes(Some(settings::QUIET_MAX_MINUTES + 1.0)).is_err());
    assert_eq!(settings::minutes_words(0.0), "off");
    assert_eq!(settings::minutes_words(1.0), "on for 1 minute");
    assert_eq!(settings::minutes_words(90.0), "on for 90 minutes");
    assert_eq!(settings::quiet_words(0, 5), "off");
    assert_eq!(settings::quiet_words(5 + 61_000, 5), "on for 2 more minutes");
}

#[test]
fn the_preview_shows_old_and_new() {
    assert_eq!(settings::new_shown("effort", "", 0.0, &[]), "default");
    assert_eq!(settings::new_shown("brain", "codex", 0.0, &brains()), "Codex (codex)");
    assert_eq!(settings::new_shown("quiet", "", 15.0, &[]), "on for 15 minutes");
}

#[test]
fn a_change_is_verified_from_the_read_back() {
    let after = settings::Now { mic: true, voice: false, listen: "wake".into(), model: Some("m1".into()), effort: None, quiet_until: 9_000, ..settings::Now::default() };
    assert!(settings::verified("mic", "on", 0.0, &after, 1_000));
    assert!(!settings::verified("voice", "on", 0.0, &after, 1_000));
    assert!(settings::verified("listen", "wake", 0.0, &after, 1_000));
    assert!(settings::verified("model", "m1", 0.0, &after, 1_000));
    assert!(settings::verified("effort", "", 0.0, &after, 1_000));
    assert!(settings::verified("quiet", "", 10.0, &after, 1_000));
    assert!(!settings::verified("quiet", "", 0.0, &after, 1_000));
    assert!(!settings::verified("brain", "x", 0.0, &after, 1_000));
    assert_eq!(settings::shown(&after, "effort", 0), "default");
    assert_eq!(settings::shown(&after, "brain", 0), "none");
}

fn input() -> ScheduleInput {
    ScheduleInput {
        name: "Nightly checks".into(),
        agent: "claude".into(),
        folder: "/work/app".into(),
        prompt: "Run the tests and write the report.".into(),
        job: None,
        repeat: Repeat::Weekly,
        time: "09:30".into(),
        date: String::new(),
        days: vec![1, 3, 9],
        enabled: false,
        follow_agent: "codex".into(),
        follow_prompt: String::new(),
        skip_unchanged: true,
    }
}

fn report_schedule(app: &crate::app::App, project: &std::path::Path, text: &str) -> (String, std::path::PathBuf) {
    let conn = nixa_store::open(&app.db_file).unwrap();
    let mut given = input();
    given.folder = project.display().to_string();
    given.agent = "missing-report-fixture-agent".into();
    given.follow_agent.clear();
    given.days = vec![1, 3];
    let schedule = nixa_store::schedules::add_schedule(&conn, &given, 0).unwrap();
    let path = crate::schedules::report::report_path(&crate::schedules::jobs::zone_of(app), &schedule, 1);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    let mut run = crate::schedules::jobs::new_run(1, nixa_store::schedules::RunResult::Started);
    run.done_at = Some(2);
    run.report = path.display().to_string();
    nixa_store::schedules::update_last_run(&conn, &schedule.id, &run).unwrap();
    (schedule.id, path)
}

#[tokio::test]
async fn schedule_report_work_selects_open_tasks() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(data.path()).unwrap();
    let (id, _) = report_schedule(&app, project.path(),
        "## Findings\n- [ ] Ignore this\n## Tasks\n- [x] Finished\n- [ ] First\n```text\n- [ ] Example\n```\n* [ ] Second\n## Output\n- [ ] Also ignored\n");
    let call = |n| json!({"action":"work","id":id,"task":n});
    let args = call(3);
    assert!(schedules::call(&app, args.as_object().unwrap()).await.unwrap_err().contains("2 open tasks"));

    let mut events = app.out.subscribe();
    for (args, selection) in [(call(2), "open task 2"), (json!({"action":"work","id":id}), "all 2 open tasks")] {
        let result = schedules::call(&app, args.as_object().unwrap()).await.unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["status"], "queued");
        assert_eq!(result["selection"], selection);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let event = events.recv().await.unwrap();
                if event["type"] == "schedule_event" {
                    let text = event["text"].as_str().unwrap();
                    assert!(text.contains("missing-report-fixture-agent"), "{text}");
                    break;
                }
            }
        }).await.unwrap();
    }
}

#[tokio::test]
async fn automatic_schedule_report_work_with_a_push_word_starts_without_a_card() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(data.path()).unwrap();
    let (id, _) = report_schedule(&app, project.path(), "## Tasks\n- [ ] git push the fix and deploy to production\n");
    let args = json!({"action":"work","id":id,"task":1});
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), schedules::call(&app, args.as_object().unwrap())).await.unwrap().unwrap();
    assert!(crate::app::lock(&app.talk.confirms).open.is_none());
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["status"], "queued");
    let conn = crate::modes::store(&app).unwrap();
    let rows = crate::modes::journal::last_for(&conn, None, 20).unwrap();
    assert!(rows.iter().any(|row| row.choice.contains("start work on the report") && row.reason.as_deref().is_some_and(|r| r.contains("production"))), "{rows:?}");
}

#[tokio::test]
async fn automatic_schedule_edits_finish_without_user_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(dir.path()).unwrap();
    let conn = nixa_store::open(&app.db_file).unwrap();
    let mut given = input();
    given.days = vec![1, 3];
    given.folder = dir.path().to_string_lossy().into_owned();
    let saved = nixa_store::schedules::add_schedule(&conn, &given, 0).unwrap();
    for action in ["resume", "pause", "remove"] {
        let args = json!({"action": action, "id": saved.id});
        let out = tokio::time::timeout(std::time::Duration::from_secs(2), schedules::call(&app, args.as_object().unwrap())).await.unwrap().unwrap();
        assert!(out.contains("done"), "{out}");
        assert!(crate::app::lock(&app.talk.confirms).open.is_none());
    }
    assert!(nixa_store::schedules::get_schedule(&conn, &saved.id).unwrap().is_none());
}

#[tokio::test]
async fn schedule_edits_follow_the_mode_of_the_schedule_space() {
    let dir = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(dir.path()).unwrap();
    let folder = dir.path().to_string_lossy().into_owned();
    let conn = nixa_store::open(&app.db_file).unwrap();
    nixa_store::spaces::set_space(&conn, "auto", &nixa_store::spaces::SpacePatch { name: Some("auto".into()), folder: Some(folder.clone()), ..Default::default() }).unwrap();
    let mut given = input();
    given.days = vec![1, 3];
    given.folder = folder;
    let saved = nixa_store::schedules::add_schedule(&conn, &given, 0).unwrap();
    let args = json!({"action": "resume", "id": saved.id});
    let out = tokio::time::timeout(std::time::Duration::from_secs(2), schedules::call(&app, args.as_object().unwrap())).await.unwrap().unwrap();
    assert!(out.contains("done"), "{out}");
    assert!(crate::app::lock(&app.talk.confirms).open.is_none());
}

#[test]
fn schedule_times_read_as_words() {
    assert_eq!(schedules::when_words(Repeat::Once, "08:00", "2026-12-01", &[]), "once on 2026-12-01 at 08:00");
    assert_eq!(schedules::when_words(Repeat::Hourly, "08:15", "", &[]), "every hour at :15");
    assert_eq!(schedules::when_words(Repeat::Daily, "08:00", "", &[]), "every day at 08:00");
    assert_eq!(schedules::when_words(Repeat::Weekdays, "08:00", "", &[]), "on weekdays at 08:00");
    assert_eq!(schedules::when_words(Repeat::Weekly, "09:30", "", &[1, 3, 9]), "every week on Mon, Wed at 09:30");
}

#[test]
fn a_schedule_preview_lists_what_is_saved() {
    let text = schedules::describe("Add schedule", &input(), "Claude", "Codex");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "Add schedule: Nightly checks?");
    assert!(lines.contains(&"Agent: Claude"));
    assert!(lines.contains(&"When: every week on Mon, Wed at 09:30"));
    assert!(lines.contains(&"Follow-up agent on the report tasks: Codex"));
    assert!(lines.contains(&"Saved as paused"));
    assert!(text.contains("Skips a timed run"));
}

#[test]
fn a_schedule_change_merges_known_fields_only() {
    let old = json!({"id": "s1", "name": "A", "time": "09:00", "enabled": true, "days": []});
    let given = json!({"time": "10:00", "enabled": false});
    let merged = schedules::merge(&old, given.as_object().unwrap()).unwrap();
    assert_eq!(merged["time"], "10:00");
    assert_eq!(merged["name"], "A");
    assert_eq!(schedules::changes(&old, &merged), vec!["time: 09:00 to 10:00".to_string(), "enabled: true to false".to_string()]);
    let bad = json!({"next_run": 5});
    assert!(schedules::merge(&old, bad.as_object().unwrap()).unwrap_err().contains("not a schedule field"));
    assert!(schedules::changes(&old, &old).is_empty());
}

fn row(agent: &str, model: &str, jobs: u64, good: u64, bad: u64, at: i64) -> ModelRow {
    let usage = (jobs > 0).then(|| nixa_store::models::cache::ModelUsage { jobs, at, ..Default::default() });
    ModelRow {
        id: format!("{agent}:{model}"),
        agent: agent.into(),
        model: model.into(),
        efforts: Vec::new(),
        rank: 1,
        listed: true,
        details: ModelDetails { good, bad, usage, plan: "Claude plan".into(), company: "Anthropic".into(), ..ModelDetails::default() },
        decision: String::new(),
        note: String::new(),
    }
}

#[test]
fn agents_are_summed_with_their_top_models() {
    let mut rows = vec![row("claude", "a", 3, 2, 1, 0), row("claude", "b", 1, 1, 0, 0), row("codex", "x", 0, 0, 0, 0)];
    for n in 0..6 {
        rows.push(row("omp", &format!("m{n}"), n + 1, 0, 0, 0));
    }
    let sums = usage::sum_agents(&rows);
    assert_eq!(sums[0].agent, "omp");
    assert_eq!(sums[0].jobs, 21);
    assert_eq!(sums[0].models.len(), usage::TOP_MODELS);
    assert_eq!(sums[0].models[0].0, "m5");
    assert_eq!(sums[1].agent, "claude");
    assert_eq!((sums[1].good, sums[1].bad), (3, 1));
    let plan = PlanUsage { plan: "Claude plan".into(), companies: vec!["Anthropic".into()], included: true, jobs: 4, good: 3, bad: 1, available: 2, models: 2, ..PlanUsage::default() };
    let text = usage::agents_text(&rows, &[plan]);
    assert!(text.contains("- claude: 4 starts, 3 good, 1 bad, last start never"), "{text}");
    assert!(text.contains("  - a: 3 starts, 2 good, 1 bad"), "{text}");
    assert!(text.contains("Agents with no counted start: codex."), "{text}");
    assert!(text.contains("- Claude plan (Anthropic): 4 starts, 3 good, 1 bad, 2 of 2 models usable, paid by a plan"), "{text}");
    assert!(usage::agents_text(&[], &[]).contains("No job start is counted yet"));
    assert_eq!(usage::starts(1), "1 start");
}

#[test]
fn brain_usage_lists_the_asked_days() {
    let day = |turns: u64, tokens: u64| DayUsage { turns, tokens, ..DayUsage::default() };
    let rows = vec![("2026-11-23".to_string(), day(2, 1_500)), ("2026-11-20".to_string(), day(1, 100)), ("2026-11-01".to_string(), day(9, 9))];
    let info = json!({"name": "Codex", "kind": "codex", "model": "m1"});
    let snap = json!({"day": "2026-11-23", "turnText": "2.0 s, 1.5k"});
    let text = usage::brain_text(Some(&info), &rows, day(2, 1_500), &snap, 7, "2026-11-17");
    assert!(text.starts_with("Chat brain now: Codex (codex), model m1."), "{text}");
    assert!(text.contains("Last turn: 2.0 s, 1.5k tokens."));
    assert!(text.contains("Today (2026-11-23): 2 turns, 1.5k tokens."));
    assert!(text.contains("Last 7 days: 3 turns, 1.6k tokens."), "{text}");
    assert!(!text.contains("2026-11-01"));
    let none = usage::brain_text(None, &[], DayUsage::default(), &json!({}), 3, "2026-11-21");
    assert!(none.contains("No chat brain is picked.") && none.contains("No saved days"));
}

#[tokio::test]
async fn usage_tool_keeps_tokens_activity_and_goals_separate() {
    use crate::app::{App, lock};
    use crate::extras::{goals::Entry, usage::Counts};
    use crate::turns::ToolCall;

    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let now = crate::util::now_ms();
    {
        let mut usage = lock(&app.extras.usage);
        usage.start(now.saturating_sub(100));
        usage.note(Counts { input: 31, output: 11, cached: 0 }, now);
        usage.end(now);
    }
    crate::extras::note_goal(&app, Entry::Held);
    let call = ToolCall { id: "usage".into(), name: usage::NAME.into(), args: json!({"days": 1}).as_object().unwrap().clone() };
    let (report, error) = crate::tools::call(&app, &call).await;
    assert!(!error, "{report}");
    let parts: Vec<_> = report.split("\n\n").collect();
    assert_eq!(parts.len(), 3, "{report}");
    assert!(parts[0].contains("1 turns, 42 tokens"), "{report}");
    assert!(parts[1].starts_with("Coding agents:") && parts[1].contains("No job start is counted yet"), "{report}");
    assert!(parts[2].contains("1 send held for the same work"), "{report}");
    let only = usage::call(&app, json!({"what": "goals"}).as_object().unwrap()).unwrap();
    assert_eq!(only, parts[2]);
    assert!(usage::call(&app, json!({"days": -1}).as_object().unwrap()).is_err());
}

#[tokio::test]
async fn settings_tool_sets_without_a_card_and_reads_back() {
    use crate::app::{App, lock};

    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    for args in [json!({"action":"get","key":17}), json!({"action":"set","key":"effort","value":false}), json!({"action":"set","key":"colour","value":"off"}), json!({"action":"set","key":"work","value":"plan"})] {
        assert!(settings::call(&app, args.as_object().unwrap()).await.is_err());
    }
    let out = settings::call(&app, json!({"action":"set","key":"git_voice","value":"off"}).as_object().unwrap()).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["status"], "verified", "{value}");
    assert!(!crate::settings_api::git_voice(&app));
    assert!(lock(&app.talk.confirms).open.is_none());
}

#[test]
fn automatic_brain_switch_keeps_waiting_while_busy() {
    assert!(settings::retry_switch(0, "The current chat brain was still busy when the switch timed out."));
    assert!(settings::retry_switch(0, "The current chat brain became busy again."));
    assert!(!settings::retry_switch(settings::BRAIN_TRIES, "The current chat brain was still busy when the switch timed out."));
    assert!(!settings::retry_switch(0, "The chat brain selection changed after approval."));
}

#[tokio::test]
async fn deferred_brain_switch_drops_busy_timeout_and_can_apply_when_idle() {
    use crate::app::App;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let approved = Some("codex".to_string());
    app.save_settings(|s| s.choice = approved.clone()).unwrap();
    crate::agent::set_busy(&app, true);
    let error = settings::switch_brain(&app, "cursor", &approved, Duration::ZERO).await.unwrap_err();
    assert!(error.contains("still busy"));
    assert_eq!(app.settings().choice, approved);
    assert!(crate::agent::busy(&app));

    crate::agent::set_busy(&app, false);
    settings::switch_brain(&app, "cursor", &approved, Duration::ZERO).await.unwrap();
    assert_eq!(app.settings().choice.as_deref(), Some("cursor"));
}

#[test]
fn diagnostics_keep_the_tail_and_hide_private_text() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("brain.log");
    std::fs::write(&log, "first line\nsecond line\nmail a.b@example.org here\n").unwrap();
    assert_eq!(diagnostics::tail(&log, 1_000), "first line\nsecond line\nmail a.b@example.org here\n");
    assert_eq!(diagnostics::tail(&log, 30), "mail a.b@example.org here\n");
    assert_eq!(diagnostics::tail(&dir.path().join("none.log"), 10), "");
    let facts = vec![("version".to_string(), "1.0".to_string()), ("contact".to_string(), "x.y@example.org".to_string())];
    let path = diagnostics::save(dir.path(), &facts, 1_795_305_600_000).unwrap();
    assert!(path.starts_with(dir.path().join(diagnostics::FOLDER)));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("Nixa diagnostics "));
    assert!(text.contains("version: 1.0") && text.contains("contact: <email>"), "{text}");
    assert!(text.contains("=== brain.log") && text.contains("mail <email> here"), "{text}");
    assert!(!text.contains("example.org"));
    assert!(!text.contains("engine.log"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}

#[tokio::test]
async fn repeats_tool_adds_lists_pauses_and_removes_through_the_brain() {
    let dir = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(dir.path()).unwrap();
    let call = |args: serde_json::Value| {
        let app = app.clone();
        async move { repeats::call(&app, args.as_object().unwrap()).await }
    };
    let out: serde_json::Value = serde_json::from_str(&call(json!({"action": "add", "text": "check idle agents", "every": 15})).await.unwrap()).unwrap();
    assert_eq!(out["status"], "done");
    let id = out["repeat"]["id"].as_str().unwrap().to_string();
    assert_eq!(out["repeat"]["every"], 15);
    assert_eq!(out["repeat"]["unit"], "minutes");
    assert_eq!(out["repeat"]["enabled"], true);
    let list: serde_json::Value = serde_json::from_str(&call(json!({"action": "list"})).await.unwrap()).unwrap();
    assert_eq!(list["repeats"][0]["id"], id.as_str());
    let out: serde_json::Value = serde_json::from_str(&call(json!({"action": "change", "id": id, "every": 2, "unit": "hours"})).await.unwrap()).unwrap();
    assert_eq!(out["repeat"]["every"], 2);
    assert_eq!(out["repeat"]["text"], "check idle agents");
    let out: serde_json::Value = serde_json::from_str(&call(json!({"action": "pause", "id": id})).await.unwrap()).unwrap();
    assert_eq!(out["repeat"]["enabled"], false);
    assert!(call(json!({"action": "add", "text": "x", "every": 0})).await.is_err());
    assert!(call(json!({"action": "pause", "id": "rep-none"})).await.is_err());
    call(json!({"action": "remove", "id": id})).await.unwrap();
    let list: serde_json::Value = serde_json::from_str(&call(json!({"action": "list"})).await.unwrap()).unwrap();
    assert_eq!(list["repeats"].as_array().unwrap().len(), 0);
}
