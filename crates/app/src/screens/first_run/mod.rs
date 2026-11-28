pub mod host;
pub mod key;
#[cfg(target_os = "macos")]
mod mac;
pub mod perms;
pub mod view;
pub mod voice;

#[cfg(test)]
mod tests;

use crate::links::brain as link;
use gpui::prelude::*;
use gpui::{App, Context, Entity, EventEmitter, Subscription};
use key::{Brain, Saving};
use nixa_ui::text_input::{InputEvent, TextInput};
use perms::{Grant, Perm};
use serde_json::{Value, json};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
use voice::{Fetch, Models, ModelsState};

pub const DONE_KEY: &str = "app.first_run.done";
pub const SHOW: &str = "first-run";
pub const TICK: Duration = Duration::from_secs(2);
pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const SAVE_WAIT: Duration = Duration::from_secs(45);
pub const BRAIN_PART: &str = "Chat brain";
pub const NEED_SPACE: &str = "Add a space first. New tabs and agents open inside a space.";
pub const NEED_SPACE_ERROR: &str = "Add a space first. There is no space for a new tab. Nothing was run.";

#[derive(Clone, Copy)]
pub struct Hooks {
    pub check: fn(Perm) -> Grant,
    pub ask: fn(Perm),
    pub open: fn(&str),
    pub models: fn() -> Result<Models, String>,
    pub fetch: fn(Sender<Fetch>),
}

