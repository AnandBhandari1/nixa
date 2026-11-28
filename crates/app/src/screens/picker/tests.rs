use super::rows::{self, PickAgent, PickSpace, PickTab, Row, Snapshot, Target};
use super::view::{Picker, PickerEvent};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use nixa_protocol::{Status, TermId};
use std::cell::RefCell;
use std::rc::Rc;

fn tab(id: &str, name: &str, status: Option<Status>, active: bool) -> PickTab {
    PickTab {
        id: id.into(),
        name: name.into(),
        active,
        status,
        agents: vec![name.into()],
        team: 0,
    }
}

fn space(id: &str, name: &str, active: bool, tabs: Vec<PickTab>) -> PickSpace {
    PickSpace {
        id: id.into(),
        name: name.into(),
        color: gpui::red(),
        active,
        tabs,
    }
}

fn agent(n: u64, tab: &str, name: &str, status: Option<Status>) -> PickAgent {
    PickAgent {
        term: TermId::from_number(n),
        tab: tab.into(),
        name: name.into(),
        title: String::new(),
        status,
        pinned: false,
        focused: false,
        space_name: "shop".into(),
        tab_name: name.into(),
    }
}

fn sample() -> Snapshot {
    Snapshot {
        spaces: vec![
            space("s1", "home", true, vec![tab("t1", "notes", None, true)]),
            space(
                "s2",
                "shop",
                false,
                vec![
                    tab("t2", "claude", Some(Status::Working), false),
                    tab("t3", "codex", Some(Status::Idle), true),
                ],
            ),
            space(
                "s3",
                "blog",
                false,
                vec![tab("t4", "omp", Some(Status::Blocked), true)],
            ),
            space("s4", "empty", false, vec![]),
        ],
        agents: vec![
            agent(2, "t2", "claude", Some(Status::Working)),
            agent(4, "t4", "omp", Some(Status::Blocked)),
        ],
    }
}

#[test]
fn busy_spaces_come_first_with_blocked_on_top() {
    let list = rows::rows(&sample(), "", false);
    assert_eq!(
        list,
        vec![
            Row::Head(2),
            Row::Tab(2, 0),
            Row::Head(1),
            Row::Tab(1, 0),
            Row::Tab(1, 1),
            Row::Head(3),
            Row::NewTabHere(3),
            Row::Quiet(1),
            Row::Agents,
            Row::Agent(0),
            Row::Agent(1),
            Row::NewTab,
            Row::NewSpace,
        ]
    );
    assert_eq!(rows::quiet_label(1, false), "earlier: 1 quiet space ▾");
    assert_eq!(rows::quiet_label(3, true), "earlier: 3 quiet spaces ▴");
}

#[test]
fn the_fold_opens_and_a_filter_shows_quiet_spaces() {
    let open = rows::rows(&sample(), "", true);
    assert!(open.contains(&Row::Head(0)) && open.contains(&Row::Tab(0, 0)));
    let found = rows::rows(&sample(), "not", false);
    assert_eq!(
        found,
        vec![
            Row::Quiet(1),
            Row::Head(0),
            Row::Tab(0, 0),
            Row::NewTab,
            Row::NewSpace
        ]
    );
    let by_space = rows::rows(&sample(), "shop cod", false);
    assert_eq!(
        by_space,
        vec![Row::Head(1), Row::Tab(1, 1), Row::NewTab, Row::NewSpace]
    );
}

