use super::model::*;
use super::*;
use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext,
};
use nixa_protocol::Status;
use std::cell::RefCell;
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<AgentsEvent>>>;

fn id(n: u64) -> TermId {
    TermId::from_number(n)
}

fn agent(n: u64, space: &str, status: Option<Status>) -> Agent {
    Agent {
        tab: format!("t{n}"),
        term: id(n),
        space: space.to_string(),
        space_name: format!("Space {space}"),
        color: 0,
        label: format!("agent {n}"),
        program: "claude".into(),
        cmd: vec!["claude".into()],
        status,
        running: true,
        pinned: false,
        deep: false,
        focused: false,
        seq: n,
        number: Some(n as u32),
    }
}

fn kid(n: u64, parent: u64, status: Option<Status>) -> Kid {
    Kid {
        term: id(n),
        parent: id(parent),
        tab: None,
        name: format!("helper {n}"),
        cmd: vec!["codex".into()],
        status,
        running: true,
    }
}

fn input(agents: Vec<Agent>) -> Input {
    Input {
        active_space: Some("s1".into()),
        tabs: agents
            .iter()
            .map(|a| (a.tab.clone(), a.space.clone()))
            .collect(),
        agents,
        ready: true,
        ..Input::default()
    }
}

fn layout(i: &Input, sort: Sort, scope: Scope) -> View {
    super::model::build(i, sort, scope, None, &BTreeMap::new())
}

fn labels(v: &View) -> Vec<(String, usize)> {
    v.groups
        .iter()
        .map(|g| (g.label.clone(), g.members.len()))
        .collect()
}

#[test]
fn priority_puts_needs_you_first_then_done_working_idle() {
    let i = input(vec![
        agent(1, "s1", Some(Status::Idle)),
        agent(2, "s1", Some(Status::Working)),
        agent(3, "s1", Some(Status::Blocked)),
        agent(4, "s1", Some(Status::Done)),
        agent(5, "s1", Some(Status::Error)),
    ]);
    let v = layout(&i, Sort::Priority, Scope::Space);
    assert_eq!(
        labels(&v),
        vec![
            ("Needs you".into(), 2),
            ("Done".into(), 1),
            ("Working".into(), 1),
            ("Idle".into(), 1)
        ]
    );
    assert_eq!(v.groups[0].members[0].agent.term, id(5));
    assert!(v.groups[0].members[0].compact);
    assert_eq!(v.order.len(), 5);
    assert_eq!(v.order[0].1, id(5));
}

#[test]
fn grouped_makes_one_section_per_space_in_space_order() {
    let mut i = input(vec![
        agent(1, "s1", Some(Status::Idle)),
        agent(2, "s1", Some(Status::Blocked)),
        agent(3, "s2", Some(Status::Working)),
    ]);
    let v = layout(&i, Sort::Grouped, Scope::All);
    assert_eq!(
        labels(&v),
        vec![("Space s1".into(), 2), ("Space s2".into(), 1)]
    );
    assert_eq!(v.groups[0].members[0].name, "agent 1");
    i.agents[1].label = "7".into();
    let v = layout(&i, Sort::Grouped, Scope::All);
    assert_eq!(v.groups[0].members[1].name, "claude");
}

#[test]
fn a_blocked_agent_in_another_space_shows_on_top() {
    let i = input(vec![
        agent(1, "s1", Some(Status::Working)),
        agent(2, "s2", Some(Status::Blocked)),
        agent(3, "s2", Some(Status::Working)),
    ]);
    let v = layout(&i, Sort::Priority, Scope::Space);
    assert_eq!(
        labels(&v),
        vec![("Needs you elsewhere".into(), 1), ("Working".into(), 1)]
    );
    assert_eq!(v.groups[0].members[0].name, "Space s2");
    let all = layout(&i, Sort::Priority, Scope::All);
    assert!(all.groups.iter().all(|g| g.id != ELSEWHERE));
    assert_eq!(all.order.len(), 3);
}

