use super::{HOUR_MS, KEEP, MINUTE_MS, RepeatMessage, Unit, change, command, due, every_text, file_of, load, now, parse, put, read, record_delivery, text_of, tick, toggle};
use crate::app::{App, lock};
use crate::cli_brains::{Cmd, Handle, Kind};
use crate::privacy::order;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

fn fields(text: &str, every: i64, unit: Unit, on: bool) -> (String, i64, Unit, bool) {
    (text.to_string(), every, unit, on)
}

#[test]
fn an_open_confirmation_retries_only_in_automatic() {
    assert!(super::retry_after(super::CONFIRM_OPEN));
    assert!(!super::retry_after("Nixa is off. Nothing was sent."));
    assert!(super::force_send(0));
    assert!(!super::force_send(1));
    let told = super::sent_anyway_event("r1");
    assert!(told.starts_with("Event: The repeat message r1 waited"));
    assert!(told.contains("sent it anyway"));
}

#[test]
fn read_checks_text_and_interval() {
    assert!(read(&json!({"text": " ", "every": 5})).is_err());
    assert!(read(&json!({"text": "hi", "every": 0})).is_err());
    assert!(read(&json!({"text": "hi", "every": 5, "unit": "days"})).is_err());
    assert!(read(&json!({"text": "hi", "every": 200, "unit": "hours"})).is_err());
    let (text, every, unit, on) = read(&json!({"text": " hi ", "every": "15"})).unwrap();
    assert_eq!((text.as_str(), every, unit, on), ("hi", 15, Unit::Minutes, true));
}

#[test]
fn put_adds_and_edits_without_losing_the_count() {
    let mut list = Vec::new();
    let id = put(&mut list, None, fields("hi", 15, Unit::Minutes, true), 1000).unwrap();
    assert_eq!(list[0].next_at, Some(1000 + 15 * MINUTE_MS));
    list[0].sent = 4;
    put(&mut list, Some(&id), fields("hello", 2, Unit::Hours, true), 5000).unwrap();
    assert_eq!(list[0].text, "hello");
    assert_eq!(list[0].sent, 4);
    assert_eq!(list[0].next_at, Some(5000 + 2 * HOUR_MS));
    put(&mut list, Some(&id), fields("hello again", 2, Unit::Hours, true), 9000).unwrap();
    assert_eq!(list[0].next_at, Some(5000 + 2 * HOUR_MS));
    assert!(put(&mut list, Some("missing"), fields("x", 1, Unit::Minutes, true), 0).is_err());
}

#[test]
fn due_sends_once_and_plans_the_next() {
    let mut list = vec![RepeatMessage { id: "a".into(), text: "hi".into(), every: 10, enabled: true, next_at: Some(100), ..RepeatMessage::default() }];
    assert!(due(&mut list, 50).is_empty());
    assert_eq!(due(&mut list, 100).as_slice(), ["a"]);
    assert_eq!(list[0].sent, 0);
    record_delivery(&mut list[0], 100, None);
    assert_eq!(list[0].sent, 1);
    assert_eq!(list[0].last_at, Some(100));
    assert_eq!(list[0].next_at, Some(100 + 10 * MINUTE_MS));
    assert!(due(&mut list, 200).is_empty());
}

#[test]
fn overdue_attempt_is_once_and_preserves_cadence() {
    let mut list = vec![RepeatMessage { id: "a".into(), text: "hi".into(), every: 1, enabled: true, next_at: Some(0), ..RepeatMessage::default() }];
    assert_eq!(due(&mut list, 2 * MINUTE_MS + 10).as_slice(), ["a"]);
    record_delivery(&mut list[0], 2 * MINUTE_MS + 10, Some("Nixa is off"));
    assert_eq!(list[0].sent, 0);
    assert_eq!(list[0].refused, 1);
    assert_eq!(list[0].last_at, None);
    assert_eq!(list[0].next_at, Some(3 * MINUTE_MS));
    assert!(due(&mut list, 2 * MINUTE_MS + 11).is_empty());
    record_delivery(&mut list[0], 3 * MINUTE_MS, None);
    assert_eq!(list[0].last_note, None);
    assert_eq!((list[0].sent, list[0].refused), (1, 1));
}

#[test]
fn paused_messages_do_not_send() {
    let mut list = vec![RepeatMessage { id: "a".into(), text: "hi".into(), every: 1, enabled: true, next_at: Some(0), ..RepeatMessage::default() }];
    toggle(&mut list, "a", false, 5).unwrap();
    assert_eq!(list[0].next_at, None);
    assert!(due(&mut list, 10_000_000).is_empty());
    toggle(&mut list, "a", true, 5).unwrap();
    assert_eq!(list[0].next_at, Some(5 + MINUTE_MS));
}

#[tokio::test]
async fn send_now_refusal_is_persisted_without_changing_the_timer() {
    let dir = tempfile::tempdir().unwrap();
    let app = crate::app::App::open(dir.path()).unwrap();
    command(&app, "repeat.save", &json!({"repeat":{"text":"check progress","every":5}})).unwrap().unwrap();
    let first = &load(&file_of(&app))[0];
    std::fs::write(dir.path().join(crate::settings_api::OFF_FILE), "").unwrap();
    let result = command(&app, "repeat.send", &json!({"id":first.id})).unwrap();
    assert!(result.unwrap_err().to_string().contains("Nixa is off"));
    let after = &load(&file_of(&app))[0];
    assert_eq!((after.sent, after.refused, after.next_at), (0, 1, first.next_at));
    assert!(after.last_note.as_deref().unwrap().contains("Nixa is off"));
}

