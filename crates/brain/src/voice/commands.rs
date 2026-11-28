use super::settings::{Listen, VoiceSettings};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Mic(bool),
    Listen(Listen),
    Voice { on: Option<bool>, name: Option<String> },
    Audio { mic: Option<String>, speaker: Option<String> },
    RefreshAudio,
    HoldStart,
    HoldEnd,
    Stop,
    Snapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    pub save: bool,
    pub restart: bool,
    pub speaker: bool,
    pub stop_speaking: bool,
    pub info: Option<String>,
}

fn flag(msg: &Value) -> Option<bool> {
    msg.get("on").and_then(Value::as_bool)
}

fn text(msg: &Value, key: &str) -> Option<String> {
    msg.get(key).and_then(Value::as_str).map(|s| s.trim().to_string())
}

pub fn parse(kind: &str, msg: &Value) -> Option<Command> {
    match kind {
        "set_mic" => flag(msg).map(Command::Mic),
        "set_listen" => text(msg, "mode").and_then(|m| Listen::parse(&m)).map(Command::Listen),
        "set_push_talk" => flag(msg).map(|on| Command::Listen(if on { Listen::Hotkey } else { Listen::Always })),
        "set_voice" => {
            let on = flag(msg);
            let name = text(msg, "name").or_else(|| text(msg, "voice"));
            (on.is_some() || name.is_some()).then_some(Command::Voice { on, name })
        }
        "set_audio" => {
            let mic = text(msg, "mic");
            let speaker = text(msg, "speaker");
            (mic.is_some() || speaker.is_some()).then_some(Command::Audio { mic, speaker })
        }
        "refresh_audio" => Some(Command::RefreshAudio),
        "hold_start" => Some(Command::HoldStart),
        "hold_end" => Some(Command::HoldEnd),
        "interrupt" | "stop" => Some(Command::Stop),
        "voice_state" => Some(Command::Snapshot),
        _ => None,
    }
}