#[test]
fn pins_stay_on_top_in_every_space_but_deep_brains_stay_home() {
    let mut pinned = agent(2, "s2", Some(Status::Idle));
    pinned.pinned = true;
    let mut deep = agent(3, "s2", Some(Status::Idle));
    deep.pinned = true;
    deep.deep = true;
    let i = input(vec![agent(1, "s1", Some(Status::Working)), pinned, deep]);
    let v = layout(&i, Sort::Grouped, Scope::Space);
    assert_eq!(v.groups[0].id, PINNED);
    assert_eq!(v.groups[0].members.len(), 1);
    assert_eq!(v.groups[0].members[0].agent.term, id(2));
    assert!(v.groups[0].members[0].full);
    assert_eq!(
        v.order.iter().map(|o| o.1.clone()).collect::<Vec<_>>(),
        vec![id(2), id(1)]
    );
}

#[test]
fn the_deep_brain_is_the_first_pinned_row_of_its_space() {
    let mut pinned = agent(2, "s1", Some(Status::Idle));
    pinned.pinned = true;
    let mut deep = agent(3, "s1", Some(Status::Idle));
    deep.pinned = true;
    deep.deep = true;
    let i = input(vec![agent(1, "s1", Some(Status::Working)), pinned, deep]);
    let v = layout(&i, Sort::Grouped, Scope::Space);
    assert_eq!(v.groups[0].id, PINNED);
    assert_eq!(v.groups[0].members[0].agent.term, id(3));
    assert_eq!(v.groups[0].members[1].agent.term, id(2));
}

#[test]
fn sub_agents_make_a_tree_and_lift_the_parent() {
    let mut i = input(vec![
        agent(1, "s1", Some(Status::Idle)),
        agent(2, "s1", Some(Status::Working)),
    ]);
    i.kids = vec![
        kid(10, 1, Some(Status::Working)),
        kid(11, 1, Some(Status::Done)),
        kid(12, 10, Some(Status::Blocked)),
    ];
    let rows = tree(&i.kids, &id(1));
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].lines, vec![Line::Mid]);
    assert_eq!(rows[1].lines, vec![Line::Pass, Line::Last]);
    assert_eq!(rows[1].kid.term, id(12));
    assert_eq!(rows[2].lines, vec![Line::Last]);
    assert_eq!(kid_state(Some(Status::Done), true), State::Idle);
    assert_eq!(kid_state(Some(Status::Error), true), State::Blocked);
    let v = layout(&i, Sort::Priority, Scope::Space);
    assert_eq!(v.groups[0].label, "Needs you");
    assert_eq!(v.groups[0].members[0].agent.term, id(1));
    assert_eq!(v.groups[0].members[0].subs.len(), 3);
    i.kids.push(kid(1, 12, Some(Status::Working)));
    assert_eq!(tree(&i.kids, &id(1)).len(), 3);
}

#[test]
fn a_viewed_done_agent_stays_in_done_until_left() {
    let mut a = agent(1, "s1", Some(Status::Done));
    a.focused = true;
    let held = hold(None, std::slice::from_ref(&a));
    assert_eq!(held.as_ref().map(|h| h.state), Some(State::Done));
    a.status = Some(Status::Idle);
    let held = hold(held, std::slice::from_ref(&a));
    assert_eq!(shown_state(&a, held.as_ref()), State::Done);
    a.focused = false;
    let other = {
        let mut b = agent(2, "s1", Some(Status::Working));
        b.focused = true;
        b
    };
    let held = hold(held, &[a.clone(), other]);
    assert_eq!(shown_state(&a, held.as_ref()), State::Idle);
}

#[test]
fn time_in_state_counts_done_and_idle_as_one() {
    let mut changed = BTreeMap::new();
    let mut a = agent(1, "s1", Some(Status::Working));
    assert!(track(&mut changed, std::slice::from_ref(&a), 1000));
    assert!(!track(&mut changed, std::slice::from_ref(&a), 2000));
    a.status = Some(Status::Done);
    assert!(track(&mut changed, std::slice::from_ref(&a), 3000));
    a.status = Some(Status::Idle);
    assert!(!track(&mut changed, std::slice::from_ref(&a), 4000));
    assert_eq!(changed["m1"].at, 3000);
    changed.insert(
        "m9".into(),
        Stamp {
            s: "idle".into(),
            at: 0,
        },
    );
    a.status = Some(Status::Working);
    assert!(track(
        &mut changed,
        std::slice::from_ref(&a),
        FORGET_AFTER_MS + 10
    ));
    assert!(!changed.contains_key("m9"));
    assert_eq!(duration(30_000), "<1m");
    assert_eq!(duration(5 * 60_000), "5m");
    assert_eq!(duration(3 * 3_600_000), "3h");
    assert_eq!(duration(50 * 3_600_000), "2d");
}

