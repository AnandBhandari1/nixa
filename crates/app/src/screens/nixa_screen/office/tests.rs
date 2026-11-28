use super::camera::{self, cam_of};
use super::plan::{self, Role};
use super::social;
use super::talk::{self, Phase, Stage};
use super::{Room, Rooms, Tab, rooms_of, shown_tab};
use crate::screens::nixa_screen::map::{Agent, DEEP, NIXA, St};

fn agent(id: &str, status: St, parent: Option<&str>) -> Agent {
    Agent {
        id: id.into(),
        title: format!("{id} · job #7"),
        program: "claude".into(),
        mono: "C".into(),
        status,
        parent: parent.map(str::to_string),
        seq: 1,
        number: Some(3),
        persona: None,
    }
}

fn room(id: &str, agents: Vec<Agent>) -> Room {
    Room {
        id: id.into(),
        name: format!("Space {id}"),
        color: 0,
        agents,
        deep: Vec::new(),
    }
}

fn rooms() -> Rooms {
    Rooms {
        list: vec![
            room("a", vec![agent("a1", St::Working, None), agent("a2", St::Idle, None)]),
            room("b", vec![agent("b1", St::Blocked, None)]),
        ],
        loose_deep: vec![agent("d1", St::Idle, None)],
        active: Some("b".into()),
    }
}

#[test]
fn tab_follows_the_active_space_until_one_is_picked() {
    let r = rooms();
    assert_eq!(shown_tab(None, &r), Tab::Space("b".into()));
    assert_eq!(shown_tab(Some(&Tab::Company), &r), Tab::Company);
    assert_eq!(shown_tab(Some(&Tab::Space("a".into())), &r), Tab::Space("a".into()));
    assert_eq!(shown_tab(Some(&Tab::Space("gone".into())), &r), Tab::Space("b".into()));
    assert_eq!(rooms_of(&r, &Tab::Space("a".into())).len(), 1);
    assert_eq!(rooms_of(&r, &Tab::Company).len(), 2);
}

#[test]
fn plan_seats_staff_in_their_space_and_brains_in_their_rooms() {
    let r = rooms();
    let list = rooms_of(&r, &Tab::Company);
    let p = plan::build(&list, &r.loose_deep, "Nixa", false);
    assert_eq!(p.areas.len(), 2);
    let a1 = plan::find(&p, "a1").unwrap();
    assert!(p.areas[0].rect.x <= a1.seat.0 && a1.seat.0 <= p.areas[0].rect.x + p.areas[0].rect.w);
    assert_eq!(plan::find(&p, NIXA).unwrap().role, Role::Cto);
    assert!(matches!(plan::find(&p, DEEP).unwrap().role, Role::Deep(_)));
    assert_eq!(p.areas[1].empty.len(), 1);
    for person in &p.people {
        assert!(person.stand.0 >= 0.0 && person.stand.0 <= p.w);
        assert!(person.stand.1 >= 0.0 && person.stand.1 <= p.h);
    }
    let one = rooms_of(&r, &Tab::Space("b".into()));
    let small = plan::build(&one, &r.loose_deep, "Nixa", false);
    assert!(plan::find(&small, "a1").is_none());
    assert!(plan::find(&small, "b1").is_some());
    let tall = plan::build(&list, &r.loose_deep, "Nixa", true);
    assert!(tall.w < p.w);
    assert!(tall.areas[1].rect.y > tall.areas[0].rect.y);
}

#[test]
fn talk_lines_match_what_happened() {
    let list = vec![agent("a1", St::Working, None), agent("b1", St::Blocked, None)];
    let give = talk::talk_of(NIXA, "a1", None, None, &list, 1);
    assert_eq!(give.first, "Please work on task #7.");
    let stuck = talk::talk_of("b1", NIXA, None, None, &list, 1);
    assert_eq!(stuck.first, "I am stuck. I need your help.");
    let ask = talk::talk_of(NIXA, DEEP, None, None, &list, 1);
    assert_eq!(ask.second, "Let me think it through.");
    let real = talk::talk_of(NIXA, "a1", Some("send"), Some("  Fix   the login page  "), &list, 1);
    assert_eq!(real.first, "Fix the login page");
    assert!(real.given);
}

#[test]
fn same_talk_twice_is_kept_once_and_real_text_wins() {
    let list = vec![agent("a1", St::Working, None)];
    let mut stage = Stage::default();
    talk::push(&mut stage, talk::talk_of(NIXA, "a1", None, None, &list, 1000));
    talk::push(&mut stage, talk::talk_of(NIXA, "a1", None, Some("Ship it"), &list, 1500));
    assert_eq!(stage.queue.len(), 1);
    assert_eq!(stage.queue[0].first, "Ship it");
    talk::push(&mut stage, talk::talk_of(NIXA, "a1", None, None, &list, 9000));
    assert_eq!(stage.queue.len(), 2);
}

#[test]
fn a_talk_walks_speaks_and_walks_back() {
    let r = rooms();
    let list = rooms_of(&r, &Tab::Company);
    let p = plan::build(&list, &r.loose_deep, "Nixa", false);
    let mut stage = Stage::default();
    talk::push(&mut stage, talk::talk_of(NIXA, "a1", None, None, &r.list[0].agents, 0));
    talk::advance(&mut stage, &p, 0);
    let mut play = stage.now.clone().unwrap();
    assert!(matches!(talk::phase(&play, 0), Phase::WalkTo(_)));
    assert_eq!(talk::cut(&play, 0).dolly, 0.0);
    let say = play.walk + 200;
    assert!(matches!(talk::phase(&play, say), Phase::Say1(_)));
    assert_eq!(talk::cut(&play, say).dolly, 1.0);
    assert!(matches!(talk::due_line(&mut play, say), Some(talk::Line::First(..))));
    assert!(talk::due_line(&mut play, say).is_none());
    let end = talk::end_ms(&play) + 1;
    assert_eq!(talk::phase(&play, end), Phase::Over);
    talk::advance(&mut stage, &p, end);
    assert!(stage.now.is_none());
}

#[test]
fn talk_with_someone_not_in_view_is_dropped() {
    let r = rooms();
    let one = rooms_of(&r, &Tab::Space("b".into()));
    let p = plan::build(&one, &r.loose_deep, "Nixa", false);
    let mut stage = Stage::default();
    talk::push(&mut stage, talk::talk_of(NIXA, "a1", None, None, &[], 0));
    talk::advance(&mut stage, &p, 0);
    assert!(stage.now.is_none());
    assert!(stage.queue.is_empty());
}

#[test]
fn camera_overview_fits_the_plan_and_close_up_zooms_in() {
    let (w, h) = (400.0, 300.0);
    let over = camera::overview(600.0, 400.0, w, h);
    let cam = cam_of(over, w, h);
    let (x0, y0) = cam.to_screen(0.0, 0.0);
    let (x1, y1) = cam.to_screen(600.0, 400.0);
    assert!(x0 >= 0.0 && x1 <= w && y0 >= camera::TOP_ROOM - 1.0 && y1 <= h);
    let close = camera::close((100.0, 100.0), (130.0, 100.0), w, h);
    assert!(close.span < over.span / 2.0);
}

