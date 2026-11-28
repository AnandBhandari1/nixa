use super::model::{self, Entry, Line, Loose, OTHER_TABS, OtherTab, Scope, Sort, State, SubRow};
use super::{AgentsEvent, AgentsView, pin, toggle_fold};
use crate::panes::{ASSISTANT, number_tip};
use crate::terms;
use crate::spaces::color as space_color;
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, Hsla, MouseButton, MouseDownEvent, SharedString,
    div, px,
};
use nixa_protocol::{Status, TermId};
use nixa_ui::chip::{chip, status_chip, status_dot};
use nixa_ui::popover::tooltip;
use nixa_ui::theme::status_color;
use nixa_ui::{Icon, Theme, icon};

fn sid(prefix: &str, key: &impl std::fmt::Display) -> SharedString {
    SharedString::from(format!("{prefix}-{key}"))
}

pub fn mark(
    t: &Theme,
    id: SharedString,
    status: Option<Status>,
    running: bool,
    compact: bool,
) -> AnyElement {
    if !running {
        return chip("Ended", t.dim).into_any_element();
    }
    match status {
        Some(s @ (Status::Blocked | Status::Error)) if compact => {
            let color = status_color(t, s);
            div()
                .flex_none()
                .size(px(14.))
                .rounded(px(4.))
                .bg(color.opacity(0.15))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.5))
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child("!")
                .into_any_element()
        }
        Some(s @ (Status::Blocked | Status::Error)) => status_chip(t, s).into_any_element(),
        Some(Status::Working) => {
            let selector = format!("{id}-working");
            div()
                .id(id.clone())
                .debug_selector(move || selector)
                .flex_none()
                .child(status_dot(t, id, Status::Working))
                .into_any_element()
        }
        Some(Status::Done) => status_dot(t, id, Status::Done),
        _ => status_dot(t, id, Status::Idle),
    }
}

fn state_status(state: State) -> Status {
    match state {
        State::Blocked => Status::Blocked,
        State::Done => Status::Done,
        State::Working => Status::Working,
        State::Idle => Status::Idle,
    }
}

fn state_color(t: &Theme, state: Option<State>) -> Hsla {
    state.map_or(t.dim, |s| status_color(t, state_status(s)))
}

pub fn render(
    v: &AgentsView,
    t: &Theme,
    now: u64,
    cx: &mut Context<AgentsView>,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    if v.input.narrow {
        for (i, e) in v.view.flat.iter().enumerate() {
            rows.push(tile(v, t, e, i, cx));
        }
        for o in &v.input.others {
            rows.push(other_tile(v, t, o, cx));
        }
        if !v.input.away.is_empty() {
            rows.push(away_row(v, t, cx));
        }
        return rows;
    }
    for g in &v.view.groups {
        let head = Head {
            id: g.id.clone(),
            label: g.label.clone(),
            state: g.state,
            count: g.members.len(),
            top: g.top(),
        };
        rows.push(header(v, t, head, cx));
        if v.folded.contains(&g.id) {
            continue;
        }
        for e in &g.members {
            rows.push(agent_row(v, t, e, now, cx));
            if v.folded.contains(&e.agent.term.to_string()) {
                continue;
            }
            for s in &e.subs {
                rows.push(sub_row(v, t, s, e, cx));
            }
        }
    }
    let others = v.input.others.len() + v.input.loose.len();
    if others > 0 {
        let head = Head {
            id: OTHER_TABS.into(),
            label: "Other tabs".into(),
            state: None,
            count: others,
            top: None,
        };
        rows.push(header(v, t, head, cx));
        if !v.folded.contains(OTHER_TABS) {
            for o in &v.input.others {
                rows.push(other_row(v, t, o, cx));
            }
            for l in &v.input.loose {
                rows.push(loose_row(v, t, l, cx));
            }
        }
    }
    if !v.input.away.is_empty() {
        rows.push(away_row(v, t, cx));
    }
    rows
}

struct Head {
    id: String,
    label: String,
    state: Option<State>,
    count: usize,
    top: Option<State>,
}

