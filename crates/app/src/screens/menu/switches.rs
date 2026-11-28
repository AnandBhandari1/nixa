use anyhow::Result;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FontWeight, SharedString, Window, div, px,
};
use nixa_store::Connection;
use nixa_ui::popover::tooltip;
use nixa_ui::toggle::toggle;
use nixa_ui::{Icon, Theme, icon, theme};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const OFF_FILE: &str = "off";
pub const HEADPHONES_FILE: &str = "headphones";
pub const AGENT_VOICE_KEY: &str = "voice.agents";
pub const GIT_VOICE_KEY: &str = "git.speak";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Nixa,
    Phones,
    AgentVoice,
    GitVoice,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub nixa: Option<bool>,
    pub phones: Option<bool>,
    pub agent_voice: Option<bool>,
    pub git_voice: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snap {
    pub linked: bool,
    pub state: String,
    pub waiting: usize,
    pub privacy: Option<bool>,
}


fn setting(store: Option<&Connection>, key: &str) -> Option<bool> {
    let conn = store?;
    nixa_store::settings::setting_bool(conn, key, true).ok()
}

pub fn load(data: &Path, store: Option<&Connection>) -> State {
    State {
        nixa: Some(!data.join(OFF_FILE).exists()),
        phones: Some(data.join(HEADPHONES_FILE).exists()),
        agent_voice: setting(store, AGENT_VOICE_KEY),
        git_voice: setting(store, GIT_VOICE_KEY),
    }
}

pub fn get(state: &State, switch: Switch) -> Option<bool> {
    match switch {
        Switch::Nixa => state.nixa,
        Switch::Phones => state.phones,
        Switch::AgentVoice => state.agent_voice,
        Switch::GitVoice => state.git_voice,
    }
}