#[test]
fn priority_breaks_ties_by_latest_change_then_newest() {
    let i = input(vec![
        agent(1, "s1", Some(Status::Working)),
        agent(2, "s1", Some(Status::Working)),
        agent(3, "s1", Some(Status::Working)),
    ]);
    let mut since = BTreeMap::new();
    since.insert(
        "m1".to_string(),
        Stamp {
            s: "working".into(),
            at: 50,
        },
    );
    let v = super::model::build(&i, Sort::Priority, Scope::Space, None, &since);
    let order: Vec<TermId> = v.order.iter().map(|o| o.1.clone()).collect();
    assert_eq!(order, vec![id(1), id(3), id(2)]);
}

#[test]
fn the_jump_goes_to_the_next_agent_that_needs_you_in_any_space() {
    let mut i = input(vec![
        agent(1, "s1", Some(Status::Blocked)),
        agent(2, "s1", Some(Status::Working)),
        agent(3, "s2", Some(Status::Done)),
    ]);
    assert_eq!(next_needy(&i).map(|n| n.1), Some(id(1)));
    i.agents[0].focused = true;
    assert_eq!(next_needy(&i).map(|n| n.1), Some(id(3)));
    i.agents[0].focused = false;
    i.agents[2].focused = true;
    assert_eq!(next_needy(&i).map(|n| n.1), Some(id(1)));
    let calm = input(vec![agent(1, "s1", Some(Status::Working))]);
    assert!(next_needy(&calm).is_none());
}

#[test]
fn next_and_previous_walk_the_drawn_order() {
    let order = vec![
        ("t1".to_string(), id(1)),
        ("t2".to_string(), id(2)),
        ("t3".to_string(), id(3)),
    ];
    assert_eq!(step(&order, Some(&id(1)), 1).map(|o| o.1), Some(id(2)));
    assert_eq!(step(&order, Some(&id(3)), 1).map(|o| o.1), Some(id(1)));
    assert_eq!(step(&order, Some(&id(1)), -1).map(|o| o.1), Some(id(3)));
    assert_eq!(step(&order, None, 1).map(|o| o.1), Some(id(1)));
    assert_eq!(step(&order, None, -1).map(|o| o.1), Some(id(3)));
    assert!(step(&[], None, 1).is_none());
}

#[test]
fn labels_use_the_name_then_a_telling_title() {
    assert_eq!(label("Reviewer", Some("x"), "claude"), "Reviewer");
    assert_eq!(
        label("", Some("✳ Fix the login"), "claude"),
        "Fix the login"
    );
    assert_eq!(label("", Some("~/Projects/x"), "claude"), "claude");
    assert_eq!(label("", Some("me@mac"), "claude"), "claude");
    assert_eq!(label("", Some("Claude"), "claude"), "claude");
    assert_eq!(label("", None, ""), "shell");
    assert!(is_deep("Nixa deep brain"));
    let role: std::collections::BTreeMap<String, String> = [(ROLE_LABEL.to_string(), DEEP_ROLE.to_string())].into();
    let other: std::collections::BTreeMap<String, String> = [(ROLE_LABEL.to_string(), "helper".to_string())].into();
    assert!(is_deep_term(&role, "codex"));
    assert!(!is_deep_term(&other, "Nixa deep brain"));
    assert!(is_deep_term(&Default::default(), "Nixa deep brain"));
    assert!(!is_deep_term(&Default::default(), "codex"));
    let mut a = agent(1, "s1", Some(Status::Idle));
    let mut b = agent(2, "s1", Some(Status::Idle));
    a.label = "3".into();
    b.label = "4".into();
    let v = layout(&input(vec![a, b]), Sort::Priority, Scope::Space);
    let m = &v.groups[0].members;
    assert_eq!(m[0].detail.as_deref(), Some("4"));
    assert_eq!(m[1].detail.as_deref(), Some("3"));
}

struct Host {
    view: Entity<AgentsView>,
}

