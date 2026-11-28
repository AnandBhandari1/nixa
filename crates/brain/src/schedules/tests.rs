use super::jobs::{self, Step, step};
use super::runner::{self, Choice, label_of, member_plan, model_args, programs};
use super::zone::{self, Day, Rule, Zone, from_env, offset_at, parse_rule, to_utc};
use super::{report, when};
use nixa_store::schedules::{LastRun, Repeat, RunResult, Schedule};
use nixa_store::time::days_from_civil;
use serde_json::json;
use std::sync::Arc;

fn ny() -> Zone {
    zone::named("America/New_York").expect("New York zone file")
}

fn rule_zone(text: &str) -> Zone {
    let rule = parse_rule(text).expect("rule");
    Zone {
        name: text.into(),
        times: Vec::new(),
        offsets: Vec::new(),
        first: rule.std,
        rule: Some(rule),
    }
}

fn utc_ms(y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
    (days_from_civil(y, m, d) * 86_400 + h * 3600 + min * 60) * 1000
}

fn local_ms(zone: &Zone, y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
    to_utc(
        zone,
        days_from_civil(y, m, d) * 86_400 + h * 3600 + min * 60,
    )
    .expect("real local time")
        * 1000
}

fn schedule(repeat: Repeat, time: &str) -> Schedule {
    Schedule {
        id: "s1".into(),
        name: "Nightly review".into(),
        agent: "codex".into(),
        folder: "/p".into(),
        prompt: "Check the build".into(),
        job: None,
        repeat,
        time: time.into(),
        date: String::new(),
        days: Vec::new(),
        enabled: true,
        follow_agent: String::new(),
        follow_prompt: String::new(),
        next_run: None,
        last_run: None,
        skip_unchanged: false,
        skipped: None,
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn rules_parse() {
    let ny = parse_rule("EST5EDT,M3.2.0,M11.1.0").unwrap();
    assert_eq!(ny.std, -5 * 3600);
    let (dst, start, end) = ny.dst.unwrap();
    assert_eq!(dst, -4 * 3600);
    assert_eq!(
        start.day,
        Day::Month {
            month: 3,
            week: 2,
            weekday: 0
        }
    );
    assert_eq!(start.time, 7200);
    assert_eq!(
        end.day,
        Day::Month {
            month: 11,
            week: 1,
            weekday: 0
        }
    );
    assert_eq!(
        parse_rule("<+0530>-5:30"),
        Some(Rule {
            std: 5 * 3600 + 1800,
            dst: None
        })
    );
    assert_eq!(parse_rule("<+08>-8").map(|r| r.std), Some(8 * 3600));
    let au = parse_rule("AEST-10AEDT,M10.1.0,M4.1.0/3").unwrap();
    assert_eq!(au.dst.unwrap().0, 11 * 3600);
    assert_eq!(au.dst.unwrap().2.time, 3 * 3600);
    assert!(parse_rule("").is_none());
    assert!(parse_rule("EST5EDT,M13.2.0,M11.1.0").is_none());
}

#[test]
fn tzif_and_rule_agree_for_new_york() {
    let file = ny();
    let rule = rule_zone("EST5EDT,M3.2.0,M11.1.0");
    assert!(!file.times.is_empty());
    assert!(file.rule.is_some());
    for at in [
        utc_ms(2026, 1, 15, 12, 0),
        utc_ms(2026, 7, 1, 12, 0),
        utc_ms(2026, 3, 8, 6, 59),
        utc_ms(2026, 3, 8, 7, 0),
        utc_ms(2026, 11, 1, 5, 59),
        utc_ms(2026, 11, 1, 6, 0),
        utc_ms(2045, 7, 1, 0, 0),
    ] {
        assert_eq!(
            offset_at(&file, at / 1000),
            offset_at(&rule, at / 1000),
            "at {at}"
        );
    }
    assert_eq!(
        offset_at(&file, utc_ms(2026, 7, 1, 12, 0) / 1000),
        -4 * 3600
    );
    assert_eq!(
        offset_at(&file, utc_ms(2026, 12, 1, 12, 0) / 1000),
        -5 * 3600
    );
}

#[test]
fn gaps_and_repeats_in_local_time() {
    let z = ny();
    let gap = days_from_civil(2026, 3, 8) * 86_400 + 2 * 3600 + 30 * 60;
    assert_eq!(to_utc(&z, gap), None);
    let twice = days_from_civil(2026, 11, 1) * 86_400 + 3600 + 30 * 60;
    assert_eq!(
        to_utc(&z, twice).map(|s| s * 1000),
        Some(utc_ms(2026, 11, 1, 5, 30))
    );
    let south = rule_zone("AEST-10AEDT,M10.1.0,M4.1.0/3");
    assert_eq!(
        offset_at(&south, utc_ms(2026, 1, 10, 0, 0) / 1000),
        11 * 3600
    );
    assert_eq!(
        offset_at(&south, utc_ms(2026, 6, 10, 0, 0) / 1000),
        10 * 3600
    );
}

#[test]
fn daily_runs_keep_the_wall_clock_over_daylight_saving() {
    let z = ny();
    let s = schedule(Repeat::Daily, "09:30");
    let before = local_ms(&z, 2026, 3, 7, 10, 0);
    let next = when::schedule_next(&z, &s, before).unwrap();
    assert_eq!(next, utc_ms(2026, 3, 8, 13, 30));
    let after = when::schedule_next(&z, &s, next).unwrap();
    assert_eq!(after, utc_ms(2026, 3, 9, 13, 30));
    assert_eq!(when::clock(&z, next), "09:30");
    let fall = when::schedule_next(&z, &s, local_ms(&z, 2026, 10, 31, 10, 0)).unwrap();
    assert_eq!(fall, utc_ms(2026, 11, 1, 14, 30));
    assert_eq!(when::clock(&z, fall), "09:30");
}

#[test]
fn a_run_in_the_skipped_hour_is_missed_not_moved() {
    let z = ny();
    let s = schedule(Repeat::Daily, "02:30");
    let next = when::schedule_next(&z, &s, local_ms(&z, 2026, 3, 7, 3, 0)).unwrap();
    assert_eq!(when::date_time(&z, next), "Mon 2026-03-09 02:30");
    let once = Schedule {
        date: "2026-03-08".into(),
        ..schedule(Repeat::Once, "02:30")
    };
    assert_eq!(
        when::schedule_next(&z, &once, local_ms(&z, 2026, 3, 1, 0, 0)),
        None
    );
}

#[test]
fn other_repeats() {
    let z = from_env(Some("UTC"));
    let sat = utc_ms(2026, 11, 21, 10, 0);
    assert_eq!(
        when::schedule_next(&z, &schedule(Repeat::Weekdays, "09:00"), sat),
        Some(utc_ms(2026, 11, 23, 9, 0))
    );
    let weekly = Schedule {
        days: vec![3],
        ..schedule(Repeat::Weekly, "09:00")
    };
    assert_eq!(
        when::schedule_next(&z, &weekly, sat),
        Some(utc_ms(2026, 11, 25, 9, 0))
    );
    assert_eq!(
        when::schedule_next(&z, &schedule(Repeat::Hourly, "00:15"), sat),
        Some(utc_ms(2026, 11, 21, 10, 15))
    );
    let india = rule_zone("<+0530>-5:30");
    assert_eq!(
        when::schedule_next(
            &india,
            &schedule(Repeat::Hourly, "00:15"),
            utc_ms(2026, 11, 21, 10, 0)
        ),
        Some(utc_ms(2026, 11, 21, 10, 45))
    );
    let once = Schedule {
        date: "2026-12-01".into(),
        ..schedule(Repeat::Once, "08:00")
    };
    assert_eq!(
        when::schedule_next(&z, &once, sat),
        Some(utc_ms(2026, 12, 1, 8, 0))
    );
    assert_eq!(
        when::schedule_next(&z, &schedule(Repeat::Daily, "9:00"), sat),
        None
    );
    assert_eq!(when::when_text(&weekly), "Wed at 09:00");
    assert_eq!(
        when::when_text(&schedule(Repeat::Hourly, "00:15")),
        "Every hour at :15"
    );
}

#[test]
fn due_runs_late_runs_and_missed_runs() {
    let z = from_env(Some("UTC"));
    let due = utc_ms(2026, 11, 22, 9, 0);
    let s = Schedule {
        next_run: Some(due),
        ..schedule(Repeat::Daily, "09:00")
    };
    assert_eq!(step(&z, &s, due - 1), None);
    let on_time = step(&z, &s, due + 1000).unwrap();
    assert_eq!(
        on_time,
        Step {
            next: Some(due + 86_400_000),
            disable: false,
            note: None,
            run: true
        }
    );
    let late = step(&z, &s, due + 2 * 3_600_000).unwrap();
    assert!(late.run);
    assert_eq!(late.next, Some(due + 86_400_000));
    assert_eq!(
        late.note.as_deref(),
        Some("Nightly review was due at 09:00. It runs once now.")
    );
    let hourly = Schedule {
        next_run: Some(due),
        ..schedule(Repeat::Hourly, "00:00")
    };
    let many = step(&z, &hourly, due + 3 * 3_600_000 + 1_800_000).unwrap();
    assert!(many.run);
    assert_eq!(many.next, Some(due + 4 * 3_600_000));
    assert_eq!(
        many.note.as_deref(),
        Some("Nightly review was due at 09:00, and 4 runs were missed. It runs once now.")
    );
    let gone = step(&z, &hourly, due + 13 * 3_600_000).unwrap();
    assert!(!gone.run);
    assert!(
        gone.note
            .unwrap()
            .contains("missed its run at 09:00, more than 12 hours ago. Next run:")
    );
    let once = Schedule {
        next_run: Some(due),
        date: "2026-11-22".into(),
        ..schedule(Repeat::Once, "09:00")
    };
    let done = step(&z, &once, due + 5000).unwrap();
    assert!(done.disable && done.run && done.next.is_none());
    let off = Schedule {
        enabled: false,
        ..s.clone()
    };
    assert_eq!(step(&z, &off, due + 5000), None);
}

#[test]
fn report_texts() {
    assert_eq!(report::slug("Nightly Review: API!"), "nightly-review-api");
    assert_eq!(report::slug("!!!"), "schedule");
    assert_eq!(
        report::open_tasks("## Tasks\n- [ ] one\n  * [ ] two\n- [x] done\n"),
        2
    );
    let work = report::work_prompt("/p/r.md", Some("2"), "Run the tests.");
    assert!(work.starts_with("Read the report /p/r.md. Work only on task 2 from its Tasks list."));
    assert!(work.ends_with(&format!("Do not wait for the user. Run the tests.\n\n{}", crate::status_rule::rule())));
    let auto = report::work_prompt("/p/r.md", None, "");
    assert!(auto.contains("deep ask") && auto.contains("leave those steps to Nixa"));
    assert!(!auto.contains("too risky, leave it open"));
    assert!(report::report_rule("/p/r.md").contains("write your report to /p/r.md"));
    let z = from_env(Some("UTC"));
    let dir = tempfile::tempdir().unwrap();
    let s = Schedule {
        folder: dir.path().display().to_string(),
        ..schedule(Repeat::Daily, "09:00")
    };
    let at = utc_ms(2026, 11, 22, 9, 30);
    let first = report::report_path(&z, &s, at);
    assert!(first.ends_with("docs/automations/nightly-review/2026-11-22-0930.md"));
    std::fs::create_dir_all(first.parent().unwrap()).unwrap();
    std::fs::write(&first, "x").unwrap();
    assert!(report::report_path(&z, &s, at).ends_with("2026-11-22-0930-2.md"));
    let run = LastRun {
        at,
        result: RunResult::Started,
        terminal: "m4".into(),
        report: String::new(),
        done_at: None,
        open_tasks: None,
        follow_terminal: String::new(),
        team_run: String::new(),
        error: String::new(),
        marker: String::new(),
    };
    let fallback = report::fallback_report(&z, &s, &run, "last line\n");
    assert!(
        fallback.contains("Terminal: m4.")
            && fallback.contains("## Tasks\n\nNone")
            && fallback.contains("last line")
    );
    assert_eq!(report::open_tasks(&fallback), 0);
}

#[test]
fn last_run_texts() {
    let z = from_env(Some("UTC"));
    let at = utc_ms(2026, 11, 22, 9, 30);
    assert_eq!(when::last_text(&z, None, "/p"), "Last: never");
    let mut run = jobs::new_run(at, RunResult::Failed);
    run.error = "No agent.".into();
    assert_eq!(
        when::last_text(&z, Some(&run), "/p"),
        "Last: failed Sun 2026-11-22 09:30. No agent."
    );
    let mut run = jobs::new_run(at, RunResult::Started);
    run.terminal = "m3".into();
    run.report = "/p/docs/automations/x/1.md".into();
    assert_eq!(
        when::last_text(&z, Some(&run), "/p"),
        "Last: started Sun 2026-11-22 09:30 in terminal m3, still running. Report: docs/automations/x/1.md."
    );
    run.done_at = Some(at + 60_000);
    run.open_tasks = Some(2);
    run.follow_terminal = "m5".into();
    assert!(when::last_text(&z, Some(&run), "/p").ends_with(
        "2 open tasks. Report: docs/automations/x/1.md. Follow-up agent in terminal m5."
    ));
}

#[test]
fn programs_and_members() {
    let list = programs(Some(
        r#"{"programs": {"codex": {"run": ["codex", "--yolo"]}, "mine": {"name": "Mine", "run": ["mine"]}, "bad": {"name": "Bad"}}}"#,
    ));
    assert_eq!(
        list.iter().find(|p| p.0 == "codex").unwrap().2,
        vec!["codex", "--yolo"]
    );
    assert_eq!(list.iter().find(|p| p.0 == "mine").unwrap().1, "Mine");
    assert!(!list.iter().any(|p| p.0 == "bad"));
    assert_eq!(programs(Some("not json")).len(), runner::BUILT_IN.len());
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("mine");
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let command = |dir: &std::path::Path| {
        runner::agent_list(dir, &[]).into_iter().find(|entry| entry.0 == "omp").unwrap().2
    };
    assert_eq!(command(dir.path()), ["omp", "--auto-approve"]);
    std::fs::write(dir.path().join(runner::PROGRAMS_FILE), r#"{"checksOff":["omp"],"checksOn":["omp"]}"#).unwrap();
    assert_eq!(command(dir.path()), ["omp"]);
    assert!(runner::installed(
        &["mine".into()],
        &[dir.path().to_path_buf()]
    ));
    assert!(runner::installed(
        &["env".into(), "A=1".into(), "mine".into()],
        &[dir.path().to_path_buf()]
    ));
    assert!(!runner::installed(
        &["nope".into()],
        &[dir.path().to_path_buf()]
    ));
    let gone = member_plan(&nixa_protocol::paths::data_dir(), None, "a3", "task").unwrap_err();
    assert!(runner::pauses(&gone));
    assert_eq!(
        model_args("codex", "gpt-5", "high"),
        vec!["--model", "gpt-5", "-c", "model_reasoning_effort=high"]
    );
    assert!(model_args("agy", "x", "").is_empty());
    let lists: [&[Choice]; 1] = [&[Choice {
        id: "member:a1".into(),
        name: "Tester".into(),
        flow: String::new(),
    }]];
    assert_eq!(label_of("member:a1", &lists), "Tester");
    assert_eq!(label_of("member:a9", &lists), "a9 (team member not found)");
    assert_eq!(label_of("gone", &lists), "gone (not found)");
    assert!(runner::is_team("team:pr") && runner::is_member("member:a1"));
}

#[test]
fn inputs_are_checked() {
    let folder = "/tmp".to_string();
    let base = json!({"name": "N", "agent": "codex", "prompt": "Do it", "repeat": "daily", "time": "09:30", "enabled": true});
    assert!(jobs::read_input(&base, folder.clone()).is_ok());
    let err = |v: serde_json::Value| {
        jobs::read_input(&v, folder.clone())
            .unwrap_err()
            .to_string()
    };
    assert_eq!(err(json!({"agent": "codex"})), "Name is required.");
    assert_eq!(
        err(json!({"name": "N", "agent": "codex", "repeat": "daily", "time": "09:30"})),
        "Instructions is required."
    );
    assert_eq!(
        err(
            json!({"name": "N", "agent": "codex", "prompt": "p", "repeat": "often", "time": "09:30"})
        ),
        "Repeat must be one of once, hourly, daily, weekdays, weekly."
    );
    assert_eq!(
        err(
            json!({"name": "N", "agent": "codex", "prompt": "p", "repeat": "daily", "time": "9:30"})
        ),
        "Use 24-hour time like 09:30."
    );
    assert_eq!(
        err(
            json!({"name": "N", "agent": "codex", "prompt": "p", "repeat": "once", "time": "09:30", "date": "2026-02-30"})
        ),
        "Use a real date like 2026-11-09."
    );
    assert_eq!(
        err(
            json!({"name": "N", "agent": "codex", "prompt": "p", "repeat": "weekly", "time": "09:30", "days": [9]})
        ),
        "Pick at least one day for a weekly schedule."
    );
    let team = jobs::read_input(&json!({"name": "N", "agent": "team:pr", "repeat": "daily", "time": "09:30", "follow_agent": "codex", "job": {"repo": "/r", "skills": ["a", "a", " "]}}), folder.clone()).unwrap();
    assert!(team.prompt.is_empty() && team.follow_agent.is_empty());
    let job = team.job.unwrap();
    assert_eq!((job.repo.as_str(), job.skills.len()), ("/r", 1));
    let plain = jobs::read_input(&json!({"name": "N", "agent": "codex", "prompt": "p", "repeat": "daily", "time": "09:30", "follow_prompt": "x", "job": {"repo": "/r"}}), folder).unwrap();
    assert!(plain.job.is_none() && plain.follow_prompt.is_empty() && !plain.enabled);
}

#[test]
fn work_folders_refuse_home_and_private_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    assert!(jobs::folder_of(&app, "~/.ssh").is_err());
    assert!(jobs::folder_of(&app, "~").is_err());
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::create_dir_all(home.join("work")).unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let roots = crate::project_tools::policy::private_roots(Some(&home), &data);
    use crate::project_tools::policy::work_folder_in;
    assert!(work_folder_in(&home, Some(&home), &roots).is_err());
    assert!(work_folder_in(&home.join(".ssh"), Some(&home), &roots).is_err());
    assert!(work_folder_in(&data, Some(&home), &roots).is_err());
    assert!(work_folder_in(std::path::Path::new("/"), Some(&home), &roots).is_err());
    assert!(work_folder_in(&home.join("work"), Some(&home), &roots).is_ok());
}

#[tokio::test]
async fn exclude_refuses_a_gitdir_outside_the_repo() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let other = dir.path().join("other");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q", "--bare"]).arg(&other).status().unwrap().success());
    std::fs::write(repo.join(".git"), format!("gitdir: {}\n", other.display())).unwrap();
    assert!(runner::exclude(&repo.display().to_string()).await.is_err());
    assert!(!std::fs::read_to_string(other.join("info/exclude")).unwrap_or_default().contains("docs/"));
}

fn app_in(dir: &std::path::Path) -> Arc<crate::app::App> {
    crate::app::App::open(dir).expect("app")
}

#[test]
fn commands_change_the_store() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let app = app_in(data.path());
    let folder = work.path().display().to_string();
    let save = json!({"schedule": {"name": "Morning", "agent": "codex", "folder": folder, "prompt": "Check", "repeat": "daily", "time": "09:30", "enabled": true}});
    jobs::command(&app, "schedule.save", &save).unwrap();
    let list = jobs::list(&app).unwrap();
    assert_eq!(list.len(), 1);
    let s = list[0].clone();
    let zone = jobs::zone_of(&app);
    assert_eq!(s.next_run, when::schedule_next(&zone, &s, jobs::now()));
    assert_eq!(
        s.folder,
        std::fs::canonicalize(work.path())
            .unwrap()
            .display()
            .to_string()
    );
    let past = json!({"schedule": {"name": "Old", "agent": "codex", "folder": folder, "prompt": "Check", "repeat": "once", "date": "2020-01-01", "time": "09:30", "enabled": true}});
    assert_eq!(
        jobs::command(&app, "schedule.save", &past)
            .unwrap_err()
            .to_string(),
        "That date and time has already passed."
    );
    let missing = json!({"schedule": {"name": "X", "agent": "codex", "folder": "/no/such/folder", "prompt": "p", "repeat": "daily", "time": "09:30"}});
    assert!(
        jobs::command(&app, "schedule.save", &missing)
            .unwrap_err()
            .to_string()
            .contains("does not exist")
    );
    let next = s.next_run.unwrap();
    jobs::command(&app, "schedule.skip", &json!({"id": s.id})).unwrap();
    let skipped = jobs::get(&app, &s.id).unwrap();
    assert_eq!(skipped.next_run, when::schedule_next(&zone, &s, next));
    let behind = jobs::now() - 3 * 86_400_000;
    nixa_store::schedules::set_next_run(&jobs::conn(&app).unwrap(), &s.id, Some(behind)).unwrap();
    let before = jobs::now();
    jobs::command(&app, "schedule.skip", &json!({"id": s.id})).unwrap();
    let ahead = jobs::get(&app, &s.id).unwrap().next_run.unwrap();
    assert!(ahead > before);
    assert!(ahead <= when::schedule_next(&zone, &s, jobs::now()).unwrap());
    jobs::command(
        &app,
        "schedule.toggle",
        &json!({"id": s.id, "enabled": false}),
    )
    .unwrap();
    let off = jobs::get(&app, &s.id).unwrap();
    assert!(!off.enabled && off.next_run.is_none());
    assert!(jobs::command(&app, "schedule.skip", &json!({"id": s.id})).is_err());
    jobs::command(
        &app,
        "schedule.toggle",
        &json!({"id": s.id, "enabled": true}),
    )
    .unwrap();
    assert!(jobs::get(&app, &s.id).unwrap().next_run.is_some());
    let edit = json!({"schedule": {"id": s.id, "name": "Evening", "agent": "zz-none", "folder": folder, "prompt": "Check", "repeat": "weekly", "days": [1, 3], "time": "18:00", "enabled": true, "skip_unchanged": true, "follow_agent": "claude", "follow_prompt": "Be brief"}});
    jobs::command(&app, "schedule.save", &edit).unwrap();
    let edited = jobs::get(&app, &s.id).unwrap();
    assert_eq!(
        (
            edited.name.as_str(),
            edited.days.clone(),
            edited.skip_unchanged,
            edited.follow_agent.as_str()
        ),
        ("Evening", vec![1, 3], true, "claude")
    );
    assert_eq!(jobs::list(&app).unwrap().len(), 1);
    jobs::command(&app, "scheduler.set", &json!({"enabled": false})).unwrap();
    assert!(!jobs::all_on(&app));
    let snap = super::snapshot(&app);
    assert_eq!(snap["enabled"], false);
    assert_eq!(snap["schedules"][0]["view"]["when"], "Mon, Wed at 18:00");
    assert_eq!(
        snap["schedules"][0]["view"]["agent_name"],
        "zz-none (not found)"
    );
    jobs::command(&app, "scheduler.set", &json!({"enabled": true})).unwrap();
    assert!(jobs::all_on(&app));
    assert_eq!(
        jobs::command(&app, "schedule.work", &json!({"id": s.id}))
            .unwrap_err()
            .to_string(),
        "Evening has no report yet."
    );
    assert_eq!(
        jobs::command(&app, "nope", &json!({}))
            .unwrap_err()
            .to_string(),
        "Unknown action nope."
    );
    jobs::command(&app, "schedule.delete", &json!({"id": s.id})).unwrap();
    assert!(jobs::list(&app).unwrap().is_empty());
    assert!(jobs::command(&app, "schedule.delete", &json!({"id": s.id})).is_err());
}

