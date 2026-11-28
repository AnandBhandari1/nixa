use crate::links::brain as link;
use crate::screens::menu::switches::OFF_FILE;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, Focusable, FontWeight, Hsla, SharedString,
    Stateful, Subscription, Window, div, px,
};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::popover::tooltip;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Icon, Theme, icon, theme};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

pub const PART: &str = "Chat brain";
pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const SAVE_WAIT: Duration = Duration::from_secs(45);
pub const MODELS_WAIT: Duration = Duration::from_secs(60);
pub const API_KINDS: [&str; 3] = ["openai", "anthropic", "ollama"];
pub const OLLAMA: &str = "ollama";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cli {
    pub choice: String,
    pub name: String,
    pub installed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Own {
    pub id: String,
    pub choice: String,
    pub name: String,
    pub kind: String,
    pub kind_title: String,
    pub base_url: String,
    pub model: String,
    pub key_source: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Current {
    pub name: String,
    pub kind: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub cli: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub description: String,
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    Loading,
    Up,
    Down(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Saving {
    Idle,
    Checking,
    Failed(String),
}

pub struct Form {
    pub id: Option<String>,
    pub kind: String,
    pub name: Entity<TextInput>,
    pub url: Entity<TextInput>,
    pub key: Entity<TextInput>,
    pub saving: Saving,
    _subs: Vec<Subscription>,
}

pub struct ChatBrainView {
    pub addr: String,
    pub data: PathBuf,
    pub name: String,
    pub reach: Reach,
    pub cli: Vec<Cli>,
    pub own: Vec<Own>,
    pub choice: Option<String>,
    pub current: Option<Current>,
    pub models: Vec<Model>,
    pub models_for: Option<String>,
    pub models_loading: bool,
    pub models_error: Option<String>,
    pub form: Option<Form>,
    pub remove_ask: Option<String>,
    pub busy: Option<String>,
    pub pending: Option<String>,
    pub error: Option<String>,
    pub off: bool,
    loads: u64,
}

pub fn kind_title(kind: &str) -> &'static str {
    match kind {
        "anthropic" => "Anthropic",
        "ollama" => "Ollama",
        _ => "OpenAI",
    }
}

pub fn default_url(kind: &str) -> &'static str {
    match kind {
        "anthropic" => "https://api.anthropic.com",
        "ollama" => "http://127.0.0.1:11434/v1",
        _ => "https://api.openai.com/v1",
    }
}

pub fn kind_tip(kind: &str) -> &'static str {
    match kind {
        "anthropic" => {
            "For any server with the Anthropic Messages API, for example Anthropic (Claude) or Ollama."
        }
        "ollama" => "For Ollama on this Mac. It needs no key.",
        _ => {
            "For any server with the OpenAI chat API, for example OpenAI, OpenRouter, LM Studio or Ollama."
        }
    }
}

pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or("").to_string()
}