impl gpui::Render for Host {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
            .size_full()
            .flex()
            .flex_col()
            .child(render(&self.view))
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<AgentsView>, &mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let (host, vcx) = cx.add_window_view(|_, cx| Host {
        view: cx.new(|cx| super::build(None, cx)),
    });
    let view = vcx.update(|_, cx| host.read(cx).view.clone());
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &AgentsEvent, _| {
            log.borrow_mut().push(e.clone())
        })
        .detach();
    });
    vcx.run_until_parked();
    (view, vcx, seen)
}

fn feed(view: &Entity<AgentsView>, vcx: &mut VisualTestContext, i: Input) {
    vcx.update(|_, cx| view.update(cx, |v, cx| set_input(v, i, cx)));
    vcx.run_until_parked();
}

fn center(vcx: &mut VisualTestContext, selector: &str) -> gpui::Point<gpui::Pixels> {
    vcx.run_until_parked();
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    vcx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

fn click(vcx: &mut VisualTestContext, selector: &str) {
    let at = center(vcx, selector);
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

fn press(vcx: &mut VisualTestContext, selector: &str, button: MouseButton) {
    let at = center(vcx, selector);
    vcx.simulate_mouse_down(at, button, Modifiers::none());
    vcx.simulate_mouse_up(at, button, Modifiers::none());
    vcx.run_until_parked();
}

fn double(vcx: &mut VisualTestContext, selector: &str) {
    let at = center(vcx, selector);
    for count in [1, 2] {
        vcx.simulate_event(MouseDownEvent {
            position: at,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count: count,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            position: at,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count: count,
        });
    }
    vcx.run_until_parked();
}

fn events(seen: &Seen) -> Vec<AgentsEvent> {
    std::mem::take(&mut *seen.borrow_mut())
}

fn read<T>(
    view: &Entity<AgentsView>,
    vcx: &mut VisualTestContext,
    f: impl FnOnce(&AgentsView) -> T,
) -> T {
    vcx.update(|_, cx| f(view.read(cx)))
}

fn sample() -> Input {
    let mut i = input(vec![
        agent(1, "s1", Some(Status::Blocked)),
        agent(2, "s1", Some(Status::Working)),
        agent(3, "s2", Some(Status::Idle)),
    ]);
    i.kids = vec![kid(7, 2, Some(Status::Working))];
    i.tabs.push(("t8".into(), "s1".into()));
    i.others = vec![OtherTab {
        tab: "t8".into(),
        name: "zsh".into(),
        cmd: Vec::new(),
        active: false,
        preview: false,
        zoomed: false,
    }];
    i
}

#[gpui::test]
fn working_circles_follow_status_at_every_depth(cx: &mut TestAppContext) {
    let selectors = [
        "ag-status-m1-working",
        "sub-status-ma-working",
        "sub-status-mb-working",
        "sub-status-mc-working",
    ];
    for status in [
        Some(Status::Working),
        Some(Status::Idle),
        Some(Status::Blocked),
        Some(Status::Error),
        Some(Status::Done),
        None,
    ] {
        for running in [true, false] {
            let (view, vcx, _) = open(cx);
            let mut i = input(vec![agent(1, "s1", status)]);
            i.agents[0].running = running;
            i.kids = vec![kid(10, 1, status), kid(11, 10, status), kid(12, 11, status)];
            for k in &mut i.kids {
                k.running = running;
            }
            feed(&view, vcx, i);
            for selector in selectors {
                assert_eq!(
                    vcx.debug_bounds(selector).is_some(),
                    running && status == Some(Status::Working),
                    "{status:?}, running={running}: {selector}",
                );
            }
        }
    }
}

#[gpui::test]
fn rows_focus_close_and_open_the_menu(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    feed(&view, vcx, sample());
    click(vcx, "ag-m1");
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::Focus {
            tab: "t1".into(),
            term: Some(id(1))
        }]
    );
    press(vcx, "ag-m2", MouseButton::Middle);
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::CloseTab { tab: "t2".into() }]
    );
    press(vcx, "ag-m2", MouseButton::Right);
    assert!(matches!(
        events(&seen).as_slice(),
        [AgentsEvent::Menu { tab: Some(tab), term: Some(term), .. }] if tab == "t2" && *term == id(2)
    ));
    click(vcx, "close-agent-m1");
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::CloseTab { tab: "t1".into() }]
    );
}