fn header(v: &AgentsView, t: &Theme, head: Head, cx: &mut Context<AgentsView>) -> AnyElement {
    let Head {
        id,
        label,
        state,
        count,
        top,
    } = head;
    let open = !v.folded.contains(&id);
    let priority = v.sort == Sort::Priority;
    let bright = t.bright;
    let key = id.clone();
    let selector = format!("agents-group-{id}");
    div()
        .id(sid("agents-group", &id))
        .debug_selector(move || selector)
        .px(px(8.))
        .pt(px(8.))
        .h(px(30.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |s| s.text_color(bright))
        .on_click(cx.listener(move |v, _, _, cx| toggle_fold(v, &key, cx)))
        .child(
            div()
                .truncate()
                .text_color(state_color(t, state))
                .child(label),
        )
        .when(priority || !open, |el| {
            el.child(div().text_color(t.dim).child(count.to_string()))
        })
        .child(div().flex_1())
        .when(!open, |el| {
            el.when_some(top.filter(|s| !priority && *s != State::Idle), |el, s| {
                el.child(mark(
                    t,
                    sid("agents-group-mark", &id),
                    Some(state_status(s)),
                    true,
                    true,
                ))
            })
            .child(icon(Icon::ChevronRight).size(px(11.)).text_color(t.dim))
        })
        .into_any_element()
}

fn hover_listener(
    key: String,
    cx: &mut Context<AgentsView>,
) -> impl Fn(&bool, &mut gpui::Window, &mut gpui::App) + 'static {
    cx.listener(move |v, on: &bool, _, cx| {
        if *on {
            v.hovered = Some(key.clone());
        } else if v.hovered.as_deref() == Some(key.as_str()) {
            v.hovered = None;
        }
        cx.notify();
    })
}

fn x_button(
    t: &Theme,
    id: SharedString,
    tip: &'static str,
    size: f32,
    event: AgentsEvent,
    cx: &mut Context<AgentsView>,
) -> AnyElement {
    let hover = t.hover;
    let bright = t.bright;
    let selector = id.to_string();
    div()
        .id(id)
        .debug_selector(move || selector)
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_color(t.dim)
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(bright))
        .tooltip(tooltip(tip))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |_, _, _, cx| {
            cx.stop_propagation();
            cx.emit(event.clone());
        }))
        .child(
            icon(Icon::X)
                .size(px(if size < 16. { 10. } else { 12. }))
                .text_color(t.dim),
        )
        .into_any_element()
}

fn close_button(
    t: &Theme,
    id: SharedString,
    tip: &'static str,
    size: f32,
    tab: String,
    cx: &mut Context<AgentsView>,
) -> AnyElement {
    x_button(t, id, tip, size, AgentsEvent::CloseTab { tab }, cx)
}

fn menu_on(
    tab: Option<String>,
    space: Option<String>,
    term: Option<TermId>,
    cx: &mut Context<AgentsView>,
) -> impl Fn(&MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    cx.listener(move |_, e: &MouseDownEvent, _, cx| {
        cx.stop_propagation();
        cx.emit(AgentsEvent::Menu {
            tab: tab.clone(),
            space: space.clone(),
            term: term.clone(),
            at: e.position,
        });
    })
}

fn middle_close(
    tab: String,
    cx: &mut Context<AgentsView>,
) -> impl Fn(&MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    cx.listener(move |_, _: &MouseDownEvent, _, cx| {
        cx.stop_propagation();
        cx.emit(AgentsEvent::CloseTab { tab: tab.clone() });
    })
}

fn middle_close_term(
    term: TermId,
    cx: &mut Context<AgentsView>,
) -> impl Fn(&MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    cx.listener(move |_, _: &MouseDownEvent, _, cx| {
        cx.stop_propagation();
        cx.emit(AgentsEvent::CloseTerm { term: term.clone() });
    })
}

fn close_term_button(
    t: &Theme,
    id: SharedString,
    tip: &'static str,
    size: f32,
    term: TermId,
    cx: &mut Context<AgentsView>,
) -> AnyElement {
    x_button(t, id, tip, size, AgentsEvent::CloseTerm { term }, cx)
}

fn fold_chip(v: &AgentsView, t: &Theme, e: &Entry, cx: &mut Context<AgentsView>) -> AnyElement {
    let key = e.agent.term.to_string();
    let open = !v.folded.contains(&key);
    let top = e
        .subs
        .iter()
        .map(|s| model::kid_state(s.kid.status, s.kid.running))
        .max();
    let hover = t.hover;
    let bright = t.bright;
    let color = if open {
        t.dim
    } else {
        state_color(t, top.filter(|s| *s != State::Idle))
    };
    let selector = format!("fold-{key}");
    div()
        .id(sid("fold", &key))
        .debug_selector(move || selector)
        .h(px(18.))
        .px(px(3.))
        .gap(px(3.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .text_size(px(11.))
        .text_color(color)
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(bright))
        .tooltip(tooltip(if open {
            "Fold the sub-agents"
        } else {
            "Show the sub-agents"
        }))
        .on_click(cx.listener(move |v, _, _, cx| {
            cx.stop_propagation();
            toggle_fold(v, &key, cx);
        }))
        .child(
            icon(if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            })
            .size(px(11.))
            .text_color(color),
        )
        .when(!open, |el| el.child(e.subs.len().to_string()))
        .into_any_element()
}