#[test]
fn start_is_the_active_tab_or_the_active_head() {
    let snap = sample();
    let picks = rows::pickable(&rows::rows(&snap, "", true));
    assert!(
        !picks
            .iter()
            .any(|r| matches!(r, Row::Agents))
    );
    assert_eq!(picks[rows::start(&snap, &picks)], Row::Tab(0, 0));
    let mut no_tab = snap.clone();
    no_tab.spaces[0].tabs[0].active = false;
    let picks = rows::pickable(&rows::rows(&no_tab, "", true));
    assert_eq!(picks[rows::start(&no_tab, &picks)], Row::Head(0));
    assert_eq!(
        rows::target(&snap, &Row::Agent(1)),
        Some(Target::Agent {
            tab: "t4".into(),
            term: TermId::from_number(4)
        })
    );
    assert!(picks.contains(&Row::Quiet(1)));
    assert_eq!(rows::target(&snap, &Row::Quiet(1)), None);
    assert_eq!(rows::target(&snap, &Row::NewTabHere(3)), Some(Target::NewTabHere("s4".into())));
    assert!(rows::rows(&snap, "empty", false).contains(&Row::NewTabHere(3)));
    assert!(rows::busy(Some(Status::Idle)));
}

#[test]
fn a_stale_row_has_no_target() {
    let snap = sample();
    let mut small = snap.clone();
    small.spaces.truncate(1);
    small.agents.clear();
    assert_eq!(rows::target(&small, &Row::Head(2)), None);
    assert_eq!(rows::target(&small, &Row::Tab(1, 1)), None);
    assert_eq!(rows::target(&small, &Row::Tab(0, 5)), None);
    assert_eq!(rows::target(&small, &Row::Agent(0)), None);
    assert_eq!(
        rows::target(&small, &Row::Tab(0, 0)),
        Some(Target::Tab("t1".into()))
    );
}

fn term(cmd: &[&str], running: bool, status: Option<&str>) -> nixa_protocol::TermInfo {
    serde_json::from_value(serde_json::json!({
        "id": "m9", "name": "", "cmd": cmd, "cwd": "/tmp", "running": running,
        "status": status, "rows": 24, "cols": 80, "created_ms": 1
    }))
    .expect("term")
}

#[test]
fn agents_are_live_programs_or_shells_with_a_status() {
    use super::data::is_agent;
    assert!(is_agent(&term(&["claude"], true, None)));
    assert!(!is_agent(&term(&["claude"], false, Some("working"))));
    assert!(!is_agent(&term(&["/bin/zsh"], true, None)));
    assert!(is_agent(&term(&["/bin/zsh"], true, Some("blocked"))));
    assert!(!is_agent(&term(&[], false, Some("blocked"))));
}

#[gpui::test]
fn a_live_idle_pane_keeps_its_space_above_earlier(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let (workspace, cx) = cx.add_window_view(crate::workspace::Workspace::new);
    workspace.update(cx, |ws, cx| {
        ws.spaces.spaces = vec![crate::spaces::Space {
            id: "s1".into(),
            name: "Split".into(),
            ..Default::default()
        }];
        let ended = term(&["agent"], false, Some("error"));
        let mut idle = term(&["agent"], true, Some("idle"));
        idle.id = TermId::from_number(10);
        for pane in [ended, idle] {
            crate::tabs::place(&mut ws.tabs, pane.id.clone(), "s1", "t1");
            ws.terms.insert(pane.id.clone(), pane);
        }

        let snap = super::data::snapshot(ws, cx);
        assert_eq!(snap.spaces[0].tabs[0].status, Some(Status::Idle));
        let list = rows::rows(&snap, "", false);
        assert!(list.starts_with(&[Row::Head(0), Row::Tab(0, 0)]));
        assert!(!list.iter().any(|row| matches!(row, Row::Quiet(_))));
    });
}

type Seen = Rc<RefCell<Vec<PickerEvent>>>;

fn open(cx: &mut TestAppContext, snap: Snapshot) -> (Entity<Picker>, &mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let holder: Rc<RefCell<Option<Entity<Picker>>>> = Rc::new(RefCell::new(None));
    let slot = holder.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let picker = super::view::new(snap, window, cx);
        *slot.borrow_mut() = Some(picker.clone());
        Host { picker }
    });
    let picker = holder.borrow().clone().expect("picker");
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&picker, move |_, e: &PickerEvent, _| {
            log.borrow_mut().push(e.clone())
        })
        .detach();
    });
    vcx.run_until_parked();
    (picker, vcx, seen)
}

struct Host {
    picker: Entity<Picker>,
}

