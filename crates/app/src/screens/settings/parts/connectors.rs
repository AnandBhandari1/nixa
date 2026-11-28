use super::outside::{self, Call, card, flag, note, row, text, title};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, SharedString, Window, div, px};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::text_input::TextInput;
use nixa_ui::toggle::toggle_row;
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const PART: &str = "Connectors";
pub const PATH: &str = "/connectors";
pub const ABOUT: &str = "A connector links Nixa to a tool you choose, with your own key. None ship with Nixa. Each one is a manifest file you put in the folder below. Outside writes run and go to the audit log.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Secret {
    pub entry: String,
    pub set: bool,
    pub required: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Action {
    pub name: String,
    pub permission: String,
    pub asks: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Connector {
    pub name: String,
    pub description: String,
    pub kind: String,
    pub status: String,
    pub trusted: bool,
    pub secrets: Vec<Secret>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub folder: String,
    pub connectors: Vec<Connector>,
}

pub struct ConnectorsView {
    pub call: Call,
    pub listing: Option<Listing>,
    pub inputs: BTreeMap<String, Entity<TextInput>>,
    pub error: Option<String>,
    pub note: Option<String>,
}

fn list<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten()
}

pub fn connector_of(v: &Value) -> Connector {
    Connector {
        name: text(v, "connector"),
        description: text(v, "description"),
        kind: text(v, "kind"),
        status: text(v, "status"),
        trusted: flag(v, "trusted"),
        secrets: list(v, "secrets").map(|s| Secret { entry: text(s, "entry"), set: flag(s, "set"), required: flag(s, "required") }).collect(),
        actions: list(v, "actions").map(|a| Action { name: text(a, "action"), permission: text(a, "permission"), asks: flag(a, "asksEveryTime") }).collect(),
    }
}

pub fn listing_of(reply: &Value) -> Option<Listing> {
    let view = reply.get("connectors").filter(|c| c.is_object())?;
    Some(Listing { folder: text(view, "folder"), connectors: list(view, "connectors").map(connector_of).filter(|c| !c.name.is_empty()).collect() })
}

pub fn trust_body(name: &str, on: bool) -> Value {
    json!({"type": if on { "trust" } else { "untrust" }, "connector": name})
}

pub fn secret_body(entry: &str, value: &str) -> Value {
    json!({"type": "secret", "entry": entry, "value": value.trim()})
}

pub fn build(call: Call) -> ConnectorsView {
    ConnectorsView { call, listing: None, inputs: BTreeMap::new(), error: None, note: None }
}

pub fn take(v: &mut ConnectorsView, reply: &Value, cx: &mut Context<ConnectorsView>) -> bool {
    let Some(listing) = listing_of(reply) else { return false };
    let entries: Vec<String> = listing.connectors.iter().flat_map(|c| c.secrets.iter().map(|s| s.entry.clone())).collect();
    v.inputs.retain(|k, _| entries.contains(k));
    for entry in entries {
        v.inputs.entry(entry).or_insert_with(|| cx.new(|cx| TextInput::new(cx, "Paste the value").masked()));
    }
    v.listing = Some(listing);
    cx.notify();
    true
}

pub fn apply(view: &Entity<ConnectorsView>, msg: &Value, cx: &mut App) -> bool {
    if msg.get("type").and_then(Value::as_str) != Some("connectors") {
        return false;
    }
    view.update(cx, |v, cx| take(v, msg, cx))
}