fn pin_button(t: &Theme, term: TermId, pinned: bool, cx: &mut Context<AgentsView>) -> AnyElement {
    let hover = t.hover;
    let bright = t.bright;
    let selector = format!("pin-{term}");
    div()
        .id(sid("pin", &term))
        .debug_selector(move || selector)
        .flex_none()
        .p(px(2.))
        .rounded(px(4.))
        .text_color(if pinned { t.accent } else { t.dim })
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(bright))
        .tooltip(tooltip(if pinned { "Unpin" } else { "Pin to the top" }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |v, _, _, cx| {
            cx.stop_propagation();
            pin(v, &term, !pinned, cx);
        }))
        .child(
            icon(if pinned { Icon::Pin } else { Icon::Plus })
                .size(px(13.))
                .text_color(if pinned { t.accent } else { t.dim }),
        )
        .into_any_element()
}

fn focus_listener(
    tab: String,
    term: TermId,
    cx: &mut Context<AgentsView>,
) -> impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    cx.listener(move |_, _: &ClickEvent, _, cx| {
        cx.emit(AgentsEvent::Focus {
            tab: tab.clone(),
            term: Some(term.clone()),
        });
    })
}

fn row_click(
    tab: String,
    term: TermId,
    cx: &mut Context<AgentsView>,
) -> impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    cx.listener(move |_, e: &ClickEvent, _, cx| {
        let tab = tab.clone();
        if e.click_count() >= 2 {
            cx.emit(AgentsEvent::Rename { tab });
            return;
        }
        cx.emit(AgentsEvent::Focus {
            tab,
            term: Some(term.clone()),
        });
    })
}

fn agent_row(
    v: &AgentsView,
    t: &Theme,
    e: &Entry,
    now: u64,
    cx: &mut Context<AgentsView>,
) -> AnyElement {
    let a = &e.agent;
    let term = a.term.clone();
    let key = term.to_string();
    let color = space_color(t, a.color);
    let hovered = v.hovered.as_deref() == Some(key.as_str());
    let hover = t.hover;
    let selector = format!("ag-{key}");
    let fold_key = key.clone();
    let has_subs = !e.subs.is_empty();
    let renaming_here = super::rename::inline_input(v, &a.tab).is_some();
    let badge = div()
        .id(sid("agent-fold", &key))
        .flex_none()
        .when(has_subs, |el| {
            el.on_click(cx.listener(move |v, _, _, cx| {
                cx.stop_propagation();
                toggle_fold(v, &fold_key, cx);
            }))
        })
        .child(terms::program_badge(t, &a.cmd, 16.));
    let name_w = if e.full && e.detail.is_some() {
        120.
    } else {
        200.
    };
    div()
        .id(sid("ag", &key))
        .debug_selector(move || selector)
        .px(px(8.))
        .h(px(30.))
        .flex_none()
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap(px(9.))
        .cursor_pointer()
        .when(a.focused, |el| el.bg(t.selected))
        .hover(move |s| s.bg(hover))
        .on_hover(hover_listener(key.clone(), cx))
        .on_mouse_down(MouseButton::Middle, middle_close(a.tab.clone(), cx))
        .on_mouse_down(
            MouseButton::Right,
            menu_on(
                Some(a.tab.clone()),
                Some(a.space.clone()),
                Some(term.clone()),
                cx,
            ),
        )
        .on_click(row_click(a.tab.clone(), term.clone(), cx))
        .child(badge)
        .when_some(super::rename::inline_input(v, &a.tab), |el, input| {
            el.child(super::rename::inline(t, input))
        })
        .when(!renaming_here, |el| el.child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .min_w_0()
                        .max_w(px(name_w))
                        .truncate()
                        .text_color(color)
                        .child(SharedString::from(e.name.clone())),
                )
                .when_some(e.detail.clone(), |el, d| {
                    el.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(t.dim)
                            .child(SharedString::from(format!("· {d}"))),
                    )
                }),
        ))
        .when_some(a.number, |el, n| {
            el.child(
                div()
                    .id(sid("ag-number", &key))
                    .flex_none()
                    .text_size(px(10.5))
                    .text_color(t.magenta)
                    .tooltip(tooltip(number_tip(n, &term, ASSISTANT)))
                    .child(format!("#{n}")),
            )
        })
        .when(has_subs, |el| el.child(fold_chip(v, t, e, cx)))
        .when(hovered, |el| {
            el.child(pin_button(t, term.clone(), a.pinned, cx))
        })
        .when(!hovered && e.since > 0, |el| {
            el.child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(t.dim)
                    .child(model::duration(now.saturating_sub(e.since))),
            )
        })
        .child(mark(
            t,
            sid("ag-status", &key),
            a.status,
            a.running,
            e.compact,
        ))
        .child(close_button(
            t,
            sid("close-agent", &key),
            "Close tab and its terminals",
            18.,
            a.tab.clone(),
            cx,
        ))
        .into_any_element()
}

