use super::text_area::{AreaEvent, TextArea};
use crate::links::brain as link;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Global, SharedString, Subscription, Window, div,
    px,
};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

pub const WAIT: Duration = Duration::from_secs(15);
pub const EMBEDDED_HEIGHT: f32 = 280.;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Document {
    Memory,
    Rules,
}

impl Document {
    pub fn word(self) -> &'static str {
        match self {
            Document::Memory => "memory",
            Document::Rules => "rules",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Document::Memory => "Memory",
            Document::Rules => "Standing rules",
        }
    }

    pub fn intro(self) -> &'static str {
        match self {
            Document::Memory => {
                "Write and edit memory notes here. Use one '- ' line per note so memory search can find it. Notes do not grant permissions. They are kept in nixa.db."
            }
            Document::Rules => {
                "Rules that always apply, as plain JSON: a title, rules, and sections with their own rules. Saved to rules.json in the data folder."
            }
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Document::Memory => "- One note per line",
            Document::Rules => {
                "{\"version\": 1, \"title\": \"Nixa chat rules\", \"rules\": [\"Answer in short plain words.\"]}"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Reading,
    Saving,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    Reload,
    Clear,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    pub text: String,
    pub saved: String,
    pub hash: Option<String>,
}

#[derive(Default)]
struct Drafts(HashMap<Document, Draft>);

impl Global for Drafts {}

pub struct DocEditor {
    pub doc: Document,
    pub addr: String,
    pub area: Entity<TextArea>,
    pub saved: String,
    pub hash: Option<String>,
    pub notice: String,
    pub error: bool,
    pub busy: Option<Busy>,
    pub ask: Option<Ask>,
    pub held: Vec<String>,
    pub problem: Option<String>,
    pub file: Option<String>,
    pub embedded: bool,
    _watch: Subscription,
}

pub fn call<T: 'static>(
    addr: &str,
    method: &'static str,
    path: String,
    body: Option<Value>,
    cx: &mut Context<T>,
    done: impl FnOnce(&mut T, Result<Value, String>, &mut Context<T>) + 'static,
) {
    let addr = addr.to_string();
    let task = cx.background_executor().spawn(async move {
        link::http(&addr, method, &path, body.as_ref(), WAIT).map_err(|e| format!("{e:#}"))
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |view, cx| {
            done(view, result, cx);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn dirty(view: &DocEditor, cx: &App) -> bool {
    view.area.read(cx).value() != view.saved
}

fn keep_draft(view: &DocEditor, cx: &mut App) {
    let draft = Draft {
        text: view.area.read(cx).value().to_string(),
        saved: view.saved.clone(),
        hash: view.hash.clone(),
    };
    cx.default_global::<Drafts>().0.insert(view.doc, draft);
}

pub fn saved_draft(doc: Document, cx: &App) -> Option<Draft> {
    cx.try_global::<Drafts>()
        .and_then(|d| d.0.get(&doc).cloned())
}

pub fn build(
    doc: Document,
    addr: String,
    embedded: bool,
    cx: &mut Context<DocEditor>,
) -> DocEditor {
    let placeholder = doc.placeholder();
    let area = cx.new(|cx| {
        let area = TextArea::new(cx, placeholder);
        if doc == Document::Rules {
            area.mono()
        } else {
            area
        }
    });
    let watch = cx.subscribe(&area, |view, _, event: &AreaEvent, cx| {
        on_area(view, event, cx)
    });
    let mut view = DocEditor {
        doc,
        addr,
        area,
        saved: String::new(),
        hash: None,
        notice: "Connecting to the brain…".into(),
        error: false,
        busy: None,
        ask: None,
        held: Vec::new(),
        problem: None,
        file: None,
        embedded,
        _watch: watch,
    };
    if let Some(draft) = saved_draft(doc, cx) {
        view.saved = draft.saved;
        view.hash = draft.hash;
        view.area
            .update(cx, |area, cx| area.set_value(draft.text, cx));
    }
    view
}

pub fn new(doc: Document, addr: String, embedded: bool, cx: &mut App) -> Entity<DocEditor> {
    let view = cx.new(|cx| build(doc, addr, embedded, cx));
    view.update(cx, |view, cx| load(view, false, cx));
    view
}

fn on_area(view: &mut DocEditor, event: &AreaEvent, cx: &mut Context<DocEditor>) {
    match event {
        AreaEvent::Changed(_) => {
            view.notice = if dirty(view, cx) {
                "Unsaved changes. Your draft stays here if you switch settings pages.".into()
            } else {
                "No unsaved changes.".into()
            };
            view.error = false;
            keep_draft(view, cx);
            cx.notify();
        }
        AreaEvent::Save => press_save(view, cx),
        AreaEvent::Cancel => {}
    }
}

pub fn path(doc: Document) -> String {
    format!("/content/{}", doc.word())
}

pub fn load(view: &mut DocEditor, discard: bool, cx: &mut Context<DocEditor>) {
    view.busy = Some(Busy::Reading);
    view.ask = None;
    let at_read = view.area.read(cx).value().to_string();
    let addr = view.addr.clone();
    call(
        &addr,
        "GET",
        path(view.doc),
        None,
        cx,
        move |view, result, cx| on_read(view, result, discard, &at_read, cx),
    );
    cx.notify();
}

pub fn on_read(
    view: &mut DocEditor,
    result: Result<Value, String>,
    discard: bool,
    at_read: &str,
    cx: &mut Context<DocEditor>,
) {
    view.busy = None;
    let data = match result {
        Ok(data) => data,
        Err(error) => {
            view.error = true;
            view.notice = format!("Could not load: {error}. Your draft is kept.");
            return;
        }
    };
    let (Some(text), Some(hash)) = (
        data.get("text").and_then(Value::as_str),
        data.get("hash").and_then(Value::as_str),
    ) else {
        view.error = true;
        view.notice = "The brain did not send valid document text. Your draft is kept.".into();
        return;
    };
    take_extras(view, &data);
    let draft = view.area.read(cx).value().to_string();
    if dirty(view, cx) && (!discard || draft != at_read) {
        let moved = view.hash.as_deref() != Some(hash);
        view.error = moved;
        view.notice = if moved {
            "Changed elsewhere. Your unsaved draft is kept. Copy it before reloading the latest text.".into()
        } else {
            "Unsaved draft restored. Save when ready.".into()
        };
        if view.hash.is_none() {
            view.hash = Some(hash.to_string());
            view.saved = text.to_string();
        }
        return;
    }
    view.saved = text.to_string();
    view.hash = Some(hash.to_string());
    view.error = false;
    let text = text.to_string();
    view.area.update(cx, |area, cx| area.set_value(text, cx));
    view.notice = "Loaded. Edit the text, then Save.".into();
    keep_draft(view, cx);
}

fn take_extras(view: &mut DocEditor, data: &Value) {
    view.held = data
        .get("held")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    view.problem = data
        .get("problem")
        .and_then(Value::as_str)
        .map(str::to_string);
    view.file = data.get("file").and_then(Value::as_str).map(str::to_string);
}

pub fn can_save(view: &DocEditor, cx: &App) -> bool {
    view.hash.is_some() && dirty(view, cx) && view.busy.is_none()
}

pub fn press_save(view: &mut DocEditor, cx: &mut Context<DocEditor>) {
    if !can_save(view, cx) {
        return;
    }
    if view.doc == Document::Memory && view.area.read(cx).value().trim().is_empty() {
        view.ask = Some(Ask::Clear);
        cx.notify();
        return;
    }
    save(view, cx);
}

pub fn save(view: &mut DocEditor, cx: &mut Context<DocEditor>) {
    view.ask = None;
    view.busy = Some(Busy::Saving);
    view.notice = "Saving…".into();
    view.error = false;
    let sent = view.area.read(cx).value().to_string();
    let body = json!({"text": sent, "expectedHash": view.hash.clone().unwrap_or_default()});
    let addr = view.addr.clone();
    call(
        &addr,
        "POST",
        path(view.doc),
        Some(body),
        cx,
        move |view, result, cx| on_saved(view, result, &sent, cx),
    );
    cx.notify();
}

pub fn on_saved(
    view: &mut DocEditor,
    result: Result<Value, String>,
    sent: &str,
    cx: &mut Context<DocEditor>,
) {
    view.busy = None;
    let data = match result {
        Ok(data) => data,
        Err(error) => {
            view.error = true;
            view.notice = kept_line(&error);
            return;
        }
    };
    let (Some(text), Some(hash)) = (
        data.get("text").and_then(Value::as_str),
        data.get("hash").and_then(Value::as_str),
    ) else {
        view.error = true;
        view.notice = "The brain did not send the saved text back. Reload to check whether the save finished.".into();
        return;
    };
    take_extras(view, &data);
    let changed = view.area.read(cx).value() != sent;
    view.saved = text.to_string();
    view.hash = Some(hash.to_string());
    view.error = false;
    if !changed {
        let text = text.to_string();
        view.area.update(cx, |area, cx| area.set_value(text, cx));
    }
    let notes: Vec<String> = data
        .get("notesSaved")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let head = if changed {
        "Saved the submitted text. Your newer edits are still unsaved."
    } else {
        "Saved. Nixa reads the changes on its next brain turn."
    };
    view.notice = if notes.is_empty() {
        head.to_string()
    } else {
        format!("{head} {}", notes.join(" "))
    };
    keep_draft(view, cx);
}

pub fn kept_line(error: &str) -> String {
    let error = error.trim();
    if error.contains("draft is kept") {
        return error.to_string();
    }
    let end = if error.ends_with('.') { "" } else { "." };
    format!("{error}{end} Your draft is kept.")
}

pub fn press_reload(view: &mut DocEditor, cx: &mut Context<DocEditor>) {
    if view.busy.is_some() {
        return;
    }
    if dirty(view, cx) {
        view.ask = Some(Ask::Reload);
        cx.notify();
    } else {
        load(view, false, cx);
    }
}

pub fn on_event(view: &mut DocEditor, msg: &Value, cx: &mut Context<DocEditor>) -> bool {
    let kind = msg.get("type").and_then(Value::as_str);
    if kind == Some("hello") {
        if view.hash.is_none() && view.busy.is_none() {
            load(view, false, cx);
        }
        return true;
    }
    if kind != Some("content_changed")
        || msg.get("document").and_then(Value::as_str) != Some(view.doc.word())
    {
        return false;
    }
    let hash = msg.get("hash").and_then(Value::as_str);
    if hash.is_some() && hash == view.hash.as_deref() || view.busy.is_some() {
        return true;
    }
    if dirty(view, cx) {
        view.error = true;
        view.notice = "Changed elsewhere. Your unsaved draft is kept. Copy it before reloading the latest text.".into();
        cx.notify();
    } else {
        load(view, false, cx);
    }
    true
}

pub fn feed(view: &Entity<DocEditor>, msg: &Value, cx: &mut App) -> bool {
    view.update(cx, |view, cx| on_event(view, msg, cx))
}

fn note(t: &Theme, text: impl Into<SharedString>, error: bool) -> gpui::Div {
    div()
        .w_full()
        .flex_none()
        .text_size(px(11.5))
        .text_color(if error { t.red } else { t.dim })
        .child(text.into())
}

fn confirm(t: &Theme, ask: Ask, cx: &mut Context<DocEditor>) -> AnyElement {
    let (line, yes) = match ask {
        Ask::Reload => (
            "Discard this unsaved draft and reload the latest saved text?",
            "Discard draft and reload",
        ),
        Ask::Clear => (
            "Save empty memory and remove all its notes from nixa.db?",
            "Save empty memory",
        ),
    };
    div()
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.red.opacity(0.5))
        .child(note(t, line, true))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(
                    button(t, "doc-keep", "Keep editing", ButtonKind::Secondary).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.ask = None;
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button(t, "doc-yes", yes, ButtonKind::Danger).on_click(cx.listener(
                        move |view, _, _, cx| match ask {
                            Ask::Reload => load(view, true, cx),
                            Ask::Clear => save(view, cx),
                        },
                    )),
                ),
        )
        .into_any_element()
}

pub fn render_editor(view: &mut DocEditor, cx: &mut Context<DocEditor>) -> AnyElement {
    let t = theme(cx).clone();
    let busy = view.busy.is_some();
    let save_on = can_save(view, cx);
    let save_label = if busy { "Working…" } else { "Save" };
    let held = view.held.len();
    div()
        .id(SharedString::from(format!("doc-{}", view.doc.word())))
        .w_full()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .when(view.embedded, |el| el.flex_none())
        .when(!view.embedded, |el| el.flex_1())
        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.bright).child(view.doc.title()))
        .child(note(&t, view.doc.intro(), false))
        .when_some(view.file.clone(), |el, file| el.child(note(&t, format!("File: {file}"), false)))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .items_center()
                .child(disabled(button(&t, "doc-save", save_label, ButtonKind::Primary), !save_on).when(save_on, |el| el.on_click(cx.listener(|view, _, _, cx| press_save(view, cx)))))
                .child(disabled(button(&t, "doc-reload", "Reload", ButtonKind::Secondary), busy).when(!busy, |el| el.on_click(cx.listener(|view, _, _, cx| press_reload(view, cx)))))
                .child(div().text_size(px(10.5)).text_color(t.dim).child("⌘S saves")),
        )
        .when_some(view.ask, |el, ask| el.child(confirm(&t, ask, cx)))
        .child(note(&t, view.notice.clone(), view.error))
        .when_some(view.problem.clone(), |el, problem| el.child(note(&t, problem, true)))
        .when(held > 0, |el| el.child(note(&t, format!("{held} {} held back from the brain: {} like an order to the brain. Edit or remove {} here.", if held == 1 { "note is" } else { "notes are" }, if held == 1 { "it reads" } else { "they read" }, if held == 1 { "it" } else { "them" }), true)))
        .when(view.hash.is_some(), |el| {
            el.child(
                div()
                    .w_full()
                    .min_h(px(120.))
                    .when(view.embedded, |b| b.h(px(EMBEDDED_HEIGHT)).flex_none())
                    .when(!view.embedded, |b| b.flex_1())
                    .child(view.area.clone()),
            )
        })
        .when(view.hash.is_none(), |el| el.child(note(&t, "The editor loads after the brain answers. Saved text is never replaced with an empty loading state.", false)))
        .into_any_element()
}

impl Render for DocEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_editor(self, cx)
    }
}
