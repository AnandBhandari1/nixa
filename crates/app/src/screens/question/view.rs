use super::card::{
    Card, CardKind, CardState, Snapshot, card_agent_title, hint, kind_label,
    place_words, send_words, snapshot_of, waiting_words,
};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, EventEmitter, FontWeight, Hsla,
    SharedString, Stateful, Window, div, px,
};
use nixa_ui::text::MONO_FONT;
use nixa_ui::{Icon, Theme, icon, theme};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionEvent {
    Answer { id: String, indexes: Vec<usize> },
    GoTo { terminal: String },
}

impl QuestionEvent {
    pub fn message(&self) -> Option<Value> {
        match self {
            QuestionEvent::Answer { id, indexes } => {
                Some(json!({"type": "answer", "id": id, "indexes": indexes}))
            }
            QuestionEvent::GoTo { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionCommand {
    OpenCard,
    GoToAgent,
}

pub fn commands() -> Vec<(&'static str, &'static str, QuestionCommand)> {
    vec![
        (
            "Show the agent question",
            "Open the question card again",
            QuestionCommand::OpenCard,
        ),
        (
            "Go to the asking agent",
            "Open the terminal of the agent that asks",
            QuestionCommand::GoToAgent,
        ),
    ]
}

pub struct QuestionView {
    pub snap: Snapshot,
    pub picked: Vec<usize>,
    pub picked_for: String,
    pub collapsed_for: Option<String>,
    pub name: String,
    pub voice: bool,
}

impl EventEmitter<QuestionEvent> for QuestionView {}

impl QuestionView {
    pub fn empty() -> QuestionView {
        QuestionView {
            snap: Snapshot::default(),
            picked: Vec::new(),
            picked_for: String::new(),
            collapsed_for: None,
            name: "Nixa".into(),
            voice: false,
        }
    }

    pub fn apply(&mut self, message: &Value, cx: &mut Context<QuestionView>) -> bool {
        let Some(snap) = snapshot_of(message) else {
            return false;
        };
        self.set_snapshot(snap, cx);
        true
    }

    pub fn set_snapshot(&mut self, snap: Snapshot, cx: &mut Context<QuestionView>) {
        self.snap = snap;
        if let Some(card) = &self.snap.card {
            let key = format!("{}:{}", card.id, card.step);
            if self.picked_for != key {
                self.picked_for = key;
                self.picked.clear();
            }
        } else {
            self.picked.clear();
            self.picked_for.clear();
            self.collapsed_for = None;
        }
        cx.notify();
    }

    pub fn run(&mut self, command: QuestionCommand, cx: &mut Context<QuestionView>) {
        match command {
            QuestionCommand::OpenCard => {
                self.collapsed_for = None;
                cx.notify();
            }
            QuestionCommand::GoToAgent => {
                if let Some(card) = &self.snap.card {
                    cx.emit(QuestionEvent::GoTo {
                        terminal: card.terminal.clone(),
                    });
                }
            }
        }
    }

    fn pick(&mut self, index: usize, cx: &mut Context<QuestionView>) {
        match self.picked.iter().position(|&i| i == index) {
            Some(at) => {
                self.picked.remove(at);
            }
            None => {
                self.picked.push(index);
                self.picked.sort_unstable();
            }
        }
        cx.notify();
    }

    fn click(&mut self, index: usize, cx: &mut Context<QuestionView>) {
        let Some(card) = self.snap.card.clone() else {
            return;
        };
        if card.state == CardState::Sending {
            return;
        }
        if card.multi {
            self.pick(index, cx);
        } else {
            cx.emit(QuestionEvent::Answer {
                id: card.id,
                indexes: vec![index],
            });
        }
    }

    fn send_picked(&mut self, cx: &mut Context<QuestionView>) {
        let Some(card) = self.snap.card.clone() else {
            return;
        };
        if self.picked.is_empty() || card.state == CardState::Sending {
            return;
        }
        cx.emit(QuestionEvent::Answer {
            id: card.id,
            indexes: self.picked.clone(),
        });
    }
}

impl Render for QuestionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render(self, cx)
    }
}

pub fn new(cx: &mut App) -> Entity<QuestionView> {
    cx.new(|_| QuestionView::empty())
}

fn chip(
    t: &Theme,
    id: &'static str,
    which: Icon,
    label: impl Into<SharedString>,
    color: Hsla,
    enabled: bool,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    let hover = t.hover;
    let color = if enabled { color } else { t.dim };
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .h(px(24.))
        .px(px(8.))
        .gap(px(5.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .text_size(px(11.5))
        .text_color(color)
        .when(enabled, move |el| {
            el.cursor_pointer().hover(move |s| s.bg(hover))
        })
        .child(icon(which).size(px(13.)).text_color(color))
        .when(!label.is_empty(), |el| el.child(label))
}

fn kind_tag(t: &Theme, kind: CardKind) -> Div {
    div()
        .absolute()
        .top(px(1.))
        .right(px(6.))
        .text_size(px(10.))
        .text_color(t.dim.opacity(0.6))
        .child(kind_label(kind))
}

fn agent_link(t: &Theme, card: &Card, cx: &mut Context<QuestionView>) -> Stateful<Div> {
    let terminal = card.terminal.clone();
    let hover = t.hover;
    div()
        .id("question-agent")
        .debug_selector(|| "question-agent".into())
        .flex_none()
        .px(px(4.))
        .rounded(px(4.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(13.))
        .text_color(t.cyan)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(QuestionEvent::GoTo {
                terminal: terminal.clone(),
            })
        }))
        .child(card_agent_title(card))
}

fn go_chip(t: &Theme, card: &Card, main: bool, cx: &mut Context<QuestionView>) -> Stateful<Div> {
    let terminal = card.terminal.clone();
    let color = if main { t.green } else { t.blue };
    chip(
        t,
        "question-go",
        Icon::Terminal,
        format!("Go to {}", card_agent_title(card)),
        color,
        true,
    )
    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
        cx.emit(QuestionEvent::GoTo {
            terminal: terminal.clone(),
        })
    }))
}