impl gpui::Render for Host {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        gpui::div()
            .size_full()
            .relative()
            .child(self.picker.clone())
    }
}

fn events(seen: &Seen) -> Vec<PickerEvent> {
    std::mem::take(&mut *seen.borrow_mut())
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.update(|window, app| window.draw(app).clear());
    let bounds = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn enter_goes_to_the_current_tab_and_arrows_move(cx: &mut TestAppContext) {
    let (picker, vcx, seen) = open(cx, sample());
    picker.update(vcx, |p, cx| {
        p.show_quiet = true;
        cx.notify();
    });
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        events(&seen),
        vec![PickerEvent::Go(Target::Tab("t1".into()))]
    );
    vcx.simulate_keystrokes("down down");
    assert_eq!(
        vcx.update(|_, cx| picker.read(cx).selected()),
        Some(Row::Agent(1))
    );
    vcx.simulate_keystrokes("up up up");
    assert_eq!(
        vcx.update(|_, cx| picker.read(cx).selected()),
        Some(Row::Head(0))
    );
    vcx.simulate_keystrokes("down down down down down");
    assert_eq!(
        vcx.update(|_, cx| picker.read(cx).selected()),
        Some(Row::NewSpace)
    );
    vcx.simulate_keystrokes("down enter");
    assert_eq!(
        events(&seen),
        vec![PickerEvent::Go(Target::Space("s3".into()))]
    );
}

#[gpui::test]
fn typing_filters_and_enter_picks_the_first_match(cx: &mut TestAppContext) {
    let (picker, vcx, seen) = open(cx, sample());
    vcx.simulate_input("codex");
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| picker.read(cx).filter.clone()), "codex");
    vcx.simulate_keystrokes("down enter");
    assert_eq!(
        events(&seen),
        vec![PickerEvent::Go(Target::Tab("t3".into()))]
    );
}

#[gpui::test]
fn ctrl_d_closes_the_selected_tab_and_escape_closes(cx: &mut TestAppContext) {
    let (_picker, vcx, seen) = open(cx, sample());
    vcx.simulate_keystrokes("down ctrl-d");
    assert_eq!(events(&seen), vec![PickerEvent::CloseTab("t4".into())]);
    vcx.simulate_keystrokes("up ctrl-d");
    assert!(events(&seen).is_empty());
    vcx.simulate_keystrokes("escape");
    assert!(events(&seen).contains(&PickerEvent::Close));
}

#[gpui::test]
fn clicks_open_the_fold_and_pick_rows(cx: &mut TestAppContext) {
    let mut snap = sample();
    snap.spaces.pop();
    let (picker, vcx, seen) = open(cx, snap);
    click(vcx, "pk-agent-m4");
    assert_eq!(
        events(&seen),
        vec![PickerEvent::Go(Target::Agent {
            tab: "t4".into(),
            term: TermId::from_number(4)
        })]
    );
    click(vcx, "pk-head-s3");
    assert_eq!(
        events(&seen),
        vec![PickerEvent::Go(Target::Space("s3".into()))]
    );
    click(vcx, "pk-earlier");
    assert!(vcx.update(|_, cx| picker.read(cx).show_quiet));
    assert!(events(&seen).is_empty());
    click(vcx, "pk-t1");
    assert_eq!(
        events(&seen),
        vec![PickerEvent::Go(Target::Tab("t1".into()))]
    );
    click(vcx, "pk-close");
    assert_eq!(events(&seen), vec![PickerEvent::Close]);
}

#[gpui::test]
fn keyboard_can_open_quiet_spaces(cx: &mut TestAppContext) {
    let (picker, vcx, seen) = open(cx, sample());
    picker.update(vcx, |p, cx| { p.at = Some(7); cx.notify(); });
    assert_eq!(vcx.update(|_, cx| picker.read(cx).selected()), Some(Row::Quiet(1)));
    vcx.simulate_keystrokes("enter");
    assert!(vcx.update(|_, cx| picker.read(cx).show_quiet));
    assert!(events(&seen).is_empty());
}
