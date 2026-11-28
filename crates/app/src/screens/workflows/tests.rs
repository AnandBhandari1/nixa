use super::api::{self, Call};
use super::flow::{self, Spot, Who, Workflow};
use super::geo::{self, Cam};
use super::layout;
use super::view::{DIRTY, Modal, Purpose, WorkflowsView};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Store {
    flows: BTreeMap<String, Value>,
    old: BTreeMap<String, Value>,
    log: Vec<(String, Value)>,
}

type Shared = Arc<Mutex<Store>>;

fn options() -> Value {
    json!({
        "customAgents": [{"id": "a1", "title": "Scout", "purpose": "find news", "role": "", "provider": "claude"}],
        "agentKinds": ["claude", "codex"],
        "mapRows": [],
        "models": [
            {"agent": "claude", "model": "opus", "efforts": ["low", "high"]},
            {"agent": "claude", "model": "sonnet", "efforts": []},
            {"agent": "codex", "model": "gpt-5", "efforts": ["medium"]}
        ],
        "skills": [{"name": "tdd"}, {"name": "review"}],
        "tools": [{"name": "nixa_task", "description": "tasks"}]
    })
}

fn build_flow() -> Value {
    flow::to_value(&flow::template("build", "Build it", "When code changes"))
}

fn run_flow() -> Value {
    let mut wf = flow::template("build", "Build it", "When code changes");
    wf.nodes.get_mut("n2").unwrap().title = "Run copy".into();
    flow::to_value(&wf)
}

fn fake(store: Shared) -> Call {
    Arc::new(move |action, args| {
        let mut s = store.lock().unwrap();
        s.log.push((action.to_string(), args.clone()));
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match action {
            "options" => Ok(json!({"changed": false, "options": options()})),
            "list" => {
                let list: Vec<Value> = s
                    .flows
                    .iter()
                    .map(|(n, f)| json!({"name": n, "title": f["title"], "when": f["when"], "nodes": 3}))
                    .collect();
                Ok(json!({"changed": false, "workflows": list}))
            }
            "runs" => Ok(
                json!({"changed": false, "runs": [{"task": 7, "box": "n2", "space": "nixa", "state": "running", "workflow": "build"}]}),
            ),
            "run" => Ok(
                json!({"changed": false, "run": {"workflow": "build", "node": "n2", "state": "running", "started": 1, "visits": {"n2": 1}, "history": []}, "workflow": run_flow(), "reports": [{"number": 1, "box": "n2", "text": "The checks passed."}]}),
            ),
            "pause" | "resume" | "cancel" | "retry" | "answer" => {
                Ok(json!({"changed": false, "text": format!("Run {action}d.")}))
            }
            "check" => {
                let empty = args["workflow"]["nodes"]["n2"]["prompt"].as_str() == Some("");
                Ok(
                    json!({"changed": false, "problems": if empty { vec!["Box n2 has no job text."] } else { vec![] }}),
                )
            }
            "read" => match s.flows.get(&name) {
                Some(f) => Ok(
                    json!({"changed": false, "workflow": f, "canUndo": s.old.contains_key(&name)}),
                ),
                None => Err(format!("No workflow {name}.")),
            },
            "put" => {
                let wf = args["workflow"].clone();
                if let Some(prev) = s.flows.insert(name.clone(), wf.clone()) {
                    s.old.insert(name.clone(), prev);
                }
                Ok(
                    json!({"changed": true, "text": format!("Saved workflow {name}."), "workflow": wf, "canUndo": s.old.contains_key(&name)}),
                )
            }
            "delete" => {
                if let Some(saved) = s.flows.remove(&name) {
                    s.old.insert(name.clone(), saved);
                }
                Ok(
                    json!({"changed": true, "text": format!("Deleted workflow {name}. Undo puts it back.")}),
                )
            }
            "undo" => {
                let back = s.old.remove(&name).ok_or("Nothing to put back.")?;
                s.flows.insert(name.clone(), back.clone());
                Ok(
                    json!({"changed": true, "text": "Put back.", "workflow": back, "canUndo": false}),
                )
            }
            other => Err(format!("unknown {other}")),
        }
    })
}

fn open(cx: &mut TestAppContext) -> (Entity<WorkflowsView>, &mut VisualTestContext, Shared) {
    cx.update(nixa_ui::init);
    cx.update(crate::screens::settings::parts::memory::text_area::bind_keys);
    let store: Shared = Arc::new(Mutex::new(Store::default()));
    store
        .lock()
        .unwrap()
        .flows
        .insert("build".into(), build_flow());
    store.lock().unwrap().flows.insert("ship".into(), {
        let mut f = build_flow();
        f["name"] = json!("ship");
        f
    });
    let call = fake(store.clone());
    let (view, vcx) = cx.add_window_view(move |_, cx| WorkflowsView::new(call, cx));
    vcx.run_until_parked();
    vcx.update(|_, _| {});
    vcx.run_until_parked();
    (view, vcx, store)
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

fn actions(store: &Shared) -> Vec<String> {
    store
        .lock()
        .unwrap()
        .log
        .iter()
        .map(|(a, _)| a.clone())
        .collect()
}

fn last(store: &Shared, action: &str) -> Value {
    store
        .lock()
        .unwrap()
        .log
        .iter()
        .rev()
        .find(|(a, _)| a == action)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("no {action} call"))
}