fn sub_row(
    v: &AgentsView,
    t: &Theme,
    s: &SubRow,
    e: &Entry,
    cx: &mut Context<AgentsView>,
) -> AnyElement {
    let k = &s.kid;
    let key = k.term.to_string();
    let hover_key = format!("sub-{key}");
    let hovered = v.hovered.as_deref() == Some(hover_key.as_str());
    let on = v.peek.as_ref() == Some(&k.term);
    let hover = t.hover;
    let line = t.border_strong;
    let color = space_color(t, e.agent.color);
    let selector = hover_key.clone();
    let peek = k.term.clone();
    let placed = k.tab.clone();
    let tip = if placed.is_some() {
        format!("Sub-agent {key}: click to show its pane, middle-click to close")
    } else {
        format!("Sub-agent {key}: click to peek at it, middle-click to close")
    };
    div()
        .id(sid("sub", &key))
        .debug_selector(move || selector)
        .pl(px(39.))
        .pr(px(8.))
        .h(px(26.))
        .flex_none()
        .rounded(px(8.))
        .flex()
        .items_center()
        .text_size(px(12.5))
        .cursor_pointer()
        .when(on, |el| el.bg(t.raised))
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(tip))
        .on_hover(hover_listener(hover_key, cx))
        .on_mouse_down(MouseButton::Middle, middle_close_term(k.term.clone(), cx))
        .on_mouse_down(
            MouseButton::Right,
            menu_on(None, Some(e.agent.space.clone()), Some(k.term.clone()), cx),
        )
        .on_click(cx.listener(move |v, _, _, cx| {
            match &placed {
                Some(tab) => cx.emit(AgentsEvent::Focus {
                    tab: tab.clone(),
                    term: Some(peek.clone()),
                }),
                None => v.peek = Some(peek.clone()),
            }
            cx.notify();
        }))
        .children(s.lines.iter().map(|l| {
            div().flex_none().h_full().mr(px(4.)).child(
                div()
                    .mt(px(-4.))
                    .w(px(9.))
                    .h(px(17.))
                    .border_color(line)
                    .when(*l != Line::Blank, |el| el.border_l_1())
                    .when(matches!(l, Line::Mid | Line::Last), |el| {
                        el.border_b_1().rounded_bl(px(6.))
                    }),
            )
        }))
        .child(div().mr(px(8.)).child(terms::program_badge(t, &k.cmd, 14.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(color)
                .child(SharedString::from(k.name.clone())),
        )
        .child(
            div()
                .flex_none()
                .ml(px(6.))
                .flex()
                .items_center()
                .gap(px(4.))
                .child(mark(
                    t,
                    sid("sub-status", &key),
                    model::kid_status(k.status, k.running),
                    true,
                    v.sort == Sort::Priority,
                ))
                .when(hovered, |el| {
                    el.child(close_term_button(
                        t,
                        sid("close-sub", &key),
                        "Close this sub-agent terminal",
                        16.,
                        k.term.clone(),
                        cx,
                    ))
                }),
        )
        .into_any_element()
}

fn other_row(v: &AgentsView, t: &Theme, o: &OtherTab, cx: &mut Context<AgentsView>) -> AnyElement {
    let key = format!("tab-{}", o.tab);
    let hovered = v.hovered.as_deref() == Some(key.as_str());
    let hover = t.hover;
    let selector = key.clone();
    let tab = o.tab.clone();
    let preview = o.preview;
    let tip = if preview {
        format!(
            "{}: preview tab, double-click to keep it open, right-click for more",
            o.name
        )
    } else {
        format!(
            "{}: click to open, double-click to rename, right-click for more",
            o.name
        )
    };
    let inline = super::rename::inline_input(v, &o.tab);
    let name = if o.zoomed {
        format!("{} Z", o.name)
    } else {
        o.name.clone()
    };
    div()
        .id(SharedString::from(key.clone()))
        .debug_selector(move || selector)
        .px(px(8.))
        .h(px(30.))
        .flex_none()
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap(px(9.))
        .cursor_pointer()
        .when(o.active, |el| el.bg(t.selected))
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(tip))
        .on_hover(hover_listener(key.clone(), cx))
        .on_mouse_down(MouseButton::Middle, middle_close(o.tab.clone(), cx))
        .on_mouse_down(
            MouseButton::Right,
            menu_on(Some(o.tab.clone()), v.input.active_space.clone(), None, cx),
        )
        .on_click(other_click(tab, preview, cx))
        .child(terms::program_badge(t, &o.cmd, 16.))
        .when_some(inline.clone(), |el, input| {
            el.child(super::rename::inline(t, input))
        })
        .when(inline.is_none(), |el| {
            el.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(if o.active { t.bright } else { t.muted })
                    .when(preview, |el| el.italic().opacity(0.75))
                    .child(SharedString::from(name)),
            )
        })
        .when(o.active || hovered, |el| {
            el.child(close_button(
                t,
                sid("close-tab", &o.tab),
                "Close this tab",
                18.,
                o.tab.clone(),
                cx,
            ))
        })
        .into_any_element()
}

