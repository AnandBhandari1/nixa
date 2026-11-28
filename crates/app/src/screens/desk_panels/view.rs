use super::model::{self, Card, PANELS, Panel};
use super::{DeskView, answer, for_panel, key, label, set_reads, switch, title};
use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, FontWeight, SharedString, Stateful, Window, div, px};
use nixa_ui::{Theme, theme};

pub const GATE_TEXT: &str = "One gate for Eyes, Browser and Hands. Nixa runs each step by itself and writes it to the audit log. Ask Nixa by voice or chat to use this tool.";
pub const CARD_TITLE: &str = "Allow this one desktop step?";
pub const BROWSER_CARD_TITLE: &str = "Allow this one browser step?";
pub const LINKED_TEXT: &str = "Connected · newest audit events first";
pub const UNLINKED_TEXT: &str = "Nixa is not connected; no step can be answered here.";

pub fn footer_text(v: &DeskView) -> String {
    let dir = model::audit_dir(&v.data);
    let base = format!("Privacy on needs a local brain. Audit and captures: {}.", dir.display());
    if v.panel == Panel::Browser {
        format!("{base} Nixa runs each browser step and writes it to the audit log.")
    } else {
        format!("{base} {}. Inputs run by themselves and go to the audit log. OCR reads text; diagrams and visual details may need your help.", reads_label(v.allow_reads))
    }
}

pub fn shows_reads(v: &DeskView) -> bool {
    v.linked && v.panel != Panel::Browser
}

pub fn reads_label(on: bool) -> &'static str {
    if on { "Reads without a card: on" } else { "Reads without a card: off" }
}

fn reads(v: &DeskView, t: &Theme, cx: &mut Context<DeskView>) -> AnyElement {
    let on = v.allow_reads;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .child(button(t, "desk-reads".into(), reads_label(on), on).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| set_reads(v, !on, cx))))
        .child(div().text_color(t.muted).child("For Eyes and Hands: list windows, read text and take shots without a card. Clicks and typing run by themselves and go to the audit log."))
        .into_any_element()
}

pub fn empty_text(panel: Panel) -> String {
    format!("No {} steps yet.", label(panel).to_lowercase())
}

fn button(t: &Theme, id: String, text: impl Into<SharedString>, on: bool) -> Stateful<gpui::Div> {
    let hover = t.hover;
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(10.))
        .py(px(6.))
        .rounded(px(5.))
        .cursor_pointer()
        .border_1()
        .border_color(t.border)
        .bg(if on { t.selected } else { t.background })
        .hover(move |s| s.bg(hover))
        .child(text.into())
}

fn switches(v: &DeskView, t: &Theme, cx: &mut Context<DeskView>) -> AnyElement {
    let mut row = div().flex().flex_row().gap(px(8.));
    for panel in PANELS {
        row = row.child(
            button(t, format!("desk-switch-{}", key(panel)), label(panel), v.panel == panel)
                .on_click(cx.listener(move |v, _: &ClickEvent, window, cx| switch(v, panel, window, cx))),
        );
    }
    row.into_any_element()
}

fn card(v: &DeskView, t: &Theme, item: &Card, name: &str, heading: &'static str, cx: &mut Context<DeskView>) -> AnyElement {
    let (yes_card, no_card) = (item.clone(), item.clone());
    let selector = format!("desk-card-{name}");
    let mut card = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex()
        .flex_col()
        .p(px(12.))
        .gap(px(8.))
        .border_1()
        .border_color(t.yellow)
        .rounded(px(6.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(heading))
        .child(div().child(item.message.clone()));
    if !item.note.trim().is_empty() {
        card = card.child(div().text_color(t.muted).child(item.note.clone()));
    }
    let busy = v.answering;
    card.child(
        div()
            .flex()
            .flex_row()
            .gap(px(10.))
            .child(button(t, format!("desk-yes-{name}"), if busy { "Sending…" } else { "Yes, this step" }, false).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| answer(v, yes_card.clone(), true, cx))))
            .child(button(t, format!("desk-no-{name}"), "No", false).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| answer(v, no_card.clone(), false, cx)))),
    )
    .into_any_element()
}