fn node(view: &Entity<WorkflowsView>, vcx: &mut VisualTestContext, id: &str) -> Option<flow::Node> {
    vcx.read(|cx| {
        view.read(cx)
            .flow
            .as_ref()
            .and_then(|f| f.nodes.get(id).cloned())
    })
}

fn set_input(
    vcx: &mut VisualTestContext,
    input: Entity<nixa_ui::text_input::TextInput>,
    text: &str,
) {
    let text = text.to_string();
    vcx.update(|_, cx| input.update(cx, |i, cx| i.set_value(text, cx)));
}

#[test]
fn layout_puts_a_chain_left_to_right() {
    let out = layout::layout(3, &[(0, 1), (1, 2)], &[(280.0, 60.0); 3]);
    assert!(out[0].0 < out[1].0 && out[1].0 < out[2].0);
    assert!((out[0].1 - out[1].1).abs() < 0.01 && (out[1].1 - out[2].1).abs() < 0.01);
    assert_eq!(out[1].0 - out[0].0, 280.0 + layout::GAP);
}

#[test]
fn layout_splits_a_branch_and_survives_a_loop() {
    let out = layout::layout(
        4,
        &[(0, 1), (0, 2), (1, 3), (2, 3), (3, 1)],
        &[(100.0, 40.0); 4],
    );
    assert_eq!(out.len(), 4);
    assert!(out[1].0 > out[0].0);
    assert!((out[1].1 - out[2].1).abs() >= 40.0 + layout::SEP - 0.01);
    assert!(out.iter().all(|(x, y)| x.is_finite() && y.is_finite()));
    assert!(layout::layout(0, &[], &[]).is_empty());
    let alone = layout::layout(2, &[(0, 0)], &[(10.0, 10.0); 2]);
    assert_eq!(alone[0].0, alone[1].0);
}

#[test]
fn spaced_keeps_order_and_gaps() {
    let out = layout::spaced(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0], &[0.0, 10.0, 10.0]);
    assert!((out[1] - out[0] - 10.0).abs() < 0.001);
    assert!((out[2] - out[1] - 10.0).abs() < 0.001);
    assert!((out.iter().sum::<f32>()).abs() < 0.001);
    assert_eq!(
        layout::spaced(&[0.0, 50.0], &[1.0, 1.0], &[0.0, 10.0]),
        vec![0.0, 50.0]
    );
}