fn waiting_line(t: &Theme, waiting: usize) -> Div {
    if waiting == 0 {
        return div()
            .flex_none()
            .text_size(px(12.))
            .text_color(t.magenta)
            .child(waiting_words(0));
    }
    div()
        .flex()
        .flex_none()
        .gap(px(4.))
        .items_center()
        .text_size(px(12.))
        .child(
            div()
                .text_color(t.green)
                .font_weight(FontWeight::SEMIBOLD)
                .child(waiting.to_string()),
        )
        .child(div().text_color(t.blue).child("more waiting"))
}

const OPTION_IDS: [&str; 10] = [
    "question-option-0",
    "question-option-1",
    "question-option-2",
    "question-option-3",
    "question-option-4",
    "question-option-5",
    "question-option-6",
    "question-option-7",
    "question-option-8",
    "question-option-9",
];

fn option_row(
    t: &Theme,
    view: &QuestionView,
    card: &Card,
    index: usize,
    cx: &mut Context<QuestionView>,
) -> Stateful<Div> {
    let option = &card.options[index];
    let on = card.multi && view.picked.contains(&index);
    let busy = card.state == CardState::Sending;
    let hover = t.hover;
    let selector = OPTION_IDS
        .get(index)
        .copied()
        .unwrap_or("question-option-more");
    let mark: AnyElement = if card.multi {
        icon(if on { Icon::Check } else { Icon::Square })
            .size(px(15.))
            .text_color(t.cyan)
            .into_any_element()
    } else {
        div()
            .w(px(18.))
            .flex_none()
            .text_color(t.cyan)
            .font_weight(FontWeight::SEMIBOLD)
            .child((index + 1).to_string())
            .into_any_element()
    };
    div()
        .id(("question-option", index))
        .debug_selector(move || selector.to_string())
        .w_full()
        .px(px(10.))
        .py(px(7.))
        .gap(px(10.))
        .flex()
        .items_start()
        .rounded(px(8.))
        .border_1()
        .border_color(if on { t.cyan } else { t.border })
        .when(!busy, move |el| {
            el.cursor_pointer().hover(move |s| s.bg(hover))
        })
        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.click(index, cx)))
        .child(mark)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(if busy { t.dim } else { t.green })
                        .whitespace_normal()
                        .child(option.label.clone()),
                )
                .when_some(option.description.clone(), |el, d| {
                    el.child(
                        div()
                            .text_size(px(11.5))
                            .text_color(t.magenta)
                            .whitespace_normal()
                            .child(d),
                    )
                }),
        )
}

fn multi_row(t: &Theme, view: &QuestionView, card: &Card, cx: &mut Context<QuestionView>) -> Div {
    let count = view.picked.len();
    let enabled = count > 0 && card.state != CardState::Sending;
    div()
        .w_full()
        .flex()
        .gap(px(8.))
        .items_center()
        .child(
            div()
                .flex_1()
                .text_size(px(11.5))
                .text_color(t.magenta)
                .child("You can pick more than one."),
        )
        .child(
            chip(
                t,
                "question-send",
                Icon::Send,
                send_words(count),
                t.green,
                enabled,
            )
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.send_picked(cx))),
        )
}