pub fn key_words(source: &str) -> &'static str {
    match source {
        "keychain" => "key in the keychain",
        "env" => "key from an env var",
        _ => "no key",
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn some_text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn parse_cli(value: &Value) -> Vec<Cli> {
    value
        .get("cli")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    let choice = some_text(item, "choice")?;
                    Some(Cli {
                        name: some_text(item, "name").unwrap_or_else(|| choice.clone()),
                        installed: item
                            .get("installed")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        choice,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_own(value: &Value) -> Vec<Own> {
    value
        .get("brains")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    let id = some_text(item, "id")?;
                    let kind = text(item, "kind");
                    Some(Own {
                        choice: some_text(item, "choice").unwrap_or_else(|| format!("api:{id}")),
                        name: some_text(item, "name").unwrap_or_else(|| id.clone()),
                        kind_title: some_text(item, "kindTitle")
                            .unwrap_or_else(|| kind_title(&kind).to_string()),
                        kind,
                        base_url: text(item, "baseUrl"),
                        model: text(item, "model"),
                        key_source: text(item, "keySource"),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_current(value: Option<&Value>) -> Option<Current> {
    let info = value.filter(|v| v.is_object())?;
    Some(Current {
        name: text(info, "name"),
        kind: text(info, "kind"),
        model: some_text(info, "model"),
        effort: some_text(info, "effort"),
        cli: info.get("cli").and_then(Value::as_bool).unwrap_or(false),
    })
}

pub fn parse_models(value: &Value) -> Vec<Model> {
    value
        .get("models")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    let id = some_text(item, "id")?;
                    let mut efforts: Vec<String> = Vec::new();
                    for effort in item
                        .get("efforts")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        if !effort.is_empty() && !efforts.iter().any(|e| e == effort) {
                            efforts.push(effort.to_string());
                        }
                    }
                    Some(Model {
                        name: some_text(item, "name").unwrap_or_else(|| id.clone()),
                        description: text(item, "description"),
                        efforts,
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn current_model(v: &ChatBrainView) -> Option<&Model> {
    let id = v.current.as_ref()?.model.as_deref()?;
    v.models.iter().find(|m| m.id == id)
}

pub fn effort_choices(v: &ChatBrainView) -> Vec<String> {
    let Some(model) = current_model(v) else {
        return Vec::new();
    };
    let mut out = vec![String::new()];
    out.extend(model.efforts.iter().cloned());
    out
}

pub fn picked_effort(v: &ChatBrainView) -> String {
    let efforts = current_model(v)
        .map(|m| m.efforts.clone())
        .unwrap_or_default();
    v.current
        .as_ref()
        .and_then(|c| c.effort.clone())
        .filter(|e| efforts.contains(e))
        .unwrap_or_default()
}

pub fn brain_label(v: &ChatBrainView) -> String {
    match &v.current {
        Some(c) if !c.name.is_empty() => c.name.clone(),
        _ => "no brain yet".into(),
    }
}

pub fn footer_text(v: &ChatBrainView) -> String {
    if v.off {
        format!(
            "{} stays off. Your choices are saved for its next start.",
            v.name
        )
    } else {
        "Your choices are saved.".into()
    }
}

pub fn build(addr: String, data: PathBuf) -> ChatBrainView {
    ChatBrainView {
        off: data.join(OFF_FILE).exists(),
        addr,
        data,
        name: "Nixa".into(),
        reach: Reach::Loading,
        cli: Vec::new(),
        own: Vec::new(),
        choice: None,
        current: None,
        models: Vec::new(),
        models_for: None,
        models_loading: false,
        models_error: None,
        form: None,
        remove_ask: None,
        busy: None,
        pending: None,
        error: None,
        loads: 0,
    }
}

pub fn new(addr: String, data: PathBuf, cx: &mut App) -> Entity<ChatBrainView> {
    cx.new(|cx| {
        let mut view = build(addr, data);
        reload(&mut view, true, cx);
        view
    })
}

pub struct Shared(pub Entity<ChatBrainView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<ChatBrainView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<ChatBrainView> {
    if let Some(view) = shared(cx) {
        view.update(cx, |v, cx| {
            reload(v, false, cx);
            cx.notify();
        });
        return view;
    }
    let view = new(
        nixa_protocol::paths::brain_addr(),
        nixa_protocol::paths::data_dir(),
        cx,
    );
    cx.set_global(Shared(view.clone()));
    view
}

pub fn apply(view: &Entity<ChatBrainView>, msg: &Value, cx: &mut App) -> bool {
    let kind = msg.get("type").and_then(Value::as_str).unwrap_or_default();
    if kind != "brain" && kind != "hello" {
        return false;
    }
    view.update(cx, |v, cx| {
        if kind == "brain" {
            v.current = parse_current(msg.get("brain"));
        }
        if kind == "hello"
            && let Some(name) = some_text(msg, "name")
        {
            v.name = name;
        }
        reload(v, false, cx);
        cx.notify();
    });
    true
}

fn call(
    addr: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    wait: Duration,
) -> Result<Value, String> {
    link::http(addr, method, path, body.as_ref(), wait).map_err(|e| format!("{e:#}"))
}

fn job(
    cx: &mut Context<ChatBrainView>,
    work: impl FnOnce() -> Result<Value, String> + Send + 'static,
    done: impl FnOnce(&mut ChatBrainView, Result<Value, String>, &mut Context<ChatBrainView>) + 'static,
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

pub fn reload(v: &mut ChatBrainView, models_too: bool, cx: &mut Context<ChatBrainView>) {
    v.off = v.data.join(OFF_FILE).exists();
    v.loads += 1;
    let load = v.loads;
    let addr = v.addr.clone();
    job(
        cx,
        move || {
            let brains = call(&addr, "GET", "/brains", None, CALL_WAIT)?;
            let health = call(&addr, "GET", "/health", None, CALL_WAIT)?;
            Ok(json!({"brains": brains, "health": health}))
        },
        move |v, result, cx| {
            if v.loads != load {
                return;
            }
            match result {
                Ok(both) => {
                    v.reach = Reach::Up;
                    let brains = &both["brains"];
                    v.cli = parse_cli(brains);
                    v.own = parse_own(brains);
                    v.choice = some_text(brains, "choice");
                    v.current = parse_current(both["health"].get("brain"));
                    if let Some(name) = some_text(&both["health"], "name") {
                        v.name = name;
                    }
                    if v.models_for != v.choice {
                        v.models.clear();
                        v.models_error = None;
                        v.models_for = v.choice.clone();
                    }
                    if models_too && !v.off && v.choice.is_some() {
                        refresh_models(v, cx);
                    }
                }
                Err(why) => v.reach = Reach::Down(why),
            }
        },
    );
}

pub fn refresh_models(v: &mut ChatBrainView, cx: &mut Context<ChatBrainView>) {
    if v.models_loading || v.choice.is_none() {
        return;
    }
    v.models_loading = true;
    v.models_error = None;
    let addr = v.addr.clone();
    let wanted = v.choice.clone();
    job(
        cx,
        move || call(&addr, "GET", "/brain/models", None, MODELS_WAIT),
        move |v, result, _| {
            v.models_loading = false;
            if v.choice != wanted {
                return;
            }
            v.models_for = wanted;
            match result {
                Ok(value) => v.models = parse_models(&value),
                Err(why) => v.models_error = Some(why),
            }
        },
    );
}

pub fn pick(v: &mut ChatBrainView, choice: &str, cx: &mut Context<ChatBrainView>) {
    if v.busy.is_some() || v.choice.as_deref() == Some(choice) {
        return;
    }
    if v.cli.iter().any(|c| c.choice == choice && !c.installed) {
        return;
    }
    v.busy = Some(format!("pick:{choice}"));
    v.error = None;
    let addr = v.addr.clone();
    let body = json!({"choice": choice});
    let picked = choice.to_string();
    job(
        cx,
        move || call(&addr, "POST", "/brain/choice", Some(body), CALL_WAIT),
        move |v, result, cx| {
            v.busy = None;
            match result {
                Ok(_) => {
                    v.choice = Some(picked.clone());
                    v.models.clear();
                    v.models_error = None;
                    v.models_for = Some(picked);
                    reload(v, true, cx);
                }
                Err(why) => v.error = Some(why),
            }
            run_pending(v, cx);
        },
    );
}

pub fn ollama_brain(v: &ChatBrainView) -> Option<&Own> {
    v.own.iter().find(|b| b.kind == OLLAMA)
}

pub fn use_ollama(v: &mut ChatBrainView, cx: &mut Context<ChatBrainView>) {
    let chosen = ollama_brain(v).map(|b| b.choice.clone());
    if v.busy.is_some() || (chosen.is_some() && v.choice == chosen) {
        return;
    }
    v.busy = Some(format!("pick:{OLLAMA}"));
    v.error = None;
    let addr = v.addr.clone();
    job(
        cx,
        move || call(&addr, "POST", "/brain/ollama", Some(json!({})), SAVE_WAIT),
        move |v, result, cx| {
            v.busy = None;
            match result {
                Ok(reply) => {
                    let picked = some_text(&reply, "choice");
                    v.choice = picked.clone();
                    v.models.clear();
                    v.models_error = None;
                    v.models_for = picked;
                    reload(v, true, cx);
                }
                Err(why) => v.error = Some(why),
            }
            run_pending(v, cx);
        },
    );
}

pub fn run_pending(v: &mut ChatBrainView, cx: &mut Context<ChatBrainView>) {
    if v.busy.is_none()
        && let Some(choice) = v.pending.take()
    {
        pick(v, &choice, cx);
    }
}

fn send_model(v: &mut ChatBrainView, body: Value, cx: &mut Context<ChatBrainView>) {
    if v.busy.is_some() {
        return;
    }
    v.busy = Some("model".into());
    v.models_error = None;
    let addr = v.addr.clone();
    job(
        cx,
        move || call(&addr, "POST", "/brain/model", Some(body), CALL_WAIT),
        move |v, result, cx| {
            v.busy = None;
            match result {
                Ok(reply) => {
                    if let Some(current) = v.current.as_mut() {
                        current.model = some_text(&reply, "model");
                        current.effort = some_text(&reply, "effort");
                    }
                }
                Err(why) => v.models_error = Some(why),
            }
            run_pending(v, cx);
        },
    );
}

pub fn set_model(v: &mut ChatBrainView, model: &str, cx: &mut Context<ChatBrainView>) {
    if v.current.as_ref().and_then(|c| c.model.as_deref()) == Some(model) {
        return;
    }
    send_model(v, json!({"model": model}), cx);
}

pub fn set_effort(v: &mut ChatBrainView, effort: &str, cx: &mut Context<ChatBrainView>) {
    if picked_effort(v) == effort || v.models_loading {
        return;
    }
    send_model(v, json!({"effort": effort}), cx);
}

fn input(
    placeholder: &'static str,
    value: &str,
    masked: bool,
    cx: &mut Context<ChatBrainView>,
) -> Entity<TextInput> {
    let made = cx.new(|cx| {
        let field = TextInput::new(cx, placeholder);
        if masked { field.masked() } else { field }
    });
    made.update(cx, |i, cx| i.set_value(value.to_string(), cx));
    made
}

pub fn open_form(
    v: &mut ChatBrainView,
    id: Option<&str>,
    window: &mut Window,
    cx: &mut Context<ChatBrainView>,
) {
    let editing = id.and_then(|id| v.own.iter().find(|b| b.id == id)).cloned();
    v.remove_ask = None;
    let name = input(
        "Name, for example My Router",
        editing.as_ref().map(|b| b.name.as_str()).unwrap_or(""),
        false,
        cx,
    );
    let url = input(
        "Base URL, empty uses the default",
        editing.as_ref().map(|b| b.base_url.as_str()).unwrap_or(""),
        false,
        cx,
    );
    let key = input("API key", "", true, cx);
    let subs = [&name, &url, &key]
        .into_iter()
        .map(|field| {
            cx.subscribe(
                field,
                |v: &mut ChatBrainView, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit(_) => save_form(v, cx),
                    InputEvent::Cancel => {
                        v.form = None;
                        cx.notify();
                    }
                    InputEvent::Changed(_) => {}
                },
            )
        })
        .collect();
    window.focus(&name.focus_handle(cx));
    v.form = Some(Form {
        id: editing.as_ref().map(|b| b.id.clone()),
        kind: editing
            .as_ref()
            .map(|b| b.kind.clone())
            .filter(|k| API_KINDS.contains(&k.as_str()))
            .unwrap_or_else(|| "openai".into()),
        name,
        url,
        key,
        saving: Saving::Idle,
        _subs: subs,
    });
}

pub fn form_body(form: &Form, cx: &App) -> Value {
    let mut body = json!({
        "kind": form.kind,
        "name": form.name.read(cx).value().trim(),
        "baseUrl": form.url.read(cx).value().trim(),
        "apiKey": form.key.read(cx).value().trim(),
    });
    if let Some(id) = &form.id {
        body["id"] = json!(id);
    }
    body
}

pub fn save_form(v: &mut ChatBrainView, cx: &mut Context<ChatBrainView>) {
    let Some(form) = v.form.as_mut() else {
        return;
    };
    if form.saving == Saving::Checking {
        return;
    }
    form.saving = Saving::Checking;
    let body = form_body(form, cx);
    let editing = form.id.clone();
    let addr = v.addr.clone();
    job(
        cx,
        move || call(&addr, "POST", "/brains", Some(body), SAVE_WAIT),
        move |v, result, cx| match result {
            Ok(reply) => {
                v.form = None;
                let was_current = editing
                    .as_ref()
                    .is_some_and(|id| v.choice.as_deref() == Some(format!("api:{id}").as_str()));
                match (editing, some_text(&reply, "choice")) {
                    (None, Some(choice)) => {
                        reload(v, false, cx);
                        v.pending = Some(choice);
                        run_pending(v, cx);
                    }
                    _ if was_current => {
                        v.models.clear();
                        v.models_for = None;
                        reload(v, true, cx);
                    }
                    _ => reload(v, false, cx),
                }
            }
            Err(why) => {
                if let Some(form) = v.form.as_mut() {
                    form.saving = Saving::Failed(why);
                }
            }
        },
    );
}

pub fn remove(v: &mut ChatBrainView, id: &str, cx: &mut Context<ChatBrainView>) {
    if v.busy.is_some() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return;
    }
    v.busy = Some(format!("remove:{id}"));
    v.remove_ask = None;
    v.error = None;
    let addr = v.addr.clone();
    let path = format!("/brains/{id}");
    job(
        cx,
        move || call(&addr, "DELETE", &path, None, CALL_WAIT),
        move |v, result, cx| {
            v.busy = None;
            match result {
                Ok(_) => reload(v, true, cx),
                Err(why) => v.error = Some(why),
            }
            run_pending(v, cx);
        },
    );
}

fn listen(
    cx: &mut Context<ChatBrainView>,
    act: impl Fn(&mut ChatBrainView, &mut Window, &mut Context<ChatBrainView>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |v, _: &ClickEvent, window, cx| {
        act(v, window, cx);
        cx.notify();
    })
}

fn clickable(id: String) -> Stateful<Div> {
    let tag = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || tag.clone())
}

fn chip(t: &Theme, id: String, which: Icon, label: &str, color: Hsla, off: bool) -> Stateful<Div> {
    let hover = t.hover;
    let el = clickable(id)
        .flex_none()
        .h(px(24.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(5.))
        .rounded(px(6.))
        .border_1()
        .border_color(color.opacity(0.45))
        .text_size(px(12.))
        .text_color(color)
        .child(icon(which).size(px(13.)).text_color(color))
        .when(!label.is_empty(), |el| {
            el.child(SharedString::from(label.to_string()))
        });
    if off {
        el.opacity(0.45).cursor_not_allowed()
    } else {
        el.cursor_pointer().hover(move |s| s.bg(hover))
    }
}

fn pick_chip(t: &Theme, id: String, label: &str, on: bool, off: bool) -> Stateful<Div> {
    let (which, color) = if on {
        (Icon::Check, t.green)
    } else {
        (Icon::Square, t.cyan)
    };
    chip(t, id, which, label, color, off).when(on, |el| el.bg(t.green.opacity(0.10)))
}

fn sub_label(t: &Theme, words: &str) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.blue)
        .child(SharedString::from(words.to_string()))
}

fn note(color: Hsla, size: f32, words: String) -> Div {
    div()
        .text_size(px(size))
        .text_color(color)
        .child(SharedString::from(words))
}

fn section(
    t: &Theme,
    id: &str,
    title: String,
    about: String,
    action: Option<AnyElement>,
    body: AnyElement,
) -> AnyElement {
    div()
        .id(SharedString::from(id.to_string()))
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.border)
        .child(
            div()
                .w_full()
                .flex()
                .items_start()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.foreground)
                                .child(SharedString::from(title)),
                        )
                        .child(note(t.dim, 11., about)),
                )
                .when_some(action, |el, action| el.child(action)),
        )
        .child(body)
        .into_any_element()
}

fn brain_chips(v: &ChatBrainView, t: &Theme, cx: &mut Context<ChatBrainView>) -> AnyElement {
    let name = v.name.clone();
    let mut row = div().w_full().flex().flex_wrap().items_center().gap(px(6.));
    let mut all: Vec<(String, String, bool)> = v
        .cli
        .iter()
        .map(|c| (c.choice.clone(), c.name.clone(), c.installed))
        .collect();
    let ollama = ollama_brain(v).map(|b| b.id.clone());
    all.extend(
        v.own
            .iter()
            .filter(|b| Some(&b.id) != ollama.as_ref())
            .map(|b| (b.choice.clone(), b.name.clone(), true)),
    );
    for (choice, label, installed) in all {
        let on = v.choice.as_deref() == Some(choice.as_str());
        let waiting = v.busy.as_deref() == Some(format!("pick:{choice}").as_str());
        let tip = if on {
            format!("{label} is {name}'s brain now")
        } else if !installed {
            format!("{label} is not installed on this Mac")
        } else {
            format!("Use {label} as {name}'s brain")
        };
        let shown = if waiting {
            format!("{label}…")
        } else {
            label.clone()
        };
        let picked = choice.clone();
        row = row.child(
            pick_chip(t, format!("cb-brain-{choice}"), &shown, on, !installed)
                .tooltip(tooltip(tip))
                .on_click(listen(cx, move |v, _, cx| pick(v, &picked, cx))),
        );
    }
    let ollama_on = ollama_brain(v).is_some_and(|b| v.choice.as_deref() == Some(b.choice.as_str()));
    let ollama_waiting = v.busy.as_deref() == Some(format!("pick:{OLLAMA}").as_str());
    let ollama_tip = if ollama_on {
        format!("Ollama is {name}'s brain now")
    } else {
        format!("Use Ollama on this Mac as {name}'s brain. No key is needed.")
    };
    row.child(
        pick_chip(t, "cb-brain-ollama".into(), if ollama_waiting { "Ollama…" } else { "Ollama" }, ollama_on, false)
            .tooltip(tooltip(ollama_tip))
            .on_click(listen(cx, |v, _, cx| use_ollama(v, cx))),
    )
    .child(
        chip(t, "cb-add".into(), Icon::Plus, "Add", t.blue, false)
            .tooltip(tooltip(
                "Add your own brain: a server with the OpenAI, Anthropic or Ollama API, with its URL, key and model",
            ))
            .on_click(listen(cx, |v, window, cx| open_form(v, None, window, cx))),
    )
    .into_any_element()
}

fn own_row(v: &ChatBrainView, t: &Theme, b: &Own, cx: &mut Context<ChatBrainView>) -> AnyElement {
    if v.remove_ask.as_deref() == Some(b.id.as_str()) {
        let keep = b.id.clone();
        let gone = b.id.clone();
        return div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(t.red.opacity(0.45))
            .bg(t.red.opacity(0.06))
            .child(note(
                t.foreground,
                12.,
                format!("Remove {} and its saved key?", b.name),
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.))
                    .child(
                        chip(
                            t,
                            format!("cb-keep-{keep}"),
                            Icon::X,
                            "Keep",
                            t.muted,
                            false,
                        )
                        .tooltip(tooltip(format!("Keep {}", b.name)))
                        .on_click(listen(cx, |v, _, _| v.remove_ask = None)),
                    )
                    .child(
                        chip(
                            t,
                            format!("cb-delete-{gone}"),
                            Icon::Trash,
                            "Remove",
                            t.red,
                            v.busy.is_some(),
                        )
                        .tooltip(tooltip(format!("Remove {} now", b.name)))
                        .on_click(listen(cx, move |v, _, cx| remove(v, &gone, cx))),
                    ),
            )
            .into_any_element();
    }
    let on = v.choice.as_deref() == Some(b.choice.as_str());
    let edit_id = b.id.clone();
    let ask_id = b.id.clone();
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(5.))
        .rounded(px(6.))
        .bg(t.surface)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(if on { t.green } else { t.foreground })
                        .child(SharedString::from(b.name.clone())),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(10.5))
                        .text_color(t.muted)
                        .child(SharedString::from(format!(
                            "{} · {} · {}",
                            b.kind_title,
                            host_of(&b.base_url),
                            key_words(&b.key_source)
                        ))),
                ),
        )
        .child(
            chip(
                t,
                format!("cb-edit-{edit_id}"),
                Icon::SquarePen,
                "",
                t.blue,
                false,
            )
            .tooltip(tooltip(format!(
                "Change the name, type, URL or key of {}",
                b.name
            )))
            .on_click(listen(cx, move |v, window, cx| {
                open_form(v, Some(&edit_id), window, cx)
            })),
        )
        .child(
            chip(
                t,
                format!("cb-remove-{ask_id}"),
                Icon::Trash,
                "",
                t.red,
                false,
            )
            .tooltip(tooltip(format!("Remove {} and its saved key", b.name)))
            .on_click(listen(cx, move |v, _, _| {
                v.form = None;
                v.remove_ask = Some(ask_id.clone());
            })),
        )
        .into_any_element()
}