#[test]
fn editing_keeps_saved_skills_and_mcps() {
    let old = nixa_store::schedules::ScheduleJob { repo: "/r".into(), skills: vec!["s1".into()], mcps: vec!["m1".into()], ..Default::default() };
    let sent = json!({"repo": "/r2"});
    let new = jobs::read_job(Some(&sent));
    let kept = jobs::keep_lists(new, Some(&old), Some(&sent)).unwrap();
    assert_eq!((kept.repo.as_str(), kept.skills.clone(), kept.mcps.clone()), ("/r2", vec!["s1".to_string()], vec!["m1".to_string()]));
    let cleared = json!({"repo": "/r2", "skills": [], "mcps": []});
    let kept = jobs::keep_lists(jobs::read_job(Some(&cleared)), Some(&old), Some(&cleared)).unwrap();
    assert!(kept.skills.is_empty() && kept.mcps.is_empty());
    let kept = jobs::keep_lists(None, Some(&old), None).unwrap();
    assert_eq!(kept.skills, vec!["s1".to_string()]);
}

#[test]
fn non_codex_effort_becomes_an_instruction() {
    assert_eq!(runner::with_effort("Do it.", "claude", "high"), "Do it.\n\nWork with high effort.");
    assert_eq!(runner::with_effort("Do it.", "codex", "high"), "Do it.");
    assert_eq!(runner::with_effort("Do it.", "claude", ""), "Do it.");
}

