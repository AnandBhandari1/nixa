use super::galaxy::{fit, random, stars};
use super::input::{split, st};
use super::map::{
    self, Agent, Burst, Flow, St, agent_events, agent_flows, deep_flow, layout, pick_shown,
};
use super::mode::{Feed, Mode, Tone, ease, energy_of, mode_of, next_glow, shown_of};
use super::orb::{lit, voice_amp};
use super::palette::{aura_color, palette_of, pink};
use super::prefs::{self, Look, Prefs};
use super::view::{NixaScreen, ScreenEvent, agent_tip};
use gpui::prelude::*;
use gpui::{
    Context, Entity, IntoElement, Modifiers, Render, TestAppContext, VisualTestContext, Window, div,
};
use nixa_protocol::{Status, TermId, TermInfo};
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Instant;

fn agent(id: &str, status: St, parent: Option<&str>) -> Agent {
    Agent {
        id: id.into(),
        title: format!("claude {id}"),
        program: "claude".into(),
        mono: "C".into(),
        status,
        parent: parent.map(str::to_string),
        seq: id.trim_start_matches('m').parse().unwrap_or(0),
        number: None,
        persona: None,
    }
}

fn up() -> Feed {
    Feed {
        up: true,
        ..Feed::default()
    }
}

#[test]
fn modes_follow_the_brain() {
    assert_eq!(mode_of(&Feed::default(), false), Mode::Wait);
    assert_eq!(
        mode_of(
            &Feed {
                off: true,
                ..Feed::default()
            },
            false
        ),
        Mode::Off
    );
    assert_eq!(mode_of(&up(), true), Mode::You);
    assert_eq!(
        mode_of(
            &Feed {
                state: "speaking".into(),
                ..up()
            },
            false
        ),
        Mode::Nixa
    );
    assert_eq!(
        mode_of(
            &Feed {
                state: "thinking".into(),
                ..up()
            },
            false
        ),
        Mode::Think
    );
    assert_eq!(mode_of(&Feed { busy: true, ..up() }, false), Mode::Think);
    assert_eq!(
        mode_of(
            &Feed {
                state: "paused".into(),
                ..up()
            },
            false
        ),
        Mode::Paused
    );
    assert_eq!(
        mode_of(
            &Feed {
                state: "off".into(),
                ..up()
            },
            false
        ),
        Mode::Off
    );
    assert_eq!(
        mode_of(
            &Feed {
                mic_off: true,
                ..up()
            },
            false
        ),
        Mode::MicOff
    );
    assert_eq!(
        mode_of(
            &Feed {
                hearing: true,
                ..up()
            },
            false
        ),
        Mode::Listen
    );
    assert_eq!(mode_of(&up(), false), Mode::Standby);
    assert_eq!(Mode::Standby.word(), "STANDBY");
    assert_eq!(Mode::MicOff.word(), "MIC OFF");
    assert_eq!(Mode::Wait.word(), "CONNECTING");
}

#[test]
fn deep_work_shows_as_thinking_in_gold() {
    assert_eq!(shown_of(Mode::Standby, true), (Mode::Think, Tone::Deep));
    assert_eq!(shown_of(Mode::Listen, true), (Mode::Think, Tone::Deep));
    assert_eq!(
        shown_of(Mode::Nixa, true),
        (Mode::Nixa, Tone::Mode(Mode::Nixa))
    );
    assert_eq!(
        shown_of(Mode::Off, true),
        (Mode::Off, Tone::Mode(Mode::Off))
    );
    assert_eq!(
        shown_of(Mode::Standby, false),
        (Mode::Standby, Tone::Mode(Mode::Standby))
    );
}

#[test]
fn energy_glow_and_easing() {
    assert_eq!(energy_of(Mode::Standby, 1.0, 1.0), 0.0);
    assert_eq!(energy_of(Mode::You, 0.0, 2.0), 1.0);
    let e = energy_of(Mode::Nixa, 0.3, 0.0);
    assert!((0.0..=1.0).contains(&e));
    let g = next_glow(0.5, Mode::Nixa, 0.1);
    assert!(g > 0.5 && g < 0.9);
    assert_eq!(next_glow(0.5, Mode::Nixa, 1.0), 0.9);
    assert_eq!(ease(0.0, 1.0), 0.5);
    assert_eq!(ease(0.004, 0.0), 0.0);
}

