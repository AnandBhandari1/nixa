use super::tree::{LEVELS, LevelInfo, Row, explicit_voice, resolved_for, shown_rows, voice_name};
use super::{VoicesView, choose, clear, current_voice, is_playing, play, reset, rows, toggle_humor, use_voice};
use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, FontWeight, SharedString, Stateful, Window, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::{Theme, theme};
use serde_json::json;

pub const LIST_WIDTH: f32 = 270.;
pub const INDENT: f32 = 14.;
pub const AUTO_LINE: &str = "Automatic voices by agent number. Manual rules still win.";
pub const AUTO_TIP: &str = "Agent 1 uses voice 1, agent 2 uses voice 2, wrapping after the last voice. Any manual rule below wins.";
pub const ROWS_HINT: &str = "Rows below show the voice each one uses now and the rule that chose it. Play speaks a sample in that voice. Rows for an agent, space or numbered agent show the voice for a finished agent.";
pub const HUMOR_TIP: &str = "Light humor only in Nixa's own spoken replies to you. Facts come first. Never in serious replies, questions or agent messages.";

fn small(t: &Theme, id: String, text: impl Into<SharedString>, on: bool, tip: String) -> Stateful<gpui::Div> {
    let hover = t.hover;
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(7.))
        .py(px(3.))
        .rounded(px(4.))
        .cursor_pointer()
        .bg(if on { t.selected } else { t.background })
        .border_1()
        .border_color(t.border)
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(tip))
        .child(text.into())
}

fn source_line(v: &VoicesView) -> &str {
    match &v.state {
        Some(s) if !s.note.is_empty() => &s.note,
        Some(s) if s.voice_on == Some(false) => "Voice is off.",
        Some(s) if s.listed_by_tool() => "In the same order as the voice tool lists them",
        Some(_) => "The voice tool list is not available. Showing the built-in list.",
        None => "Connecting to Nixa…",
    }
}

fn header(v: &VoicesView, t: &Theme, cx: &mut Context<VoicesView>) -> AnyElement {
    let mut row = div().flex().flex_row().items_center().gap(px(10.)).px(px(16.)).py(px(10.)).child(
        div()
            .id("voices-auto")
            .flex_1()
            .min_w_0()
            .text_color(t.muted)
            .tooltip(tooltip(AUTO_TIP))
            .child(AUTO_LINE),
    );
    if v.busy {
        row = row.child(div().id("voices-saving").flex_none().text_color(t.muted).child("Saving…"));
    } else if v.confirm_reset {
        row = row.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .child(div().text_color(t.red).child("Remove every voice rule?"))
                .child(small(t, "voices-reset-yes".into(), "Yes, reset", false, "Return to automatic agent voices and built-in voices for other lines".into()).on_click(cx.listener(|v, _: &ClickEvent, _, cx| reset(v, cx))))
                .child(small(t, "voices-reset-no".into(), "Keep", false, "Keep the voice rules".into()).on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                    v.confirm_reset = false;
                    cx.notify();
                }))),
        );
    } else {
        row = row.child(small(t, "voices-reset".into(), "Reset voice rules", false, "Remove manual rules and return to automatic agent voices".into()).on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
            v.confirm_reset = true;
            cx.notify();
        })));
    }
    row.into_any_element()
}