fn audit(v: &DeskView, t: &Theme) -> AnyElement {
    let rows = for_panel(&v.rows, v.panel);
    let mut list = div()
        .id("desk-audit")
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .gap(px(8.))
        .overflow_y_scroll()
        .track_scroll(&v.scroll);
    if rows.is_empty() {
        list = list.child(div().text_color(t.dim).child(empty_text(v.panel)));
    }
    for (n, row) in rows.into_iter().enumerate() {
        let selector = format!("desk-row-{n}");
        list = list.child(
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .flex_none()
                .flex()
                .flex_col()
                .p(px(10.))
                .gap(px(5.))
                .border_1()
                .border_color(t.border)
                .rounded(px(5.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(row.head.clone()))
                .child(div().text_color(t.muted).child(row.body.clone())),
        );
    }
    list.into_any_element()
}

fn browser_settings(v: &mut DeskView, t: &Theme, cx: &mut Context<DeskView>) -> Option<AnyElement> {
    let state = v.browser.as_ref().filter(|_| v.linked && v.panel == Panel::Browser)?;
    let on = state.get("on").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let override_command = state.get("commandFromEnv").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let current = state.get("command").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
    let input = v.browser_command.get_or_insert_with(|| cx.new(|cx| {
        let mut input = nixa_ui::text_input::TextInput::new(cx, "Browser provider command");
        input.set_value(current.clone(), cx);
        input
    })).clone();
    Some(div().flex().flex_col().gap(px(6.))
        .child(div().flex().child(button(t, "browser-toggle".into(), if on { "Turn browser off" } else { "Turn browser on" }, on)
            .on_click(cx.listener(move |v, _, _, cx| super::save_browser(v, serde_json::json!({"on": !on}), cx)))))
        .when(override_command, |el| el.child(div().text_color(t.muted).child(format!("Command set by environment: {current}"))))
        .when(!override_command, |el| el.child(div().flex().gap(px(8.))
            .child(div().flex_1().min_w_0().child(input.clone()))
            .child(button(t, "browser-command-save".into(), "Save command", false)
                .on_click(cx.listener(move |v, _, _, cx| {
                    let command = input.read(cx).value().trim().to_owned();
                    super::save_browser(v, serde_json::json!({"command": command}), cx);
                })))))
        .into_any_element())
}

impl Render for DeskView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let desk = self.desk.clone().map(|c| card(self, &t, &c, "desk", CARD_TITLE, cx));
        let talk = self.talk.clone().map(|c| card(self, &t, &c, "browser", BROWSER_CARD_TITLE, cx));
        let error = self.error.clone();
        let browser = browser_settings(self, &t, cx);
        div()
            .id("desk")
            .debug_selector(|| "desk".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .p(px(14.))
            .gap(px(10.))
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.))
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(title(self.panel)))
            .when(self.panel != Panel::Browser || self.browser.as_ref().and_then(|state| state.get("on")).and_then(serde_json::Value::as_bool) == Some(true), |el| el.child(div().child(model::help(self.panel))))
            .child(div().text_color(t.muted).child(GATE_TEXT))
            .child(switches(self, &t, cx))
            .children(browser)
            .when_some(desk, |el, card| el.child(card))
            .when_some(talk, |el, card| el.child(card))
            .when_some(error, |el, e| el.child(div().text_color(t.red).child(e)))
            .child(div().text_color(t.muted).child(if self.linked { LINKED_TEXT } else { UNLINKED_TEXT }))
            .child(audit(self, &t))
            .when(shows_reads(self), |el| el.child(reads(self, &t, cx)))
            .child(div().text_color(t.muted).child(footer_text(self)))
    }
}
