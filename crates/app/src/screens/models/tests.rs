use super::model::{self, Field, Row};
use super::table::{ModelsView, RowAct};
use super::{BoardView, Call, api};
use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Log = Arc<Mutex<Vec<(String, Option<Value>)>>>;

fn rows_json() -> Value {
    json!([
        {"id": "claude::opus", "agent": "claude", "model": "opus", "rank": 1, "company": "Anthropic", "plan": "Max",
         "cost": "premium", "efforts": ["high"], "capabilities": ["planning", "coding", "ui"], "publicScore": 91.25,
         "publicSource": "board A", "ownScore": 80.0, "good": 4, "bad": 1, "secondary": "codex::gpt-5",
         "pinned": true, "movedBy": "Sam", "movedAt": "2026-11-20T10:00:00Z", "pins": ["planning"], "publicNames": ["opus 5"]},
        {"id": "codex::gpt-5", "agent": "codex", "model": "gpt-5", "rank": 2, "company": "OpenAI", "plan": "Pro",
         "cost": "standard", "capabilities": ["coding", "testing"], "public_score": null, "own_score": null,
         "good": 0, "bad": 0, "disabled": true},
        {"id": "omp::flash", "agent": "omp", "model": "flash", "rank": 3, "company": "Google", "plan": "Free",
         "cost": "cheap", "capabilities": ["cheap", "speed"], "listed": false},
        {"agent": "bad", "model": "no id"}
    ])
}

fn list_json() -> Value {
    json!({"ok": true, "data": {
        "rows": rows_json(),
        "plans": [{"plan": "Max", "companies": ["Anthropic"], "included": true, "jobs": 3, "good": 2, "bad": 1, "available": "yes", "lastAt": "2026-11-21"}],
        "decisions": (1..=7).map(|n| json!({"model": format!("m{n}"), "rule": "capability-rank"})).collect::<Vec<_>>(),
        "file": "/tmp/nixa.db",
        "publicRefresh": {"at": "2026-11-19T08:00:00Z"}
    }})
}

fn view_json() -> Value {
    let mut rows = rows_json();
    rows[0]["secondaryChain"] = json!(["codex::gpt-5", "omp::flash"]);
    json!({
        "rows": rows,
        "listed": 2,
        "pinned": 1,
        "choices": [
            {"capability": "planning", "chosen": {"id": "claude::opus"}, "rule": "capability-pin"},
            {"capability": "vision", "chosen": null, "rule": "no-free-model"}
        ],
        "file": "/tmp/nixa.db"
    })
}

fn fake(log: Log, fail: Arc<Mutex<Option<String>>>) -> Call {
    Arc::new(move |action, body| {
        log.lock().unwrap().push((action.to_string(), body.clone()));
        if let Some(e) = fail.lock().unwrap().clone() {
            return Err(e);
        }
        match action {
            "list" => Ok(list_json()),
            "view" => Ok(view_json()),
            _ => Ok(json!({"rows": rows_json()})),
        }
    })
}

struct Rig<'a> {
    view: Entity<ModelsView>,
    vcx: &'a mut VisualTestContext,
    log: Log,
    fail: Arc<Mutex<Option<String>>>,
}

fn open(cx: &mut TestAppContext) -> Rig<'_> {
    cx.update(nixa_ui::init);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(Mutex::new(None));
    let call = fake(log.clone(), fail.clone());
    let (view, vcx) = cx.add_window_view(move |_, cx| ModelsView::new(call, cx));
    vcx.run_until_parked();
    Rig {
        view,
        vcx,
        log,
        fail,
    }
}

fn selector(name: &str) -> &'static str {
    Box::leak(name.to_string().into_boxed_str())
}

