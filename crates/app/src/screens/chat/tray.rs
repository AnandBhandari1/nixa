use super::format::{CONFIRM_CHARS, WHY_CHARS, clip};
use super::kinds::kind_label;
use super::parts::{KIND_TAG_PR, box_chip, kind_tag};
use super::wire::Command;
use super::{ChatView, send};
use gpui::prelude::*;
use gpui::{AnyElement, Context, div, px, relative};
use nixa_ui::theme::tint;
use nixa_ui::{Icon, Theme};

fn clean_box(view: &ChatView, t: &Theme, cx: &mut Context<ChatView>) -> AnyElement {
    let name = view.snap.name().to_string();
    div()
        .id("chat-clean-ask")
        .w_full()
        .relative()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(tint(t.yellow, 0.47))
        .bg(tint(t.yellow, 0.07))
        .child(kind_tag(t, kind_label("clean", &name)))
        .child(
            div()
                .pr(px(KIND_TAG_PR))
                .text_size(px(12.))
                .text_color(t.foreground)
                .child(format!("Start the chat fresh? {name} begins a new brain chat and keeps a short note of the last lines. Saved memory stays.")),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .justify_end()
                .child(
                    box_chip(t, "chat-clean-no", Icon::X, "Keep", "Keep the chat", t.muted, true)
                        .debug_selector(|| "chat-clean-no".into())
                        .on_click(cx.listener(|view, _, _, cx| {
                            cx.stop_propagation();
                            view.clean_ask = false;
                            cx.notify();
                        })),
                )
                .child(
                    box_chip(t, "chat-clean-yes", Icon::Eraser, "Clean", "Clean the chat now", t.green, true)
                        .debug_selector(|| "chat-clean-yes".into())
                        .on_click(cx.listener(|view, _, _, cx| {
                            cx.stop_propagation();
                            view.clean_ask = false;
                            send(Command::Clean, cx);
                        })),
                ),
        )
        .into_any_element()
}

fn confirm_box(view: &ChatView, t: &Theme, cx: &mut Context<ChatView>) -> Option<AnyElement> {
    let confirm = view.snap.confirm.as_ref()?;
    let name = view.snap.name().to_string();
    let text = if confirm.text.starts_with("Desktop step ") {
        confirm.text.clone()
    } else {
        clip(&confirm.text, CONFIRM_CHARS)
    };
    let why = (!confirm.why.is_empty()).then(|| clip(&confirm.why, WHY_CHARS));
    let yes_id = confirm.id.clone();
    let no_id = confirm.id.clone();
    Some(
        div()
            .id("chat-confirm")
            .w_full()
            .relative()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(t.yellow)
            .child(kind_tag(t, kind_label("confirm", &name)))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(t.blue)
                    .child(format!("{} ASKS", name.to_uppercase())),
            )
            .child(div().text_size(px(13.)).text_color(t.yellow).child(text))
            .children(why.map(|w| div().text_size(px(11.5)).text_color(t.magenta).child(w)))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.5))
                            .text_color(t.magenta)
                            .child("Say yes or no, or click."),
                    )
                    .child(
                        box_chip(
                            t,
                            "chat-confirm-yes",
                            Icon::Check,
                            "Yes",
                            "Allow this action",
                            t.green,
                            true,
                        )
                        .debug_selector(|| "chat-confirm-yes".into())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            send(
                                Command::Confirm {
                                    id: yes_id.clone(),
                                    approve: true,
                                },
                                cx,
                            );
                        })),
                    )
                    .child(
                        box_chip(
                            t,
                            "chat-confirm-no",
                            Icon::X,
                            "No",
                            "Do not do it",
                            t.red,
                            true,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            send(
                                Command::Confirm {
                                    id: no_id.clone(),
                                    approve: false,
                                },
                                cx,
                            );
                        })),
                    ),
            )
            .into_any_element(),
    )
}

pub fn render(
    view: &ChatView,
    t: &Theme,
    question: Option<AnyElement>,
    cx: &mut Context<ChatView>,
) -> Option<AnyElement> {
    let confirm = confirm_box(view, t, cx).or_else(|| crate::screens::talk_card::render(t, view.snap.name(), cx));
    if !view.clean_ask && confirm.is_none() && question.is_none() {
        return None;
    }
    Some(
        div()
            .id("chat-tray")
            .debug_selector(|| "chat-tray".into())
            .w_full()
            .flex_none()
            .max_h(relative(0.45))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .px(px(12.))
            .py(px(10.))
            .gap(px(8.))
            .border_t_1()
            .border_color(t.border)
            .when(view.clean_ask, |el| el.child(clean_box(view, t, cx)))
            .children(confirm)
            .children(question)
            .into_any_element(),
    )
}
