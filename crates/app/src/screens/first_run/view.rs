use super::key::{self, Brain, Saving};
use super::perms::{self, Grant, Perm};
use super::voice::{CREDIT, ModelsState, size_words};
use super::{FirstRunView, steps_shown};
use crate::screens::settings::parts::chat_brain::{kind_title, kind_tip};
use gpui::prelude::*;
use gpui::{ClickEvent, Context, Div, FontWeight, Hsla, SharedString, Stateful, Window, div, px};
use nixa_ui::button::{ButtonKind, button, disabled, icon_label_button};
use nixa_ui::{Icon, Theme, icon, text, theme};

pub const WIDTH: f32 = 660.;
pub const SHOWN_SPACES: usize = 3;

type Cx<'a> = Context<'a, FirstRunView>;

fn words(color: Hsla, size: f32, s: impl Into<SharedString>) -> Div {
    div().text_size(px(size)).text_color(color).child(s.into())
}

fn tagged(el: Stateful<Div>, tag: String) -> Stateful<Div> {
    el.debug_selector(move || tag)
}

fn act(
    el: Stateful<Div>,
    off: bool,
    cx: &mut Cx,
    run: impl Fn(&mut FirstRunView, &mut Cx) + 'static,
) -> Stateful<Div> {
    let el = disabled(el, off);
    if off {
        return el;
    }
    el.on_click(cx.listener(move |v, _: &ClickEvent, _: &mut Window, cx| {
        run(v, cx);
        cx.notify();
    }))
}

fn mark(t: &Theme, step: usize, ok: bool) -> Div {
    let base = div()
        .flex_none()
        .size(px(22.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center();
    if ok {
        base.bg(t.green.opacity(0.16))
            .child(icon(Icon::Check).size(px(13.)).text_color(t.green))
    } else {
        base.border_1()
            .border_color(t.border_strong)
            .text_size(px(text::SM))
            .text_color(t.muted)
            .child(SharedString::from(step.to_string()))
    }
}

fn card(t: &Theme, step: usize, title: &str, ok: bool) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(16.))
        .rounded(px(10.))
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(mark(t, step, ok))
                .child(
                    div()
                        .text_size(px(text::MD))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.bright)
                        .child(SharedString::from(title.to_string())),
                ),
        )
}

fn done_line(t: &Theme, s: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(icon(Icon::CircleCheck).size(px(14.)).text_color(t.green))
        .child(words(t.foreground, text::BASE, s))
}

pub fn space_words(names: &[String]) -> String {
    let shown: Vec<&str> = names.iter().take(SHOWN_SPACES).map(String::as_str).collect();
    let more = names.len().saturating_sub(SHOWN_SPACES);
    let list = shown.join(", ");
    match (names.len(), more) {
        (1, _) => format!("Your space: {list}"),
        (_, 0) => format!("Your spaces: {list}"),
        _ => format!("Your spaces: {list} and {more} more"),
    }
}

fn need_space_note(t: &Theme, cx: &mut Cx) -> Div {
    let add = tagged(
        icon_label_button(t, "first-run-need-space-add", Icon::FolderPlus, "Add space", ButtonKind::Primary),
        "first-run-need-space-add".into(),
    );
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.))
        .p(px(12.))
        .rounded(px(10.))
        .border_1()
        .border_color(t.yellow.opacity(0.5))
        .bg(t.yellow.opacity(0.08))
        .debug_selector(|| "first-run-need-space".into())
        .child(icon(Icon::CircleAlert).size(px(16.)).text_color(t.yellow))
        .child(div().flex_1().min_w_0().child(words(t.foreground, text::BASE, super::NEED_SPACE)))
        .child(act(add, false, cx, |_, cx| super::pick_space(cx)))
}

fn space_card(v: &FirstRunView, t: &Theme, step: usize, cx: &mut Cx) -> Div {
    let has = !v.spaces.is_empty();
    let title = if has { "Spaces" } else { "Add your first space" };
    let mut el = card(t, step, title, has);
    if has {
        el = el.child(done_line(t, space_words(&v.spaces)));
        let more = tagged(
            icon_label_button(t, "first-run-add-space", Icon::FolderPlus, "Add another space", ButtonKind::Secondary),
            "first-run-add-space".into(),
        );
        return el.child(div().flex().child(act(more, false, cx, |_, cx| super::pick_space(cx))));
    }
    let add = tagged(
        icon_label_button(t, "first-run-add-space", Icon::FolderPlus, "Add your first space", ButtonKind::Primary),
        "first-run-add-space".into(),
    );
    el.child(words(
        t.muted,
        text::BASE,
        "A space is a project folder. Your agents work inside it. Nixa starts with no spaces, so pick a folder.",
    ))
    .child(div().flex().child(act(add, false, cx, |_, cx| super::pick_space(cx))))
}

fn perm_icon(p: Perm) -> Icon {
    match p {
        Perm::Mic => Icon::Mic,
        Perm::InputMonitoring => Icon::Commands,
        Perm::Accessibility => Icon::Shield,
        Perm::ScreenRecording => Icon::Monitor,
    }
}