fn click(vcx: &mut VisualTestContext, name: &str) {
    vcx.run_until_parked();
    let bounds = vcx
        .debug_bounds(selector(name))
        .unwrap_or_else(|| panic!("{name} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn drawn(vcx: &mut VisualTestContext, name: &str) -> bool {
    vcx.run_until_parked();
    vcx.debug_bounds(selector(name)).is_some()
}

fn last(log: &Log) -> (String, Value) {
    log.lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(a, _)| a != "list" && a != "view")
        .cloned()
        .map(|(a, b)| (a, b.unwrap_or(Value::Null)))
        .expect("a request")
}

fn writes(log: &Log) -> usize {
    log.lock()
        .unwrap()
        .iter()
        .filter(|(a, _)| a != "list" && a != "view")
        .count()
}

#[test]
fn the_table_is_read_in_both_spellings() {
    let table = model::parse_table(&list_json());
    assert_eq!(table.rows.len(), 3);
    let opus = &table.rows[0];
    assert_eq!(opus.public_score, Some(91.25));
    assert_eq!(opus.moved_by.as_deref(), Some("Sam"));
    assert_eq!(opus.public_names, vec!["opus 5".to_string()]);
    assert!(opus.pinned);
    assert_eq!(table.rows[1].public_score, None);
    assert!(table.rows[1].disabled);
    assert_eq!(table.rows[2].listed, Some(false));
    assert_eq!(
        model::time_text(&table.plans[0].last_at).as_deref(),
        Some("2026-11-21")
    );
    assert_eq!(table.file.as_deref(), Some("/tmp/nixa.db"));
    let bare = model::parse_table(&json!({"rows": "nonsense"}));
    assert!(bare.rows.is_empty());
    assert_eq!(
        model::reply_rows(&json!({"rows": rows_json()})).map(|r| r.len()),
        Some(3)
    );
    assert_eq!(model::reply_rows(&json!({"refreshed": true})), None);
}

#[test]
fn the_store_shape_is_read() {
    let value = json!({
        "rows": [
            {"id": "a::b", "agent": "a", "model": "b", "rank": 1, "listed": true, "company": "C", "plan": "P", "cost": "cheap",
             "capabilities": [], "capabilities_by": "", "public_score": 70.0, "public_source": "", "public_at": 0, "public_names": [],
             "own_score": null, "good": 1, "bad": 0, "usage": null, "results": {}, "secondary": null, "secondary_auto": false,
             "disabled": false, "pinned": false, "pins": [], "moved_by": "", "moved_at": 0},
            {"id": "c::d", "agent": "c", "model": "d", "rank": 2, "moved_by": "Nixa", "moved_at": 1_763_800_000_000_i64},
            {"id": "e::f", "rank": "not a number"}
        ],
        "plans": [{"plan": "P", "companies": ["C"], "included": false, "jobs": 2, "good": 1, "bad": 1, "available": 3, "models": 4, "last_at": 0, "capabilities": {}}],
        "decisions": [{"model": null, "rule": "no-free-model", "capabilities": [], "at": 5}],
        "public_refresh": {"at": 1_763_800_000_000_i64, "attempt_at": 0}
    });
    let table = model::parse_table(&value);
    assert_eq!(table.rows.len(), 2);
    let [a, c] = [&table.rows[0], &table.rows[1]];
    assert_eq!(model::moved_text(a), "—");
    assert_eq!(model::moved_tip(a), "Initial order");
    assert_eq!(
        model::scores_tip(a),
        "No public match\nOur results: 1 good, 0 bad"
    );
    assert_eq!(model::moved_text(c), "Nixa");
    assert_eq!(model::moved_tip(c), "2025-11-22 08:26 UTC");
    assert_eq!(model::plan_cells(&table.plans[0])[3], "3");
    assert_eq!(model::plan_cells(&table.plans[0])[4], "Not used yet");
    assert!(model::info_text(false, &table).ends_with("2025-11-22 08:26 UTC"));
    assert_eq!(
        model::decision_lines(&table.decisions),
        vec!["No model available · no-free-model"]
    );
    assert_eq!(model::board_cells(c)[12], "2025-11-22 08:26 UTC");
    assert_eq!(
        model::time_text(&json!("2026-11-01")).as_deref(),
        Some("2026-11-01")
    );
    assert_eq!(model::time_text(&json!(-5)), None);
}

#[test]
fn texts_match_the_source_screen() {
    let table = model::parse_table(&list_json());
    let [opus, gpt, flash] = [&table.rows[0], &table.rows[1], &table.rows[2]];
    assert_eq!(model::scores(opus), "91.2 / 80.0");
    assert_eq!(model::scores(gpt), "unknown / unknown");
    assert_eq!(
        model::scores_tip(gpt),
        "No public match\nOur results: 0 good, 0 bad"
    );
    assert_eq!(model::name_tip(opus), "Anthropic · Max\nEfforts: high");
    assert_eq!(model::state_text(opus), "available");
    assert_eq!(model::state_text(gpt), "taken out");
    assert_eq!(model::state_text(flash), "unlisted");
    assert_eq!(model::board_state_text(gpt), "disabled");
    assert_eq!(model::secondary_text(opus, &table.rows), "codex · gpt-5");
    assert_eq!(model::secondary_text(gpt, &table.rows), "none");
    assert_eq!(model::secondary_tip(gpt), "No secondary");
    assert_eq!(model::moved_text(gpt), "—");
    assert_eq!(model::moved_tip(gpt), "Initial order");
    let lines = model::decision_lines(&table.decisions);
    assert_eq!(
        lines,
        vec![
            "m7 · capability-rank",
            "m6 · capability-rank",
            "m5 · capability-rank",
            "m4 · capability-rank",
            "m3 · capability-rank"
        ]
    );
    assert_eq!(
        model::decision_lines(&[model::Decision {
            model: None,
            rule: "no-free-model".into()
        }]),
        vec!["No model available · no-free-model"]
    );
    assert_eq!(
        model::info_text(false, &table),
        "Public / our score (0–100). Unknown scores stay unknown. Public refresh: 2026-11-19T08:00:00Z"
    );
    assert!(model::info_text(false, &model::Table::default()).ends_with("not yet"));
    assert_eq!(model::info_text(true, &table), model::SAVING_TEXT);
    assert_eq!(
        model::plan_cells(&table.plans[0]),
        ["Max", "3", "2 / 1", "yes", "2026-11-21"].map(String::from)
    );
    assert_eq!(
        model::plan_tip(&table.plans[0]),
        "Anthropic · Included in plan"
    );
    assert_eq!(model::shown(&table.rows, "coding").len(), 2);
    assert_eq!(model::shown(&table.rows, "").len(), 3);
}

#[test]
fn bodies_match_the_brain_actions() {
    let row = Row {
        id: "a::b".into(),
        rank: 1,
        ..Row::default()
    };
    assert_eq!(model::up_rank(&row), 1);
    assert_eq!(model::down_rank(&row, 1), 1);
    assert_eq!(
        model::down_rank(
            &Row {
                rank: 2,
                ..row.clone()
            },
            3
        ),
        3
    );
    assert_eq!(
        model::up_rank(&Row {
            rank: 3,
            ..row.clone()
        }),
        2
    );
    assert_eq!(model::drop_body(Some("a::b"), &row), None);
    assert_eq!(model::drop_body(None, &row), None);
    assert_eq!(
        model::drop_body(Some("x::y"), &row),
        Some(json!({"id": "x::y", "rank": 1}))
    );
    assert_eq!(model::pin_body(&row), json!({"id": "a::b", "pinned": true}));
    assert_eq!(
        model::out_body(&row),
        json!({"id": "a::b", "disabled": true})
    );
    assert_eq!(model::rank_body(""), json!({"capabilities": []}));
    assert_eq!(model::rank_body("ui"), json!({"capabilities": ["ui"]}));
    assert_eq!(
        model::fields_body("a::b", " ui, ,coding ", " c::d ", "", "x , y"),
        json!({"id": "a::b", "capabilities": ["ui", "coding"], "secondary": "c::d", "pins": [], "public_names": ["x", "y"]})
    );
    assert_eq!(api::route("list").1, "/models");
    assert_eq!(
        api::route("view"),
        ("GET", "/models/view".into(), api::CALL_WAIT)
    );
    assert_eq!(api::route("refresh").2, api::REFRESH_WAIT);
    assert_eq!(
        api::route("move"),
        ("POST", "/models/move".into(), api::CALL_WAIT)
    );
}

#[test]
fn the_board_lines_show_every_column() {
    let board = model::parse_board(&view_json());
    assert_eq!(
        model::board_title(&board),
        "Model leaderboard · 2 listed · 1 pinned · 3 rows · Read only"
    );
    assert_eq!(
        model::choice_text(&board.choices[0]),
        "planning: claude::opus"
    );
    assert_eq!(
        model::choice_text(&board.choices[1]),
        "vision: No available model"
    );
    let cells = model::board_cells(&board.rows[0]);
    assert_eq!(cells.len(), model::BOARD_COLUMNS.len());
    assert_eq!(cells[9], "codex::gpt-5 → omp::flash");
    assert_eq!(cells[10], "★ Pinned");
    assert_eq!(model::board_cells(&board.rows[1])[9], "—");
    assert_eq!(model::board_cells(&board.rows[2])[13], "unlisted");
    assert_eq!(model::board_width(), 2280.0);
    assert!(model::failed_text("down").contains("Showing last received snapshot"));
}

#[test]
fn commands_and_show_names() {
    let names: Vec<&str> = super::commands().iter().map(|c| c.0).collect();
    assert_eq!(
        names,
        vec!["Model leaderboard", "Model leaderboard: read only"]
    );
}

#[gpui::test]
fn the_table_draws_rows_plans_and_decisions(cx: &mut TestAppContext) {
    let rig = open(cx);
    for id in ["claude::opus", "codex::gpt-5", "omp::flash"] {
        assert!(drawn(rig.vcx, &format!("model-{id}")), "{id}");
    }
    assert!(drawn(rig.vcx, "plan-0"));
    assert!(!drawn(rig.vcx, "models-empty"));
    assert!(!drawn(rig.vcx, "models-edit"));
    rig.view.read_with(rig.vcx, |v, _| {
        assert!(v.connected);
        assert_eq!(v.table.decisions.len(), 7);
    });
}

#[gpui::test]
fn filter_chips_hide_rows(cx: &mut TestAppContext) {
    let rig = open(cx);
    click(rig.vcx, "cap-cheap");
    let first = rig.vcx.debug_bounds("model-omp::flash").expect("flash row");
    let top = rig.vcx.debug_bounds("plan-0").expect("plans");
    assert!(first.origin.y > top.origin.y);
    rig.view.read_with(rig.vcx, |v, _| {
        assert_eq!(v.filter, "cheap");
        assert_eq!(model::shown(&v.table.rows, &v.filter).len(), 1);
    });
    click(rig.vcx, "cap-all");
    rig.view.read_with(rig.vcx, |v, _| assert_eq!(v.filter, ""));
    let opus = rig
        .vcx
        .debug_bounds("model-claude::opus")
        .expect("opus row");
    let flash = rig.vcx.debug_bounds("model-omp::flash").expect("flash row");
    assert!(opus.origin.y < flash.origin.y);
}

#[gpui::test]
fn rerank_sends_the_filter(cx: &mut TestAppContext) {
    let rig = open(cx);
    click(rig.vcx, "cap-ui");
    click(rig.vcx, "rank");
    assert_eq!(
        last(&rig.log),
        ("rank".into(), json!({"capabilities": ["ui"]}))
    );
    click(rig.vcx, "public");
    assert_eq!(last(&rig.log).0, "refresh");
}

#[gpui::test]
fn row_buttons_send_moves_and_edits(cx: &mut TestAppContext) {
    let rig = open(cx);
    click(rig.vcx, "up-codex::gpt-5");
    assert_eq!(
        last(&rig.log),
        ("move".into(), json!({"id": "codex::gpt-5", "rank": 1}))
    );
    click(rig.vcx, "down-omp::flash");
    assert_eq!(
        last(&rig.log),
        ("move".into(), json!({"id": "omp::flash", "rank": 3}))
    );
    click(rig.vcx, "pin-claude::opus");
    assert_eq!(
        last(&rig.log),
        (
            "edit".into(),
            json!({"id": "claude::opus", "pinned": false})
        )
    );
    click(rig.vcx, "out-codex::gpt-5");
    assert_eq!(
        last(&rig.log),
        (
            "edit".into(),
            json!({"id": "codex::gpt-5", "disabled": false})
        )
    );
}

#[gpui::test]
fn drag_a_row_onto_another_sets_its_rank(cx: &mut TestAppContext) {
    let rig = open(cx);
    let from = rig
        .vcx
        .debug_bounds("drag-omp::flash")
        .expect("drag handle")
        .center();
    let to = rig
        .vcx
        .debug_bounds("model-claude::opus")
        .expect("target row")
        .center();
    rig.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    rig.vcx.run_until_parked();
    rig.view.read_with(rig.vcx, |v, _| {
        assert_eq!(v.drag.as_deref(), Some("omp::flash"))
    });
    rig.vcx
        .simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::none());
    rig.vcx
        .simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    rig.vcx.run_until_parked();
    assert_eq!(
        last(&rig.log),
        ("move".into(), json!({"id": "omp::flash", "rank": 1}))
    );
    rig.view
        .read_with(rig.vcx, |v, _| assert!(v.drag.is_none()));
}