pub fn live() -> Hooks {
    Hooks {
        check: perms::check,
        ask: perms::ask,
        open: perms::open_outside,
        models: voice::status,
        fetch: voice::fetch,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirstRunEvent {
    PickSpace,
    Finish,
    Settings(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
}

pub struct FirstRunView {
    pub hooks: Hooks,
    pub addr: String,
    pub spaces: Vec<String>,
    pub shown: bool,
    pub need_space: bool,
    pub done: bool,
    pub asked: bool,
    focus: gpui::FocusHandle,
    focus_pending: bool,
    previous_focus: Option<gpui::FocusHandle>,
    pub grants: Vec<(Perm, Grant)>,
    pub asked_perm: Option<Perm>,
    pub models: ModelsState,
    pub brain: Brain,
    pub brains: Value,
    pub kind: String,
    pub key: Entity<TextInput>,
    pub saving: Saving,
    loads: u64,
    loading: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<FirstRunEvent> for FirstRunView {}

pub fn commands() -> Vec<(&'static str, &'static str, Action)> {
    vec![(
        "Set up Nixa",
        "Open the first run steps again: space, permissions, voice models and chat brain",
        Action::Open,
    )]
}

pub fn wanted(spaces: usize, asked: bool) -> bool {
    asked || spaces == 0
}

pub fn close(v: &mut FirstRunView) -> bool {
    if !v.asked {
        return false;
    }
    v.asked = false;
    v.focus_pending = false;
    true
}

pub fn steps_shown(v: &FirstRunView) -> bool {
    !v.done || v.asked
}

pub fn is_done(conn: Option<&nixa_store::Connection>) -> bool {
    conn.and_then(|c| nixa_store::settings::setting_bool(c, DONE_KEY, false).ok())
        .unwrap_or(false)
}

pub fn mark_done(conn: &nixa_store::Connection) -> anyhow::Result<()> {
    nixa_store::settings::set_setting_bool(conn, DONE_KEY, true)
}

pub fn build(hooks: Hooks, addr: String, done: bool, cx: &mut Context<FirstRunView>) -> FirstRunView {
    let key = cx.new(|cx| TextInput::new(cx, "API key").masked());
    let sub = cx.subscribe(&key, |v: &mut FirstRunView, _, event: &InputEvent, cx| {
        if let InputEvent::Submit(_) = event {
            save_key(v, cx);
            cx.notify();
        }
    });
    FirstRunView {
        hooks,
        addr,
        spaces: Vec::new(),
        shown: false,
        need_space: false,
        done,
        asked: false,
        focus: cx.focus_handle(),
        focus_pending: false,
        previous_focus: None,
        grants: perms::check_all(hooks.check),
        asked_perm: None,
        models: ModelsState::Loading,
        brain: Brain::Loading,
        brains: Value::Null,
        kind: key::KINDS[0].to_string(),
        key,
        saving: Saving::Idle,
        loads: 0,
        loading: false,
        _subs: vec![sub],
    }
}

pub fn new(hooks: Hooks, addr: String, done: bool, cx: &mut App) -> Entity<FirstRunView> {
    cx.new(|cx| {
        let v = build(hooks, addr, done, cx);
        start_ticks(cx);
        v
    })
}

pub fn set_spaces(v: &mut FirstRunView, names: Vec<String>, shown: bool) -> bool {
    if v.spaces == names && v.shown == shown {
        return false;
    }
    if !names.is_empty() {
        v.need_space = false;
    }
    v.spaces = names;
    v.shown = shown;
    true
}

pub fn ask_for_space(v: &mut FirstRunView) -> bool {
    if !v.spaces.is_empty() || v.need_space {
        return false;
    }
    v.need_space = true;
    true
}

pub fn refresh(v: &mut FirstRunView, cx: &mut Context<FirstRunView>) {
    check_perms(v);
    load_models(v, cx);
    load_brain(v, cx);
}

pub fn check_perms(v: &mut FirstRunView) -> bool {
    let fresh = perms::check_all(v.hooks.check);
    let changed = fresh != v.grants;
    v.grants = fresh;
    changed
}

pub fn ask(v: &mut FirstRunView, p: Perm) {
    (v.hooks.ask)(p);
    v.asked_perm = Some(p);
    check_perms(v);
}

pub fn open_settings(v: &FirstRunView, p: Perm) {
    (v.hooks.open)(&perms::settings_url(p));
}

pub fn pick_kind(v: &mut FirstRunView, kind: &str) {
    if key::KINDS.contains(&kind) && v.saving != Saving::Checking {
        v.kind = kind.to_string();
        v.saving = Saving::Idle;
    }
}

pub fn pick_space(cx: &mut Context<FirstRunView>) {
    cx.emit(FirstRunEvent::PickSpace);
}

pub fn finish(v: &mut FirstRunView, cx: &mut Context<FirstRunView>) {
    if v.spaces.is_empty() {
        return;
    }
    v.done = true;
    v.asked = false;
    cx.emit(FirstRunEvent::Finish);
}

pub fn brain_settings(cx: &mut Context<FirstRunView>) {
    cx.emit(FirstRunEvent::Settings(BRAIN_PART));
}

pub fn load_models(v: &mut FirstRunView, cx: &mut Context<FirstRunView>) {
    if matches!(v.models, ModelsState::Fetching { .. }) {
        return;
    }
    if !matches!(v.models, ModelsState::Known(_)) {
        v.models = ModelsState::Loading;
    }
    let read = v.hooks.models;
    job(cx, read, |v, result, _| {
        if matches!(v.models, ModelsState::Fetching { .. }) {
            return;
        }
        v.models = match result {
            Ok(models) => ModelsState::Known(models),
            Err(why) => ModelsState::Failed(why),
        };
    });
}

pub fn start_fetch(v: &mut FirstRunView, cx: &mut Context<FirstRunView>) {
    if matches!(v.models, ModelsState::Fetching { .. }) {
        return;
    }
    v.models = ModelsState::Fetching { line: "Starting the download".into() };
    let (tx, rx) = channel();
    (v.hooks.fetch)(tx);
    drain(rx, cx);
}

pub fn fetch_step(v: &mut FirstRunView, msg: Option<Fetch>, cx: &mut Context<FirstRunView>) -> bool {
    match msg {
        Some(Fetch::Line(line)) => {
            v.models = ModelsState::Fetching { line };
            true
        }
        Some(Fetch::Done(Ok(()))) => {
            v.models = ModelsState::Loading;
            load_models(v, cx);
            false
        }
        Some(Fetch::Done(Err(why))) => {
            v.models = ModelsState::Failed(why);
            false
        }
        None => {
            if matches!(v.models, ModelsState::Fetching { .. }) {
                v.models = ModelsState::Failed("The download stopped.".into());
            }
            false
        }
    }
}

fn drain(rx: Receiver<Fetch>, cx: &mut Context<FirstRunView>) {
    cx.spawn(async move |this, cx| {
        let mut rx = rx;
        loop {
            let next = cx.background_executor().spawn(async move {
                let msg = rx.recv().ok();
                (rx, msg)
            });
            let (back, msg) = next.await;
            rx = back;
            let more = this
                .update(cx, |v, cx| {
                    let more = fetch_step(v, msg, cx);
                    cx.notify();
                    more
                })
                .unwrap_or(false);
            if !more {
                break;
            }
        }
    })
    .detach();
}

pub fn load_brain(v: &mut FirstRunView, cx: &mut Context<FirstRunView>) {
    v.loads += 1;
    v.loading = true;
    let load = v.loads;
    let addr = v.addr.clone();
    job(
        cx,
        move || {
            let brains = call(&addr, "GET", "/brains", None, CALL_WAIT)?;
            let health = call(&addr, "GET", "/health", None, CALL_WAIT)?;
            Ok(json!({"brains": brains, "health": health}))
        },
        move |v, result: Result<Value, String>, _| {
            if v.loads != load {
                return;
            }
            v.loading = false;
            match result {
                Ok(both) => {
                    v.brain = key::parse(&both["brains"], &both["health"]);
                    v.brains = both["brains"].clone();
                }
                Err(why) => v.brain = Brain::Down(why),
            }
        },
    );
}

pub fn save_key(v: &mut FirstRunView, cx: &mut Context<FirstRunView>) {
    if v.saving == Saving::Checking {
        return;
    }
    let typed = v.key.read(cx).value().to_string();
    if let Some(problem) = key::key_problem(&typed) {
        v.saving = Saving::Failed(problem.into());
        return;
    }
    let name = key::unique_name(&v.kind, &v.brains);
    let body = key::body(&v.kind, &name, &typed);
    v.saving = Saving::Checking;
    let addr = v.addr.clone();
    job(
        cx,
        move || {
            let reply = call(&addr, "POST", "/brains", Some(body), SAVE_WAIT)?;
            let choice = reply
                .get("choice")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .ok_or("The brain saved the key but gave no brain to pick.")?
                .to_string();
            call(&addr, "POST", "/brain/choice", Some(json!({"choice": choice})), CALL_WAIT)
        },
        |v, result, cx| match result {
            Ok(_) => {
                v.saving = Saving::Idle;
                v.key.update(cx, |k, cx| k.set_value("", cx));
                load_brain(v, cx);
            }
            Err(why) => v.saving = Saving::Failed(why),
        },
    );
}

pub fn tick(v: &mut FirstRunView, cx: &mut Context<FirstRunView>) -> bool {
    if !v.shown {
        return false;
    }
    let changed = check_perms(v);
    if !v.loading && !matches!(v.brain, Brain::Ready { .. }) && v.saving != Saving::Checking {
        load_brain(v, cx);
    }
    changed
}

fn start_ticks(cx: &mut Context<FirstRunView>) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            let alive = this.update(cx, |v, cx| {
                if tick(v, cx) {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
}

fn call(addr: &str, method: &str, path: &str, body: Option<Value>, wait: Duration) -> Result<Value, String> {
    link::http(addr, method, path, body.as_ref(), wait).map_err(|e| format!("{e:#}"))
}

fn job<T: Send + 'static>(
    cx: &mut Context<FirstRunView>,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl FnOnce(&mut FirstRunView, Result<T, String>, &mut Context<FirstRunView>) + 'static,
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
