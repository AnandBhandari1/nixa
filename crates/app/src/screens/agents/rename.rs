use super::{AgentsEvent, AgentsView};
use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, Focusable, SharedString, Subscription, Window, div, px};
use nixa_protocol::TermId;
use nixa_ui::Theme;
use nixa_ui::modal::{dialog, overlay};
use nixa_ui::text_input::{InputEvent, TextInput};

pub const RENAME_WIDTH: f32 = 380.;

pub struct Renaming {
    pub tab: String,
    pub term: TermId,
    pub start: String,
    pub input: Option<Entity<TextInput>>,
    events: Option<Subscription>,
}

pub fn open(v: &mut AgentsView, tab: &str, term: TermId, name: &str, cx: &mut Context<AgentsView>) {
    v.renaming = Some(Renaming {
        tab: tab.to_string(),
        term,
        start: name.to_string(),
        input: None,
        events: None,
    });
    cx.notify();
}

pub fn close(v: &mut AgentsView, cx: &mut Context<AgentsView>) {
    if v.renaming.take().is_some() {
        cx.notify();
    }
}

pub fn submit(v: &mut AgentsView, value: &str, cx: &mut Context<AgentsView>) {
    let Some(r) = v.renaming.take() else {
        return;
    };
    let name = value.trim();
    if !name.is_empty() && name != r.start.trim() {
        cx.emit(AgentsEvent::SetName {
            term: r.term,
            name: name.to_string(),
        });
    }
    cx.notify();
}

pub fn ensure_input(v: &mut AgentsView, window: &mut Window, cx: &mut Context<AgentsView>) -> Option<Entity<TextInput>> {
    let r = v.renaming.as_mut()?;
    if let Some(input) = &r.input {
        return Some(input.clone());
    }
    let input = cx.new(|cx| TextInput::new(cx, "Tab name"));
    let start = r.start.clone();
    input.update(cx, |i, cx| i.set_value(start, cx));
    window.focus(&input.focus_handle(cx));
    let events = cx.subscribe(&input, |v: &mut AgentsView, _, event: &InputEvent, cx| match event {
        InputEvent::Submit(value) => submit(v, value, cx),
        InputEvent::Cancel => close(v, cx),
        InputEvent::Changed(_) => {}
    });
    r.input = Some(input.clone());
    r.events = Some(events);
    Some(input)
}

fn button(t: &Theme, id: &'static str, text: &'static str, strong: bool) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    div()
        .id(id)
        .debug_selector(move || id.into())
        .h(px(28.))
        .px(px(12.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .bg(if strong { t.selected } else { t.raised })
        .text_size(px(12.))
        .text_color(t.bright)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(text)
}

pub fn inline_tab(v: &AgentsView) -> Option<&str> {
    let r = v.renaming.as_ref()?;
    if v.input.narrow {
        return None;
    }
    let in_groups = v
        .view
        .groups
        .iter()
        .filter(|g| !v.folded.contains(&g.id))
        .flat_map(|g| g.members.iter())
        .any(|e| e.agent.tab == r.tab);
    let in_others = !v.folded.contains(super::model::OTHER_TABS)
        && v.input.others.iter().any(|o| o.tab == r.tab);
    (in_groups || in_others).then_some(r.tab.as_str())
}

pub fn inline_input(v: &AgentsView, tab: &str) -> Option<Entity<TextInput>> {
    let r = v.renaming.as_ref()?;
    (r.tab == tab && inline_tab(v).is_some())
        .then(|| r.input.clone())
        .flatten()
}

pub fn inline(t: &Theme, input: Entity<TextInput>) -> AnyElement {
    div()
        .id("agents-rename-inline")
        .debug_selector(|| "agents-rename-inline".into())
        .flex_1()
        .min_w_0()
        .h(px(24.))
        .px(px(4.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(t.accent)
        .bg(t.raised)
        .on_click(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(input)
        .into_any_element()
}

pub fn render(
    v: &mut AgentsView,
    t: &Theme,
    window: &mut Window,
    cx: &mut Context<AgentsView>,
) -> Option<AnyElement> {
    let input = ensure_input(v, window, cx)?;
    if inline_tab(v).is_some() {
        return None;
    }
    let id = SharedString::from(format!("agents-rename-dialog-{}", v.renaming.as_ref()?.tab));
    let this = cx.entity();
    let (shut, cancel, save) = (this.clone(), this.clone(), this.clone());
    let typed = input.clone();
    let footer = div()
        .flex()
        .gap(px(8.))
        .child(
            button(t, "agents-rename-cancel", "Cancel", false)
                .on_click(move |_, _, cx| cancel.update(cx, close)),
        )
        .child(
            button(t, "agents-rename-save", "Save", true).on_click(move |_, _, cx| {
                let value = typed.read(cx).value().to_string();
                save.update(cx, |v, cx| submit(v, &value, cx));
            }),
        )
        .into_any_element();
    let body = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().id("agents-rename-input").debug_selector(|| "agents-rename-input".into()).child(input))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(t.dim)
                .child(SharedString::from("Enter saves. Esc cancels. The name stays on the tab's first pane.")),
        );
    let card = dialog(
        t,
        id,
        "Rename tab",
        RENAME_WIDTH,
        body,
        Some(footer),
        move |_, cx| shut.update(cx, close),
    );
    Some(
        overlay(t, "agents-rename", card, move |_, cx| this.update(cx, close))
            .into_any_element(),
    )
}