#[test]
fn template_links_and_box_ops() {
    let mut wf = flow::template("build", "", "When code changes");
    assert_eq!(wf.title, "build");
    assert_eq!(flow::ids(&wf), ["n1", "n2", "n3"]);
    assert_eq!(wf.links.len(), 2);
    assert_eq!(wf.limits, json!({"maxSteps": 30}));
    assert_eq!(wf.nodes["n2"].who.kind, "deep-brain");
    wf.layout.insert("n2".into(), Spot { x: 100.0, y: 50.0 });
    let made = flow::connect(&mut wf, "n2", "fail", flow::NEW_TARGET).unwrap();
    assert_eq!(made, "n4");
    assert_eq!(
        wf.layout["n4"],
        Spot {
            x: 100.0 + flow::GAP_X,
            y: 50.0 + flow::SIDE_STEP
        }
    );
    assert_eq!(wf.links.last().unwrap().id, "l3");
    assert_eq!(flow::connect(&mut wf, "n2", "fail", "n3"), None);
    assert_eq!(
        wf.links
            .iter()
            .filter(|l| l.from == "n2" && l.kind == "fail")
            .count(),
        1
    );
    assert_eq!(wf.links.last().unwrap().to, "n3");
    assert!(!flow::remove_box(&mut wf, "n1"));
    assert!(flow::remove_box(&mut wf, "n2"));
    assert!(wf.links.iter().all(|l| l.from != "n2" && l.to != "n2"));
    let id = flow::add_box(&mut wf, Spot { x: 5.0, y: 5.0 });
    assert_eq!(id, "n5");
    assert_eq!(wf.layout["n5"].y, 5.0 + flow::BELOW);
    let copy = flow::copy_of(&wf, "build-copy", "build");
    assert_eq!(
        (copy.name.as_str(), copy.title.as_str()),
        ("build-copy", "build copy")
    );
    let mut ids = vec!["n10", "n2", "x", "n1"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(flow::next_id(ids.iter().map(String::as_str), 'n'), "n11");
}

#[test]
fn rows_follow_the_fields() {
    let mut wf = flow::template("build", "", "w");
    let base = flow::HEAD + 8.0;
    let options = flow::parse_options(&json!({"options": options()})).unwrap();
    assert_eq!(
        flow::rows(&wf, "n1", false, &options),
        base + 2.0 * flow::ROW
    );
    assert_eq!(flow::rows(&wf, "n3", false, &options), base);
    assert_eq!(
        flow::rows(&wf, "n2", false, &options),
        base + 10.0 * flow::ROW
    );
    assert_eq!(
        flow::rows(&wf, "n2", true, &options),
        base + 10.0 * flow::ROW
    );
    let n = wf.nodes.get_mut("n2").unwrap();
    n.model = "opus".into();
    n.fallback.who = Who {
        kind: "agent".into(),
        id: "codex".into(),
    };
    n.fallback.model = "gpt-5".into();
    assert_eq!(
        flow::rows(&wf, "n2", false, &options),
        base + 13.0 * flow::ROW
    );
    wf.nodes.get_mut("n2").unwrap().model = "sonnet".into();
    assert_eq!(
        flow::rows(&wf, "n2", false, &options),
        base + 12.0 * flow::ROW
    );
}

#[test]
fn names_and_text_clip() {
    assert!(flow::valid_name("build-review-2"));
    assert!(!flow::valid_name("-build"));
    assert!(!flow::valid_name("Build"));
    assert!(!flow::valid_name(""));
    assert!(!flow::valid_name(&"a".repeat(61)));
    assert_eq!(flow::clip("abcdef", 4), "abc…");
    assert_eq!(flow::clip("abc", 4), "abc");
}

#[test]
fn json_round_trip_keeps_unknown_fields() {
    let raw = json!({"version": 1, "name": "x", "title": "X", "when": "w", "owner": "sam",
        "nodes": {"n1": {"type": "start", "title": "Start", "color": "red"}},
        "links": [{"id": "l1", "from": "n1", "to": "n1", "kind": "pass", "bend": 2}],
        "layout": {"n1": {"x": 10.6, "y": -3.2}}, "limits": {"max_steps": 9}});
    let wf: Workflow = serde_json::from_value(raw).unwrap();
    assert_eq!(wf.nodes["n1"].limit, 1);
    assert_eq!(wf.nodes["n1"].gate, "none");
    let back = flow::to_value(&wf);
    assert_eq!(back["owner"], "sam");
    assert_eq!(back["nodes"]["n1"]["color"], "red");
    assert_eq!(back["links"][0]["bend"], 2);
    assert_eq!(back["layout"]["n1"], json!({"x": 11, "y": -3}));
    assert_eq!(back["limits"], json!({"max_steps": 9}));
    assert_eq!(back["nodes"]["n1"]["type"], "start");
}

#[test]
fn replies_parse_in_both_shapes() {
    let camel = json!({"options": options()});
    let snake = json!({"ok": true, "data": {"options": {"custom_agents": [{"id": "a1"}], "agent_kinds": ["omp"], "map_rows": [{"id": "r1", "job": "builder"}]}}});
    let o = flow::parse_options(&camel).unwrap();
    assert_eq!(o.custom_agents[0].provider, "claude");
    assert_eq!(o.agent_kinds, ["claude", "codex"]);
    let s = flow::parse_options(&snake).unwrap();
    assert_eq!(s.agent_kinds, ["omp"]);
    assert_eq!(s.map_rows[0].job, "builder");
    assert!(flow::can_undo(&json!({"can_undo": true})));
    assert!(flow::can_undo(&json!({"canUndo": true})));
    assert_eq!(
        flow::text_of(&json!({"ok": true, "data": {"text": "hi"}})),
        "hi"
    );
    let runs = flow::parse_runs(
        &json!({"runs": [{"task": "#5", "box": "n2", "space": "s", "state": "paused", "workflow": "w"}]}),
    );
    assert_eq!((runs[0].task, runs[0].stage.as_str()), (5, "n2"));
    let (run, wf) = flow::parse_run(&json!({"run": {"node": "n2", "state": "waiting", "visits": {"n2": 2}, "history": [{"box": "n2", "outcome": "fail"}]}, "workflow": build_flow()})).unwrap();
    assert_eq!(wf.name, "build");
    assert_eq!(
        flow::run_line(&wf.nodes["n2"], &run, "n2"),
        "visits 2 · last fail · now waiting"
    );
    assert!(flow::parse_run(&json!({"run": null})).is_none());
    assert_eq!(
        flow::parse_names(&json!({"workflows": [{"name": ""}, {"name": "a", "broken": "bad"}]}))
            .len(),
        1
    );
}

#[test]
fn picker_lists_and_labels() {
    let o = flow::parse_options(&json!({"options": options()})).unwrap();
    let who = flow::who_options(&o, false);
    assert_eq!(who[0].value, "agent:claude");
    assert_eq!(who[2].value, "custom-agent:a1");
    assert_eq!(who.last().unwrap().value, "user:");
    let fb = flow::who_options(&o, true);
    assert_eq!(fb.last().unwrap().value, ":");
    assert_eq!(
        flow::who_from("custom-agent:a1"),
        Who {
            kind: "custom-agent".into(),
            id: "a1".into()
        }
    );
    assert_eq!(flow::who_from(":"), Who::default());
    let scout = Who {
        kind: "custom-agent".into(),
        id: "a1".into(),
    };
    assert_eq!(flow::who_label(&o, &scout), "Scout");
    assert_eq!(flow::kind_of(&o, &scout), "claude");
    assert_eq!(
        flow::who_label(
            &o,
            &Who {
                kind: "deep-brain".into(),
                id: String::new()
            }
        ),
        "Deep brain"
    );
    assert_eq!(flow::who_label(&o, &Who::default()), "choose");
    assert_eq!(flow::model_options(&o, "claude").len(), 3);
    assert_eq!(flow::effort_options(&o, "claude", "opus").len(), 3);
    assert!(flow::no_list(
        &o,
        &Who {
            kind: "agent".into(),
            id: "omp".into()
        }
    ));
    assert_eq!(flow::filter_options(&who, "scout").len(), 1);
    assert_eq!(flow::gate_label("deep-brain"), "deep brain");
    let wf = flow::template("b", "", "w");
    let targets = flow::target_options(&wf, "n2");
    assert_eq!(
        targets.iter().map(|t| t.value.as_str()).collect::<Vec<_>>(),
        ["+new", "n3"]
    );
}

#[test]
fn camera_fit_and_zoom() {
    let cam = geo::fit((1000.0, 600.0), (-100.0, -50.0, 100.0, 50.0));
    assert_eq!(cam.z, 1.0);
    assert_eq!(cam.sx(0.0), 500.0);
    assert_eq!(cam.sy(0.0), 300.0);
    let small = geo::fit((300.0, 200.0), (0.0, 0.0, 4000.0, 100.0));
    assert_eq!(small.z, geo::MIN_FIT);
    let z = geo::zoom_at(
        Cam {
            z: 1.0,
            x: 0.0,
            y: 0.0,
        },
        2.0,
        100.0,
        100.0,
    );
    assert_eq!((z.z, z.x, z.y), (2.0, -100.0, -100.0));
    assert_eq!(geo::zoom_at(z, 10.0, 0.0, 0.0).z, geo::MAX_ZOOM);
    let wf = flow::template("b", "", "w");
    let options = flow::Options::default();
    let spots = geo::auto_spots(&wf, false, &options);
    assert!(spots["n1"].x < spots["n2"].x && spots["n2"].x < spots["n3"].x);
    let curves = geo::curves(&wf, &spots, Cam::default(), false, &options);
    assert_eq!(curves.len(), 2);
    assert!(curves[0].tip.0 > curves[0].start.0);
}

#[test]
fn request_args_carry_the_action() {
    assert_eq!(
        api::args("read", json!({"name": "b"})),
        json!({"action": "read", "name": "b"})
    );
    assert_eq!(api::args("list", Value::Null), json!({"action": "list"}));
}

#[gpui::test]
fn opens_the_first_workflow_and_draws_boxes(cx: &mut TestAppContext) {
    let (view, vcx, store) = open(cx);
    let a = actions(&store);
    assert_eq!(&a[..3], ["options", "list", "runs"]);
    assert!(a.contains(&"read".to_string()));
    assert!(a.contains(&"check".to_string()));
    vcx.read(|cx| {
        let v = view.read(cx);
        assert!(v.booted);
        assert_eq!(v.name, "build");
        assert_eq!(v.names.len(), 2);
        assert_eq!(v.options.models.len(), 3);
        assert!(!v.dirty());
    });
    for id in [
        "wf-node-n1",
        "wf-node-n2",
        "wf-node-n3",
        "wf-item-ship",
        "wf-run-7",
        "wf-who-n2",
        "wf-connect-n1",
    ] {
        assert!(drawn(vcx, id), "{id}");
    }
    assert!(!drawn(vcx, "wf-connect-n3"));
}

#[gpui::test]
fn add_undo_redo_and_save(cx: &mut TestAppContext) {
    let (view, vcx, store) = open(cx);
    click(vcx, "wf-head-n2");
    vcx.read(|cx| assert_eq!(view.read(cx).selected.as_deref(), Some("n2")));
    click(vcx, "wf-add");
    assert!(node(&view, vcx, "n4").is_some());
    vcx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(v.selected.as_deref(), Some("n4"));
        assert!(v.dirty());
        assert_eq!(v.flow.as_ref().unwrap().layout.len(), 4);
    });
    click(vcx, "wf-undo");
    assert!(node(&view, vcx, "n4").is_none());
    click(vcx, "wf-redo");
    assert!(node(&view, vcx, "n4").is_some());
    click(vcx, "wf-save");
    let put = last(&store, "put");
    assert_eq!(put["name"], "build");
    assert!(put["workflow"]["nodes"]["n4"].is_object());
    vcx.read(|cx| {
        let v = view.read(cx);
        assert!(!v.dirty());
        assert!(v.can_undo_saved);
        assert_eq!(v.note, "Saved workflow build.");
    });
    click(vcx, "wf-saved-undo");
    assert_eq!(
        *actions(&store).iter().rev().find(|a| *a == "undo").unwrap(),
        "undo"
    );
}