fn strip(t: &Theme, card: &Card, cx: &mut Context<QuestionView>) -> AnyElement {
    div()
        .id("question-strip")
        .w_full()
        .relative()
        .flex()
        .gap(px(6.))
        .px(px(8.))
        .pr(px(78.))
        .py(px(6.))
        .items_center()
        .rounded(px(8.))
        .border_1()
        .border_color(t.cyan.opacity(0.47))
        .bg(t.cyan.opacity(0.06))
        .child(kind_tag(t, card.kind))
        .child(agent_link(t, card, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .text_color(t.magenta)
                .child(place_words(card)),
        )
        .child(
            chip(t, "question-open", Icon::Layers, "Open", t.blue, true).on_click(cx.listener(
                |view, _: &ClickEvent, _, cx| view.run(super::QuestionCommand::OpenCard, cx),
            )),
        )
        .child(go_chip(t, card, false, cx))
        .into_any_element()
}

fn block(
    t: &Theme,
    view: &QuestionView,
    card: &Card,
    cx: &mut Context<QuestionView>,
) -> AnyElement {
    let card_id = card.id.clone();
    let options: Vec<Stateful<Div>> = (0..card.options.len())
        .map(|i| option_row(t, view, card, i, cx))
        .collect();
    let step = (card.steps > 1).then(|| format!("Question {} of {}", card.step, card.steps));
    div()
        .id("question-card")
        .w_full()
        .relative()
        .flex()
        .flex_col()
        .gap(px(9.))
        .p(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.cyan.opacity(0.47))
        .bg(t.cyan.opacity(0.06))
        .child(kind_tag(t, card.kind))
        .child(
            div()
                .w_full()
                .flex()
                .gap(px(4.))
                .items_center()
                .child(agent_link(t, card, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(t.magenta)
                        .child(place_words(card)),
                )
                .child(
                    chip(t, "question-hide", Icon::X, "", t.magenta, true).on_click(cx.listener(
                        move |view, _: &ClickEvent, _, cx| {
                            view.collapsed_for = Some(card_id.clone());
                            cx.notify();
                        },
                    )),
                ),
        )
        .when(step.is_some() || card.header.is_some(), |el| {
            el.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .items_center()
                    .when_some(step.clone(), |row, s| {
                        row.child(div().text_size(px(11.5)).text_color(t.blue).child(s))
                    })
                    .when_some(card.header.clone(), |row, h| {
                        row.child(
                            div()
                                .px(px(6.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(t.cyan)
                                .text_size(px(11.))
                                .text_color(t.cyan)
                                .child(h),
                        )
                    }),
            )
        })
        .child(
            div()
                .text_size(px(if card.unread { 13. } else { 14.5 }))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if card.unread { t.foreground } else { t.green })
                .whitespace_normal()
                .child(card.title.clone()),
        )
        .when_some(card.detail.clone(), |el, d| {
            el.child(
                div()
                    .id("question-detail")
                    .w_full()
                    .max_h(px(150.))
                    .overflow_y_scroll()
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .bg(t.surface)
                    .font_family(MONO_FONT)
                    .text_size(px(11.5))
                    .text_color(t.cyan)
                    .whitespace_normal()
                    .child(d),
            )
        })
        .when(!options.is_empty(), |el| {
            el.child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(options),
            )
        })
        .when(card.multi, |el| el.child(multi_row(t, view, card, cx)))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(if card.unread { t.yellow } else { t.magenta })
                .whitespace_normal()
                .child(hint(card, &view.name, view.voice)),
        )
        .child(div().w_full().h(px(1.)).bg(t.border))
        .child(
            div()
                .w_full()
                .flex()
                .gap(px(6.))
                .items_center()
                .child(waiting_line(t, view.snap.waiting))
                .child(div().flex_1())
                .child(go_chip(t, card, card.unread, cx)),
        )
        .into_any_element()
}

fn card_part(t: &Theme, view: &QuestionView, cx: &mut Context<QuestionView>) -> Option<AnyElement> {
    let card = view.snap.card.clone()?;
    if view.collapsed_for.as_deref() == Some(card.id.as_str()) {
        return Some(strip(t, &card, cx));
    }
    Some(block(t, view, &card, cx))
}

pub fn render(view: &QuestionView, cx: &mut Context<QuestionView>) -> AnyElement {
    let t = theme(cx).clone();
    let card = card_part(&t, view, cx);
    div()
        .id("question")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(10.))
        .when_some(card, |el, c| el.child(c))
        .into_any_element()
}

