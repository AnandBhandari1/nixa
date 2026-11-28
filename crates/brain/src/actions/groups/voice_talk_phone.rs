use crate::actions::{Action, Body, Kind, inputs};
use serde_json::json;

pub fn list() -> Vec<Action> {
    let mut out = voice();
    out.extend(voices());
    out.extend(phone());
    out.extend(talk());
    out
}

fn on_input(what: &str) -> serde_json::Value {
    inputs(json!({"on": {"type": "boolean", "description": what}}), &["on"])
}

fn voice() -> Vec<Action> {
    vec![
        Action {
            name: "voice.status",
            method: "GET",
            path: "/voice",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the voice settings, the mic state and the voice in use.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "voice.devices",
            method: "GET",
            path: "/voice/devices",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the mics and speakers on this Mac.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "voice.mic",
            method: "POST",
            path: "/voice",
            body: Body::Field("type", "set_mic"),
            kind: Kind::Write,
            help: "Turn the mic on or off.",
            inputs: on_input("true turns the mic on, false turns it off."),
            card: Some("Turn the mic on: {on}?"),
        },
        Action {
            name: "voice.listen",
            method: "POST",
            path: "/voice",
            body: Body::Field("type", "set_listen"),
            kind: Kind::Write,
            help: "Pick how the mic listens: Right Command hotkey or always on.",
            inputs: inputs(
                json!({"mode": {"type": "string", "enum": ["hotkey", "always"], "description": "hotkey: tap Right Command (or F9) to talk and again to send; always: hears everything while the mic is on."}}),
                &["mode"],
            ),
            card: Some("Set the listen mode to {mode}?"),
        },
        Action {
            name: "voice.set",
            method: "POST",
            path: "/voice",
            body: Body::Field("type", "set_voice"),
            kind: Kind::Write,
            help: "Turn the spoken voice on or off, or pick the voice name. Give on, name or both.",
            inputs: inputs(
                json!({
                    "on": {"type": "boolean", "description": "true turns the spoken voice on, false turns it off."},
                    "name": {"type": "string", "description": "The voice name to use."}
                }),
                &[],
            ),
            card: Some("Change the spoken voice: on {on}, name {name}?"),
        },
        Action {
            name: "voice.audio",
            method: "POST",
            path: "/voice",
            body: Body::Field("type", "set_audio"),
            kind: Kind::Write,
            help: "Pick the mic or the speaker device. Give mic, speaker or both.",
            inputs: inputs(
                json!({
                    "mic": {"type": "string", "description": "Mic name from voice.devices. Empty text means the default mic."},
                    "speaker": {"type": "string", "description": "Speaker name from voice.devices. Empty text means the default speaker."}
                }),
                &[],
            ),
            card: Some("Use the mic {mic} and the speaker {speaker}?"),
        },
        Action {
            name: "voice.stop",
            method: "POST",
            path: "/voice",
            body: Body::Field("type", "stop"),
            kind: Kind::Write,
            help: "Stop Nixa talking and stop the brain's current reply.",
            inputs: inputs(json!({}), &[]),
            card: Some("Stop Nixa talking and stop the current reply?"),
        },
    ]
}

fn level_input() -> serde_json::Value {
    json!({"type": "string", "enum": ["default", "terminal", "model", "agent", "space", "kind"], "description": "What the voice is for. default is the voice for everything."})
}