fn flag(path: &Path, on: bool) -> Result<()> {
    if on {
        if let Some(dir) = path.parent() {
            nixa_protocol::paths::ensure_private_dir(dir)?;
        }
        std::fs::write(path, "")?;
    } else if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub fn set(switch: Switch, on: bool, data: &Path, store: Option<&Connection>) -> Result<()> {
    let need = || anyhow::anyhow!("The store is not open, so this switch cannot be saved.");
    match switch {
        Switch::Nixa => flag(&data.join(OFF_FILE), !on),
        Switch::Phones => flag(&data.join(HEADPHONES_FILE), on),
        Switch::AgentVoice => {
            nixa_store::settings::set_setting_bool(store.ok_or_else(need)?, AGENT_VOICE_KEY, on)
        }
        Switch::GitVoice => {
            nixa_store::settings::set_setting_bool(store.ok_or_else(need)?, GIT_VOICE_KEY, on)
        }
    }
}

pub fn tell(switch: Switch, on: bool) -> Value {
    let kind = match switch {
        Switch::Nixa => "set_nixa",
        Switch::Phones => "set_headphones",
        Switch::AgentVoice => "set_agent_voice",
        Switch::GitVoice => "set_git_voice",
    };
    json!({"type": kind, "on": on})
}

pub fn apply(snap: &mut Snap, msg: &Value) -> bool {
    let kind = msg.get("type").and_then(Value::as_str).unwrap_or_default();
    let text = |key: &str| msg.get(key).and_then(Value::as_str).map(str::to_string);
    match kind {
        "hello" => {
            snap.linked = true;
            true
        }
        "state" => {
            if let Some(state) = text("state") {
                snap.state = state;
            }
            if let Some(on) = msg.get("privacy").and_then(Value::as_bool) {
                snap.privacy = Some(on);
            }
            true
        }
        "prompts" => match crate::screens::question::card::snapshot_of(msg) {
            Some(prompts) => {
                snap.waiting = usize::from(prompts.card.is_some()) + prompts.waiting;
                true
            }
            None => false,
        },
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    Switch(Switch, Option<bool>),
    NixaScreen,
    ShowCard,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: &'static str,
    pub icon: Icon,
    pub label: String,
    pub hint: String,
    pub tip: String,
    pub kind: RowKind,
}

fn word(on: Option<bool>) -> &'static str {
    match on {
        None => "…",
        Some(true) => "on",
        Some(false) => "off",
    }
}

fn switch_row(
    id: &'static str,
    switch: Switch,
    on: Option<bool>,
    label: &str,
    hint: &str,
    tip: String,
) -> Row {
    Row {
        id,
        icon: if on == Some(false) {
            Icon::Minus
        } else {
            Icon::Check
        },
        label: format!("{label}: {}", word(on)),
        hint: hint.to_string(),
        tip,
        kind: RowKind::Switch(switch, on),
    }
}

pub fn rows(state: &State, snap: &Snap, name: &str) -> Vec<Row> {
    let nixa_word = match state.nixa {
        None => "…".to_string(),
        Some(false) => "off".to_string(),
        Some(true) if !snap.linked => "on, starting".to_string(),
        Some(true) if snap.state.is_empty() => "on".to_string(),
        Some(true) => format!("on, {}", snap.state),
    };
    let waiting = if state.nixa == Some(true) {
        snap.waiting
    } else {
        0
    };
    let waiting_word = if waiting > 0 {
        format!(", {waiting} waiting")
    } else {
        String::new()
    };
    let mut list = vec![
        Row {
            id: "switch-nixa",
            icon: if state.nixa == Some(false) { Icon::Minus } else { Icon::Check },
            label: format!("{name}: {nixa_word}{waiting_word}"),
            hint: "voice questions".into(),
            tip: format!("Turn {name} on or off. {name} reads agent questions aloud, answers your requests and controls the app by voice. Off: no mic and no {name} voice. Agent voice is a separate switch and is not changed."),
            kind: RowKind::Switch(Switch::Nixa, state.nixa),
        },
        switch_row(
            "switch-phones",
            Switch::Phones,
            state.phones,
            "Headphones mode",
            "nixa",
            format!("Use with headphones. The mic stays open while {name} talks, so you can talk over it or say: stop. Keep it off with speakers, or {name} hears itself."),
        ),
        switch_row(
            "switch-agent-voice",
            Switch::AgentVoice,
            state.agent_voice,
            "Agent voice",
            "speak",
            format!("Read agent replies and alerts aloud, or not. {name} is a separate switch and is not changed."),
        ),
        switch_row(
            "switch-git-voice",
            Switch::GitVoice,
            state.git_voice,
            "Git voice",
            "git",
            "Speak git results, failures and prompts, or keep them as popups only. The Agent voice switch still turns this speech off.".into(),
        ),
        Row {
            id: "switch-nixa-screen",
            icon: Icon::Layers,
            label: format!("{name} screen"),
            hint: "⌥⇧M".into(),
            tip: format!("Show {name}, the deep brain and the agents as a live map over the whole window. Esc, × or Option+Shift+M closes it."),
            kind: RowKind::NixaScreen,
        },
    ];
    if waiting > 0 {
        list.push(Row {
            id: "switch-card",
            icon: Icon::Layers,
            label: format!("Show {name} card"),
            hint: format!("{waiting} waiting"),
            tip: "Show the card with the agent question that is waiting".into(),
            kind: RowKind::ShowCard,
        });
    }
    list
}

#[derive(Clone, Debug, PartialEq)]
pub struct Privacy {
    pub on: Option<bool>,
    pub label: &'static str,
    pub note: String,
    pub tip: &'static str,
    pub command: Option<Value>,
}

pub fn privacy(snap: &Snap, name: &str) -> Privacy {
    match snap.privacy {
        None => Privacy {
            on: None,
            label: "Private and confidential: …",
            note: format!("{name} has not said yet if privacy is on."),
            tip: "The switch works after it does.",
            command: None,
        },
        Some(true) => Privacy {
            on: Some(true),
            label: "Private and confidential: on",
            note: format!(
                "{name} does not answer questions about your private things or this computer."
            ),
            tip: "Click to turn it off.",
            command: Some(json!({"type": "set_privacy", "on": false})),
        },
        Some(false) => Privacy {
            on: Some(false),
            label: "Private and confidential: off",
            note: format!("{name} answers questions about you and this computer as normal."),
            tip: "Click to turn it on again.",
            command: Some(json!({"type": "set_privacy", "on": true})),
        },
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SwitchEvent {
    Send(Value),
    NixaScreen,
    ShowCard,
}

pub struct SwitchesView {
    pub state: State,
    pub snap: Snap,
    pub error: Option<String>,
    pub name: String,
    pub data: PathBuf,
    pub store: Option<Connection>,
}

impl EventEmitter<SwitchEvent> for SwitchesView {}

pub struct Shared(pub Entity<SwitchesView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<SwitchesView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn build(data: PathBuf, store: Option<Connection>) -> SwitchesView {
    SwitchesView {
        state: load(&data, store.as_ref()),
        snap: Snap::default(),
        error: None,
        name: "Nixa".into(),
        data,
        store,
    }
}

pub fn new(data: PathBuf, store: Option<Connection>, cx: &mut App) -> Entity<SwitchesView> {
    cx.new(|_| build(data, store))
}

pub fn sync_agent_voice(view: &mut SwitchesView, msg: &Value) -> bool {
    if msg.get("type").and_then(Value::as_str) != Some("agent_voice") {
        return false;
    }
    let Some(on) = msg.get("on").and_then(Value::as_bool) else {
        return false;
    };
    if let Some(store) = view.store.as_ref() {
        nixa_store::settings::set_setting_bool(store, AGENT_VOICE_KEY, on).ok();
    }
    view.state.agent_voice = Some(on);
    true
}

pub fn feed(view: &Entity<SwitchesView>, msg: &Value, cx: &mut App) {
    view.update(cx, |v, cx| {
        if sync_agent_voice(v, msg) {
            cx.notify();
        }
        if msg.get("type").and_then(Value::as_str) == Some("hello")
            && let Some(name) = msg
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.is_empty())
        {
            v.name = name.to_string();
        }
        if apply(&mut v.snap, msg) {
            cx.notify();
        }
    });
}

pub fn set_linked(view: &Entity<SwitchesView>, linked: bool, cx: &mut App) {
    view.update(cx, |v, cx| {
        if v.snap.linked != linked {
            v.snap.linked = linked;
            cx.notify();
        }
    });
}

pub fn set_error(view: &Entity<SwitchesView>, error: Option<String>, cx: &mut App) {
    view.update(cx, |v, cx| {
        v.error = error;
        cx.notify();
    });
}

pub fn flip(view: &mut SwitchesView, switch: Switch) -> Option<bool> {
    let next = !get(&view.state, switch).unwrap_or(true);
    match set(switch, next, &view.data, view.store.as_ref()) {
        Ok(()) => {
            view.error = None;
            match switch {
                Switch::Nixa => view.state.nixa = Some(next),
                Switch::Phones => view.state.phones = Some(next),
                Switch::AgentVoice => view.state.agent_voice = Some(next),
                Switch::GitVoice => view.state.git_voice = Some(next),
            }
            Some(next)
        }
        Err(e) => {
            view.error = Some(format!("Could not change the switch: {e:#}"));
            None
        }
    }
}

fn pick(view: &mut SwitchesView, kind: &RowKind, cx: &mut Context<SwitchesView>) {
    match kind {
        RowKind::Switch(switch, _) => {
            if let Some(on) = flip(view, *switch) {
                if view.snap.linked {
                    cx.emit(SwitchEvent::Send(tell(*switch, on)));
                } else {
                    view.error = Some(format!(
                        "Saved here. {} is not connected, so it was not told. Change it again after it starts.",
                        view.name
                    ));
                }
            }
        }
        RowKind::NixaScreen => cx.emit(SwitchEvent::NixaScreen),
        RowKind::ShowCard => cx.emit(SwitchEvent::ShowCard),
    }
    cx.notify();
}

fn send(view: &mut SwitchesView, command: Value, cx: &mut Context<SwitchesView>) {
    if !view.snap.linked {
        view.error = Some(format!(
            "{} is not connected. Change it again after it starts.",
            view.name
        ));
    } else {
        view.error = None;
        cx.emit(SwitchEvent::Send(command));
    }
    cx.notify();
}

fn row_element(t: &Theme, row: &Row, cx: &mut Context<SwitchesView>) -> AnyElement {
    let hover = t.hover;
    let kind = row.kind.clone();
    let id = SharedString::from(row.id);
    let selector = row.id;
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .h(px(34.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(7.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(row.tip.clone()))
        .on_click(cx.listener(move |view, _, _, cx| pick(view, &kind, cx)))
        .child(icon(row.icon).size(px(14.)).text_color(t.muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.foreground)
                .child(SharedString::from(row.label.clone())),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(t.dim)
                .child(SharedString::from(row.hint.clone())),
        )
        .when_some(
            match row.kind {
                RowKind::Switch(_, on) => Some(on.unwrap_or(false)),
                _ => None,
            },
            |el, on| {
                el.child(toggle(
                    t,
                    SharedString::from(format!("{}-toggle", row.id)),
                    on,
                ))
            },
        )
        .into_any_element()
}

fn action_button(
    t: &Theme,
    id: SharedString,
    label: String,
    selected: bool,
    command: Option<Value>,
    cx: &mut Context<SwitchesView>,
) -> AnyElement {
    let live = command.is_some();
    let hover = t.hover;
    let selector = id.to_string();
    div()
        .id(id)
        .debug_selector(move || selector)
        .flex_none()
        .px(px(9.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(if selected { t.green } else { t.border })
        .text_size(px(12.))
        .text_color(if !live {
            t.dim
        } else if selected {
            t.green
        } else {
            t.foreground
        })
        .when(live, |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        .when_some(command, |el, command| {
            el.on_click(cx.listener(move |view, _, _, cx| send(view, command.clone(), cx)))
        })
        .child(SharedString::from(label))
        .into_any_element()
}

fn section(t: &Theme, title: String, body: impl IntoElement) -> AnyElement {
    div()
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(8.))
        .py(px(10.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.blue)
                .child(SharedString::from(title)),
        )
        .child(body)
        .into_any_element()
}

fn note(t: &Theme, text: String) -> AnyElement {
    div()
        .text_size(px(11.5))
        .text_color(t.dim)
        .child(SharedString::from(text))
        .into_any_element()
}

pub fn controls(view: &SwitchesView, t: &Theme, cx: &mut Context<SwitchesView>) -> Vec<AnyElement> {
    let name = view.name.clone();
    if !view.snap.linked {
        return vec![note(
            t,
            format!(
                "Connect {name} to change privacy. The on or off switch above is always available."
            ),
        )];
    }
    let p = privacy(&view.snap, &name);
    vec![
        section(
            t,
            "Privacy".into(),
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().flex().child(action_button(
                    t,
                    "switch-privacy".into(),
                    p.label.to_string(),
                    p.on == Some(true),
                    p.command.clone(),
                    cx,
                )))
                .child(note(t, format!("{} {}", p.note, p.tip))),
        ),
    ]
}

impl Render for SwitchesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let rows: Vec<AnyElement> = rows(&self.state, &self.snap, &self.name)
            .iter()
            .map(|row| row_element(&t, row, cx))
            .collect();
        let controls = controls(self, &t, cx);
        div()
            .id("switches")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.8))
            .children(rows)
            .children(controls)
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .id("switches-error")
                        .debug_selector(|| "switches-error".into())
                        .text_size(px(11.))
                        .text_color(t.red)
                        .child(SharedString::from(error)),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mms-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn switches_live_in_flag_files_and_the_store() {
        let data = temp("flags");
        let store = nixa_store::open_memory().unwrap();
        let first = load(&data, Some(&store));
        assert_eq!(
            first,
            State {
                nixa: Some(true),
                phones: Some(false),
                agent_voice: Some(true),
                git_voice: Some(true)
            }
        );
        set(Switch::Nixa, false, &data, Some(&store)).unwrap();
        set(Switch::Phones, true, &data, Some(&store)).unwrap();
        set(Switch::AgentVoice, false, &data, Some(&store)).unwrap();
        set(Switch::GitVoice, false, &data, Some(&store)).unwrap();
        assert!(data.join(OFF_FILE).exists());
        assert!(data.join(HEADPHONES_FILE).exists());
        assert_eq!(
            load(&data, Some(&store)),
            State {
                nixa: Some(false),
                phones: Some(true),
                agent_voice: Some(false),
                git_voice: Some(false)
            }
        );
        set(Switch::Nixa, true, &data, Some(&store)).unwrap();
        set(Switch::Nixa, true, &data, Some(&store)).unwrap();
        assert!(!data.join(OFF_FILE).exists());
        assert_eq!(load(&data, None).agent_voice, None);
        assert!(set(Switch::GitVoice, true, &data, None).is_err());
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn rows_say_the_state_like_today() {
        let state = State {
            nixa: Some(true),
            phones: Some(false),
            agent_voice: None,
            git_voice: Some(true),
        };
        let mut snap = Snap::default();
        let list = rows(&state, &snap, "Nixa");
        let labels: Vec<&str> = list.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Nixa: on, starting",
                "Headphones mode: off",
                "Agent voice: …",
                "Git voice: on",
                "Nixa screen"
            ]
        );
        assert_eq!(list[1].icon, Icon::Minus);
        snap.linked = true;
        snap.state = "listening".into();
        snap.waiting = 2;
        let list = rows(&state, &snap, "Nova");
        assert_eq!(list[0].label, "Nova: on, listening, 2 waiting");
        snap.state.clear();
        assert_eq!(rows(&state, &snap, "Nixa")[0].label, "Nixa: on, 2 waiting");
        assert_eq!(list.last().unwrap().label, "Show Nova card");
        assert_eq!(list.last().unwrap().hint, "2 waiting");
        let off = State {
            nixa: Some(false),
            ..state
        };
        let list = rows(&off, &snap, "Nixa");
        assert_eq!(list[0].label, "Nixa: off");
        assert_eq!(list.len(), 5);
    }

    #[test]
    fn brain_messages_fill_the_snapshot() {
        let mut snap = Snap::default();
        assert!(apply(&mut snap, &json!({"type": "hello"})));
        assert!(apply(
            &mut snap,
            &json!({"type": "state", "state": "thinking", "privacy": false})
        ));
        assert!(!apply(&mut snap, &json!({"type": "work", "work": {"hold": null}})));
        assert!(!apply(&mut snap, &json!({"type": "chat"})));
        assert_eq!(
            (snap.state.as_str(), snap.privacy),
            ("thinking", Some(false))
        );
        let p = privacy(&snap, "Nixa");
        assert_eq!(p.command, Some(json!({"type": "set_privacy", "on": true})));
        assert_eq!(p.label, "Private and confidential: off");
    }

    #[test]
    fn privacy_is_unknown_until_the_brain_says_it() {
        let mut snap = Snap::default();
        apply(&mut snap, &json!({"type": "hello"}));
        apply(&mut snap, &json!({"type": "state", "state": "listening"}));
        let p = privacy(&snap, "Nixa");
        assert_eq!((p.on, p.command), (None, None));
        assert_eq!(p.label, "Private and confidential: …");
        apply(&mut snap, &json!({"type": "state", "privacy": true}));
        let p = privacy(&snap, "Nixa");
        assert_eq!(p.on, Some(true));
        assert_eq!(p.command, Some(json!({"type": "set_privacy", "on": false})));
    }

    #[test]
    fn each_switch_has_a_brain_message() {
        assert_eq!(
            tell(Switch::AgentVoice, false),
            json!({"type": "set_agent_voice", "on": false})
        );
        assert_eq!(
            tell(Switch::Nixa, true),
            json!({"type": "set_nixa", "on": true})
        );
        assert_eq!(
            tell(Switch::Phones, true),
            json!({"type": "set_headphones", "on": true})
        );
        assert_eq!(
            tell(Switch::GitVoice, false),
            json!({"type": "set_git_voice", "on": false})
        );
        let data = temp("sync");
        let mut view = build(data.clone(), Some(nixa_store::open_memory().unwrap()));
        assert!(sync_agent_voice(
            &mut view,
            &json!({"type": "agent_voice", "on": false})
        ));
        assert!(!sync_agent_voice(
            &mut view,
            &json!({"type": "agent_voice"})
        ));
        assert_eq!(view.state.agent_voice, Some(false));
        assert_eq!(load(&data, view.store.as_ref()).agent_voice, Some(false));
        std::fs::remove_dir_all(&data).ok();
    }
}
