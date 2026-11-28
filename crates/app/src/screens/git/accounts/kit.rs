use gpui::prelude::*;
use gpui::{Context, Div, ElementId, Entity, FontWeight, SharedString, Stateful, div, px};
use nixa_git::accounts::{Env, env_at};
use nixa_ui::Theme;
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::icons::spinner;
use nixa_ui::text_input::TextInput;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Ctx {
    pub env: Env,
    pub db: PathBuf,
}

pub fn real() -> Ctx {
    Ctx {
        env: env_at(&nixa_protocol::paths::data_dir()),
        db: nixa_store::db::db_path(),
    }
}

pub fn job<V: 'static, T: Send + 'static>(
    cx: &mut Context<V>,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&mut V, T, &mut Context<V>) + 'static,
) {
    let task = cx.background_executor().spawn(async move { work() });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            done(v, result, cx);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn get_setting(db: &std::path::Path, key: &str) -> Option<Value> {
    let conn = nixa_store::db::open(db).ok()?;
    nixa_store::settings::get_setting(&conn, key).ok().flatten()
}

pub fn set_setting(db: &std::path::Path, key: &str, value: &Value) -> Result<(), String> {
    let conn = nixa_store::db::open(db).map_err(|e| format!("{e:#}"))?;
    nixa_store::settings::set_setting(&conn, key, value).map_err(|e| format!("{e:#}"))
}

pub fn all_settings(db: &std::path::Path) -> std::collections::BTreeMap<String, Value> {
    nixa_store::db::open(db)
        .ok()
        .and_then(|conn| nixa_store::settings::settings_with_prefix(&conn, "git.").ok())
        .unwrap_or_default()
}

pub fn input<V: 'static>(placeholder: &str, value: &str, masked: bool, cx: &mut Context<V>) -> Entity<TextInput> {
    let placeholder = placeholder.to_string();
    let made = cx.new(|cx| {
        let field = TextInput::new(cx, placeholder);
        if masked { field.masked() } else { field }
    });
    let value = value.to_string();
    made.update(cx, |i, cx| i.set_value(value, cx));
    made
}

pub fn value(input: &Entity<TextInput>, cx: &gpui::App) -> String {
    input.read(cx).value().to_string()
}

pub fn field(input: &Entity<TextInput>, tag: &str) -> Div {
    let tag = tag.to_string();
    div()
        .w_full()
        .flex()
        .flex_col()
        .debug_selector(move || tag.clone())
        .child(input.clone())
}

fn words(color: gpui::Hsla, size: f32, text: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .text_size(px(size))
        .text_color(color)
        .whitespace_normal()
        .child(text.into())
}

pub fn hint(t: &Theme, text: impl Into<SharedString>) -> Div {
    words(t.dim, 11.5, text)
}

pub fn error_text(t: &Theme, text: impl Into<SharedString>) -> Div {
    words(t.red, 12., text).pt(px(4.))
}

pub fn ok_text(t: &Theme, text: impl Into<SharedString>) -> Div {
    words(t.green, 12., text).pt(px(4.))
}

pub fn warn_text(t: &Theme, text: impl Into<SharedString>) -> Div {
    words(t.yellow, 12., text).pt(px(4.))
}

pub fn label(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .pt(px(14.))
        .pb(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.blue)
        .child(text.into())
}

pub fn strong(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.bright)
        .child(text.into())
}

pub fn small(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().min_w_0().text_size(px(11.5)).text_color(t.muted).whitespace_normal().child(text.into())
}

pub fn card(t: &Theme, id: impl Into<ElementId>, accent: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(4.))
        .mb(px(8.))
        .p(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(if accent { t.accent } else { t.border })
        .bg(t.chrome)
}

pub fn row() -> Div {
    div().w_full().flex().flex_row().items_center().gap(px(8.))
}

pub fn wrap_row() -> Div {
    row().flex_wrap()
}

pub fn busy_line(t: &Theme, id: impl Into<ElementId>, text: impl Into<SharedString>) -> Div {
    row()
        .child(spinner(id, 13., t.muted))
        .child(div().text_size(px(12.)).text_color(t.muted).child(text.into()))
}

pub fn act(t: &Theme, id: impl Into<SharedString>, text: impl Into<SharedString>, kind: ButtonKind, off: bool) -> Stateful<Div> {
    let id: SharedString = id.into();
    let tag = id.to_string();
    disabled(button(t, ElementId::Name(id), text, kind), off)
        .h(px(24.))
        .px(px(9.))
        .debug_selector(move || tag.clone())
}

