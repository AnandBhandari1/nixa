use serde::Deserialize;
use serde_json::Value;

pub const KEEP_ITEMS: usize = 400;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Item {
    pub id: u64,
    #[serde(default)]
    pub who: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub at: u64,
    #[serde(default)]
    pub typed: bool,
    #[serde(default)]
    pub quiet: bool,
    #[serde(default)]
    pub routine: bool,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub meta: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Link {
    #[default]
    Starting,
    Off,
    Up,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Topic {
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub topic: Option<Topic>,
    pub hold: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub tokens: u64,
    pub cached: u64,
    pub turns: u64,
    pub last: Option<(u64, u64)>,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub context: Option<u8>,
}

pub fn context_pct(v: &Value) -> Option<u8> {
    v.get("context_pct")
        .and_then(Value::as_f64)
        .filter(|p| p.is_finite())
        .map(|p| p.clamp(0.0, 100.0).round() as u8)
}

pub fn is_turn_usage(v: &Value) -> bool {
    v.get("input").is_some() || v.get("output").is_some() || v.get("model").is_some()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Confirm {
    pub id: String,
    pub text: String,
    pub why: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Brain {
    pub name: String,
    pub kind: String,
    pub model: String,
    pub model_name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub link: Link,
    pub name: String,
    pub items: Vec<Item>,
    pub busy: bool,
    pub draft: String,
    pub state: String,
    pub progress: String,
    pub mic_off: bool,
    pub voice_off: bool,
    pub mic_ready: Option<bool>,
    pub hearing: bool,
    pub listen: Option<String>,
    pub wake_active: bool,
    pub wake_until: u64,
    pub key_listen: String,
    pub echo_cancel: bool,
    pub brain_ready: Option<bool>,
    pub brain: Option<Brain>,
    pub work: Work,
    pub usage: Option<Usage>,
    pub confirm: Option<Confirm>,
}

impl Snapshot {
    pub fn name(&self) -> &str {
        if self.name.trim().is_empty() {
            "Nixa"
        } else {
            self.name.trim()
        }
    }

    pub fn off(&self) -> bool {
        self.state == "off"
    }
}

fn pick<'a>(v: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names
        .iter()
        .find_map(|n| v.get(*n))
        .filter(|x| !x.is_null())
}

fn flag(v: &Value, names: &[&str]) -> Option<bool> {
    pick(v, names).and_then(Value::as_bool)
}

fn text(v: &Value, names: &[&str]) -> Option<String> {
    pick(v, names).and_then(Value::as_str).map(str::to_string)
}

fn number(v: &Value, names: &[&str]) -> u64 {
    pick(v, names)
        .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64)))
        .unwrap_or(0)
}

