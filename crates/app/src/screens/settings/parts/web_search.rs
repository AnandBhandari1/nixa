use super::outside::{self, Call, card, field, flag, note, row, text, title};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, SharedString, Window, div, px};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::text_input::TextInput;
use nixa_ui::toggle::toggle_row;
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};

pub const PART: &str = "Web search";
pub const PATH: &str = "/web";
pub const ABOUT: &str = "Off by default. When it is on, Nixa sends only a short public question, never private names, paths, code or keys. Results are data, never orders.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Web {
    pub on: bool,
    pub provider: String,
    pub exa_key: bool,
    pub installed: bool,
    pub running: bool,
    pub port: u64,
    pub folder: String,
    pub settings: String,
}

pub struct WebView {
    pub call: Call,
    pub web: Option<Web>,
    pub key: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub folder: Entity<TextInput>,
    pub settings: Entity<TextInput>,
    pub error: Option<String>,
    pub note: Option<String>,
}

pub fn web_of(reply: &Value) -> Option<Web> {
    let w = reply.get("web").filter(|w| w.is_object())?;
    let s = w.get("searxng").cloned().unwrap_or(Value::Null);
    Some(Web {
        on: flag(w, "on"),
        provider: text(w, "provider"),
        exa_key: flag(w, "exa_key"),
        installed: flag(&s, "installed"),
        running: flag(&s, "running"),
        port: s.get("port").and_then(Value::as_u64).unwrap_or(0),
        folder: text(&s, "folder"),
        settings: text(&s, "settings"),
    })
}

pub fn searxng_body(port: &str, folder: &str, settings: &str) -> Result<Value, String> {
    let mut body = json!({"type": "set_web", "folder": folder.trim(), "settings": settings.trim()});
    let port = port.trim();
    if !port.is_empty() {
        let n: u64 = port.parse().map_err(|_| "The port must be a number.".to_string())?;
        body["port"] = json!(n);
    }
    Ok(body)
}

pub fn status_line(web: &Web) -> String {
    let state = if web.on { "on" } else { "off" };
    match web.provider.as_str() {
        "exa" => format!("Web search is {state}, with Exa. {}", if web.exa_key { "Your Exa key is saved in the keychain." } else { "No Exa key yet." }),
        _ => format!(
            "Web search is {state}, with local SearXNG on port {}. {}",
            web.port,
            match (web.installed, web.running) {
                (false, _) => "SearXNG is not installed in the folder below.",
                (true, true) => "SearXNG is running.",
                (true, false) => "SearXNG starts when Nixa first searches.",
            }
        ),
    }
}

fn input(placeholder: &'static str, masked: bool, cx: &mut Context<WebView>) -> Entity<TextInput> {
    cx.new(|cx| {
        let i = TextInput::new(cx, placeholder);
        if masked { i.masked() } else { i }
    })
}

pub fn build(call: Call, cx: &mut Context<WebView>) -> WebView {
    WebView {
        call,
        web: None,
        key: input("Your Exa key", true, cx),
        port: input("Port, for example 8888", false, cx),
        folder: input("SearXNG folder, empty for the default", false, cx),
        settings: input("SearXNG settings file, empty for the default", false, cx),
        error: None,
        note: None,
    }
}

pub fn take(v: &mut WebView, reply: &Value, cx: &mut Context<WebView>) -> bool {
    let Some(web) = web_of(reply) else { return false };
    let fresh = v.web.is_none();
    if fresh || v.web.as_ref().is_some_and(|w| w.port != web.port || w.folder != web.folder || w.settings != web.settings) {
        v.port.update(cx, |i, cx| i.set_value(web.port.to_string(), cx));
        v.folder.update(cx, |i, cx| i.set_value(web.folder.clone(), cx));
        v.settings.update(cx, |i, cx| i.set_value(web.settings.clone(), cx));
    }
    v.web = Some(web);
    cx.notify();
    true
}

pub fn apply(view: &Entity<WebView>, msg: &Value, cx: &mut App) -> bool {
    if msg.get("type").and_then(Value::as_str) != Some("web") {
        return false;
    }
    view.update(cx, |v, cx| take(v, msg, cx))
}