#[gpui::test]
fn drop_on_itself_or_outside_moves_nothing(cx: &mut TestAppContext) {
    let rig = open(cx);
    let from = rig
        .vcx
        .debug_bounds("drag-omp::flash")
        .expect("drag handle")
        .center();
    rig.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    rig.vcx
        .simulate_mouse_up(from, MouseButton::Left, Modifiers::none());
    rig.vcx.run_until_parked();
    assert_eq!(writes(&rig.log), 0);
    rig.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    let header = rig.vcx.debug_bounds("reload").expect("reload").center();
    rig.vcx
        .simulate_mouse_up(header, MouseButton::Left, Modifiers::none());
    rig.vcx.run_until_parked();
    rig.view
        .read_with(rig.vcx, |v, _| assert!(v.drag.is_none()));
    assert_eq!(writes(&rig.log), 0);
}

#[gpui::test]
fn edit_panel_saves_fields_and_cost(cx: &mut TestAppContext) {
    let rig = open(cx);
    click(rig.vcx, "edit-claude::opus");
    assert!(drawn(rig.vcx, "models-edit"));
    rig.view.read_with(rig.vcx, |v, cx| {
        assert_eq!(v.field(Field::Capabilities, cx), "planning, coding, ui");
        assert_eq!(v.field(Field::Secondary, cx), "codex::gpt-5");
        assert_eq!(v.field(Field::Pins, cx), "planning");
        assert_eq!(v.field(Field::PublicNames, cx), "opus 5");
    });
    rig.view.update(rig.vcx, |v, cx| {
        let input = v.inputs[0].1.clone();
        input.update(cx, |i, cx| i.set_value("ui, vision", cx));
    });
    click(rig.vcx, "save-fields");
    assert_eq!(
        last(&rig.log),
        (
            "edit".into(),
            json!({"id": "claude::opus", "capabilities": ["ui", "vision"], "secondary": "codex::gpt-5", "pins": ["planning"], "public_names": ["opus 5"]})
        )
    );
    click(rig.vcx, "cost-cheap");
    assert_eq!(
        last(&rig.log),
        (
            "edit".into(),
            json!({"id": "claude::opus", "cost": "cheap"})
        )
    );
    click(rig.vcx, "close-edit");
    rig.view
        .read_with(rig.vcx, |v, _| assert!(v.selected.is_none()));
}

