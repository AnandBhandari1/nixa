use super::fun;
use super::plan::{self, Plan};
use super::soap::Gate;
use super::talk::{self, Stage};
use super::{Room, Rooms, Tab, rooms_of};
use crate::screens::nixa_screen::map::{Agent, DEEP, NIXA, St};
use serde_json::json;

fn agent(id: &str, status: St, title: &str) -> Agent {
    Agent {
        id: id.into(),
        title: title.into(),
        program: "claude".into(),
        mono: "C".into(),
        status,
        parent: None,
        seq: 1,
        number: Some(3),
        persona: None,
    }
}

fn agents() -> Vec<Agent> {
    vec![
        agent("a1", St::Working, "Fix login page"),
        agent("a2", St::Idle, "Old job"),
        agent("a3", St::Done, "Older job"),
    ]
}

fn plan_now() -> Plan {
    let r = Rooms {
        list: vec![Room { id: "a".into(), name: "Space a".into(), color: 0, agents: agents(), deep: Vec::new() }],
        loose_deep: vec![agent("d1", St::Idle, "")],
        active: None,
    };
    plan::build(&rooms_of(&r, &Tab::Company), &r.loose_deep, "Nixa", false)
}

fn gate(on: bool, office: bool, idle: bool, talk: bool) -> Gate {
    Gate { office, open: true, on, up: true, idle, talk }
}

#[test]
fn fun_gate_needs_switch_and_office() {
    assert!(fun::on(gate(true, true, true, false)));
    assert!(!fun::on(gate(false, true, true, false)));
    assert!(!fun::on(gate(true, false, true, false)));
    assert!(!fun::on(Gate { open: false, ..gate(true, true, true, false) }));
    let mut f = fun::Fun::default();
    assert_eq!(fun::gate_msg(&mut f, gate(true, true, true, false), 1000), Some(json!({"on": true, "office": true})));
    assert_eq!(fun::gate_msg(&mut f, gate(true, true, false, true), 2000), None);
    assert_eq!(fun::gate_msg(&mut f, gate(false, true, true, false), 3000), Some(json!({"on": false, "office": true})));
    assert!(fun::gate_msg(&mut f, gate(false, true, true, false), 3000 + fun::GATE_EVERY_MS).is_some());
}

#[test]
fn real_talk_waits_for_the_rewrite_and_uses_it() {
    let p = plan_now();
    let list = agents();
    let mut stage = Stage::default();
    talk::push(&mut stage, talk::talk_of(DEEP, NIXA, None, None, &list, 1000));
    let asks = fun::asks(&mut stage, &p, &list, gate(true, true, false, false), 1000);
    assert_eq!(asks.len(), 1);
    let (id, body) = &asks[0];
    assert_eq!(body["first"], "Here is my advice. Go ahead.");
    assert_eq!(body["second"], "Thanks, that helps a lot.");
    assert_eq!((body["on"].clone(), body["office"].clone()), (json!(true), json!(true)));
    assert!(body["known"].as_array().unwrap().iter().any(|k| k == "Nixa"));
    assert!(fun::asks(&mut stage, &p, &list, gate(true, true, false, false), 1100).is_empty());
    talk::advance(&mut stage, &p, 1500);
    assert!(stage.now.is_none());
    let reply = json!({"type": fun::FUN_EVENT, "id": id, "first": "Advice is served. Go for it.", "second": "Lovely, thanks!"});
    assert!(fun::apply(&mut stage, &reply));
    assert!(!fun::apply(&mut stage, &json!({"id": 999, "first": "x"})));
    talk::advance(&mut stage, &p, 1600);
    let play = stage.now.clone().unwrap();
    assert_eq!(play.talk.first, "Advice is served. Go for it.");
    assert_eq!(play.talk.second, "Lovely, thanks!");
    let mut late = Stage::default();
    talk::push(&mut late, talk::talk_of(DEEP, NIXA, None, None, &list, 0));
    fun::asks(&mut late, &p, &list, gate(true, true, false, false), 0);
    talk::advance(&mut late, &p, talk::HOLD_MS - 1);
    assert!(late.now.is_none());
    talk::advance(&mut late, &p, talk::HOLD_MS);
    assert_eq!(late.now.unwrap().talk.first, "Here is my advice. Go ahead.");
    let mut down = Stage::default();
    talk::push(&mut down, talk::talk_of(DEEP, NIXA, None, None, &list, 0));
    let id = fun::asks(&mut down, &p, &list, gate(true, true, false, false), 0)[0].0;
    fun::unask(&mut down, id);
    talk::advance(&mut down, &p, 1);
    assert!(down.now.is_some());
}