fn loose_row(v: &AgentsView, t: &Theme, l: &Loose, cx: &mut Context<AgentsView>) -> AnyElement {
    let hover = t.hover;
    let term = l.term.clone();
    let key = format!("loose-{term}");
    let hovered = v.hovered.as_deref() == Some(key.as_str());
    let selector = key.clone();
    div()
        .id(sid("loose", &term))
        .debug_selector(move || selector)
        .px(px(8.))
        .h(px(30.))
        .flex_none()
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap(px(9.))
        .cursor_pointer()
        .opacity(0.75)
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(
            "A terminal with no tab here: click to open it in a tab, middle-click to close",
        ))
        .on_hover(hover_listener(key, cx))
        .on_mouse_down(MouseButton::Middle, middle_close_term(term.clone(), cx))
        .on_click(cx.listener(move |_, _, _, cx| {
            cx.emit(AgentsEvent::Adopt { term: term.clone() });
        }))
        .child(terms::program_badge(t, &l.cmd, 16.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.muted)
                .child(SharedString::from(l.name.clone())),
        )
        .when(!hovered, |el| {
            el.child(
                div()
                    .flex_none()
                    .text_size(px(10.5))
                    .text_color(t.dim)
                    .child("open"),
            )
        })
        .when(hovered, |el| {
            el.child(close_term_button(
                t,
                sid("close-loose", &l.term),
                "Close this terminal",
                18.,
                l.term.clone(),
                cx,
            ))
        })
        .into_any_element()
}

fn away_row(v: &AgentsView, t: &Theme, cx: &mut Context<AgentsView>) -> AnyElement {
    let n = v.input.away.len();
    let narrow = v.input.narrow;
    let hover = t.hover;
    let bright = t.bright;
    let tip = format!(
        "{n} tab{} put away in this space: click to bring one back",
        if n == 1 { "" } else { "s" }
    );
    div()
        .id("tabs-away")
        .debug_selector(|| "tabs-away".into())
        .px(px(if narrow { 0. } else { 8. }))
        .h(px(if narrow { 26. } else { 30. }))
        .flex_none()
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap(px(if narrow { 3. } else { 9. }))
        .when(narrow, |el| el.justify_center())
        .text_size(px(if narrow { 11. } else { 12. }))
        .text_color(t.dim)
        .cursor_pointer()
        .tooltip(tooltip(tip))
        .hover(move |s| s.bg(hover).text_color(bright))
        .on_click(cx.listener(|v, e: &ClickEvent, _, cx| {
            v.away_menu = Some(e.position());
            cx.notify();
        }))
        .child(icon(Icon::Archive).size(px(14.)).text_color(t.dim))
        .child(if narrow {
            n.to_string()
        } else {
            format!("{n} put away")
        })
        .into_any_element()
}