fn grant_color(t: &Theme, g: Grant) -> Hsla {
    match g {
        Grant::Allowed => t.green,
        Grant::Denied => t.red,
        Grant::NotAsked => t.yellow,
        Grant::Unknown => t.muted,
    }
}

fn perm_row(t: &Theme, p: Perm, g: Grant, cx: &mut Cx) -> Div {
    let slug = perms::slug(p);
    let mut buttons = div().flex_none().flex().items_center().gap(px(6.));
    if perms::can_ask(p, g) {
        let id = format!("first-run-ask-{slug}");
        let el = tagged(button(t, SharedString::from(id.clone()), "Ask", ButtonKind::Secondary), id);
        buttons = buttons.child(act(el, false, cx, move |v, _| super::ask(v, p)));
    }
    if g != Grant::Allowed {
        let id = format!("first-run-open-{slug}");
        let el = tagged(button(t, SharedString::from(id.clone()), "Open System Settings", ButtonKind::Ghost), id);
        buttons = buttons.child(act(el, false, cx, move |v, _| super::open_settings(v, p)));
    }
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.))
        .py(px(6.))
        .child(icon(perm_icon(p)).size(px(16.)).text_color(t.muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(text::BASE))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.foreground)
                                .child(SharedString::from(perms::title(p))),
                        )
                        .child(words(grant_color(t, g), text::SM, perms::grant_words(g))),
                )
                .child(words(t.muted, text::SM, perms::why(p))),
        )
        .child(buttons)
}

fn perm_card(v: &FirstRunView, t: &Theme, step: usize, cx: &mut Cx) -> Div {
    let all = perms::allowed_count(&v.grants) == v.grants.len();
    let mut el = card(t, step, "Permissions", all).child(words(
        t.muted,
        text::BASE,
        "Each one is optional. A part without its permission stays off. You can change them any time in System Settings.",
    ));
    for (p, g) in v.grants.clone() {
        el = el.child(perm_row(t, p, g, cx));
    }
    if let Some(p) = v.asked_perm
        && v.grants.iter().any(|(q, g)| *q == p && *g != Grant::Allowed)
        && !perms::after_ask(p).is_empty()
    {
        el = el.child(words(t.yellow, text::SM, perms::after_ask(p)));
    }
    let again = tagged(button(t, "first-run-check", "Check again", ButtonKind::Ghost), "first-run-check".into());
    el.child(div().flex().child(act(again, false, cx, |v, _| {
        super::check_perms(v);
    })))
}

fn credit(t: &Theme) -> Div {
    words(t.dim, text::XS, CREDIT).debug_selector(|| "first-run-model-credit".into())
}

fn voice_card(v: &FirstRunView, t: &Theme, step: usize, cx: &mut Cx) -> Div {
    let ready = matches!(&v.models, ModelsState::Known(m) if m.ready);
    let el = card(t, step, "Voice models", ready);
    let el = match &v.models {
        ModelsState::Loading => el.child(words(t.muted, text::BASE, "Checking the voice models...")),
        ModelsState::Known(m) if m.ready => el.child(done_line(t, "Voice models are ready. Speech to text runs on this Mac.")),
        ModelsState::Known(m) => {
            let get = tagged(
                button(t, "first-run-fetch", "Download voice models", ButtonKind::Primary),
                "first-run-fetch".into(),
            );
            el.child(words(
                t.muted,
                text::BASE,
                format!(
                    "Speech to text runs on this Mac, so nothing you say leaves it. Nixa needs {} first.",
                    m.missing.join(" and ")
                ),
            ))
            .child(words(
                t.muted,
                text::SM,
                format!(
                    "The download is {} from the sherpa-onnx release page. Every file is checked before it is used. You can skip this; voice stays off until then.",
                    size_words(m.bytes)
                ),
            ))
            .child(div().flex().child(act(get, false, cx, super::start_fetch)))
        }
        ModelsState::Fetching { line } => el.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(Icon::Spinner).size(px(14.)).text_color(t.blue))
                .child(words(t.foreground, text::BASE, line.clone())),
        ),
        ModelsState::Failed(why) => {
            let again = tagged(button(t, "first-run-models-again", "Check again", ButtonKind::Secondary), "first-run-models-again".into());
            el.child(words(t.red, text::BASE, why.clone()))
                .child(div().flex().child(act(again, false, cx, super::load_models)))
        }
    };
    el.child(credit(t))
}

fn kind_chip(v: &FirstRunView, t: &Theme, kind: &'static str, cx: &mut Cx) -> Stateful<Div> {
    let on = v.kind == kind;
    let id = format!("first-run-kind-{kind}");
    let which = if on { ButtonKind::Primary } else { ButtonKind::Secondary };
    let el = tagged(button(t, SharedString::from(id.clone()), kind_title(kind), which), id);
    act(el, v.saving == Saving::Checking, cx, move |v, _| super::pick_kind(v, kind))
}

