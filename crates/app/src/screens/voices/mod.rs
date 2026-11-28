pub mod model;
pub mod tree;
mod view;

#[cfg(test)]
mod tests;

pub use model::{VoiceState, parse};
pub use tree::{Row, build_rows, explicit_voice, row_ids, row_specs};

use crate::links::brain as link;
use gpui::{App, AppContext, Context, Entity, ScrollHandle, Task};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::Duration;

pub const REFRESH: Duration = Duration::from_secs(3);
pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const PATH: &str = "/voices";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub level: String,
    pub key: String,
    pub label: String,
    pub id: String,
}

impl Default for Target {
    fn default() -> Self {
        Target { level: "default".into(), key: String::new(), label: "Default voice".into(), id: "default".into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playing {
    pub key: String,
    pub id: Option<u64>,
    pub turn: u64,
}

pub struct VoicesView {
    pub addr: String,
    pub state: Option<VoiceState>,
    pub reached: bool,
    pub error: Option<String>,
    pub target: Target,
    pub playing: Option<Playing>,
    pub idled: HashSet<u64>,
    pub confirm_reset: bool,
    pub show_models: bool,
    pub busy: bool,
    pub seq: u64,
    pub turns: u64,
    pub sent: Vec<String>,
    pub last: Option<Value>,
    refresh: Option<Task<()>>,
    pub voices_scroll: ScrollHandle,
    pub rows_scroll: ScrollHandle,
}

pub fn build(addr: String) -> VoicesView {
    VoicesView {
        addr,
        state: None,
        reached: false,
        error: None,
        target: Target::default(),
        playing: None,
        idled: HashSet::new(),
        confirm_reset: false,
        show_models: false,
        busy: false,
        seq: 0,
        turns: 0,
        sent: Vec::new(),
        last: None,
        refresh: None,
        voices_scroll: ScrollHandle::new(),
        rows_scroll: ScrollHandle::new(),
    }
}

pub struct Shared(pub Entity<VoicesView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<VoicesView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<VoicesView> {
    let view = shared(cx).unwrap_or_else(|| {
        let view = cx.new(|_| build(nixa_protocol::paths::brain_addr()));
        cx.set_global(Shared(view.clone()));
        view
    });
    start_refresh(&view, cx);
    view
}

pub fn start_refresh(view: &Entity<VoicesView>, cx: &mut App) {
    view.update(cx, |v, cx| {
        if v.refresh.is_some() || v.addr.is_empty() {
            return;
        }
        v.refresh = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                let alive = this.update(cx, |v, cx| {
                    if !v.busy {
                        load(v, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
        if !v.busy {
            load(v, cx);
        }
    });
}

pub fn stop_refresh(view: &Entity<VoicesView>, cx: &mut App) {
    view.update(cx, |v, cx| {
        if v.refresh.take().is_some() {
            v.seq += 1;
        }
        stop_sample(v, cx);
    });
}

pub fn apply(view: &Entity<VoicesView>, msg: &Value, cx: &mut App) -> bool {
    if msg.get("type").and_then(Value::as_str) != Some("voices_idle") {
        return false;
    }
    let Some(id) = msg.get("id").and_then(Value::as_u64) else { return false };
    view.update(cx, |v, cx| {
        idle(v, id);
        cx.notify();
    });
    true
}

pub fn idle(v: &mut VoicesView, id: u64) {
    v.idled.insert(id);
    if v.playing.as_ref().is_some_and(|p| p.id == Some(id)) {
        v.playing = None;
    }
}

pub fn rows(v: &VoicesView) -> Vec<Row> {
    build_rows(v.state.as_ref())
}

pub fn take(v: &mut VoicesView, value: &Value) -> bool {
    match parse(value) {
        Some(state) => {
            v.state = Some(state);
            v.reached = true;
            v.error = None;
            true
        }
        None => false,
    }
}

fn call(addr: &str, body: &Value) -> Result<Value, String> {
    let out = link::http(addr, "POST", PATH, Some(body), CALL_WAIT).map_err(|e| format!("{e:#}"))?;
    Ok(out.get("data").cloned().unwrap_or(Value::Null))
}

fn job(
    v: &mut VoicesView,
    body: Value,
    cx: &mut Context<VoicesView>,
    done: impl FnOnce(&mut VoicesView, Result<Value, String>, &mut Context<VoicesView>) + 'static,
) {
    v.last = Some(body.clone());
    if v.addr.is_empty() {
        return;
    }
    let addr = v.addr.clone();
    let task = cx.background_executor().spawn(async move { call(&addr, &body) });
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

pub fn load(v: &mut VoicesView, cx: &mut Context<VoicesView>) {
    if v.addr.is_empty() {
        return;
    }
    v.seq += 1;
    let seq = v.seq;
    let list = rows(v);
    v.sent = row_ids(&list);
    job(v, json!({"action": "state", "rows": row_specs(&list)}), cx, move |v, result, cx| {
        if v.seq != seq {
            return;
        }
        match result {
            Ok(data) => {
                if take(v, &data) && row_ids(&rows(v)) != v.sent {
                    load(v, cx);
                }
            }
            Err(why) => {
                v.reached = false;
                v.error = Some(why);
            }
        }
    });
}

pub fn edit(v: &mut VoicesView, action: &str, args: Value, cx: &mut Context<VoicesView>) {
    if v.busy {
        return;
    }
    v.seq += 1;
    v.busy = !v.addr.is_empty();
    let mut body = match args {
        Value::Object(map) => Value::Object(map),
        _ => json!({}),
    };
    body["action"] = json!(action);
    body["rows"] = row_specs(&rows(v));
    job(v, body, cx, |v, result, cx| {
        v.busy = false;
        match result {
            Ok(data) => {
                take(v, &data);
                if v.refresh.is_some() && row_ids(&rows(v)) != v.sent {
                    load(v, cx);
                }
            }
            Err(why) => v.error = Some(why),
        }
    });
    cx.notify();
}

pub fn play(v: &mut VoicesView, key: &str, args: Value, cx: &mut Context<VoicesView>) {
    if v.playing.as_ref().is_some_and(|p| p.key == key) {
        v.playing = None;
        job(v, json!({"action": "stop"}), cx, |v, result, _| {
            if let Err(why) = result {
                v.error = Some(why);
            }
        });
        cx.notify();
        return;
    }
    v.turns += 1;
    let turn = v.turns;
    v.playing = Some(Playing { key: key.to_string(), id: None, turn });
    let mut body = match args {
        Value::Object(map) => Value::Object(map),
        _ => json!({}),
    };
    body["action"] = json!("play");
    job(v, body, cx, move |v, result, _| {
        let mine = v.playing.as_ref().is_some_and(|p| p.turn == turn);
        match result {
            Ok(out) => {
                v.error = None;
                let id = out.get("id").and_then(Value::as_u64);
                if mine {
                    match id {
                        Some(id) if !v.idled.contains(&id) => {
                            if let Some(p) = v.playing.as_mut() {
                                p.id = Some(id);
                            }
                        }
                        _ => v.playing = None,
                    }
                }
            }
            Err(why) => {
                if mine {
                    v.playing = None;
                }
                v.error = Some(why);
            }
        }
    });
    cx.notify();
}

pub fn is_playing(v: &VoicesView, key: &str) -> bool {
    v.playing.as_ref().is_some_and(|p| p.key == key)
}

pub fn choose(v: &mut VoicesView, row: &Row, cx: &mut Context<VoicesView>) {
    v.target = Target { level: row.level.to_string(), key: row.key.clone(), label: row.label.clone(), id: row.id.clone() };
    cx.notify();
}

pub fn use_voice(v: &mut VoicesView, voice: &str, cx: &mut Context<VoicesView>) {
    let args = json!({"level": v.target.level, "key": v.target.key, "voice": voice});
    edit(v, "set", args, cx);
}

pub fn clear(v: &mut VoicesView, row: &Row, cx: &mut Context<VoicesView>) {
    edit(v, "clear", json!({"level": row.level, "key": row.key}), cx);
}

pub fn toggle_humor(v: &mut VoicesView, cx: &mut Context<VoicesView>) {
    let on = v.state.as_ref().is_some_and(|s| s.humor_on());
    edit(v, "humor", json!({"value": if on { "off" } else { "light" }}), cx);
}

pub fn reset(v: &mut VoicesView, cx: &mut Context<VoicesView>) {
    v.confirm_reset = false;
    edit(v, "reset", json!({}), cx);
}

pub fn stop_sample(v: &mut VoicesView, cx: &mut Context<VoicesView>) {
    if v.playing.take().is_some() {
        job(v, json!({"action": "stop"}), cx, |_, _, _| {});
    }
    cx.notify();
}

pub fn current_voice(v: &VoicesView) -> Option<String> {
    let list = rows(v);
    let here = list.iter().find(|r| r.id == v.target.id)?;
    explicit_voice(&v.state.as_ref()?.map, here)
}