fn tile(
    v: &AgentsView,
    t: &Theme,
    e: &Entry,
    i: usize,
    cx: &mut Context<AgentsView>,
) -> AnyElement {
    let a = &e.agent;
    let key = a.term.to_string();
    let color = space_color(t, a.color);
    let hovered = v.hovered.as_deref() == Some(key.as_str());
    let number = a.number.unwrap_or(i as u32 + 1);
    let where_ = [a.space_name.clone(), a.label.clone()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let hover = t.hover;
    let selector = format!("ag-{key}");
    let corner = if hovered {
        close_button(
            t,
            sid("close-agent", &key),
            "Close tab and its terminals",
            14.,
            a.tab.clone(),
            cx,
        )
    } else {
        mark(t, sid("ag-status", &key), a.status, a.running, true)
    };
    div()
        .w_full()
        .flex_none()
        .py(px(3.))
        .flex()
        .justify_center()
        .child(
            div()
                .id(sid("ag", &key))
                .debug_selector(move || selector)
                .relative()
                .w(px(52.))
                .h(px(44.))
                .flex_none()
                .rounded(px(9.))
                .border_1()
                .border_color(if a.focused { color } else { t.border })
                .bg(if a.focused { t.selected } else { t.surface })
                .cursor_pointer()
                .tooltip(tooltip(format!("#{number} · {where_}")))
                .hover(move |s| s.bg(hover))
                .on_hover(hover_listener(key.clone(), cx))
                .on_mouse_down(MouseButton::Middle, middle_close(a.tab.clone(), cx))
                .on_mouse_down(
                    MouseButton::Right,
                    menu_on(
                        Some(a.tab.clone()),
                        Some(a.space.clone()),
                        Some(a.term.clone()),
                        cx,
                    ),
                )
                .on_click(focus_listener(a.tab.clone(), a.term.clone(), cx))
                .child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(2.))
                        .child(terms::program_badge(t, &a.cmd, 16.))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(color)
                                .child(number.to_string()),
                        ),
                )
                .when(v.scope == Scope::All, |el| {
                    el.child(
                        div()
                            .absolute()
                            .left(px(0.))
                            .top(px(10.))
                            .w(px(3.))
                            .h(px(22.))
                            .rounded(px(2.))
                            .bg(color),
                    )
                })
                .child(div().absolute().top(px(3.)).right(px(3.)).child(corner)),
        )
        .into_any_element()
}

fn other_tile(v: &AgentsView, t: &Theme, o: &OtherTab, cx: &mut Context<AgentsView>) -> AnyElement {
    let tab = o.tab.clone();
    let preview = o.preview;
    let hover = t.hover;
    let selector = format!("tab-{}", o.tab);
    div()
        .w_full()
        .flex_none()
        .py(px(3.))
        .flex()
        .justify_center()
        .child(
            div()
                .id(sid("tab", &o.tab))
                .debug_selector(move || selector)
                .w(px(52.))
                .h(px(36.))
                .flex_none()
                .rounded(px(9.))
                .border_1()
                .border_color(if o.active { t.accent } else { t.border })
                .when(!o.active, |el| el.bg(t.surface))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .tooltip(tooltip(o.name.clone()))
                .on_mouse_down(MouseButton::Middle, middle_close(o.tab.clone(), cx))
                .on_mouse_down(
                    MouseButton::Right,
                    menu_on(Some(o.tab.clone()), v.input.active_space.clone(), None, cx),
                )
                .on_click(other_click(tab, preview, cx))
                .child(terms::program_badge(t, &o.cmd, 16.)),
        )
        .into_any_element()
}

pub fn collapse(t: &Theme, cx: &mut Context<AgentsView>) -> AnyElement {
    let bright = t.bright;
    div()
        .h(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .id("agents-open-sidebar")
                .debug_selector(|| "agents-open-sidebar".into())
                .px(px(6.))
                .text_size(px(13.))
                .text_color(t.dim)
                .cursor_pointer()
                .hover(move |s| s.text_color(bright))
                .tooltip(tooltip("Open the sidebar (⌘B)"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(AgentsEvent::ToggleSidebar)))
                .child("»"),
        )
        .into_any_element()
}

fn other_click(
    tab: String,
    preview: bool,
    cx: &mut Context<AgentsView>,
) -> impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    cx.listener(move |_, e: &ClickEvent, _, cx| {
        let tab = tab.clone();
        if e.click_count() >= 2 {
            cx.emit(if preview {
                AgentsEvent::KeepPreview { tab }
            } else {
                AgentsEvent::Rename { tab }
            });
            return;
        }
        cx.emit(AgentsEvent::Focus { tab, term: None });
    })
}