fn voice_row(v: &VoicesView, t: &Theme, voice: &str, number: usize, current: Option<&str>, cx: &mut Context<VoicesView>) -> AnyElement {
    let normal = v.state.as_ref().is_some_and(|s| s.is_normal(voice));
    let used = current.is_some_and(|c| c.eq_ignore_ascii_case(voice));
    let use_button = small(t, format!("voices-use-{voice}"), if used { "Chosen" } else { "Use" }, used, format!("Use {voice} for: {}", v.target.label));
    let use_name = voice.to_string();
    let use_button = if v.busy {
        nixa_ui::button::disabled(use_button, true)
    } else {
        use_button.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| use_voice(v, &use_name, cx)))
    };
    let key = format!("voice:{voice}");
    let playing = is_playing(v, &key);
    let sample = v.state.as_ref().map(|s| s.sample.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| "a short sample".into());
    let (name, play_name) = (voice.to_string(), voice.to_string());
    div()
        .id(SharedString::from(format!("voices-voice-{voice}")))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(3.))
        .border_b_1()
        .border_color(t.border)
        .child(div().w(px(24.)).flex_none().text_color(t.dim).child(number.to_string()))
        .child(
            div()
                .id(SharedString::from(format!("voices-name-{voice}")))
                .flex_1()
                .min_w_0()
                .truncate()
                .tooltip(tooltip(if normal { format!("{voice} is in the phone's voice rotation") } else { format!("{voice} is a character voice") }))
                .child(name),
        )
        .child(div().flex_none().text_color(t.dim).child(if normal { "phone" } else { "" }))
        .child(
            small(t, format!("voices-play-{voice}"), if playing { "Stop" } else { "Play" }, playing, format!("Hear {voice} say: {sample}"))
                .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| play(v, &format!("voice:{play_name}"), json!({"voice": play_name}), cx))),
        )
        .child(use_button)
        .into_any_element()
}

fn voice_list(v: &VoicesView, t: &Theme, cx: &mut Context<VoicesView>) -> AnyElement {
    let voices = v.state.as_ref().map(|s| s.voices.clone()).unwrap_or_default();
    let current = current_voice(v);
    let mut list = div().id("voices-list").flex_1().min_h_0().flex().flex_col().overflow_y_scroll().track_scroll(&v.voices_scroll);
    for (index, voice) in voices.iter().enumerate() {
        list = list.child(voice_row(v, t, voice, index + 1, current.as_deref(), cx));
    }
    list = list.child(div().w_full().h(px(24.)).flex_none());
    div()
        .w(px(LIST_WIDTH))
        .flex_none()
        .h_full()
        .min_h_0()
        .flex()
        .flex_col()
        .border_r_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .px(px(8.))
                .py(px(6.))
                .border_b_1()
                .border_color(t.border)
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Voices ({})", voices.len())))
                .child(
                    div()
                        .id("voices-target")
                        .debug_selector(|| "voices-target".into())
                        .truncate()
                        .text_color(t.muted)
                        .tooltip(tooltip(v.target.label.clone()))
                        .child(format!("Choosing for: {}", v.target.label)),
                )
                .child(div().text_color(t.dim).child(source_line(v).to_string())),
        )
        .child(list)
        .into_any_element()
}

fn rule_row(v: &VoicesView, t: &Theme, row: &Row, cx: &mut Context<VoicesView>) -> AnyElement {
    let state = v.state.as_ref();
    let resolved = resolved_for(state, row);
    let own = state.and_then(|s| explicit_voice(&s.map, row));
    let selected = v.target.id == row.id;
    let key = format!("row:{}", row.id);
    let playing = is_playing(v, &key);
    let line = resolved.map(|r| format!("{} · {}", voice_name(&r.voice), r.rule.text)).unwrap_or_else(|| "…".into());
    let rule_text = resolved.map(|r| r.rule.text.clone()).unwrap_or_default();
    let uses = resolved.map(|r| voice_name(&r.voice)).unwrap_or_default();
    let humor_on = state.is_some_and(|s| s.humor_on());
    let (pick_row, clear_row, ctx) = (row.clone(), row.clone(), row.ctx.clone());
    let id = row.id.clone();
    let top = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .id(SharedString::from(format!("voices-label-{}", row.id)))
                .flex_1()
                .min_w_0()
                .tooltip(tooltip(row.hint.clone()))
                .child(row.label.clone()),
        )
        .when(row.id == "kind:chat", |el| {
            el.child(
                nixa_ui::button::disabled(small(t, "voices-humor".into(), format!("Humor: {}", if humor_on { "on" } else { "off" }), humor_on, HUMOR_TIP.into()), v.busy)
                    .when(!v.busy, |el| el.on_click(cx.listener(|v, _: &ClickEvent, _, cx| toggle_humor(v, cx)))),
            )
        })
        .child(
            small(t, format!("voices-pick-{}", row.id), if selected { "Choosing" } else { "Voice" }, selected, format!("Pick the voice for {} from the list on the left", row.label))
                .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| choose(v, &pick_row, cx))),
        )
        .when(own.is_some(), |el| {
            el.child(
                nixa_ui::button::disabled(small(t, format!("voices-clear-{}", row.id), "Clear", false, format!("Remove the voice set on {}", row.label)), v.busy)
                    .when(!v.busy, |el| el.on_click(cx.listener(move |v, _: &ClickEvent, _, cx| clear(v, &clear_row, cx)))),
            )
        })
        .child(
            small(t, format!("voices-rowplay-{}", row.id), if playing { "Stop" } else { "Play" }, playing, format!("Hear the voice this row uses: {uses}"))
                .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| play(v, &key, json!({"ctx": ctx}), cx))),
        );
    div()
        .id(SharedString::from(format!("voices-row-{}", row.id)))
        .debug_selector(move || format!("voices-row-{id}"))
        .w_full()
        .flex()
        .flex_col()
        .gap(px(2.))
        .py(px(5.))
        .pr(px(8.))
        .pl(px(8. + row.indent as f32 * INDENT))
        .border_b_1()
        .border_color(t.border)
        .bg(if selected { t.selected } else { t.background })
        .child(top)
        .child(
            div()
                .id(SharedString::from(format!("voices-rule-{}", row.id)))
                .w_full()
                .text_color(t.muted)
                .text_size(px(11.))
                .tooltip(tooltip(rule_text))
                .child(line),
        )
        .into_any_element()
}

