use super::forms;
use super::order::{self, Row};
use super::{Counts, Drag, Form, SpacesEvent, SpacesView, place};
use crate::spaces::{self, Space};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, SharedString, Window, div, px,
};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::icon_button::{IconSize, icon_button};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon, theme};

const NAME_MAX: f32 = 150.;

fn row_height(tree: bool) -> f32 {
    if tree { 28. } else { 30. }
}

fn number_tip(n: usize) -> String {
    format!("Space {n}. Say \"space {n}\" to Nixa.")
}

pub fn lock_tip(locked: bool) -> &'static str {
    if locked {
        "Locked: Nixa starts no new work here. Click to open it."
    } else {
        "Open to work. Click to lock it."
    }
}

fn header(t: &Theme, narrow: bool, cx: &mut Context<SpacesView>) -> impl IntoElement + use<> {
    let buttons = div()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(
            icon_button(t, "space-new-folder", Icon::FolderPlus, IconSize::Toolbar, false)
                .debug_selector(|| "space-new-folder".into())
                .tooltip(tooltip("New space in another folder…"))
                .on_click(cx.listener(|_, _, _, cx| forms::pick_folder(cx))),
        )
        .child(
            icon_button(t, "space-new", Icon::Plus, IconSize::Toolbar, false)
                .debug_selector(|| "space-new".into())
                .tooltip(tooltip("New space"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SpacesEvent::New(None)))),
        );
    div()
        .h(px(31.))
        .flex_none()
        .flex()
        .items_center()
        .text_size(px(11.5))
        .text_color(t.dim)
        .map(|el| {
            if narrow {
                el.justify_center()
            } else {
                el.pl(px(16.)).pr(px(10.)).justify_between().child("Spaces")
            }
        })
        .child(buttons)
}

fn tail(t: &Theme, row: &Row, name: &str) -> AnyElement {
    let Some(g) = &row.git else {
        return div().flex_1().into_any_element();
    };
    let text = order::tail_text(g, row.tree.is_some(), name);
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .gap(px(5.))
        .justify_end()
        .items_center()
        .text_size(px(11.5))
        .text_color(t.dim)
        .when(!text.is_empty(), |el| el.child(div().min_w_0().truncate().child(text)))
        .when(g.ahead > 0, |el| {
            el.child(div().flex_none().text_color(t.green).child(format!("\u{2191}{}", g.ahead)))
        })
        .when(g.behind > 0, |el| {
            el.child(div().flex_none().text_color(t.red).child(format!("\u{2193}{}", g.behind)))
        })
        .into_any_element()
}

fn urgency(t: &Theme, id: &str, c: Option<Counts>, fill: Hsla) -> Option<AnyElement> {
    let c = c.filter(|c| c.blocked + c.done > 0)?;
    let pill = |text: String, color: Hsla| {
        div()
            .h(px(16.))
            .px(px(4.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color)
            .child(text)
    };
    let blocked_id = format!("space-blocked-{id}");
    let done_id = format!("space-done-{id}");
    Some(
        div()
            .flex_none()
            .flex()
            .gap(px(4.))
            .items_center()
            .when(c.blocked > 0, |el| {
                el.child(
                    pill(c.blocked.to_string(), t.red)
                        .bg(t.red.opacity(0.15))
                        .debug_selector(move || blocked_id.clone()),
                )
            })
            .when(c.done > 0, |el| {
                el.child(pill(c.done.to_string(), fill).debug_selector(move || done_id.clone()))
            })
            .into_any_element(),
    )
}

fn lock_button(t: &Theme, space: &Space, cx: &mut Context<SpacesView>) -> impl IntoElement + use<> {
    let locked = space.locked;
    let id = space.id.clone();
    let selector = format!("space-lock-{}", space.id);
    let hover = t.hover;
    div()
        .id(SharedString::from(format!("space-lock-{}", space.id)))
        .debug_selector(move || selector.clone())
        .size(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor_pointer()
        .opacity(if locked { 1. } else { 0.45 })
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(lock_tip(locked)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |v, _: &MouseUpEvent, _, cx| {
                cx.stop_propagation();
                v.drag = None;
                forms::toggle_lock(v, &id, cx);
                cx.notify();
            }),
        )
        .child(
            icon(if locked { Icon::Lock } else { Icon::Unlock })
                .size(px(13.))
                .text_color(if locked { t.yellow } else { t.dim }),
        )
}

fn drop_line(t: &Theme, top: bool) -> impl IntoElement + use<> {
    div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(2.))
        .bg(t.accent)
        .map(|el| if top { el.top_0() } else { el.bottom_0() })
}

