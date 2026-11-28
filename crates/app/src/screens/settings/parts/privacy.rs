use crate::links::brain as link;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Div, Entity, SharedString, Window, div, px};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::text_input::TextInput;
use nixa_ui::toggle::toggle_row;
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};
use std::time::Duration;

pub const PART: &str = "Privacy";
pub const OWNER_PATH: &str = "/privacy/owner";
pub const CHECK_PATH: &str = "/privacy/check";
pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const MAX_ALIASES: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    pub on: bool,
    pub brain: String,
    pub model: String,
}

pub struct PrivacyView {
    pub addr: String,
    pub owner: Entity<TextInput>,
    pub aliases: Entity<TextInput>,
    pub brain: Entity<TextInput>,
    pub model: Entity<TextInput>,
    pub check: Check,
    pub saved: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub note: Option<String>,
    pub sent: Vec<(String, Value)>,
    pub owner_seq: u64,
    pub check_seq: u64,
    pub pending: usize,
}

pub fn split_names(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in text.split([',', '\n']).map(str::trim).filter(|n| !n.is_empty()) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(name)) && out.len() < MAX_ALIASES {
            out.push(name.to_string());
        }
    }
    out
}

pub fn owner_body(owner: &str, aliases: &str) -> Value {
    json!({"owner": owner.trim(), "aliases": split_names(aliases)})
}