#[gpui::test]
fn agent_model_and_effort_pickers_chain(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    click(vcx, "wf-who-n2");
    vcx.read(|cx| {
        let p = view.read(cx).picker.clone().unwrap();
        assert_eq!(
            p.purpose,
            Purpose::Who {
                id: "n2".into(),
                fb: false
            }
        );
        assert_eq!(p.options.len(), 6);
    });
    click(vcx, "wf-opt-0");
    vcx.read(|cx| {
        let p = view.read(cx).picker.clone().unwrap();
        assert_eq!(p.title, "Model of claude");
    });
    click(vcx, "wf-opt-1");
    vcx.read(|cx| assert_eq!(view.read(cx).picker.as_ref().unwrap().title, "Effort"));
    click(vcx, "wf-opt-2");
    let n = node(&view, vcx, "n2").unwrap();
    assert_eq!(
        (n.who.kind.as_str(), n.who.id.as_str()),
        ("agent", "claude")
    );
    assert_eq!(n.model, "opus");
    assert_eq!(n.effort, "high");
    vcx.read(|cx| assert!(view.read(cx).picker.is_none()));
    assert!(drawn(vcx, "wf-effort-n2"));
    click(vcx, "wf-fb-n2");
    vcx.update(|_, cx| {
        let f = view.read(cx).inputs.filter.clone();
        f.update(cx, |i, cx| i.set_value("codex", cx));
    });
    click(vcx, "wf-opt-0");
    click(vcx, "wf-picker-close");
    let n = node(&view, vcx, "n2").unwrap();
    assert_eq!(n.fallback.who.id, "codex");
    assert!(drawn(vcx, "wf-fbmodel-n2"));
    click(vcx, "wf-gate-n2");
    click(vcx, "wf-opt-1");
    assert_eq!(node(&view, vcx, "n2").unwrap().gate, "checks");
    click(vcx, "wf-retry-n2+");
    click(vcx, "wf-limit-n2+");
    click(vcx, "wf-limit-n2-");
    click(vcx, "wf-limit-n2-");
    let n = node(&view, vcx, "n2").unwrap();
    assert_eq!((n.retry, n.limit), (1, 1));
}

