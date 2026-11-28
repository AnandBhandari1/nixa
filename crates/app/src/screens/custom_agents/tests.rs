use super::model::{self, Act, Agent, Form, Profile, Tone};
use super::view::{CustomAgentsView, Mode};
use super::{Call, api};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

type Log = Arc<Mutex<Vec<(String, Option<Value>)>>>;

fn list_json() -> Value {
    json!({"ok": true, "text": "", "data": {
        "agents": [
            {"id": "a1", "title": "Scout", "purpose": "find news", "instructions": "Name the source.", "provider": "claude", "session": "s-1",
             "space": "s1", "space_name": "nixa", "terminal": "m4", "last_used": 1, "state": "free", "number": 2, "can_resume": true, "starts_new": false},
            {"id": "a2", "title": "Builder", "purpose": "write code", "provider": "codex", "role": "reviewer", "job": "developer",
             "start": {"agent": "codex", "model": "gpt-5", "effort": "high"}, "space": "", "state": "closed", "can_resume": false, "starts_new": true, "why": "no session id was saved"},
            {"id": "a3", "title": "Old", "purpose": "gone helper", "provider": "", "state": "closed", "can_resume": false, "starts_new": false, "why": "no provider was saved", "off": true}
        ],
        "spaces": [{"id": "s1", "name": "nixa"}],
        "providers": ["claude", "codex"],
        "profile_agents": ["claude", "codex", "omp"],
        "workflows": [{"name": "build-review", "title": "Build then review"}],
        "teams": [{"id": "t1", "name": "Feature", "purpose": "ship", "flow": "pr", "steps": [{"agent": "a2"}, {"agent": "a1"}]}]
    }})
}

fn fake(log: Log) -> Call {
    Arc::new(move |action, body| {
        log.lock().unwrap().push((action.to_string(), body.clone()));
        match action {
            "list" => Ok(list_json()),
            "use" => Ok(
                json!({"ok": true, "text": "Builder started.", "data": {"ok": true, "how": "started", "terminal": "m9", "show": true}}),
            ),
            "add"
                if body
                    .as_ref()
                    .and_then(|b| b.get("title"))
                    .and_then(Value::as_str)
                    == Some("Taken") =>
            {
                Err("A custom agent with this title is saved already.".into())
            }
            other => Ok(json!({"ok": true, "text": format!("Done {other}."), "data": {}})),
        }
    })
}

fn open(cx: &mut TestAppContext) -> (Entity<CustomAgentsView>, &mut VisualTestContext, Log) {
    cx.update(nixa_ui::init);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let call = fake(log.clone());
    let (view, vcx) =
        cx.add_window_view(move |_, cx| CustomAgentsView::new(Mode::Window, call, cx));
    vcx.run_until_parked();
    (view, vcx, log)
}

fn click(vcx: &mut VisualTestContext, selector: &str) {
    vcx.run_until_parked();
    let name: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let bounds = vcx
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn drawn(vcx: &mut VisualTestContext, selector: &str) -> bool {
    vcx.run_until_parked();
    let name: &'static str = Box::leak(selector.to_string().into_boxed_str());
    vcx.debug_bounds(name).is_some()
}

fn last(log: &Log) -> (String, Value) {
    let (action, body) = log
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(a, _)| a != "list")
        .cloned()
        .expect("a request");
    (action, body.unwrap_or(Value::Null))
}

fn agent(state: &str) -> Agent {
    Agent {
        id: "a1".into(),
        title: "Scout".into(),
        purpose: "p".into(),
        state: state.into(),
        terminal: "m4".into(),
        ..Agent::default()
    }
}

