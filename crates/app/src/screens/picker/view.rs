use super::rows::{self, PickAgent, PickSpace, PickTab, Row, Snapshot, Target};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, Focusable, FontWeight, KeyDownEvent,
    MouseButton, ScrollHandle, SharedString, Subscription, Window, div, px, relative,
};
use nixa_protocol::Status;
use nixa_ui::chip::status_dot;
use nixa_ui::icon_button::{IconSize, icon_button};
use nixa_ui::metrics::RADIUS_POPOVER;
use nixa_ui::popover::{shadow, tooltip};
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Icon, Theme, icon, theme};

pub const WIDTH: f32 = 640.0;
pub const TOP: f32 = 40.0;
pub const LIST_MAX: f32 = 320.0;
pub const PLACEHOLDER: &str = "Go to a tab, space or agent…";

#[derive(Debug, Clone, PartialEq)]
pub enum PickerEvent {
    Go(Target),
    CloseTab(String),
    Close,
}

pub struct Picker {
    pub input: Entity<TextInput>,
    pub filter: String,
    pub at: Option<usize>,
    pub show_quiet: bool,
    pub snapshot: Snapshot,
    scroll: ScrollHandle,
    _events: Subscription,
}

impl EventEmitter<PickerEvent> for Picker {}

pub fn commands() -> Vec<(&'static str, &'static str)> {
    vec![("Go to a tab, space or agent", "⌘P")]
}

pub fn new(snapshot: Snapshot, window: &mut Window, cx: &mut App) -> Entity<Picker> {
    cx.new(|cx| {
        let input = cx.new(|cx| TextInput::new(cx, PLACEHOLDER).with_icon(Icon::Search));
        let events = cx.subscribe(
            &input,
            |p: &mut Picker, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(query) => {
                    p.filter = query.clone();
                    p.at = Some(0);
                    cx.notify();
                }
                InputEvent::Submit(_) => p.choose(cx),
                InputEvent::Cancel => cx.emit(PickerEvent::Close),
            },
        );
        window.focus(&input.focus_handle(cx));
        Picker {
            input,
            filter: String::new(),
            at: None,
            show_quiet: false,
            snapshot,
            scroll: ScrollHandle::new(),
            _events: events,
        }
    })
}

