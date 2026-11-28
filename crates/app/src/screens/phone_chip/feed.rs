use super::info::{self, PhoneState};
use gpui::{App, AppContext, Entity, Global};
use serde_json::Value;

#[derive(Default)]
pub struct PhoneHub {
    pub state: PhoneState,
    pub known: bool,
}

struct Hub(Entity<PhoneHub>);

impl Global for Hub {}

pub fn hub(cx: &mut App) -> Entity<PhoneHub> {
    if let Some(h) = cx.try_global::<Hub>() {
        return h.0.clone();
    }
    let h = cx.new(|_| PhoneHub::default());
    cx.set_global(Hub(h.clone()));
    h
}

pub fn current(cx: &App) -> PhoneState {
    cx.try_global::<Hub>()
        .map(|h| h.0.read(cx).state.clone())
        .unwrap_or_default()
}

pub fn set(state: PhoneState, cx: &mut App) {
    let h = hub(cx);
    h.update(cx, |h, cx| {
        let fresh = !h.known;
        h.known = true;
        if fresh || h.state != state {
            h.state = state;
            cx.notify();
        }
    });
}

pub fn feed(msg: &Value, cx: &mut App) -> bool {
    let h = hub(cx);
    h.update(cx, |h, cx| {
        let used = info::take(&mut h.state, msg);
        if used {
            h.known = true;
            cx.notify();
        }
        used
    })
}