fn brain_card(v: &FirstRunView, t: &Theme, step: usize, cx: &mut Cx) -> Div {
    let ready = matches!(v.brain, Brain::Ready { .. });
    let el = card(t, step, "Chat brain", ready);
    let other = tagged(
        button(t, "first-run-brain-settings", "Use a coding agent or a local server instead", ButtonKind::Ghost),
        "first-run-brain-settings".into(),
    );
    let other = act(other, false, cx, |_, cx| super::brain_settings(cx));
    match &v.brain {
        Brain::Loading => el.child(words(t.muted, text::BASE, "Looking for Nixa's brain...")),
        Brain::Down(why) => {
            let again = tagged(button(t, "first-run-brain-again", "Try again", ButtonKind::Secondary), "first-run-brain-again".into());
            el.child(words(t.muted, text::BASE, "Nixa's brain is not answering yet. It starts with the app, so wait a moment."))
                .child(words(t.dim, text::SM, why.clone()))
                .child(div().flex().child(act(again, false, cx, super::load_brain)))
        }
        Brain::Ready { name } => {
            let change = tagged(button(t, "first-run-brain-settings", "Change in Settings", ButtonKind::Ghost), "first-run-brain-settings".into());
            el.child(done_line(t, format!("Chat brain: {name}")))
                .child(div().flex().child(act(change, false, cx, |_, cx| super::brain_settings(cx))))
        }
        Brain::Missing => {
            let checking = v.saving == Saving::Checking;
            let label = if checking { "Checking the key..." } else { "Check and save" };
            let save = tagged(button(t, "first-run-save-key", label, ButtonKind::Primary), "first-run-save-key".into());
            let mut kinds = div().flex().items_center().gap(px(6.));
            for kind in key::KINDS {
                kinds = kinds.child(kind_chip(v, t, kind, cx));
            }
            let mut el = el
                .child(words(
                    t.muted,
                    text::BASE,
                    "Nixa needs a chat brain to talk and decide. Paste an API key from your own account. The key stays in the macOS keychain.",
                ))
                .child(kinds)
                .child(words(t.dim, text::SM, kind_tip(&v.kind)))
                .child(words(t.dim, text::SM, key::key_help(&v.kind)))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .debug_selector(|| "first-run-key".into())
                        .child(v.key.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(act(save, checking, cx, super::save_key))
                        .child(other),
                );
            if let Saving::Failed(why) = &v.saving {
                el = el.child(words(t.red, text::SM, why.clone()));
            }
            el
        }
    }
}

fn footer(v: &FirstRunView, t: &Theme, cx: &mut Cx) -> Div {
    let none = v.spaces.is_empty();
    let start = tagged(
        button(t, "first-run-finish", "Start using Nixa", ButtonKind::Primary),
        "first-run-finish".into(),
    );
    let note = if none {
        "Add a space to start. The other steps can wait."
    } else {
        "You can open these steps again with Set up Nixa in the command list."
    };
    div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .child(words(t.muted, text::SM, note))
        .child(act(start, none, cx, super::finish))
}

fn close_button(v: &FirstRunView, t: &Theme, cx: &mut Cx) -> Option<Stateful<Div>> {
    if v.spaces.is_empty() {
        return None;
    }
    let el = tagged(
        icon_label_button(t, "first-run-close", Icon::Check, "Done", ButtonKind::Secondary),
        "first-run-close".into(),
    );
    Some(act(el, false, cx, super::finish))
}

fn header(v: &FirstRunView, t: &Theme, steps: bool, cx: &mut Cx) -> Div {
    let (title, about) = if steps {
        ("Welcome to Nixa", "Set up a few things. All of them can change later in Settings.")
    } else {
        ("No spaces yet", "Add a project folder to start work.")
    };
    let words_col = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(text::XL))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.bright)
                .child(SharedString::from(title)),
        )
        .child(words(t.muted, text::BASE, about));
    div()
        .w_full()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(words_col)
        .when_some(close_button(v, t, cx), |el, b| el.child(b))
}

impl Render for FirstRunView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_pending) {
            self.previous_focus = window.focused(cx);
            let focus = self.focus.clone();
            cx.defer_in(window, move |_, window, _| window.focus(&focus));
        }
        let t = theme(cx).clone();
        let steps = steps_shown(self);
        let mut col = div()
            .w_full()
            .max_w(px(WIDTH))
            .flex()
            .flex_col()
            .gap(px(14.))
            .py(px(28.))
            .px(px(20.))
            .when(self.need_space && self.spaces.is_empty(), |col| col.child(need_space_note(&t, cx)))
            .child(header(self, &t, steps, cx))
            .child(space_card(self, &t, 1, cx));
        if steps {
            col = col
                .child(perm_card(self, &t, 2, cx))
                .child(voice_card(self, &t, 3, cx))
                .child(brain_card(self, &t, 4, cx))
                .child(footer(self, &t, cx));
        }
        div()
            .id("first-run")
            .debug_selector(|| "first-run".into())
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && super::close(view) {
                    if let Some(focus) = view.previous_focus.take() {
                        window.focus(&focus);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .size_full()
            .overflow_y_scroll()
            .bg(t.canvas)
            .flex()
            .justify_center()
            .child(col)
    }
}