fn voices() -> Vec<Action> {
    vec![
        Action {
            name: "voices.list",
            method: "GET",
            path: "/voices",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the voice list, the voice map and the humor setting.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "voices.state",
            method: "POST",
            path: "/voices",
            body: Body::Field("action", "state"),
            kind: Kind::Read,
            help: "Show the voice map, and which voice each given row would use.",
            inputs: inputs(
                json!({
                    "rows": {
                        "type": "array",
                        "description": "Rows to check, each with an id and a ctx of terminal, model, agent, space and kind.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {"type": "string"},
                                "ctx": {
                                    "type": "object",
                                    "properties": {
                                        "terminal": {"type": "string"},
                                        "model": {"type": "string"},
                                        "agent": {"type": "string"},
                                        "space": {"type": "string"},
                                        "kind": {"type": "string"}
                                    }
                                }
                            },
                            "required": ["id"]
                        }
                    }
                }),
                &[],
            ),
            card: None,
        },
        Action {
            name: "voices.humor",
            method: "POST",
            path: "/voices",
            body: Body::Field("action", "humor"),
            kind: Kind::Write,
            help: "Set the humor in spoken lines to light or off.",
            inputs: inputs(json!({"value": {"type": "string", "enum": ["light", "off"], "description": "light or off."}}), &["value"]),
            card: Some("Set the voice humor to {value}?"),
        },
        Action {
            name: "voices.set",
            method: "POST",
            path: "/voices",
            body: Body::Field("action", "set"),
            kind: Kind::Write,
            help: "Pick a voice for everything, or for one terminal, model, agent, space or kind.",
            inputs: inputs(
                json!({
                    "level": level_input(),
                    "key": {"type": "string", "description": "The terminal, model, agent, space or kind name. Not needed for default."},
                    "voice": {"type": "string", "description": "Voice name from voices.list."}
                }),
                &["level", "voice"],
            ),
            card: Some("Use the voice {voice} for {level} {key}?"),
        },
        Action {
            name: "voices.clear",
            method: "POST",
            path: "/voices",
            body: Body::Field("action", "clear"),
            kind: Kind::Write,
            help: "Remove a picked voice for everything, or for one terminal, model, agent, space or kind.",
            inputs: inputs(
                json!({
                    "level": level_input(),
                    "key": {"type": "string", "description": "The terminal, model, agent, space or kind name. Not needed for default."}
                }),
                &["level"],
            ),
            card: Some("Remove the picked voice for {level} {key}?"),
        },
        Action {
            name: "voices.reset",
            method: "POST",
            path: "/voices",
            body: Body::Field("action", "reset"),
            kind: Kind::Write,
            help: "Remove every picked voice and go back to the normal voices.",
            inputs: inputs(json!({}), &[]),
            card: Some("Remove every picked voice?"),
        },
        Action {
            name: "voices.play",
            method: "POST",
            path: "/voices",
            body: Body::Field("action", "play"),
            kind: Kind::Write,
            help: "Play a short sample of a voice, or of the voice a ctx would use.",
            inputs: inputs(
                json!({
                    "voice": {"type": "string", "description": "Voice name from voices.list. Leave out to use the ctx."},
                    "ctx": {
                        "type": "object",
                        "description": "Where the voice is used, to find its voice.",
                        "properties": {
                            "terminal": {"type": "string"},
                            "model": {"type": "string"},
                            "agent": {"type": "string"},
                            "space": {"type": "string"},
                            "kind": {"type": "string"}
                        }
                    }
                }),
                &[],
            ),
            card: Some("Play a voice sample (voice {voice}, for {ctx.agent} {ctx.terminal})?"),
        },
        Action {
            name: "voices.stop",
            method: "POST",
            path: "/voices",
            body: Body::Field("action", "stop"),
            kind: Kind::Write,
            help: "Stop a voice sample that is playing.",
            inputs: inputs(json!({}), &[]),
            card: Some("Stop the voice sample?"),
        },
    ]
}

fn phone() -> Vec<Action> {
    vec![
        Action {
            name: "phone.status",
            method: "GET",
            path: "/phone",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the phone door, its address, the pairing QR code, the mobile setting and the paired phones.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "phone.door",
            method: "POST",
            path: "/phone",
            body: Body::Field("type", "set_phone"),
            kind: Kind::Write,
            help: "Turn the phone door on or off.",
            inputs: on_input("true opens the phone door, false closes it."),
            card: Some("Turn the phone door on: {on}?"),
        },
        Action {
            name: "phone.mobile",
            method: "POST",
            path: "/phone",
            body: Body::Field("type", "set_mobile"),
            kind: Kind::Write,
            help: "Turn the mobile app link on or off.",
            inputs: on_input("true turns the mobile link on, false turns it off."),
            card: Some("Turn the mobile link on: {on}?"),
        },
        Action {
            name: "phone.new_key",
            method: "POST",
            path: "/phone",
            body: Body::Field("type", "phone_new_key"),
            kind: Kind::Write,
            help: "Make a new pairing code. Every paired phone is signed out.",
            inputs: inputs(json!({}), &[]),
            card: Some("Make a new phone pairing code? Every paired phone is signed out and must pair again."),
        },
        Action {
            name: "phone.remove",
            method: "POST",
            path: "/phone",
            body: Body::Field("type", "phone_remove"),
            kind: Kind::Write,
            help: "Remove a paired phone.",
            inputs: inputs(json!({"id": {"type": "string", "description": "Phone id from phone.status."}}), &["id"]),
            card: Some("Remove the paired phone {id}?"),
        },
    ]
}

fn talk() -> Vec<Action> {
    vec![
        Action {
            name: "talk.status",
            method: "GET",
            path: "/talk",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the talk state: presence, the open yes card, the spoken line and progress.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "talk.quiet",
            method: "POST",
            path: "/talk/quiet",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Start quiet time for some minutes. 0 ends quiet time.",
            inputs: inputs(json!({"minutes": {"type": "number", "minimum": 0, "description": "How many minutes. 0 ends quiet time."}}), &["minutes"]),
            card: Some("Set quiet time for {minutes} minutes?"),
        },
    ]
}
