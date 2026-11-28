use super::{
    Form, JobPick, PickState, Picks, Repo, Schedules, Snapshot, Values, build, done, feed, payload,
    settle_job, take,
};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use nixa_store::schedules::Repeat;
use serde_json::{Value, json};

fn snapshot() -> Value {
    json!({
        "enabled": true,
        "zone": "UTC",
        "agents": [{"id": "codex", "name": "Codex"}, {"id": "claude", "name": "Claude Code"}],
        "members": [{"id": "member:a1", "name": "Tester"}],
        "teams": [{"id": "team:pr", "name": "PR team", "flow": "pr"}],
        "spaces": [{"id": "s1", "name": "nixa", "folder": "/tmp/nixa"}],
        "schedules": [{
            "id": "s1", "name": "Nightly review", "agent": "codex", "folder": "/tmp/nixa", "prompt": "Check the build",
            "job": null, "repeat": "daily", "time": "09:30", "date": "", "days": [], "enabled": true,
            "follow_agent": "", "follow_prompt": "", "next_run": 1795000000000_i64, "last_run": {"at": 1794900000000_i64, "result": "started", "terminal": "m4", "report": "/tmp/nixa/docs/automations/nightly-review/a.md", "done_at": 1794900060000_i64, "open_tasks": 2},
            "skip_unchanged": false, "skipped": null, "created_at": 1, "updated_at": 1,
            "view": {"when": "Daily at 09:30", "next": "Mon 2026-11-23 09:30", "last": "Last: done, 2 open tasks.", "failed": false, "agent_name": "Codex", "follow_name": "", "report_done": true, "busy": false}
        }]
    })
}

fn open(cx: &mut TestAppContext) -> (Entity<Schedules>, &mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let (view, cx) = cx.add_window_view(|_, cx| build(None, cx));
    view.update(cx, |part, _| {
        take(part, serde_json::from_value(snapshot()).unwrap())
    });
    cx.run_until_parked();
    (view, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, app| window.draw(app).clear());
}