use super::motion;
use super::plan::{CELL_W, LOUNGE_ROOM};
use super::sprite::{self, Act, Arm, Face, Pose};
use gpui::hsla;

#[test]
fn trip_starts_at_zero_ends_at_one_and_never_goes_back() {
    for total in [100.0, 900.0, 4000.0] {
        assert_eq!(motion::trip(0.0, total), 0.0);
        assert!((motion::trip(total, total) - 1.0).abs() < 1e-5);
        let mut last = 0.0;
        for i in 0..=200 {
            let k = motion::trip(total * i as f32 / 200.0, total);
            assert!(k + 1e-6 >= last);
            last = k;
        }
    }
}

#[test]
fn easing_curves_hit_their_ends_and_out_back_overshoots_a_little() {
    for f in [motion::in_cubic, motion::out_cubic, motion::in_out_cubic] {
        assert_eq!(f(0.0), 0.0);
        assert!((f(1.0) - 1.0).abs() < 1e-6);
        assert_eq!(f(-3.0), 0.0);
    }
    assert!((motion::out_back(1.0, 0.9) - 1.0).abs() < 1e-6);
    let top = (0..=1000)
        .map(|i| motion::out_back(i as f32 / 1000.0, 0.9))
        .fold(0.0, f32::max);
    assert!((1.02..=1.04).contains(&top));
    assert_eq!(motion::seed("a"), motion::seed("a"));
    assert_ne!(motion::hash(1, 2), motion::hash(1, 3));
}

#[test]
fn sprite_cells_never_panic_for_any_pose_step_or_style() {
    let base = sprite::looks_of("x", Role::Staff, hsla(0.5, 0.5, 0.5, 1.0));
    let faces = [Face::Down, Face::Up, Face::Left, Face::Right];
    let mut poses = vec![
        Pose::Sit { typing: true, away: false },
        Pose::Sit { typing: false, away: true },
        Pose::Sofa,
        Pose::Stretch,
    ];
    for f in faces {
        poses.push(Pose::Stand { face: f });
        for step in 0..4 {
            poses.push(Pose::Walk { face: f, step });
        }
    }
    let acts = [
        Act::default(),
        Act { blink: true, look: 1, mouth: true, arm: Arm::Up, ..Act::default() },
        Act { look: -1, arm: Arm::Wave(true), squash: true, ..Act::default() },
    ];
    for style in 0..5 {
        for glasses in [false, true] {
            let l = sprite::Looks { style, glasses, tie: !glasses, ..base };
            for pose in &poses {
                let (start, end) = sprite::rows_of(*pose, &l);
                assert!(start >= -3 && end <= 20);
                for act in &acts {
                    for row in -4..21 {
                        for col in 0..15 {
                            let _ = sprite::cell(*pose, &l, act, row, col);
                        }
                    }
                }
            }
        }
    }
    let side = sprite::cell(Pose::Stand { face: Face::Right }, &base, &Act::default(), 5, 9);
    let left = sprite::cell(Pose::Stand { face: Face::Left }, &base, &Act::default(), 5, 4);
    assert_eq!(side, b'e');
    assert_eq!(left, b'e');
}

#[test]
fn sprite_helpers_pick_faces_steps_and_head_tops() {
    assert_eq!(sprite::face_of(5.0, 1.0), Face::Right);
    assert_eq!(sprite::face_of(-5.0, 1.0), Face::Left);
    assert_eq!(sprite::face_of(1.0, -5.0), Face::Up);
    assert_eq!(sprite::face_of(0.0, 5.0), Face::Down);
    assert_eq!(sprite::step_of(0.0), 0);
    assert_eq!(sprite::step_of(6.5), 1);
    assert_eq!(sprite::step_of(24.0), 0);
    let at = (10.0, 100.0);
    assert_eq!(sprite::head_y(Pose::Sit { typing: false, away: false }, at), 86.0);
    assert_eq!(sprite::head_y(Pose::Stand { face: Face::Down }, at), 80.0);
    assert_eq!(sprite::head_y(Pose::Sofa, at), 83.0);
}

#[test]
fn plan_uses_bigger_cells_and_desks_line_up_with_lanes() {
    let r = rooms();
    let list = rooms_of(&r, &Tab::Company);
    let p = plan::build(&list, &r.loose_deep, "Nixa", false);
    let (desk, seat, stand) = plan::desk_at(0.0, 0.0);
    assert_eq!((desk.x, desk.y, desk.w, desk.h), (10.0, 32.0, 44.0, 16.0));
    assert_eq!(seat, (32.0, 30.0));
    assert_eq!(stand, (32.0, 64.0));
    let a = &p.areas[0];
    for (x, _) in &a.cells {
        assert!(((x - a.left) / CELL_W).fract().abs() < 1e-4);
    }
    assert!(p.sofa.is_some());
    assert!(!p.windows.is_empty());
    assert_eq!(p.coolers.len(), p.areas.len() + 1);
    assert!(p.plants.len() >= p.areas.len() * 2);
    for h in &p.hangouts {
        assert!(h.at.0 >= 0.0 && h.at.0 <= p.w && h.at.1 >= 0.0 && h.at.1 <= p.h);
    }
    let t = p.table;
    for d in p.people.iter().filter(|q| matches!(q.role, Role::Deep(_))) {
        assert!(d.stand.1 < p.meet.y + p.meet.h);
        assert!(d.seat.1 - sprite::SIT_TOP > p.meet.y);
        assert!(d.seat.1 == t.y + 2.0 || d.seat.1 == t.y + t.h + 20.0);
    }
}

#[test]
fn narrow_plan_puts_the_lounge_below_the_last_space() {
    let r = rooms();
    let list = rooms_of(&r, &Tab::Company);
    let tall = plan::build(&list, &r.loose_deep, "Nixa", true);
    assert!(tall.narrow);
    assert!(tall.lounge.w >= LOUNGE_ROOM);
    let bottom = tall
        .areas
        .iter()
        .map(|a| a.rect.y + a.rect.h)
        .fold(0.0, f32::max);
    assert!(tall.lounge.y > bottom);
    assert!(tall.lounge.y + tall.lounge.h <= tall.h);
    let sofa = tall.sofa.unwrap();
    assert!(sofa.y >= tall.lounge.y);
    assert!(tall.hangouts.iter().any(|h| h.kind == plan::Hang::Sofa && h.at.1 > bottom));
    assert_eq!(tall.cabin.w + tall.meet.w, plan::CABIN_W_NARROW + plan::MEET_W_NARROW);
}

use super::npc;
use super::path;
use super::view;
use crate::screens::nixa_screen::mode::Mode;

fn three_rooms() -> Rooms {
    let many = |p: &str, n: usize, st: St| {
        (0..n).map(|i| agent(&format!("{p}{i}"), st, None)).collect::<Vec<_>>()
    };
    let mut a = many("a", 3, St::Idle);
    a.push(agent("aw", St::Working, None));
    Rooms {
        list: vec![
            room("a", a),
            room("b", many("b", 2, St::Done)),
            room("c", many("c", 5, St::Idle)),
        ],
        loose_deep: vec![agent("d1", St::Working, None), agent("d2", St::Idle, None)],
        active: None,
    }
}