fn section(v: &VoicesView, t: &Theme, level: &LevelInfo, list: &[Row], cx: &mut Context<VoicesView>) -> AnyElement {
    let mut out = div().w_full().flex().flex_col().child(
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(8.))
            .px(px(8.))
            .py(px(5.))
            .bg(t.hover)
            .font_weight(FontWeight::SEMIBOLD)
            .child(div().flex_none().child(format!("Level {}", level.number)))
            .child(div().flex_none().child(level.title))
            .child(div().flex_1().min_w(px(120.)).text_color(t.dim).font_weight(FontWeight::NORMAL).child(level.hint)),
    );
    for row in list {
        out = out.child(rule_row(v, t, row, cx));
    }
    out.into_any_element()
}

fn rule_list(v: &VoicesView, t: &Theme, cx: &mut Context<VoicesView>) -> AnyElement {
    let list = rows(v);
    let mut body = div().id("voices-rules").flex_1().min_w_0().h_full().min_h_0().flex().flex_col().overflow_y_scroll().track_scroll(&v.rows_scroll);
    if v.state.is_none() {
        let words = if v.reached || v.error.is_none() { "Loading the voices…" } else { "Connecting to Nixa…" };
        return body.child(div().p(px(16.)).text_color(t.dim).child(words)).into_any_element();
    }
    body = body.child(div().px(px(8.)).py(px(5.)).text_color(t.dim).child(ROWS_HINT));
    for level in LEVELS.iter() {
        let shown = shown_rows(&list, v.state.as_ref(), level.id, v.show_models);
        body = body.child(section(v, t, level, &shown, cx));
    }
    let models = list.iter().filter(|r| r.model).count();
    if models > 0 {
        let words = if v.show_models { "Hide unused models".to_string() } else { format!("Show all models ({models})") };
        body = body.child(
            div().flex().flex_row().px(px(8.)).py(px(6.)).child(
                small(t, "voices-models".into(), words, false, "Models are listed only when they have a voice, until you show them all".into()).on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                    v.show_models = !v.show_models;
                    cx.notify();
                })),
            ),
        );
    }
    body.child(div().w_full().h(px(24.)).flex_none()).into_any_element()
}

impl Render for VoicesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let error = self.error.clone();
        div()
            .id("voices")
            .debug_selector(|| "voices".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .text_color(t.foreground)
            .bg(t.background)
            .child(header(self, &t, cx))
            .when_some(error, |el, e| el.child(div().px(px(12.)).py(px(4.)).text_color(t.red).child(e)))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .border_t_1()
                    .border_color(t.border)
                    .child(voice_list(self, &t, cx))
                    .child(rule_list(self, &t, cx)),
            )
    }
}
