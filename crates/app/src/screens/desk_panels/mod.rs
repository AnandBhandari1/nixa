pub mod model;
mod view;

#[cfg(test)]
mod tests;

pub use model::{Card, CardKey, Event, Loaded, Panel, Row, event_of, for_panel, key, label, panel_of, title};

use crate::links::brain::http;
use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Bounds, Context, Entity, Global, ScrollHandle, TitlebarOptions, WeakEntity, Window,
    WindowBounds, WindowOptions, px, size,
};
use nixa_protocol::paths;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub const REFRESH: Duration = Duration::from_secs(2);
pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const WIDTH: f32 = 880.0;
pub const HEIGHT: f32 = 720.0;
pub const DESKTOP_PATH: &str = "/desktop";
pub const ANSWER_PATH: &str = "/desktop/answer";
pub const READS_PATH: &str = "/desktop/reads";
pub const TALK_PATH: &str = "/talk";
pub const TALK_ANSWER_PATH: &str = "/talk/confirm";
pub const BROWSER_PATH: &str = "/browser";
pub const ANSWERED_KEPT: usize = 20;

pub type Call = Arc<dyn Fn(&str, &str, Option<Value>) -> Result<Value, String> + Send + Sync>;

pub fn brain() -> Call {
    Arc::new(|method, path, body| http(&paths::brain_addr(), method, path, body.as_ref(), CALL_WAIT).map_err(|e| format!("{e:#}")))
}

pub struct DeskView {
    pub panel: Panel,
    pub call: Call,
    pub data: PathBuf,
    pub rows: Vec<Row>,
    pub desk: Option<Card>,
    pub talk: Option<Card>,
    pub allow_reads: bool,
    pub browser: Option<Value>,
    pub browser_command: Option<Entity<nixa_ui::text_input::TextInput>>,
    pub answered: Vec<String>,
    pub answering: bool,
    pub linked: bool,
    pub error: Option<String>,
    pub ticking: bool,
    pub loading: bool,
    pub scroll: ScrollHandle,
}

pub fn build(panel: Panel, call: Call, data: PathBuf) -> DeskView {
    DeskView {
        panel,
        call,
        data,
        rows: Vec::new(),
        desk: None,
        talk: None,
        allow_reads: false,
        browser: None,
        browser_command: None,
        answered: Vec::new(),
        answering: false,
        linked: false,
        error: None,
        ticking: false,
        loading: false,
        scroll: ScrollHandle::new(),
    }
}

pub fn new(panel: Panel, call: Call, data: PathBuf, cx: &mut App) -> Entity<DeskView> {
    cx.new(|cx| {
        let mut v = build(panel, call, data);
        refresh(&mut v, cx);
        tick(&mut v, cx);
        v
    })
}

pub fn commands() -> Vec<(&'static str, &'static str, Panel)> {
    vec![
        ("Nixa's eyes", "See the desktop steps Nixa reads and answer its step card", Panel::Eyes),
        ("Nixa's browser", "See the browser steps Nixa takes and answer its step card", Panel::Browser),
        ("Nixa's hands", "See the clicks and typing Nixa asks for and answer its step card", Panel::Hands),
    ]
}

pub fn run(panel: Panel, cx: &mut App) {
    open(panel, cx);
}

pub fn show(what: &str, cx: &mut App) -> bool {
    match panel_of(what) {
        Some(panel) => {
            open(panel, cx);
            true
        }
        None => false,
    }
}

fn fresh(v: &DeskView, card: Option<Card>) -> Option<Card> {
    card.filter(|c| !v.answered.contains(&model::card_tag(c)))
}

pub fn take(v: &mut DeskView, msg: &Value) -> bool {
    match event_of(msg) {
        Event::Desk(card) => v.desk = fresh(v, card),
        Event::Talk(card) => v.talk = fresh(v, card),
        Event::TalkDone(id) => {
            let open = v.talk.as_ref().and_then(|c| match c.key {
                CardKey::Talk(n) => Some(n),
                CardKey::Desk(_) => None,
            });
            if id.is_some() && id != open {
                return false;
            }
            v.talk = None;
        }
        Event::Other => return false,
    }
    true
}

pub fn apply(msg: &Value, cx: &mut App) -> bool {
    let Some(view) = shared(cx) else { return false };
    view.update(cx, |v, cx| {
        let changed = take(v, msg);
        if changed {
            cx.notify();
        }
        changed
    })
}

pub fn set_loaded(v: &mut DeskView, result: Result<Loaded, String>) {
    match result {
        Ok(loaded) => {
            if !v.linked {
                v.error = None;
            }
            v.linked = true;
            v.desk = fresh(v, loaded.desk);
            v.talk = fresh(v, loaded.talk);
            v.rows = loaded.rows;
            v.browser = loaded.browser;
            v.allow_reads = loaded.allow_reads;
        }
        Err(_) => {
            v.linked = false;
            v.desk = None;
            v.talk = None;
            v.answered.clear();
        }
    }
}