fn full_row(v: &SpacesView, t: &Theme, row: &Row, cx: &mut Context<SpacesView>) -> AnyElement {
    let Some(space) = v.input.spaces.get(row.index).cloned() else {
        return div().into_any_element();
    };
    let on = v.input.active.as_deref() == Some(space.id.as_str());
    let fill = spaces::color(t, space.color);
    let n = row.index + 1;
    let dragging = v.drag.as_ref().is_some_and(|d| d.block == row.block);
    let target = v
        .drag
        .as_ref()
        .filter(|d| d.over == Some(row.block) && d.block != row.block);
    let line_bottom = target.is_some_and(|d| d.block < row.block && row.pos == row.end);
    let line_top = target.is_some_and(|d| d.block > row.block && row.pos == row.start);
    let renaming = match &v.form {
        Form::Rename { id, input } if *id == space.id => Some(input.clone()),
        _ => None,
    };
    let hover = t.hover;
    let selector = format!("space-row-{}", space.id);
    let (down_id, move_row, up_id, menu_id) = (space.id.clone(), row.clone(), space.id.clone(), space.id.clone());
    let tree = row.tree.is_some();
    let lead: AnyElement = if tree {
        div()
            .pl(px(22.))
            .flex_none()
            .child(icon(Icon::GitBranch).size(px(13.)).text_color(t.dim))
            .into_any_element()
    } else {
        div()
            .size(px(18.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(icon(spaces::shape(&space.id)).size(px(16.)).text_color(fill))
            .into_any_element()
    };
    let name: AnyElement = match renaming {
        Some(input) => div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(input)
            .into_any_element(),
        None => div()
            .flex_shrink_0()
            .max_w(px(NAME_MAX))
            .truncate()
            .text_color(t.foreground)
            .child(SharedString::from(space.name.clone()))
            .into_any_element(),
    };
    div()
        .id(SharedString::from(format!("space-row-{}", space.id)))
        .debug_selector(move || selector.clone())
        .relative()
        .px(px(8.))
        .h(px(row_height(tree)))
        .flex_none()
        .flex()
        .flex_col()
        .justify_center()
        .rounded(px(8.))
        .cursor_pointer()
        .when(on, |el| el.bg(t.selected))
        .when(!on, |el| el.hover(move |s| s.bg(hover)))
        .when(dragging, |el| el.opacity(0.5))
        .when(line_bottom, |el| el.child(drop_line(t, false)))
        .when(line_top, |el| el.child(drop_line(t, true)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |v, _: &MouseDownEvent, _, _| {
                if move_row.tree.is_some() || matches!(v.form, Form::Rename { .. }) {
                    return;
                }
                v.drag = Some(Drag {
                    id: down_id.clone(),
                    block: move_row.block,
                    over: Some(move_row.block),
                    first: move_row.first,
                    last: move_row.last,
                });
            }),
        )
        .on_mouse_move({
            let row = row.clone();
            cx.listener(move |v, e: &MouseMoveEvent, _, cx| drag_over(v, &row, e, cx))
        })
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |v, e: &MouseUpEvent, window, cx| drop_on(v, &up_id, e, window, cx)),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |_, e: &MouseDownEvent, _, cx| {
                cx.emit(SpacesEvent::Menu { id: menu_id.clone(), at: e.position })
            }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(9.))
                .child(lead)
                .child(
                    div()
                        .id(SharedString::from(format!("space-number-{}", space.id)))
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(t.magenta)
                        .tooltip(tooltip(number_tip(n)))
                        .child(n.to_string()),
                )
                .child(name)
                .child(tail(t, row, &space.name))
                .children(urgency(t, &space.id, v.input.counts.get(&space.id).copied(), fill))
                .child(lock_button(t, &space, cx)),
        )
        .into_any_element()
}

fn drag_over(v: &mut SpacesView, row: &Row, e: &MouseMoveEvent, cx: &mut Context<SpacesView>) {
    let Some(drag) = v.drag.as_mut() else {
        return;
    };
    if e.pressed_button != Some(MouseButton::Left) {
        v.drag = None;
        cx.notify();
        return;
    }
    if drag.over != Some(row.block) || drag.first != row.first || drag.last != row.last {
        drag.over = Some(row.block);
        drag.first = row.first;
        drag.last = row.last;
        cx.notify();
    }
}

