use super::api::{self, Call, Host};
use super::drop::{Drop, option_id, trigger_id};
use super::model::{self, Col, Filters, Sort};
use super::view::{Panel, TasksView, set, set_area};
use gpui::{App, Entity, Modifiers, TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Log = Arc<Mutex<Vec<(String, Value)>>>;

static FOCUSED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static SETTINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn fake_focus(id: &str, _: &mut App) -> bool {
    FOCUSED.lock().unwrap().push(id.to_string());
    id == "m4"
}

fn fake_space(_: &App) -> String {
    "s1".into()
}

fn fake_settings(part: &str, _: &mut App) {
    SETTINGS.lock().unwrap().push(part.to_string());
}

fn fake_url(url: &str, _: &mut App) {
    OPENED.lock().unwrap().push(url.to_string());
}

fn host() -> Host {
    Host {
        focus: fake_focus,
        active_space: fake_space,
        settings: fake_settings,
        open_url: fake_url,
    }
}

fn base_options() -> Value {
    json!({
        "spaces": [{"id": "s1", "name": "nixa-app", "folder": "/p/nixa-app"}, {"id": "s2", "name": "nixa", "folder": "/p/nixa"}],
        "assignees": [
            {"value": "codex", "label": "Codex", "kind": "agent"},
            {"value": "member:dev", "label": "Developer", "kind": "member", "detail": "builds"},
            {"value": "custom:a1", "label": "Scout", "kind": "custom"}
        ],
        "workflows": [{"name": "build-review", "title": "Build then review", "when": "code work"}]
    })
}

fn options() -> Value {
    base_options()
}

fn tasks() -> Vec<Value> {
    vec![
        json!({"id": 1, "title": "Write the queue", "description": "Port the queue.", "status": "todo", "space_id": "s1", "assigned": "codex", "assigned_kind": "agent", "group": "project", "source": "manual", "created_at": 10, "comments": 2}),
        task_two(),
        json!({"id": 3, "title": "Brain gap", "description": "", "status": "blocked", "needs_user": true, "blocked_on": "user", "group": "nixa-gap", "source": "nixa", "created_at": 30}),
    ]
}

fn task_two() -> Value {
    json!({"id": 2, "title": "Fix login", "description": "<p>Broken <b>login</b></p><ul><li>one</li><li>two</li></ul>", "status": "doing", "space_id": "s1", "assigned": "dev", "assigned_kind": "member", "terminal_id": "m4", "claimed_by": "Codex", "group": "project", "source": "agent", "created_at": 20})
}

struct Brain {
    log: Log,
    tasks: Arc<Mutex<Vec<Value>>>,
}

fn fake(brain: &Brain) -> Call {
    let log = brain.log.clone();
    let store = brain.tasks.clone();
    Arc::new(move |action, args| {
        log.lock().unwrap().push((action.to_string(), args.clone()));
        let mut list = store.lock().unwrap();
        let id = args.get("id").and_then(Value::as_i64).unwrap_or(0);
        let find = |list: &Vec<Value>| list.iter().find(|t| t["id"] == json!(id)).cloned();
        match action {
            "options" => Ok(options()),
            "list" => {
                let status = args.get("status").and_then(Value::as_str).unwrap_or("");
                let wanted: Vec<&str> = status.split(',').filter(|s| !s.is_empty()).collect();
                let out: Vec<Value> = list
                    .iter()
                    .filter(|t| {
                        wanted.is_empty() || wanted.contains(&t["status"].as_str().unwrap_or(""))
                    })
                    .cloned()
                    .collect();
                Ok(json!({"tasks": out}))
            }
            "show" => match find(&list) {
                Some(task) => Ok(json!({"task": task, "comments": [
                    {"id": 7, "author_kind": "agent", "author": "Codex", "kind": "progress", "text": "Half done", "at": 5, "links": {"files": ["a.rs"], "pr": "https://github.com/x/pr/1"}}
                ], "run": if id == 1 { json!({"state": "running", "line": "Stage 1 of 2", "title": "Build then review"}) } else { Value::Null }})),
                None => Err(format!("No task #{id}.")),
            },
            "update" => {
                let Some(task) = list.iter_mut().find(|t| t["id"] == json!(id)) else {
                    return Err(format!("No task #{id}."));
                };
                for key in ["status", "title", "description", "workflow"] {
                    if let Some(v) = args.get(key) {
                        task[key] = v.clone();
                    }
                }
                if let Some(v) = args.get("space") {
                    task["space_id"] = if v == "none" { json!("") } else { v.clone() };
                }
                Ok(json!({"task": task.clone()}))
            }
            "add" => {
                let next = list.len() as i64 + 10;
                let task = json!({"id": next, "title": args["title"], "description": args.get("description").cloned().unwrap_or(json!("")), "status": "todo", "space_id": args.get("space").cloned().unwrap_or(json!("")), "assigned": args.get("assign").cloned().unwrap_or(json!("")), "group": args["group"], "created_at": 99});
                list.push(task.clone());
                Ok(json!({"task": task}))
            }
            "start" => {
                let Some(task) = list.iter_mut().find(|t| t["id"] == json!(id)) else {
                    return Err("gone".into());
                };
                task["status"] = json!("doing");
                task["terminal_id"] = json!("m9");
                Ok(json!({"task": task.clone()}))
            }
            "delete" => {
                list.retain(|t| t["id"] != json!(id));
                Ok(json!({"ok": true}))
            }
            "continue" => Ok(json!({"task": find(&list), "how": "handed", "terminal": "m12"})),
            "speak" | "speak_stop" | "comment" | "release" | "archive" | "workflow" => {
                Ok(json!({"task": find(&list)}))
            }
            other => Err(format!("unknown {other}")),
        }
    })
}

fn brain() -> Brain {
    Brain {
        log: Arc::new(Mutex::new(Vec::new())),
        tasks: Arc::new(Mutex::new(tasks())),
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    b: &Brain,
) -> (Entity<TasksView>, &'a mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let call = fake(b);
    let (view, vcx) = cx.add_window_view(move |_, cx| TasksView::new(call, host(), 0, cx));
    vcx.run_until_parked();
    (view, vcx)
}

fn bounds_of(vcx: &mut VisualTestContext, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
    vcx.run_until_parked();
    let name: &'static str = Box::leak(selector.to_string().into_boxed_str());
    vcx.debug_bounds(name)
}

fn click(vcx: &mut VisualTestContext, selector: &str) {
    let bounds = bounds_of(vcx, selector).unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn drawn(vcx: &mut VisualTestContext, selector: &str) -> bool {
    bounds_of(vcx, selector).is_some()
}

fn calls(b: &Brain, action: &str) -> Vec<Value> {
    b.log
        .lock()
        .unwrap()
        .iter()
        .filter(|(a, _)| a == action)
        .map(|(_, v)| v.clone())
        .collect()
}

fn last(b: &Brain, action: &str) -> Value {
    calls(b, action)
        .pop()
        .unwrap_or_else(|| panic!("no {action} call"))
}

fn settle(vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(Duration::from_millis(400));
    vcx.run_until_parked();
}

#[test]
fn list_args_follow_the_filters() {
    let open = model::OPEN_STATUSES.join(",");
    assert_eq!(
        model::list_args(&Filters::default(), ""),
        json!({"status": open})
    );
    let done = Filters {
        done: true,
        ..Filters::default()
    };
    assert_eq!(
        model::list_args(&done, " fix "),
        json!({"q": "fix", "old": true})
    );
    let closed = Filters {
        status: "done".into(),
        space: "none".into(),
        assigned: "codex".into(),
        ..Filters::default()
    };
    assert_eq!(
        model::list_args(&closed, ""),
        json!({"status": "done", "assigned": "codex", "old": true})
    );
    let archived = Filters {
        archived: true,
        status: "open".into(),
        group: "nixa-gap".into(),
        ..Filters::default()
    };
    assert_eq!(
        model::list_args(&archived, ""),
        json!({"status": open, "group": "nixa-gap", "old": true, "archived": true})
    );
    let tasks = model::parse_tasks(&json!({"tasks": tasks()}));
    let no_space = Filters {
        space: "none".into(),
        ..Filters::default()
    };
    assert_eq!(
        model::shown(&tasks, &no_space)
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![3]
    );
}

#[test]
fn tasks_read_both_key_styles() {
    let (snake, camel) = (
        json!({"id": 4, "space_id": "s1", "needs_user": true, "terminal_id": "m2"}),
        json!({"id": "4", "spaceId": "s1", "needsUser": true, "terminalId": "m2"}),
    );
    let snake = model::parse_task(&snake).unwrap();
    let camel = model::parse_task(&camel).unwrap();
    assert_eq!(snake, camel);
    assert!(model::parse_task(&json!({"title": "no id"})).is_none());
    let old = model::Task {
        id: 4,
        comments: 3,
        run_box: "box".into(),
        ..Default::default()
    };
    let merged = model::merge_task(&old, &json!({"id": 4, "title": "new"}));
    assert_eq!(
        (
            merged.title.as_str(),
            merged.comments,
            merged.run_box.as_str()
        ),
        ("new", 3, "box")
    );
    let o = model::parse_options(&options());
    assert_eq!(o.statuses.len(), 7);
    assert_eq!(model::nixa_space_id(&o.spaces), "s2");
    let custom = model::parse_task(&json!({"id": 9, "assigned": "a1", "assigned_kind": "member", "then_member": "custom:a1"})).unwrap();
    assert_eq!(model::assigned_value(&o, &custom), "custom:a1");
    assert_eq!(model::assigned_name(&o, &custom), "Scout");
    assert_eq!(model::then_value(&o, &custom), "custom:a1");
    assert!(model::then_options(&o).iter().any(|option| option.value == "custom:a1"));
}

#[test]
fn sort_and_group_match_the_table() {
    let o = model::parse_options(&options());
    let list = model::parse_tasks(&json!({"tasks": tasks()}));
    let ids = |s: Sort| {
        model::sort_tasks(&list, s, &o)
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(Sort::default()), vec![1, 2, 3]);
    assert_eq!(ids(model::flip(Sort::default(), Col::Id)), vec![3, 2, 1]);
    assert_eq!(
        ids(Sort {
            col: Col::Status,
            up: true
        }),
        vec![1, 2, 3]
    );
    assert_eq!(
        ids(Sort {
            col: Col::Assigned,
            up: true
        }),
        vec![1, 2, 3]
    );
    assert_eq!(
        ids(Sort {
            col: Col::Assigned,
            up: false
        }),
        vec![2, 1, 3]
    );
    let groups = model::group_tasks(&list, &o);
    assert_eq!(
        groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
        vec!["nixa-app", "No space"]
    );
    assert_eq!(model::assigned_name(&o, &list[1]), "Developer");
}

#[test]
fn texts_match_the_old_window() {
    assert_eq!(model::ago(0, 5), "");
    assert_eq!(model::ago(100, 30_000), "just now");
    assert_eq!(model::ago(1, 3 * 3_600_000), "3 h ago");
    assert_eq!(model::ago(1, 36 * 3_600_000), "2 days ago");
    assert_eq!(model::clock_text(0, 330), "1970-01-01 05:30");
    let task = model::Task {
        id: 5,
        title: "x".repeat(70),
        status: "doing".into(),
        ..Default::default()
    };
    let undo = model::undo_of(&task, "done");
    assert_eq!(undo.back, "todo");
    assert!(model::undo_text(&undo).ends_with("… is done. It left the list."));
    assert_eq!(
        model::run_tip(&task, "", ""),
        "Run now. Pick a Space and an Assigned agent first."
    );
    assert_eq!(
        model::continue_note("handed", "m3"),
        "Handed over. The new agent is in terminal m3."
    );
    assert_eq!(model::window_title(2), "Tasks (2)");
    assert!(model::is_html("<p>hi</p>"));
    assert!(!model::is_html("a < b and c > d"));
    assert_eq!(
        super::description::parse("<p>Broken <b>login</b></p><ul><li>one</li><li>two &amp; more</li></ul>").0,
        "Broken login\n• one\n• two & more"
    );
    let (text, spans) = super::description::parse(
        "<p>Read <a href='https://example.com/?a=1&amp;b=2'><strong>the guide</strong></a></p><script>hidden</script><a href='file:///private'>plain</a>",
    );
    assert_eq!(text, "Read the guide\nplain");
    let guide = spans.iter().find(|span| span.link.is_some()).unwrap();
    assert_eq!(text[guide.range.clone()].trim(), "the guide");
    assert!(guide.bold);
    assert_eq!(guide.link.as_deref(), Some("https://example.com/?a=1&b=2"));
    assert!(spans.iter().all(|span| span.link.as_deref() != Some("file:///private")));
    assert!(model::safe_link("https://tasks.example/x"));
    assert!(model::safe_link("mailto:a@b.c"));
    assert!(!model::safe_link("file:///etc/passwd"));
    assert!(!model::safe_link("https:///x"));
    assert_eq!(
        api::body("list", json!({"q": 1})),
        json!({"action": "list", "args": {"q": 1}})
    );
    assert_eq!(api::wait_of("speak"), Duration::from_secs(900));
    assert_eq!(option_id(Drop::FStatus, "a b"), "tasks-dd-f-status-a-b");
    assert_eq!(trigger_id(Drop::NGroup), "tasks-dd-n-group");
}

#[gpui::test]
fn the_table_loads_sorts_and_groups(cx: &mut TestAppContext) {
    let b = brain();
    let (view, vcx) = open(cx, &b);
    assert!(drawn(vcx, "tasks-row-1"));
    assert!(drawn(vcx, "tasks-row-3"));
    assert!(drawn(vcx, "tasks-waiting"));
    assert_eq!(last(&b, "list"), json!({"status": "blocked,review"}));
    assert_eq!(
        calls(&b, "list")[0],
        json!({"status": model::OPEN_STATUSES.join(",")})
    );
    view.read_with(vcx, |v, _| assert_eq!(v.waiting, 1));
    view.read_with(vcx, |v, _| assert_eq!(v.title_shown, "Tasks (1)"));
    click(vcx, "tasks-col-id");
    view.read_with(vcx, |v, _| {
        assert_eq!(
            v.sort,
            Sort {
                col: Col::Id,
                up: false
            }
        )
    });
    click(vcx, "tasks-col-title");
    view.read_with(vcx, |v, _| {
        assert_eq!(
            v.sort,
            Sort {
                col: Col::Title,
                up: true
            }
        )
    });
    click(vcx, "tasks-group");
    view.read_with(vcx, |v, _| assert!(!v.grouped));
}

#[gpui::test]
fn filters_reload_the_list(cx: &mut TestAppContext) {
    let b = brain();
    let (view, vcx) = open(cx, &b);
    click(vcx, &trigger_id(Drop::FStatus));
    view.read_with(vcx, |v, _| assert_eq!(v.open, Some(Drop::FStatus)));
    click(vcx, &option_id(Drop::FStatus, "blocked"));
    let args = calls(&b, "list").into_iter().rev().nth(1).unwrap();
    assert_eq!(args, json!({"status": "blocked"}));
    view.read_with(vcx, |v, _| {
        assert_eq!(v.filters.status, "blocked");
        assert!(v.open.is_none());
    });
    click(vcx, "tasks-done");
    let args = calls(&b, "list").into_iter().rev().nth(1).unwrap();
    assert_eq!(args, json!({"status": "blocked", "old": true}));
    click(vcx, "tasks-archived");
    let args = calls(&b, "list").into_iter().rev().nth(1).unwrap();
    assert_eq!(args["archived"], json!(true));
    let search = view.read_with(vcx, |v, _| v.inputs.search.clone());
    vcx.update(|_, cx| set(&search, "login", cx));
    view.update(vcx, |v, cx| v.reload(cx));
    vcx.run_until_parked();
    assert_eq!(
        calls(&b, "list").into_iter().rev().nth(1).unwrap()["q"],
        json!("login")
    );
}

#[gpui::test]
fn done_and_undo_from_the_row(cx: &mut TestAppContext) {
    let b = brain();
    let (view, vcx) = open(cx, &b);
    click(vcx, "tasks-row-done-1");
    assert_eq!(last(&b, "update"), json!({"id": 1, "status": "done"}));
    assert!(drawn(vcx, "tasks-undo-do"));
    view.read_with(vcx, |v, _| {
        assert_eq!(v.undo.as_ref().map(|u| u.back.as_str()), Some("todo"))
    });
    settle(vcx);
    view.read_with(vcx, |v, _| assert!(!v.tasks.iter().any(|t| t.id == 1)));
    click(vcx, "tasks-undo-do");
    assert_eq!(last(&b, "update"), json!({"id": 1, "status": "todo"}));
    settle(vcx);
    view.read_with(vcx, |v, _| {
        assert!(v.tasks.iter().any(|t| t.id == 1));
        assert!(v.undo.is_none());
    });
}

#[gpui::test]
fn run_without_a_space_opens_the_task(cx: &mut TestAppContext) {
    let b = brain();
    let (view, vcx) = open(cx, &b);
    click(vcx, "tasks-row-run-1");
    assert_eq!(last(&b, "start"), json!({"id": 1}));
    view.update(vcx, |v, cx| {
        let task = v.tasks.iter().find(|t| t.id == 3).cloned().unwrap();
        v.tasks.push(model::Task {
            id: 9,
            status: "todo".into(),
            ..task
        });
        cx.notify();
    });
    click(vcx, "tasks-row-run-9");
    view.read_with(vcx, |v, _| {
        assert_eq!(v.selected, Some(9));
        assert!(v.error.contains("needs a Space and an Assigned agent"));
    });
    assert_eq!(calls(&b, "start").len(), 1);
}

#[gpui::test]
fn the_detail_panel_edits_the_task(cx: &mut TestAppContext) {
    let b = brain();
    let (view, vcx) = open(cx, &b);
    click(vcx, "tasks-row-1");
    assert_eq!(last(&b, "show"), json!({"id": 1}));
    view.read_with(vcx, |v, cx| {
        assert_eq!(v.panel, Some(Panel::Detail));
        assert_eq!(v.inputs.title.read(cx).value(), "Write the queue");
        assert_eq!(v.detail.as_ref().unwrap().comments.len(), 1);
    });
    assert!(drawn(vcx, "tasks-d-continue"));
    assert!(drawn(vcx, "tasks-d-run-pause"));
    assert!(drawn(vcx, "tasks-c-7-link-0"));
    let title = view.read_with(vcx, |v, _| v.inputs.title.clone());
    vcx.update(|_, cx| set(&title, "Write the queue now", cx));
    click(vcx, "tasks-d-text-save");
    assert_eq!(
        last(&b, "update"),
        json!({"id": 1, "title": "Write the queue now"})
    );
    click(vcx, &trigger_id(Drop::DSpace));
    click(vcx, &option_id(Drop::DSpace, "s2"));
    assert_eq!(last(&b, "update"), json!({"id": 1, "space": "s2"}));
    click(vcx, &trigger_id(Drop::DWorkflow));
    click(vcx, &option_id(Drop::DWorkflow, "build-review"));
    assert_eq!(
        last(&b, "update"),
        json!({"id": 1, "workflow": "build-review"})
    );
    click(vcx, &trigger_id(Drop::DGroup));
    click(vcx, &option_id(Drop::DGroup, "nixa-gap"));
    assert_eq!(last(&b, "update"), json!({"id": 1, "group": "nixa-gap"}));
    click(vcx, "tasks-d-run-pause");
    assert_eq!(last(&b, "workflow"), json!({"id": 1, "do": "pause"}));
    click(vcx, "tasks-d-run-cancel");
    click(vcx, "tasks-run-cancel-yes");
    assert_eq!(last(&b, "workflow"), json!({"id": 1, "do": "cancel"}));
    view.update(vcx, |v, cx| v.open_link("https://github.com/x/pr/1", cx));
    assert_eq!(
        OPENED.lock().unwrap().last().cloned(),
        Some("https://github.com/x/pr/1".into())
    );
    view.update(vcx, |v, cx| v.open_link("file:///etc/hosts", cx));
    view.read_with(vcx, |v, _| {
        assert_eq!(v.error, "This is not a web or mail link: file:///etc/hosts")
    });
    view.update(vcx, |v, _| v.error.clear());
    let comment = view.read_with(vcx, |v, _| v.inputs.comment.clone());
    view.update(vcx, |v, cx| v.add_comment(cx));
    view.read_with(vcx, |v, _| assert_eq!(v.error, "Type the comment first."));
    vcx.update(|_, cx| set_area(&comment, "Looks good", cx));
    view.update_in(vcx, |v, window, cx| {
        v.pick(Drop::DKind, "result", window, cx)
    });
    view.update(vcx, |v, cx| v.add_comment(cx));
    vcx.run_until_parked();
    assert_eq!(
        last(&b, "comment"),
        json!({"id": 1, "kind": "result", "text": "Looks good"})
    );
    click(vcx, "tasks-d-desc-play");
    assert_eq!(last(&b, "speak"), json!({"id": 1}));
    click(vcx, "tasks-panel-close");
    view.read_with(vcx, |v, _| assert!(v.panel.is_none()));
}

#[gpui::test]
fn a_held_task_asks_before_hand_over(cx: &mut TestAppContext) {
    let b = brain();
    let (view, vcx) = open(cx, &b);
    click(vcx, "tasks-row-2");
    assert!(drawn(vcx, "tasks-d-desc-view"));
    click(vcx, "tasks-d-desc-edit");
    view.read_with(vcx, |v, _| assert_eq!(v.raw_text, Some(2)));
    click(vcx, "tasks-d-terminal");
    assert_eq!(FOCUSED.lock().unwrap().last().cloned(), Some("m4".into()));
    click(vcx, &trigger_id(Drop::DAssign));
    click(vcx, &option_id(Drop::DAssign, "codex"));
    view.read_with(vcx, |v, _| {
        assert_eq!(v.confirm_handover, Some((2, "codex".into())))
    });
    assert!(calls(&b, "update").is_empty());
    click(vcx, "tasks-handover-yes");
    assert_eq!(last(&b, "continue"), json!({"id": 2, "assign": "codex"}));
    view.read_with(vcx, |v, _| {
        assert!(v.confirm_handover.is_none());
        assert_eq!(v.note, "#2: Handed over. The new agent is in terminal m12.");
    });
    click(vcx, "tasks-d-release");
    assert_eq!(last(&b, "release"), json!({"id": 2}));
    click(vcx, "tasks-d-delete");
    assert!(drawn(vcx, "tasks-delete-no"));
    click(vcx, "tasks-delete-yes");
    assert_eq!(last(&b, "delete"), json!({"id": 2, "confirm": true}));
    view.read_with(vcx, |v, _| {
        assert!(v.panel.is_none());
        assert!(!v.tasks.iter().any(|t| t.id == 2));
    });
}

#[gpui::test]
fn the_new_task_box_adds_and_starts(cx: &mut TestAppContext) {
    let b = brain();
    let (view, vcx) = open(cx, &b);
    click(vcx, "tasks-new");
    view.read_with(vcx, |v, _| {
        assert_eq!(v.panel, Some(Panel::New));
        assert_eq!(v.draft.space, "s1");
    });
    click(vcx, "tasks-n-add");
    view.read_with(vcx, |v, _| {
        assert_eq!(v.error, "Type a title for the new task.")
    });
    let (title, description) = view.read_with(vcx, |v, _| {
        (v.inputs.new_title.clone(), v.inputs.new_description.clone())
    });
    vcx.update(|_, cx| {
        set(&title, "Ship the tasks window", cx);
        set_area(&description, "With every button", cx);
    });
    click(vcx, "tasks-n-add-start");
    assert!(calls(&b, "add").is_empty());
    click(vcx, &trigger_id(Drop::NAssign));
    click(vcx, &option_id(Drop::NAssign, "member:dev"));
    click(vcx, &trigger_id(Drop::NGroup));
    click(vcx, &option_id(Drop::NGroup, "nixa-gap"));
    view.read_with(vcx, |v, _| assert_eq!(v.draft.space, "s2"));
    click(vcx, "tasks-n-add-start");
    assert_eq!(
        last(&b, "add"),
        json!({"title": "Ship the tasks window", "description": "With every button", "group": "nixa-gap", "space": "s2", "assign": "member:dev"})
    );
    let new_id = last(&b, "start")["id"].as_i64().unwrap();
    view.read_with(vcx, |v, _| {
        assert_eq!(v.panel, Some(Panel::Detail));
        assert_eq!(v.selected, Some(new_id));
        assert_eq!(
            v.tasks
                .iter()
                .find(|t| t.id == new_id)
                .map(|t| t.status.as_str()),
            Some("doing")
        );
    });
}

#[gpui::test]
fn spaces_and_live_updates(cx: &mut TestAppContext) {
    let b = brain();
    let (view, vcx) = open(cx, &b);
    {
        assert!(!drawn(vcx, "tasks-pull"));
        assert!(!drawn(vcx, "tasks-pull-line"));
    }
    click(vcx, "tasks-spaces");
    assert_eq!(
        SETTINGS.lock().unwrap().last().cloned(),
        Some("Spaces".into())
    );
    let before = calls(&b, "list").len();
    b.tasks
        .lock()
        .unwrap()
        .push(json!({"id": 50, "title": "From an agent", "status": "todo", "created_at": 1}));
    vcx.update(|_, cx| {
        let entity = view.clone();
        entity.update(cx, |v, cx| {
            v.changed(&json!({"type": "tasks_changed", "waiting": 4}), cx)
        });
    });
    view.read_with(vcx, |v, _| assert_eq!(v.waiting, 4));
    settle(vcx);
    assert!(calls(&b, "list").len() > before);
    assert!(drawn(vcx, "tasks-row-50"));
}

#[gpui::test]
fn apply_reaches_open_windows(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let b = brain();
    let call = fake(&b);
    let view = cx.update(|cx| super::new(call, host(), cx));
    cx.run_until_parked();
    let before = calls(&b, "list").len();
    let reached = cx.update(|cx| super::apply(&json!({"type": "tasks_changed"}), cx));
    assert_eq!(reached, 1);
    assert_eq!(
        cx.update(|cx| super::apply(&json!({"type": "chat"}), cx)),
        0
    );
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(calls(&b, "list").len() > before);
    cx.update(|cx| super::apply(&json!({"type": "workflows_changed"}), cx));
    cx.run_until_parked();
    assert!(!calls(&b, "options").is_empty());
    drop(view);
}

#[gpui::test]
fn offline_shows_and_retries(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let up = Arc::new(Mutex::new(false));
    let b = brain();
    let inner = fake(&b);
    let gate = up.clone();
    let call: Call = Arc::new(move |action, args| {
        if *gate.lock().unwrap() {
            inner(action, args)
        } else {
            Err(model::NOT_CONNECTED.to_string())
        }
    });
    let (view, vcx) = cx.add_window_view(move |_, cx| TasksView::new(call, host(), 0, cx));
    vcx.run_until_parked();
    assert!(drawn(vcx, "tasks-loading"));
    view.read_with(vcx, |v, _| assert!(!v.connected && !v.loaded));
    *up.lock().unwrap() = true;
    vcx.executor().advance_clock(Duration::from_secs(4));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| assert!(v.connected && v.loaded));
    assert!(drawn(vcx, "tasks-row-1"));
}

#[gpui::test]
fn escape_closes_in_steps(cx: &mut TestAppContext) {
    let b = brain();
    let (view, vcx) = open(cx, &b);
    click(vcx, "tasks-row-1");
    click(vcx, &trigger_id(Drop::DStatus));
    view.read_with(vcx, |v, _| assert_eq!(v.open, Some(Drop::DStatus)));
    let focus = view.read_with(vcx, gpui::Focusable::focus_handle);
    vcx.update(|w, _| w.focus(&focus));
    vcx.simulate_keystrokes("escape");
    view.read_with(vcx, |v, _| assert!(v.open.is_none() && v.panel.is_some()));
    vcx.simulate_keystrokes("escape");
    view.read_with(vcx, |v, _| assert!(v.panel.is_none()));
    vcx.simulate_keystrokes("escape");
    assert!(drawn(vcx, "tasks-row-1"));
    let search = view.read_with(vcx, |v, _| v.inputs.search.clone());
    vcx.update(|window, cx| window.focus(&gpui::Focusable::focus_handle(&search, cx)));
    vcx.simulate_keystrokes("escape");
    assert!(drawn(vcx, "tasks-row-1"));
}