#[gpui::test]
fn skills_pick_many_and_job_text(cx: &mut TestAppContext) {
    let (view, vcx, store) = open(cx);
    click(vcx, "wf-skills-n2");
    click(vcx, "wf-opt-0");
    click(vcx, "wf-opt-1");
    click(vcx, "wf-opt-0");
    click(vcx, "wf-picker-done");
    assert_eq!(node(&view, vcx, "n2").unwrap().skills, ["review"]);
    click(vcx, "wf-job-n2");
    vcx.read(|cx| assert_eq!(view.read(cx).modal, Some(Modal::Prompt("n2".into()))));
    vcx.update(|_, cx| {
        let area = view.read(cx).inputs.prompt.clone();
        area.update(cx, |a, cx| a.set_value("", cx));
    });
    click(vcx, "wf-prompt-ok");
    assert_eq!(node(&view, vcx, "n2").unwrap().prompt, "");
    vcx.read(|cx| assert_eq!(view.read(cx).problems, ["Box n2 has no job text."]));
    assert!(drawn(vcx, "wf-problem-0"));
    assert_eq!(
        last(&store, "check")["workflow"]["nodes"]["n2"]["prompt"],
        ""
    );
}

#[gpui::test]
fn connect_makes_a_new_box_and_unlink_removes(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    click(vcx, "wf-connect-n2");
    click(vcx, "wf-opt-2");
    vcx.read(|cx| {
        assert_eq!(
            view.read(cx).picker.as_ref().unwrap().title,
            "Connect with fail to"
        )
    });
    click(vcx, "wf-opt-0");
    vcx.read(|cx| {
        let v = view.read(cx);
        let f = v.flow.as_ref().unwrap();
        assert!(
            f.links
                .iter()
                .any(|l| l.from == "n2" && l.to == "n4" && l.kind == "fail")
        );
        assert_eq!(v.selected.as_deref(), Some("n4"));
    });
    click(vcx, "wf-unlink-l3");
    vcx.read(|cx| assert_eq!(view.read(cx).flow.as_ref().unwrap().links.len(), 2));
}