#[gpui::test]
fn escape_closes_the_edit_panel_first(cx: &mut TestAppContext) {
    let rig = open(cx);
    rig.view
        .update(rig.vcx, |v, cx| v.press(RowAct::Edit, "codex::gpt-5", cx));
    assert!(drawn(rig.vcx, "models-edit"));
    rig.view.update_in(rig.vcx, |v, window, cx| {
        window.focus(&gpui::Focusable::focus_handle(v, cx))
    });
    rig.vcx.simulate_keystrokes("escape");
    rig.view
        .read_with(rig.vcx, |v, _| assert!(v.selected.is_none()));
}

#[gpui::test]
fn errors_show_and_keep_rows(cx: &mut TestAppContext) {
    let rig = open(cx);
    *rig.fail.lock().unwrap() = Some("Only the Nixa window may change the model table.".into());
    click(rig.vcx, "pin-codex::gpt-5");
    assert!(drawn(rig.vcx, "models-error"));
    assert!(drawn(rig.vcx, "model-codex::gpt-5"));
    *rig.fail.lock().unwrap() = None;
    click(rig.vcx, "reload");
    rig.view
        .read_with(rig.vcx, |v, _| assert!(v.error.is_empty()));
}

#[gpui::test]
fn no_brain_shows_connecting(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let call: Call = Arc::new(|_, _| Err("nixa-brain is not running".into()));
    let (view, vcx) = cx.add_window_view(move |_, cx| ModelsView::new(call, cx));
    assert!(drawn(vcx, "models-empty"));
    assert!(drawn(vcx, "models-error"));
    view.read_with(vcx, |v, _| assert!(!v.connected));
}