impl Picker {
    pub fn set_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.snapshot = snapshot;
        cx.notify();
    }

    pub fn rows(&self) -> Vec<Row> {
        rows::rows(&self.snapshot, &self.filter, self.show_quiet)
    }

    pub fn selected(&self) -> Option<Row> {
        let picks = rows::pickable(&self.rows());
        let at = self
            .at
            .unwrap_or_else(|| rows::start(&self.snapshot, &picks))
            .min(picks.len().saturating_sub(1));
        picks.get(at).cloned()
    }

    pub fn choose(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.selected() {
            self.go(&row, cx);
        }
    }

    fn go(&mut self, row: &Row, cx: &mut Context<Self>) {
        if let Row::Quiet(_) = row {
            self.show_quiet = !self.show_quiet;
            cx.notify();
            return;
        }
        if let Some(target) = rows::target(&self.snapshot, row) {
            cx.emit(PickerEvent::Go(target));
        }
    }

    fn step(&mut self, delta: isize) {
        let all = self.rows();
        let picks = rows::pickable(&all);
        if picks.is_empty() {
            return;
        }
        let len = picks.len() as isize;
        let at = self
            .at
            .unwrap_or_else(|| rows::start(&self.snapshot, &picks))
            .min(picks.len() - 1) as isize;
        let next = (at + delta).rem_euclid(len) as usize;
        self.at = Some(next);
        if let Some(index) = all.iter().position(|r| *r == picks[next]) {
            self.scroll.scroll_to_item(index);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let ctrl = event.keystroke.modifiers.control;
        match key {
            "escape" => cx.emit(PickerEvent::Close),
            "down" => self.step(1),
            "up" => self.step(-1),
            "d" if ctrl => {
                if let Some(Row::Tab(si, ti)) = self.selected() {
                    let id = self.snapshot.spaces[si].tabs[ti].id.clone();
                    cx.emit(PickerEvent::CloseTab(id));
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Picker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render(self, cx)
    }
}

fn glyph(t: &Theme, id: SharedString, status: Option<Status>) -> AnyElement {
    let inner = match status {
        Some(status) => status_dot(t, id, status),
        None => div().text_color(t.dim).child("·").into_any_element(),
    };
    div()
        .w(px(14.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(inner)
        .into_any_element()
}

fn mark(t: &Theme, status: Option<Status>) -> Option<AnyElement> {
    let (label, color) = match status {
        Some(Status::Blocked) => ("needs you", t.yellow),
        Some(Status::Done) => ("finished", t.green),
        _ => return None,
    };
    Some(
        div()
            .flex_none()
            .text_size(px(11.))
            .text_color(color)
            .child(label)
            .into_any_element(),
    )
}

fn line(t: &Theme, id: SharedString, on: bool) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    div()
        .id(id.clone())
        .debug_selector(move || id.to_string())
        .mx(px(4.))
        .px(px(10.))
        .h(px(32.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(7.))
        .cursor_pointer()
        .when(on, |el| el.bg(t.hover))
        .hover(move |s| s.bg(hover))
}

fn head(t: &Theme, space: &PickSpace, on: bool) -> gpui::Stateful<gpui::Div> {
    let id = SharedString::from(format!("pk-head-{}", space.id));
    let hover = t.hover;
    let bright = t.bright;
    let letter = space
        .name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into());
    div()
        .id(id.clone())
        .debug_selector(move || id.to_string())
        .mx(px(4.))
        .px(px(10.))
        .pt(px(10.))
        .pb(px(3.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if on { t.bright } else { t.dim })
        .cursor_pointer()
        .tooltip(tooltip(format!("Go to space: {}", space.name)))
        .when(on, |el| el.bg(t.hover))
        .hover(move |s| s.bg(hover).text_color(bright))
        .child(
            div()
                .size(px(18.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .bg(space.color)
                .text_size(px(10.5))
                .font_weight(FontWeight::BOLD)
                .text_color(t.background)
                .child(SharedString::from(letter)),
        )
        .child(SharedString::from(space.name.clone()))
}

fn tab_row(t: &Theme, space: &PickSpace, tab: &PickTab, on: bool) -> gpui::Stateful<gpui::Div> {
    let here = space.active && tab.active;
    let mut who = tab.agents.join(" · ");
    if tab.team > 0 {
        who.push_str(&format!("  +{}", tab.team));
    }
    line(t, SharedString::from(format!("pk-{}", tab.id)), on)
        .child(glyph(t, format!("pk-st-{}", tab.id).into(), tab.status))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.bright)
                .when(here, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(SharedString::from(tab.name.clone())),
        )
        .children(mark(t, tab.status))
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(t.dim)
                .child(SharedString::from(who)),
        )
}

fn agent_row(t: &Theme, agent: &PickAgent, on: bool) -> gpui::Stateful<gpui::Div> {
    let place = [agent.space_name.as_str(), agent.tab_name.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let name = if !agent.name.is_empty() {
        agent.name.clone()
    } else if !agent.title.is_empty() {
        agent.title.clone()
    } else {
        "agent".into()
    };
    line(
        t,
        SharedString::from(format!("pk-agent-{}", agent.term)),
        on,
    )
    .child(glyph(
        t,
        format!("pk-ast-{}", agent.term).into(),
        agent.status,
    ))
    .child(
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_color(t.bright)
            .when(agent.focused, |d| d.font_weight(FontWeight::SEMIBOLD))
            .child(SharedString::from(name)),
    )
    .when(agent.pinned, |el| {
        el.child(icon(Icon::Pin).size(px(11.)).flex_none().text_color(t.dim))
    })
    .children(mark(t, agent.status))
    .child(
        div()
            .flex_none()
            .text_size(px(11.))
            .text_color(t.dim)
            .child(SharedString::from(place)),
    )
}

fn row_element(t: &Theme, p: &Picker, row: &Row, on: bool, cx: &mut Context<Picker>) -> AnyElement {
    let snap = &p.snapshot;
    let target = rows::target(snap, row);
    let fold = matches!(row, Row::Quiet(_));
    let click = cx.listener(move |p: &mut Picker, _, _, cx| {
        if fold {
            p.show_quiet = !p.show_quiet;
            cx.notify();
        } else if let Some(target) = target.clone() {
            cx.emit(PickerEvent::Go(target));
        }
    });
    match row {
        Row::Head(si) => head(t, &snap.spaces[*si], on)
            .on_click(click)
            .into_any_element(),
        Row::Tab(si, ti) => tab_row(t, &snap.spaces[*si], &snap.spaces[*si].tabs[*ti], on)
            .on_click(click)
            .into_any_element(),
        Row::Agent(i) => agent_row(t, &snap.agents[*i], on)
            .on_click(click)
            .into_any_element(),
        Row::Agents => div()
            .px(px(10.))
            .pt(px(10.))
            .pb(px(3.))
            .flex_none()
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.dim)
            .child("Agents")
            .into_any_element(),
        Row::Quiet(count) => {
            let bright = t.bright;
            div()
                .id("pk-earlier")
                .debug_selector(|| "pk-earlier".into())
                .mt(px(6.))
                .px(px(10.))
                .py(px(6.))
                .flex_none()
                .border_t_1()
                .border_color(t.border)
                .text_size(px(12.))
                .text_color(if on { t.bright } else { t.dim })
                .when(on, |el| el.bg(t.hover))
                .cursor_pointer()
                .hover(move |s| s.text_color(bright))
                .on_click(click)
                .child(SharedString::from(rows::quiet_label(*count, p.show_quiet)))
                .into_any_element()
        }
        Row::NewTabHere(si) => line(t, format!("pk-new-{}", snap.spaces[*si].id).into(), on)
            .on_click(click)
            .child("+ New tab here")
            .into_any_element(),
        Row::NewTab | Row::NewSpace => {
            let (id, label) = if *row == Row::NewTab {
                ("pk-new", "New tab")
            } else {
                ("pk-space-here", "New space")
            };
            line(t, id.into(), on)
                .gap(px(8.))
                .when(*row == Row::NewTab, |el| el.mt(px(4.)))
                .text_color(t.muted)
                .on_click(click)
                .child(div().w(px(14.)).flex_none().child("+"))
                .child(label)
                .into_any_element()
        }
    }
}

pub fn render(p: &Picker, cx: &mut Context<Picker>) -> AnyElement {
    let t = theme(cx).clone();
    let all = p.rows();
    let selected = p.selected();
    let rows: Vec<AnyElement> = all
        .iter()
        .map(|row| {
            let on = selected.as_ref() == Some(row);
            row_element(&t, p, row, on, cx)
        })
        .collect();
    div()
        .id("picker-scrim")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(TOP))
        .bg(t.scrim)
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| cx.emit(PickerEvent::Close)),
        )
        .child(
            div()
                .id("picker")
                .debug_selector(|| "picker".into())
                .w(px(WIDTH))
                .max_w_full()
                .max_h(relative(0.8))
                .p(px(18.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .rounded(px(RADIUS_POPOVER))
                .border_1()
                .border_color(t.border)
                .bg(t.popover)
                .shadow(shadow(&t))
                .text_size(px(13.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_key_down(cx.listener(Picker::key))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.bright)
                                .child("Go to…"),
                        )
                        .child(div().text_size(px(11.)).text_color(t.dim).child("Esc"))
                        .child(
                            icon_button(&t, "pk-close", Icon::X, IconSize::Small, false)
                                .debug_selector(|| "pk-close".into())
                                .tooltip(tooltip("Close (Esc)"))
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.emit(PickerEvent::Close);
                                })),
                        ),
                )
                .child(div().flex().flex_col().child(p.input.clone()))
                .child(
                    div().relative().max_h(px(LIST_MAX)).min_h_0().flex_shrink().flex().flex_col().child(div()
                        .id("pk-list")
                        .max_h(px(LIST_MAX))
                        .min_h_0()
                        .flex_shrink()
                        .overflow_y_scroll()
                        .track_scroll(&p.scroll)
                        .pb(px(6.))
                        .flex()
                        .flex_col()
                        .children(rows),
                    ).child(nixa_ui::scroll::scrollbar(&t, &p.scroll)),
                ),
        )
        .into_any_element()
}
