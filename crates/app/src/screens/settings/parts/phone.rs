use crate::links::brain as link;
use crate::screens::phone_chip::chips::{self, ChipSpec};
use crate::screens::phone_chip::feed::{self, PhoneHub};
use crate::screens::phone_chip::info::{self, Camera, Door, Phone, PhoneState, clock};
use crate::screens::phone_chip::qr::qr_box;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, FontWeight, SharedString, Stateful,
    Subscription, Window, div, px,
};
use nixa_ui::theme::tint;
use nixa_ui::{Icon, Theme, theme};
use serde_json::Value;
use std::time::Duration;

pub const PART: &str = chips::PART;
pub const PATH: &str = "/phone";
pub const REFRESH: Duration = Duration::from_secs(3);
pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const PAIR_QR: f32 = 220.;
pub const ROOT_QR: f32 = 160.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    Loading,
    Up,
    NoDoor,
    Down(String),
}

pub struct PhoneView {
    pub addr: String,
    pub name: String,
    pub hub: Entity<PhoneHub>,
    pub reach: Reach,
    pub busy: bool,
    pub error: Option<String>,
    pub sent: Vec<Value>,
    pub offset_min: i32,
    loads: u64,
    ticking: bool,
    _watch: Subscription,
}

pub fn build(addr: String, cx: &mut Context<PhoneView>) -> PhoneView {
    let hub = feed::hub(cx);
    let watch = cx.observe(&hub, |v, hub, cx| {
        if hub.read(cx).known && v.reach == Reach::Loading {
            v.reach = Reach::Up;
        }
        cx.notify();
    });
    let known = hub.read(cx).known;
    PhoneView {
        addr,
        name: "Nixa".into(),
        hub,
        reach: if known { Reach::Up } else { Reach::Loading },
        busy: false,
        error: None,
        sent: Vec::new(),
        offset_min: if cfg!(test) {
            0
        } else {
            crate::screens::tabbar::local_offset_min()
        },
        loads: 0,
        ticking: false,
        _watch: watch,
    }
}

pub fn new(addr: String, cx: &mut App) -> Entity<PhoneView> {
    cx.new(|cx| {
        let mut v = build(addr, cx);
        reload(&mut v, cx);
        tick(&mut v, cx);
        v
    })
}