#[gpui::test]
fn sections_fold_and_show_counts(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    feed(&view, vcx, sample());
    let groups = read(&view, vcx, |v| labels(&v.view));
    assert_eq!(groups, vec![("Needs you".into(), 1), ("Working".into(), 1)]);
    click(vcx, "agents-group-sec-blocked");
    assert!(read(&view, vcx, |v| v.folded.contains("sec-blocked")));
    click(vcx, "agents-group-sec-blocked");
    assert!(!read(&view, vcx, |v| v.folded.contains("sec-blocked")));
    click(vcx, "agents-group-other-tabs");
    assert!(read(&view, vcx, |v| v.folded.contains(OTHER_TABS)));
}

#[gpui::test]
fn the_sort_menu_and_scope_switch_change_the_list(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    feed(&view, vcx, sample());
    click(vcx, "ag-sort");
    assert!(read(&view, vcx, |v| v.sort_menu.is_some()));
    click(vcx, "agents-sort-menu-1");
    assert_eq!(read(&view, vcx, |v| v.sort), Sort::Grouped);
    assert!(read(&view, vcx, |v| v.sort_menu.is_none()));
    assert_eq!(
        read(&view, vcx, |v| labels(&v.view)),
        vec![("Space s1".into(), 2)]
    );
    click(vcx, "scope-all");
    assert_eq!(read(&view, vcx, |v| v.scope), Scope::All);
    assert_eq!(
        read(&view, vcx, |v| labels(&v.view)),
        vec![("Space s1".into(), 2), ("Space s2".into(), 1)]
    );
    click(vcx, "scope-space");
    assert_eq!(read(&view, vcx, |v| v.scope), Scope::Space);
}

#[gpui::test]
fn the_jump_button_focuses_the_agent_that_needs_you(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    feed(&view, vcx, sample());
    click(vcx, "ag-next-needy");
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::Focus {
            tab: "t1".into(),
            term: Some(id(1))
        }]
    );
}

#[gpui::test]
fn hover_shows_the_pin_and_pinning_moves_the_row(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    feed(&view, vcx, sample());
    let at = center(vcx, "ag-m2");
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(read(&view, vcx, |v| v.hovered.clone()), Some("m2".into()));
    click(vcx, "pin-m2");
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::Pin {
            term: id(2),
            on: true
        }]
    );
    assert_eq!(read(&view, vcx, |v| v.view.groups[0].id.clone()), PINNED);
    assert_eq!(
        read(&view, vcx, |v| v.pins.get(&id(2)).map(|p| p.0)),
        Some(true)
    );
}

#[gpui::test]
fn sub_agents_fold_and_open_a_peek(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    feed(&view, vcx, sample());
    click(vcx, "sub-m7");
    assert_eq!(read(&view, vcx, |v| v.peek.clone()), Some(id(7)));
    assert!(vcx.debug_bounds("agents-peek-term").is_some());
    click(vcx, "agents-peek-close");
    assert!(read(&view, vcx, |v| v.peek.is_none()));
    click(vcx, "fold-m2");
    assert!(read(&view, vcx, |v| v.folded.contains("m2")));
    let mut gone = sample();
    gone.kids.clear();
    feed(&view, vcx, sample());
    vcx.update(|_, cx| view.update(cx, |v, _| v.peek = Some(id(7))));
    feed(&view, vcx, gone);
    assert!(read(&view, vcx, |v| v.peek.is_none()));
}

#[gpui::test]
fn a_peek_opens_the_sub_agent_in_a_pane_and_a_placed_one_focuses(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    feed(&view, vcx, sample());
    click(vcx, "sub-m7");
    events(&seen);
    click(vcx, "agents-peek-open");
    assert_eq!(events(&seen), vec![AgentsEvent::PopOut { term: id(7) }]);
    assert!(read(&view, vcx, |v| v.peek.is_none()));
    let mut placed = sample();
    for k in placed.kids.iter_mut().filter(|k| k.term == id(7)) {
        k.tab = Some("t2".into());
    }
    feed(&view, vcx, placed);
    click(vcx, "sub-m7");
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::Focus {
            tab: "t2".into(),
            term: Some(id(7))
        }]
    );
    assert!(read(&view, vcx, |v| v.peek.is_none()));
}

fn rename_box(view: &Entity<AgentsView>, vcx: &mut VisualTestContext) -> Entity<nixa_ui::text_input::TextInput> {
    read(view, vcx, |v| v.renaming.as_ref().and_then(|r| r.input.clone())).expect("rename box")
}