pub fn check_body(on: bool, brain: &str, model: &str) -> Value {
    let brain = brain.trim();
    let model = model.trim();
    json!({"on": on, "brain": if brain.is_empty() { Value::Null } else { json!(brain) }, "model": if model.is_empty() { Value::Null } else { json!(model) }})
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn owner_of(reply: &Value) -> Option<(String, Vec<String>, bool)> {
    let view = reply.get("owner").filter(|o| o.is_object()).unwrap_or(reply);
    let owner = view.get("owner")?.as_str()?.to_string();
    let aliases = view
        .get("aliases")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    Some((owner, aliases, view.get("saved").and_then(Value::as_bool).unwrap_or(false)))
}

pub fn check_of(reply: &Value) -> Option<Check> {
    let s = reply.get("setting").filter(|s| s.is_object())?;
    Some(Check { on: s.get("on").and_then(Value::as_bool).unwrap_or(false), brain: text(s, "brain"), model: text(s, "model") })
}

fn input(placeholder: &'static str, cx: &mut Context<PrivacyView>) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(cx, placeholder))
}

pub fn build(addr: String, cx: &mut Context<PrivacyView>) -> PrivacyView {
    PrivacyView {
        addr,
        owner: input("Your first name", cx),
        aliases: input("Other names, for example a nickname, with commas", cx),
        brain: input("Saved API brain id", cx),
        model: input("Model name, empty for the brain's model", cx),
        check: Check::default(),
        saved: false,
        busy: false,
        error: None,
        note: None,
        sent: Vec::new(),
        owner_seq: 0,
        check_seq: 0,
        pending: 0,
    }
}

pub fn begin(v: &mut PrivacyView, path: &str) -> u64 {
    v.pending += 1;
    v.busy = true;
    if path == OWNER_PATH {
        v.owner_seq += 1;
        v.owner_seq
    } else {
        v.check_seq += 1;
        v.check_seq
    }
}

pub fn finish(v: &mut PrivacyView, path: &str, seq: u64) -> bool {
    v.pending = v.pending.saturating_sub(1);
    v.busy = v.pending > 0;
    seq == if path == OWNER_PATH { v.owner_seq } else { v.check_seq }
}

pub fn take_owner(v: &mut PrivacyView, reply: &Value, cx: &mut Context<PrivacyView>) -> bool {
    let Some((owner, aliases, saved)) = owner_of(reply) else { return false };
    v.owner.update(cx, |i, cx| i.set_value(owner, cx));
    v.aliases.update(cx, |i, cx| i.set_value(aliases.join(", "), cx));
    v.saved = saved;
    cx.notify();
    true
}

pub fn take_check(v: &mut PrivacyView, reply: &Value, cx: &mut Context<PrivacyView>) -> bool {
    let Some(check) = check_of(reply) else { return false };
    v.brain.update(cx, |i, cx| i.set_value(check.brain.clone(), cx));
    v.model.update(cx, |i, cx| i.set_value(check.model.clone(), cx));
    v.check = check;
    cx.notify();
    true
}

pub fn apply(view: &Entity<PrivacyView>, msg: &Value, cx: &mut App) -> bool {
    match msg.get("type").and_then(Value::as_str) {
        Some("privacy_owner") => view.update(cx, |v, cx| {
            v.error = msg.get("error").and_then(Value::as_str).map(str::to_string);
            take_owner(v, msg, cx)
        }),
        Some("privacy_check") => view.update(cx, |v, cx| {
            v.error = msg.get("error").and_then(Value::as_str).map(str::to_string);
            take_check(v, msg, cx)
        }),
        _ => false,
    }
}

fn call(v: &mut PrivacyView, method: &'static str, path: &'static str, body: Option<Value>, cx: &mut Context<PrivacyView>) {
    if let Some(body) = &body {
        v.sent.push((path.to_string(), body.clone()));
    }
    let seq = begin(v, path);
    let addr = v.addr.clone();
    let task = cx.background_executor().spawn(async move { link::http(&addr, method, path, body.as_ref(), CALL_WAIT).map_err(|e| format!("{e:#}")) });
    cx.spawn(async move |this, cx| {
        let reply = task.await;
        this.update(cx, |v, cx| {
            if !finish(v, path, seq) {
                cx.notify();
                return;
            }
            match reply {
                Ok(reply) => {
                    v.error = None;
                    let took = if path == OWNER_PATH { take_owner(v, &reply, cx) } else { take_check(v, &reply, cx) };
                    if took && method == "POST" {
                        v.note = Some("Saved.".into());
                    }
                }
                Err(e) => v.error = Some(e),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn reload(v: &mut PrivacyView, cx: &mut Context<PrivacyView>) {
    call(v, "GET", OWNER_PATH, None, cx);
    call(v, "GET", CHECK_PATH, None, cx);
}

pub fn save_names(v: &mut PrivacyView, cx: &mut Context<PrivacyView>) {
    let body = owner_body(v.owner.read(cx).value(), v.aliases.read(cx).value());
    v.note = None;
    call(v, "POST", OWNER_PATH, Some(body), cx);
}

pub fn save_check(v: &mut PrivacyView, on: bool, cx: &mut Context<PrivacyView>) {
    let body = check_body(on, v.brain.read(cx).value(), v.model.read(cx).value());
    v.note = None;
    call(v, "POST", CHECK_PATH, Some(body), cx);
}

pub fn new(addr: String, cx: &mut App) -> Entity<PrivacyView> {
    cx.new(|cx| {
        let mut v = build(addr, cx);
        reload(&mut v, cx);
        v
    })
}

pub struct Shared(pub Entity<PrivacyView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<PrivacyView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<PrivacyView> {
    if let Some(view) = shared(cx) {
        view.update(cx, reload);
        return view;
    }
    let view = new(nixa_protocol::paths::brain_addr(), cx);
    cx.set_global(Shared(view.clone()));
    view
}

fn card(t: &Theme) -> Div {
    div().w_full().flex().flex_col().gap(px(8.)).p(px(12.)).rounded(px(8.)).border_1().border_color(t.border)
}

fn note(color: gpui::Hsla, size: f32, text: impl Into<SharedString>) -> Div {
    div().text_size(px(size)).text_color(color).child(text.into())
}

fn field(t: &Theme, label: &str, input: &Entity<TextInput>, tag: &'static str, about: &str) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(note(t.muted, 11., label.to_string()))
        .child(div().w_full().flex().flex_col().debug_selector(move || tag.to_string()).child(input.clone()))
        .child(note(t.dim, 10.5, about.to_string()))
}

pub fn body(v: &PrivacyView, t: &Theme, cx: &mut Context<PrivacyView>) -> AnyElement {
    let on = v.check.on;
    let names = card(t)
        .child(div().text_size(px(13.)).text_color(t.bright).child("Your names"))
        .child(note(t.dim, 11.5, "Nixa keeps private things about these names away from cloud brains. Without a saved name it uses your Mac login name."))
        .child(field(t, "Your name", &v.owner, "privacy-owner", if v.saved { "Saved." } else { "Not saved yet. The login name is used." }))
        .child(field(t, "Other names", &v.aliases, "privacy-aliases", "Up to 10 names, with commas between them."))
        .child(
            div().flex().child(
                button(t, "privacy-save-names", "Save names", ButtonKind::Secondary)
                    .debug_selector(|| "privacy-save-names".into())
                    .on_click(cx.listener(|v, _, _, cx| save_names(v, cx))),
            ),
        );
    let check = card(t)
        .child(
            toggle_row(t, "privacy-check", "Message check", Some("Before a message goes to a cloud brain, a saved API brain checks it for private things.".into()), on)
                .debug_selector(|| "privacy-check".into())
                .on_click(cx.listener(move |v, _, _, cx| save_check(v, !on, cx))),
        )
        .child(field(t, "Check brain", &v.brain, "privacy-check-brain", "The id of a saved API brain from Chat brain. Best a local one."))
        .child(field(t, "Check model", &v.model, "privacy-check-model", "Leave empty to use that brain's own model."))
        .child(
            div().flex().child(
                button(t, "privacy-save-check", "Save check", ButtonKind::Secondary)
                    .debug_selector(|| "privacy-save-check".into())
                    .on_click(cx.listener(move |v, _, _, cx| save_check(v, on, cx))),
            ),
        );
    div()
        .id("privacy-part")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(names)
        .child(check)
        .when_some(v.note.clone(), |el, n| el.child(note(t.green, 12., n)))
        .when_some(v.error.clone(), |el, e| el.child(note(t.yellow, 12., e)))
        .into_any_element()
}

impl Render for PrivacyView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div()
            .id("settings-privacy-scroll")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .child(body(self, &t, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};

    #[test]
    fn names_and_check_bodies() {
        assert_eq!(split_names(" Sam, sam ,, Sammy\nS. "), vec!["Sam", "Sammy", "S."]);
        assert_eq!(split_names(&(0..20).map(|n| format!("n{n}")).collect::<Vec<_>>().join(",")).len(), MAX_ALIASES);
        assert_eq!(owner_body(" Sam ", "Sammy, S"), json!({"owner": "Sam", "aliases": ["Sammy", "S"]}));
        assert_eq!(check_body(true, " local ", ""), json!({"on": true, "brain": "local", "model": null}));
        assert_eq!(check_body(false, "", "m"), json!({"on": false, "brain": null, "model": "m"}));
        let reply = json!({"ok": true, "owner": "Sam", "aliases": ["Sammy"], "saved": true});
        assert_eq!(owner_of(&reply), Some(("Sam".into(), vec!["Sammy".into()], true)));
        let event = json!({"type": "privacy_owner", "owner": {"owner": "Ann", "aliases": [], "saved": false}});
        assert_eq!(owner_of(&event), Some(("Ann".into(), Vec::new(), false)));
        assert_eq!(check_of(&json!({"setting": {"on": true, "brain": "b1", "model": null}})), Some(Check { on: true, brain: "b1".into(), model: String::new() }));
        assert_eq!(check_of(&json!({"ok": true})), None);
    }

    #[gpui::test]
    fn a_late_reply_does_not_undo_a_save(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let (view, vcx) = cx.add_window_view(|_, cx| build("127.0.0.1:9".into(), cx));
        vcx.update(|_, cx| {
            view.update(cx, |v, _| {
                let first_get = begin(v, OWNER_PATH);
                let check_get = begin(v, CHECK_PATH);
                let save = begin(v, OWNER_PATH);
                assert!(v.busy);
                assert!(finish(v, OWNER_PATH, save));
                assert!(v.busy);
                assert!(!finish(v, OWNER_PATH, first_get));
                assert!(v.busy);
                assert!(finish(v, CHECK_PATH, check_get));
                assert!(!v.busy);
            });
        });
    }

    #[gpui::test]
    fn the_part_shows_and_saves_the_settings(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let (view, vcx) = cx.add_window_view(|_, cx| build("127.0.0.1:9".into(), cx));
        vcx.update(|_, cx| {
            assert!(apply(&view, &json!({"type": "privacy_owner", "owner": {"owner": "Sam", "aliases": ["Sammy"], "saved": true}}), cx));
            assert!(apply(&view, &json!({"type": "privacy_check", "setting": {"on": false, "brain": "b1", "model": null}}), cx));
            assert!(!apply(&view, &json!({"type": "phone"}), cx));
            let v = view.read(cx);
            assert_eq!(v.owner.read(cx).value(), "Sam");
            assert_eq!(v.aliases.read(cx).value(), "Sammy");
            assert_eq!(v.brain.read(cx).value(), "b1");
        });
        vcx.run_until_parked();
        for tag in ["privacy-owner", "privacy-aliases", "privacy-check", "privacy-check-brain", "privacy-save-names"] {
            assert!(vcx.debug_bounds(tag).is_some(), "{tag} is not drawn");
        }
        let toggle = vcx.debug_bounds("privacy-check").unwrap();
        vcx.simulate_click(toggle.center(), Modifiers::none());
        let save = vcx.debug_bounds("privacy-save-names").unwrap();
        vcx.simulate_click(save.center(), Modifiers::none());
        vcx.update(|_, cx| {
            let sent = view.read(cx).sent.clone();
            assert_eq!(sent[0], (CHECK_PATH.to_string(), json!({"on": true, "brain": "b1", "model": null})));
            assert_eq!(sent[1], (OWNER_PATH.to_string(), json!({"owner": "Sam", "aliases": ["Sammy"]})));
        });
    }
}