pub struct Shared(pub Entity<PhoneView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<PhoneView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<PhoneView> {
    if let Some(view) = shared(cx) {
        view.update(cx, |v, cx| {
            reload(v, cx);
            tick(v, cx);
            cx.notify();
        });
        return view;
    }
    let view = new(nixa_protocol::paths::brain_addr(), cx);
    cx.set_global(Shared(view.clone()));
    view
}

pub fn apply(view: &Entity<PhoneView>, msg: &Value, cx: &mut App) -> bool {
    if msg.get("type").and_then(Value::as_str) == Some("hello") {
        let name = msg
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !name.is_empty() {
            view.update(cx, |v, cx| {
                v.name = name;
                cx.notify();
            });
            return true;
        }
        return false;
    }
    feed::feed(msg, cx)
}

fn call(addr: &str, method: &str, body: Option<Value>) -> Result<Value, String> {
    link::http(addr, method, PATH, body.as_ref(), CALL_WAIT).map_err(|e| format!("{e:#}"))
}

pub fn take(v: &mut PhoneView, reply: &Value, cx: &mut Context<PhoneView>) {
    match info::parse_reply(reply) {
        Some(state) => {
            v.reach = if state.door.is_some() {
                Reach::Up
            } else {
                Reach::NoDoor
            };
            feed::set(state, cx);
        }
        None => v.reach = Reach::NoDoor,
    }
}

pub fn reload(v: &mut PhoneView, cx: &mut Context<PhoneView>) {
    if v.addr.is_empty() {
        return;
    }
    v.loads += 1;
    let load = v.loads;
    let addr = v.addr.clone();
    let task = cx.background_executor().spawn(async move {
        let phone = call(&addr, "GET", None)?;
        let health = link::http(&addr, "GET", "/health", None, CALL_WAIT).unwrap_or(Value::Null);
        Ok::<_, String>((phone, health))
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            if v.loads != load {
                return;
            }
            match result {
                Ok((phone, health)) => {
                    take(v, &phone, cx);
                    let name = health
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    if !name.is_empty() {
                        v.name = name;
                    }
                }
                Err(why) if why.contains("answered 404") => v.reach = Reach::NoDoor,
                Err(why) => v.reach = Reach::Down(why),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn tick(v: &mut PhoneView, cx: &mut Context<PhoneView>) {
    if v.ticking || v.addr.is_empty() {
        return;
    }
    v.ticking = true;
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(REFRESH).await;
            let alive = this.update(cx, |v, cx| {
                if !v.busy {
                    reload(v, cx);
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
}

pub fn send(v: &mut PhoneView, step: Value, cx: &mut Context<PhoneView>) {
    if v.busy {
        return;
    }
    v.sent.push(step.clone());
    if v.addr.is_empty() {
        return;
    }
    v.busy = true;
    v.loads += 1;
    let addr = v.addr.clone();
    let task = cx
        .background_executor()
        .spawn(async move { call(&addr, "POST", Some(step)) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            v.busy = false;
            match result {
                Ok(out) => {
                    take(v, &out, cx);
                    v.error = None;
                }
                Err(why) => v.error = Some(why),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

fn chip(t: &Theme, spec: &ChipSpec, busy: bool) -> Stateful<Div> {
    chips::chip(t, spec).when(busy, |el| el.opacity(0.5))
}

fn small(t: &Theme, words: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .text_color(t.dim)
        .child(words.into())
}

fn note(t: &Theme, words: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(t.dim)
        .child(words.into())
}

fn section(t: &Theme, title: String, words: &str) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.blue)
                .child(title),
        )
        .when(!words.is_empty(), |el| el.child(note(t, words.to_string())))
}

pub fn door_spec(door: &Door, t: &Theme) -> ChipSpec {
    if door.on {
        ChipSpec {
            id: "phone-door".into(),
            which: Icon::Check,
            label: "Phone door open".into(),
            tip: "Click to close the phone door. Paired phones stay paired.".into(),
            color: t.green,
        }
    } else {
        ChipSpec {
            id: "phone-door".into(),
            which: Icon::X,
            label: "Phone door closed".into(),
            tip: "Click to open the phone door on your Wi-Fi.".into(),
            color: t.dim,
        }
    }
}

pub fn voice_specs(state: &PhoneState, t: &Theme) -> [ChipSpec; 2] {
    let pick = |id: &str, place: &str, on: bool, about: &str| ChipSpec {
        id: id.to_string().into(),
        which: if on { Icon::Check } else { Icon::Square },
        label: format!("Voice on {place}"),
        tip: if on { format!("Voice plays on {place}. {about}") } else { format!("Play the voice on {place}. {about}") },
        color: if on { t.green } else { t.cyan },
    };
    [
        pick("phone-voice-mac", "the Mac", !state.mobile, "The phone stays silent."),
        pick("phone-voice-phone", "the phone", state.mobile, "The Mac stays silent."),
    ]
}

pub fn new_key_spec(t: &Theme) -> ChipSpec {
    ChipSpec {
        id: "phone-new-key".into(),
        which: Icon::Refresh,
        label: "New code".into(),
        tip: "Make a new pairing code. Every phone is signed out and must pair again.".into(),
        color: t.yellow,
    }
}

fn remove_spec(t: &Theme, id: String) -> ChipSpec {
    ChipSpec {
        id: id.into(),
        which: Icon::Trash,
        label: "Remove".into(),
        tip: "Remove this phone. It must pair again to come back.".into(),
        color: t.red,
    }
}

fn phone_row(v: &PhoneView, t: &Theme, phone: &Phone, cx: &mut Context<PhoneView>) -> AnyElement {
    let offset = v.offset_min;
    let words = info::row_note(phone, |at| clock(at, offset));
    let remove = remove_spec(t, format!("phone-remove-{}", phone.id));
    let id = phone.id.clone();
    let row_id = format!("phone-row-{}", phone.id);
    let busy = v.busy;
    div()
        .debug_selector(move || row_id.clone())
        .w_full()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(if phone.connected {
            tint(t.green, 0.47)
        } else {
            t.border
        })
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.))
                .items_center()
                .child(
                    div()
                        .size(px(8.))
                        .flex_none()
                        .rounded_full()
                        .bg(if phone.connected { t.green } else { t.dim }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(12.5)).child(phone.name.clone()))
                        .child(div().text_size(px(10.5)).text_color(t.dim).child(words)),
                )
                .child(chip(t, &remove, busy).on_click(
                    cx.listener(move |v, _: &ClickEvent, _, cx| send(v, info::remove(&id), cx)),
                )),
        )
        .into_any_element()
}

fn camera_section(v: &PhoneView, t: &Theme, camera: &Camera, cx: &mut Context<PhoneView>) -> AnyElement {
    let busy = v.busy;
    let offset = v.offset_min;
    let mut out = section(t, "Phone camera".into(), "Nixa can take a photo with the camera of a connected phone at any time and read it. The photo is deleted after the answer.")
        .debug_selector(|| "phone-camera".into());
    if !camera.model.is_empty() {
        out = out.child(small(t, format!("Vision model: {}", camera.model)).debug_selector(|| "phone-camera-model".into()));
    }
    if camera.privacy {
        out = out.child(note(t, "Privacy is on, so only a vision model on this Mac is used."));
    }
    if !camera.log.is_empty() {
        let clear = ChipSpec { id: "phone-camera-clear".into(), which: Icon::Trash, label: "Clear".into(), tip: "Delete the whole photo log now.".into(), color: t.red };
        out = out.child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.))
                .items_center()
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(format!("Photos, last 7 days ({})", camera.log.len())))
                .child(chip(t, &clear, busy).on_click(cx.listener(|v, _: &ClickEvent, _, cx| send(v, info::camera_log_clear(), cx)))),
        );
        for (n, look) in camera.log.iter().enumerate() {
            let id = format!("phone-camera-log-{n}");
            out = out.child(
                div()
                    .debug_selector(move || id.clone())
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .p(px(8.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(t.border)
                    .child(small(t, format!("{} · {} · {} camera", clock(look.at, offset), look.phone, look.camera)))
                    .child(div().text_size(px(12.)).child(look.question.clone()))
                    .child(div().text_size(px(12.)).text_color(t.dim).child(look.answer.clone()))
                    .when(!look.model.is_empty(), |el| el.child(small(t, look.model.clone()))),
            );
        }
    }
    out.into_any_element()
}

fn root_section(v: &PhoneView, t: &Theme, door: &Door) -> AnyElement {
    let mut words = format!(
        "Scan this on a new phone to download the {} certificate. On the phone, open Settings, search for \"CA certificate\" and install the file, so the app can talk to this Mac safely.",
        v.name
    );
    if door.root_sha256.is_some() {
        words.push_str(" Check that its SHA-256 fingerprint matches the one below.");
    }
    words.push_str(" Then pair the app with the code above.");
    section(
        t,
        "New phone? Install the certificate first (once)".into(),
        &words,
    )
    .when(!door.root_qr.is_empty(), |el| {
        el.child(qr_box("phone-root-qr", &door.root_qr, ROOT_QR))
    })
    .when_some(door.root_url.clone(), |el, url| el.child(small(t, url)))
    .when_some(
        door.root_sha256.as_deref().map(info::fingerprint),
        |el, lines| {
            el.child(
                div()
                    .debug_selector(|| "phone-root-sha".into())
                    .flex()
                    .flex_col()
                    .child(small(t, "SHA-256 fingerprint"))
                    .children(
                        lines
                            .into_iter()
                            .map(|line| small(t, line).font_family("Menlo")),
                    ),
            )
        },
    )
    .into_any_element()
}

fn door_body(
    v: &PhoneView,
    t: &Theme,
    state: &PhoneState,
    door: &Door,
    cx: &mut Context<PhoneView>,
) -> Vec<AnyElement> {
    let mut out: Vec<AnyElement> = Vec::new();
    let on = door.on;
    let mobile = state.mobile;
    let busy = v.busy;
    let mut top = div()
        .w_full()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(8.))
        .items_center()
        .child(chip(t, &door_spec(door, t), busy).on_click(
            cx.listener(move |v, _: &ClickEvent, _, cx| send(v, info::set_door(!on), cx)),
        ))
        .children(voice_specs(state, t).into_iter().enumerate().map(|(i, spec)| {
            let to_phone = i == 1;
            chip(t, &spec, busy).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                if to_phone != mobile {
                    send(v, info::set_voice(to_phone), cx);
                }
            }))
        }));
    if let Some(address) = &door.address {
        top = top.child(small(t, format!("Wi-Fi {address}")));
    }
    out.push(top.into_any_element());
    if let Some(error) = &door.error {
        out.push(
            div()
                .text_size(px(12.))
                .text_color(t.red)
                .child(error.clone())
                .into_any_element(),
        );
    }
    if !on {
        out.push(
            div()
                .text_size(px(12.))
                .text_color(t.dim)
                .child(format!("The phone door lets the {0} app on your Android phone, on the same Wi-Fi, show {0} and play its voice.", v.name))
                .into_any_element(),
        );
    }
    if on && door.address.is_some() {
        out.push(
            section(t, format!("Pair the {} app", v.name), &format!("Open the {} app on your Android phone and scan this code. The phone stays paired until you remove it.", v.name))
                .when(!door.pair_qr.is_empty(), |el| el.child(qr_box("phone-pair-qr", &door.pair_qr, PAIR_QR)))
                .child(div().flex().child(chip(t, &new_key_spec(t), busy).on_click(cx.listener(|v, _: &ClickEvent, _, cx| send(v, info::new_key(), cx)))))
                .into_any_element(),
        );
    }
    if on && (door.root_url.is_some() || !door.root_qr.is_empty()) {
        out.push(root_section(v, t, door));
    }
    if !door.phones.is_empty() {
        out.push(
            section(t, format!("Paired phones ({})", door.phones.len()), "").into_any_element(),
        );
        for phone in &door.phones {
            out.push(phone_row(v, t, phone, cx));
        }
    }
    if let Some(camera) = &state.camera {
        out.push(camera_section(v, t, camera, cx));
    }
    out
}