#[gpui::test]
fn canvas_keys_move_fit_and_delete(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    click(vcx, "wf-head-n2");
    let before = vcx.read(|cx| view.read(cx).spots["n2"]);
    vcx.simulate_keystrokes("right");
    vcx.simulate_keystrokes("shift-down");
    vcx.run_until_parked();
    vcx.read(|cx| {
        let p = view.read(cx).flow.as_ref().unwrap().layout["n2"];
        assert_eq!(p.x, (before.x + 20.0).round());
        assert_eq!(p.y, (before.y + 80.0).round());
    });
    let z = vcx.read(|cx| view.read(cx).cam.unwrap().z);
    vcx.simulate_keystrokes("-");
    vcx.run_until_parked();
    vcx.read(|cx| assert!(view.read(cx).cam.unwrap().z < z));
    vcx.simulate_keystrokes("f");
    vcx.run_until_parked();
    vcx.read(|cx| assert_eq!(view.read(cx).cam.unwrap().z, z));
    vcx.simulate_keystrokes("backspace");
    vcx.run_until_parked();
    assert!(node(&view, vcx, "n2").is_none());
    vcx.simulate_keystrokes("cmd-z");
    vcx.run_until_parked();
    assert!(node(&view, vcx, "n2").is_some());
    click(vcx, "wf-head-n1");
    vcx.simulate_keystrokes("backspace");
    vcx.run_until_parked();
    assert!(node(&view, vcx, "n1").is_some());
}

#[gpui::test]
fn box_drag_moves_and_link_drag_opens_a_type_picker(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    let start = vcx.debug_bounds("wf-head-n2").unwrap().center();
    let next = start + gpui::point(gpui::px(20.0), gpui::px(30.0));
    let button = gpui::MouseButton::Left;
    let modifiers = Modifiers::none();
    vcx.simulate_mouse_down(start, button, modifiers);
    vcx.simulate_mouse_move(next, button, modifiers);
    vcx.simulate_mouse_up(next, button, modifiers);
    vcx.run_until_parked();
    vcx.read(|cx| {
        let view = view.read(cx);
        assert_eq!(view.undo.len(), 1);
        assert!(view.flow.as_ref().unwrap().layout.contains_key("n2"));
    });

    let start = vcx.debug_bounds("wf-link-drag-n1").unwrap().center();
    let end = vcx.debug_bounds("wf-head-n3").unwrap().center();
    vcx.simulate_mouse_down(start, button, modifiers);
    vcx.simulate_mouse_move(end, button, modifiers);
    vcx.simulate_mouse_up(end, button, modifiers);
    vcx.run_until_parked();
    vcx.read(|cx| {
        let view = view.read(cx);
        assert!(matches!(view.picker.as_ref().map(|p| &p.purpose),
            Some(Purpose::LinkTo { from, to }) if from == "n1" && to == "n3"));
    });
    click(vcx, "wf-opt-0");
    vcx.read(|cx| {
        assert!(
            view.read(cx)
                .flow
                .as_ref()
                .unwrap()
                .links
                .iter()
                .any(|link| link.from == "n1" && link.kind == "pass" && link.to == "n3")
        )
    });
}

#[gpui::test]
fn auto_layout_clears_saved_spots(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    click(vcx, "wf-head-n2");
    vcx.simulate_keystrokes("down");
    vcx.run_until_parked();
    click(vcx, "wf-auto");
    vcx.read(|cx| {
        let v = view.read(cx);
        assert!(v.flow.as_ref().unwrap().layout.is_empty());
        assert!(!v.dirty());
        assert_eq!(v.undo.len(), 1);
    });
    click(vcx, "wf-undo");
    vcx.read(|cx| assert_eq!(view.read(cx).flow.as_ref().unwrap().layout.len(), 3));
}

#[gpui::test]
fn new_copy_and_delete_workflows(cx: &mut TestAppContext) {
    let (view, vcx, store) = open(cx);
    click(vcx, "wf-new");
    let (name, title, when) = vcx.read(|cx| {
        let i = &view.read(cx).inputs;
        (i.name.clone(), i.title.clone(), i.when.clone())
    });
    set_input(vcx, name.clone(), "Bad Name");
    click(vcx, "wf-new-ok");
    assert!(drawn(vcx, "wf-form-error"));
    vcx.read(|cx| assert_eq!(view.read(cx).form_error, super::view::BAD_NAME));
    set_input(vcx, name, "fix-flow");
    set_input(vcx, title, "Fix flow");
    set_input(vcx, when, "When a bug is found");
    click(vcx, "wf-new-ok");
    let put = last(&store, "put");
    assert_eq!(put["workflow"]["when"], "When a bug is found");
    assert_eq!(put["workflow"]["nodes"]["n2"]["title"], "Work");
    vcx.read(|cx| assert_eq!(view.read(cx).name, "fix-flow"));
    click(vcx, "wf-copy");
    click(vcx, "wf-copy-ok");
    assert_eq!(last(&store, "put")["name"], "fix-flow-copy");
    vcx.read(|cx| assert_eq!(view.read(cx).name, "fix-flow-copy"));
    click(vcx, "wf-del");
    click(vcx, "wf-del-no");
    vcx.read(|cx| assert!(!view.read(cx).confirm_delete));
    assert!(!actions(&store).contains(&"delete".to_string()));
    click(vcx, "wf-del");
    click(vcx, "wf-del-yes");
    assert_eq!(last(&store, "delete")["name"], "fix-flow-copy");
    vcx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(v.names.len(), 3);
        assert_eq!(v.name, "build");
    });
    click(vcx, "wf-restore-deleted");
    assert_eq!(last(&store, "undo")["name"], "fix-flow-copy");
    vcx.read(|cx| assert_eq!(view.read(cx).name, "fix-flow-copy"));
}