fn send(v: &mut ConnectorsView, method: &'static str, body: Option<Value>, done: String, cx: &mut Context<ConnectorsView>) {
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
                            v.note = Some(done);
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

pub fn reload(v: &mut ConnectorsView, cx: &mut Context<ConnectorsView>) {
    send(v, "GET", None, String::new(), cx);
}

pub fn set_trust(v: &mut ConnectorsView, name: String, on: bool, cx: &mut Context<ConnectorsView>) {
    let done = format!("{name} is {}.", if on { "trusted" } else { "not trusted" });
    send(v, "POST", Some(trust_body(&name, on)), done, cx);
}

pub fn save_secret(v: &mut ConnectorsView, entry: String, cx: &mut Context<ConnectorsView>) {
    let Some(input) = v.inputs.get(&entry).cloned() else { return };
    let value = input.read(cx).value().trim().to_string();
    if value.is_empty() {
        v.error = Some(format!("Type the value for {entry} first."));
        cx.notify();
        return;
    }
    input.update(cx, |i, cx| i.set_value("", cx));
    send(v, "POST", Some(secret_body(&entry, &value)), format!("{entry} saved in the keychain."), cx);
}

pub fn remove_secret(v: &mut ConnectorsView, entry: String, cx: &mut Context<ConnectorsView>) {
    let done = format!("{entry} removed.");
    send(v, "POST", Some(json!({"type": "secret_remove", "entry": entry})), done, cx);
}

pub fn new(call: Call, cx: &mut App) -> Entity<ConnectorsView> {
    cx.new(|cx| {
        let mut v = build(call);
        reload(&mut v, cx);
        v
    })
}

pub struct Shared(pub Entity<ConnectorsView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<ConnectorsView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<ConnectorsView> {
    if let Some(view) = shared(cx) {
        view.update(cx, reload);
        return view;
    }
    let view = new(outside::brain(), cx);
    cx.set_global(Shared(view.clone()));
    view
}

fn secret_row(v: &ConnectorsView, t: &Theme, s: &Secret, cx: &mut Context<ConnectorsView>) -> AnyElement {
    let entry = s.entry.clone();
    let (save_entry, gone_entry) = (entry.clone(), entry.clone());
    let state = match (s.set, s.required) {
        (true, _) => "saved",
        (false, true) => "needed",
        (false, false) => "not set",
    };
    let tag = format!("connector-secret-{entry}");
    let mut el = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(note(t.muted, 11., format!("{entry} ({state})")));
    if let Some(input) = v.inputs.get(&entry) {
        let shown = tag.clone();
        el = el.child(div().w_full().flex().flex_col().debug_selector(move || shown.clone()).child(input.clone()));
    }
    let save_tag = format!("connector-save-{entry}");
    let gone_tag = format!("connector-remove-{entry}");
    el.child(
        row()
            .child(button(t, SharedString::from(save_tag.clone()), "Save", ButtonKind::Secondary).debug_selector(move || save_tag.clone()).on_click(cx.listener(move |v, _, _, cx| save_secret(v, save_entry.clone(), cx))))
            .when(s.set, |r| r.child(button(t, SharedString::from(gone_tag.clone()), "Remove", ButtonKind::Ghost).debug_selector(move || gone_tag.clone()).on_click(cx.listener(move |v, _, _, cx| remove_secret(v, gone_entry.clone(), cx))))),
    )
    .into_any_element()
}

fn connector_card(v: &ConnectorsView, t: &Theme, c: &Connector, cx: &mut Context<ConnectorsView>) -> AnyElement {
    let name = c.name.clone();
    let trusted = c.trusted;
    let tag = format!("connector-trust-{name}");
    let actions: Vec<String> = c.actions.iter().map(|a| format!("{} ({}{})", a.name, a.permission, if a.asks { ", asks every time" } else { "" })).collect();
    let mut el = card(t)
        .child(row().child(title(t, &c.name)).child(note(t.dim, 11., format!("{} · {}", c.kind, c.status))))
        .when(!c.description.is_empty(), |el| el.child(note(t.dim, 11.5, c.description.clone())))
        .child(
            toggle_row(t, SharedString::from(tag.clone()), "Trusted", Some("A trusted connector runs its outside writes without asking each time. Stop steps, such as money, always ask.".into()), trusted)
                .debug_selector(move || tag.clone())
                .on_click(cx.listener(move |v, _, _, cx| set_trust(v, name.clone(), !trusted, cx))),
        );
    if !actions.is_empty() {
        el = el.child(note(t.muted, 11., format!("Steps: {}", actions.join(", "))));
    }
    for s in &c.secrets {
        el = el.child(secret_row(v, t, s, cx));
    }
    el.into_any_element()
}

pub fn body(v: &ConnectorsView, t: &Theme, cx: &mut Context<ConnectorsView>) -> AnyElement {
    let Some(listing) = v.listing.clone() else {
        return div()
            .id("connectors-part")
            .child(note(t.muted, 12., v.error.clone().unwrap_or_else(|| "Asking Nixa for the connectors…".into())))
            .into_any_element();
    };
    let mut out = div()
        .id("connectors-part")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(card(t).child(note(t.dim, 11.5, ABOUT)).child(note(t.muted, 11.5, format!("Connector folder: {}", listing.folder))).child(
            button(t, "connectors-reload", "Look again", ButtonKind::Secondary).debug_selector(|| "connectors-reload".into()).on_click(cx.listener(|v, _, _, cx| reload(v, cx))),
        ));
    if listing.connectors.is_empty() {
        out = out.child(div().debug_selector(|| "connectors-empty".into()).child(note(t.muted, 12., "No connectors yet.")));
    }
    for c in &listing.connectors {
        out = out.child(connector_card(v, t, c, cx));
    }
    out.when_some(v.note.clone(), |el, n| el.child(note(t.green, 12., n)))
        .when_some(v.error.clone(), |el, e| el.child(note(t.yellow, 12., e)))
        .into_any_element()
}

impl Render for ConnectorsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div()
            .id("settings-connectors-scroll")
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
    use std::sync::{Arc, Mutex};

    fn shop(trusted: bool, set: bool) -> Value {
        json!({"ok": true, "connectors": {"folder": "~/Library/Application Support/Nixa/connectors", "card": null, "connectors": [
            {"connector": "shop", "description": "Orders of a test shop", "kind": "http", "status": if set { "ready" } else { "needs secret: shop.token" }, "trusted": trusted, "hosts": ["shop.test"], "hostsChecked": true,
             "secrets": [{"entry": "shop.token", "set": set, "required": true}],
             "actions": [{"action": "orders", "description": "List orders", "permission": "read", "reach": "outside", "asksEveryTime": false}, {"action": "refund", "permission": "write", "reach": "outside", "asksEveryTime": true}]},
            {"connector": "broken", "status": "broken: bad manifest"}
        ]}})
    }

    #[test]
    fn the_listing_reads_connectors_secrets_and_steps() {
        let listing = listing_of(&shop(false, false)).unwrap();
        assert_eq!(listing.folder, "~/Library/Application Support/Nixa/connectors");
        assert_eq!(listing.connectors.len(), 2);
        let c = &listing.connectors[0];
        assert_eq!((c.name.as_str(), c.kind.as_str(), c.trusted), ("shop", "http", false));
        assert_eq!(c.secrets, vec![Secret { entry: "shop.token".into(), set: false, required: true }]);
        assert_eq!(c.actions[1], Action { name: "refund".into(), permission: "write".into(), asks: true });
        assert_eq!(listing.connectors[1].status, "broken: bad manifest");
        assert_eq!(trust_body("shop", true), json!({"type": "trust", "connector": "shop"}));
        assert_eq!(secret_body("shop.token", " v "), json!({"type": "secret", "entry": "shop.token", "value": "v"}));
        assert_eq!(listing_of(&json!({"ok": true})), None);
    }

    #[gpui::test]
    fn the_part_trusts_and_saves_a_secret(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let state = Arc::new(Mutex::new(shop(false, false)));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (s, log) = (state.clone(), sent.clone());
        let call: Call = Arc::new(move |_, _, body| {
            log.lock().unwrap().push(body.clone());
            let mut now = s.lock().unwrap();
            match body.as_ref().and_then(|b| b.get("type")).and_then(Value::as_str) {
                Some("trust") => *now = shop(true, false),
                Some("secret") => *now = shop(true, true),
                _ => {}
            }
            Ok(now.clone())
        });
        let (view, vcx) = cx.add_window_view(|_, cx| {
            let mut v = build(call);
            reload(&mut v, cx);
            v
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("connector-trust-shop").is_some());
        assert!(vcx.debug_bounds("connector-secret-shop.token").is_some());
        assert!(vcx.debug_bounds("connector-remove-shop.token").is_none());
        let trust = vcx.debug_bounds("connector-trust-shop").unwrap();
        vcx.simulate_click(trust.center(), Modifiers::none());
        vcx.run_until_parked();
        view.read_with(vcx, |v, _| assert!(v.listing.as_ref().unwrap().connectors[0].trusted));
        vcx.update(|_, cx| view.update(cx, |v, cx| v.inputs["shop.token"].update(cx, |i, cx| i.set_value("own-value", cx))));
        let save = vcx.debug_bounds("connector-save-shop.token").unwrap();
        vcx.simulate_click(save.center(), Modifiers::none());
        vcx.run_until_parked();
        view.read_with(vcx, |v, cx| {
            assert_eq!(v.inputs["shop.token"].read(cx).value(), "");
            assert_eq!(v.note.as_deref(), Some("shop.token saved in the keychain."));
        });
        assert!(vcx.debug_bounds("connector-remove-shop.token").is_some());
        let bodies: Vec<Value> = sent.lock().unwrap().iter().flatten().cloned().collect();
        assert_eq!(bodies, vec![trust_body("shop", true), secret_body("shop.token", "own-value")]);
    }

    #[gpui::test]
    fn an_empty_folder_says_no_connectors(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let call: Call = Arc::new(|_, _, _| Ok(json!({"ok": true, "connectors": {"folder": "~/x", "connectors": [], "card": null}})));
        let (_view, vcx) = cx.add_window_view(|_, cx| {
            let mut v = build(call);
            reload(&mut v, cx);
            v
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("connectors-empty").is_some());
    }
}