impl Render for PhoneView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let state = self.hub.read(cx).state.clone();
        let mut body = div()
            .id("phone-body")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .py(px(12.))
            .overflow_y_scroll();
        match (&state.door, &self.reach) {
            (Some(door), _) => {
                let door = door.clone();
                for el in door_body(self, &t, &state, &door, cx) {
                    body = body.child(el);
                }
            }
            (None, Reach::Down(why)) => {
                body = body.child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.red)
                        .child(format!("The brain is not reached: {why}")),
                );
            }
            (None, Reach::NoDoor) | (None, Reach::Up) => {
                body = body.child(note(
                    &t,
                    "This brain has no phone door yet. Restart the brain.",
                ));
            }
            (None, Reach::Loading) => {
                body = body.child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.dim)
                        .child("Connecting to the brain…"),
                );
            }
        }
        if let Some(error) = self.error.clone() {
            body = body.child(div().text_size(px(11.5)).text_color(t.red).child(error));
        }
        body = body.child(div().w_full().h(px(24.)).flex_none());
        div()
            .id("phone-part")
            .debug_selector(|| "phone-part".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(t.foreground)
            .child(body)
            .child(
                div()
                    .flex_none()
                    .py(px(10.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(note(&t, info::footer(&state, &self.name))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use serde_json::json;

    fn door(on: bool) -> Value {
        let code: Vec<String> = (0..5)
            .map(|y| {
                (0..5)
                    .map(|x| if (x * y) % 3 == 0 { '1' } else { '0' })
                    .collect()
            })
            .collect();
        json!({
            "ok": true,
            "mobile": false,
            "phone": {
                "on": on,
                "address": "192.168.1.20",
                "pair_qr": code,
                "root_url": "http://192.168.1.20:7431/nixa-root.crt",
                "root_sha256": "ab".repeat(32),
                "phones": [
                    {"id": "p1", "name": "Pixel", "pairedAt": 1000, "connected": true, "voice": false}
                ]
            }
        })
    }

    fn open(cx: &mut TestAppContext, reply: Value) -> (Entity<PhoneView>, &mut VisualTestContext) {
        cx.update(nixa_ui::init);
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut v = build(String::new(), cx);
            take(&mut v, &reply, cx);
            v
        });
        cx.run_until_parked();
        (view, cx)
    }

    fn bounds(cx: &mut VisualTestContext, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        cx.update(|window, app| window.draw(app).clear());
        cx.debug_bounds(Box::leak(selector.to_string().into_boxed_str()))
    }

    fn click(cx: &mut VisualTestContext, selector: &str) {
        let at = bounds(cx, selector).unwrap_or_else(|| panic!("{selector} is not on screen"));
        cx.simulate_click(at.center(), Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn an_open_door_shows_codes_and_phones(cx: &mut TestAppContext) {
        let (view, cx) = open(cx, door(true));
        for part in [
            "phone-door",
            "phone-voice-mac",
            "phone-voice-phone",
            "phone-pair-qr",
            "phone-root-qr",
            "phone-root-sha",
            "phone-new-key",
            "phone-row-p1",
            "phone-remove-p1",
        ] {
            assert!(bounds(cx, part).is_some(), "{part}");
        }
        let pair_qr = bounds(cx, "phone-pair-qr").unwrap();
        let root_qr = bounds(cx, "phone-root-qr").unwrap();
        assert!(pair_qr.size.width > root_qr.size.width);
        click(cx, "phone-door");
        click(cx, "phone-voice-mac");
        click(cx, "phone-voice-phone");
        click(cx, "phone-new-key");
        click(cx, "phone-remove-p1");
        let sent = view.read_with(cx, |v, _| v.sent.clone());
        assert_eq!(
            sent,
            vec![
                json!({"type": "set_phone", "on": false}),
                json!({"type": "set_mobile", "on": true}),
                json!({"type": "phone_new_key"}),
                json!({"type": "phone_remove", "id": "p1"}),
            ]
        );
    }

    #[gpui::test]
    fn a_closed_door_opens_on_click(cx: &mut TestAppContext) {
        let (view, cx) = open(cx, door(false));
        click(cx, "phone-door");
        let sent = view.read_with(cx, |v, _| v.sent.clone());
        assert_eq!(sent, vec![json!({"type": "set_phone", "on": true})]);
        let state = view.read_with(cx, |v, cx| v.hub.read(cx).state.clone());
        assert!(!state.door_on());
        assert_eq!(state.phones().len(), 1);
    }

    #[gpui::test]
    fn no_certificate_hides_its_section(cx: &mut TestAppContext) {
        let reply = json!({"ok": true, "mobile": false, "phone": {"on": true, "address": "192.168.1.20", "phones": []}});
        let (_view, cx) = open(cx, reply);
        assert!(bounds(cx, "phone-new-key").is_some());
        assert!(bounds(cx, "phone-root-qr").is_none());
        assert!(bounds(cx, "phone-root-sha").is_none());
    }

    #[gpui::test]
    fn brain_messages_reach_the_part(cx: &mut TestAppContext) {
        let (view, cx) = open(cx, json!({"ok": true}));
        assert_eq!(view.read_with(cx, |v, _| v.reach.clone()), Reach::NoDoor);
        assert!(bounds(cx, "phone-part").is_some());
        cx.update(|_, app| {
            assert!(apply(&view, &json!({"type": "hello", "name": "Nova"}), app));
            assert!(apply(&view, &json!({"type": "phone", "phone": {"on": true, "address": "10.0.0.2"}, "mobile": true}), app));
            assert!(!apply(&view, &json!({"type": "chat"}), app));
        });
        cx.run_until_parked();
        view.read_with(cx, |v, cx| {
            assert_eq!(v.name, "Nova");
            let state = v.hub.read(cx).state.clone();
            assert!(state.mobile && state.door_on());
        });
        assert!(bounds(cx, "phone-door").is_some());
        assert!(bounds(cx, "phone-new-key").is_some());
    }

    fn camera_reply(privacy: bool) -> Value {
        let mut reply = door(true);
        reply["camera"] = json!({
            "privacy": privacy, "model": "llava on Local Ollama",
            "log": [{"at": 5_000, "phone": "Pixel", "camera": "front", "question": "Is the child dancing?", "answer": "The child is dancing.", "model": "llava"}]
        });
        reply
    }

    #[gpui::test]
    fn the_camera_parts_show_and_send_their_commands(cx: &mut TestAppContext) {
        let (view, cx) = open(cx, camera_reply(false));
        for part in ["phone-camera", "phone-camera-model", "phone-camera-log-0", "phone-camera-clear"] {
            assert!(bounds(cx, part).is_some(), "{part}");
        }
        assert!(bounds(cx, "phone-camera-allow").is_none());
        click(cx, "phone-camera-clear");
        let sent = view.read_with(cx, |v, _| v.sent.clone());
        assert_eq!(sent, vec![json!({"type": "camera_log_clear"})]);
        let _ = camera_reply(true);
    }

    #[gpui::test]
    fn no_camera_object_hides_the_camera_parts(cx: &mut TestAppContext) {
        let (_view, cx) = open(cx, door(true));
        assert!(bounds(cx, "phone-camera").is_none());
        assert!(bounds(cx, "phone-door").is_some());
    }

    #[gpui::test]
    fn a_brain_error_shows_on_the_part(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut v = build(String::new(), cx);
            v.reach = Reach::Down("connection refused".into());
            v
        });
        cx.run_until_parked();
        assert!(bounds(cx, "phone-part").is_some());
        assert_eq!(
            view.read_with(cx, |v, _| v.reach.clone()),
            Reach::Down("connection refused".into())
        );
    }
}
