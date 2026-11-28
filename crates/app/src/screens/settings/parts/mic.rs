use crate::links::brain as link;
use gpui::prelude::*;
use gpui::{App, ClickEvent, Context, Entity, EventEmitter, Hsla, SharedString, Stateful, Window, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon, theme};
use serde_json::{Value, json};
use std::time::Duration;

pub const PART: &str = "Mic and speaker";
pub const REFRESH: Duration = Duration::from_secs(3);
pub const CALL_WAIT: Duration = Duration::from_secs(15);
pub const MIC_BASE: &str = "Mac default mic";
pub const SPEAKER_BASE: &str = "Mac sound output";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicEvent {
    OpenPart(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listen {
    Hotkey,
    Always,
}

impl Listen {
    pub const ALL: [Listen; 2] = [Listen::Hotkey, Listen::Always];

    pub fn word(self) -> &'static str {
        match self {
            Listen::Hotkey => "hotkey",
            Listen::Always => "always",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snap {
    pub mic_on: bool,
    pub voice_on: bool,
    pub listen: String,
    pub mic: String,
    pub speaker: String,
    pub listening: bool,
    pub error: Option<String>,
    pub key_access: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Devices {
    pub mics: Vec<String>,
    pub speakers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    Loading,
    Up,
    Down(String),
}

pub struct MicView {
    pub addr: String,
    pub name: String,
    pub snap: Option<Snap>,
    pub devices: Option<Devices>,
    pub reach: Reach,
    pub error: Option<String>,
    pub busy: bool,
    pub loads: u64,
    pub sent: Vec<Value>,
    pub ticking: bool,
}

impl EventEmitter<MicEvent> for MicView {}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn parse_snap(value: &Value) -> Option<Snap> {
    let view = value.get("voice").filter(|v| v.is_object()).unwrap_or(value);
    let settings = view.get("settings")?;
    let live = view.get("live").cloned().unwrap_or(Value::Null);
    Some(Snap {
        mic_on: flag(settings, "micOn"),
        voice_on: flag(settings, "voiceOn"),
        listen: Some(text(settings, "listen")).filter(|l| !l.is_empty()).unwrap_or_else(|| "hotkey".into()),
        mic: text(settings, "micDevice"),
        speaker: text(settings, "speakerDevice"),
        listening: flag(&live, "listening"),
        error: Some(text(&live, "error")).filter(|e| !e.is_empty()),
        key_access: flag(view, "keyAccess"),
    })
}

pub fn parse_devices(value: &Value) -> Option<Devices> {
    let list = |key: &str| -> Option<Vec<String>> {
        Some(value.get(key)?.as_array()?.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
    };
    Some(Devices { mics: list("mics")?, speakers: list("speakers")? })
}

pub fn pick_name(names: &[String], wanted: &str) -> Option<usize> {
    let want = wanted.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    names.iter().position(|n| n.to_lowercase() == want).or_else(|| names.iter().position(|n| n.to_lowercase().contains(&want)))
}

pub fn missing(names: &[String], chosen: &str) -> bool {
    !chosen.trim().is_empty() && !names.is_empty() && pick_name(names, chosen).is_none()
}

pub fn choices(names: &[String], chosen: &str, base: &str) -> Vec<Choice> {
    let chosen = chosen.trim();
    let mut out = vec![Choice { value: String::new(), label: base.to_string(), on: chosen.is_empty() }];
    for name in names {
        out.push(Choice { value: name.clone(), label: name.clone(), on: !chosen.is_empty() && name == chosen });
    }
    if !chosen.is_empty() && !names.iter().any(|n| n == chosen) {
        let label = if missing(names, chosen) { format!("{chosen} (not connected)") } else { chosen.to_string() };
        out.push(Choice { value: chosen.to_string(), label, on: true });
    }
    out
}

pub fn mic_note(name: &str, mic: &str, gone: bool) -> String {
    if mic.trim().is_empty() {
        format!("{name} listens with the Mac's default mic.")
    } else if gone {
        format!("{mic} is not connected. {name} listens with the Mac default mic until it is back.")
    } else {
        format!("{name} listens with {mic}.")
    }
}

pub fn speaker_note(name: &str, speaker: &str, gone: bool) -> String {
    if speaker.trim().is_empty() {
        format!("{name} and agent voices play on the Mac sound output, together with YouTube and other sound.")
    } else if gone {
        format!("{speaker} is not connected. Voices play on the Mac sound output until it is back.")
    } else {
        format!("{name} and agent voices play on {speaker}. YouTube and other Mac sound stay on the Mac sound output.")
    }
}

pub fn listen_now(snap: &Snap) -> Listen {
    match snap.listen.as_str() {
        "always" | "wake" => Listen::Always,
        _ => Listen::Hotkey,
    }
}

pub fn listen_steps(snap: &Snap, to: Listen) -> Vec<Value> {
    let mut out = Vec::new();
    if listen_now(snap) != to || snap.listen != to.word() {
        out.push(json!({"type": "set_listen", "mode": to.word()}));
    }
    if to == Listen::Always && !snap.mic_on {
        out.push(json!({"type": "set_mic", "on": true}));
    }
    out
}

pub fn listen_label(mode: Listen) -> String {
    match mode {
        Listen::Hotkey => "Hotkey (Right ⌘ or F9)".into(),
        Listen::Always => "Always on".into(),
    }
}

pub fn listen_tip(mode: Listen, name: &str) -> String {
    match mode {
        Listen::Hotkey => format!("Tap Right Command (or F9) to start, tap it again to send. {name} does not hear the room between presses."),
        Listen::Always => format!("{name} hears the mic all the time while the mic is on. The hotkey does nothing."),
    }
}

pub fn listen_note(mode: Listen, name: &str, key_access: bool) -> String {
    match mode {
        Listen::Always => format!("{name} hears every line from the mic while the mic is on and picks what is meant for it. The hotkey does nothing."),
        Listen::Hotkey => {
            let base = format!("{name} does not listen to the room. Tap Right Command (or F9) once, wait for the tink sound, then talk. Tap it again to send what you said. It works while Nixa is in the background, and even when the mic switch is off. On a Mac keyboard you may need to hold fn with F9.");
            if key_access { base } else { format!("{base} macOS has not allowed the key yet: turn on Input Monitoring for Nixa in System Settings, Privacy and Security.") }
        }
    }
}

pub const FOOTER: &str = "YouTube, music and other Mac sound use the Mac sound output. Change it in Control Center, Sound.";

pub fn build(addr: String) -> MicView {
    MicView { addr, name: "Nixa".into(), snap: None, devices: None, reach: Reach::Loading, error: None, busy: false, loads: 0, sent: Vec::new(), ticking: false }
}

pub fn new(addr: String, cx: &mut App) -> Entity<MicView> {
    cx.new(|cx| {
        let mut v = build(addr);
        reload(&mut v, true, cx);
        tick(&mut v, cx);
        v
    })
}

pub struct Shared(pub Entity<MicView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<MicView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<MicView> {
    if let Some(view) = shared(cx) {
        view.update(cx, |v, cx| {
            reload(v, true, cx);
            tick(v, cx);
            cx.notify();
        });
        return view;
    }
    let view = new(nixa_protocol::paths::brain_addr(), cx);
    cx.set_global(Shared(view.clone()));
    view
}

pub fn take_voice(v: &mut MicView, value: &Value) -> bool {
    match parse_snap(value) {
        Some(snap) => {
            v.snap = Some(snap);
            v.reach = Reach::Up;
            true
        }
        None => false,
    }
}

pub fn take_devices(v: &mut MicView, value: &Value) -> bool {
    match parse_devices(value) {
        Some(devices) => {
            v.devices = Some(devices);
            true
        }
        None => false,
    }
}

pub fn apply(view: &Entity<MicView>, msg: &Value, cx: &mut App) -> bool {
    let kind = msg.get("type").and_then(Value::as_str).unwrap_or_default();
    view.update(cx, |v, cx| {
        let used = match kind {
            "voice" => take_voice(v, msg),
            "audio" => take_devices(v, msg),
            "hello" => {
                let name = text(msg, "name");
                if !name.is_empty() {
                    v.name = name;
                }
                true
            }
            _ => false,
        };
        if used {
            cx.notify();
        }
        used
    })
}

fn call(addr: &str, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    link::http(addr, method, path, body.as_ref(), CALL_WAIT).map_err(|e| format!("{e:#}"))
}

pub fn reload(v: &mut MicView, devices_too: bool, cx: &mut Context<MicView>) {
    if v.addr.is_empty() {
        return;
    }
    v.loads += 1;
    let load = v.loads;
    let addr = v.addr.clone();
    let task = cx.background_executor().spawn(async move {
        let voice = call(&addr, "GET", "/voice", None)?;
        let health = call(&addr, "GET", "/health", None).unwrap_or(Value::Null);
        let devices = if devices_too { Some(call(&addr, "GET", "/voice/devices", None)?) } else { None };
        Ok::<_, String>((voice, health, devices))
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            if v.loads != load {
                return;
            }
            match result {
                Ok((voice, health, devices)) => {
                    take_voice(v, &voice);
                    let name = text(&health, "name");
                    if !name.is_empty() {
                        v.name = name;
                    }
                    if let Some(devices) = devices {
                        take_devices(v, &devices);
                    }
                }
                Err(why) => v.reach = Reach::Down(why),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn tick(v: &mut MicView, cx: &mut Context<MicView>) {
    if v.ticking || v.addr.is_empty() {
        return;
    }
    v.ticking = true;
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(REFRESH).await;
            let alive = this.update(cx, |v, cx| {
                if !v.busy {
                    reload(v, false, cx);
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
}

pub fn send(v: &mut MicView, steps: Vec<Value>, cx: &mut Context<MicView>) {
    if steps.is_empty() || v.busy {
        return;
    }
    v.sent.extend(steps.iter().cloned());
    if v.addr.is_empty() {
        return;
    }
    v.busy = true;
    v.loads += 1;
    let addr = v.addr.clone();
    let task = cx.background_executor().spawn(async move {
        let mut last = Value::Null;
        for step in steps {
            last = call(&addr, "POST", "/voice", Some(step))?;
        }
        Ok::<_, String>(last)
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            v.busy = false;
            match result {
                Ok(out) => {
                    take_voice(v, &out);
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

pub fn refresh(v: &mut MicView, cx: &mut Context<MicView>) {
    v.sent.push(json!({"type": "refresh_audio"}));
    reload(v, true, cx);
}

pub fn audio_step(key: &str, value: &str) -> Value {
    let mut step = serde_json::Map::new();
    step.insert("type".into(), json!("set_audio"));
    step.insert(key.into(), json!(value));
    Value::Object(step)
}

pub fn pick_audio(v: &mut MicView, key: &'static str, value: &str, cx: &mut Context<MicView>) {
    send(v, vec![audio_step(key, value)], cx);
}

pub fn pick_listen(v: &mut MicView, mode: Listen, cx: &mut Context<MicView>) {
    let Some(snap) = v.snap.clone() else { return };
    send(v, listen_steps(&snap, mode), cx);
}

fn chip(t: &Theme, id: String, which: Icon, label: String, color: Hsla, tip: String) -> Stateful<gpui::Div> {
    let hover = t.hover;
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .h(px(24.))
        .px(px(8.))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .text_size(px(11.5))
        .text_color(color)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(tip))
        .child(icon(which).size(px(13.)).text_color(color))
        .child(label)
}

fn title(t: &Theme, words: String) -> gpui::Div {
    div().text_size(px(11.)).text_color(t.blue).child(words)
}

fn note(t: &Theme, words: String) -> gpui::Div {
    div().text_size(px(11.5)).text_color(t.dim).child(words)
}

fn audio_row(v: &MicView, t: &Theme, key: &'static str, heading: String, list: Vec<Choice>, words: String, cx: &mut Context<MicView>) -> gpui::Div {
    let mut row = div().w_full().flex().flex_row().flex_wrap().gap(px(6.)).items_center();
    for (index, item) in list.into_iter().enumerate() {
        let (which, color) = if item.on { (Icon::Check, t.green) } else { (Icon::Square, t.cyan) };
        let tip = if item.on { format!("{} is chosen", item.label) } else { format!("Use {}", item.label) };
        let value = item.value.clone();
        let on = item.on;
        row = row.child(chip(t, format!("mic-{key}-{index}"), which, item.label, color, tip).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
            if !on {
                pick_audio(v, key, &value, cx);
            }
        })));
    }
    let _ = v;
    div().w_full().flex().flex_col().gap(px(6.)).child(title(t, heading)).child(row).child(note(t, words))
}

fn listen_row(v: &MicView, t: &Theme, snap: &Snap, cx: &mut Context<MicView>) -> gpui::Div {
    let now = listen_now(snap);
    let mut row = div().w_full().flex().flex_row().flex_wrap().gap(px(6.)).items_center();
    for mode in Listen::ALL {
        let on = mode == now;
        let (which, color) = if on { (Icon::Check, t.green) } else { (Icon::Square, t.cyan) };
        let label = listen_label(mode);
        let tip = if on { format!("{label} is chosen. {}", listen_tip(mode, &v.name)) } else { format!("Use {label}. {}", listen_tip(mode, &v.name)) };
        row = row.child(chip(t, format!("mic-listen-{}", mode.word()), which, label, color, tip).on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
            if !on {
                pick_listen(v, mode, cx);
            }
        })));
    }
    div().w_full().flex().flex_col().gap(px(6.)).child(title(t, "LISTENING".into())).child(row).child(note(t, listen_note(now, &v.name, snap.key_access)))
}

fn get_voice_models(cx: &mut App) {
    use crate::screens::first_run::{self, Action};

    first_run::host::run(Action::Open, cx);
    if let Some(view) = first_run::host::view(cx) {
        view.update(cx, |view, cx| {
            first_run::start_fetch(view, cx);
            cx.notify();
        });
    }
    if let Some(window) = super::super::workspace(cx) {
        window.update(cx, |_, window, _| window.activate_window()).ok();
    }
}

impl Render for MicView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let up = self.snap.is_some();
        let mut actions = div().w_full().flex().flex_row().gap(px(6.)).items_center().child(
            chip(&t, "mic-refresh".into(), Icon::Refresh, "Refresh".into(), t.cyan, "Look for mics and speakers again, for example after you connect a headset".into())
                .when(!up, |el| el.opacity(0.45))
                .on_click(cx.listener(|v, _: &ClickEvent, _, cx| refresh(v, cx))),
        );
        actions = actions.child(
            chip(&t, "mic-voices".into(), Icon::Volume, "Voices".into(), t.cyan, "Pick the voice for each kind of speech, space, agent and terminal".into())
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(MicEvent::OpenPart("Voices")))),
        );
        actions = actions.child(
            chip(&t, "mic-get-models".into(), Icon::Refresh, "Get voice models".into(), t.cyan, "Download missing speech and voice activity models".into())
                .on_click(|_, _, cx| get_voice_models(cx)),
        );
        let mut body = div().id("mic-body").flex_1().min_h_0().w_full().flex().flex_col().gap(px(16.)).py(px(12.)).overflow_y_scroll().child(actions);
        match (&self.reach, self.snap.clone()) {
            (Reach::Down(why), None) => {
                body = body.child(div().text_size(px(12.)).text_color(t.red).child(format!("The brain is not reached: {why}")));
            }
            (_, None) => {
                body = body.child(div().text_size(px(12.)).text_color(t.dim).child("Connecting to the brain…"));
            }
            (_, Some(snap)) => {
                let devices = self.devices.clone().unwrap_or_default();
                let mic_gone = missing(&devices.mics, &snap.mic);
                let speaker_gone = missing(&devices.speakers, &snap.speaker);
                let name = self.name.clone();
                body = body
                    .child(audio_row(self, &t, "mic", format!("MIC: {} LISTENS HERE", name.to_uppercase()), choices(&devices.mics, &snap.mic, MIC_BASE), mic_note(&name, &snap.mic, mic_gone), cx))
                    .child(audio_row(self, &t, "speaker", "SPEAKER: VOICES PLAY HERE".into(), choices(&devices.speakers, &snap.speaker, SPEAKER_BASE), speaker_note(&name, &snap.speaker, speaker_gone), cx))
                    .child(listen_row(self, &t, &snap, cx));
                if let Some(error) = snap.error.as_deref() {
                    let message = if error.contains("vad model missing:") {
                        "Voice activity detection needs a model. Select Get voice models above to download it.".to_owned()
                    } else {
                        format!("The mic did not start: {error}")
                    };
                    body = body.child(div().text_size(px(11.5)).text_color(t.red).child(message));
                }
            }
        }
        if let Some(error) = self.error.clone() {
            body = body.child(div().text_size(px(11.5)).text_color(t.red).child(error));
        }
        body = body.child(div().debug_selector(|| "mic-model-credit".into()).child(note(&t, crate::screens::first_run::voice::CREDIT.into())));
        body = body.child(div().w_full().h(px(24.)).flex_none());
        div()
            .id("mic-part")
            .debug_selector(|| "mic-part".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(t.foreground)
            .child(body)
            .child(div().flex_none().py(px(10.)).border_t_1().border_color(t.border).child(note(&t, FOOTER.into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn voice(mic_on: bool, listen: &str) -> Value {
        json!({"voice": {"settings": {"micOn": mic_on, "voiceOn": true, "listen": listen, "micDevice": "USB Mic", "speakerDevice": null}, "live": {"listening": mic_on, "error": null}, "keyAccess": false}})
    }

    #[test]
    fn device_choices_mark_the_chosen_and_the_missing_one() {
        let mics = names(&["MacBook Pro Microphone", "USB Mic"]);
        let list = choices(&mics, "USB Mic", MIC_BASE);
        assert_eq!(list.iter().map(|c| (c.label.as_str(), c.on)).collect::<Vec<_>>(), vec![(MIC_BASE, false), ("MacBook Pro Microphone", false), ("USB Mic", true)]);
        let gone = choices(&mics, "AirPods", MIC_BASE);
        assert_eq!(gone.last().map(|c| (c.label.as_str(), c.on, c.value.as_str())), Some(("AirPods (not connected)", true, "AirPods")));
        assert!(choices(&mics, "", MIC_BASE)[0].on);
        assert!(!missing(&mics, "usb"));
        assert!(missing(&mics, "AirPods"));
        assert!(!missing(&[], "AirPods"));
        assert_eq!(choices(&[], "AirPods", SPEAKER_BASE)[1].label, "AirPods");
        assert_eq!(mic_note("Nixa", "", false), "Nixa listens with the Mac's default mic.");
        assert!(mic_note("Nixa", "AirPods", true).starts_with("AirPods is not connected."));
        assert!(speaker_note("Nova", "Desk", false).starts_with("Nova and agent voices play on Desk."));
        assert_eq!(audio_step("speaker", ""), json!({"type": "set_audio", "speaker": ""}));
    }

    #[test]
    fn two_listen_choices_send_the_right_steps() {
        let fresh = parse_snap(&voice(false, "hotkey")).unwrap();
        assert_eq!(listen_now(&fresh), Listen::Hotkey);
        assert!(listen_steps(&fresh, Listen::Hotkey).is_empty());
        assert_eq!(listen_steps(&fresh, Listen::Always), vec![json!({"type": "set_listen", "mode": "always"}), json!({"type": "set_mic", "on": true})]);
        let always = parse_snap(&voice(true, "always")).unwrap();
        assert_eq!(listen_now(&always), Listen::Always);
        assert!(listen_steps(&always, Listen::Always).is_empty());
        assert_eq!(listen_steps(&always, Listen::Hotkey), vec![json!({"type": "set_listen", "mode": "hotkey"})]);
        let old = parse_snap(&voice(true, "wake")).unwrap();
        assert_eq!(listen_now(&old), Listen::Always);
        assert_eq!(listen_steps(&old, Listen::Always), vec![json!({"type": "set_listen", "mode": "always"})]);
        assert_eq!(listen_now(&parse_snap(&voice(false, "")).unwrap()), Listen::Hotkey);
        assert_eq!(listen_label(Listen::Hotkey), "Hotkey (Right ⌘ or F9)");
        assert_eq!(listen_label(Listen::Always), "Always on");
        assert!(listen_note(Listen::Hotkey, "Nixa", false).contains("Input Monitoring"));
        assert!(!listen_note(Listen::Hotkey, "Nixa", true).contains("Input Monitoring"));
        assert!(listen_note(Listen::Hotkey, "Nixa", true).contains("Tap it again to send"));
    }

    #[test]
    fn brain_answers_are_read() {
        let snap = parse_snap(&voice(true, "")).unwrap();
        assert_eq!((snap.listen.as_str(), snap.mic.as_str(), snap.speaker.as_str(), snap.key_access), ("hotkey", "USB Mic", "", false));
        assert!(parse_snap(&json!({"ok": true})).is_none());
        let devices = parse_devices(&json!({"ok": true, "mics": ["A", " ", 3], "speakers": ["B"]})).unwrap();
        assert_eq!(devices, Devices { mics: names(&["A"]), speakers: names(&["B"]) });
        assert!(parse_devices(&json!({"mics": []})).is_none());
    }

    fn open(cx: &mut TestAppContext) -> (Entity<MicView>, &mut VisualTestContext, Rc<RefCell<Vec<MicEvent>>>) {
        cx.update(nixa_ui::init);
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut v = build(String::new());
            take_voice(&mut v, &voice(false, "hotkey"));
            take_devices(&mut v, &json!({"mics": ["USB Mic"], "speakers": ["Desk Speakers"]}));
            v
        });
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        cx.update(|_, app| {
            app.subscribe(&view, move |_, e: &MicEvent, _| sink.borrow_mut().push(e.clone())).detach();
        });
        cx.run_until_parked();
        (view, cx, seen)
    }

    fn click(cx: &mut VisualTestContext, selector: &str) {
        cx.update(|window, app| window.draw(app).clear());
        let bounds = cx.debug_bounds(Box::leak(selector.to_string().into_boxed_str())).unwrap_or_else(|| panic!("{selector} is not on screen"));
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn the_speech_model_credit_is_shown(cx: &mut TestAppContext) {
        let (_, cx, _) = open(cx);
        cx.update(|window, app| window.draw(app).clear());
        assert!(cx.debug_bounds("mic-model-credit").is_some());
        assert!(crate::screens::first_run::voice::CREDIT.contains("Parakeet TDT 0.6B v3 by NVIDIA, CC BY 4.0 (creativecommons.org/licenses/by/4.0)"));
    }

    #[gpui::test]
    fn clicks_send_the_voice_messages(cx: &mut TestAppContext) {
        let (view, cx, seen) = open(cx);
        click(cx, "mic-listen-always");
        click(cx, "mic-speaker-1");
        click(cx, "mic-mic-1");
        click(cx, "mic-mic-0");
        click(cx, "mic-refresh");
        click(cx, "mic-voices");
        let sent = view.read_with(cx, |v, _| v.sent.clone());
        assert_eq!(
            sent,
            vec![
                json!({"type": "set_listen", "mode": "always"}),
                json!({"type": "set_mic", "on": true}),
                json!({"type": "set_audio", "speaker": "Desk Speakers"}),
                json!({"type": "set_audio", "mic": ""}),
                json!({"type": "refresh_audio"}),
            ]
        );
        assert_eq!(*seen.borrow(), vec![MicEvent::OpenPart("Voices")]);
        let mut changed = voice(true, "always");
        changed["type"] = json!("voice");
        cx.update(|_, app| {
            assert!(apply(&view, &changed, app));
            assert!(apply(&view, &json!({"type": "audio", "mics": ["New Mic"], "speakers": []}), app));
            assert!(apply(&view, &json!({"type": "hello", "name": "Nova"}), app));
            assert!(!apply(&view, &json!({"type": "chat"}), app));
        });
        view.read_with(cx, |v, _| {
            assert_eq!(v.snap.as_ref().map(listen_now), Some(Listen::Always));
            assert_eq!(v.devices.as_ref().map(|d| d.mics.clone()), Some(names(&["New Mic"])));
            assert_eq!(v.name, "Nova");
        });
    }
}
