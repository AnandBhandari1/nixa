use super::{Form, Item, Shown, frequency, groups, payload, snapshot, status};
use serde_json::json;

fn form() -> Form {
    Form { id: None, unit: "minutes".into(), enabled: true, error: None }
}

#[test]
fn payload_needs_text_and_a_whole_number() {
    assert!(payload(&form(), "  ", "15").is_err());
    assert!(payload(&form(), "hi", "x").is_err());
    assert!(payload(&form(), "hi", "0").is_err());
    let v = payload(&form(), " check tasks ", " 15 ").unwrap();
    assert_eq!(v, json!({"text": "check tasks", "every": 15, "unit": "minutes", "enabled": true}));
}

#[test]
fn payload_keeps_the_id_when_editing() {
    let f = Form { id: Some("rep-1".into()), unit: "hours".into(), enabled: false, error: None };
    let v = payload(&f, "hi", "2").unwrap();
    assert_eq!(v["id"], "rep-1");
    assert_eq!(v["unit"], "hours");
    assert_eq!(v["enabled"], false);
}

#[test]
fn snapshot_needs_the_repeats_list() {
    assert!(snapshot(&json!({"schedules": []})).is_none());
    let snap = snapshot(&json!({"repeats": [{"id": "rep-1", "text": "hi", "every": 5, "unit": "minutes", "enabled": true, "sent": 2}]})).unwrap();
    assert_eq!(snap.repeats.len(), 1);
    assert_eq!(snap.repeats[0].sent, 2);
}

#[test]
fn status_shows_next_and_sent_count() {
    let mut item = Item { enabled: true, sent: 3, view: Shown { every: "Every 15 minutes".into(), next: "Mon 10:30".into(), last: String::new() }, ..Item::default() };
    assert_eq!(status(&item), "Every 15 minutes · next Mon 10:30 · sent 3 times");
    item.enabled = false;
    item.sent = 0;
    assert_eq!(status(&item), "Every 15 minutes · paused · not sent yet");
    item.refused = 2;
    assert!(status(&item).ends_with("2 refused"));
}

#[test]
fn equivalent_intervals_share_a_group_sorted_by_next_delivery() {
    let items = [
        Item { id: "paused".into(), every: 1, unit: "hours".into(), ..Item::default() },
        Item { id: "hourly".into(), every: 60, unit: "minutes".into(), next_at: Some(10), ..Item::default() },
        Item { id: "short".into(), every: 15, unit: "minutes".into(), ..Item::default() },
    ];
    let groups = groups(&items);
    assert_eq!(groups.keys().copied().collect::<Vec<_>>(), [15, 60]);
    assert_eq!(groups[&60].iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["hourly", "paused"]);
    assert_eq!(frequency(60), "Every hour");
}

#[test]
fn payload_refuses_more_than_seven_days() {
    let hours = Form { id: None, unit: "hours".into(), enabled: true, error: None };
    assert!(payload(&hours, "hi", "9223372036854775807").is_err());
    assert!(payload(&hours, "hi", "169").is_err());
    assert!(payload(&hours, "hi", "168").is_ok());
    assert!(payload(&form(), "hi", "10081").is_err());
    assert!(payload(&form(), "hi", "10080").is_ok());
}
