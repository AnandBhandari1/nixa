use crate::screens::nixa_screen::office::soap::Models;
use crate::screens::nixa_screen::prefs::Look;
use crate::screens::nixa_screen::{self, KEY, NixaScreen};
use gpui::prelude::*;
use gpui::{AnyElement, App, Div, Entity, MouseButton, SharedString, Window, div, px};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::popover::tooltip;
use nixa_ui::text_input::TextInput;
use nixa_ui::toggle::toggle_row;
use nixa_ui::{Theme, theme};

pub const PART: &str = nixa_screen::PART;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartAction {
    KeepOpen,
    Quiet,
    Office,
    Soap,
    Show,
}

pub fn act(screen: &Entity<NixaScreen>, action: PartAction, window: &mut Window, cx: &mut App) {
    screen.update(cx, |s, cx| match action {
        PartAction::KeepOpen => {
            let on = !s.prefs.keep_open;
            s.set_keep(on, cx);
        }
        PartAction::Quiet => {
            let on = !s.prefs.quiet;
            s.set_quiet(on, cx);
        }
        PartAction::Office => {
            let next = if s.prefs.look == Look::Office { Look::Universe } else { Look::Office };
            s.set_look(next, cx);
        }
        PartAction::Soap => {
            let on = !s.prefs.soap;
            s.set_soap(on, cx);
        }
        PartAction::Show => s.request_show(cx),
    });
    if action == PartAction::Show {
        window.remove_window();
    } else {
        window.refresh();
    }
}

fn card(t: &Theme) -> gpui::Div {
    div().rounded(px(8.0)).border_1().border_color(t.border)
}

pub fn model_line(m: &Models) -> String {
    if m.waiting && !m.known {
        return "Asking the brain for the models...".into();
    }
    if m.model.trim().is_empty() {
        return "Model: not known yet.".into();
    }
    if m.url.trim().is_empty() {
        format!("Model: {}", m.model)
    } else {
        format!("Model: {} at {}", m.model, m.url)
    }
}

fn model_chip(t: &Theme, i: usize, name: &str, on: bool) -> gpui::Stateful<Div> {
    let id = format!("nixa-soap-model-{i}");
    let hover = t.hover;
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .h(px(22.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .border_1()
        .border_color(if on { t.accent } else { t.border })
        .cursor_pointer()
        .text_size(px(11.0))
        .whitespace_nowrap()
        .text_color(if on { t.foreground } else { t.muted })
        .when(on, |el| el.bg(t.selected))
        .hover(move |s| s.bg(hover))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(name.to_string())
}

fn soap_models(t: &Theme, screen: &Entity<NixaScreen>, m: &Models, input: Entity<TextInput>) -> Div {
    let mut chips = div().flex().flex_wrap().gap(px(6.0));
    for (i, name) in m.models.iter().enumerate() {
        let pick = screen.clone();
        let chosen = name.clone();
        chips = chips.child(
            model_chip(t, i, name, *name == m.model)
                .tooltip(tooltip(format!("Use {name} for office soap")))
                .on_click(move |_, window, cx| {
                    pick.update(cx, |s, cx| s.pick_soap_model(&chosen, cx));
                    window.refresh();
                }),
        );
    }
    let on_use = screen.clone();
    let typed = input.clone();
    let on_refresh = screen.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p(px(10.0))
        .child(div().text_size(px(12.0)).text_color(t.foreground).child(model_line(m)))
        .when(m.known && m.models.is_empty() && m.error.is_none(), |el| {
            el.child(
                div()
                    .text_size(px(11.0))
                    .text_color(t.muted)
                    .child("No models are installed in Ollama yet."),
            )
        })
        .when(!m.models.is_empty(), |el| el.child(chips))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .debug_selector(|| "nixa-soap-field".into())
                        .child(input),
                )
                .child(
                    button(t, "nixa-soap-use", "Use", ButtonKind::Secondary)
                        .debug_selector(|| "nixa-soap-use".into())
                        .tooltip(tooltip("Use the typed model name"))
                        .on_click(move |_, window, cx| {
                            let name = typed.read(cx).value().to_string();
                            on_use.update(cx, |s, cx| s.pick_soap_model(&name, cx));
                            window.refresh();
                        }),
                )
                .child(
                    button(t, "nixa-soap-refresh", "Refresh", ButtonKind::Ghost)
                        .debug_selector(|| "nixa-soap-refresh".into())
                        .tooltip(tooltip("Ask Ollama again for the installed models"))
                        .on_click(move |_, window, cx| {
                            on_refresh.update(cx, |s, cx| s.ask_soap_models(cx));
                            window.refresh();
                        }),
                ),
        )
        .when_some(m.error.clone(), |el, e| {
            el.child(
                div()
                    .debug_selector(|| "nixa-soap-error".into())
                    .text_size(px(12.0))
                    .text_color(t.yellow)
                    .child(e),
            )
        })
}