#[test]
fn palette_uses_theme_colours() {
    let t = nixa_ui::theme::default_theme();
    let (p, rose) = pink(&t);
    assert!((p.h * 360.0 - 330.0).abs() < 0.5);
    assert!((rose.h * 360.0 - 345.0).abs() < 0.5);
    assert!(rose.l > p.l);
    let deep = palette_of(Tone::Deep, &t);
    assert!((deep.main.h * 360.0 - 46.0).abs() < 0.5);
    let off = palette_of(Tone::Mode(Mode::Off), &t);
    assert_eq!(off.main, t.dim);
    assert_eq!(
        palette_of(Tone::Mode(Mode::MicOff), &t),
        palette_of(Tone::Mode(Mode::Standby), &t)
    );
    assert_eq!(aura_color("m1"), aura_color("m1"));
    assert_ne!(aura_color("m1"), aura_color("m2"));
}

#[test]
fn galaxy_is_fixed_and_fills_the_window() {
    let sky = stars();
    assert_eq!(
        sky.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![260, 90, 26]
    );
    assert_eq!(sky[0][0], stars()[0][0]);
    assert!(
        sky.iter()
            .flatten()
            .all(|s| s.x >= 0.0 && s.x <= 1600.0 && s.y >= 0.0 && s.y <= 1000.0)
    );
    let r = random(3, 10);
    assert!((0.0..1.0).contains(&r));
    let f = fit(800.0, 1000.0);
    assert_eq!(f.scale, 1.0);
    assert_eq!(f.left, -400.0);
    assert_eq!(f.top, 0.0);
}

#[test]
fn core_bars_and_meter() {
    assert_eq!(voice_amp(Mode::Standby, 3, 1.0, 1.0), 0.0);
    assert!(voice_amp(Mode::Think, 3, 1.0, 0.0) >= 0.12);
    assert!(voice_amp(Mode::You, 3, 1.0, 1.0) > 0.0);
    assert_eq!(voice_amp(Mode::You, 3, 1.0, 0.0), 0.0);
    assert_eq!(lit(0.0), 0);
    assert_eq!(lit(1.0), 20);
    assert_eq!(lit(7.0), 20);
}

#[test]
fn layout_places_core_agents_kids_and_brains() {
    let agents = vec![
        agent("m1", St::Working, None),
        agent("m2", St::Blocked, None),
        agent("m3", St::Done, Some("m1")),
        agent("m4", St::Idle, Some("m9")),
        agent("m5", St::Idle, Some("d1")),
    ];
    let deep = vec![Agent {
        title: "Deep brain · codex".into(),
        ..agent("d1", St::Working, None)
    }];
    let l = layout(1200.0, 800.0, &agents, &deep);
    assert_eq!(l.nixa.x, 600.0);
    assert_eq!(l.nixa.y, 448.0);
    assert_eq!(l.nixa.size, 272.0);
    assert_eq!(l.brains.len(), 1);
    assert_eq!(l.brains[0].kind, map::Kind::Codex);
    assert!(l.brains[0].spot.y < l.nixa.y);
    assert_eq!(l.agents.len(), 5);
    for p in &l.agents {
        assert!(
            p.x >= 0.0 && p.x <= 1200.0 && p.y >= 0.0 && p.y <= 800.0,
            "{p:?}"
        );
    }
    let kid = l.agents.iter().find(|p| p.agent.id == "m3").unwrap();
    assert_eq!(kid.parent.as_deref(), Some("m1"));
    assert_eq!(kid.radius, map::KID_RADIUS);
    let lost = l.agents.iter().find(|p| p.agent.id == "m4").unwrap();
    assert_eq!(lost.parent, None);
    let under_deep = l.agents.iter().find(|p| p.agent.id == "m5").unwrap();
    assert_eq!(under_deep.parent.as_deref(), Some("d1"));
    assert!(map::node_of(&l, map::DEEP).is_some());
    assert!(map::endpoints(&l, map::NIXA, "m2").is_some());
    assert!(map::endpoints(&l, map::NIXA, "nobody").is_none());
}