#[test]
fn the_list_is_read_and_cleaned() {
    let list = model::parse_list(&list_json());
    assert_eq!(list.agents.len(), 3);
    assert_eq!(
        list.agents[1].start,
        Some(Profile {
            agent: "codex".into(),
            model: "gpt-5".into(),
            effort: "high".into()
        })
    );
    assert_eq!(list.workflows[0].name, "build-review");
    assert_eq!(
        model::team_line(&list.teams[0], &list.agents),
        "1. Builder  →  2. Scout"
    );
    let bare =
        model::parse_list(&json!({"agents": [{"title": "no id"}], "spaces": [{"name": "x"}]}));
    assert!(bare.agents.is_empty() && bare.spaces.is_empty());
    assert_eq!(
        model::providers(&bare).len(),
        model::FALLBACK_PROVIDERS.len()
    );
    assert_eq!(
        model::profile_agents(&bare).len(),
        nixa_store::agents::PROFILE_AGENTS.len()
    );
    assert_eq!(model::reply_text(&json!({"text": "hi"})), "hi");
}

#[test]
fn texts_match_the_card() {
    assert_eq!(model::state_text("waiting"), "Waiting for an answer");
    assert_eq!(model::state_text(""), "Closed");
    assert_eq!(model::ago(10 * 60_000, 0), "never");
    assert_eq!(model::ago(60_000, 50_000), "just now");
    assert_eq!(model::ago(30 * 60_000, 1), "30 min ago");
    assert_eq!(model::ago(5 * 3_600_000, 1), "5 h ago");
    assert_eq!(model::ago(72 * 3_600_000, 1), "3 days ago");
    let mut a = agent("free");
    a.number = Some(7);
    assert_eq!(model::where_text(&a), "In the sidebar as #7");
    a.state = "closed".into();
    assert_eq!(model::where_text(&a), "Not in the sidebar now");
    assert_eq!(model::place_line(&a), "no provider · no space");
    a.start = Some(Profile {
        agent: "omp".into(),
        ..Profile::default()
    });
    assert_eq!(
        model::place_line(&a),
        "no provider · starts in the task's space"
    );
    assert_eq!(
        model::job_line(&a).as_deref(),
        Some("No job · starts as omp · default model · default effort")
    );
    a.why = "no session id was saved".into();
    assert_eq!(
        model::warning(&a).as_deref(),
        Some("Cannot resume: no session id was saved")
    );
    a.starts_new = true;
    assert_eq!(model::warning(&a), None);
}

#[test]
fn buttons_follow_state_profile_and_role() {
    let free = agent("free");
    assert_eq!(
        model::acts(&free, false),
        [
            Act::Open,
            Act::Edit,
            Act::MakeReviewer,
            Act::Workflow,
            Act::Forget
        ]
    );
    let mut closed = agent("closed");
    closed.can_resume = true;
    assert_eq!(
        model::acts(&closed, true),
        [
            Act::Resume,
            Act::Edit,
            Act::MakeReviewer,
            Act::Workflow,
            Act::ForgetYes
        ]
    );
    let mut profiled = agent("working");
    profiled.start = Some(Profile {
        agent: "codex".into(),
        ..Profile::default()
    });
    profiled.role = "reviewer".into();
    assert_eq!(
        model::acts(&profiled, false),
        [
            Act::Open,
            Act::Start,
            Act::Edit,
            Act::TurnOff,
            Act::StopReviewing,
            Act::Workflow,
            Act::Forget
        ]
    );
    assert_eq!(model::act_label(Act::Start, &profiled, false), "New tab");
    profiled.off = true;
    assert!(!model::acts(&profiled, false).contains(&Act::Start));
    assert!(model::acts(&profiled, false).contains(&Act::TurnOn));
    assert_eq!(
        model::act_label(Act::Start, &agent("closed"), true),
        "Starting…"
    );
    let mut fresh = agent("closed");
    fresh.starts_new = true;
    assert_eq!(model::act_label(Act::Resume, &fresh, false), "Start");
    assert_eq!(model::act_tone(Act::ForgetYes), Tone::Danger);
    assert_eq!(model::act_tone(Act::Resume), Tone::Good);
    let (action, body) = model::act_request(Act::Start, &agent("free"), Some("s1")).unwrap();
    assert_eq!(
        (action, body),
        (
            "use",
            json!({"id": "a1", "show": true, "space": "s1", "copy": true})
        )
    );
    assert_eq!(
        model::act_request(Act::Start, &agent("closed"), None)
            .unwrap()
            .1,
        json!({"id": "a1", "show": true})
    );
    assert_eq!(
        model::act_request(Act::TurnOff, &free, None).unwrap().1,
        json!({"id": "a1", "off": "off"})
    );
    assert_eq!(
        model::act_request(Act::StopReviewing, &free, None)
            .unwrap()
            .1,
        json!({"id": "a1", "role": "none"})
    );
    assert_eq!(
        model::act_request(Act::ForgetYes, &free, None).unwrap().0,
        "forget"
    );
    assert_eq!(model::act_request(Act::Edit, &free, None), None);
    assert_eq!(
        model::workflow_request(&free, None).1,
        json!({"id": "a1", "workflow": "none"})
    );
    assert!(
        model::act_tip(Act::MakeReviewer, &free)
            .starts_with("Make this the review agent of its space.")
    );
}

