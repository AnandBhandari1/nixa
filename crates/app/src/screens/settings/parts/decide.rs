use super::outside::{self, Call, card, flag, note, row, text, title};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, SharedString, Window, div, px};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::toggle::toggle_row;
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};

pub const PART: &str = "Decision model";
pub const PATH: &str = "/decide";
pub const EVENT: &str = "decide.settings";
pub const ABOUT: &str = "Off is the current setup. On sends small typed decisions to an OpenCode decision model first. When it is not sure enough, the chat brain decides as today.";
pub const MODELS: [&str; 3] = ["jev-1.14", "gpt-6-luna-decision", "gemini-decision-1"];
pub const LEVELS: [f64; 3] = [0.75, 0.85, 0.95];
pub const USES: [(&str, &str, &str); 9] = [
    ("questions", "Agent questions", "Pick the answer for a waiting agent. Risky answers still go to the chat brain."),
    ("review", "Task review", "Judge a task in review: done, needs a fix, or unclear."),
    ("drones", "Drone steps", "Answer yes or no questions inside drone steps."),
    ("topic", "Topic guard", "Refuse messages that are not about code, tech or using Nixa."),
    ("idle", "Close finished terminals", "In the idle check, close a finished agent terminal when its screen shows the work is done."),
    ("followup", "Close or keep after a job", "Answer the deep brain question: close a finished agent's terminal or keep it open."),
    ("routing", "Task routing", "Pick the agent map row and workflow for a task, and answer workflow stage questions."),
    ("screens", "Quiet agent check", "Before Nixa reports a quiet child agent, check if it is still working."),
    ("chat", "Chat lookup route", "Pick where a lookup question is answered: knowledge, memory, web or both. Email and calendar stay rule based."),
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decide {
    pub on: bool,
    pub model: String,
    pub level: f64,
    pub questions: bool,
    pub review: bool,
    pub drones: bool,
    pub topic: bool,
    pub idle: bool,
    pub followup: bool,
    pub routing: bool,
    pub screens: bool,
    pub chat: bool,
    pub server: String,
    pub calls: u64,
    pub failed: u64,
    pub cost: f64,
}

pub struct DecideView {
    pub call: Call,
    pub decide: Option<Decide>,
    pub error: Option<String>,
    pub note: Option<String>,
}

pub fn decide_of(reply: &Value) -> Option<Decide> {
    let s = reply.get("settings").filter(|s| s.is_object())?;
    let today = reply.get("today").cloned().unwrap_or(Value::Null);
    Some(Decide {
        on: flag(s, "on"),
        model: text(s, "model"),
        level: s.get("level").and_then(Value::as_f64).unwrap_or(0.85),
        questions: flag(s, "questions"),
        review: flag(s, "review"),
        drones: flag(s, "drones"),
        topic: flag(s, "topic"),
        idle: flag(s, "idle"),
        followup: flag(s, "followup"),
        routing: flag(s, "routing"),
        screens: flag(s, "screens"),
        chat: flag(s, "chat"),
        server: reply.get("server").and_then(Value::as_str).unwrap_or("starts when needed").to_string(),
        calls: today.get("calls").and_then(Value::as_u64).unwrap_or(0),
        failed: today.get("failed").and_then(Value::as_u64).unwrap_or(0),
        cost: today.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
    })
}

pub fn status_line(d: &Decide) -> String {
    let route = if d.on { "Decision model first" } else { "Current setup" };
    format!("Route: {route}. OpenCode server: {}. Today: {} calls, {} failed, ${:.4}.", d.server, d.calls, d.failed, d.cost)
}

pub fn use_on(d: &Decide, name: &str) -> bool {
    match name {
        "questions" => d.questions,
        "review" => d.review,
        "drones" => d.drones,
        "topic" => d.topic,
        "idle" => d.idle,
        "followup" => d.followup,
        "routing" => d.routing,
        "screens" => d.screens,
        "chat" => d.chat,
        _ => false,
    }
}

pub fn build(call: Call) -> DecideView {
    DecideView { call, decide: None, error: None, note: None }
}

pub fn take(v: &mut DecideView, reply: &Value, cx: &mut Context<DecideView>) -> bool {
    let Some(d) = decide_of(reply) else { return false };
    v.decide = Some(d);
    cx.notify();
    true
}

pub fn apply(view: &Entity<DecideView>, msg: &Value, cx: &mut App) -> bool {
    if msg.get("type").and_then(Value::as_str) != Some(EVENT) {
        return false;
    }
    let status = msg.get("status").cloned().unwrap_or(Value::Null);
    view.update(cx, |v, cx| take(v, &status, cx))
}

fn send(v: &mut DecideView, method: &'static str, body: Option<Value>, done: &'static str, cx: &mut Context<DecideView>) {
    v.note = None;
    let call = v.call.clone();
    let task = cx.background_executor().spawn(async move { call(method, PATH, body) });
    cx.spawn(async move |this, cx| {
        let reply = task.await;
        this.update(cx, |v, cx| {
            match reply {
                Ok(reply) => match outside::error_of(&reply) {
                    Some(e) => v.error = Some(e),
                    None => {
                        v.error = None;
                        take(v, &reply, cx);
                        if !done.is_empty() {
                            v.note = Some(done.into());
                        }
                    }
                },
                Err(e) => v.error = Some(e),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn reload(v: &mut DecideView, cx: &mut Context<DecideView>) {
    send(v, "GET", None, "", cx);
}

pub fn save(v: &mut DecideView, body: Value, cx: &mut Context<DecideView>) {
    send(v, "POST", Some(body), "Saved.", cx);
}

pub fn new(call: Call, cx: &mut App) -> Entity<DecideView> {
    cx.new(|cx| {
        let mut v = build(call);
        reload(&mut v, cx);
        v
    })
}

pub struct Shared(pub Entity<DecideView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<DecideView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<DecideView> {
    if let Some(view) = shared(cx) {
        view.update(cx, reload);
        return view;
    }
    let view = new(outside::brain(), cx);
    cx.set_global(Shared(view.clone()));
    view
}

fn pick(t: &Theme, tag: String, label: String, on: bool, body: Value, cx: &mut Context<DecideView>) -> impl IntoElement {
    let selector = tag.clone();
    button(t, SharedString::from(tag), label, if on { ButtonKind::Primary } else { ButtonKind::Secondary })
        .debug_selector(move || selector.clone())
        .on_click(cx.listener(move |v, _, _, cx| save(v, body.clone(), cx)))
}

pub fn body(v: &DecideView, t: &Theme, cx: &mut Context<DecideView>) -> AnyElement {
    let Some(d) = v.decide.clone() else {
        return div()
            .id("decide-part")
            .child(note(t.muted, 12., v.error.clone().unwrap_or_else(|| "Asking Nixa for the decision model setting…".into())))
            .into_any_element();
    };
    let on = d.on;
    let mut models = row();
    for m in MODELS {
        models = models.child(pick(t, format!("decide-model-{m}"), m.to_string(), d.model == m, json!({"model": m}), cx));
    }
    let mut levels = row();
    for l in LEVELS {
        levels = levels.child(pick(t, format!("decide-level-{}", (l * 100.0) as u32), format!("{l:.2}"), (d.level - l).abs() < 1e-6, json!({"level": l}), cx));
    }
    let main = card(t)
        .child(
            toggle_row(t, "decide-on", "Decision model first", Some(ABOUT.into()), on)
                .debug_selector(|| "decide-on".into())
                .on_click(cx.listener(move |v, _, _, cx| save(v, json!({"on": !on}), cx))),
        )
        .child(note(t.muted, 11., "Model"))
        .child(models)
        .child(note(t.muted, 11., "Sure enough level. Below it the chat brain decides."))
        .child(levels)
        .child(note(t.dim, 11.5, status_line(&d)));
    let mut uses = card(t).child(title(t, "Use it for")).child(note(t.dim, 11.5, "Each use works only while Decision model first is on."));
    for (name, label, about) in USES {
        let now = use_on(&d, name);
        let tag = format!("decide-use-{name}");
        let selector = tag.clone();
        uses = uses.child(
            toggle_row(t, SharedString::from(tag), label, Some(about.into()), now)
                .debug_selector(move || selector.clone())
                .on_click(cx.listener(move |v, _, _, cx| save(v, json!({name: !now}), cx))),
        );
    }
    div()
        .id("decide-part")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(main)
        .child(uses)
        .when_some(v.note.clone(), |el, n| el.child(note(t.green, 12., n)))
        .when_some(v.error.clone(), |el, e| el.child(note(t.yellow, 12., e)))
        .into_any_element()
}

impl Render for DecideView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div().id("settings-decide-scroll").size_full().min_h_0().overflow_y_scroll().child(body(self, &t, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};
    use std::sync::{Arc, Mutex};

    fn status(on: bool) -> Value {
        json!({"ok": true, "route": "Current setup", "server": "starts when needed", "models": MODELS,
            "settings": {"on": on, "model": "jev-1.14", "level": 0.85, "questions": true, "review": true, "drones": true},
            "today": {"day": "2026-11-26", "calls": 3, "failed": 1, "cost": 0.0002}})
    }

    #[test]
    fn reads_the_status() {
        let d = decide_of(&status(false)).unwrap();
        assert!(!d.on && d.questions && d.server == "starts when needed");
        assert_eq!((d.calls, d.failed), (3, 1));
        assert!(status_line(&d).starts_with("Route: Current setup. OpenCode server: starts when needed."));
        assert!(status_line(&d).ends_with("Today: 3 calls, 1 failed, $0.0002."));
        assert_eq!(decide_of(&json!({"ok": true})), None);
    }

    #[gpui::test]
    fn the_part_turns_it_on_and_changes_the_model_level_and_a_use(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let state = Arc::new(Mutex::new(status(false)));
        let sent: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let (st, se) = (state.clone(), sent.clone());
        let call: Call = Arc::new(move |_, _, body| {
            let mut now = st.lock().unwrap();
            if let Some(Value::Object(b)) = &body {
                se.lock().unwrap().push(Value::Object(b.clone()));
                for (k, val) in b {
                    now["settings"][k] = val.clone();
                }
            }
            Ok(now.clone())
        });
        let (view, vcx) = cx.add_window_view(|_, cx| {
            let mut v = build(call);
            reload(&mut v, cx);
            v
        });
        vcx.run_until_parked();
        for tag in ["decide-on", "decide-model-gemini-decision-1", "decide-level-95", "decide-use-review"] {
            let at = vcx.debug_bounds(tag).unwrap_or_else(|| panic!("{tag} is not drawn"));
            vcx.simulate_click(at.center(), Modifiers::none());
            vcx.run_until_parked();
        }
        view.read_with(vcx, |v, _| {
            let d = v.decide.as_ref().unwrap();
            assert!(d.on && !d.review);
            assert_eq!(d.model, "gemini-decision-1");
            assert!((d.level - 0.95).abs() < 1e-9);
        });
        assert_eq!(*sent.lock().unwrap(), vec![json!({"on": true}), json!({"model": "gemini-decision-1"}), json!({"level": 0.95}), json!({"review": false})]);
        vcx.update(|_, cx| assert!(apply(&view, &json!({"type": EVENT, "status": status(false)}), cx)));
        view.read_with(vcx, |v, _| assert!(!v.decide.as_ref().unwrap().on));
    }
}