#[gpui::test]
fn tasks_opens_without_a_workspace(cx: &mut TestAppContext) {
    let rig = open(cx);
    click(rig.vcx, "tasks");
    assert_eq!(rig.vcx.update(|_, cx| cx.windows().len()), 2);
}

#[gpui::test]
fn the_poll_reads_again_when_idle(cx: &mut TestAppContext) {
    let rig = open(cx);
    let before = rig.log.lock().unwrap().len();
    rig.vcx.executor().advance_clock(Duration::from_secs(4));
    rig.vcx.run_until_parked();
    assert!(rig.log.lock().unwrap().len() > before);
    rig.view
        .update(rig.vcx, |v, cx| v.start_drag("omp::flash", cx));
    let during = rig.log.lock().unwrap().len();
    rig.vcx.executor().advance_clock(Duration::from_secs(4));
    rig.vcx.run_until_parked();
    assert_eq!(rig.log.lock().unwrap().len(), during);
}

#[gpui::test]
fn the_board_is_read_only(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(Mutex::new(None));
    let base = fake(log.clone(), fail.clone());
    let call: Call = Arc::new(move |action, body| {
        let mut reply = base(action, body)?;
        reply.as_object_mut().unwrap().remove("file");
        Ok(reply)
    });
    let (view, vcx) = cx.add_window_view(move |_, cx| BoardView::new(call, cx));
    assert!(drawn(vcx, "board-claude::opus"));
    assert!(drawn(vcx, "choice-planning"));
    assert!(drawn(vcx, "board-title"));
    assert!(!drawn(vcx, "board-empty"));
    assert!(!drawn(vcx, "board-status"));
    let bar = vcx.debug_bounds("board-horizontal-scroll").unwrap();
    let first = vcx.debug_bounds("board-claude::opus").unwrap().left();
    vcx.simulate_click(
        gpui::point(bar.right() - gpui::px(2.0), bar.center().y),
        Modifiers::none(),
    );
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("board-claude::opus").unwrap().left() < first);
    *fail.lock().unwrap() = Some("down".into());
    click(vcx, "board-reload");
    assert!(drawn(vcx, "board-error"));
    assert!(drawn(vcx, "board-claude::opus"));
    view.read_with(vcx, |v, _| assert_eq!(v.board.rows.len(), 3));
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|(a, b)| a == "view" && b.is_none())
    );
}