#[test]
fn forms_become_request_bodies() {
    let mut a = agent("closed");
    let form = Form {
        title: " Scout ".into(),
        purpose: "p".into(),
        instructions: "".into(),
        session: "".into(),
        model: "m".into(),
        effort: "".into(),
    };
    assert_eq!(
        model::edit_body(&a, &form, "codex"),
        json!({"id": "a1", "title": "Scout", "purpose": "p", "instructions": ""})
    );
    a.start = Some(Profile {
        agent: "omp".into(),
        ..Profile::default()
    });
    assert_eq!(
        model::edit_body(&a, &form, "codex"),
        json!({"id": "a1", "title": "Scout", "purpose": "p", "instructions": "", "agent": "codex", "model": "m", "effort": "default"})
    );
    assert_eq!(
        model::add_body(&Form::default(), "", ""),
        Err("A custom agent needs a title and a purpose.".into())
    );
    let body = model::add_body(&form, "codex", "s1").unwrap();
    assert_eq!(body["provider"], "codex");
    assert_eq!(body["space"], "s1");
    assert_eq!(model::form_of(&a).title, "Scout");
    assert_eq!(
        model::used_terminal(&json!({"data": {"show": true, "terminal": "m9"}})).as_deref(),
        Some("m9")
    );
    assert_eq!(
        model::used_terminal(&json!({"data": {"show": false, "terminal": "m9"}})),
        None
    );
    assert_eq!(
        api::route("use"),
        ("POST", "/custom-agents/use".to_string(), api::USE_WAIT)
    );
    assert_eq!(api::route("list").0, "GET");
}

#[gpui::test]
fn cards_show_state_warning_and_teams(cx: &mut TestAppContext) {
    let (view, vcx, log) = open(cx);
    assert!(drawn(vcx, "agent-a1"));
    assert!(drawn(vcx, "agent-a2"));
    assert!(drawn(vcx, "warn-a3"));
    assert!(!drawn(vcx, "warn-a2"));
    assert!(drawn(vcx, "open-a1"));
    assert!(drawn(vcx, "start-a2"));
    assert!(drawn(vcx, "review-off-a2"));
    assert!(drawn(vcx, "off-a2"));
    vcx.read(|cx| {
        let v = view.read(cx);
        assert!(v.loaded);
        assert_eq!(v.list.teams.len(), 1);
    });
    assert_eq!(log.lock().unwrap()[0].0, "list");
}

#[gpui::test]
fn edit_saves_title_and_profile(cx: &mut TestAppContext) {
    let (view, vcx, log) = open(cx);
    click(vcx, "edit-a2");
    vcx.read(|cx| assert_eq!(view.read(cx).editing.as_deref(), Some("a2")));
    assert!(drawn(vcx, "edit-agent-omp"));
    vcx.update(|_, cx| {
        let title = view.read(cx).inputs.title.clone();
        title.update(cx, |i, cx| i.set_value("Builder two", cx));
    });
    click(vcx, "edit-agent-omp");
    click(vcx, "edit-save");
    let (action, body) = last(&log);
    assert_eq!(action, "edit");
    assert_eq!(body["title"], "Builder two");
    assert_eq!(body["agent"], "omp");
    assert_eq!(body["model"], "gpt-5");
    assert_eq!(body["effort"], "high");
    vcx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(v.editing, None);
        assert_eq!(v.note, "Done edit.");
    });
}