fn drop_on(v: &mut SpacesView, id: &str, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<SpacesView>) {
    let drag = v.drag.take();
    if let Some(d) = drag.as_ref()
        && let Some(over) = d.over
        && over != d.block
    {
        if let Some(to) = order::drop_index(&v.input.spaces, &d.id, d.block < over, d.first, d.last) {
            let mut list = spaces::Spaces {
                spaces: std::mem::take(&mut v.input.spaces),
                active: None,
            };
            spaces::move_to(&mut list, &d.id, to);
            v.input.spaces = list.spaces;
            cx.emit(SpacesEvent::Move { id: d.id.clone(), to });
        }
        cx.notify();
        return;
    }
    if matches!(&v.form, Form::Rename { id: r, .. } if r == id) {
        return;
    }
    if e.click_count >= 2 && !v.input.narrow {
        forms::start_rename(v, id, window, cx);
    } else {
        cx.emit(SpacesEvent::Select(id.to_string()));
        v.input.active = Some(id.to_string());
    }
    cx.notify();
}

fn tile(v: &SpacesView, t: &Theme, row: &Row, cx: &mut Context<SpacesView>) -> AnyElement {
    let Some(space) = v.input.spaces.get(row.index).cloned() else {
        return div().into_any_element();
    };
    let on = v.input.active.as_deref() == Some(space.id.as_str());
    let fill = spaces::color(t, space.color);
    let c = v.input.counts.get(&space.id).copied().unwrap_or_default();
    let branch = row
        .git
        .as_ref()
        .and_then(|g| g.branch.clone())
        .filter(|b| *b != space.name);
    let tip: Vec<String> = [
        Some(format!("{}. {}", row.index + 1, space.name)),
        row.tree.map(|_| "worktree".to_string()),
        branch,
        space.locked.then(|| "locked".to_string()),
        (c.blocked > 0).then(|| format!("{} need you", c.blocked)),
        (c.done > 0).then(|| format!("{} done", c.done)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let corner: Option<AnyElement> = if c.blocked > 0 {
        Some(
            div()
                .absolute()
                .top(px(2.))
                .right(px(2.))
                .h(px(14.))
                .min_w(px(14.))
                .px(px(3.))
                .rounded_full()
                .bg(t.red)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(9.5))
                .font_weight(FontWeight::BOLD)
                .text_color(t.background)
                .child(c.blocked.to_string())
                .into_any_element(),
        )
    } else if space.locked {
        Some(
            div()
                .absolute()
                .top(px(3.))
                .right(px(3.))
                .child(icon(Icon::Lock).size(px(10.)).text_color(t.yellow))
                .into_any_element(),
        )
    } else if c.done > 0 {
        Some(
            div()
                .absolute()
                .top(px(4.))
                .right(px(4.))
                .size(px(6.))
                .rounded_full()
                .bg(fill)
                .into_any_element(),
        )
    } else {
        None
    };
    let id = space.id.clone();
    let selector = format!("space-tile-{}", space.id);
    div()
        .w_full()
        .flex_none()
        .py(px(3.))
        .flex()
        .justify_center()
        .child(
            div()
                .id(SharedString::from(format!("space-tile-{}", space.id)))
                .debug_selector(move || selector.clone())
                .relative()
                .w(px(52.))
                .h(px(38.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9.))
                .border_1()
                .border_color(if on { fill } else { fill.opacity(0.25) })
                .bg(if on { fill.opacity(0.2) } else { t.surface })
                .cursor_pointer()
                .tooltip(tooltip(tip.join(" \u{b7} ")))
                .hover(move |s| s.bg(fill.opacity(0.15)).border_color(fill))
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.input.active = Some(id.clone());
                    cx.emit(SpacesEvent::Select(id.clone()));
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::BOLD)
                        .whitespace_nowrap()
                        .text_color(if on { t.foreground } else { fill })
                        .child(spaces::short_name(&space)),
                )
                .children(corner),
        )
        .into_any_element()
}