#[gpui::test]
fn the_rename_box_saves_a_new_name_only(cx: &mut TestAppContext) {
    for narrow in [false, true] {
        let (view, vcx, seen) = open(cx);
        let mut input = sample();
        input.narrow = narrow;
        feed(&view, vcx, input);
        vcx.update(|_, cx| view.update(cx, |v, cx| rename::open(v, "t8", id(8), "notes", cx)));
        vcx.run_until_parked();
        assert_eq!(vcx.debug_bounds("agents-rename-input").is_some(), narrow);
        assert_eq!(vcx.debug_bounds("agents-rename-inline").is_some(), !narrow);
        let input = rename_box(&view, vcx);
        assert_eq!(vcx.update(|_, cx| input.read(cx).value().to_string()), "notes");
        vcx.update(|_, cx| input.update(cx, |i, cx| i.set_value("  build box ", cx)));
        if narrow {
            click(vcx, "agents-rename-save");
        } else {
            vcx.simulate_keystrokes("enter");
            vcx.run_until_parked();
        }
        assert_eq!(
            events(&seen),
            vec![AgentsEvent::SetName {
                term: id(8),
                name: "build box".into()
            }]
        );
        assert!(read(&view, vcx, |v| v.renaming.is_none()));
        for typed in ["notes", "   "] {
            vcx.update(|_, cx| view.update(cx, |v, cx| rename::open(v, "t8", id(8), "notes", cx)));
            vcx.run_until_parked();
            let input = rename_box(&view, vcx);
            vcx.update(|_, cx| input.update(cx, |i, cx| i.set_value(typed, cx)));
            if narrow {
                click(vcx, "agents-rename-save");
            } else {
                vcx.simulate_keystrokes("enter");
                vcx.run_until_parked();
            }
            assert!(events(&seen).is_empty(), "{typed:?} must not rename");
            assert!(read(&view, vcx, |v| v.renaming.is_none()));
        }
        vcx.update(|_, cx| view.update(cx, |v, cx| rename::open(v, "t8", id(8), "notes", cx)));
        vcx.run_until_parked();
        if narrow {
            click(vcx, "agents-rename-cancel");
        } else {
            vcx.simulate_keystrokes("escape");
            vcx.run_until_parked();
        }
        assert!(read(&view, vcx, |v| v.renaming.is_none()));
        assert!(events(&seen).is_empty());
    }
}

#[test]
fn only_the_files_preview_tab_is_marked() {
    let other = |tab: &str| OtherTab {
        tab: tab.into(),
        name: tab.into(),
        cmd: Vec::new(),
        active: false,
        preview: true,
        zoomed: false,
    };
    let mut i = Input {
        others: vec![other("t8"), other("t9")],
        ..Input::default()
    };
    mark_previews(&mut i, Some("t9"));
    assert_eq!(
        i.others.iter().map(|o| o.preview).collect::<Vec<_>>(),
        vec![false, true]
    );
    mark_previews(&mut i, None);
    assert!(i.others.iter().all(|o| !o.preview));
}

#[gpui::test]
fn put_away_and_bring_back(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    feed(&view, vcx, sample());
    vcx.update(|_, cx| view.update(cx, |v, cx| put_away(v, "t8", cx)));
    vcx.run_until_parked();
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::PutAway {
            tab: "t8".into(),
            next: Some("t2".into())
        }]
    );
    assert!(read(&view, vcx, |v| v.away.contains("t8")));
    let mut i = sample();
    i.others.clear();
    i.away = vec![Away {
        tab: "t8".into(),
        name: "zsh".into(),
    }];
    feed(&view, vcx, i);
    click(vcx, "tabs-away");
    assert!(read(&view, vcx, |v| v.away_menu.is_some()));
    click(vcx, "agents-away-menu-0");
    assert!(!read(&view, vcx, |v| v.away.contains("t8")));
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::Focus {
            tab: "t8".into(),
            term: None
        }]
    );
    let mut stale = sample();
    stale.tabs.retain(|(t, _)| t != "t8");
    vcx.update(|_, cx| {
        view.update(cx, |v, _| {
            v.away.insert("t8".into());
        })
    });
    feed(&view, vcx, stale);
    assert!(!read(&view, vcx, |v| v.away.contains("t8")));
}