fn send(v: &mut WebView, method: &'static str, body: Option<Value>, done: &'static str, cx: &mut Context<WebView>) {
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

pub fn reload(v: &mut WebView, cx: &mut Context<WebView>) {
    send(v, "GET", None, "", cx);
}

pub fn set_on(v: &mut WebView, on: bool, cx: &mut Context<WebView>) {
    send(v, "POST", Some(json!({"type": "set_web", "on": on})), if on { "Web search is on." } else { "Web search is off." }, cx);
}

pub fn set_provider(v: &mut WebView, provider: &'static str, cx: &mut Context<WebView>) {
    send(v, "POST", Some(json!({"type": "set_web", "provider": provider})), "Saved.", cx);
}

pub fn save_key(v: &mut WebView, cx: &mut Context<WebView>) {
    let key = v.key.read(cx).value().trim().to_string();
    if key.is_empty() {
        v.error = Some("Type your Exa key first.".into());
        cx.notify();
        return;
    }
    v.key.update(cx, |i, cx| i.set_value("", cx));
    send(v, "POST", Some(json!({"type": "web_key", "key": key})), "Key saved in the keychain.", cx);
}

pub fn remove_key(v: &mut WebView, cx: &mut Context<WebView>) {
    send(v, "POST", Some(json!({"type": "web_key_remove"})), "Key removed.", cx);
}

pub fn save_searxng(v: &mut WebView, cx: &mut Context<WebView>) {
    match searxng_body(v.port.read(cx).value(), v.folder.read(cx).value(), v.settings.read(cx).value()) {
        Ok(body) => send(v, "POST", Some(body), "Saved.", cx),
        Err(e) => {
            v.error = Some(e);
            cx.notify();
        }
    }
}

pub fn stop(v: &mut WebView, cx: &mut Context<WebView>) {
    send(v, "POST", Some(json!({"type": "web_stop"})), "SearXNG stopped.", cx);
}

pub fn new(call: Call, cx: &mut App) -> Entity<WebView> {
    cx.new(|cx| {
        let mut v = build(call, cx);
        reload(&mut v, cx);
        v
    })
}

pub struct Shared(pub Entity<WebView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<WebView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<WebView> {
    if let Some(view) = shared(cx) {
        view.update(cx, reload);
        return view;
    }
    let view = new(outside::brain(), cx);
    cx.set_global(Shared(view.clone()));
    view
}

fn provider_button(t: &Theme, word: &'static str, label: &'static str, on: bool, cx: &mut Context<WebView>) -> impl IntoElement {
    let tag = format!("web-provider-{word}");
    button(t, SharedString::from(tag.clone()), label, if on { ButtonKind::Primary } else { ButtonKind::Secondary })
        .debug_selector(move || tag.clone())
        .on_click(cx.listener(move |v, _, _, cx| set_provider(v, word, cx)))
}

pub fn body(v: &WebView, t: &Theme, cx: &mut Context<WebView>) -> AnyElement {
    let Some(web) = v.web.clone() else {
        return div()
            .id("web-part")
            .child(note(t.muted, 12., v.error.clone().unwrap_or_else(|| "Asking Nixa for the web search settings…".into())))
            .into_any_element();
    };
    let on = web.on;
    let exa = web.provider == "exa";
    let main = card(t)
        .child(
            toggle_row(t, "web-on", "Web search", Some(ABOUT.into()), on)
                .debug_selector(|| "web-on".into())
                .on_click(cx.listener(move |v, _, _, cx| set_on(v, !on, cx))),
        )
        .child(note(t.muted, 11., "Search with"))
        .child(row().child(provider_button(t, "searxng", "Local SearXNG", !exa, cx)).child(provider_button(t, "exa", "Exa", exa, cx)))
        .child(note(t.dim, 11.5, status_line(&web)));
    let exa_card = card(t)
        .child(title(t, "Exa"))
        .child(note(t.dim, 11.5, "Exa is an outside search service. It uses your own key, kept in the macOS keychain. Nixa never shows the key again."))
        .child(field(t, "Exa key", &v.key, "web-exa-key", if web.exa_key { "A key is saved. Type a new one to change it." } else { "No key yet." }))
        .child(
            row()
                .child(button(t, "web-save-key", "Save key", ButtonKind::Secondary).debug_selector(|| "web-save-key".into()).on_click(cx.listener(|v, _, _, cx| save_key(v, cx))))
                .when(web.exa_key, |el| el.child(button(t, "web-remove-key", "Remove key", ButtonKind::Ghost).debug_selector(|| "web-remove-key".into()).on_click(cx.listener(|v, _, _, cx| remove_key(v, cx))))),
        );
    let local = card(t)
        .child(title(t, "Local SearXNG"))
        .child(note(t.dim, 11.5, "SearXNG runs on this Mac only. Install it yourself, then point Nixa at its folder. Nixa starts it when needed and stops it after 30 idle minutes."))
        .child(field(t, "Port", &v.port, "web-port", "1024 to 65535. Nixa's own ports are refused."))
        .child(field(t, "Folder", &v.folder, "web-folder", "A full path, or empty for the default."))
        .child(field(t, "Settings file", &v.settings, "web-settings", "A full path, or empty for the default."))
        .child(
            row()
                .child(button(t, "web-save-searxng", "Save", ButtonKind::Secondary).debug_selector(|| "web-save-searxng".into()).on_click(cx.listener(|v, _, _, cx| save_searxng(v, cx))))
                .when(web.running, |el| el.child(button(t, "web-stop", "Stop SearXNG", ButtonKind::Ghost).debug_selector(|| "web-stop".into()).on_click(cx.listener(|v, _, _, cx| stop(v, cx))))),
        );
    div()
        .id("web-part")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(main)
        .child(if exa { exa_card } else { local })
        .when_some(v.note.clone(), |el, n| el.child(note(t.green, 12., n)))
        .when_some(v.error.clone(), |el, e| el.child(note(t.yellow, 12., e)))
        .into_any_element()
}

impl Render for WebView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div()
            .id("settings-web-scroll")
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

    fn view_json(on: bool, provider: &str, key: bool) -> Value {
        json!({"ok": true, "web": {"on": on, "provider": provider, "providers": ["searxng", "exa"], "exa_key": key, "searxng": {"installed": false, "running": false, "port": 8888, "folder": "~/.local/share/searxng/src", "settings": "~/.config/searxng/settings.yml", "folder_set": false, "settings_set": false}}})
    }

    type Sent = Arc<Mutex<Vec<(String, Option<Value>)>>>;

    fn fake(state: Arc<Mutex<Value>>, sent: Sent) -> Call {
        Arc::new(move |method, path, body| {
            sent.lock().unwrap().push((format!("{method} {path}"), body.clone()));
            let mut now = state.lock().unwrap();
            if let Some(b) = &body {
                if let Some(on) = b.get("on").and_then(Value::as_bool) {
                    now["web"]["on"] = json!(on);
                }
                if let Some(p) = b.get("provider").and_then(Value::as_str) {
                    now["web"]["provider"] = json!(p);
                }
                if b.get("type").and_then(Value::as_str) == Some("web_key") {
                    now["web"]["exa_key"] = json!(true);
                }
            }
            Ok(now.clone())
        })
    }

    #[test]
    fn web_settings_read_and_bodies() {
        let web = web_of(&view_json(false, "searxng", false)).unwrap();
        assert!(!web.on);
        assert_eq!(web.port, 8888);
        assert_eq!(web.folder, "~/.local/share/searxng/src");
        assert!(status_line(&web).starts_with("Web search is off, with local SearXNG on port 8888."));
        assert!(status_line(&web_of(&view_json(true, "exa", true)).unwrap()).contains("Your Exa key is saved"));
        assert_eq!(web_of(&json!({"ok": true})), None);
        assert_eq!(searxng_body(" 9000 ", "", "/x/settings.yml").unwrap(), json!({"type": "set_web", "folder": "", "settings": "/x/settings.yml", "port": 9000}));
        assert!(searxng_body("abc", "", "").is_err());
        assert_eq!(searxng_body("", "/a", "").unwrap().get("port"), None);
    }

    #[gpui::test]
    fn the_part_turns_search_on_picks_exa_and_saves_the_key(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let state = Arc::new(Mutex::new(view_json(false, "searxng", false)));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let call = fake(state.clone(), sent.clone());
        let (view, vcx) = cx.add_window_view(|_, cx| {
            let mut v = build(call, cx);
            reload(&mut v, cx);
            v
        });
        vcx.run_until_parked();
        for tag in ["web-on", "web-provider-searxng", "web-provider-exa", "web-port", "web-folder", "web-save-searxng"] {
            assert!(vcx.debug_bounds(tag).is_some(), "{tag} is not drawn");
        }
        let on = vcx.debug_bounds("web-on").unwrap();
        vcx.simulate_click(on.center(), Modifiers::none());
        vcx.run_until_parked();
        view.read_with(vcx, |v, _| assert!(v.web.as_ref().unwrap().on));
        let exa = vcx.debug_bounds("web-provider-exa").unwrap();
        vcx.simulate_click(exa.center(), Modifiers::none());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("web-exa-key").is_some());
        assert!(vcx.debug_bounds("web-remove-key").is_none());
        vcx.update(|_, cx| view.update(cx, |v, cx| v.key.update(cx, |i, cx| i.set_value("my-own-key", cx))));
        let save = vcx.debug_bounds("web-save-key").unwrap();
        vcx.simulate_click(save.center(), Modifiers::none());
        vcx.run_until_parked();
        view.read_with(vcx, |v, cx| {
            assert!(v.web.as_ref().unwrap().exa_key);
            assert_eq!(v.key.read(cx).value(), "");
            assert_eq!(v.note.as_deref(), Some("Key saved in the keychain."));
        });
        assert!(vcx.debug_bounds("web-remove-key").is_some());
        let posts: Vec<Value> = sent.lock().unwrap().iter().filter_map(|(_, b)| b.clone()).collect();
        assert_eq!(posts, vec![json!({"type": "set_web", "on": true}), json!({"type": "set_web", "provider": "exa"}), json!({"type": "web_key", "key": "my-own-key"})]);
        vcx.update(|_, cx| assert!(apply(&view, &json!({"type": "web", "web": {"on": false, "provider": "exa", "exa_key": true, "searxng": {}}}), cx)));
        view.read_with(vcx, |v, _| assert!(!v.web.as_ref().unwrap().on));
    }
}
