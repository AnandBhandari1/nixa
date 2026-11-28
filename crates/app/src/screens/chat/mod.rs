mod composer;
mod format;
mod header;
mod kinds;
mod list;
pub mod model;
mod parts;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod view_tests;
mod tray;
pub mod wire;
mod work;

use gpui::prelude::*;
use crate::screens::question::QuestionView;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, Focusable, FontWeight,
    ScrollHandle, Subscription, Window, div, px,
};
use nixa_ui::metrics::PANEL_WIDTH;
use crate::screens::settings::parts::memory::text_area::{AreaEvent, TextArea};
use nixa_ui::{Theme, theme};
use model::{Link, Snapshot};
use serde_json::Value;
use std::collections::HashSet;
use wire::Command;

pub use wire::{Wire, wire as to_wire};

pub const SETTINGS_PART: &str = "Chat brain";

#[derive(Clone, Debug, PartialEq)]
pub enum ChatEvent {
    Send(Command),
    OpenSettings(&'static str),
    Hide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Focus,
    Clean,
    Stop,
    ToggleSkipped,
    JumpNewest,
    ChatBrain,
}

pub struct ChatView {
    pub snap: Snapshot,
    pub input: Entity<TextArea>,
    pub scroll: ScrollHandle,
    pub width: f32,
    pub offset_min: i32,
    pub space: Option<(String, String)>,
    pub show_quiet: bool,
    pub shown: HashSet<u64>,
    pub clean_ask: bool,
    pub question: Option<Entity<QuestionView>>,
    pub font_family: Option<String>,
    pub user_moved: bool,
    pub watch_scroll: bool,
    pub follow: bool,
    pub last_id: Option<u64>,
    pub tray_key: String,
    pub me: gpui::WeakEntity<ChatView>,
    pub menu: Option<(header::ChatMenu, gpui::Point<gpui::Pixels>)>,
    _input_events: Subscription,
    _input_grow: Subscription,
    _phone: Subscription,
    _talk: Vec<Subscription>,
    _question: Option<Subscription>,
}

impl EventEmitter<ChatEvent> for ChatView {}

pub const COMPOSER_ROWS: u16 = 6;

pub fn build(cx: &mut Context<ChatView>) -> ChatView {
    let input = cx.new(|cx| TextArea::new(cx, "Message Nixa").auto_rows(1, COMPOSER_ROWS).enter_sends());
    let events = cx.subscribe(&input, |view: &mut ChatView, input, event: &AreaEvent, cx| {
        if let AreaEvent::Save = event {
            let text = input.read(cx).value().to_string();
            send_typed(view, text, cx);
        }
    });
    let grow = cx.observe(&input, |view: &mut ChatView, _, cx| {
        if !view.user_moved {
            view.follow = true;
            cx.notify();
        }
    });
    let hub = crate::screens::phone_chip::hub(cx);
    let phone = cx.observe(&hub, |_, _, cx| cx.notify());
    let card = crate::screens::talk_card::hub(cx);
    let progress = crate::screens::progress::hub(cx);
    let quiet = crate::screens::quiet_switch::hub(cx);
    let talk = vec![
        cx.observe(&card, |_, _, cx| cx.notify()),
        cx.observe(&progress, |_, _, cx| cx.notify()),
        cx.observe(&quiet, |_, _, cx| cx.notify()),
    ];
    ChatView {
        me: cx.entity().downgrade(),
        snap: Snapshot { mic_off: true, ..Snapshot::default() },
        input,
        scroll: ScrollHandle::new(),
        width: PANEL_WIDTH,
        offset_min: format::local_offset_min(),
        space: None,
        show_quiet: false,
        menu: None,
        shown: HashSet::new(),
        clean_ask: false,
        question: None,
        font_family: None,
        user_moved: false,
        watch_scroll: false,
        follow: true,
        last_id: None,
        tray_key: String::new(),
        _input_events: events,
        _input_grow: grow,
        _phone: phone,
        _talk: talk,
        _question: None,
    }
}

pub fn new(cx: &mut App) -> Entity<ChatView> {
    cx.new(build)
}

pub fn set_font_family(view: &Entity<ChatView>, family: String, cx: &mut App) {
    view.update(cx, |v, cx| {
        if v.font_family.as_deref() != Some(family.as_str()) {
            v.font_family = Some(family);
            cx.notify();
        }
    });
}

pub fn set_question(view: &Entity<ChatView>, question: Entity<QuestionView>, cx: &mut App) {
    view.update(cx, |v, cx| {
        v._question = Some(cx.observe(&question, |_, _, cx| cx.notify()));
        v.question = Some(question);
        cx.notify();
    });
}

pub fn render(view: &Entity<ChatView>) -> AnyElement {
    view.clone().into_any_element()
}

pub fn apply(view: &Entity<ChatView>, msg: &Value, cx: &mut App) {
    view.update(cx, |v, cx| {
        if model::apply(&mut v.snap, msg) {
            cx.notify();
        }
    });
}

pub fn set_link(view: &Entity<ChatView>, link: Link, cx: &mut App) {
    view.update(cx, |v, cx| {
        if v.snap.link != link {
            v.snap.link = link;
            cx.notify();
        }
    });
}

pub fn set_space(view: &Entity<ChatView>, space: Option<(String, String)>, cx: &mut App) {
    view.update(cx, |v, cx| {
        if v.space != space {
            v.space = space;
            cx.notify();
        }
    });
}

pub fn set_width(view: &Entity<ChatView>, width: f32, cx: &mut App) {
    view.update(cx, |v, cx| {
        if (v.width - width).abs() > 0.5 {
            v.width = width;
            v.follow = !v.user_moved;
            cx.notify();
        }
    });
}

pub fn commands() -> Vec<(&'static str, Action)> {
    vec![
        ("Type to Nixa", Action::Focus),
        ("Clean the chat", Action::Clean),
        ("Stop Nixa speaking or thinking", Action::Stop),
        ("Show or hide skipped lines", Action::ToggleSkipped),
        ("Jump to newest message", Action::JumpNewest),
        ("Open Chat brain settings", Action::ChatBrain),
    ]
}

pub fn run(view: &Entity<ChatView>, action: Action, window: &mut Window, cx: &mut App) {
    view.update(cx, |v, cx| {
        match action {
            Action::Focus => window.focus(&v.input.focus_handle(cx)),
            Action::Clean => v.clean_ask = true,
            Action::Stop => cx.emit(ChatEvent::Send(Command::Stop)),
            Action::ToggleSkipped => composer::toggle_skipped(v),
            Action::JumpNewest => jump_newest(v),
            Action::ChatBrain => cx.emit(ChatEvent::OpenSettings(SETTINGS_PART)),
        }
        cx.notify();
    });
}

pub fn send(cmd: Command, cx: &mut Context<ChatView>) {
    cx.emit(ChatEvent::Send(cmd));
    cx.notify();
}

pub fn send_typed(view: &mut ChatView, text: String, cx: &mut Context<ChatView>) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    view.input.update(cx, |input, cx| input.set_value("", cx));
    jump_newest(view);
    send(Command::Typed(text), cx);
}

