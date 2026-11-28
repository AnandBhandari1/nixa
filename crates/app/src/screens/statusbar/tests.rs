use super::*;
use gpui::{Context, IntoElement, Modifiers, Render, TestAppContext, VisualTestContext};
use std::cell::RefCell;

struct Host {
    status: Status,
    seen: Rc<RefCell<Vec<StatusAction>>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = nixa_ui::theme(cx).clone();
        let seen = self.seen.clone();
        let on: OnStatus = Rc::new(move |a, _, _| seen.borrow_mut().push(a));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1())
            .child(render(&self.status, &t, on))
    }
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
fn clicks_send_actions(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let t = cx.update(|app| nixa_ui::theme(app).clone());
    let status = Status {
        engine: Some(engine_dot(&t, Some(2), false, None)),
        brain: Some(Dot {
            color: t.green,
            text: "Brain: on".into(),
            detail: Some("pid 9".into()),
        }),
        notice: Some("Saved".into()),
        folder: "~/p".into(),
        branch: Some("main".into()),
        tasks_waiting: 2,
        events_unseen: 0,
    };
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let (_, cx) = cx.add_window_view(move |_, _| Host { status, seen: sink });
    click(cx, "status-brain");
    click(cx, "status-engine");
    click(cx, "status-tasks");
    click(cx, "status-notice");
    click(cx, "status-commands");
    redraw(cx);
    assert!(cx.debug_bounds("status-branch").is_some());
    assert_eq!(
        *seen.borrow(),
        vec![
            StatusAction::Detail("pid 9".into()),
            StatusAction::Tasks,
            StatusAction::DismissNotice,
            StatusAction::Commands,
        ]
    );
}

#[gpui::test]
fn engine_words(cx: &mut TestAppContext) {
    cx.update(nixa_ui::init);
    let t = cx.update(|app| nixa_ui::theme(app).clone());
    assert_eq!(
        engine_dot(&t, Some(1), false, None).text,
        "Engine: on, 1 terminal"
    );
    assert_eq!(
        engine_dot(&t, Some(3), false, None).text,
        "Engine: on, 3 terminals"
    );
    assert_eq!(engine_dot(&t, None, true, None).text, "Engine: connecting");
    assert_eq!(
        engine_dot(&t, None, false, None).text,
        "Engine: not running"
    );
    let down = engine_dot(&t, None, false, Some((t.yellow, "Engine: starting".into())));
    assert_eq!(down.text, "Engine: starting");
    assert_eq!(down.color, t.yellow);
}

#[test]
fn branch_from_head_and_worktree_file() {
    let dir = std::env::temp_dir().join(format!("nixa-status-git-{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(dir.join("repo/.git")).unwrap();
    fs::create_dir_all(dir.join("repo/deep/er")).unwrap();
    fs::write(dir.join("repo/.git/HEAD"), "ref: refs/heads/dev_x\n").unwrap();
    assert_eq!(
        branch_of(&dir.join("repo/deep/er").display().to_string()),
        Some("dev_x".into())
    );
    fs::create_dir_all(dir.join("wt-git")).unwrap();
    fs::write(dir.join("wt-git/HEAD"), "0123456789abcdef\n").unwrap();
    fs::create_dir_all(dir.join("wt")).unwrap();
    fs::write(
        dir.join("wt/.git"),
        format!("gitdir: {}\n", dir.join("wt-git").display()),
    )
    .unwrap();
    assert_eq!(
        branch_of(&dir.join("wt").display().to_string()),
        Some("0123456".into())
    );
    fs::write(dir.join("wt-git/HEAD"), "é€ŧ¥ü¨ˆøπ\n").unwrap();
    assert_eq!(branch_of(&dir.join("wt").display().to_string()), None);
    fs::remove_dir_all(&dir).ok();
    assert_eq!(branch_of("/"), None);
}