fn small_button(
    t: &Theme,
    id: &'static str,
    label: &'static str,
    kind: ButtonKind,
    cx: &mut Context<SpacesView>,
    act: impl Fn(&mut SpacesView, &mut Context<SpacesView>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    button(t, id, label, kind)
        .debug_selector(move || id.to_string())
        .on_click(cx.listener(move |v, _, _, cx| {
            act(v, cx);
            cx.notify();
        }))
}

fn form_box(t: &Theme) -> gpui::Div {
    div()
        .mx(px(4.))
        .my(px(3.))
        .p(px(8.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .rounded(px(8.))
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .text_size(px(12.))
        .text_color(t.foreground)
}

fn form(v: &SpacesView, t: &Theme, id: &str, cx: &mut Context<SpacesView>) -> Option<AnyElement> {
    let name = v.input.spaces.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_default();
    let cancel = |cx: &mut Context<SpacesView>| {
        small_button(t, "space-form-cancel", "Cancel", ButtonKind::Ghost, cx, |v, _| super::set_form(v, Form::None, None))
    };
    let actions = || div().flex().gap(px(6.)).justify_end();
    match &v.form {
        Form::Branch { id: f, input, busy } if f == id => Some(
            form_box(t)
                .child(div().text_color(t.dim).child(format!("New worktree from {name}")))
                .child(div().flex().flex_col().child(input.clone()))
                .child(
                    actions()
                        .child(cancel(cx))
                        .child(small_button(
                            t,
                            "space-branch-add",
                            if *busy { "Making…" } else { "Make worktree" },
                            ButtonKind::Primary,
                            cx,
                            forms::submit_branch,
                        )),
                )
                .into_any_element(),
        ),
        Form::Close { id: f, tabs } if f == id => Some(
            form_box(t)
                .child(format!(
                    "Close space {name} and its {tabs} {}?",
                    if *tabs == 1 { "tab" } else { "tabs" }
                ))
                .child(
                    actions()
                        .child(cancel(cx))
                        .child(small_button(t, "space-close-yes", "Close", ButtonKind::Danger, cx, forms::confirm_close)),
                )
                .into_any_element(),
        ),
        Form::Remove { id: f, busy } if f == id => Some(
            form_box(t)
                .child(format!("Remove worktree {name}? Its folder is deleted. Git refuses when it has changes."))
                .child(
                    actions().child(cancel(cx)).child(small_button(
                        t,
                        "space-remove-yes",
                        if *busy { "Removing…" } else { "Remove" },
                        ButtonKind::Danger,
                        cx,
                        forms::confirm_remove,
                    )),
                )
                .into_any_element(),
        ),
        Form::Open { id: f, list } if f == id => {
            let hover = t.hover;
            let rows: Vec<AnyElement> = list
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    let label = w.branch.clone().unwrap_or_else(|| spaces::folder_name(&w.path));
                    let home = spaces::home();
                    let short = spaces::short_folder(&w.path, &home);
                    let selector = format!("space-open-{i}");
                    div()
                        .id(("space-open", i))
                        .debug_selector(move || selector.clone())
                        .h(px(26.))
                        .px(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |v, _, _, cx| {
                            forms::open_worktree(v, i, cx);
                            cx.notify();
                        }))
                        .child(icon(Icon::GitBranch).size(px(12.)).text_color(t.dim))
                        .child(div().flex_none().child(label))
                        .child(div().flex_1().min_w_0().truncate().text_color(t.dim).child(short))
                        .into_any_element()
                })
                .collect();
            Some(
                form_box(t)
                    .child(div().text_color(t.dim).child("Open a worktree as a space"))
                    .children(rows)
                    .child(actions().child(cancel(cx)))
                    .into_any_element(),
            )
        }
        _ => None,
    }
}

impl gpui::Render for SpacesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let narrow = self.input.narrow;
        let rows = order::ordered(&self.input.spaces, |s| place(self, s), &self.checks.git, &self.shown);
        let mut list: Vec<AnyElement> = Vec::new();
        for row in &rows {
            if narrow {
                list.push(tile(self, &t, row, cx));
                continue;
            }
            list.push(full_row(self, &t, row, cx));
            if let Some(f) = form(self, &t, &row.id, cx) {
                list.push(f);
            }
        }
        if self.shown.len() < self.input.spaces.len() {
            list.push(div().debug_selector(|| "spaces-loading".into()).flex_none()
                .px(px(6.)).py(px(5.)).text_color(t.dim).child("Loading…").into_any_element());
        }
        div()
            .id("spaces")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.chrome)
            .text_size(px(13.))
            .child(header(&t, narrow, cx))
            .child(
                nixa_ui::scroll::scroll_area(&t, "spaces-list", &self.scroll, div()
                    .px(px(if narrow { 6. } else { 8. }))
                    .pb(px(14.))
                    .flex()
                    .flex_col()
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|v, _: &MouseUpEvent, _, cx| {
                            if v.drag.take().is_some() {
                                cx.notify();
                            }
                        }),
                    )
                    .children(list),
                ).flex_1(),
            )
    }
}
