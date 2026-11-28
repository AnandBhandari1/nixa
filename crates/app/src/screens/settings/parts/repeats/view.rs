use super::{Item, Repeats, UNITS, clip, frequency, groups, open_form, save_form, send, status};
use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, ElementId, Hsla, SharedString, Stateful, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon, theme};
use serde_json::json;

fn button(t: &Theme, id: String, label: &str, tip: &str, color: Hsla, which: Option<Icon>, enabled: bool) -> Stateful<Div> {
    let selector = id.clone();
    let hover = t.hover;
    let color = if enabled { color } else { t.dim };
    div()
        .id(ElementId::Name(id.into()))
        .debug_selector(move || selector)
        .min_h(px(25.))
        .px(px(8.))
        .py(px(4.))
        .gap(px(5.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .text_size(px(11.5))
        .text_color(color)
        .tooltip(tooltip(SharedString::from(tip.to_string())))
        .when(enabled, |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        .when(!enabled, |el| el.opacity(0.6))
        .when_some(which, |el, which| el.child(icon(which).size(px(12.)).text_color(color)))
        .when(!label.is_empty(), |el| el.child(label.to_string()))
}

fn line(text: impl Into<SharedString>, color: Hsla, size: f32) -> Div {
    div().text_size(px(size)).text_color(color).whitespace_normal().child(text.into())
}

fn card(t: &Theme, border: Hsla) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(11.))
        .rounded(px(8.))
        .border_1()
        .border_color(border)
        .bg(t.chrome)
}

fn row_buttons() -> Div {
    div().w_full().flex().flex_wrap().gap(px(5.))
}

fn form_box(part: &Repeats, t: &Theme, cx: &mut Context<Repeats>) -> AnyElement {
    let Some(form) = part.form.clone() else {
        return div().into_any_element();
    };
    let busy = part.pending.is_some();
    let mut units = row_buttons();
    for (key, label) in UNITS {
        let on = form.unit == key;
        units = units.child(
            button(t, format!("repeat-unit-{key}"), label, label, if on { t.green } else { t.muted }, on.then_some(Icon::Check), true)
                .on_click(cx.listener(move |part, _, _, cx| {
                    if let Some(f) = part.form.as_mut() {
                        f.unit = key.to_string();
                    }
                    cx.notify();
                })),
        );
    }
    let enabled = form.enabled;
    let mut out = card(t, t.blue.opacity(0.4))
        .id("repeat-form")
        .debug_selector(|| "repeat-form".into())
        .child(line(if form.id.is_some() { "Edit repeat message" } else { "New repeat message" }, t.bright, 13.))
        .child(line("Message to Nixa", t.muted, 11.))
        .child(part.inputs.text.clone().into_any_element())
        .child(line("Send it every", t.muted, 11.))
        .child(div().w(px(120.)).child(part.inputs.every.clone().into_any_element()))
        .child(units)
        .child(
            row_buttons().child(
                button(t, "repeat-form-on".into(), if enabled { "On" } else { "Paused" }, "Send it on the timer, or keep it paused", if enabled { t.green } else { t.muted }, Some(if enabled { Icon::Check } else { Icon::Square }), true)
                    .on_click(cx.listener(|part, _, _, cx| {
                        if let Some(f) = part.form.as_mut() {
                            f.enabled = !f.enabled;
                        }
                        cx.notify();
                    })),
            ),
        );
    if let Some(error) = &form.error {
        out = out.child(line(error.clone(), t.red, 12.));
    }
    out.child(
        row_buttons()
            .child(
                button(t, "repeat-form-save".into(), "Save", "Save the repeat message", t.green, Some(Icon::Check), !busy)
                    .when(!busy, |el| el.on_click(cx.listener(|part, _, _, cx| save_form(part, cx)))),
            )
            .child(
                button(t, "repeat-form-cancel".into(), "Cancel", "Close without saving", t.muted, None, true).on_click(cx.listener(|part, _, _, cx| {
                    part.form = None;
                    cx.notify();
                })),
            ),
    )
    .into_any_element()
}

fn row(part: &Repeats, t: &Theme, item: &Item, cx: &mut Context<Repeats>) -> AnyElement {
    let busy = part.pending.is_some();
    let id = item.id.clone();
    let on = item.enabled;
    let mut buttons = row_buttons();
    let edit_item = item.clone();
    buttons = buttons.child(
        button(t, format!("repeat-edit-{id}"), "Edit", "Change the message or how often", t.muted, Some(Icon::SquarePen), !busy)
            .when(!busy, |el| el.on_click(cx.listener(move |part, _, _, cx| open_form(part, Some(&edit_item), cx)))),
    );
    let toggle_id = id.clone();
    buttons = buttons.child(
        button(t, format!("repeat-toggle-{id}"), if on { "Pause" } else { "Resume" }, if on { "Stop sending it for now" } else { "Send it on the timer again" }, t.muted, Some(if on { Icon::Square } else { Icon::Play }), !busy)
            .when(!busy, |el| {
                el.on_click(cx.listener(move |part, _, _, cx| {
                    send(part, "repeat.toggle", json!({"id": toggle_id, "enabled": !on}), if on { "Pause" } else { "Resume" }, cx)
                }))
            }),
    );
    let send_id = id.clone();
    buttons = buttons.child(
        button(t, format!("repeat-send-{id}"), "Send now", "Send the message to Nixa chat now", t.blue, Some(Icon::Send), !busy)
            .when(!busy, |el| el.on_click(cx.listener(move |part, _, _, cx| send(part, "repeat.send", json!({"id": send_id}), "Send now", cx)))),
    );
    let asking = part.removing.as_deref() == Some(id.as_str());
    if asking {
        let remove_id = id.clone();
        buttons = buttons
            .child(
                button(t, format!("repeat-remove-yes-{id}"), "Yes, remove", "Remove this repeat message", t.red, Some(Icon::Trash), !busy)
                    .when(!busy, |el| el.on_click(cx.listener(move |part, _, _, cx| send(part, "repeat.delete", json!({"id": remove_id}), "Remove", cx)))),
            )
            .child(button(t, format!("repeat-remove-no-{id}"), "Keep", "Do not remove it", t.muted, None, true).on_click(cx.listener(|part, _, _, cx| {
                part.removing = None;
                cx.notify();
            })));
    } else {
        let ask_id = id.clone();
        buttons = buttons.child(
            button(t, format!("repeat-remove-{id}"), "Remove", "Remove this repeat message", t.red, Some(Icon::Trash), !busy).when(!busy, |el| {
                el.on_click(cx.listener(move |part, _, _, cx| {
                    part.removing = Some(ask_id.clone());
                    cx.notify();
                }))
            }),
        );
    }
    card(t, if on { t.border } else { t.border.opacity(0.6) })
        .id(SharedString::from(format!("repeat-{id}")))
        .child(line(clip(&item.text), if on { t.bright } else { t.muted }, 12.5))
        .child(line(status(item), t.dim, 11.))
        .when(item.sent > 0 && !item.view.last.is_empty(), |el| el.child(line(format!("Last sent: {}", item.view.last), t.dim, 11.)))
        .when_some(item.last_note.as_ref(), |el, note| el.child(line(format!("Last attempt: {note}"), t.red, 11.)))
        .child(buttons)
        .into_any_element()
}

pub fn render(part: &mut Repeats, cx: &mut Context<Repeats>) -> AnyElement {
    let t = theme(cx).clone();
    let Some(snap) = part.snap.clone() else {
        let text = match &part.load_error {
            Some(e) => format!("Nixa is not connected ({e}). It reconnects by itself."),
            None => "Loading repeat messages…".into(),
        };
        return div().id("repeats").size_full().flex().items_center().justify_center().p(px(20.)).child(line(text, t.dim, 12.)).into_any_element();
    };
    let header = div()
        .w_full()
        .flex_none()
        .px(px(4.))
        .py(px(10.))
        .gap(px(8.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(t.border)
        .child(icon(Icon::Refresh).size(px(15.)).text_color(t.blue))
        .child(div().flex_1().min_w_0().child(line("Repeat messages go to Nixa chat on a timer. They never count as your approval.", t.dim, 11.5)))
        .child(
            button(&t, "repeat-new".into(), "New repeat message", "Add a repeat message", t.green, Some(Icon::Plus), true)
                .on_click(cx.listener(|part, _, _, cx| open_form(part, None, cx))),
        );
    let mut list = div().id("repeats-list").flex_1().min_h_0().overflow_y_scroll().py(px(12.)).flex().flex_col().gap(px(10.));
    if let Some(error) = &part.load_error {
        list = list.child(card(&t, t.red.opacity(0.4)).child(line(error.clone(), t.red, 12.)));
    }
    if let Some(notice) = part.notice.clone() {
        let color = if notice.ok { t.green } else { t.red };
        list = list.child(
            card(&t, color.opacity(0.4)).id("repeat-notice").child(
                div()
                    .flex()
                    .gap(px(6.))
                    .items_center()
                    .child(div().flex_1().child(line(notice.text, color, 12.)))
                    .child(button(&t, "repeat-notice-close".into(), "", "Dismiss", t.muted, Some(Icon::X), true).on_click(cx.listener(|part, _, _, cx| {
                        part.notice = None;
                        cx.notify();
                    }))),
            ),
        );
    }
    if part.form.is_some() {
        list = list.child(form_box(part, &t, cx));
    }
    for (minutes, items) in groups(&snap.repeats) {
        let group = div().id(SharedString::from(format!("repeat-group-{minutes}"))).flex().flex_col().gap(px(6.))
            .child(line(format!("{} · {} {}", frequency(minutes), items.len(), if items.len() == 1 { "message" } else { "messages" }), t.bright, 13.))
            .children(items.into_iter().map(|item| row(part, &t, item, cx)));
        list = list.child(group);
    }
    if snap.repeats.is_empty() && part.form.is_none() {
        list = list.child(card(&t, t.border).id("repeats-empty").child(line("No repeat messages yet. Click New repeat message.", t.dim, 12.)));
    }
    div()
        .id("repeats")
        .track_focus(&part.focus)
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .text_size(px(12.8))
        .child(header)
        .child(list)
        .into_any_element()
}
