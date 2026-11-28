use super::card::{Card, CardKind, CardOption, CardState, Snapshot};
use super::view::{QuestionEvent, QuestionView};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

fn card(kind: CardKind, labels: &[&str], multi: bool) -> Card {
    Card {
        id: "p4".into(),
        kind,
        agent: "claude".into(),
        terminal: "m9".into(),
        terminal_name: None,
        place: Some("shop".into()),
        title: "Which database should I use?".into(),
        header: None,
        detail: None,
        options: labels
            .iter()
            .map(|l| CardOption {
                label: l.to_string(),
                description: None,
            })
            .collect(),
        multi,
        step: 1,
        steps: 1,
        state: CardState::Waiting,
        unread: false,
        auto: false,
    }
}

type Seen = Rc<RefCell<Vec<QuestionEvent>>>;

fn open(
    cx: &mut TestAppContext,
    snap: Snapshot,
) -> (Entity<QuestionView>, &mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let (view, cx) = cx.add_window_view(|_, _| QuestionView::empty());
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|_, app| {
        app.subscribe(&view, move |_, event: &QuestionEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
    });
    view.update(cx, |v, cx| v.set_snapshot(snap, cx));
    cx.run_until_parked();
    (view, cx, seen)
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

#[gpui::test]
fn one_click_sends_one_option(cx: &mut TestAppContext) {
    let snap = Snapshot {
        card: Some(card(CardKind::Question, &["SQLite", "Postgres"], false)),
        waiting: 2,
        ..Snapshot::default()
    };
    let (_view, cx, seen) = open(cx, snap);
    click(cx, "question-option-1");
    assert_eq!(
        *seen.borrow(),
        vec![QuestionEvent::Answer {
            id: "p4".into(),
            indexes: vec![1]
        }]
    );
    assert_eq!(
        seen.borrow()[0].message().unwrap(),
        json!({"type": "answer", "id": "p4", "indexes": [1]})
    );
}

#[gpui::test]
fn multi_pick_sends_the_picked(cx: &mut TestAppContext) {
    let snap = Snapshot {
        card: Some(card(CardKind::Question, &["Tests", "Docs", "Lint"], true)),
        ..Snapshot::default()
    };
    let (view, cx, seen) = open(cx, snap);
    click(cx, "question-send");
    assert!(seen.borrow().is_empty());
    click(cx, "question-option-2");
    click(cx, "question-option-0");
    click(cx, "question-option-1");
    click(cx, "question-option-1");
    assert!(seen.borrow().is_empty());
    view.read_with(cx, |v, _| assert_eq!(v.picked, vec![0, 2]));
    click(cx, "question-send");
    assert_eq!(
        *seen.borrow(),
        vec![QuestionEvent::Answer {
            id: "p4".into(),
            indexes: vec![0, 2]
        }]
    );
}

#[gpui::test]
fn sending_blocks_clicks(cx: &mut TestAppContext) {
    let mut c = card(CardKind::Permission, &["Yes", "Yes, always", "No"], false);
    c.state = CardState::Sending;
    let (_view, cx, seen) = open(
        cx,
        Snapshot {
            card: Some(c),
            ..Snapshot::default()
        },
    );
    click(cx, "question-option-0");
    assert!(seen.borrow().is_empty());
}

#[gpui::test]
fn go_to_agent_and_collapse(cx: &mut TestAppContext) {
    let snap = Snapshot {
        card: Some(card(CardKind::Plan, &["Approve", "Reject"], false)),
        ..Snapshot::default()
    };
    let (view, cx, seen) = open(cx, snap);
    click(cx, "question-go");
    click(cx, "question-agent");
    assert_eq!(
        *seen.borrow(),
        vec![
            QuestionEvent::GoTo {
                terminal: "m9".into()
            },
            QuestionEvent::GoTo {
                terminal: "m9".into()
            }
        ]
    );
    assert!(seen.borrow()[0].message().is_none());
    click(cx, "question-hide");
    view.read_with(cx, |v, _| {
        assert_eq!(v.collapsed_for.as_deref(), Some("p4"))
    });
    assert!(shown(cx, "question-open"));
    click(cx, "question-open");
    view.read_with(cx, |v, _| assert!(v.collapsed_for.is_none()));
    click(cx, "question-option-0");
    assert_eq!(
        seen.borrow().last(),
        Some(&QuestionEvent::Answer {
            id: "p4".into(),
            indexes: vec![0]
        })
    );
}

#[gpui::test]
fn reads_only_prompt_snapshots(cx: &mut TestAppContext) {
    let (view, cx, seen) = open(cx, Snapshot::default());
    view.update(cx, |v, cx| {
        assert!(v.apply(
            &json!({"type": "prompts", "card": null, "waiting": 0, "mode": "ask"}),
            cx
        ));
        assert!(!v.apply(&json!({"type": "chat"}), cx));
    });
    assert!(seen.borrow().is_empty());
}

#[gpui::test]
fn a_new_card_clears_the_picks(cx: &mut TestAppContext) {
    let snap = Snapshot {
        card: Some(card(CardKind::Question, &["a", "b"], true)),
        ..Snapshot::default()
    };
    let (view, cx, _seen) = open(cx, snap);
    click(cx, "question-option-1");
    view.read_with(cx, |v, _| assert_eq!(v.picked, vec![1]));
    let mut next = card(CardKind::Question, &["a", "b"], true);
    next.id = "p5".into();
    view.update(cx, |v, cx| {
        v.set_snapshot(
            Snapshot {
                card: Some(next),
                ..Snapshot::default()
            },
            cx,
        )
    });
    view.read_with(cx, |v, _| assert!(v.picked.is_empty()));
}