fn shown(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    redraw(cx);
    cx.debug_bounds(selector).is_some()
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    redraw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn form(agent: &str) -> Form {
    Form {
        id: None,
        agent: Some(agent.into()),
        follow_agent: None,
        repeat: Repeat::Daily,
        days: Vec::new(),
        enabled: true,
        skip_unchanged: false,
        error: None,
        job: JobPick::default(),
    }
}

fn values(prompt: &str) -> Values {
    Values {
        name: "N".into(),
        folder: "/tmp".into(),
        prompt: prompt.into(),
        time: "09:30".into(),
        date: "2026-12-01".into(),
        follow_prompt: "Be brief".into(),
    }
}

#[test]
fn the_snapshot_reads_the_brain_shape() {
    let snap: Snapshot = serde_json::from_value(snapshot()).unwrap();
    assert_eq!(snap.schedules.len(), 1);
    assert_eq!(snap.schedules[0].schedule.time, "09:30");
    assert_eq!(snap.schedules[0].view.agent_name, "Codex");
    assert_eq!(snap.teams[0].flow, "pr");
}

#[test]
fn payloads_are_checked_like_today() {
    assert_eq!(
        payload(&form(""), &values("x"), false, &[]).unwrap_err(),
        "Name, agent, folder, instructions and time are needed."
    );
    assert_eq!(
        payload(&form("codex"), &values(""), false, &[]).unwrap_err(),
        "Name, agent, folder, instructions and time are needed."
    );
    let ok = payload(&form("codex"), &values("#12 fix"), false, &[]).unwrap();
    assert_eq!(ok["prompt"], "12 fix");
    assert_eq!(
        payload(&form("codex"), &values("#12"), false, &[]).unwrap()["prompt"],
        "12"
    );
    assert!(
        ok.get("date").is_none() && ok.get("days").is_none() && ok.get("follow_agent").is_none()
    );
    let mut weekly = form("codex");
    weekly.repeat = Repeat::Weekly;
    weekly.days = vec![5, 1];
    weekly.follow_agent = Some("claude".into());
    weekly.skip_unchanged = true;
    let w = payload(&weekly, &values("x"), false, &[]).unwrap();
    assert_eq!(w["days"], json!([1, 5]));
    assert_eq!(w["follow_agent"], "claude");
    assert_eq!(w["follow_prompt"], "Be brief");
    assert_eq!(w["skip_unchanged"], true);
    let mut once = form("codex");
    once.repeat = Repeat::Once;
    assert_eq!(
        payload(&once, &values("x"), false, &[]).unwrap()["date"],
        "2026-12-01"
    );
    let mut pr = form("team:pr");
    pr.follow_agent = Some("claude".into());
    let two = vec![
        Repo {
            folder: "/a".into(),
            ..Repo::default()
        },
        Repo {
            folder: "/b".into(),
            ..Repo::default()
        },
    ];
    assert_eq!(
        payload(&pr, &values(""), true, &two).unwrap_err(),
        "Pick the repo."
    );
    pr.job.repo = "/a".into();
    assert_eq!(
        payload(&pr, &values("fix all"), true, &two).unwrap_err(),
        "For the PR team the instructions are one PR number, or empty for every open PR."
    );
    let p = payload(&pr, &values("42"), true, &two).unwrap();
    assert_eq!(p["job"], json!({"repo": "/a"}));
    assert!(p.get("follow_agent").is_none());
}

#[test]
fn a_single_repo_is_picked_by_itself() {
    let mut f = form("team:pr");
    let repo = Repo {
        name: "nixa".into(),
        folder: "/r".into(),
        branches: vec!["main".into(), "dev".into()],
        current: "dev".into(),
    };
    let picks = Picks {
        folder: "/r".into(),
        state: PickState::Ready,
        repos: vec![repo],
    };
    settle_job(&mut f, &picks, "/r");
    assert_eq!((f.job.repo.as_str(), f.job.branch.as_str()), ("/r", "dev"));
    f.job.branch = "main".into();
    settle_job(&mut f, &picks, "/r");
    assert_eq!(f.job.branch, "main");
    settle_job(
        &mut f,
        &Picks {
            repos: Vec::new(),
            ..picks
        },
        "/r",
    );
    assert!(f.job.repo.is_empty() && f.job.branch.is_empty());
}

#[gpui::test]
fn rows_show_and_new_opens_the_form(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    assert!(shown(cx, "schedule-run-s1"));
    assert!(shown(cx, "schedule-work-s1"));
    assert!(shown(cx, "schedule-skip-s1"));
    click(cx, "schedule-new");
    assert!(view.read_with(cx, |p, _| p.form.is_some()));
    assert!(shown(cx, "schedule-agent-codex"));
    assert!(shown(cx, "schedule-agent-member-a1"));
    assert!(shown(cx, "schedule-agent-team-pr"));
    assert!(shown(cx, "schedule-follow-none"));
    click(cx, "schedule-save");
    assert_eq!(
        view.read_with(cx, |p, _| p.form.as_ref().and_then(|f| f.error.clone()))
            .as_deref(),
        Some("Name, agent, folder, instructions and time are needed.")
    );
    click(cx, "schedule-repeat-weekly");
    assert!(shown(cx, "schedule-day-3"));
    click(cx, "schedule-day-3");
    assert_eq!(
        view.read_with(cx, |p, _| p.form.as_ref().map(|f| f.days.clone())),
        Some(vec![3])
    );
    click(cx, "schedule-agent-team-pr");
    assert_eq!(
        view.read_with(cx, |p, _| p.form.as_ref().and_then(|f| f.agent.clone()))
            .as_deref(),
        Some("team:pr")
    );
    assert!(shown(cx, "schedule-picks-load"));
    click(cx, "schedule-cancel");
    assert!(view.read_with(cx, |p, _| p.form.is_none()));
}

#[gpui::test]
fn confirm_then_send_without_a_brain_says_so(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "schedule-delete-s1");
    let confirm = view.read_with(cx, |p, _| p.confirm.clone()).unwrap();
    assert_eq!(confirm.title, "Delete Nightly review?");
    assert!(confirm.danger);
    assert_eq!(confirm.detail, "Codex in /tmp/nixa\nCheck the build");
    click(cx, "schedule-confirm-yes");
    let notice = view.read_with(cx, |p, _| p.notice.clone()).unwrap();
    assert!(!notice.ok);
    assert_eq!(notice.text, "Nixa is not connected. Nothing was sent.");
    click(cx, "schedule-confirm-no");
    assert!(view.read_with(cx, |p, _| p.confirm.is_none()));
    click(cx, "schedule-edit-s1");
    let name = view.read_with(cx, |p, app| p.inputs.name.read(app).value().to_string());
    assert_eq!(name, "Nightly review");
    assert_eq!(
        view.read_with(cx, |p, _| p.form.as_ref().and_then(|f| f.id.clone()))
            .as_deref(),
        Some("s1")
    );
}

#[gpui::test]
fn answers_and_events_update_the_part(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.update(cx, |part, cx| {
        part.pending = Some("Pause".into());
        let mut snap = snapshot();
        snap["enabled"] = json!(false);
        snap["schedules"] = json!([]);
        done(part, Ok(snap), cx);
    });
    let (notice, snap) = view.read_with(cx, |p, _| (p.notice.clone(), p.snap.clone()));
    assert_eq!(notice.map(|n| n.text).as_deref(), Some("Pause: done."));
    assert!(!snap.unwrap().enabled);
    assert!(shown(cx, "schedules-empty"));
    view.update(cx, |part, cx| {
        part.pending = Some("Add schedule".into());
        done(
            part,
            Err("That date and time has already passed.".into()),
            cx,
        );
        feed(
            part,
            &json!({"type": "schedule_event", "text": "Nightly review started in terminal m9."}),
            cx,
        );
        let mut again = snapshot();
        again["type"] = json!("schedules");
        feed(part, &again, cx);
    });
    let (notice, events, count) = view.read_with(cx, |p, _| {
        (
            p.notice.clone().unwrap(),
            p.events.clone(),
            p.snap.as_ref().map(|s| s.schedules.len()),
        )
    });
    assert!(!notice.ok && notice.text == "That date and time has already passed.");
    assert_eq!(
        events,
        vec!["Nightly review started in terminal m9.".to_string()]
    );
    assert_eq!(count, Some(1));
}

#[test]
fn saved_skills_and_mcps_are_sent_back() {
    let mut f = form("team:t");
    f.job.repo = "/a".into();
    f.job.skills = vec!["s1".into()];
    f.job.mcps = vec!["m1".into()];
    let p = payload(&f, &values(""), false, &[]).unwrap();
    assert_eq!(p["job"]["skills"], json!(["s1"]));
    assert_eq!(p["job"]["mcps"], json!(["m1"]));
}