#[test]
fn layout_survives_a_parent_loop() {
    let agents = vec![
        agent("m1", St::Idle, Some("m2")),
        agent("m2", St::Idle, Some("m1")),
    ];
    let l = layout(900.0, 600.0, &agents, &[]);
    assert_eq!(l.agents.len(), 2);
    assert!(l.agents.iter().all(|p| p.parent.is_none()));
    let empty = layout(900.0, 600.0, &[], &[]);
    assert!(empty.agents.is_empty() && empty.brains.is_empty());
}

#[test]
fn many_agents_keep_the_needy_and_their_parents() {
    let mut list: Vec<Agent> = (1..=30)
        .map(|i| agent(&format!("m{i}"), St::Idle, None))
        .collect();
    list.push(agent("m40", St::Blocked, Some("m30")));
    let shown = pick_shown(&list);
    assert!(shown.len() <= 25);
    assert!(shown.iter().any(|a| a.id == "m40"));
    assert!(shown.iter().any(|a| a.id == "m30"));
    assert!(shown.windows(2).all(|w| w[0].seq <= w[1].seq));
}

#[test]
fn flows_and_bursts_follow_status_changes() {
    let before: BTreeMap<String, St> = [
        ("m1".to_string(), St::Idle),
        ("m2".to_string(), St::Working),
    ]
    .into();
    let now_list = vec![
        agent("m1", St::Working, None),
        agent("m2", St::Done, None),
        agent("m3", St::Idle, Some("m1")),
    ];
    let (seen, flows) = agent_flows(Some(&before), &now_list, 1000, 0);
    assert_eq!(seen.len(), 3);
    assert_eq!(
        flows,
        vec![
            Flow {
                from: "nixa".into(),
                to: "m1".into(),
                at: 1000
            },
            Flow {
                from: "m2".into(),
                to: "nixa".into(),
                at: 1000
            },
            Flow {
                from: "m1".into(),
                to: "m3".into(),
                at: 1000
            },
        ]
    );
    let (_, from_deep) = agent_flows(Some(&before), &now_list[..1], 1000, 900);
    assert_eq!(from_deep[0].from, "deep");
    assert!(agent_flows(None, &now_list, 5, 0).1.is_empty());
    let bursts = agent_events(Some(&before), &now_list, 7);
    assert_eq!(bursts.len(), 2);
    assert!(bursts.iter().any(|e| e.kind == Burst::Nova && e.id == "m2"));
    assert!(bursts.iter().any(|e| e.kind == Burst::Warp && e.id == "m3"));
    assert_eq!(deep_flow(None, true, 1), None);
    assert_eq!(deep_flow(Some(false), true, 1).unwrap().to, "deep");
    assert_eq!(deep_flow(Some(true), false, 1).unwrap().from, "deep");
    assert_eq!(deep_flow(Some(true), true, 1), None);
    let mut live = vec![
        Flow {
            from: "a".into(),
            to: "b".into(),
            at: 0,
        },
        Flow {
            from: "a".into(),
            to: "b".into(),
            at: 1500,
        },
    ];
    map::live_flows(&mut live, 2100);
    assert_eq!(live.len(), 1);
}

#[test]
fn agent_words() {
    let mut a = agent("m7", St::Blocked, Some("m2"));
    assert_eq!(a.name(), "agent m7");
    a.number = Some(3);
    a.title = "claude · fix #12 login".into();
    assert_eq!(a.label(), "3 · claude");
    a.title = "Fix #12 login · claude".into();
    assert_eq!(a.task().as_deref(), Some("12"));
    assert_eq!(
        agent_tip(&a, false),
        "agent 3 · blocked · parent m2 unavailable"
    );
    assert_eq!(agent_tip(&a, true), "agent 3 · blocked · parent m2");
    a.title.clear();
    assert_eq!(a.label(), "agent 3");
}