fn grown(r: plan::R, p: (f32, f32)) -> bool {
    p.0 > r.x - 2.0 && p.0 < r.x + r.w + 2.0 && p.1 > r.y - 2.0 && p.1 < r.y + r.h + 2.0
}

#[test]
fn routes_go_around_desks_and_tables_and_end_where_asked() {
    let r = three_rooms();
    let list = rooms_of(&r, &Tab::Company);
    for narrow in [false, true] {
        let p = plan::build(&list, &r.loose_deep, "Nixa", narrow);
        let mut blocks: Vec<plan::R> = p.people.iter().filter_map(|q| q.desk).collect();
        blocks.extend(p.areas.iter().flat_map(|a| a.empty.iter().copied()));
        blocks.push(p.table);
        let mut points: Vec<(f32, f32)> = p.people.iter().map(|q| q.stand).collect();
        for h in &p.hangouts {
            if matches!(h.kind, plan::Hang::Visit | plan::Hang::Stretch) {
                continue;
            }
            for slot in 0..h.slots {
                points.push(plan::slot_at(h, slot));
            }
        }
        for (i, &a) in points.iter().enumerate() {
            for (j, &b) in points.iter().enumerate() {
                if i == j || plan::distance(a, b) < 0.5 {
                    continue;
                }
                let route = path::route(&p, a, b);
                assert_eq!(route.pts[0], a);
                assert_eq!(route.pts[route.n - 1], b);
                assert!(route.len > 0.0);
                let mut d = 0.0;
                while d <= route.len {
                    let (at, _) = path::at(&route, d);
                    for k in &blocks {
                        assert!(!grown(*k, at), "narrow {narrow} {a:?} -> {b:?} hits {k:?} at {at:?}");
                    }
                    d += 2.0;
                }
                let back = path::reversed(&route);
                assert_eq!(back.pts[0], b);
                assert_eq!(path::at(&back, back.len + 5.0).0, a);
            }
        }
    }
}

#[test]
fn rooms_are_found_and_route_faces_follow_the_leg() {
    let r = three_rooms();
    let list = rooms_of(&r, &Tab::Company);
    let p = plan::build(&list, &r.loose_deep, "Nixa", false);
    let a = p.areas[0].rect;
    assert_eq!(path::room_of(&p, (a.x + 5.0, a.y + 5.0)), Some(path::Where::Area(0)));
    assert_eq!(path::room_of(&p, (5.0, 5.0)), Some(path::Where::Cabin));
    assert_eq!(path::room_of(&p, (p.w - 5.0, 5.0)), Some(path::Where::Meet));
    assert_eq!(path::room_of(&p, (p.lounge.x + 20.0, 5.0)), Some(path::Where::Lounge));
    assert_eq!(path::room_of(&p, (5.0, plan::STRIP_H + 4.0)), None);
    let s = p.people[0].stand;
    let route = path::route(&p, s, (s.0 + 30.0, s.1));
    assert_eq!(route.n, 2);
    assert_eq!(path::at(&route, 10.0).1, sprite::Face::Right);
}

fn idle_space() -> plan::Plan {
    let agents = (0..8).map(|i| agent(&format!("s{i}"), St::Idle, None)).collect();
    let r = Rooms {
        list: vec![room("x", agents)],
        loose_deep: Vec::new(),
        active: None,
    };
    let list = rooms_of(&r, &Tab::Company);
    plan::build(&list, &r.loose_deep, "Nixa", false)
}

fn rising(p: &plan::Person, at: (f32, f32)) -> bool {
    let top = view::perch(p).1.min(p.stand.1);
    let low = view::perch(p).1.max(p.stand.1);
    (at.0 - p.seat.0).abs() < 0.01 && at.1 >= top - 0.01 && at.1 <= low + 0.01
}

#[test]
fn idle_staff_never_share_a_spot_and_never_jump() {
    let p = idle_space();
    let mut out = 0;
    let mut now = 0;
    while now <= 600_000 {
        let a = npc::life(&p, now, &[]);
        let b = npc::life(&p, now + 50, &[]);
        let rest: Vec<(f32, f32)> = a.iter().flatten().filter(|n| n.resting).map(|n| n.at).collect();
        for (i, x) in rest.iter().enumerate() {
            for y in &rest[i + 1..] {
                assert!(plan::distance(*x, *y) >= 3.0, "shared spot at {now}");
            }
        }
        for ((x, y), q) in a.iter().zip(&b).zip(&p.people) {
            if let (Some(x), Some(y)) = (x, y) {
                let near = plan::distance(x.at, y.at) <= 3.0 || (rising(q, x.at) || rising(q, y.at));
                assert!(near, "jump at {now}: {:?} -> {:?}", x.at, y.at);
            }
        }
        out += a.iter().flatten().count();
        now += 250;
    }
    assert!(out > 0);
}

#[test]
fn busy_people_stay_home_and_trips_are_fixed_per_round() {
    let p = idle_space();
    let ids: Vec<&str> = p.people.iter().map(|q| q.id.as_str()).collect();
    for now in (0..300_000).step_by(1000) {
        assert!(npc::life(&p, now, &ids).iter().all(Option::is_none));
    }
    let first = &p.people[0];
    let t = npc::trip_of(first, &p, 3);
    assert_eq!(t, npc::trip_of(first, &p, 3));
    assert_eq!(plan::weight(plan::Hang::Tea), 3);
    assert_eq!(plan::weight(plan::Hang::Window), 1);
    let tea = p.hangouts.iter().find(|h| h.kind == plan::Hang::Tea).unwrap();
    assert_eq!(tea.slots, 2);
    assert!(plan::distance(plan::slot_at(tea, 0), plan::slot_at(tea, 1)) >= 10.0);
}

#[test]
fn a_talk_starts_where_the_walker_is_now() {
    let p = idle_space();
    let mut found = None;
    for now in (0..2_000_000).step_by(500) {
        let life = npc::life(&p, now, &[]);
        if let Some(i) = life
            .iter()
            .position(|n| n.is_some_and(|n| n.resting && n.hang == Some(plan::Hang::Tea)))
        {
            found = Some((now, i));
            break;
        }
    }
    let (now, i) = found.expect("someone goes for tea");
    let who = p.people[i].id.clone();
    let t = talk::talk_of(&who, NIXA, None, None, &[], now);
    let play = talk::start(t, &p, now, &[]).unwrap();
    assert_ne!(play.from_at, p.people[i].stand);
    let s = view::spot_of(&p.people[i], Some(&play), &p, now, None);
    assert!(plan::distance(s.at, play.from_at) < 0.01);
}