#[gpui::test]
fn other_tabs_open_rename_and_keep_previews(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    let mut i = sample();
    i.others.push(OtherTab {
        tab: "t9".into(),
        name: "notes.md".into(),
        cmd: Vec::new(),
        active: false,
        preview: true,
        zoomed: false,
    });
    i.loose = vec![Loose {
        term: id(9),
        name: "zsh".into(),
        cmd: Vec::new(),
    }];
    feed(&view, vcx, i);
    click(vcx, "tab-t8");
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::Focus {
            tab: "t8".into(),
            term: None
        }]
    );
    double(vcx, "tab-t8");
    assert!(events(&seen).contains(&AgentsEvent::Rename { tab: "t8".into() }));
    double(vcx, "tab-t9");
    assert!(events(&seen).contains(&AgentsEvent::KeepPreview { tab: "t9".into() }));
    click(vcx, "loose-m9");
    assert_eq!(events(&seen), vec![AgentsEvent::Adopt { term: id(9) }]);
}

#[gpui::test]
fn the_narrow_sidebar_shows_tiles(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    let mut i = sample();
    i.narrow = true;
    feed(&view, vcx, i);
    assert!(vcx.debug_bounds("ag-next-needy").is_none());
    let tile = vcx.debug_bounds("ag-m1").expect("tile drawn");
    assert_eq!(tile.size.width, gpui::px(52.));
    click(vcx, "ag-m2");
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::Focus {
            tab: "t2".into(),
            term: Some(id(2))
        }]
    );
    click(vcx, "agents-open-sidebar");
    assert_eq!(events(&seen), vec![AgentsEvent::ToggleSidebar]);
}

#[gpui::test]
fn commands_run_the_same_actions(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    let mut i = sample();
    i.agents[1].focused = true;
    feed(&view, vcx, i);
    assert_eq!(commands().len(), 10);
    vcx.update(|_, cx| run(&view, Action::NextAgent, cx));
    vcx.update(|_, cx| run(&view, Action::PrevAgent, cx));
    assert_eq!(
        events(&seen),
        vec![
            AgentsEvent::Focus {
                tab: "t1".into(),
                term: Some(id(1))
            },
            AgentsEvent::Focus {
                tab: "t1".into(),
                term: Some(id(1))
            }
        ]
    );
    vcx.update(|_, cx| run(&view, Action::SortGrouped, cx));
    assert_eq!(read(&view, vcx, |v| v.sort), Sort::Grouped);
    vcx.update(|_, cx| run(&view, Action::PinFocused, cx));
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::Pin {
            term: id(2),
            on: true
        }]
    );
    vcx.update(|_, cx| run(&view, Action::PutAwayFocused, cx));
    assert!(read(&view, vcx, |v| v.away.contains("t2")));
}

#[test]
fn a_pin_that_never_comes_back_is_dropped() {
    let mut pins = BTreeMap::new();
    pins.insert(id(1), (true, 1_000));
    pins.insert(id(2), (true, 1_000));
    pins.insert(id(3), (false, 1_000));
    let real = |t: &TermId| match t.number() {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    };
    let kept = settle_pins(&pins, real, 2_000);
    assert_eq!(kept.keys().cloned().collect::<Vec<_>>(), vec![id(2)]);
    assert!(settle_pins(&pins, real, 1_000 + PIN_WAIT_MS).is_empty());
}

#[gpui::test]
fn hover_is_forgotten_when_the_row_goes(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx);
    feed(&view, vcx, sample());
    let at = center(vcx, "ag-m2");
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(read(&view, vcx, |v| v.hovered.clone()), Some("m2".into()));
    let mut fewer = sample();
    fewer.agents.retain(|a| a.term != id(2));
    feed(&view, vcx, fewer);
    assert!(read(&view, vcx, |v| v.hovered.is_none()));
}