fn term(
    id: &str,
    cmd: &[&str],
    status: Option<Status>,
    parent: Option<&str>,
    name: &str,
) -> TermInfo {
    serde_json::from_value(json!({
        "id": id, "name": name, "cmd": cmd, "cwd": "/tmp", "running": true,
        "status": status, "parent": parent, "rows": 24, "cols": 80, "created_ms": 1,
        "seq": id.trim_start_matches('m').parse::<u64>().unwrap_or(0)
    }))
    .unwrap()
}

#[test]
fn terminals_split_into_agents_and_deep_brains() {
    let list = [
        term("m1", &["claude"], Some(Status::Working), None, "builder"),
        term("m2", &["zsh"], None, None, "shell"),
        term("m3", &["zsh"], None, None, "holder"),
        term("m4", &["codex"], Some(Status::Error), Some("m3"), "kid"),
        term("m5", &["codex"], None, None, "deep brain"),
        term("m6", &["claude"], None, None, "far idle"),
    ];
    let terms: BTreeMap<TermId, TermInfo> = list.into_iter().map(|t| (t.id.clone(), t)).collect();
    let id = |s: &str| s.parse::<TermId>().unwrap();
    let here: BTreeSet<TermId> = [id("m1"), id("m2"), id("m5")].into();
    let placed: BTreeSet<TermId> = [id("m1"), id("m2"), id("m5"), id("m6")].into();
    let numbers: BTreeMap<TermId, u32> = [(id("m1"), 1)].into();
    let s = split(&terms, &here, &placed, &numbers);
    let ids: Vec<&str> = s.agents.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, vec!["m1", "m3", "m4"]);
    assert_eq!(s.agents[0].number, Some(1));
    assert_eq!(s.agents[2].status, St::Blocked);
    assert_eq!(s.deep.len(), 1);
    assert_eq!(s.deep[0].id, "m5");
    assert_eq!(st(None), St::Idle);
}

#[test]
fn deep_brains_are_found_by_their_role_label() {
    let mut labelled = term("m7", &["codex"], None, None, "space helper");
    labelled.labels.insert("nixa.role".into(), "deep-brain".into());
    let mut named = term("m8", &["claude"], None, None, "deep brain look-alike");
    named.labels.insert("nixa.role".into(), "helper".into());
    let list = [labelled, named, term("m9", &["claude"], None, None, "deep brain")];
    let terms: BTreeMap<TermId, TermInfo> = list.into_iter().map(|t| (t.id.clone(), t)).collect();
    let here: BTreeSet<TermId> = terms.keys().cloned().collect();
    let s = split(&terms, &here, &here, &BTreeMap::new());
    let deep: Vec<&str> = s.deep.iter().map(|a| a.id.as_str()).collect();
    let agents: Vec<&str> = s.agents.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(deep, vec!["m7", "m9"]);
    assert_eq!(agents, vec!["m8"]);
}

#[test]
fn prefs_are_saved_in_the_store() {
    let conn = nixa_store::open_memory().unwrap();
    assert_eq!(prefs::load(Some(&conn)), Prefs::default());
    prefs::save(
        Some(&conn),
        Prefs {
            keep_open: true,
            quiet: true,
            look: Look::Office,
            still: true,
            mute: true,
            soap: true,
        },
    )
    .unwrap();
    assert_eq!(
        prefs::load(Some(&conn)),
        Prefs {
            keep_open: true,
            quiet: true,
            look: Look::Office,
            still: true,
            mute: true,
            soap: true,
        }
    );
    assert!(prefs::save(None, Prefs::default()).is_err());
    assert_eq!(prefs::load(None), Prefs::default());
}

#[derive(Clone, Copy, PartialEq)]
enum Shown {
    Screen,
    Part,
}

struct Host {
    screen: Entity<NixaScreen>,
    part: Shown,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().size_full().relative();
        match self.part {
            Shown::Part => root.child(crate::screens::settings::parts::nixa_screen::render(cx)),
            Shown::Screen => root.child(self.screen.clone()),
        }
    }
}