#[test]
fn fallover_skips_an_uninstalled_first_program() {
    use nixa_store::models::Program;
    let list = vec![
        Program { key: "codex".into(), run: vec!["codex".into()], installed: false },
        Program { key: "claude".into(), run: vec!["claude".into()], installed: true },
    ];
    let order = vec!["codex".to_string(), "claude".to_string()];
    let found = runner::fallover_runs(&list, &order, "codex");
    assert_eq!(found.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(), vec!["claude"]);
}

#[test]
fn fallover_refuses_when_every_model_is_unavailable() {
    use nixa_store::models::Program;
    let claude = Program { key: "claude".into(), run: vec!["claude".into()], installed: true };
    let refused = runner::fallover_choice("Codex", vec![(claude.clone(), runner::Tried::Refused("No available model for claude".into()))]);
    let text = format!("{:#}", refused.unwrap_err());
    assert!(text.contains("no fallover agent has an available model") && text.contains("claude"), "{text}");
    let (program, prepared) = runner::fallover_choice("Codex", vec![(claude.clone(), runner::Tried::Unknown)]).unwrap();
    assert_eq!(program.key, "claude");
    assert!(prepared.is_none());
    let none = runner::fallover_choice("Codex", Vec::new()).unwrap_err();
    assert_eq!(none.to_string(), "Codex is not installed.");
}

#[tokio::test]
async fn a_due_automatic_schedule_waits_for_hold_to_expire() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    let mut schedule = schedule(Repeat::Once, "09:00");
    let now = crate::util::now_ms();
    schedule.next_run = Some(now as i64);
    assert!(crate::modes::heard(&app, "hold everything", true));
    assert!(step(&ny(), &schedule, now as i64).unwrap().run);
    assert!(jobs::automatic_due_held(&app, now + 20));
    assert!(!jobs::automatic_due_held(&app, now + crate::term_tools::HOLD_END_MS + 1000));
    assert_eq!(schedule.next_run, Some(now as i64));
}

#[test]
fn a_held_schedule_tells_the_brain() {
    let text = jobs::held_event("Daily notes", "Hold is on, so no new work starts.");
    assert!(text.starts_with("Event: The schedule Daily notes did not run"));
    assert!(text.to_lowercase().contains("do not ask the user"), "{text}");
}