#[gpui::test]
fn on_off_reviewer_workflow_and_forget(cx: &mut TestAppContext) {
    let (view, vcx, log) = open(cx);
    click(vcx, "off-a2");
    assert_eq!(
        last(&log),
        ("edit".into(), json!({"id": "a2", "off": "off"}))
    );
    click(vcx, "review-on-a1");
    assert_eq!(
        last(&log),
        ("edit".into(), json!({"id": "a1", "role": "reviewer"}))
    );
    click(vcx, "workflow-a1");
    vcx.read(|cx| assert_eq!(view.read(cx).picking.as_deref(), Some("a1")));
    click(vcx, "wf-a1-build-review");
    assert_eq!(
        last(&log),
        (
            "edit".into(),
            json!({"id": "a1", "workflow": "build-review"})
        )
    );
    click(vcx, "forget-a3");
    vcx.read(|cx| assert_eq!(view.read(cx).forgetting.as_deref(), Some("a3")));
    assert_eq!(last(&log).0, "edit");
    click(vcx, "forget-yes-a3");
    assert_eq!(last(&log), ("forget".into(), json!({"id": "a3"})));
    vcx.read(|cx| assert_eq!(view.read(cx).forgetting, None));
}

#[gpui::test]
fn add_by_hand_checks_title_and_sends_choices(cx: &mut TestAppContext) {
    let (view, vcx, log) = open(cx);
    click(vcx, "add");
    vcx.read(|cx| assert!(view.read(cx).adding));
    click(vcx, "add-save");
    vcx.read(|cx| {
        assert_eq!(
            view.read(cx).error,
            "A custom agent needs a title and a purpose."
        )
    });
    vcx.update(|_, cx| {
        let v = view.read(cx);
        let (title, purpose) = (v.inputs.title.clone(), v.inputs.purpose.clone());
        title.update(cx, |i, cx| i.set_value("Taken", cx));
        purpose.update(cx, |i, cx| i.set_value("find news", cx));
    });
    click(vcx, "add-provider-codex");
    click(vcx, "add-space-s1");
    click(vcx, "add-save");
    let (action, body) = last(&log);
    assert_eq!(action, "add");
    assert_eq!(body["provider"], "codex");
    assert_eq!(body["space"], "s1");
    vcx.read(|cx| {
        assert_eq!(
            view.read(cx).error,
            "A custom agent with this title is saved already."
        )
    });
    assert!(drawn(vcx, "agents-error"));
}

#[gpui::test]
fn start_and_open_ask_for_the_terminal(cx: &mut TestAppContext) {
    let (view, vcx, log) = open(cx);
    click(vcx, "start-a2");
    assert_eq!(
        last(&log),
        ("use".into(), json!({"id": "a2", "show": true}))
    );
    vcx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(v.note, "Builder started.");
        assert_eq!(v.opened, ["m9"]);
        assert_eq!(v.busy, None);
    });
    assert!(drawn(vcx, "agents-note"));
    click(vcx, "open-a1");
    vcx.read(|cx| assert_eq!(view.read(cx).opened, ["m9", "m4"]));
}

#[gpui::test]
fn brain_events_reload_and_show(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let call = fake(log.clone());
    let view = cx.update(|cx| super::new(Mode::Settings, call, cx));
    cx.run_until_parked();
    let lists = || {
        log.lock()
            .unwrap()
            .iter()
            .filter(|(a, _)| a == "list")
            .count()
    };
    assert_eq!(lists(), 1);
    let shown = cx.update(|cx| super::apply(&json!({"type": "custom_agents_changed"}), cx));
    cx.run_until_parked();
    assert_eq!(shown, None);
    assert_eq!(lists(), 2);
    let shown =
        cx.update(|cx| super::apply(&json!({"type": "custom_agents_show", "terminal": "m3"}), cx));
    assert_eq!(shown.as_deref(), Some("m3"));
    assert_eq!(
        cx.update(|cx| super::apply(&json!({"type": "chat"}), cx)),
        None
    );
    cx.update(|_| drop(view));
    cx.run_until_parked();
    assert_eq!(cx.update(super::refresh), 0);
}