pub fn pill(t: &Theme, id: impl Into<SharedString>, text: impl Into<SharedString>, active: bool, off: bool) -> Stateful<Div> {
    let id: SharedString = id.into();
    let tag = id.to_string();
    let hover = t.hover;
    disabled(
        div()
            .id(ElementId::Name(id))
            .debug_selector(move || tag.clone())
            .flex_none()
            .h(px(24.))
            .px(px(9.))
            .flex()
            .items_center()
            .rounded(px(12.))
            .border_1()
            .border_color(if active { t.accent } else { t.border })
            .when(active, |el| el.bg(t.selected))
            .text_size(px(12.))
            .text_color(if active { t.bright } else { t.muted })
            .cursor_pointer()
            .hover(move |s| s.bg(hover)),
        off,
    )
    .child(text.into())
}

pub fn radio(t: &Theme, id: impl Into<SharedString>, on: bool, text: impl Into<SharedString>, detail: &str) -> Stateful<Div> {
    let id: SharedString = id.into();
    let tag = id.to_string();
    let hover = t.hover;
    div()
        .id(ElementId::Name(id))
        .debug_selector(move || tag.clone())
        .w_full()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(10.))
        .px(px(8.))
        .py(px(5.))
        .rounded(px(7.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(
            div()
                .mt(px(2.))
                .size(px(14.))
                .flex_none()
                .rounded_full()
                .border_1()
                .border_color(if on { t.accent } else { t.border_strong })
                .flex()
                .items_center()
                .justify_center()
                .when(on, |el| el.child(div().size(px(6.)).rounded_full().bg(t.accent))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(px(12.5)).text_color(t.foreground).child(text.into()))
                .when(!detail.is_empty(), |el| el.child(hint(t, detail.to_string()))),
        )
}

pub fn check(t: &Theme, id: impl Into<SharedString>, on: bool, text: impl Into<SharedString>) -> Stateful<Div> {
    let id: SharedString = id.into();
    let tag = id.to_string();
    let hover = t.hover;
    div()
        .id(ElementId::Name(id))
        .debug_selector(move || tag.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .px(px(6.))
        .py(px(4.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(
            div()
                .size(px(14.))
                .flex_none()
                .rounded(px(3.))
                .border_1()
                .border_color(if on { t.accent } else { t.border_strong })
                .when(on, |el| el.bg(t.accent))
                .flex()
                .items_center()
                .justify_center()
                .when(on, |el| {
                    el.child(nixa_ui::icon(nixa_ui::Icon::Check).size(px(10.)).text_color(t.accent_foreground))
                }),
        )
        .child(div().text_size(px(12.5)).text_color(t.foreground).child(text.into()))
}

pub fn dot(color: gpui::Hsla, size: f32) -> Div {
    div().size(px(size)).flex_none().rounded_full().bg(color)
}

pub fn profile_color(t: &Theme, name: &str) -> gpui::Hsla {
    match name {
        "ok" => t.green,
        "purple" => t.magenta,
        "cyan" => t.cyan,
        "warn" => t.yellow,
        _ => t.blue,
    }
}

pub fn color_name(name: &str) -> &'static str {
    match name {
        "ok" => "Green",
        "purple" => "Purple",
        "cyan" => "Cyan",
        "warn" => "Yellow",
        _ => "Blue",
    }
}

pub fn short_path(path: &str, home: &std::path::Path) -> String {
    let base = home.display().to_string();
    let base = base.trim_end_matches('/');
    match path.strip_prefix(base) {
        Some("") if !base.is_empty() => "~".to_string(),
        Some(rest) if !base.is_empty() && rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

pub fn repo_name(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path).to_string()
}

pub fn safe_id(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

pub fn scroll_body(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .size_full()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .pr(px(8.))
        .pb(px(16.))
        .text_size(px(12.8))
}

pub enum WindowKey {
    Close,
    Submit,
}

pub fn window_key(event: &gpui::KeyDownEvent) -> Option<WindowKey> {
    let key = &event.keystroke;
    let modifiers = key.modifiers;
    match key.key.as_str() {
        "escape" => Some(WindowKey::Close),
        "enter" if modifiers.platform && !modifiers.control && !modifiers.alt && !modifiers.shift => Some(WindowKey::Submit),
        _ => None,
    }
}