pub fn fetch(call: &Call) -> Result<Loaded, String> {
    let desktop = call("GET", DESKTOP_PATH, None)?;
    let talk = call("GET", TALK_PATH, None).ok();
    let browser = call("GET", BROWSER_PATH, None).ok();
    Ok(model::loaded_of(&desktop, talk.as_ref(), browser.as_ref()))
}

pub fn refresh(v: &mut DeskView, cx: &mut Context<DeskView>) {
    if v.loading {
        return;
    }
    v.loading = true;
    let call = v.call.clone();
    let task = cx.background_executor().spawn(async move { fetch(&call) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            v.loading = false;
            if !v.answering {
                set_loaded(v, result);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn answer_body(key: &CardKey, yes: bool) -> (&'static str, Value) {
    match key {
        CardKey::Desk(id) => (ANSWER_PATH, json!({"id": id, "answer": if yes { "yes" } else { "no" }})),
        CardKey::Talk(id) => (TALK_ANSWER_PATH, json!({"id": id, "approve": yes})),
    }
}

pub fn answer(v: &mut DeskView, card: Card, yes: bool, cx: &mut Context<DeskView>) {
    if v.answering {
        return;
    }
    v.answering = true;
    v.error = None;
    let (path, body) = answer_body(&card.key, yes);
    let call = v.call.clone();
    let task = cx.background_executor().spawn(async move { call("POST", path, Some(body)) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            v.answering = false;
            match result {
                Ok(_) => {
                    v.answered.push(model::card_tag(&card));
                    let over = v.answered.len().saturating_sub(ANSWERED_KEPT);
                    v.answered.drain(..over);
                    if v.desk.as_ref().is_some_and(|c| c.key == card.key) {
                        v.desk = None;
                    }
                    if v.talk.as_ref().is_some_and(|c| c.key == card.key) {
                        v.talk = None;
                    }
                }
                Err(e) => v.error = Some(format!("Not sent to Nixa: {e}")),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn reads_of(reply: &Value, asked: bool) -> bool {
    reply.get("allowReads").and_then(Value::as_bool).unwrap_or(asked)
}

pub fn set_reads(v: &mut DeskView, on: bool, cx: &mut Context<DeskView>) {
    if v.answering {
        return;
    }
    v.answering = true;
    v.error = None;
    let call = v.call.clone();
    let task = cx.background_executor().spawn(async move { call("POST", READS_PATH, Some(json!({"allowReads": on}))) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            v.answering = false;
            match result {
                Ok(reply) => v.allow_reads = reads_of(&reply, on),
                Err(e) => v.error = Some(format!("Not sent to Nixa: {e}")),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

fn save_browser(v: &mut DeskView, body: Value, cx: &mut Context<DeskView>) {
    if v.answering || !v.linked || v.browser.is_none() {
        return;
    }
    v.answering = true;
    v.error = None;
    let call = v.call.clone();
    let task = cx.background_executor().spawn(async move { call("POST", BROWSER_PATH, Some(body)) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            v.answering = false;
            match result {
                Ok(reply) => v.browser = reply.get("browser").cloned(),
                Err(error) => v.error = Some(format!("Could not save browser settings: {error}")),
            }
            cx.notify();
        }).ok();
    }).detach();
    cx.notify();
}

pub fn switch(v: &mut DeskView, panel: Panel, window: &mut Window, cx: &mut Context<DeskView>) {
    v.panel = panel;
    v.scroll = ScrollHandle::new();
    window.set_window_title(title(panel));
    cx.notify();
}

fn tick(v: &mut DeskView, cx: &mut Context<DeskView>) {
    if v.ticking {
        return;
    }
    v.ticking = true;
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(REFRESH).await;
            if this.update(cx, refresh).is_err() {
                break;
            }
        }
    })
    .detach();
}

struct DeskWindow {
    handle: AnyWindowHandle,
    view: WeakEntity<DeskView>,
}

impl Global for DeskWindow {}

pub fn shared(cx: &App) -> Option<Entity<DeskView>> {
    cx.try_global::<DeskWindow>().and_then(|g| g.view.upgrade())
}

fn options(panel: Panel, cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(title(panel).into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(520.0), px(420.0))),
        ..WindowOptions::default()
    }
}

pub fn open(panel: Panel, cx: &mut App) {
    cx.defer(move |cx| open_now(panel, cx));
}

fn open_now(panel: Panel, cx: &mut App) {
    let existing = cx.try_global::<DeskWindow>().and_then(|g| g.handle.downcast::<DeskView>());
    if let Some(handle) = existing
        && handle
            .update(cx, |view, window, cx| {
                switch(view, panel, window, cx);
                refresh(view, cx);
                window.activate_window();
            })
            .is_ok()
    {
        return;
    }
    let data = paths::data_dir();
    let mut made = None;
    let opened = cx.open_window(options(panel, cx), |_, cx| {
        let view = new(panel, brain(), data, cx);
        made = Some(view.downgrade());
        view
    });
    if let (Ok(handle), Some(view)) = (opened, made) {
        cx.set_global(DeskWindow { handle: handle.into(), view });
    }
}