#[test]
fn working_staff_sip_pause_and_type() {
    let mut seen = [false; 3];
    for now in (0..40_000).step_by(100) {
        seen[view::rhythm("w1", now) as usize] = true;
    }
    assert_eq!(seen, [true; 3]);
    let r = three_rooms();
    let list = rooms_of(&r, &Tab::Company);
    let p = plan::build(&list, &r.loose_deep, "Nixa", false);
    let w = plan::find(&p, "aw").unwrap();
    for now in (0..40_000).step_by(100) {
        let s = view::seated(w, now);
        match view::rhythm("aw", now) {
            0 => assert_eq!(s.prop, sprite::Prop::Cup { lift: true }),
            1 => assert_eq!(s.act.hands, (false, false)),
            _ => assert!(matches!(s.pose, Pose::Sit { typing: true, .. })),
        }
    }
    assert_eq!(view::cto_status(Mode::Think), St::Working);
    assert_eq!(view::cto_status(Mode::Off), St::Idle);
}

#[test]
fn cto_walks_a_round_when_free_and_stays_home_when_busy() {
    let r = three_rooms();
    let list = rooms_of(&r, &Tab::Company);
    let p = plan::build(&list, &r.loose_deep, "Nixa", true);
    let ci = p.people.iter().position(|q| q.id == NIXA).unwrap();
    let mut out = 0;
    let mut last: Option<(f32, f32)> = None;
    for now in (0..npc::CTO_PERIOD * 3).step_by(50) {
        let n = npc::life(&p, now, &[])[ci];
        if let (Some(n), Some(l)) = (n, last) {
            let cto = &p.people[ci];
            assert!(plan::distance(n.at, l) <= 3.0 || (rising(cto, n.at) || rising(cto, l)));
        }
        last = n.map(|n| n.at);
        out += n.is_some() as usize;
        assert!(npc::life(&p, now, &[NIXA])[ci].is_none());
    }
    assert!(out > 0);
    let mut busy = p.clone();
    busy.people[ci].status = St::Working;
    assert!((0..npc::CTO_PERIOD).step_by(500).all(|t| npc::life(&busy, t, &[])[ci].is_none()));
}

#[test]
fn chat_pairs_pick_one_speaker_and_nod_briefly() {
    let a = super::social::make(1, 2, ("x", "y"), 0);
    let b = super::social::make(1, 2, ("x", "y"), 1800);
    assert_ne!(a.speaker, b.speaker);
    assert_eq!(a.speaker + a.listener, 3);
    let nods = (0..900).filter(|t| super::social::nod(&a, *t)).count();
    assert_eq!(nods, 150);
}

use super::bubble;
use super::moment;

fn play_of(walk: u64, say1: u64, say2: u64) -> talk::Playing {
    talk::Playing {
        talk: talk::talk_of(NIXA, "a1", None, None, &[], 0),
        start: 1000,
        walk,
        say1,
        say2,
        said: 0,
        from_at: (0.0, 0.0),
        to_at: (100.0, 0.0),
        lead: 0,
        open: false,
    }
}

#[test]
fn camera_cut_dollies_in_holds_and_dollies_out_with_bars_first() {
    let p = play_of(3000, 3000, 2000);
    let (a, c) = (p.start + 3000, p.start + 8000);
    let cut = |t: u64| talk::cut(&p, t);
    assert_eq!(cut(a - 751).dolly, 0.0);
    assert!(cut(a + 150).dolly > 0.99);
    assert_eq!(cut(c + 200).dolly, 1.0);
    assert_eq!(cut(c + 1250).dolly, 0.0);
    assert_eq!(cut(a - 750 - 121).bars, 0.0);
    assert!(cut(a - 750 - 60).bars > 0.0);
    assert_eq!(cut(a - 750 - 60).dolly, 0.0);
    assert_eq!(cut(c + 650 + 500).bars, 0.0);
    let mut last = 0.0;
    for t in (p.start..a + 150).step_by(10) {
        let d = cut(t).dolly;
        assert!(d >= last - 1e-6);
        last = d;
    }
    assert!(talk::end_ms(&p) >= 8000 + 1250);
    let short = play_of(700, 2400, 1600);
    assert!(talk::end_ms(&short) >= 700 + 4000 + 1250);
}

#[test]
fn walk_time_follows_route_length_and_stays_in_bounds() {
    assert_eq!(talk::walk_ms(0.0), talk::WALK_MIN);
    assert_eq!(talk::walk_ms(70.0), 1440);
    assert_eq!(talk::walk_ms(100_000.0), talk::WALK_MAX);
    assert_eq!(talk::WALK_MAX, 5200);
}

#[test]
fn camera_blend_pushes_a_little_when_zooming_in_and_close_keeps_both_inside() {
    let a = camera::Shot { cx: 0.0, cy: 0.0, span: 400.0 };
    let b = camera::Shot { cx: 100.0, cy: 50.0, span: 100.0 };
    let tight = (0..=100)
        .map(|i| camera::blend(a, b, i as f32 / 100.0, true).span)
        .fold(f32::MAX, f32::min);
    assert!(tight < 100.0 && tight > 94.0);
    let out = (0..=100)
        .map(|i| camera::blend(a, b, i as f32 / 100.0, false).span)
        .fold(f32::MAX, f32::min);
    assert!((out - 100.0).abs() < 0.01);
    assert_eq!(camera::blend(a, b, 0.0, true), a);
    let (w, h) = (400.0, 300.0);
    for (p, q) in [((0.0, 0.0), (30.0, 0.0)), ((0.0, 0.0), (200.0, 10.0))] {
        let shot = camera::close(p, q, w, h);
        let cam = cam_of(shot, w, h);
        for x in [p.0, q.0] {
            let (sx, _) = cam.to_screen(x, 0.0);
            assert!(sx >= w * 0.15 - 0.5 && sx <= w * 0.85 + 0.5);
        }
        assert!(shot.span >= camera::CLOSE_SPAN);
    }
}

#[test]
fn typewriter_starts_late_ends_full_and_never_goes_back() {
    assert_eq!(bubble::shown_chars(30, 0, 3000), 0);
    assert_eq!(bubble::shown_chars(30, 119, 3000), 0);
    assert_eq!(bubble::shown_chars(30, 120 + 2100, 3000), 30);
    assert_eq!(bubble::shown_chars(30, 120 + 400, 1000), 30);
    assert_eq!(bubble::shown_chars(0, 5000, 3000), 0);
    let mut last = 0;
    for tb in (0..5000).step_by(7) {
        let n = bubble::shown_chars(30, tb, 3000);
        assert!(n >= last && n <= 30);
        last = n;
    }
}

#[test]
fn bubbles_pop_in_dim_and_pop_out() {
    let start = bubble::pop_in(0);
    assert!((start.scale - 0.6).abs() < 1e-4 && start.alpha == 0.0);
    let peak = (0..220).map(|t| bubble::pop_in(t).scale).fold(0.0, f32::max);
    assert!(peak > 1.0);
    let full = bubble::pop_in(400);
    assert!((full.scale - 1.0).abs() < 1e-4 && full.alpha == 1.0);
    let dim = bubble::dimmed(full, 180);
    assert!((dim.alpha - 0.45).abs() < 1e-4 && (dim.scale - 0.92).abs() < 1e-4 && dim.up == 6.0);
    let gone = bubble::popped_out(full, 160);
    assert!(gone.alpha == 0.0 && (gone.scale - 0.85).abs() < 1e-4);
}