fn field(t: &Theme, label: &str, input: &Entity<TextInput>, tag: &str, about: String) -> Div {
    let tag = tag.to_string();
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(note(t.muted, 11., label.to_string()))
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .debug_selector(move || tag.clone())
                .child(input.clone()),
        )
        .child(note(t.dim, 10.5, about))
}

fn form_box(
    v: &ChatBrainView,
    t: &Theme,
    form: &Form,
    cx: &mut Context<ChatBrainView>,
) -> AnyElement {
    let editing = form
        .id
        .as_ref()
        .and_then(|id| v.own.iter().find(|b| &b.id == id));
    let head = match editing {
        Some(b) => format!("CHANGE {}", b.name.to_uppercase()),
        None => "ADD A BRAIN".into(),
    };
    let mut kinds = div().flex().flex_wrap().gap(px(6.));
    for kind in API_KINDS {
        let on = form.kind == kind;
        kinds = kinds.child(
            pick_chip(
                t,
                format!("cb-type-{kind}"),
                &format!("{} API", kind_title(kind)),
                on,
                false,
            )
            .tooltip(tooltip(kind_tip(kind)))
            .on_click(listen(cx, move |v, _, _| {
                if let Some(form) = v.form.as_mut() {
                    form.kind = kind.to_string();
                }
            })),
        );
    }
    let key_about = match editing {
        Some(b) => format!("Empty keeps the saved key ({}).", key_words(&b.key_source)),
        None => {
            "Saved only on this Mac, in the macOS keychain. A server on this Mac may need no key."
                .into()
        }
    };
    let checking = form.saving == Saving::Checking;
    let status = match &form.saving {
        Saving::Idle => None,
        Saving::Checking => Some((t.yellow, format!("{} checks the URL and key…", v.name))),
        Saving::Failed(why) => Some((t.red, why.clone())),
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.blue.opacity(0.45))
        .bg(t.blue.opacity(0.06))
        .child(note(t.blue, 11., head))
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(note(t.muted, 11., "Type".into()))
                .child(kinds),
        )
        .child(field(
            t,
            "Name",
            &form.name,
            "cb-name",
            "Shown on the brain chip.".into(),
        ))
        .child(field(
            t,
            "URL",
            &form.url,
            "cb-url",
            format!(
                "Empty uses {}. A server on this Mac can use http://127.0.0.1.",
                default_url(&form.kind)
            ),
        ))
        .child(field(t, "API key", &form.key, "cb-key", key_about))
        .when_some(status, |el, (color, words)| {
            el.child(note(color, 11.5, words))
        })
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(6.))
                .child(
                    button(t, "cb-cancel", "Cancel", ButtonKind::Ghost)
                        .debug_selector(|| "cb-cancel".into())
                        .tooltip(tooltip("Close without saving"))
                        .on_click(listen(cx, |v, _, _| v.form = None)),
                )
                .child(
                    disabled(
                        button(
                            t,
                            "cb-save",
                            if checking { "Checking…" } else { "Save" },
                            ButtonKind::Primary,
                        ),
                        checking,
                    )
                    .debug_selector(|| "cb-save".into())
                    .tooltip(tooltip(format!(
                        "{} checks the URL and key, then saves. Then pick the model in the list.",
                        v.name
                    )))
                    .on_click(listen(cx, |v, _, cx| save_form(v, cx))),
                ),
        )
        .into_any_element()
}