async fn delivery_update(app: &App, events: &mut broadcast::Receiver<Value>) -> RepeatMessage {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = load(&file_of(app)).remove(0);
            if row.sent + row.refused > 0 {
                return row;
            }
            events.recv().await.unwrap();
        }
    }).await.unwrap()
}

#[tokio::test]
async fn send_now_counts_after_the_cli_queue_accepts() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    app.save_settings(|s| s.choice = Some("codex".into())).unwrap();
    let (tx, mut queue) = mpsc::unbounded_channel();
    lock(&app.cli.live).run = Some(Handle { id: 1, kind: Kind::Codex, tx });

    command(&app, "repeat.save", &json!({"repeat": {"text": "check progress", "every": 5}})).unwrap().unwrap();
    let first = load(&file_of(&app)).remove(0);
    let ahead = order::take(&app.privacy.order);
    let mut events = app.out.subscribe();
    command(&app, "repeat.send", &json!({"id": first.id})).unwrap().unwrap();
    assert_eq!(load(&file_of(&app)), vec![first.clone()]);
    assert!(matches!(queue.try_recv(), Err(mpsc::error::TryRecvError::Empty)));

    drop(ahead);
    let Cmd::Tell(line) = tokio::time::timeout(Duration::from_secs(3), queue.recv()).await.unwrap().unwrap() else {
        panic!("the brain queue did not receive the repeat");
    };
    assert!(line.contains("Scheduled repeat: \"check progress\""));
    let after = delivery_update(&app, &mut events).await;
    assert_eq!((after.sent, after.refused, after.next_at), (1, 0, first.next_at));
    assert!(after.last_at.is_some());
    assert_eq!(after.last_note, None);
}

#[tokio::test]
async fn timer_counts_refusal_when_the_brain_disappears_before_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    app.save_settings(|s| s.choice = Some("codex".into())).unwrap();
    let first_due = now() - 2 * MINUTE_MS;
    change(&app, |list| {
        list.push(RepeatMessage {
            id: "first".into(),
            text: "check progress".into(),
            every: 5,
            enabled: true,
            next_at: Some(first_due),
            ..RepeatMessage::default()
        });
        Ok(())
    }).unwrap();
    let ahead = order::take(&app.privacy.order);
    let mut events = app.out.subscribe();
    tick(&app);
    let pending = load(&file_of(&app)).remove(0);
    assert_eq!((pending.sent, pending.refused, pending.last_at), (0, 0, None));
    assert_eq!(pending.next_at, Some(first_due + 5 * MINUTE_MS));

    app.save_settings(|s| s.choice = None).unwrap();
    drop(ahead);
    let after = delivery_update(&app, &mut events).await;
    assert_eq!((after.sent, after.refused, after.last_at), (0, 1, None));
    assert_eq!(after.next_at, pending.next_at);
    assert_eq!(after.last_note.as_deref(), Some("No chat brain is selected. Nothing was sent."));
    assert!(lock(&app.brain).queue.is_empty());
}

#[test]
fn the_file_round_trips() {
    let list = vec![RepeatMessage { id: "a".into(), text: "hi".into(), every: 3, unit: Unit::Hours, enabled: true, sent: 2, ..RepeatMessage::default() }];
    assert_eq!(parse(Some(&text_of(&list))), list);
    assert!(parse(Some("not json")).is_empty());
    assert_eq!(every_text(&list[0]), "Every 3 hours");
}

#[test]
fn huge_intervals_are_refused_without_overflow() {
    assert!(read(&json!({"text": "hi", "every": i64::MAX, "unit": "hours"})).is_err());
    assert!(read(&json!({"text": "hi", "every": "9223372036854775807", "unit": "minutes"})).is_err());
    assert!(read(&json!({"text": "hi", "every": 168, "unit": "hours"})).is_ok());
    let big = RepeatMessage { id: "a".into(), text: "hi".into(), every: i64::MAX, unit: Unit::Hours, enabled: true, ..RepeatMessage::default() };
    assert!(parse(Some(&text_of(std::slice::from_ref(&big)))).is_empty());
    assert_eq!(big.gap_ms(), 168 * HOUR_MS);
}

#[test]
fn keeps_at_most_fifty_but_loads_every_saved_one() {
    assert_eq!(KEEP, 50);
    let mut list = Vec::new();
    for n in 0..KEEP {
        put(&mut list, None, fields(&format!("note {n}"), 5, Unit::Minutes, true), n as i64).unwrap();
    }
    let error = put(&mut list, None, fields("one more", 5, Unit::Minutes, true), 99).unwrap_err().to_string();
    assert!(error.contains("up to 50"), "{error}");
    let mut more = list.clone();
    more.push(RepeatMessage { id: "extra".into(), text: "old".into(), every: 5, ..RepeatMessage::default() });
    assert_eq!(parse(Some(&text_of(&more))).len(), KEEP + 1);
}