#[gpui::test]
fn workflow_details_and_box_state_are_editable(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    click(vcx, "wf-details");
    let (title, when, steps, box_title) = vcx.read(|cx| {
        let inputs = &view.read(cx).inputs;
        (
            inputs.title.clone(),
            inputs.when.clone(),
            inputs.step_limit.clone(),
            inputs.box_title.clone(),
        )
    });
    set_input(vcx, title, "Updated title");
    set_input(vcx, when, "After a change");
    set_input(vcx, steps, "801");
    click(vcx, "wf-details-ok");
    assert!(drawn(vcx, "wf-form-error"));
    let steps = vcx.read(|cx| view.read(cx).inputs.step_limit.clone());
    set_input(vcx, steps, "80");
    click(vcx, "wf-details-ok");
    vcx.read(|cx| {
        let flow = view.read(cx).flow.as_ref().unwrap();
        assert_eq!(
            (flow.title.as_str(), flow.when.as_str(), flow.max_steps()),
            ("Updated title", "After a change", 80)
        );
    });
    click(vcx, "wf-head-n2");
    click(vcx, "wf-box-rename");
    set_input(vcx, box_title, "Build it");
    click(vcx, "wf-details-ok");
    click(vcx, "wf-box-toggle");
    let stage = node(&view, vcx, "n2").unwrap();
    assert_eq!(stage.title, "Build it");
    assert!(stage.off);
}

#[gpui::test]
fn dirty_flow_blocks_switching(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    click(vcx, "wf-head-n2");
    click(vcx, "wf-add");
    click(vcx, "wf-item-ship");
    vcx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(v.error, DIRTY);
        assert_eq!(v.name, "build");
    });
    click(vcx, "wf-revert");
    vcx.read(|cx| assert!(!view.read(cx).dirty()));
    click(vcx, "wf-item-ship");
    vcx.read(|cx| assert_eq!(view.read(cx).name, "ship"));
}

#[gpui::test]
fn run_view_is_read_only_with_run_buttons(cx: &mut TestAppContext) {
    let (view, vcx, store) = open(cx);
    click(vcx, "wf-run-7");
    vcx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(v.run_view.as_ref().unwrap().task, 7);
        assert_eq!(v.flow.as_ref().unwrap().nodes["n2"].title, "Run copy");
        assert!(v.note.starts_with("Task #7"));
        assert!(!v.editable());
    });
    assert!(drawn(vcx, "wf-run-pause"));
    assert!(!drawn(vcx, "wf-run-resume"));
    click(vcx, "wf-who-n2");
    vcx.read(|cx| assert!(view.read(cx).picker.is_none()));
    click(vcx, "wf-run-reports");
    assert!(drawn(vcx, "wf-report-list"));
    vcx.read(|cx| {
        assert_eq!(
            view.read(cx).run_view.as_ref().unwrap().reports[0].text,
            "The checks passed."
        )
    });
    click(vcx, "wf-reports-close");
    click(vcx, "wf-run-pause");
    assert_eq!(last(&store, "pause"), json!({"task": 7}));
    click(vcx, "wf-run-cancel");
    click(vcx, "wf-run-cancel-yes");
    assert_eq!(last(&store, "cancel"), json!({"task": 7}));
    click(vcx, "wf-run-close");
    vcx.read(|cx| {
        let v = view.read(cx);
        assert!(v.run_view.is_none());
        assert_eq!(v.flow.as_ref().unwrap().nodes["n2"].title, "Work");
    });
}

#[gpui::test]
fn waiting_run_offers_retry_and_answers(cx: &mut TestAppContext) {
    let (view, vcx, store) = open(cx);
    click(vcx, "wf-run-7");
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            let rv = v.run_view.as_mut().unwrap();
            rv.run.state = "waiting".into();
            rv.run.ask = Some(flow::Ask {
                stage: "n2".into(),
                cause: "no-result".into(),
                why: "the agent sent no result".into(),
                gate: false,
            });
            cx.notify();
        })
    });
    assert!(drawn(vcx, "wf-run-ask"));
    click(vcx, "wf-run-retry");
    assert_eq!(last(&store, "retry"), json!({"task": 7}));
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            let rv = v.run_view.as_mut().unwrap();
            rv.run.state = "waiting".into();
            rv.run.ask = Some(flow::Ask {
                stage: "n2".into(),
                cause: "gate".into(),
                why: String::new(),
                gate: true,
            });
            cx.notify();
        })
    });
    click(vcx, "wf-run-answer-revise");
    assert_eq!(last(&store, "answer"), json!({"task": 7, "text": "revise"}));
    let run = flow::Run {
        state: "waiting".into(),
        ask: Some(flow::Ask {
            stage: "n2".into(),
            cause: "gate".into(),
            why: String::new(),
            gate: true,
        }),
        ..flow::Run::default()
    };
    assert_eq!(
        flow::waiting_text(&run).as_deref(),
        Some("Waiting at n2: gate")
    );
}