#[gpui::test]
fn a_changed_event_reloads_open_views(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(Mutex::new(None));
    let call = fake(log.clone(), fail);
    let (_view, vcx) = cx.add_window_view(move |_, cx| {
        let table = super::new_table(call.clone(), cx);
        let board = super::new_board(call, cx);
        ModelsHolder(table, board)
    });
    vcx.run_until_parked();
    let before = log.lock().unwrap().len();
    let changed = vcx.update(|_, cx| super::apply(&json!({"type": "models_changed"}), cx));
    assert!(changed);
    let other = vcx.update(|_, cx| super::apply(&json!({"type": "custom_agents_changed"}), cx));
    assert!(!other);
    vcx.run_until_parked();
    assert_eq!(log.lock().unwrap().len(), before + 2);
}

struct ModelsHolder(Entity<ModelsView>, Entity<BoardView>);

impl gpui::Render for ModelsHolder {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::ParentElement;
        gpui::div().child(self.0.clone()).child(self.1.clone())
    }
}

#[test]
fn scores_status_uses_the_brain_setting_and_note() {
    let table =
        model::parse_table(&json!({"scoresOnline": false, "scoresNote": "Scores are off."}));
    assert_eq!(
        model::scores_status(table.scores_online, &table.scores_note),
        Some("Scores are off.")
    );
    let board = model::parse_board(&json!({"scoresOnline": true}));
    assert_eq!(
        model::scores_status(board.scores_online, &board.scores_note),
        Some("Public scores are on.")
    );
}