pub fn jump_newest(view: &mut ChatView) {
    view.user_moved = false;
    view.watch_scroll = false;
    view.follow = true;
}

fn frame(t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("chat")
        .on_key_down(|event, window, cx| {
            let key = &event.keystroke;
            if key.key == "tab" && !key.modifiers.control && !key.modifiers.alt && !key.modifiers.platform && !key.modifiers.function {
                if key.modifiers.shift {
                    window.focus_prev();
                } else {
                    window.focus_next();
                }
                cx.stop_propagation();
            }
        })
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(t.background)
        .text_color(t.foreground)
        .text_size(px(13.))
}

fn message(t: &Theme, view: &ChatView) -> AnyElement {
    let name = view.snap.name().to_string();
    let (title, text, color) = match view.snap.link {
        Link::Off => (
            format!("{name} is off"),
            format!("Turn {name} on with the {name} button in the top bar. The chat shows here when the {name} brain runs."),
            t.dim,
        ),
        _ => (
            format!("{name} is starting…"),
            "Connecting to the brain. This takes a few seconds.".to_string(),
            t.magenta,
        ),
    };
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px(px(20.))
        .gap(px(8.))
        .child(parts::avatar(color, 36., parts::first_letter(&name)))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.foreground)
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(t.dim)
                .text_center()
                .child(text),
        )
        .into_any_element()
}

fn question_part(view: &ChatView, cx: &App) -> Option<AnyElement> {
    let question = view.question.as_ref()?;
    let q = question.read(cx);
    q.snap.card.is_some().then(|| question.clone().into_any_element())
}

impl gpui::Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let head = header::render(self, &t, cx);
        if self.snap.link != Link::Up {
            return frame(&t).child(head).child(message(&t, self));
        }
        let off = self.snap.off();
        let name = self.snap.name().to_string();
        let (body, jump) = (list::render(self, &t, cx), list::jump(self, &t, cx));
        let question = question_part(self, cx);
        let tray = tray::render(self, &t, question, cx);
        let foot = if off {
            div()
                .w_full()
                .flex_none()
                .px(px(12.))
                .py(px(10.))
                .border_t_1()
                .border_color(t.border)
                .text_size(px(11.))
                .text_color(t.dim)
                .child(format!(
                    "{name} is off. Brain settings can still be changed. Mic and voice stay off."
                ))
                .into_any_element()
        } else {
            composer::render(self, &t, cx)
        };
        frame(&t)
            .child(head)
            .child(body)
            .children(jump)
            .children(tray)
            .child(foot)
            .children(header::menus(self, &t, cx))
    }
}