fn engine_body(v: &ChatBrainView, t: &Theme, cx: &mut Context<ChatBrainView>) -> AnyElement {
    let mut body = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(10.))
        .when_some(v.error.clone(), |el, why| el.child(note(t.red, 12., why)))
        .child(brain_chips(v, t, cx));
    if !v.own.is_empty() {
        let mut list = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(sub_label(t, "Your own brains"));
        for b in &v.own {
            list = list.child(own_row(v, t, b, cx));
        }
        body = body.child(list);
    }
    if let Some(form) = &v.form {
        body = body.child(form_box(v, t, form, cx));
    }
    body.into_any_element()
}

fn model_row(
    v: &ChatBrainView,
    t: &Theme,
    m: &Model,
    cx: &mut Context<ChatBrainView>,
) -> AnyElement {
    let on = v.current.as_ref().and_then(|c| c.model.as_deref()) == Some(m.id.as_str());
    let hover = t.hover;
    let id = m.id.clone();
    let tip = if on {
        "This model is in use".to_string()
    } else {
        format!("Use {} for {}'s brain", m.name, v.name)
    };
    clickable(format!("cb-model-{}", m.id))
        .w_full()
        .flex_none()
        .flex()
        .items_start()
        .gap(px(8.))
        .px(px(8.))
        .py(px(5.))
        .rounded(px(6.))
        .border_1()
        .border_color(if on {
            t.green
        } else {
            gpui::transparent_black()
        })
        .when(on, |el| el.bg(t.green.opacity(0.08)))
        .when(!on, |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        .tooltip(tooltip(tip))
        .on_click(listen(cx, move |v, _, cx| set_model(v, &id, cx)))
        .child(
            icon(if on { Icon::Check } else { Icon::Square })
                .size(px(14.))
                .text_color(if on { t.green } else { t.muted }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(12.5))
                                .text_color(if on { t.green } else { t.foreground })
                                .child(SharedString::from(m.name.clone())),
                        )
                        .when(m.name != m.id, |el| {
                            el.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(10.5))
                                    .text_color(t.dim)
                                    .child(SharedString::from(m.id.clone())),
                            )
                        }),
                )
                .when(!m.description.is_empty(), |el| {
                    el.child(note(t.dim, 11., m.description.clone()))
                }),
        )
        .into_any_element()
}