pub fn render(cx: &mut App) -> AnyElement {
    let t = theme(cx).clone();
    let Some(screen) = nixa_screen::shared(cx) else {
        return div()
            .id("nixa-screen-part")
            .text_color(t.muted)
            .child("The Nixa screen is not ready yet.")
            .into_any_element();
    };
    let (prefs, name, error, models) = {
        let s = screen.read(cx);
        (s.prefs, s.feed.name().to_string(), s.error.clone(), s.soap_models.clone())
    };
    let input = screen.update(cx, |s, cx| s.soap_field(cx));
    let on_soap = screen.clone();
    let on_keep = screen.clone();
    let on_quiet = screen.clone();
    let on_show = screen.clone();
    let on_office = screen.clone();
    div()
        .id("nixa-screen-part")
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            card(&t).child(
                toggle_row(
                    &t,
                    "nixa-screen-keep",
                    "Keep open",
                    Some(SharedString::from(format!(
                        "The {name} screen opens by itself when the app starts. Esc does not close it; only × does."
                    ))),
                    prefs.keep_open,
                )
                .debug_selector(|| "nixa-screen-keep".into())
                .on_click(move |_, window, cx| act(&on_keep, PartAction::KeepOpen, window, cx)),
            ),
        )
        .child(
            card(&t).child(
                toggle_row(
                    &t,
                    "nixa-screen-quiet",
                    "Quiet galaxy",
                    Some("Stop the slow glow breathing and the small bot sway.".into()),
                    prefs.quiet,
                )
                .debug_selector(|| "nixa-screen-quiet".into())
                .on_click(move |_, window, cx| act(&on_quiet, PartAction::Quiet, window, cx)),
            ),
        )
        .child(
            card(&t).child(
                toggle_row(
                    &t,
                    "nixa-screen-office",
                    "Office look",
                    Some("Show your agents as people in an office instead of a galaxy.".into()),
                    prefs.look == Look::Office,
                )
                .debug_selector(|| "nixa-screen-office".into())
                .on_click(move |_, window, cx| act(&on_office, PartAction::Office, window, cx)),
            ),
        )
        .child(
            card(&t)
                .child(
                    toggle_row(
                        &t,
                        "nixa-screen-soap",
                        "Office soap (local model)",
                        Some(SharedString::from(format!(
                            "Small office stories by a local Ollama model. It only runs while the Office view is open and {name} is idle."
                        ))),
                        prefs.soap,
                    )
                    .debug_selector(|| "nixa-screen-soap".into())
                    .on_click(move |_, window, cx| act(&on_soap, PartAction::Soap, window, cx)),
                )
                .child(div().border_t_1().border_color(t.border).child(soap_models(&t, &screen, &models, input))),
        )
        .child(
            div().flex().child(
                button(&t, "nixa-screen-show", format!("Show the {name} screen"), ButtonKind::Secondary)
                    .debug_selector(|| "nixa-screen-show".into())
                    .tooltip(tooltip(format!("Show the {name} screen now ({KEY})")))
                    .on_click(move |_, window, cx| act(&on_show, PartAction::Show, window, cx)),
            ),
        )
        .when_some(error, |el, e| el.child(div().text_size(px(12.0)).text_color(t.yellow).child(e)))
        .into_any_element()
}