#[gpui::test]
fn putting_away_a_tab_of_another_space_picks_a_tab_of_that_space(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    let mut i = input(vec![
        agent(1, "s1", Some(Status::Working)),
        agent(2, "s2", Some(Status::Working)),
        agent(3, "s2", Some(Status::Idle)),
        agent(4, "s2", Some(Status::Idle)),
    ]);
    i.tabs.push(("t9".into(), "s2".into()));
    feed(&view, vcx, i);
    vcx.update(|_, cx| view.update(cx, |v, cx| run_put_away(v, "t3", cx)));
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::PutAway {
            tab: "t3".into(),
            next: Some("t4".into())
        }]
    );
    vcx.update(|_, cx| view.update(cx, |v, cx| run_put_away(v, "t9", cx)));
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::PutAway {
            tab: "t9".into(),
            next: Some("t4".into())
        }]
    );
    vcx.update(|_, cx| view.update(cx, |v, cx| run_put_away(v, "t1", cx)));
    assert_eq!(
        events(&seen),
        vec![AgentsEvent::PutAway {
            tab: "t1".into(),
            next: None
        }]
    );
}

fn run_put_away(v: &mut AgentsView, tab: &str, cx: &mut Context<AgentsView>) {
    put_away(v, tab, cx);
}

#[gpui::test]
fn narrow_other_tiles_open_the_menu_and_rename(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    let mut i = sample();
    i.narrow = true;
    feed(&view, vcx, i);
    press(vcx, "tab-t8", MouseButton::Right);
    assert!(matches!(
        events(&seen).as_slice(),
        [AgentsEvent::Menu { tab: Some(tab), term: None, .. }] if tab == "t8"
    ));
    double(vcx, "tab-t8");
    assert!(events(&seen).contains(&AgentsEvent::Rename { tab: "t8".into() }));
}

#[gpui::test]
fn sub_agent_and_loose_rows_quick_close(cx: &mut TestAppContext) {
    let (view, vcx, seen) = open(cx);
    let mut i = sample();
    i.loose = vec![Loose {
        term: id(9),
        name: "zsh".into(),
        cmd: Vec::new(),
    }];
    feed(&view, vcx, i.clone());

    // Sub-agent row: middle-click emits CloseTerm.
    press(vcx, "sub-m7", MouseButton::Middle);
    assert_eq!(events(&seen), vec![AgentsEvent::CloseTerm { term: id(7) }]);
    assert!(read(&view, vcx, |v| v.peek.is_none()));

    // Sub-agent row: hover reveals X button, survives sync, and clicking X emits CloseTerm only.
    assert!(vcx.debug_bounds("close-sub-m7").is_none());
    let sub_at = center(vcx, "sub-m7");
    vcx.simulate_mouse_move(sub_at, None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(read(&view, vcx, |v| v.hovered.clone()), Some("sub-m7".into()));
    let mut refreshed = i.clone();
    refreshed.agents[0].label = "agent 1 updated".into();
    feed(&view, vcx, refreshed);
    assert_eq!(read(&view, vcx, |v| v.hovered.clone()), Some("sub-m7".into()));
    click(vcx, "close-sub-m7");
    assert_eq!(events(&seen), vec![AgentsEvent::CloseTerm { term: id(7) }]);
    assert!(read(&view, vcx, |v| v.peek.is_none()));

    // Removing the sub-agent clears its hover state.
    let mut without_sub = i.clone();
    without_sub.kids.clear();
    feed(&view, vcx, without_sub);
    assert!(read(&view, vcx, |v| v.hovered.is_none()));

    // Loose terminal row: middle-click emits CloseTerm.
    press(vcx, "loose-m9", MouseButton::Middle);
    assert_eq!(events(&seen), vec![AgentsEvent::CloseTerm { term: id(9) }]);

    // Loose terminal row: hover reveals X button, survives sync, and clicking X emits CloseTerm (not Adopt).
    assert!(vcx.debug_bounds("close-loose-m9").is_none());
    let loose_at = center(vcx, "loose-m9");
    vcx.simulate_mouse_move(loose_at, None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(read(&view, vcx, |v| v.hovered.clone()), Some("loose-m9".into()));
    let mut refreshed_loose = i.clone();
    refreshed_loose.agents[0].label = "agent 1 again".into();
    feed(&view, vcx, refreshed_loose);
    assert_eq!(read(&view, vcx, |v| v.hovered.clone()), Some("loose-m9".into()));
    click(vcx, "close-loose-m9");
    assert_eq!(events(&seen), vec![AgentsEvent::CloseTerm { term: id(9) }]);

    // Removing the loose terminal clears its hover state.
    let mut without_loose = i;
    without_loose.loose.clear();
    feed(&view, vcx, without_loose);
    assert!(read(&view, vcx, |v| v.hovered.is_none()));
}