fn effort_row(v: &ChatBrainView, t: &Theme, cx: &mut Context<ChatBrainView>) -> AnyElement {
    let choices = effort_choices(v);
    let picked = picked_effort(v);
    let model = current_model(v).cloned();
    let mut out = div()
        .id("cb-effort")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(sub_label(t, "Thinking effort"));
    if choices.len() > 1
        && let Some(model) = model
    {
        let mut row = div().w_full().flex().flex_wrap().gap(px(6.));
        for effort in choices {
            let on = effort == picked;
            let label = if effort.is_empty() {
                "Provider default".to_string()
            } else {
                effort.clone()
            };
            let tag = if effort.is_empty() {
                "default".to_string()
            } else {
                effort.clone()
            };
            let tip = if effort.is_empty() {
                "Let this model use its provider default".to_string()
            } else {
                format!("Use the provider's {effort} effort for {}", model.name)
            };
            let wanted = effort.clone();
            row = row.child(
                pick_chip(t, format!("cb-effort-{tag}"), &label, on, v.models_loading)
                    .tooltip(tooltip(tip))
                    .on_click(listen(cx, move |v, _, cx| set_effort(v, &wanted, cx))),
            );
        }
        out = out.child(row).child(note(
            t.dim,
            10.5,
            "Only choices advertised for this model. Applies from the next request.".into(),
        ));
    } else {
        let has_model = v.current.as_ref().is_some_and(|c| c.model.is_some());
        let words = if !has_model {
            "Pick a model to see its supported efforts."
        } else if v.models_loading {
            "Loading this model's effort choices…"
        } else {
            "No effort choices advertised for this model. Using the provider default."
        };
        out = out.child(note(t.dim, 11.5, words.into()));
    }
    out.into_any_element()
}

fn model_body(v: &ChatBrainView, t: &Theme, cx: &mut Context<ChatBrainView>) -> AnyElement {
    let has_model = v.current.as_ref().is_some_and(|c| c.model.is_some());
    let mut list = div()
        .id("cb-model-list")
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .p(px(4.))
        .gap(px(3.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .bg(t.surface);
    if v.models.is_empty() {
        let words = if v.choice.is_none() {
            "Pick a brain first."
        } else if v.models_loading {
            "Loading the model list…"
        } else {
            "No model list yet. Click Refresh to load it."
        };
        list = list.child(
            div()
                .px(px(4.))
                .py(px(3.))
                .child(note(t.muted, 12., words.into())),
        );
    }
    for m in &v.models {
        list = list.child(model_row(v, t, m, cx));
    }
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(10.))
        .when_some(v.models_error.clone(), |el, why| {
            el.child(note(t.red, 12., why))
        })
        .when(!has_model && !v.models.is_empty(), |el| {
            el.child(note(t.yellow, 12., "Click a model to use it.".into()))
        })
        .child(list)
        .child(effort_row(v, t, cx))
        .into_any_element()
}

fn refresh_chip(v: &ChatBrainView, t: &Theme, cx: &mut Context<ChatBrainView>) -> AnyElement {
    let label = if v.models_loading {
        "Loading…"
    } else {
        "Refresh"
    };
    chip(
        t,
        "cb-refresh".into(),
        Icon::Refresh,
        label,
        t.cyan,
        v.models_loading || v.choice.is_none(),
    )
    .tooltip(tooltip(format!(
        "Load the model list without turning {} on",
        v.name
    )))
    .on_click(listen(cx, |v, _, cx| refresh_models(v, cx)))
    .into_any_element()
}

fn down_box(t: &Theme, why: &str, cx: &mut Context<ChatBrainView>) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.yellow.opacity(0.45))
        .child(note(
            t.foreground,
            12.5,
            "The Nixa brain is not running, so these settings cannot be read now.".into(),
        ))
        .child(note(t.dim, 11., why.to_string()))
        .child(note(
            t.muted,
            11.5,
            "The window starts the brain by itself. Try again in a moment.".into(),
        ))
        .child(
            div().flex().child(
                button(t, "cb-retry", "Try again", ButtonKind::Secondary)
                    .debug_selector(|| "cb-retry".into())
                    .on_click(listen(cx, |v, _, cx| {
                        v.reach = Reach::Loading;
                        reload(v, true, cx);
                    })),
            ),
        )
        .into_any_element()
}