fn blank_none(value: &str) -> Option<String> {
    Some(value.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn listen_text(mode: Listen, name: &str) -> String {
    match mode {
        Listen::Hotkey => format!("Mic: hotkey. Tap Right Command (or F9) once and talk. Tap it again to send. {name} does not hear the room between presses."),
        Listen::Always => format!("Mic: always on. {name} hears everything while the mic is on and picks what is meant for it."),
    }
}

pub fn apply(s: &mut VoiceSettings, command: &Command, name: &str) -> Change {
    let mut change = Change::default();
    match command {
        Command::Mic(on) => {
            if s.mic_on != *on {
                s.mic_on = *on;
                change.save = true;
                change.restart = true;
            }
        }
        Command::Listen(mode) => {
            if s.listen != *mode {
                s.listen = *mode;
                change.save = true;
                change.restart = true;
                change.info = Some(listen_text(*mode, name));
            }
        }
        Command::Voice { on, name: voice } => {
            if let Some(on) = on
                && s.voice_on != *on
            {
                s.voice_on = *on;
                change.save = true;
                change.stop_speaking = !*on;
            }
            if let Some(voice) = voice {
                let voice = blank_none(voice);
                if s.voice != voice {
                    s.voice = voice;
                    change.save = true;
                }
            }
        }
        Command::Audio { mic, speaker } => {
            if let Some(mic) = mic {
                let mic = blank_none(mic);
                if s.mic_device != mic {
                    change.info = Some(format!("Mic: {}.", mic.as_deref().unwrap_or("Mac default")));
                    s.mic_device = mic;
                    change.save = true;
                    change.restart = true;
                }
            }
            if let Some(speaker) = speaker {
                let speaker = blank_none(speaker);
                if s.speaker_device != speaker {
                    let line = format!("Speaker: {}.", speaker.as_deref().unwrap_or("Mac output"));
                    change.info = Some(match change.info.take() {
                        Some(before) => format!("{before} {line}"),
                        None => line,
                    });
                    s.speaker_device = speaker;
                    change.save = true;
                    change.speaker = true;
                }
            }
        }
        Command::Stop => change.stop_speaking = true,
        Command::RefreshAudio | Command::HoldStart | Command::HoldEnd | Command::Snapshot => {}
    }
    change
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_the_voice_messages() {
        assert_eq!(parse("set_mic", &json!({"on": true})), Some(Command::Mic(true)));
        assert_eq!(parse("set_mic", &json!({})), None);
        assert_eq!(parse("set_listen", &json!({"mode": "hotkey"})), Some(Command::Listen(Listen::Hotkey)));
        assert_eq!(parse("set_listen", &json!({"mode": "wake"})), Some(Command::Listen(Listen::Always)));
        assert_eq!(parse("set_listen", &json!({"mode": "loud"})), None);
        assert_eq!(parse("set_push_talk", &json!({"on": false})), Some(Command::Listen(Listen::Always)));
        assert_eq!(parse("set_push_talk", &json!({"on": true})), Some(Command::Listen(Listen::Hotkey)));
        assert_eq!(parse("set_voice", &json!({"on": true})), Some(Command::Voice { on: Some(true), name: None }));
        assert_eq!(parse("set_voice", &json!({"name": "Harper"})), Some(Command::Voice { on: None, name: Some("Harper".into()) }));
        assert_eq!(parse("set_voice", &json!({})), None);
        assert_eq!(parse("set_audio", &json!({"mic": "BlackHole 2ch"})), Some(Command::Audio { mic: Some("BlackHole 2ch".into()), speaker: None }));
        assert_eq!(parse("set_audio", &json!({})), None);
        assert_eq!(parse("interrupt", &json!({})), Some(Command::Stop));
        assert_eq!(parse("stop", &json!({})), Some(Command::Stop));
        assert_eq!(parse("hold_start", &json!({})), Some(Command::HoldStart));
        assert_eq!(parse("say", &json!({"text": "hi"})), None);
    }

    #[test]
    fn mic_change_restarts_only_when_it_changes() {
        let mut s = VoiceSettings::default();
        let c = apply(&mut s, &Command::Mic(true), "Nixa");
        assert!(c.save && c.restart && s.mic_on && s.listen == Listen::Hotkey);
        assert!(c.info.is_none());
        assert_eq!(apply(&mut s, &Command::Mic(true), "Nixa"), Change::default());
        let off = apply(&mut s, &Command::Mic(false), "Nixa");
        assert!(!s.mic_on && s.listen == Listen::Hotkey && off.restart);
    }

    #[test]
    fn listen_switches_between_hotkey_and_always() {
        let mut s = VoiceSettings::default();
        let c = apply(&mut s, &Command::Listen(Listen::Always), "Nixa");
        assert!(s.listen == Listen::Always && c.restart && c.save);
        assert!(c.info.unwrap().starts_with("Mic: always on."));
        assert_eq!(apply(&mut s, &Command::Listen(Listen::Always), "Nixa"), Change::default());
        let back = apply(&mut s, &Command::Listen(Listen::Hotkey), "Nixa");
        assert!(s.listen == Listen::Hotkey && back.restart);
        assert!(back.info.unwrap().contains("Tap it again to send"));
    }

    #[test]
    fn voice_switches() {
        let mut s = VoiceSettings::default();
        let on = apply(&mut s, &Command::Voice { on: Some(true), name: Some("Harper".into()) }, "Nixa");
        assert!(s.voice_on && on.save && !on.stop_speaking && !on.restart);
        assert_eq!(s.voice.as_deref(), Some("Harper"));
        let off = apply(&mut s, &Command::Voice { on: Some(false), name: None }, "Nixa");
        assert!(off.stop_speaking && !s.voice_on);
        apply(&mut s, &Command::Voice { on: None, name: Some(" ".into()) }, "Nixa");
        assert_eq!(s.voice, None);
    }

    #[test]
    fn audio_devices_change_mic_and_speaker() {
        let mut s = VoiceSettings::default();
        let c = apply(&mut s, &Command::Audio { mic: Some("BlackHole 2ch".into()), speaker: Some("BlackHole 2ch".into()) }, "Nixa");
        assert!(c.restart && c.speaker && c.save);
        assert_eq!(c.info.as_deref(), Some("Mic: BlackHole 2ch. Speaker: BlackHole 2ch."));
        let back = apply(&mut s, &Command::Audio { mic: Some(String::new()), speaker: None }, "Nixa");
        assert_eq!(s.mic_device, None);
        assert!(back.restart && !back.speaker);
        assert_eq!(back.info.as_deref(), Some("Mic: Mac default."));
        assert!(apply(&mut s, &Command::Stop, "Nixa").stop_speaking);
    }
}