#[test]
fn idle_acting_breathes_blinks_and_glances_on_a_schedule() {
    let sit = Pose::Sit { typing: false, away: false };
    let (mut lift, mut blink, mut look) = (0, 0, 0);
    for now in (0..60_000).step_by(10) {
        let a = motion::act_of(42, sit, now);
        lift += (a.lift < 0.0) as u32;
        blink += a.blink as u32;
        look += (a.look != 0) as u32;
        assert!(a.look.abs() <= 1);
    }
    assert!(lift > 2000 && lift < 3000);
    assert!(blink > 0 && blink < 400);
    assert!(look > 0 && look < 1000);
    for now in (0..20_000).step_by(10) {
        let a = motion::act_of(42, Pose::Sofa, now);
        assert!(a.lift == 0.0 && !a.blink);
    }
    let own = Act { lift: 1.0, hands: (true, false), ..Act::default() };
    let idle = Act { lift: -1.0, blink: true, look: 1, ..Act::default() };
    let m = motion::merge(idle, own);
    assert_eq!((m.lift, m.blink, m.look, m.hands), (1.0, true, 1, (true, false)));
    let ups = (0..3000).step_by(90).filter(|t| motion::hands_of(5, *t).0).count();
    assert!(ups > 0 && ups < 34);
}

#[test]
fn sips_go_up_hold_and_come_down() {
    assert_eq!(motion::sip_k(0, 900), 0.0);
    assert_eq!(motion::sip_k(450, 900), 1.0);
    assert!(motion::sip_k(899, 900) < 0.05);
    assert_eq!(motion::sip_k(900, 900), 0.0);
    let t = motion::time_of(0.82, 2000.0);
    assert!((motion::trip(t, 2000.0) - 0.82).abs() < 1e-3);
}

#[test]
fn status_moments_follow_their_timing() {
    assert_eq!(moment::jump(0).0, 0.0);
    assert!((moment::jump(175).0 + 3.0).abs() < 1e-3);
    assert!(moment::jump(380).1 && !moment::jump(300).1);
    assert_eq!(moment::jump(1400), (0.0, false));
    assert_eq!(moment::fade(0), 1.0);
    assert_eq!(moment::fade(1400), 0.0);
    let (x, y) = moment::confetti(900, 0);
    assert!(x > 16.0 && y > 0.0);
    assert_eq!(moment::confetti(0, 3), (0.0, 0.0));
    assert_eq!(moment::ring(0), (6.0, 0.5));
    assert!((moment::ring(600).0 - 10.0).abs() < 1e-4);
    assert_eq!(moment::pull_in(0), 2.0);
    assert_eq!(moment::pull_in(250), 0.0);
    assert_eq!(moment::dot_size(Some(200), 8.0), 11.0);
    assert_eq!(moment::dot_size(None, 8.0), 8.0);
    assert_eq!(super::furniture::blocked_glow(0), 0.35);
}

#[test]
fn seen_ignores_first_sight_records_changes_and_drops_leavers() {
    let r = rooms();
    let list = rooms_of(&r, &Tab::Company);
    let mut p = plan::build(&list, &r.loose_deep, "Nixa", false);
    let mut stage = Stage::default();
    talk::see(&mut stage, &p, 1000);
    assert_eq!(stage.seen.len(), p.people.len());
    assert!(stage.seen.iter().all(|e| e.2 == 0));
    assert!(!talk::fresh(&stage.seen, 1000));
    let i = p.people.iter().position(|q| q.id == "a2").unwrap();
    p.people[i].status = St::Done;
    talk::see(&mut stage, &p, 5000);
    assert_eq!(talk::age(&stage.seen, "a2", 5300), Some(300));
    assert!(talk::fresh(&stage.seen, 6000));
    assert!(!talk::fresh(&stage.seen, 6400));
    assert_eq!(talk::age(&stage.seen, "a1", 5300), None);
    talk::see(&mut stage, &p, 9000);
    assert_eq!(talk::age(&stage.seen, "a2", 9000), Some(4000));
    p.people.remove(i);
    talk::see(&mut stage, &p, 9500);
    assert!(stage.seen.iter().all(|e| e.0 != "a2"));
}

#[test]
fn talkers_rise_walk_the_route_and_sit_back() {
    let r = rooms();
    let list = rooms_of(&r, &Tab::Company);
    let p = plan::build(&list, &r.loose_deep, "Nixa", false);
    let mut stage = Stage::default();
    talk::push(&mut stage, talk::talk_of(NIXA, "a1", None, None, &r.list[0].agents, 0));
    talk::advance(&mut stage, &p, 0);
    let play = stage.now.clone().unwrap();
    let from = plan::find(&p, NIXA).unwrap();
    let to = plan::find(&p, "a1").unwrap();
    let meet = plan::meet_point(from.stand, to.stand);
    let route = path::route(&p, play.from_at, meet);
    assert_eq!(play.walk, talk::walk_ms(route.len));
    let at = |who: &plan::Person, t: u64| view::spot_of(who, Some(&play), &p, t, None);
    assert!(matches!(at(to, 10).pose, Pose::Sit { .. }));
    assert!(matches!(at(to, play.walk - 1).pose, Pose::Stand { .. }));
    let mut last = at(from, talk::SEAT_MS).at;
    for t in (talk::SEAT_MS..play.walk).step_by(20) {
        let s = at(from, t);
        assert!(plan::distance(s.at, last) <= 4.0);
        last = s.at;
    }
    let say = at(from, play.walk + 600);
    assert_eq!(say.at, meet);
    assert!(matches!(say.pose, Pose::Stand { face: Face::Right | Face::Left }));
    let wave = at(from, play.walk + 100);
    assert!(matches!(wave.act.arm, Arm::Wave(_)));
    let end = play.walk * 2 + play.say1 + play.say2;
    assert!(matches!(at(from, end + 5).pose, Pose::Sit { .. }));
    assert!(matches!(at(to, end + 5).pose, Pose::Sit { .. }));
}

#[test]
fn wander_trips_rise_from_the_chair_and_sit_back_smoothly() {
    let p = idle_space();
    let mut checked = 0;
    let stretch = |n: &npc::Npc| n.hang == Some(plan::Hang::Stretch);
    for (i, q) in p.people.iter().enumerate() {
        let mut last = npc::life(&p, 0, &[])[i];
        for now in (10..300_000).step_by(10) {
            let n = npc::life(&p, now, &[])[i];
            match (n, last) {
                (Some(n), None) if !stretch(&n) => {
                    assert!(plan::distance(n.at, view::perch(q)) <= 2.5, "start {:?}", n.at);
                    checked += 1;
                }
                (None, Some(l)) if !stretch(&l) => {
                    assert!(plan::distance(l.at, view::perch(q)) <= 2.5, "end {:?}", l.at);
                }
                (Some(n), Some(l)) => assert!(plan::distance(n.at, l.at) <= 3.0),
                _ => {}
            }
            last = n;
        }
    }
    assert!(checked > 0);
    let q = &p.people[0];
    assert_eq!(npc::rise_at(q, 0, 1000, 0), None);
    assert_eq!(npc::rise_at(q, 0, 1000, npc::RISE), Some(view::perch(q)));
    assert_eq!(npc::rise_at(q, npc::RISE, 1000, npc::RISE), None);
    let back = npc::rise_at(q, 999, 1000, npc::RISE).unwrap();
    assert!(plan::distance(back, view::perch(q)) < 0.5);
}