impl Render for ChatBrainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let v: &ChatBrainView = self;
        let mut body = div()
            .id("cb-body")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .pr(px(4.))
            .pb(px(12.))
            .overflow_y_scroll();
        match &v.reach {
            Reach::Loading => {
                body = body.child(note(t.dim, 12., "Connecting to settings…".into()));
            }
            Reach::Down(why) => {
                body = body.child(down_box(&t, why, cx));
            }
            Reach::Up => {
                let engine = engine_body(v, &t, cx);
                let models = model_body(v, &t, cx);
                let refresh = refresh_chip(v, &t, cx);
                body = body
                    .child(section(
                        &t,
                        "cb-engine",
                        "Engine".into(),
                        format!(
                            "Choose {}'s brain or add your own OpenAI, Anthropic or Ollama brain.",
                            v.name
                        ),
                        None,
                        engine,
                    ))
                    .child(section(
                        &t,
                        "cb-models",
                        format!("Model: {}", brain_label(v)),
                        format!("The model and thinking effort {}'s brain uses.", v.name),
                        Some(refresh),
                        models,
                    ));
            }
        }
        div()
            .id("chat-brain-part")
            .size_full()
            .flex()
            .flex_col()
            .debug_selector(|| "cb-part".into())
            .child(body)
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .debug_selector(|| "cb-footer".into())
                    .mt(px(6.))
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(note(t.dim, 11., footer_text(v))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

    #[derive(Default)]
    struct Fake {
        brains: Vec<Value>,
        choice: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        ollama_down: bool,
    }

    fn info(fake: &Fake) -> Value {
        match fake.choice.as_deref() {
            None => Value::Null,
            Some("codex") => {
                json!({"name": "Codex", "kind": "codex", "model": fake.model, "effort": fake.effort, "cli": true})
            }
            Some("dim") => {
                json!({"name": "DimCode", "kind": "dim", "cli": true})
            }
            Some(choice) => {
                let id = choice.trim_start_matches("api:");
                let name = fake
                    .brains
                    .iter()
                    .find(|b| b["id"] == id)
                    .map(|b| text(b, "name"))
                    .unwrap_or_default();
                json!({"name": name, "kind": "openai", "model": fake.model, "effort": fake.effort, "cli": false})
            }
        }
    }

    fn answer(fake: &mut Fake, method: &str, path: &str, body: &Value) -> (u16, Value) {
        match (method, path) {
            ("GET", "/brains") => (
                200,
                json!({
                    "ok": true,
                    "brains": fake.brains,
                    "cli": [
                        {"choice": "codex", "name": "Codex", "installed": true},
                        {"choice": "kimi", "name": "Kimi", "installed": false},
                        {"choice": "jcode", "name": "Jcode", "installed": true},
                        {"choice": "dim", "name": "DimCode", "installed": true},
                        {"choice": "omp", "name": "omp", "installed": true},
                        {"choice": "claude", "name": "Claude Code", "installed": true},
                        {"choice": "crush", "name": "Crush", "installed": true},
                    ],
                    "choice": fake.choice,
                }),
            ),
            ("GET", "/health") => (
                200,
                json!({"ok": true, "name": "Nixa", "brain": info(fake)}),
            ),
            ("POST", "/brain/choice") => {
                fake.choice = some_text(body, "choice");
                fake.model = None;
                fake.effort = None;
                (200, json!({"ok": true, "choice": fake.choice}))
            }
            ("GET", "/brain/models") => match fake.choice.as_deref() {
                Some("dim") => (502, json!({"ok": false, "error": "DimCode needs provider credentials."})),
                Some("codex") => (
                    200,
                    json!({"ok": true, "models": [
                        {"id": "gpt-5", "name": "GPT-5", "description": "Big", "efforts": ["low", "high", "high", ""]},
                        {"id": "gpt-5-mini", "name": "gpt-5-mini", "description": "", "efforts": []},
                    ]}),
                ),
                Some(_) => (
                    200,
                    json!({"ok": true, "models": [{"id": "local-1", "name": "local-1", "description": "", "efforts": []}]}),
                ),
                None => (
                    400,
                    json!({"ok": false, "error": "No brain is picked yet."}),
                ),
            },
            ("POST", "/brain/model") => {
                if let Some(model) = some_text(body, "model") {
                    fake.model = Some(model);
                }
                if body.get("effort").is_some() {
                    fake.effort = some_text(body, "effort");
                }
                (
                    200,
                    json!({"ok": true, "model": fake.model, "effort": fake.effort}),
                )
            }
            ("POST", "/brains") => {
                if text(body, "name") == "Bad" {
                    return (
                        400,
                        json!({"ok": false, "error": "The key was refused (401). Nothing was saved."}),
                    );
                }
                let id = some_text(body, "id").unwrap_or_else(|| "abcd1234".into());
                fake.brains.retain(|b| b["id"] != id.as_str());
                fake.brains.push(json!({
                    "id": id, "choice": format!("api:{id}"), "name": text(body, "name"), "kind": text(body, "kind"),
                    "kindTitle": kind_title(&text(body, "kind")), "baseUrl": text(body, "baseUrl"), "model": "", "keySource": "none",
                }));
                (
                    200,
                    json!({"ok": true, "id": id, "choice": format!("api:{id}")}),
                )
            }
            ("POST", "/brain/ollama") => {
                if fake.ollama_down {
                    return (
                        502,
                        json!({"ok": false, "error": "Ollama is not reachable at http://127.0.0.1:11434/v1. Could not reach the URL."}),
                    );
                }
                if !fake.brains.iter().any(|b| b["kind"] == OLLAMA) {
                    fake.brains.push(json!({
                        "id": "0a11a0aa", "choice": "api:0a11a0aa", "name": "Ollama", "kind": OLLAMA,
                        "kindTitle": "Ollama", "baseUrl": default_url(OLLAMA), "model": "small:1b", "keySource": "none",
                    }));
                }
                let brain = fake.brains.iter().find(|b| b["kind"] == OLLAMA).cloned().unwrap_or_default();
                fake.choice = some_text(&brain, "choice");
                (200, json!({"ok": true, "id": brain["id"], "choice": brain["choice"], "name": "Ollama"}))
            }
            ("DELETE", p) if p.starts_with("/brains/") => {
                let id = p.trim_start_matches("/brains/");
                fake.brains.retain(|b| b["id"] != id);
                if fake.choice.as_deref() == Some(format!("api:{id}").as_str()) {
                    fake.choice = None;
                }
                (200, json!({"ok": true}))
            }
            _ => (404, json!({"ok": false, "error": "no such route"})),
        }
    }

    fn serve(fake: Fake) -> (String, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let fake = Arc::new(Mutex::new(fake));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut first = String::new();
                reader.read_line(&mut first).ok();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut raw = vec![0u8; length];
                reader.read_exact(&mut raw).ok();
                let body: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
                let mut parts = first.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                let (code, reply) = answer(&mut fake.lock().expect("fake"), &method, &path, &body);
                log.lock().expect("seen").push((method, path, body));
                let text = reply.to_string();
                let head = format!(
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    text.len()
                );
                stream.write_all(head.as_bytes()).ok();
                stream.write_all(text.as_bytes()).ok();
            }
        });
        (addr, seen)
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mo-w42-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn calls(seen: &Seen) -> Vec<(String, String, Value)> {
        std::mem::take(&mut *seen.lock().expect("seen"))
    }

    fn paths_of(list: &[(String, String, Value)]) -> Vec<String> {
        list.iter().map(|(m, p, _)| format!("{m} {p}")).collect()
    }

    struct Host {
        view: Entity<ChatBrainView>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.view.clone())
        }
    }

    fn open(
        cx: &mut TestAppContext,
        addr: String,
        data: PathBuf,
    ) -> (Entity<ChatBrainView>, &mut VisualTestContext) {
        cx.update(nixa_ui::init);
        let (host, vcx) = cx.add_window_view(move |_, cx| Host {
            view: new(addr, data, cx),
        });
        let view = vcx.update(|_, cx| host.read(cx).view.clone());
        vcx.run_until_parked();
        (view, vcx)
    }

    fn click(vcx: &mut VisualTestContext, selector: &'static str) {
        vcx.update(|window, app| window.draw(app).clear());
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        vcx.simulate_click(bounds.center(), Modifiers::none());
        vcx.run_until_parked();
    }

    fn drawn(vcx: &mut VisualTestContext, selector: &'static str) -> bool {
        vcx.update(|window, app| window.draw(app).clear());
        vcx.debug_bounds(selector).is_some()
    }

    #[test]
    fn reads_brain_answers() {
        let list = json!({
            "brains": [{"id": "abcd1234", "choice": "api:abcd1234", "name": "Router", "kind": "openai", "kindTitle": "OpenAI", "baseUrl": "https://openrouter.ai/api/v1", "keySource": "keychain"}, {"name": "no id"}],
            "cli": [{"choice": "codex", "name": "Codex", "installed": true}, {"choice": "agy", "installed": false}, {}],
        });
        let own = parse_own(&list);
        assert_eq!(own.len(), 1);
        assert_eq!(own[0].choice, "api:abcd1234");
        assert_eq!(host_of(&own[0].base_url), "openrouter.ai");
        assert_eq!(key_words(&own[0].key_source), "key in the keychain");
        let cli = parse_cli(&list);
        assert_eq!(
            cli,
            vec![
                Cli {
                    choice: "codex".into(),
                    name: "Codex".into(),
                    installed: true
                },
                Cli {
                    choice: "agy".into(),
                    name: "agy".into(),
                    installed: false
                },
            ]
        );
        assert_eq!(parse_current(Some(&Value::Null)), None);
        let current = parse_current(Some(
            &json!({"name": "Codex", "kind": "codex", "model": "", "effort": "high", "cli": true}),
        ))
        .expect("current");
        assert_eq!(current.model, None);
        assert_eq!(current.effort.as_deref(), Some("high"));
        let models = parse_models(
            &json!({"models": [{"id": "m", "efforts": ["low", "low", "", 3, "high"]}, {"name": "no id"}]}),
        );
        assert_eq!(
            models,
            vec![Model {
                id: "m".into(),
                name: "m".into(),
                description: String::new(),
                efforts: vec!["low".into(), "high".into()]
            }]
        );
        assert_eq!(host_of("http://127.0.0.1:11434/v1"), "127.0.0.1:11434");
        assert_eq!(default_url("anthropic"), "https://api.anthropic.com");
        assert_eq!(kind_title("other"), "OpenAI");
    }

    #[test]
    fn effort_choices_follow_the_model() {
        let mut v = build("127.0.0.1:1".into(), temp("efforts"));
        assert!(effort_choices(&v).is_empty());
        v.models = vec![Model {
            id: "m".into(),
            name: "M".into(),
            description: String::new(),
            efforts: vec!["low".into(), "high".into()],
        }];
        v.current = Some(Current {
            name: "Codex".into(),
            kind: "codex".into(),
            model: Some("m".into()),
            effort: Some("max".into()),
            cli: true,
        });
        assert_eq!(
            effort_choices(&v),
            vec!["".to_string(), "low".into(), "high".into()]
        );
        assert_eq!(picked_effort(&v), "");
        v.current.as_mut().expect("current").effort = Some("high".into());
        assert_eq!(picked_effort(&v), "high");
        assert_eq!(brain_label(&v), "Codex");
        assert_eq!(footer_text(&v), "Your choices are saved.");
        v.off = true;
        assert!(footer_text(&v).contains("stays off"));
    }

    #[gpui::test]
    fn picks_a_brain_model_and_effort(cx: &mut TestAppContext) {
        let (addr, seen) = serve(Fake::default());
        let (view, vcx) = open(cx, addr, temp("pick"));
        assert_eq!(paths_of(&calls(&seen)), vec!["GET /brains", "GET /health"]);
        assert_eq!(vcx.update(|_, cx| view.read(cx).reach.clone()), Reach::Up);
        assert!(drawn(vcx, "cb-brain-kimi"));
        for selector in ["cb-brain-jcode", "cb-brain-dim", "cb-brain-omp", "cb-brain-claude", "cb-brain-crush"] {
            assert!(drawn(vcx, selector));
        }
        click(vcx, "cb-brain-kimi");
        assert!(calls(&seen).is_empty());
        click(vcx, "cb-brain-codex");
        let sent = calls(&seen);
        assert_eq!(
            paths_of(&sent),
            vec![
                "POST /brain/choice",
                "GET /brains",
                "GET /health",
                "GET /brain/models"
            ]
        );
        assert_eq!(sent[0].2, json!({"choice": "codex"}));
        assert_eq!(vcx.update(|_, cx| view.read(cx).models.len()), 2);
        assert!(!drawn(vcx, "cb-effort-high"));
        click(vcx, "cb-model-gpt-5");
        let sent = calls(&seen);
        assert_eq!(sent[0].2, json!({"model": "gpt-5"}));
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).current.clone().and_then(|c| c.model)),
            Some("gpt-5".into())
        );
        click(vcx, "cb-effort-high");
        assert_eq!(calls(&seen)[0].2, json!({"effort": "high"}));
        assert_eq!(vcx.update(|_, cx| picked_effort(view.read(cx))), "high");
        click(vcx, "cb-effort-default");
        assert_eq!(calls(&seen)[0].2, json!({"effort": ""}));
        assert_eq!(vcx.update(|_, cx| picked_effort(view.read(cx))), "");
        click(vcx, "cb-model-gpt-5");
        assert!(calls(&seen).is_empty());
        click(vcx, "cb-refresh");
        assert_eq!(paths_of(&calls(&seen)), vec!["GET /brain/models"]);
    }

    #[gpui::test]
    fn the_ollama_chip_makes_and_picks_the_ollama_brain(cx: &mut TestAppContext) {
        let (addr, seen) = serve(Fake::default());
        let (view, vcx) = open(cx, addr, temp("ollama"));
        calls(&seen);
        assert!(drawn(vcx, "cb-brain-ollama"));
        click(vcx, "cb-brain-ollama");
        let sent = calls(&seen);
        assert_eq!(
            paths_of(&sent),
            vec!["POST /brain/ollama", "GET /brains", "GET /health", "GET /brain/models"]
        );
        assert_eq!(vcx.update(|_, cx| view.read(cx).choice.clone()), Some("api:0a11a0aa".into()));
        assert_eq!(vcx.update(|_, cx| view.read(cx).models.len()), 1);
        assert!(drawn(vcx, "cb-brain-ollama"));
        assert!(!drawn(vcx, "cb-brain-api:0a11a0aa"));
        click(vcx, "cb-brain-ollama");
        assert!(calls(&seen).is_empty());
        click(vcx, "cb-brain-codex");
        calls(&seen);
        click(vcx, "cb-brain-ollama");
        assert_eq!(paths_of(&calls(&seen))[0], "POST /brain/ollama");
        assert_eq!(vcx.update(|_, cx| view.read(cx).choice.clone()), Some("api:0a11a0aa".into()));
    }

    #[gpui::test]
    fn ollama_down_shows_the_error_and_keeps_the_choice(cx: &mut TestAppContext) {
        let fake = Fake { choice: Some("codex".into()), ollama_down: true, ..Fake::default() };
        let (addr, seen) = serve(fake);
        let (view, vcx) = open(cx, addr, temp("ollama-down"));
        calls(&seen);
        click(vcx, "cb-brain-ollama");
        assert_eq!(paths_of(&calls(&seen)), vec!["POST /brain/ollama"]);
        let shown = vcx.update(|_, cx| view.read(cx).error.clone()).unwrap_or_default();
        assert!(shown.contains("Ollama is not reachable"), "{shown}");
        assert_eq!(vcx.update(|_, cx| view.read(cx).choice.clone()), Some("codex".into()));
    }

    #[test]
    fn ollama_is_an_add_kind() {
        assert!(API_KINDS.contains(&OLLAMA));
        assert_eq!(kind_title(OLLAMA), "Ollama");
        assert_eq!(default_url(OLLAMA), "http://127.0.0.1:11434/v1");
    }

    #[gpui::test]
    fn dimcode_can_be_picked_and_shows_the_provider_setup_error(cx: &mut TestAppContext) {
        let (addr, seen) = serve(Fake::default());
        let (view, vcx) = open(cx, addr, temp("dim"));
        calls(&seen);
        click(vcx, "cb-brain-dim");
        assert_eq!(calls(&seen)[0].2, json!({"choice": "dim"}));
        assert!(drawn(vcx, "cb-brain-dim"));
        assert_eq!(vcx.update(|_, cx| view.read(cx).models_error.clone()).as_deref(), Some("DimCode needs provider credentials."));
    }

    #[gpui::test]
    fn adds_edits_and_removes_an_own_brain(cx: &mut TestAppContext) {
        let (addr, seen) = serve(Fake::default());
        let (view, vcx) = open(cx, addr, temp("own"));
        calls(&seen);
        click(vcx, "cb-add");
        assert!(vcx.update(|_, cx| {
            view.read(cx)
                .form
                .as_ref()
                .is_some_and(|f| f.key.read(cx).is_masked())
        }));
        vcx.simulate_input("Bad");
        click(vcx, "cb-save");
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).form.as_ref().map(|f| f.saving.clone())),
            Some(Saving::Failed(
                "The key was refused (401). Nothing was saved.".into()
            ))
        );
        vcx.simulate_keystrokes("cmd-a backspace");
        vcx.simulate_input("Local");
        click(vcx, "cb-type-anthropic");
        vcx.update(|_, cx| {
            let form = view.read(cx).form.as_ref().expect("form");
            let (url, key) = (form.url.clone(), form.key.clone());
            url.update(cx, |i, cx| i.set_value("http://127.0.0.1:11434", cx));
            key.update(cx, |i, cx| i.set_value("not-a-real-key", cx));
        });
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        let sent = calls(&seen);
        let save = sent
            .iter()
            .rev()
            .find(|(m, p, _)| m == "POST" && p == "/brains")
            .expect("save");
        assert_eq!(
            save.2,
            json!({"kind": "anthropic", "name": "Local", "baseUrl": "http://127.0.0.1:11434", "apiKey": "not-a-real-key"})
        );
        assert!(paths_of(&sent).contains(&"POST /brain/choice".to_string()));
        assert!(vcx.update(|_, cx| view.read(cx).form.is_none()));
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).choice.clone()),
            Some("api:abcd1234".into())
        );
        assert_eq!(vcx.update(|_, cx| view.read(cx).own.len()), 1);
        assert_eq!(vcx.update(|_, cx| view.read(cx).models.len()), 1);
        click(vcx, "cb-edit-abcd1234");
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).form.as_ref().map(|f| (
                f.id.clone(),
                f.name.read(cx).value().to_string(),
                f.kind.clone()
            ))),
            Some((Some("abcd1234".into()), "Local".into(), "anthropic".into()))
        );
        vcx.simulate_keystrokes("escape");
        assert!(vcx.update(|_, cx| view.read(cx).form.is_none()));
        click(vcx, "cb-edit-abcd1234");
        vcx.simulate_input(" 2");
        click(vcx, "cb-save");
        let sent = calls(&seen);
        let save = sent
            .iter()
            .find(|(m, p, _)| m == "POST" && p == "/brains")
            .expect("edit");
        assert_eq!(save.2["id"], "abcd1234");
        assert_eq!(save.2["name"], "Local 2");
        assert_eq!(save.2["apiKey"], "");
        click(vcx, "cb-remove-abcd1234");
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).remove_ask.clone()),
            Some("abcd1234".into())
        );
        click(vcx, "cb-keep-abcd1234");
        assert_eq!(vcx.update(|_, cx| view.read(cx).remove_ask.clone()), None);
        calls(&seen);
        click(vcx, "cb-remove-abcd1234");
        click(vcx, "cb-delete-abcd1234");
        assert_eq!(paths_of(&calls(&seen))[0], "DELETE /brains/abcd1234");
        assert!(vcx.update(|_, cx| view.read(cx).own.is_empty()));
        assert_eq!(vcx.update(|_, cx| view.read(cx).choice.clone()), None);
        assert!(vcx.update(|_, cx| view.read(cx).models.is_empty()));
    }

    #[gpui::test]
    fn a_new_brain_is_picked_after_a_busy_call(cx: &mut TestAppContext) {
        let (addr, seen) = serve(Fake::default());
        let (view, vcx) = open(cx, addr, temp("busy"));
        click(vcx, "cb-add");
        vcx.simulate_input("Local");
        vcx.update(|_, cx| view.update(cx, |v, _| v.busy = Some("model".into())));
        click(vcx, "cb-save");
        assert!(!paths_of(&calls(&seen)).contains(&"POST /brain/choice".to_string()));
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).pending.clone()),
            Some("api:abcd1234".into())
        );
        vcx.update(|_, cx| {
            view.update(cx, |v, cx| {
                v.busy = None;
                run_pending(v, cx);
            })
        });
        vcx.run_until_parked();
        assert!(paths_of(&calls(&seen)).contains(&"POST /brain/choice".to_string()));
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).choice.clone()),
            Some("api:abcd1234".into())
        );
        assert_eq!(vcx.update(|_, cx| view.read(cx).pending.clone()), None);
    }

    #[gpui::test]
    fn works_while_nixa_is_off(cx: &mut TestAppContext) {
        let data = temp("off");
        std::fs::write(data.join(OFF_FILE), "").expect("off");
        let (addr, seen) = serve(Fake {
            choice: Some("codex".into()),
            ..Fake::default()
        });
        let (view, vcx) = open(cx, addr, data);
        assert_eq!(paths_of(&calls(&seen)), vec!["GET /brains", "GET /health"]);
        assert!(vcx.update(|_, cx| view.read(cx).off));
        assert!(vcx.update(|_, cx| view.read(cx).models.is_empty()));
        vcx.update(|window, app| window.draw(app).clear());
        let part = vcx.debug_bounds("cb-part").expect("part");
        let footer = vcx.debug_bounds("cb-footer").expect("footer");
        assert_eq!(footer.size.width, part.size.width);
        click(vcx, "cb-refresh");
        assert_eq!(paths_of(&calls(&seen)), vec!["GET /brain/models"]);
        click(vcx, "cb-model-gpt-5-mini");
        assert_eq!(calls(&seen)[0].2, json!({"model": "gpt-5-mini"}));
        assert!(drawn(vcx, "cb-model-gpt-5"));
    }

    #[gpui::test]
    fn a_missing_brain_shows_try_again(cx: &mut TestAppContext) {
        let (view, vcx) = open(cx, "127.0.0.1:1".into(), temp("down"));
        assert!(matches!(
            vcx.update(|_, cx| view.read(cx).reach.clone()),
            Reach::Down(_)
        ));
        click(vcx, "cb-retry");
        assert!(matches!(
            vcx.update(|_, cx| view.read(cx).reach.clone()),
            Reach::Down(_)
        ));
        let (addr, _) = serve(Fake::default());
        vcx.update(|_, cx| view.update(cx, |v, _| v.addr = addr));
        click(vcx, "cb-retry");
        assert_eq!(vcx.update(|_, cx| view.read(cx).reach.clone()), Reach::Up);
    }

    #[gpui::test]
    fn brain_events_reload_the_part(cx: &mut TestAppContext) {
        let (addr, seen) = serve(Fake::default());
        let (view, vcx) = open(cx, addr, temp("events"));
        calls(&seen);
        let used = vcx.update(|_, cx| apply(&view, &json!({"type": "chat", "text": "hi"}), cx));
        assert!(!used);
        let used = vcx.update(|_, cx| apply(&view, &json!({"type": "hello", "name": "Nova"}), cx));
        assert!(used);
        vcx.run_until_parked();
        assert_eq!(paths_of(&calls(&seen)), vec!["GET /brains", "GET /health"]);
        assert_eq!(vcx.update(|_, cx| view.read(cx).name.clone()), "Nixa");
    }
}