#[test]
fn late_rewrite_updates_bubble_times() {
    let p = plan_now();
    let list = agents();
    let mut stage = Stage::default();
    talk::push(&mut stage, talk::talk_of(DEEP, NIXA, None, None, &list, 0));
    let id = fun::asks(&mut stage, &p, &list, gate(false, true, false, false), 0)[0].0;
    talk::advance(&mut stage, &p, talk::HOLD_MS);
    let before = stage.now.clone().unwrap();
    let long = "Here is my advice, written out with a lot more words so it takes longer.";
    assert!(fun::apply(&mut stage, &json!({"id": id, "first": long, "second": "Thanks a lot, that really helps me today."})));
    let after = stage.now.clone().unwrap();
    assert_eq!(after.say1, talk::say1_ms(long));
    assert!(after.say1 > before.say1 && after.say2 > before.say2);
    let mut said = after;
    said.said = 1;
    stage.now = Some(said.clone());
    assert!(fun::apply(&mut stage, &json!({"id": id, "first": "x y z", "second": "Ok."})));
    let kept = stage.now.unwrap();
    assert_eq!((kept.say1, kept.talk.first.as_str()), (said.say1, long));
    assert_eq!(kept.say2, talk::say2_ms("Ok."));
}

#[test]
fn banter_waits_a_random_gap_and_only_while_busy() {
    for seed in [0, 7, 12_345, 99_999_999] {
        let g = fun::gap(seed);
        assert!((fun::GAP_MIN..=fun::GAP_MAX).contains(&g), "{g}");
    }
    assert_ne!(fun::gap(1), fun::gap(2));
    let mut f = fun::Fun::default();
    let busy = gate(true, true, false, false);
    fun::schedule(&mut f, busy, 1000);
    let due = f.banter_at;
    assert!((1000 + fun::GAP_MIN..=1000 + fun::GAP_MAX).contains(&due));
    fun::schedule(&mut f, busy, 5000);
    assert_eq!(f.banter_at, due);
    assert!(!fun::banter_due(&f, busy, false, due - 1));
    assert!(fun::banter_due(&f, busy, false, due));
    assert!(!fun::banter_due(&f, gate(true, true, false, true), false, due));
    assert!(!fun::banter_due(&f, busy, true, due));
    assert!(!fun::banter_due(&f, gate(false, true, false, false), false, due));
    f.banter_asked = Some(due);
    assert!(!fun::banter_due(&f, busy, false, due + 1));
    fun::schedule(&mut f, gate(true, true, true, false), due);
    assert_eq!(f.banter_at, 0);
    assert!(!fun::banter_due(&f, gate(true, true, true, false), false, due));
}

#[test]
fn banter_picks_two_free_people_and_a_real_task() {
    let p = plan_now();
    for now in [0, 1000, 77_000, 1_234_000] {
        let body = fun::banter_pick(&p, &agents(), gate(true, true, false, false), now).unwrap();
        assert_eq!(body["task"], "Fix login page");
        let (who, to) = (body["who"].as_str().unwrap(), body["to"].as_str().unwrap());
        assert_ne!(who, to);
        assert!(["a2", "a3"].contains(&who) && ["a2", "a3"].contains(&to));
        assert!(!body["owner"].as_str().unwrap().is_empty());
    }
    let idle: Vec<Agent> = agents().into_iter().map(|a| Agent { status: St::Idle, ..a }).collect();
    let r = Rooms { list: vec![Room { id: "a".into(), name: "A".into(), color: 0, agents: idle.clone(), deep: Vec::new() }], loose_deep: Vec::new(), active: None };
    let quiet = plan::build(&rooms_of(&r, &Tab::Company), &[], "Nixa", false);
    assert!(fun::banter_pick(&quiet, &idle, gate(true, true, false, false), 0).is_none());
    let msg = json!({"type": fun::BANTER_EVENT, "who": "a2", "to": "a3", "first": "Ravi is wrestling the login page.", "second": "Brave."});
    let t = fun::banter_talk(&msg, &p, 5).unwrap();
    assert!(t.banter && t.ready && t.given);
    assert!(fun::banter_talk(&json!({"who": "a1", "to": "a3", "first": "x", "second": "y"}), &p, 5).is_none());
    let mut stage = Stage::default();
    talk::push(&mut stage, t);
    assert!(fun::asks(&mut stage, &p, &agents(), gate(true, true, false, false), 5).is_empty());
    fun::drop_banter(&mut stage);
    assert!(stage.queue.is_empty());
}