#[gpui::test]
fn brain_events_reload(cx: &mut TestAppContext) {
    let (view, vcx, store) = open(cx);
    let reads = |s: &Shared| {
        s.lock()
            .unwrap()
            .log
            .iter()
            .filter(|(a, _)| a == "read")
            .count()
    };
    let before = reads(&store);
    store.lock().unwrap().flows.get_mut("build").unwrap()["title"] = json!("Changed");
    vcx.update(|_, cx| view.update(cx, |v, cx| v.changed(Some("build"), cx)));
    vcx.run_until_parked();
    assert_eq!(reads(&store), before + 1);
    vcx.read(|cx| assert_eq!(view.read(cx).flow.as_ref().unwrap().title, "Changed"));
    vcx.update(|_, cx| view.update(cx, |v, cx| v.changed(Some("ship"), cx)));
    vcx.run_until_parked();
    assert_eq!(reads(&store), before + 1);
    let runs = actions(&store).iter().filter(|a| *a == "runs").count();
    vcx.update(|_, cx| view.update(cx, |v, cx| v.load_runs(cx)));
    vcx.run_until_parked();
    assert_eq!(
        actions(&store).iter().filter(|a| *a == "runs").count(),
        runs + 1
    );
}

struct PartHost {
    view: Entity<WorkflowsView>,
}

impl gpui::Render for PartHost {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        crate::screens::settings::parts::workflows::render(&self.view, cx)
    }
}

#[gpui::test]
fn settings_part_offers_the_window(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let call = fake(Shared::default());
    let (_, vcx) = cx.add_window_view(|_, cx| PartHost {
        view: super::new(call, cx),
    });
    assert!(drawn(vcx, crate::screens::settings::parts::workflows::OPEN));
    assert_eq!(
        crate::screens::settings::parts::workflows::PART,
        "Workflows"
    );
}

#[gpui::test]
fn start_switch_opens_screens(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    vcx.update(|_, cx| view.update(cx, |v, cx| v.start_with("picker", cx)));
    vcx.run_until_parked();
    vcx.read(|cx| assert_eq!(view.read(cx).picker.as_ref().unwrap().title, "Agent"));
    vcx.update(|_, cx| view.update(cx, |v, cx| v.close_picker(cx)));
    vcx.update(|_, cx| view.update(cx, |v, cx| v.start_with("run:7", cx)));
    vcx.run_until_parked();
    vcx.read(|cx| assert_eq!(view.read(cx).run_view.as_ref().unwrap().task, 7));
}

fn empty<'a>(
    cx: &'a mut TestAppContext,
    start: &'static str,
) -> (Entity<WorkflowsView>, &'a mut VisualTestContext) {
    cx.update(nixa_ui::init);
    cx.update(crate::screens::settings::parts::memory::text_area::bind_keys);
    let call = fake(Arc::new(Mutex::new(Store::default())));
    let (view, vcx) = cx.add_window_view(move |_, cx| {
        let mut view = WorkflowsView::new(call, cx);
        view.start_with(start, cx);
        view
    });
    vcx.run_until_parked();
    (view, vcx)
}

#[gpui::test]
fn a_run_start_opens_with_no_saved_workflow(cx: &mut TestAppContext) {
    let (view, vcx) = empty(cx, "run:7");
    vcx.read(|cx| {
        let v = view.read(cx);
        assert!(v.booted && v.names.is_empty());
        assert!(v.start.is_none());
        assert_eq!(v.run_view.as_ref().map(|r| r.task), Some(7));
    });
}

#[gpui::test]
fn new_and_picker_starts_open_the_new_box_with_no_saved_workflow(cx: &mut TestAppContext) {
    let (view, vcx) = empty(cx, "new");
    vcx.read(|cx| {
        let v = view.read(cx);
        assert!(v.start.is_none() && v.flow.is_none());
        assert!(matches!(v.modal, Some(Modal::New)));
    });
    vcx.update(|_, cx| view.update(cx, |v, cx| v.close_modal(cx)));
    vcx.update(|_, cx| view.update(cx, |v, cx| v.start_with("picker", cx)));
    vcx.run_until_parked();
    vcx.read(|cx| assert!(matches!(view.read(cx).modal, Some(Modal::New))));
}

#[gpui::test]
fn brain_down_shows_connecting(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let call: Call = Arc::new(|_, _| Err("Not linked to the brain.".to_string()));
    let (view, vcx) = cx.add_window_view(move |_, cx| WorkflowsView::new(call, cx));
    vcx.run_until_parked();
    vcx.read(|cx| {
        let v = view.read(cx);
        assert!(!v.booted);
        assert_eq!(v.link_error, "Not linked to the brain.");
        assert!(v.flow.is_none());
    });
    assert!(drawn(vcx, "wf-new"));
}