#[test]
fn visit_hosts_stay_fixed_when_other_people_change_status() {
    let agents = (0..6)
        .map(|i| agent(&format!("v{i}"), if i % 2 == 0 { St::Idle } else { St::Working }, None))
        .collect();
    let r = Rooms {
        list: vec![room("v", agents)],
        loose_deep: Vec::new(),
        active: None,
    };
    let list = rooms_of(&r, &Tab::Company);
    let p = plan::build(&list, &r.loose_deep, "Nixa", false);
    let mut found = 0;
    for (vi, v) in p.people.iter().enumerate() {
        if v.role != Role::Staff || v.status != St::Idle {
            continue;
        }
        for round in 0..200 {
            let Some(t) = npc::trip_of(v, &p, round).filter(|t| t.host.is_some()) else {
                continue;
            };
            let host = t.host.unwrap();
            for oi in 0..p.people.len() {
                if oi == vi || oi == host || p.people[oi].role != Role::Staff {
                    continue;
                }
                let mut other = p.clone();
                other.people[oi].status = match other.people[oi].status {
                    St::Working => St::Idle,
                    _ => St::Working,
                };
                let u = npc::trip_of(&other.people[vi], &other, round);
                if let Some(u) = u.filter(|u| u.host.is_some()) {
                    assert_eq!(u.host, t.host);
                    assert_eq!(u.to, t.to);
                    assert_eq!(u.start, t.start);
                }
            }
            found += 1;
        }
    }
    assert!(found > 0);
}

#[test]
fn talk_end_is_kept_per_person_and_holds_people_at_their_desks() {
    let p = idle_space();
    let mut ends = Vec::new();
    talk::mark_end(&mut ends, &p, "s1", 100);
    talk::mark_end(&mut ends, &p, "s1", 500);
    talk::mark_end(&mut ends, &p, "nobody", 500);
    assert_eq!(ends.len(), 1);
    assert_eq!(npc::last_end(&ends, "s1"), Some(500));
    let mut found = None;
    for now in (0..2_000_000).step_by(500) {
        let life = npc::life(&p, now, &[]);
        if let Some(i) = life.iter().position(|n| n.is_some_and(|n| n.resting)) {
            found = Some((now, i));
            break;
        }
    }
    let (now, i) = found.expect("someone wanders");
    let ends = vec![(p.people[i].id.clone(), now)];
    assert!(npc::life_after(&p, now, &[], &ends)[i].is_none());
    assert!(npc::life_after(&p, now, &[], &[])[i].is_some());
}

#[test]
fn a_walker_called_from_a_hangout_ends_at_the_desk() {
    let p = idle_space();
    let mut found = None;
    for now in (0..2_000_000).step_by(500) {
        let life = npc::life(&p, now, &[]);
        if let Some(i) = life
            .iter()
            .position(|n| n.is_some_and(|n| n.resting && n.hang == Some(plan::Hang::Tea)))
        {
            found = Some((now, i));
            break;
        }
    }
    let (now, i) = found.expect("someone goes for tea");
    let who = p.people[i].id.clone();
    let mut stage = Stage::default();
    talk::push(&mut stage, talk::talk_of(&who, NIXA, None, None, &[], now));
    talk::advance(&mut stage, &p, now);
    let play = stage.now.clone().unwrap();
    let from = &p.people[i];
    let end = play.start + talk::end_ms(&play);
    let last = view::spot_of(from, Some(&play), &p, end - 1, None);
    assert!(plan::distance(last.at, view::perch(from)) < 0.5);
    assert!(matches!(view::spot_of(from, Some(&play), &p, end, None).pose, Pose::Sit { .. }));
    let c = play.start + play.walk + play.say1 + play.say2;
    let mut prev = view::spot_of(from, Some(&play), &p, c, None).at;
    for t in (c..end).step_by(10) {
        let s = view::spot_of(from, Some(&play), &p, t, None);
        if !rising(from, s.at) {
            assert!(plan::distance(s.at, prev) <= 4.0, "jump at {t}");
        }
        prev = s.at;
    }
    talk::advance(&mut stage, &p, end);
    assert!(stage.now.is_none());
    assert_eq!(npc::last_end(&stage.talk_end, &who), Some(end));
    assert!(npc::life_after(&p, end, &[], &stage.talk_end)[i].is_none());
}

#[test]
fn people_at_their_own_desk_paint_with_the_desk_set() {
    let r = rooms();
    let list = rooms_of(&r, &Tab::Company);
    let p = plan::build(&list, &r.loose_deep, "Nixa", false);
    let q = plan::find(&p, "a2").unwrap();
    let d = q.desk.unwrap();
    let stand = |at| view::Spot {
        at,
        pose: Pose::Stand { face: Face::Down },
        prop: sprite::Prop::None,
        act: Act::default(),
        sip: 0.0,
    };
    assert!(view::at_desk(q, d, &stand((q.seat.0, q.seat.1 + 2.0))));
    assert!(view::at_desk(q, d, &stand(view::perch(q))));
    assert!(!view::at_desk(q, d, &stand(q.stand)));
    assert!(!view::at_desk(q, d, &stand((q.seat.0 + 20.0, q.seat.1 + 2.0))));
    assert!(!view::at_desk(q, d, &stand((q.seat.0, q.seat.1 - 10.0))));
    let mut sit = stand(q.seat);
    sit.pose = Pose::Sit { typing: false, away: false };
    assert!(!view::at_desk(q, d, &sit));
}

#[test]
fn tags_show_the_agent_name_first() {
    let mut a = agent("m1", St::Idle, None);
    assert_eq!(plan::short_tag(&a), "claude 3");
    a.persona = Some("Maya".into());
    assert_eq!(plan::short_tag(&a), "Maya · claude 3");
}

#[test]
fn chatter_lines_come_and_go() {
    let pair = social::Pair { speaker: 1, listener: 2, seed: 0 };
    let line = social::pair_line(&pair, 100).unwrap();
    assert_eq!(line.who, 1);
    assert!(social::CHATTER.contains(&line.text));
    assert!(social::pair_line(&pair, social::SAY_MS + 10).is_none());
    assert!(social::waves("Hi!"));
    assert!(!social::waves("Coffee?"));
}

#[test]
fn real_lines_keep_the_first_sentence_short() {
    assert_eq!(talk::cut_text("Done with the login fix. Tests pass too."), "Done with the login fix.");
    let long = "word ".repeat(40);
    let cut = talk::cut_text(&long);
    assert!(cut.chars().count() <= talk::TEXT_MAX);
    assert!(cut.ends_with("word…"));
}