fn items_of(v: &Value) -> Vec<Item> {
    v.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|i| serde_json::from_value(i.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn add_item(snap: &mut Snapshot, item: Item) {
    if item.who == "nixa" {
        snap.draft.clear();
    }
    match snap.items.iter().position(|i| i.id == item.id) {
        Some(at) => snap.items[at] = item,
        None => {
            let at = snap.items.partition_point(|i| i.id < item.id);
            snap.items.insert(at, item);
        }
    }
    if snap.items.len() > KEEP_ITEMS {
        let cut = snap.items.len() - KEEP_ITEMS;
        snap.items.drain(..cut);
    }
}

fn set_brain(snap: &mut Snapshot, v: Option<&Value>) {
    snap.brain = v.filter(|b| b.is_object()).map(|b| Brain {
        name: text(b, &["name"]).unwrap_or_default(),
        kind: text(b, &["kind"]).unwrap_or_default(),
        model: text(b, &["model"]).unwrap_or_default(),
        model_name: text(b, &["model_name", "modelName"]).unwrap_or_default(),
    });
}

fn set_voice(snap: &mut Snapshot, v: &Value) {
    if let Some(state) = text(v, &["state"]) {
        snap.state = state;
    }
    if let Some(p) = text(v, &["progress"]) {
        snap.progress = p;
    }
    if let Some(x) = flag(v, &["mic_off", "micOff"]) {
        snap.mic_off = x;
    }
    if let Some(x) = flag(v, &["voice_off", "voiceOff"]) {
        snap.voice_off = x;
    }
    if let Some(x) = flag(v, &["mic_ready", "micReady"]) {
        snap.mic_ready = Some(x);
    }
    if let Some(x) = flag(v, &["hearing", "talking"]) {
        snap.hearing = x;
    }
    if let Some(x) = text(v, &["listen"]) {
        snap.listen = Some(x);
    }
    if let Some(x) = flag(v, &["wake_active", "wakeActive"]) {
        snap.wake_active = x;
    }
    if let Some(x) = text(v, &["key_listen", "keyListen"]) {
        snap.key_listen = x;
    }
    if let Some(x) = flag(v, &["echo_cancel", "echoCancel", "headphones"]) {
        snap.echo_cancel = x;
    }
    if let Some(x) = flag(v, &["brain_ready", "brainReady"]) {
        snap.brain_ready = Some(x);
    }
}

fn set_speaking(snap: &mut Snapshot, on: bool) {
    if on {
        snap.state = "speaking".into();
    } else if snap.state == "speaking" {
        snap.state.clear();
    }
}

fn set_voice_view(snap: &mut Snapshot, view: &Value) {
    if let Some(s) = view.get("settings").filter(|s| s.is_object()) {
        if let Some(on) = flag(s, &["mic_on", "micOn"]) {
            snap.mic_off = !on;
        }
        if let Some(on) = flag(s, &["voice_on", "voiceOn"]) {
            snap.voice_off = !on;
        }
        if let Some(listen) = text(s, &["listen"]) {
            snap.listen = Some(listen);
        }
    }
    if let Some(live) = view.get("live").filter(|l| l.is_object()) {
        let listening = flag(live, &["listening"]).unwrap_or(false);
        let stt = flag(live, &["stt_ready", "sttReady"]).unwrap_or(false);
        snap.mic_ready = Some(listening && stt);
        if let Some(on) = flag(live, &["user_talking", "userTalking"]) {
            snap.hearing = on;
        }
        if let Some(on) = flag(live, &["holding"]) {
            snap.key_listen = if on { "on".into() } else { String::new() };
        }
        if let Some(on) = flag(live, &["nixa_talking", "nixaTalking"]) {
            set_speaking(snap, on || flag(view, &["speaking"]).unwrap_or(false));
        }
    } else if let Some(on) = flag(view, &["speaking"]) {
        set_speaking(snap, on);
    }
}

fn set_talking(snap: &mut Snapshot, v: &Value) {
    let on = flag(v, &["on"]).unwrap_or(false);
    match text(v, &["who"]).as_deref() {
        Some("nixa") => set_speaking(snap, on),
        Some("user") => snap.hearing = on,
        _ => set_voice(snap, v),
    }
}

fn set_work(snap: &mut Snapshot, v: &Value) {
    let v = v.get("work").filter(|w| w.is_object()).unwrap_or(v);
    if let Some(hold) = flag(v, &["hold"]) {
        snap.work.hold = hold;
    }
    if v.get("topic").is_some() {
        snap.work.topic = v.get("topic").filter(|t| t.is_object()).map(|t| Topic {
            kind: text(t, &["kind"]).unwrap_or_default(),
            text: text(t, &["text"]).unwrap_or_default(),
        });
    }
}

fn set_turn_usage(snap: &mut Snapshot, v: &Value) {
    let input = number(v, &["input"]);
    let output = number(v, &["output"]);
    let mut usage = snap.usage.clone().unwrap_or_default();
    usage.tokens += input + output;
    usage.turns += 1;
    usage.model = text(v, &["model"]).unwrap_or_default();
    usage.input = input;
    usage.output = output;
    usage.context = context_pct(v);
    snap.usage = Some(usage);
}

fn set_usage(snap: &mut Snapshot, v: &Value) {
    if is_turn_usage(v) {
        set_turn_usage(snap, v);
        return;
    }
    let today = v.get("today").unwrap_or(v);
    let last = v
        .get("turn")
        .filter(|t| t.is_object())
        .map(|t| (number(t, &["ms"]), number(t, &["tokens"])));
    snap.usage = Some(Usage {
        tokens: number(today, &["tokens"]),
        cached: number(today, &["cached"]),
        turns: number(today, &["turns"]),
        last,
        ..Usage::default()
    });
}

fn set_confirm(snap: &mut Snapshot, v: &Value) {
    let text_of = text(v, &["text", "confirm"]).unwrap_or_default();
    snap.confirm = (!text_of.trim().is_empty()).then(|| Confirm {
        id: text(v, &["id"]).unwrap_or_default(),
        text: text_of,
        why: text(v, &["why"]).unwrap_or_default(),
    });
}

pub fn apply(snap: &mut Snapshot, msg: &Value) -> bool {
    let Some(kind) = msg.get("type").and_then(Value::as_str) else {
        return false;
    };
    match kind {
        "hello" => {
            snap.link = Link::Up;
            if let Some(name) = text(msg, &["name"]) {
                snap.name = name;
            }
            snap.busy = flag(msg, &["busy"]).unwrap_or(false);
            snap.items = items_of(msg.get("items").unwrap_or(&Value::Null));
            snap.draft.clear();
        }
        "chat" => match msg
            .get("item")
            .and_then(|i| serde_json::from_value::<Item>(i.clone()).ok())
        {
            Some(item) => add_item(snap, item),
            None => return false,
        },
        "chat_cleaned" => {
            snap.items.clear();
            snap.draft.clear();
        }
        "busy" => {
            snap.busy = flag(msg, &["busy"]).unwrap_or(false);
            if !snap.busy {
                snap.draft.clear();
            }
        }
        "delta" => snap
            .draft
            .push_str(&text(msg, &["text"]).unwrap_or_default()),
        "draft" => snap.draft = text(msg, &["text"]).unwrap_or_default(),
        "reply" => snap.draft.clear(),
        "health" => {
            if let Some(name) = text(msg, &["name"]) {
                snap.name = name;
            }
            if let Some(busy) = flag(msg, &["busy"]) {
                snap.busy = busy;
            }
            set_brain(snap, msg.get("brain"));
        }
        "brain" => set_brain(snap, msg.get("brain").or(Some(msg))),
        "state" => set_voice(snap, msg),
        "voice" => match msg.get("voice").filter(|x| x.is_object()) {
            Some(view) => set_voice_view(snap, view),
            None => set_voice(snap, msg),
        },
        "talking" => set_talking(snap, msg),
        "level" => match flag(msg, &["talking"]) {
            Some(on) => snap.hearing = on,
            None => return false,
        },
        "wake" => snap.wake_until = number(msg, &["until"]),
        "work" => set_work(snap, msg),
        "usage" => set_usage(snap, msg),
        "confirm" => set_confirm(snap, msg),
        "confirm_done" => snap.confirm = None,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hello_then_chat_items() {
        let mut snap = Snapshot::default();
        assert!(apply(
            &mut snap,
            &json!({"type": "hello", "name": "Nixa", "busy": false, "items": [
                {"id": 1, "who": "user", "text": "hi", "at": 5, "typed": true},
                {"id": 2, "who": "nixa", "text": "hello", "at": 6}
            ]})
        ));
        assert_eq!(snap.link, Link::Up);
        assert_eq!(snap.items.len(), 2);
        apply(
            &mut snap,
            &json!({"type": "chat", "item": {"id": 3, "who": "info", "text": "note", "at": 7}}),
        );
        apply(
            &mut snap,
            &json!({"type": "chat", "item": {"id": 2, "who": "nixa", "text": "hello again", "at": 6, "quiet": true}}),
        );
        assert_eq!(
            snap.items.iter().map(|i| i.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(snap.items[1].text, "hello again");
        assert!(snap.items[1].quiet);
        apply(&mut snap, &json!({"type": "chat_cleaned"}));
        assert!(snap.items.is_empty());
    }

    #[test]
    fn drafts_stream_and_clear() {
        let mut snap = Snapshot::default();
        apply(&mut snap, &json!({"type": "busy", "busy": true}));
        apply(&mut snap, &json!({"type": "delta", "text": "Hel"}));
        apply(&mut snap, &json!({"type": "delta", "text": "lo"}));
        assert_eq!(snap.draft, "Hello");
        apply(
            &mut snap,
            &json!({"type": "chat", "item": {"id": 9, "who": "nixa", "text": "Hello", "at": 1}}),
        );
        assert!(snap.draft.is_empty());
        apply(&mut snap, &json!({"type": "draft", "text": "x"}));
        apply(&mut snap, &json!({"type": "busy", "busy": false}));
        assert!(snap.draft.is_empty() && !snap.busy);
    }

    #[test]
    fn health_voice_work_usage_confirm() {
        let mut snap = Snapshot::default();
        apply(
            &mut snap,
            &json!({"type": "health", "name": "Juno", "busy": true, "brain": {"name": "Claude", "kind": "anthropic", "model": "claude-x"}}),
        );
        assert_eq!(snap.name(), "Juno");
        assert_eq!(snap.brain.as_ref().unwrap().model, "claude-x");
        apply(&mut snap, &json!({"type": "health", "brain": null}));
        assert!(snap.brain.is_none());
        apply(
            &mut snap,
            &json!({"type": "state", "mic_off": true, "listen": "wake", "wakeActive": true}),
        );
        assert!(snap.mic_off && snap.wake_active);
        assert_eq!(snap.listen.as_deref(), Some("wake"));
        apply(
            &mut snap,
            &json!({"type": "work", "work": {"hold": true, "topic": {"kind": "plan", "text": "t"}}}),
        );
        assert!(snap.work.hold);
        assert_eq!(snap.work.topic.as_ref().map(|t| t.kind.as_str()), Some("plan"));
        apply(&mut snap, &json!({"type": "work", "topic": null}));
        assert!(snap.work.topic.is_none());
        apply(
            &mut snap,
            &json!({"type": "usage", "today": {"tokens": 1500, "cached": 20, "turns": 3}, "turn": {"ms": 2300, "tokens": 400}}),
        );
        assert_eq!(
            snap.usage,
            Some(Usage {
                tokens: 1500,
                cached: 20,
                turns: 3,
                last: Some((2300, 400)),
                ..Usage::default()
            })
        );
        apply(
            &mut snap,
            &json!({"type": "confirm", "id": "c1", "text": "Delete it?", "why": "asked"}),
        );
        assert_eq!(snap.confirm.as_ref().unwrap().id, "c1");
        apply(&mut snap, &json!({"type": "confirm", "text": ""}));
        assert!(snap.confirm.is_none());
        assert!(!apply(&mut snap, &json!({"type": "unknown"})));
        assert!(!apply(&mut snap, &json!({"no": "type"})));
    }

    #[test]
    fn brain_voice_events_fill_the_card() {
        let mut snap = Snapshot::default();
        assert!(apply(
            &mut snap,
            &json!({"type": "voice", "voice": {
                "settings": {"micOn": false, "voiceOn": false, "listen": "wake", "pushTalk": true},
                "live": {"listening": true, "sttReady": true, "userTalking": false, "nixaTalking": false, "holding": true},
                "voice": "x", "speaking": false
            }})
        ));
        assert!(snap.mic_off && snap.voice_off);
        assert_eq!(snap.listen.as_deref(), Some("wake"));
        assert_eq!(snap.mic_ready, Some(true));
        assert_eq!(snap.key_listen, "on");
        apply(
            &mut snap,
            &json!({"type": "voice", "voice": {"settings": {"micOn": true, "voiceOn": true, "listen": "always"}, "live": {"listening": false, "sttReady": true, "nixaTalking": true}}}),
        );
        assert!(!snap.mic_off && !snap.voice_off);
        assert_eq!(snap.mic_ready, Some(false));
        assert_eq!(snap.state, "speaking");
        apply(
            &mut snap,
            &json!({"type": "talking", "who": "nixa", "on": false}),
        );
        assert_eq!(snap.state, "");
        apply(
            &mut snap,
            &json!({"type": "talking", "who": "user", "on": true}),
        );
        assert!(snap.hearing);
        apply(
            &mut snap,
            &json!({"type": "level", "mic": 0.2, "talking": false}),
        );
        assert!(!snap.hearing);
        apply(&mut snap, &json!({"type": "state", "state": "off"}));
        apply(
            &mut snap,
            &json!({"type": "talking", "who": "nixa", "on": false}),
        );
        assert_eq!(snap.state, "off");
        apply(&mut snap, &json!({"type": "wake", "until": 5000}));
        assert_eq!(snap.wake_until, 5000);
    }

    #[test]
    fn the_item_list_is_capped() {
        let mut snap = Snapshot::default();
        for id in 0..(KEEP_ITEMS as u64 + 10) {
            apply(
                &mut snap,
                &json!({"type": "chat", "item": {"id": id, "who": "user", "text": "x", "at": 0}}),
            );
        }
        assert_eq!(snap.items.len(), KEEP_ITEMS);
        assert_eq!(snap.items[0].id, 10);
    }
}