type Seen = Rc<RefCell<Vec<ScreenEvent>>>;

fn open(
    cx: &mut TestAppContext,
    part: Shown,
) -> (&mut VisualTestContext, Entity<NixaScreen>, Seen) {
    cx.update(nixa_ui::init);
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let screen = cx.update(|cx| {
        let conn = nixa_store::open_memory().ok();
        let screen = cx.new(|cx| NixaScreen::build(None, conn, cx));
        cx.set_global(super::Shared(screen.clone()));
        cx.subscribe(&screen, move |_, e: &ScreenEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach();
        screen
    });
    let held = screen.clone();
    let (_, vcx) = cx.add_window_view(move |_, _| Host { screen: held, part });
    screen.update(vcx, |s, cx| {
        s.set_feed(Feed {
            up: true,
            ..Feed::default()
        });
        s.set_agents(
            vec![
                agent("m1", St::Working, None),
                agent("m2", St::Idle, Some("m1")),
            ],
            vec![Agent {
                title: "Deep brain".into(),
                ..agent("m9", St::Idle, None)
            }],
            1,
        );
        s.show(cx);
    });
    (vcx, screen, seen)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, app| window.draw(app).clear());
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    redraw(cx);
    let b = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(b.center(), Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn screen_draws_agents_and_buttons_work(cx: &mut TestAppContext) {
    let (cx, screen, seen) = open(cx, Shown::Screen);
    redraw(cx);
    for id in [
        "nixa-screen",
        "nixa-core",
        "nixa-screen-gear",
        "nixa-screen-close",
        "universe-agent-m1",
        "universe-agent-m2",
        "nixa-deep-m9",
    ] {
        assert!(cx.debug_bounds(id).is_some(), "{id} is missing");
    }
    click(cx, "nixa-screen-gear");
    click(cx, "nixa-screen-close");
    assert_eq!(
        *seen.borrow(),
        vec![ScreenEvent::Settings, ScreenEvent::Close]
    );
    screen.update(cx, |s, cx| {
        s.set_feed(Feed {
            up: false,
            off: true,
            ..Feed::default()
        });
        cx.notify();
    });
    click(cx, "nixa-core");
    assert_eq!(seen.borrow().last(), Some(&ScreenEvent::TurnNixaOn));
}

#[gpui::test]
fn core_click_stops_and_turns_the_mic_on(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    screen.update(cx, |s, cx| {
        s.set_feed(Feed {
            up: true,
            busy: true,
            ..Feed::default()
        });
        cx.notify();
    });
    click(cx, "nixa-core");
    screen.update(cx, |s, cx| {
        s.set_feed(Feed {
            up: true,
            mic_off: true,
            ..Feed::default()
        });
        cx.notify();
    });
    click(cx, "nixa-core");
    screen.update(cx, |s, cx| {
        s.set_feed(Feed {
            up: true,
            ..Feed::default()
        });
        cx.notify();
    });
    click(cx, "nixa-core");
    let sent = screen.read_with(cx, |s, _| s.sent.clone());
    assert_eq!(
        sent,
        vec![
            json!({"type": "stop"}),
            json!({"type": "set_mic", "on": true})
        ]
    );
    screen.update(cx, |s, cx| {
        s.set_feed(Feed {
            up: true,
            state: "off".into(),
            ..Feed::default()
        });
        cx.notify();
    });
    click(cx, "nixa-core");
    let sent = screen.read_with(cx, |s, _| s.sent.clone());
    assert_eq!(sent.last(), Some(&json!({"type": "resume"})));
}

#[gpui::test]
fn escape_closes_unless_kept_open(cx: &mut TestAppContext) {
    let (cx, screen, seen) = open(cx, Shown::Screen);
    redraw(cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(*seen.borrow(), vec![ScreenEvent::Close]);
    screen.update(cx, |s, cx| s.set_keep(true, cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(seen.borrow().len(), 1);
}

#[gpui::test]
fn frames_run_with_level_flows_and_bursts(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    screen.update(cx, |s, _| {
        let now = Instant::now();
        let wall = super::now_ms();
        assert!(!s.on_message(
            &json!({"type": "level", "mic": 0.8, "talking": true}),
            now,
            wall
        ));
        assert!(s.on_message(
            &json!({"type": "universe_flow", "from": "nixa", "to": "m1", "at": wall}),
            now,
            wall
        ));
        assert!(!s.on_message(
            &json!({"type": "universe_flow", "from": "", "to": "m1", "at": wall}),
            now,
            wall
        ));
        assert_eq!(s.real_mode(now), Mode::You);
        assert!(s.step(now, wall));
        assert!(s.live.level > 0.3);
        assert_eq!(s.live.flows.len(), 1);
        assert_eq!(
            s.sent,
            vec![json!({"type": "universe_meteor", "direction": "outgoing", "at": wall})]
        );
        assert!(s.on_message(
            &json!({"type": "flow", "from": "core", "to": "deep", "kind": "ask"}),
            now,
            wall
        ));
        assert!(!s.on_message(&json!({"type": "flow", "from": "m1", "kind": "send"}), now, wall));
        assert_eq!(s.live.flows.len(), 2);

        s.set_agents(
            vec![agent("m1", St::Done, None), agent("m3", St::Idle, None)],
            vec![],
            wall,
        );
        assert_eq!(s.live.events.len(), 2);
    });
    redraw(cx);
    screen.update(cx, |s, cx| {
        s.set_quiet(true, cx);
        assert_eq!(s.time(), 0.0);
    });
    redraw(cx);
}

#[gpui::test]
fn idle_screen_skips_frames(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    screen.update(cx, |s, _| {
        s.set_agents(vec![agent("m1", St::Idle, None)], vec![], 1);
        s.set_agents(vec![agent("m1", St::Idle, None)], vec![], 10_000);
        let now = Instant::now();
        let wall = super::now_ms();
        s.live.flows.clear();
        s.live.events.clear();
        assert!(s.step(now, wall));
        assert!(!s.step(now, wall));
    });
}

#[gpui::test]
fn settings_part_switches_and_show(cx: &mut TestAppContext) {
    let (cx, screen, seen) = open(cx, Shown::Part);
    click(cx, "nixa-screen-keep");
    assert!(screen.read_with(cx, |s, _| s.prefs.keep_open));
    click(cx, "nixa-screen-quiet");
    assert!(screen.read_with(cx, |s, _| s.prefs.quiet));
    click(cx, "nixa-screen-keep");
    assert!(!screen.read_with(cx, |s, _| s.prefs.keep_open));
    assert!(screen.read_with(cx, |s, _| s.error.is_none()));
    click(cx, "nixa-screen-show");
    assert_eq!(seen.borrow().last(), Some(&ScreenEvent::Show));
}

#[gpui::test]
fn the_look_switch_leaves_room_for_the_window_buttons(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    redraw(cx);
    let galaxy = cx.debug_bounds("nixa-look-universe").expect("switch in the galaxy");
    assert!(f32::from(galaxy.left()) >= super::office::view::WINDOW_BUTTONS, "{galaxy:?}");
    screen.update(cx, |s, cx| s.set_look(Look::Office, cx));
    redraw(cx);
    let shop = cx.debug_bounds("nixa-look-universe").expect("switch in the office");
    assert!(f32::from(shop.left()) >= super::office::view::WINDOW_BUTTONS, "{shop:?}");
}

fn soap_ready(cx: &mut VisualTestContext, screen: &Entity<NixaScreen>) {
    screen.update(cx, |s, cx| {
        s.set_rooms(super::office::Rooms {
            list: vec![super::office::Room {
                id: "s".into(),
                name: "Space s".into(),
                color: 0,
                agents: vec![agent("m1", St::Working, None), agent("m2", St::Idle, Some("m1"))],
                deep: vec![Agent {
                    title: "Deep brain".into(),
                    ..agent("m9", St::Idle, None)
                }],
            }],
            loose_deep: Vec::new(),
            active: Some("s".into()),
        });
        s.set_look(Look::Office, cx);
        s.set_soap(true, cx);
    });
}

fn soap_scene() -> serde_json::Value {
    json!({"type": "office_scene", "scene": {"where": "tea", "cast": ["m2", "m9"], "mood": "fun",
        "lines": [{"who": "m2", "text": "Tea again?", "act": "sip"}, {"who": "m9", "text": "Always.", "act": "laugh"}]}})
}

#[gpui::test]
fn soap_scene_plays_and_a_real_talk_stops_it(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    soap_ready(cx, &screen);
    screen.update(cx, |s, _| {
        let wall = super::now_ms();
        s.office.soap.asked_at = Some(wall);
        assert!(s.on_message(&soap_scene(), Instant::now(), wall));
        assert!(s.office.soap.show.is_some());
        s.add_talk("m1", "m2", None, None, wall);
        assert!(s.office.soap.show.is_none());
        assert!(s.office.soap.ended_at.is_some());
        s.office.soap.asked_at = Some(wall);
        assert!(!s.on_message(&soap_scene(), Instant::now(), wall));
        assert!(s.office.soap.show.is_none());
    });
    redraw(cx);
}

#[gpui::test]
fn soap_scene_is_dropped_when_not_asked_or_the_tab_changes(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    soap_ready(cx, &screen);
    screen.update(cx, |s, cx| {
        let wall = super::now_ms();
        assert!(!s.on_message(&soap_scene(), Instant::now(), wall));
        assert!(s.office.soap.show.is_none());
        s.office.soap.asked_at = Some(wall);
        assert!(s.on_message(&soap_scene(), Instant::now(), wall));
        assert!(s.office.soap.show.is_some());
        s.office.soap.asked_at = Some(wall);
        s.set_tab(Some(super::office::Tab::Company), cx);
        assert!(s.office.soap.show.is_none());
        assert!(s.office.soap.asked_at.is_none());
        assert!(!s.on_message(&soap_scene(), Instant::now(), wall));
        assert!(s.office.soap.show.is_none());
    });
    redraw(cx);
}

#[gpui::test]
fn soap_lines_speak_only_with_sound_on(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    soap_ready(cx, &screen);
    let said = |s: &NixaScreen| s.sent.iter().filter(|m| m["type"] == "office_say").count();
    for mute in [true, false] {
        screen.update(cx, |s, cx| {
            s.set_mute(mute, cx);
            s.office.soap = Default::default();
            let wall = super::now_ms();
            s.office.soap.asked_at = Some(wall);
            s.on_message(&soap_scene(), Instant::now(), wall);
            let show = s.office.soap.show.as_mut().expect("scene plays");
            let at = show.lines_at + 50;
            show.start = wall - at;
            let before = said(s);
            s.step(Instant::now(), wall);
            assert!(s.office.soap.show.is_some());
            assert_eq!(said(s) - before, usize::from(!mute), "mute {mute}");
        });
        redraw(cx);
    }
}

#[gpui::test]
fn soap_asks_the_brain_when_calm(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    soap_ready(cx, &screen);
    screen.update(cx, |s, _| {
        let wall = super::now_ms();
        s.step(Instant::now(), wall);
        let ask = s.sent.iter().find(|m| m["type"] == "office_soap").expect("asked");
        let ids: Vec<&str> = ask["cast"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"m2") && !ids.contains(&"m1"));
        assert_eq!(ask["work"][0]["who"], "m1");
        let n = s.sent.len();
        s.step(Instant::now(), wall + 1000);
        assert_eq!(s.sent.len(), n);
    });
}

#[gpui::test]
fn settings_part_soap_switch_and_models(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Part);
    assert!(!screen.read_with(cx, |s, _| s.prefs.soap));
    click(cx, "nixa-screen-soap");
    assert!(screen.read_with(cx, |s, _| s.prefs.soap));
    screen.update(cx, |s, cx| {
        s.ask_soap_models(cx);
        assert!(s.sent.iter().any(|m| m["type"] == "office_soap_models"));
        s.on_message(
            &json!({"type": "office_soap_models", "model": "gemma4:e4b-it-qat", "url": "http://127.0.0.1:11434",
                "models": ["gemma4:e4b-it-qat", "llama3.2:3b"], "error": null}),
            Instant::now(),
            1,
        );
    });
    click(cx, "nixa-soap-model-1");
    let picked = screen.read_with(cx, |s, _| {
        s.sent.iter().any(|m| m["type"] == "office_soap_model" && m["model"] == "llama3.2:3b")
    });
    assert!(picked);
    screen.update(cx, |s, _| {
        s.on_message(&json!({"type": "office_soap_models", "model": "x", "url": "u", "models": [], "error": "Ollama is not running."}), Instant::now(), 1);
        assert_eq!(s.soap_models.error.as_deref(), Some("Ollama is not running."));
    });
    redraw(cx);
    assert!(cx.debug_bounds("nixa-soap-error").is_some());
    assert!(cx.debug_bounds("nixa-soap-field").is_some());
}

#[gpui::test]
fn office_mic_talk_zooms_to_nixa_and_pauses_the_soap(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    soap_ready(cx, &screen);
    screen.update(cx, |s, _| {
        let wall = super::now_ms();
        s.office.soap.asked_at = Some(wall);
        assert!(s.on_message(&soap_scene(), Instant::now(), wall));
        assert!(s.office.soap.show.is_some());
        let user = json!({"type": "chat", "item": {"id": 1, "who": "user", "text": "How is the build?", "at": wall}});
        assert!(s.on_message(&user, Instant::now(), wall));
        assert!(s.office.soap.show.is_none());
        assert!(s.soap_gate().talk);
        let reply = json!({"type": "chat", "item": {"id": 2, "who": "nixa", "text": "It is green. All tests pass.", "at": wall}});
        assert!(s.on_message(&reply, Instant::now(), wall + 10));
        let start = json!({"type": "office_voice", "who": "nixa", "text": "It is green. All tests pass.", "state": "start", "src": "chat"});
        assert!(s.on_message(&start, Instant::now(), wall + 20));
        assert_eq!(super::office::call::line(&s.office.call, wall + 30).unwrap().text, "It is green.");
        assert!(s.office.queue.is_empty());
        assert!(!s.sent.iter().any(|m| m["type"] == "office_fun"));
    });
    redraw(cx);
}

#[gpui::test]
fn office_voice_events_hold_the_soap_line(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    soap_ready(cx, &screen);
    screen.update(cx, |s, _| {
        let wall = super::now_ms();
        s.office.soap.asked_at = Some(wall);
        s.on_message(&soap_scene(), Instant::now(), wall);
        let show = s.office.soap.show.as_mut().expect("scene plays");
        let at = show.lines_at + 50;
        show.start = wall - at;
        s.step(Instant::now(), wall);
        let line = s.office.voice.line.clone().expect("line posted");
        assert_eq!(line.text, "Tea again?");
        let start = json!({"type": "office_voice", "who": line.who, "text": line.text, "state": "start", "src": "office"});
        assert!(s.on_message(&start, Instant::now(), wall + 100));
        s.step(Instant::now(), wall + 200);
        let show = s.office.soap.show.as_ref().expect("still on");
        assert!(show.open);
        assert!(show.times[0].1 > 8000);
    });
    redraw(cx);
}

#[gpui::test]
fn office_agent_leave_and_back_events(cx: &mut TestAppContext) {
    let (cx, screen, _) = open(cx, Shown::Screen);
    soap_ready(cx, &screen);
    screen.update(cx, |s, _| {
        let wall = super::now_ms();
        let gone = json!({"type": "agent_leave", "id": "m1", "name": "", "until": 0, "line": "", "to": "m2", "to_name": ""});
        assert!(s.on_message(&gone, Instant::now(), wall));
        assert_eq!(s.office.leave.trips.len(), 1);
        let back = json!({"type": "agent_back", "id": "m1", "name": "", "line": "Back at my desk."});
        assert!(s.on_message(&back, Instant::now(), wall + 5000));
        assert!(s.office.leave.trips[0].back);
    });
    redraw(cx);
}