mod soap_tests {
    use super::super::npc::{self, Npc};
    use super::super::plan::{self, Hang, Plan};
    use super::super::soap::{self, Gate, Place, Soap};
    use super::super::sprite::{Pose, Prop};
    use super::{agent, room};
    use super::super::{Room, Rooms, Tab, rooms_of};
    use crate::screens::nixa_screen::map::{Agent, NIXA, St};
    use serde_json::json;

    fn office() -> (Rooms, Vec<Agent>) {
        let long = Agent {
            title: "Fix the login page so that people can sign in with their own keys again".into(),
            ..agent("a3", St::Working, None)
        };
        let list = vec![agent("a1", St::Idle, None), agent("a2", St::Done, None), long, agent("a4", St::Blocked, None)];
        let mut r = room("a", list.clone());
        r.deep = vec![agent("d1", St::Idle, None)];
        let rooms = Rooms { list: vec![r], loose_deep: Vec::new(), active: None };
        (rooms, list)
    }

    fn plan_of(rooms: &Rooms) -> Plan {
        let list: Vec<&Room> = rooms_of(rooms, &Tab::Company);
        plan::build(&list, &rooms.loose_deep, "Nixa", false)
    }

    fn open_gate() -> Gate {
        Gate { office: true, open: true, on: true, up: true, idle: true, talk: false }
    }

    #[test]
    fn meeting_marks_stay_clear_of_deep_seats() {
        for narrow in [false, true] {
            for count in [1usize, 2, 5, 8] {
                let deep: Vec<Agent> = (0..count).map(|i| agent(&format!("d{i}"), St::Idle, None)).collect();
                let rooms = Rooms { list: vec![room("a", vec![agent("a1", St::Idle, None)])], loose_deep: deep, active: None };
                let list: Vec<&Room> = rooms_of(&rooms, &Tab::Company);
                let p = plan::build(&list, &rooms.loose_deep, "Nixa", narrow);
                let marks = soap::meeting_marks(&p, 4);
                assert_eq!(marks.len(), 4);
                let spots: Vec<(f32, f32)> = p
                    .people
                    .iter()
                    .filter(|d| matches!(d.role, plan::Role::Deep(_)))
                    .flat_map(|d| [d.seat, d.stand])
                    .collect();
                for (i, m) in marks.iter().enumerate() {
                    assert!(p.meet.x < m.to.0 && m.to.0 < p.meet.x + p.meet.w, "{:?}", m.to);
                    assert!(p.meet.y < m.to.1 && m.to.1 < p.meet.y + p.meet.h, "{:?}", m.to);
                    for s in &spots {
                        let apart = (m.to.0 - s.0).abs() >= 14.0 || (m.to.1 - s.1).abs() >= 20.0;
                        assert!(apart, "mark {:?} on deep spot {:?}", m.to, s);
                    }
                    for o in &marks[i + 1..] {
                        assert!((m.to.0 - o.to.0).abs() >= 14.0 || (m.to.1 - o.to.1).abs() >= 20.0);
                    }
                }
            }
        }
    }

    #[test]
    fn soap_asks_only_when_everything_is_calm() {
        let s = Soap::default();
        assert!(soap::should_ask(&s, open_gate(), 1000));
        for g in [
            Gate { office: false, ..open_gate() },
            Gate { open: false, ..open_gate() },
            Gate { on: false, ..open_gate() },
            Gate { up: false, ..open_gate() },
            Gate { idle: false, ..open_gate() },
            Gate { talk: true, ..open_gate() },
        ] {
            assert!(!soap::should_ask(&s, g, 1000), "{g:?}");
        }
        let mut pending = Soap { asked_at: Some(1000), ..Soap::default() };
        assert!(!soap::should_ask(&pending, open_gate(), 20_000));
        soap::expire(&mut pending, 20_000);
        assert_eq!(pending.asked_at, Some(1000));
        soap::expire(&mut pending, 31_000);
        assert_eq!(pending.asked_at, None);
        assert_eq!(pending.ended_at, Some(31_000));
        assert!(!soap::should_ask(&pending, open_gate(), 50_000));
        assert!(soap::should_ask(&pending, open_gate(), 76_000));
        let (rooms, _) = office();
        let p = plan_of(&rooms);
        let scene = soap::scene_of(&scene_msg(&["a1", "a2"], &[("a1", "Hi"), ("a2", "Hey")], "tea"), &p).unwrap();
        let playing = Soap { show: soap::start(scene, &p, &npc::life(&p, 0, &[]), 0), ..Soap::default() };
        assert!(playing.show.is_some());
        assert!(!soap::should_ask(&playing, open_gate(), 900_000));
    }

    #[test]
    fn soap_payload_has_only_free_people_and_short_tasks() {
        let (rooms, agents) = office();
        let p = plan_of(&rooms);
        let life = npc::life(&p, 0, &[]);
        let body = soap::payload(&p, &life, &agents, 21, 0).unwrap();
        let ids: Vec<&str> = body["cast"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"a1") && ids.contains(&"a2") && ids.contains(&"d1") && ids.contains(&NIXA));
        assert!(!ids.contains(&"a3") && !ids.contains(&"a4"));
        let a1 = body["cast"].as_array().unwrap().iter().find(|c| c["id"] == "a1").unwrap();
        assert_eq!(a1["name"], "claude 3");
        assert_eq!(a1["space"], "Space a");
        assert_eq!(a1["role"], "staff");
        let d1 = body["cast"].as_array().unwrap().iter().find(|c| c["id"] == "d1").unwrap();
        assert_eq!(d1["role"], "deep");
        assert_eq!(d1["at"], "meeting");
        let work = body["work"].as_array().unwrap();
        assert_eq!(work.len(), 1);
        assert_eq!(work[0]["who"], "a3");
        assert_eq!(work[0]["task"].as_str().unwrap().chars().count(), soap::TASK_MAX);
        assert_eq!(body["hour"], 21);

        let many: Vec<Agent> = (0..14).map(|i| agent(&format!("x{i}"), St::Idle, None)).collect();
        let busy: Vec<Agent> = (0..9).map(|i| agent(&format!("w{i}"), St::Working, None)).collect();
        let big = Rooms { list: vec![room("x", many), room("w", busy.clone())], loose_deep: Vec::new(), active: None };
        let bp = plan_of(&big);
        let body = soap::payload(&bp, &npc::life(&bp, 0, &[]), &busy, 9, 5000).unwrap();
        assert_eq!(body["cast"].as_array().unwrap().len(), soap::CAST_MAX);
        assert_eq!(body["work"].as_array().unwrap().len(), soap::WORK_MAX);

        let lonely = Rooms { list: vec![room("y", vec![agent("y1", St::Working, None)])], loose_deep: Vec::new(), active: None };
        let lp = plan_of(&lonely);
        let mut lp_busy = lp.clone();
        if let Some(c) = lp_busy.people.iter_mut().find(|p| p.id == NIXA) {
            c.status = St::Working;
        }
        assert!(soap::payload(&lp_busy, &npc::life(&lp_busy, 0, &[]), &[], 9, 0).is_none());
    }

    fn scene_msg(cast: &[&str], lines: &[(&str, &str)], place: &str) -> serde_json::Value {
        let lines: Vec<serde_json::Value> = lines.iter().map(|(w, t)| json!({"who": w, "text": t, "act": "wave"})).collect();
        json!({"type": "office_scene", "scene": {"where": place, "cast": cast, "mood": "fun", "lines": lines}})
    }

    #[test]
    fn soap_scenes_are_checked_against_the_office() {
        let (rooms, _) = office();
        let p = plan_of(&rooms);
        let long = "word ".repeat(40);
        let msg = scene_msg(&["a1", "ghost", "a2", "a3"], &[("a1", "Tea?"), ("ghost", "Boo"), ("a2", &long), ("a3", "busy")], "sofa");
        let sc = soap::scene_of(&msg, &p).unwrap();
        assert_eq!(sc.cast, vec!["a1".to_string(), "a2".to_string()]);
        assert_eq!(sc.lines.len(), 2);
        assert!(sc.lines[1].text.chars().count() <= soap::TEXT_MAX);
        assert_eq!(sc.place, Place::Sofa);
        assert!(soap::scene_of(&scene_msg(&["a1", "ghost"], &[("a1", "Hi")], "tea"), &p).is_none());
        assert!(soap::scene_of(&scene_msg(&["a1", "a2"], &[], "tea"), &p).is_none());
        assert!(soap::scene_of(&scene_msg(&["a1", "a2"], &[("ghost", "Hi"), ("a1", "  ")], "tea"), &p).is_none());
        assert!(soap::scene_of(&json!({"type": "office_scene", "error": "no model"}), &p).is_none());
    }

    #[test]
    fn soap_line_times_follow_the_text() {
        assert_eq!(soap::line_ms(""), 1800);
        assert_eq!(soap::line_ms(&"a".repeat(30)), 700 + 55 * 30);
        assert_eq!(soap::line_ms(&"a".repeat(90)), 4800);
        let (rooms, _) = office();
        let p = plan_of(&rooms);
        let msg = scene_msg(&["a1", "a2"], &[("a1", "Hello there"), ("a2", "Hi"), ("a1", "Tea?")], "tea");
        let show = soap::start(soap::scene_of(&msg, &p).unwrap(), &p, &npc::life(&p, 0, &[]), 0).unwrap();
        assert!(show.lines_at >= soap::ZOOM_MS);
        for w in show.times.windows(2) {
            assert_eq!(w[1].0, w[0].0 + w[0].1 + soap::GAP_MS);
        }
        assert_eq!(soap::line_at(&show, show.lines_at - 1), None);
        assert_eq!(soap::line_at(&show, show.lines_at + 10), Some((0, 10)));
        let mut s = show.clone();
        assert!(soap::due(&mut s, show.lines_at + 10).is_some());
        assert!(soap::due(&mut s, show.lines_at + 20).is_none());
        assert_eq!(soap::dolly(&show, 0), 0.0);
        assert_eq!(soap::dolly(&show, show.lines_at), 1.0);
        assert_eq!(soap::dolly(&show, soap::back_at(&show) + soap::ZOOM_MS), 0.0);
        assert!(!soap::is_over(&show, soap::back_at(&show)));
        assert!(soap::is_over(&show, soap::over_at(&show)));
    }

    #[test]
    fn soap_voice_needs_the_close_up_and_sound() {
        assert!(soap::speak_ok(false, false, true, 1.0));
        assert!(!soap::speak_ok(true, false, true, 1.0));
        assert!(!soap::speak_ok(false, true, true, 1.0));
        assert!(!soap::speak_ok(false, false, false, 1.0));
        assert!(!soap::speak_ok(false, false, true, 0.2));
    }

    fn sitter(spot: (usize, u8), at: (f32, f32)) -> Npc {
        Npc { at, pose: Pose::Sofa, prop: Prop::None, hang: Some(Hang::Sofa), resting: true, spot: Some(spot), host: None }
    }

    #[test]
    fn soap_gathering_spots_stay_clear_of_other_people() {
        let (rooms, _) = office();
        let p = plan_of(&rooms);
        let cast = vec!["a1".to_string(), "a2".to_string(), "d1".to_string()];
        let empty: Vec<Option<Npc>> = vec![None; p.people.len()];
        for place in [Place::Sofa, Place::Tea, Place::Window, Place::Desk, Place::Meeting] {
            let (_, marks, _, order) = soap::marks_for(place, &p, &empty, &cast);
            assert_eq!(marks.len(), 3, "{place:?}");
            assert_eq!(order.len(), 3);
            for (i, a) in marks.iter().enumerate() {
                for b in &marks[i + 1..] {
                    assert!(plan::distance(a.to, b.to) > 8.0, "{place:?} {a:?} {b:?}");
                }
                for q in p.people.iter().filter(|q| !cast.contains(&q.id)) {
                    assert!(plan::distance(a.to, q.seat) > 4.0, "{place:?} sits on {}", q.id);
                }
            }
        }
        let sofa = p.hangouts.iter().position(|h| h.kind == Hang::Sofa).unwrap();
        let mut life = empty.clone();
        let other = p.people.iter().position(|q| q.id == "a4").unwrap();
        life[other] = Some(sitter((sofa, 0), p.hangouts[sofa].at));
        let (got, marks, _, _) = soap::marks_for(Place::Sofa, &p, &life, &cast);
        assert_ne!(got, Place::Sofa);
        assert!(marks.iter().all(|m| plan::distance(m.to, p.hangouts[sofa].at) > 4.0));

        let msg = scene_msg(&["a1", "a2"], &[("a1", "Sofa time"), ("a2", "Yes")], "sofa");
        let show = soap::start(soap::scene_of(&msg, &p).unwrap(), &p, &empty, 0).unwrap();
        assert_eq!(show.place, Place::Sofa);
        let mut held = empty.clone();
        held[other] = Some(sitter((show.hangs[2], 0), p.hangouts[show.hangs[2]].at));
        soap::hold_off(&mut held, &p, &show);
        assert!(held[other].is_none());
        let a1 = p.people.iter().find(|q| q.id == "a1").unwrap();
        let there = soap::spot_of(&show, &p, a1, show.lines_at).unwrap();
        assert_eq!(there.pose, Pose::Sofa);
        let home = soap::spot_of(&show, &p, a1, soap::over_at(&show)).unwrap();
        assert!(matches!(home.pose, Pose::Sit { .. }));
    }

    #[test]
    fn soap_finish_sends_the_cast_back_to_life() {
        let (rooms, _) = office();
        let p = plan_of(&rooms);
        let msg = scene_msg(&["a1", "a2"], &[("a1", "Hi"), ("a2", "Hey")], "tea");
        let mut s = Soap { show: soap::start(soap::scene_of(&msg, &p).unwrap(), &p, &npc::life(&p, 0, &[]), 0), ..Soap::default() };
        let mut ends = Vec::new();
        soap::finish(&mut s, &mut ends, &p, 4000);
        assert!(s.show.is_none());
        assert_eq!(s.ended_at, Some(4000));
        assert!(ends.contains(&("a1".to_string(), 4000)) && ends.contains(&("a2".to_string(), 4000)));
    }
}
